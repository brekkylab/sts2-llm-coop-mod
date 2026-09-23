use std::sync::Arc;

use ailoy::agent::AgentBuilder;
use ailoy::message::{Message, Part, Role};
use anyhow::{Context as _, Result};
use cortex::console::Console;
use futures::StreamExt as _;
use sts2_core::{CoopConfig, LegalAction, Sts2Client};
use tokio::sync::Mutex;

use crate::event::{AgentEvent, Events};

#[derive(Debug, Clone)]
pub struct Choice {
    /// The whole turn, in order; at least one. The mod plays them back to back once
    /// the partner ends their turn, with no round trip in between.
    pub action_ids: Vec<String>,
    /// Line for the speech bubble; also logged as `reason=`.
    pub say: Option<String>,
}

impl Choice {
    pub fn first(&self) -> &str {
        &self.action_ids[0]
    }
}

/// One conversation per combat.
///
/// A fresh `Agent` is built for every decision, but the history carries over. It
/// lives here rather than in the `Agent` because we rewrite it: after each decision
/// the full briefing is shrunk to one line.
///
/// The console is shared; the FUSE-T mount can't be remounted per decision.
pub struct Decider {
    console: Arc<Mutex<Option<Console>>>,
    model: String,
    client: Sts2Client,
    /// Last language read, kept for when the game doesn't answer.
    language: std::sync::RwLock<String>,
    events: Events,
    history: Vec<Message>,
    /// Where to dump each outgoing prompt; None in the replay tool. Must be outside
    /// the mount, or the agent could read its own prompts back.
    prompt_log: Option<std::path::PathBuf>,
}

impl Decider {
    /// Takes the console already wrapped: the bridge loop shares the same handle.
    pub fn new(
        console: Arc<Mutex<Option<Console>>>,
        config: &CoopConfig,
        events: Events,
        prompt_log: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            console,
            client: Sts2Client::from_env(),
            // Not an in-game setting yet: only the bridge knows which providers have
            // keys in .env, and a typo would fail at request time.
            model: std::env::var("STS2_MODEL")
                .unwrap_or_else(|_| "bedrock/global.anthropic.claude-sonnet-5".into()),
            language: std::sync::RwLock::new(config.language.clone()),
            events,
            history: Vec::new(),
            prompt_log,
        }
    }

    /// History is in memory only; after a bridge restart it is rebuilt from `said/`.
    pub fn begin_combat(&mut self, said: &[(String, String)]) {
        self.history = crate::history::from_said(said);
    }

    pub fn end_combat(&mut self) {
        self.history.clear();
    }

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Read on every decision so an in-game change applies immediately.
    /// `config()` falls back to a default when the game doesn't answer, which would
    /// silently reset the language.
    async fn current_language(&self) -> String {
        let fetched = self.client.config().await.language;
        let mut held = self.language.write().expect("language lock");
        if *held != fetched {
            println!("대화 언어: {fetched}");
            *held = fetched.clone();
        }
        fetched
    }

    /// After combat, lets the agent rewrite `notes/learned.md` with its tools.
    ///
    /// The whole list is revised (merge repeats, drop contradictions) rather than
    /// appended to, and left alone if nothing was learned; the caller diffs the file
    /// to see whether it changed. The only place tools are used: it's outside the
    /// latency budget. Input is the conversation only. The partner's cards aren't
    /// in the data, so a silent combat would only produce made-up observations.
    pub async fn summarize(&mut self, said: &[(String, String)]) -> Result<()> {
        if said.is_empty() {
            return Ok(());
        }

        let transcript = said
            .iter()
            .map(|(who, text)| {
                let label = if who == "human" { "동료" } else { "나" };
                format!("{label}: {text}")
            })
            .collect::<Vec<_>>()
            .join("\n");

        let lang = self.current_language().await;
        let mut agent = AgentBuilder::new(&self.model)
            .instruction(&summary_rules(&lang))
            .system_tools()
            .shared_console(self.console.clone())
            // max_tokens / effort / thinking_display: patched ailoy, see ask().
            .max_tokens(4096)
            .effort("medium")
            .thinking_display("summarized")
            .build()
            .context("building the summary agent")?;

        // Includes what was said on the reward and map screens, not just in combat.
        let query = Message::new(Role::User).with_contents([Part::text(format!(
            "방금 끝난 전투를 전후해 동료와 이런 말을 주고받았습니다.\n\n{transcript}"
        ))]);

        let t0 = std::time::Instant::now();
        let (mut input, mut output, mut cache_read) = (0u64, 0u64, 0u64);
        let mut stream = agent.run(query);
        while let Some(out) = stream.next().await {
            let out = out.context("the summary turn failed")?;
            if let Some(u) = &out.usage {
                input += u.input_tokens;
                output += u.output_tokens;
                cache_read += u.cache_read_input_tokens.unwrap_or(0);
            }
            for part in out.message.tool_calls.iter().flatten() {
                if let Part::Function { function, .. } = part {
                    self.events.emit(AgentEvent::Tool {
                        name: function.name.clone(),
                        arg: serde_json::to_string(&function.arguments)
                            .unwrap_or_default()
                            .chars()
                            .take(120)
                            .collect(),
                    });
                }
            }
        }
        drop(stream);

        // The poll loop is stalled for this long (we run inside it), so
        // a fast partner can reach the next combat before we're back. If this grows
        // past ~15 s, move it to its own task without holding `&mut Decider`.
        println!(
            "  요약: {:.1}초 · 입력 {input}(+캐시 {cache_read}) · 출력 {output}",
            t0.elapsed().as_secs_f32()
        );
        self.events.emit(AgentEvent::Tokens {
            input,
            output,
            cache_read,
            truncated: false,
        });
        Ok(())
    }

    /// Picks the turn. A malformed answer is an error, and the game falls back to
    /// its heuristic.
    ///
    /// `actions` are the ones legal when the game asked, not the current ones: the
    /// board can move while the question waits, and an answer from the current
    /// state could be rejected at commit.
    pub async fn decide(
        &mut self,
        brief: &str,
        actions: &[LegalAction],
        state: &sts2_core::CombatState,
        unanswered: &[(String, String)],
    ) -> Result<Choice> {
        // Speech that arrived while the game was asking never went through
        // `respond()`; this is its only way into the prompt.
        let said = crate::history::join_said(unanswered);

        let text = self.ask(prompt(brief, actions, &said)).await?;

        // Before parse_choice's `?`: a malformed answer is still in the history, and
        // its full briefing must not ride along into the next decision. The partner's
        // words stay in the shrunk line so the history shows why a plan changed.
        self.shrink_last_turn(&turn_with_said(state, &said));

        parse_choice(&text, actions)
            .with_context(|| format!("no usable CHOSEN: line in the answer:\n{text}"))
    }

    pub fn shrink_last_turn(&mut self, line: &str) {
        crate::history::shrink_last_query(&mut self.history, line);
    }

    /// Replies to the partner while the game isn't asking. Words only, no `CHOSEN:`;
    /// a changed plan shows up at the next decision.
    pub async fn respond(
        &mut self,
        brief: &str,
        said: &[(String, String)],
        state: &sts2_core::CombatState,
    ) -> Result<String> {
        let joined = crate::history::join_said(said);
        let text = self.ask(respond_prompt(brief, &joined)).await?;

        // Keep the board with the words, so the reply can be understood later.
        crate::history::shrink_last_query(&mut self.history, &turn_with_said(state, &joined));
        let said = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with("CHOSEN:"))
            .next_back()
            .unwrap_or("")
            .to_string();

        if said.is_empty() {
            anyhow::bail!("the agent said nothing");
        }
        Ok(said)
    }

    /// Builds an agent, runs one query and returns the assistant's text.
    async fn ask(&mut self, query: Message) -> Result<String> {
        let mut agent = AgentBuilder::new(&self.model)
            .instruction(&rules_for(&self.current_language().await))
            .history(self.history.clone())
            // No system_tools() here: a tool call is a round trip the partner waits
            // on, and its output would stay in the history for the whole combat.
            .shared_console(self.console.clone())
            // NOTE: the next three builder methods are NOT in upstream ailoy. They
            // come from a local ailoy patch this project was built against (see the
            // README). With a stock ailoy, add them or drop these lines:
            //   max_tokens(n)          -> "max_tokens": n
            //   effort(e)              -> "output_config": {"effort": e}
            //   thinking_display(d)    -> "thinking": {"type": "adaptive", "display": d}
            // (Anthropic Messages API; Sonnet 5 rejects the older thinking.budget_tokens
            // and temperature with a 400.)
            //
            // Smaller limits produced empty answers: thinking used everything and no
            // text was left. 512, 768 and 1024 all did; 4096 was also faster.
            .max_tokens(4096)
            // The real thinking knob. "low" suits picking from a list whose numbers
            // the game already computed; the default "high" overran the limit.
            .effort("low")
            // The default, "omitted", returns empty thinking blocks. Same cost either
            // way; this just makes the model's reasoning visible in the window.
            .thinking_display("summarized")
            .build()
            .context("building the agent")?;

        // The only record of what was actually sent; the window shows replies only.
        if let Some(path) = &self.prompt_log {
            let _ = std::fs::write(
                path,
                crate::history::render_for_log(&self.history, &query),
            );
        }

        let mut text = String::new();
        let mut turns = 0usize;
        let mut tools = 0usize;
        let mut input_tokens = 0u64;
        let mut output_tokens = 0u64;
        let mut cache_read = 0u64;
        let mut truncated = false;
        let mut stream = agent.run(query);
        while let Some(output) = stream.next().await {
            let output = output.context("the agent turn failed")?;

            turns += 1;

            for part in output.message.tool_calls.iter().flatten() {
                tools += 1;
                if let Part::Function { function, .. } = part {
                    self.events.emit(AgentEvent::Tool {
                        name: function.name.clone(),
                        // One line's worth for the window.
                        arg: serde_json::to_string(&function.arguments)
                            .unwrap_or_default()
                            .chars()
                            .take(120)
                            .collect(),
                    });
                }
            }

            if let Some(u) = &output.usage {
                input_tokens += u.input_tokens;
                output_tokens += u.output_tokens;
                cache_read += u.cache_read_input_tokens.unwrap_or(0);
            }
            if matches!(output.finish_reason, ailoy::message::FinishReason::Length { .. }) {
                truncated = true;
            }

            if output.message.role == Role::Assistant {
                let piece = message_text(&output.message);

                // Only CHOSEN: and the last line are used; the rest of the answer is
                // kept nowhere else, and it's the clue when an answer looks wrong.
                let thinking = output.message.thinking.clone().filter(|t| !t.is_empty());
                if thinking.is_some() || !piece.trim().is_empty() {
                    self.events.emit(AgentEvent::Raw {
                        thinking,
                        text: piece.clone(),
                    });
                }

                text.push_str(&piece);
                text.push('\n');
            }
        }
        // The stream borrows `agent` mutably until dropped (E0499 otherwise).
        drop(stream);

        // Take the history back minus the system message: ailoy skips the
        // instruction whenever the history already has one, so the first decision's
        // rules (and language) would stick for the whole combat.
        self.history = std::mem::take(&mut agent.state.history);
        crate::history::strip_system(&mut self.history);

        // `output` includes thinking; the answer itself is ~40 tokens, so the rest
        // is how long the model thought.
        let shown: usize = text.trim().chars().count();
        println!(
            "  토큰: 입력 {input_tokens}(+캐시 {cache_read}) · 출력 {output_tokens} (글 {shown}자){}",
            if truncated { " · 상한에 걸림" } else { "" }
        );
        self.events.emit(AgentEvent::Tokens {
            input: input_tokens,
            output: output_tokens,
            cache_read,
            truncated,
        });

        // Tells "spent everything thinking" from "only called tools".
        if text.trim().is_empty() {
            eprintln!("  빈 답 (메시지 {turns}개 · 도구 {tools}회)");
        }

        Ok(text)
    }
}

/// Thinking budget stated in the rules text only; the API side is `effort`.
const THINKING_BUDGET: u64 = 1024;

/// Rules are in English (about half the tokens of the Korean version); the
/// language to speak is stated separately.
fn rules_for(language: &str) -> String {
    let rules = RULES.replace("{budget}", &THINKING_BUDGET.to_string());
    format!("{rules}\n\n## What language\n\nSpeak {language}. That is the only thing in {language} — these rules are not.\n")
}

/// Separate from the combat rules, which would pull answers toward `CHOSEN:`.
fn summary_rules(language: &str) -> String {
    SUMMARY_RULES.replace("{language}", language)
}

const SUMMARY_RULES: &str = r#"You are the AI teammate, looking back on a combat that just ended.

Your notes about your partner live at `notes/learned.md`. Read it, then rewrite
it whole — you are revising a list, not appending to it.

Merge what repeats. Drop what this combat contradicts. Keep it under twelve
lines: these ride along in every decision you make.

Write only what your partner said or asked for. You cannot see what cards they
played — the game hides their hand. If this combat taught you nothing about
them, leave the file alone and say so.

You cannot write anywhere else. `memory.md` is your partner's own file.

Speak {language}."#;

/// How to behave in this seat, not the rules of the game. Keep it short: it rides
/// along with every decision.
const RULES: &str = r#"You are the AI teammate in a two-player game of Slay the Spire 2.

## Who is who

You are `the AI teammate`. The person playing beside you is `the human player`.

Every action names its target as one of those two. `-> the AI teammate` means it
lands on you; `-> the human player` means it lands on your partner. **This is what
decides who gets a potion or a block card.**

**You cannot see your partner's hand.** The game hides it, and we do not read
around that. If you need to know whether the two of you can finish an enemy this
turn, ask.

## How to answer

Two lines. **The choice comes first.**

    CHOSEN: 1, 3
    <one sentence>

Order matters: if the answer gets cut off, a sentence written first takes the
choice down with it and the whole decision is thrown away.

Pick by **number**, in the order you mean to play them. One is fine, several are
fine. Chain as far as your energy goes, and include the end-turn action if you
mean to end there.

**You are not asked again between them** — they go out in the order you wrote. So
order them so each one is still playable after the last. If one becomes illegal
partway, the rest is dropped and you are asked again.

Only numbers from the list. You cannot plan around a card you have not drawn yet.

The sentence is what your partner reads. Say what you mean to do and why, or ask
them for something. **Keep it short** — it goes into a speech bubble in the game
and gets clipped. Don't copy arithmetic into it; your partner sees the same board.

## How long to think

**Finish thinking within {budget} tokens.** This is a choice from a list. The
numbers in the briefing are what the game computed — don't recount them, don't
weigh branch after branch.

Go over that and there is no room left for the answer, and **the whole decision is
thrown away.** That has happened: all the tokens spent thinking, not one character
written, and the game fell back to its own heuristic.

Choose even when you are unsure. **A worse move beats no move.**

## What you can see

The board is already in this message. Judge from it. You have no tools here —
every answer comes from what you were given.

Two lists of notes ride along: what your partner told you, and what you worked
out about them. **When they disagree, the one your partner wrote wins.**"#;


/// Rebuilt from files every time, so it stays the same size all combat.
fn prompt(brief: &str, actions: &[LegalAction], said: &str) -> Message {
    // Numbered, not by id: listing several long ids per turn got answers cut off at
    // the token limit, and a truncated id throws the whole decision away.
    let actions = actions
        .iter()
        .enumerate()
        .map(|(i, a)| {
            format!(
                "- `{}` — {}",
                i + 1,
                a.description.as_deref().unwrap_or(&a.action_id)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Partner's words go before the format instruction, which must stay last.
    let said = if said.is_empty() {
        String::new()
    } else {
        format!("\n\n## 동료가 방금 한 말\n\n{said}")
    };

    Message::new(Role::User).with_contents([Part::text(format!(
        "{brief}\n\n## 지금 낼 수 있는 행동\n\n{actions}{said}\n\n\
         이번 턴에 둘 것을 **번호로** 골라 `CHOSEN: 1, 3` 으로 **먼저** 적고, \
         그 아래에 한 문장을 쓰십시오."
    ))])
}

/// A past turn as one history line, with the partner's words if any.
fn turn_with_said(state: &sts2_core::CombatState, said: &str) -> String {
    let line = sts2_core::render::turn_line(state);
    if said.is_empty() {
        line
    } else {
        format!("{line} · 동료: {said}")
    }
}

fn respond_prompt(brief: &str, said: &str) -> Message {
    Message::new(Role::User).with_contents([Part::text(format!(
        "{brief}\n\n동료: {said}\n\n**한 문장으로만** \
         답하십시오. **지금은 수를 두는 자리가 아닙니다** — `CHOSEN:` 을 쓰지 \
         마십시오. 필요하면 계획을 바꾸어도 되고, 그것은 다음에 둘 때 드러납니다."
    ))])
}

fn message_text(m: &Message) -> String {
    m.contents
        .iter()
        .filter_map(|p| match p {
            Part::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Reads the last `CHOSEN:` line (models sometimes revise). No such line is a
/// failure; scraping ids out of prose would play the wrong move.
fn parse_choice(text: &str, actions: &[LegalAction]) -> Option<Choice> {
    let line = text
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("CHOSEN:"))
        .map(str::trim)?;

    let picks: Vec<String> = line
        .split(',')
        .map(|p| p.trim().trim_matches('`').to_string())
        .filter(|p| !p.is_empty())
        .collect();

    if picks.is_empty() {
        return None;
    }

    // Accept numbers or ids (old replays, habit). One unknown step rejects the
    // whole plan; a half-valid plan would stall midway, hard to diagnose.
    let mut action_ids = Vec::with_capacity(picks.len());
    for p in &picks {
        let resolved = match p.parse::<usize>() {
            Ok(n) if n >= 1 && n <= actions.len() => actions[n - 1].action_id.clone(),
            Ok(_) => return None,
            Err(_) if actions.iter().any(|a| &a.action_id == p) => p.clone(),
            Err(_) => return None,
        };
        action_ids.push(resolved);
    }

    // A card can be played once.
    let mut seen = std::collections::HashSet::new();
    let action_ids: Vec<String> = action_ids
        .into_iter()
        .filter(|id| seen.insert(id.clone()))
        .collect();

    let say = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with("CHOSEN:"))
        .next_back()
        .map(str::to_string);

    Some(Choice { action_ids, say })
}

#[cfg(test)]
mod tests {
    use super::*;

    impl Decider {
        fn for_test() -> Self {
            Self {
                console: Arc::new(Mutex::new(None)),
                client: Sts2Client::from_env(),
                model: "test".into(),
                language: std::sync::RwLock::new("한국어".into()),
                events: Events::new(),
                history: Vec::new(),
                prompt_log: None,
            }
        }
    }

    #[test]
    fn begins_a_combat_with_the_talk_so_far() {
        let mut d = Decider::for_test();
        d.begin_combat(&[
            ("human".to_string(), "방어 챙겨".to_string()),
            ("agent".to_string(), "그럴게".to_string()),
        ]);
        assert_eq!(d.history_len(), 2);
    }

    /// Only the latest briefing may stay in full.
    #[test]
    fn a_finished_turn_shrinks_to_one_line() {
        let mut d = Decider::for_test();
        d.history = vec![
            crate::history::user("[아주 긴 브리핑]"),
            Message::new(Role::Assistant).with_contents([Part::text("CHOSEN: 1")]),
        ];
        d.shrink_last_turn("턴 3 — 적 슬라임 18");
        assert_eq!(
            crate::history::text_of(&d.history[0]),
            "턴 3 — 적 슬라임 18"
        );
    }

    #[test]
    fn ends_a_combat_by_dropping_the_history() {
        let mut d = Decider::for_test();
        d.begin_combat(&[
            ("human".to_string(), "방어 챙겨".to_string()),
            ("agent".to_string(), "그럴게".to_string()),
        ]);
        d.end_combat();
        assert_eq!(d.history_len(), 0);
    }


    fn state() -> Vec<LegalAction> {
        let s: sts2_core::CombatState = serde_json::from_str(include_str!(
            "../../sts2-core/src/sample_state.json"
        ))
        .expect("표본이 파싱되지 않는다");
        s.legal_actions
    }

    #[test]
    fn reads_the_last_chosen_line() {
        let s = state();
        let id = &s[0].action_id;
        let text = format!("먼저 이걸 생각했다\nCHOSEN: 없는거\n아니 이게 낫다\nCHOSEN: {id}");
        let c = parse_choice(&text, &s).expect("골라야 한다");
        assert_eq!(c.action_ids, vec![id.clone()]);
        assert_eq!(c.say.as_deref(), Some("아니 이게 낫다"));
    }

    #[test]
    fn tolerates_backticks() {
        let s = state();
        let id = &s[0].action_id;
        let c = parse_choice(&format!("CHOSEN: `{id}`"), &s).expect("골라야 한다");
        assert_eq!(c.action_ids, vec![id.clone()]);
    }

    #[test]
    fn reads_a_whole_turn_in_order() {
        let s = state();
        let a = s[0].action_id.clone();
        let b = s[1].action_id.clone();
        let c = parse_choice(&format!("이번 턴 계획\nCHOSEN: {a}, `{b}`"), &s)
            .expect("골라야 한다");
        assert_eq!(c.action_ids, vec![a.clone(), b]);
        assert_eq!(c.first(), a);
    }

    #[test]
    fn reads_numbers_from_the_list() {
        let s = state();
        let c = parse_choice("번호로 고른다\nCHOSEN: 1, 2", &s).expect("골라야 한다");
        assert_eq!(c.action_ids, vec![s[0].action_id.clone(), s[1].action_id.clone()]);
    }

    /// The list is 1-based.
    #[test]
    fn refuses_a_number_outside_the_list() {
        let s = state();
        let past_end = s.len() + 1;
        assert!(parse_choice(&format!("CHOSEN: {past_end}"), &s).is_none());
        assert!(parse_choice("CHOSEN: 0", &s).is_none());
    }

    #[test]
    fn refuses_a_plan_with_an_invented_step() {
        let s = state();
        let a = s[0].action_id.clone();
        assert!(parse_choice(&format!("CHOSEN: {a}, play_card_made_up"), &s).is_none());
    }

    /// Left in, the mod would abort the plan with `action_missing`.
    #[test]
    fn drops_a_repeated_step() {
        let s = state();
        let a = s[0].action_id.clone();
        let c = parse_choice(&format!("CHOSEN: {a}, {a}"), &s).expect("골라야 한다");
        assert_eq!(c.action_ids, vec![a]);
    }

    #[test]
    fn refuses_an_invented_action() {
        assert!(parse_choice("CHOSEN: play_card_made_up", &state()).is_none());
    }

    #[test]
    fn refuses_prose_without_the_line() {
        let s = state();
        let text = format!("{} 를 내겠습니다", s[0].action_id);
        assert!(parse_choice(&text, &s).is_none());
    }
}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn carries_the_language_into_the_rules() {
        let r = rules_for("English");
        assert!(r.contains("English"), "{r}");
        assert!(r.contains("Slay the Spire 2"), "규칙 본문이 사라졌다");
    }
}
