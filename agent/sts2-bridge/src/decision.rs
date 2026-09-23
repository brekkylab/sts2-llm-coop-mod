use anyhow::Result;
use sts2_core::{CombatState, PendingDecision, Sts2Client};

use sts2_bridge::agent::Decider;
use sts2_bridge::event::{AgentEvent, Events};
use sts2_bridge::record::{self, Recorder};
use sts2_bridge::session::Session;

/// Carries the game's open question through to an answer, and wakes the agent when
/// the partner speaks even if the game isn't asking.
pub struct DecisionLoop {
    last_answered: Option<String>,
    recorder: Recorder,
    decider: Decider,

    /// Partner utterances only; the agent's own are never counted.
    last_said_count: usize,
    /// The partner spoke and no move has reflected it yet. Cleared only when a move
    /// is made, not when replying in words, so the next decision is recorded as
    /// `human_said` and a changed plan counts as `replaced`.
    human_input_pending: bool,
    /// Partner utterances the agent has seen (in a prompt). Decides what the next
    /// decision prompt carries.
    shown_up_to: usize,
    /// Partner utterances already reacted to, so none is answered twice. Differs
    /// from `shown_up_to` after a replan: we reacted (dropped the plan) but the
    /// agent hasn't seen the words yet.
    answered_up_to: usize,
    decided_this_turn: Option<(i32, String)>,
    /// `/state/combat` reports turn 0 outside combat.
    last_combat_turn: i32,
    /// Consecutive stale questions, for diagnostics.
    stale_ticks: usize,

    events: Events,
}

impl DecisionLoop {
    /// `said_so_far`: partner utterances already on disk. Starting at 0 would treat
    /// everything left from before a bridge restart as new.
    pub fn new(recorder: Recorder, decider: Decider, events: Events, said_so_far: usize) -> Self {
        Self {
            last_answered: None,
            recorder,
            decider,
            last_said_count: said_so_far,
            human_input_pending: false,
            shown_up_to: said_so_far,
            answered_up_to: said_so_far,
            decided_this_turn: None,
            last_combat_turn: 0,
            stale_ticks: 0,
            events,
        }
    }

    /// Raises the flag on new speech; never clears it.
    fn note_human_speech(&mut self, session: &Session) {
        let n = session.human_said_count();

        // Cleared from outside (our own clears go through `forget_said()`). Only
        // lower the cursors: a count can't tell survivors from new arrivals.
        if n < self.last_said_count {
            self.last_said_count = n;
            self.shown_up_to = self.shown_up_to.min(n);
            self.answered_up_to = self.answered_up_to.min(n);
            return;
        }

        if n > self.last_said_count {
            self.last_said_count = n;
            self.human_input_pending = true;
            if let Some((who, text)) = session.last_said() {
                self.events.emit(AgentEvent::Said { who, text });
            }
        }
    }

    /// True if something was answered.
    pub async fn tick(
        &mut self,
        client: &Sts2Client,
        state: &CombatState,
        session: &Session,
    ) -> Result<bool> {
        let p: PendingDecision = client.pending_decision().await?;

        if state.in_combat && state.turn > 0 {
            self.last_combat_turn = state.turn;
        }

        self.note_human_speech(session);
        if self
            .decided_this_turn
            .as_ref()
            .is_some_and(|(t, _)| *t != state.turn)
        {
            self.decided_this_turn = None;
        }

        // The partner spoke while the game isn't asking: reply in words, or drop a
        // held plan so the game asks again.
        if self.human_input_pending && !p.pending && self.answered_up_to < self.last_said_count {
            // Take the words before moving the cursor past them.
            let said_now = session.human_said_since(self.answered_up_to);
            self.answered_up_to = self.last_said_count;

            // Holding a plan: drop it instead of replying. The game re-asks, and the
            // new plan's line is the reply (replying here too would double both the
            // talk and the model calls). `human_input_pending` and `shown_up_to` stay,
            // so the next decision carries the words and is recorded as `human_said`.
            if state.me.as_ref().is_some_and(|m| m.holding) {
                match client.replan().await {
                    Ok(true) => println!("  ↻ 계획을 버렸다 — 다시 묻는다"),
                    Ok(false) => {}
                    Err(e) => eprintln!("  replan: {e}"),
                }
                return Ok(true);
            }

            return self.respond_to_human(state, session, &said_now).await;
        }

        let (Some(request_id), true) = (p.request_id.clone(), p.pending) else {
            return Ok(false);
        };

        if self.last_answered.as_deref() == Some(request_id.as_str()) {
            return Ok(false);
        }

        // Answer from the actions legal when the game asked. Requiring an exact
        // fingerprint match wasted the whole 20 s whenever a new action became
        // playable meanwhile (the game never refreshes its question). Skip only if
        // one of the asked actions has disappeared.
        let Some(asked) = asked_actions(&p, state) else {
            self.stale_ticks += 1;
            if self.stale_ticks % STALE_COMPLAIN_EVERY == 0 {
                eprintln!(
                    "물음이 {}번째 낡았다 (물음 {:?})",
                    self.stale_ticks,
                    p.snapshot_id.as_deref().unwrap_or("-")
                );
            }
            return Ok(false);
        };
        self.stale_ticks = 0;

        // The mod filters out-of-combat decisions; empty here means that leaked.
        if asked.is_empty() {
            eprintln!("결정 요청 {request_id} 인데 낼 수 있는 행동이 없다");
            return Ok(false);
        }

        println!(
            "결정 요청 {request_id} (남은 {}ms) — 턴 {}",
            p.deadline_ms, state.turn
        );
        self.events.emit(AgentEvent::Thinking { turn: state.turn });

        // Latency is measured from here; the early returns above aren't decisions.
        let trigger = if self.human_input_pending {
            "human_said"
        } else {
            "turn_start"
        };
        let t0 = std::time::Instant::now();
        let outcome;
        let mut action_id = None;
        let mut error = None;
        let mut say = None;

        // Everything unseen, not just the last utterance.
        let unanswered = session.human_said_since(self.shown_up_to);
        match self
            .decider
            .decide(&session.brief_for_prompt(state), &asked, state, &unanswered)
            .await
        {
            Ok(c) => {
                println!(
                    "  → {} · {}",
                    c.action_ids.join(" → "),
                    c.say.as_deref().unwrap_or("-")
                );
                self.events.emit(AgentEvent::Chose {
                    action_id: c.action_ids.join(" → "),
                    say: c.say.clone(),
                });
                say = c.say.clone();

                // The plan's line is not written to said/, which would fill up with
                // the agent's monologue; the game log carries it as `reason=`.

                match client
                    // act_now is always false: only the partner's button reorders
                    // play. The model misread "kill it now" as "go first".
                    .answer(&request_id, &c.action_ids, c.say.as_deref(), false)
                    .await
                {
                    Ok(()) => {
                        // A different first move in the same turn after the partner
                        // spoke: the plan changed. This ratio is what the project
                        // measures.
                        let changed = matches!(
                            &self.decided_this_turn,
                            Some((t, prev)) if *t == state.turn && *prev != *c.first()
                        );
                        outcome = if changed && trigger == "human_said" {
                            println!("  ↻ 계획이 바뀌었다");
                            "replaced"
                        } else {
                            "executed"
                        };
                        self.decided_this_turn = Some((state.turn, c.first().to_string()));
                        action_id = Some(c.action_ids.join(" → "));
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        outcome = record::classify_refusal(&msg);
                        eprintln!("  answer: {msg}");
                        error = Some(msg);
                    }
                }
            }
            Err(e) => {
                // Don't answer: the game falls back to its heuristic on timeout.
                outcome = "agent_error";
                let msg = format!("{e:#}");
                eprintln!("  agent: {msg}");
                error = Some(msg);
            }
        }

        self.recorder.write(&record::Decision {
            t: now_rfc3339(),
            turn: state.turn,
            trigger,
            latency_ms: t0.elapsed().as_millis(),
            outcome,
            said: say,
            action_id,
            error,
        })?;
        self.events.emit(AgentEvent::Settled {
            outcome,
            latency_ms: t0.elapsed().as_millis(),
        });

        // The only place the flag is cleared.
        self.human_input_pending = false;
        // Seen (in the prompt) and answered (the plan's line).
        self.shown_up_to = self.last_said_count;
        self.answered_up_to = self.last_said_count;

        if outcome == "executed" || outcome == "replaced" {
            self.last_answered = Some(request_id);
            return Ok(true);
        }

        Ok(false)
    }

    /// Rebuilds the history from `said/` and sets `shown_up_to` to match.
    ///
    /// `from_said` leaves out the trailing run of partner utterances (unanswered),
    /// so the cursor is lowered by that much and the first decision carries them.
    /// Without this, the partner's last words before a bridge restart would be lost.
    /// `answered_up_to` is left alone: no reply is owed for words spoken while the
    /// bridge was down.
    pub fn begin_combat(&mut self, said: &[(String, String)]) {
        self.decider.begin_combat(said);

        let humans = said.iter().filter(|(who, _)| who == "human").count();
        let trailing = said
            .iter()
            .rev()
            .take_while(|(who, _)| who == "human")
            .count();
        self.shown_up_to = humans - trailing;
    }

    /// We cleared `said/`: counting restarts from zero. Only the clearer can know
    /// this; a count alone can't tell "survived the clear" from "arrived after", and
    /// words spoken during the summary would otherwise be marked as seen.
    pub fn forget_said(&mut self) {
        self.last_said_count = 0;
        self.shown_up_to = 0;
        self.answered_up_to = 0;
        // Or the next combat's first decision is recorded as `human_said`.
        self.human_input_pending = false;
    }

    /// Call after the summary.
    pub fn end_combat(&mut self) {
        self.decider.end_combat();
    }

    pub async fn summarize_combat(&mut self, said: &[(String, String)]) -> Result<()> {
        self.decider.summarize(said).await
    }

    /// Only when no plan is held; words are all that's possible now.
    async fn respond_to_human(
        &mut self,
        state: &CombatState,
        session: &Session,
        said_now: &[(String, String)],
    ) -> Result<bool> {
        println!("동료가 말했다 — 답한다 (턴 {})", state.turn);

        // Show thinking, or nothing appears for seconds and the following `Settled`
        // overwrites the previous decision's row in the window.
        self.events.emit(AgentEvent::Thinking {
            turn: self.last_combat_turn,
        });

        let t0 = std::time::Instant::now();
        let result = self
            .decider
            .respond(&session.brief_for_prompt(state), said_now, state)
            .await;

        let (outcome, said, error) = match &result {
            Ok(s) => {
                println!("  ↩ {s}");
                // Seen: the words were in the query and stay in the history. (Not on
                // failure, where the history isn't updated.)
                self.shown_up_to = self.last_said_count;
                session.say("agent", s).ok();
                // The only place agent speech reaches the window.
                self.events.emit(AgentEvent::Said {
                    who: "agent".into(),
                    text: s.clone(),
                });
                ("said_only", Some(s.clone()), None)
            }
            Err(e) => {
                let msg = format!("{e:#}");
                eprintln!("  agent: {msg}");
                ("agent_error", None, Some(msg))
            }
        };

        self.recorder.write(&record::Decision {
            t: now_rfc3339(),
            turn: self.last_combat_turn,
            trigger: "human_said",
            latency_ms: t0.elapsed().as_millis(),
            outcome,
            said,
            action_id: None,
            error,
        })?;
        self.events.emit(AgentEvent::Settled {
            outcome,
            latency_ms: t0.elapsed().as_millis(),
        });

        // `human_input_pending` stays: a reply isn't a move.
        Ok(result.is_ok())
    }
}

/// Complain every 25 stale ticks (5 s at 200 ms polling).
const STALE_COMPLAIN_EVERY: usize = 25;

/// The actions legal when the game asked (the fingerprint is ids joined by `|`),
/// with descriptions from the current state. None if any has disappeared.
fn asked_actions(p: &PendingDecision, state: &CombatState) -> Option<Vec<sts2_core::LegalAction>> {
    let fingerprint = p.snapshot_id.as_deref()?;
    if fingerprint.is_empty() {
        return None;
    }

    let mut out = Vec::new();
    for id in fingerprint.split('|') {
        let found = state.legal_actions.iter().find(|a| a.action_id == id)?;
        out.push(found.clone());
    }
    Some(out)
}

/// Hand-rolled to avoid a chrono dependency.
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Days since 1970-01-01 to a calendar date.
    let mut y = 1970u64;
    let mut d = days;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let len = if leap { 366 } else { 365 };
        if d < len {
            break;
        }
        d -= len;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut mo = 0usize;
    while d >= months[mo] {
        d -= months[mo];
        mo += 1;
    }

    format!(
        "{y:04}-{:02}-{:02}T{h:02}:{m:02}:{s:02}Z",
        mo + 1,
        d + 1
    )
}

#[cfg(test)]
mod asked_tests {
    use super::*;

    fn state() -> CombatState {
        serde_json::from_str(include_str!("../../sts2-core/src/sample_state.json"))
            .expect("표본이 파싱되지 않는다")
    }

    fn pending(fingerprint: &str) -> PendingDecision {
        serde_json::from_value(serde_json::json!({
            "pending": true,
            "requestId": "r1",
            "snapshotId": fingerprint,
            "deadlineMs": 20000,
        }))
        .expect("물음이 파싱되지 않는다")
    }

    /// A newly playable card must not make the question look stale.
    #[test]
    fn a_grown_state_still_answers_the_question() {
        let s = state();
        let first_two = format!(
            "{}|{}",
            s.legal_actions[0].action_id, s.legal_actions[1].action_id
        );

        let asked = asked_actions(&pending(&first_two), &s).expect("진행해야 한다");
        assert_eq!(asked.len(), 2, "물음에 있던 둘만 후보다");
        assert_eq!(asked[0].action_id, s.legal_actions[0].action_id);
    }

    #[test]
    fn a_vanished_action_makes_the_question_stale() {
        let s = state();
        let gone = format!("{}|play_card_gone", s.legal_actions[0].action_id);
        assert!(asked_actions(&pending(&gone), &s).is_none());
    }
}
