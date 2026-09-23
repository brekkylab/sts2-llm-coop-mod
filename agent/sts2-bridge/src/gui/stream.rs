//! Right pane: what the agent is doing.

use eframe::egui;
use sts2_bridge::event::AgentEvent;

/// Bounded so long combats don't grow memory.
pub const MAX_TURNS: usize = 40;

const MAX_ENTRIES: usize = 60;

/// Decisions and conversation share one stream, so "the partner spoke and the plan
/// changed" reads in one place.
#[derive(Debug, Clone, PartialEq)]
pub enum Entry {
    Tool { name: String, arg: String },
    Chose {
        action_id: String,
        say: Option<String>,
    },
    /// `who` is `human` or `agent`.
    Said { who: String, text: String },
    Settled { outcome: String, latency_ms: u128 },
    Tokens { input: u64, output: u64, cache_read: u64, truncated: bool },
    Memory { changed: bool, lines: usize },
    /// Raw model output, drawn only in debug mode.
    Raw { thinking: Option<String>, text: String },
}

#[derive(Debug, Default)]
pub struct Turn {
    pub turn: i32,
    pub entries: Vec<Entry>,
}

impl Turn {
    fn push(&mut self, e: Entry) {
        self.entries.push(e);
        if self.entries.len() > MAX_ENTRIES {
            self.entries.remove(0);
        }
    }
}

#[derive(Debug, Default)]
pub struct StreamView {
    pub turns: Vec<Turn>,
    pub thinking: bool,
    pub turn: i32,
    pub thinking_since: Option<std::time::Instant>,
    pub last_latency_ms: Option<u128>,
    draft: String,
    debug: bool,
}

impl StreamView {
    /// Same turn number, same block: a turn often has several decisions.
    fn current(&mut self) -> &mut Turn {
        let turn = self.turn;

        // Words from just before combat (turn 0) fold into the first turn.
        if let Some(last) = self.turns.last_mut() {
            if last.turn == 0 && turn > 0 {
                last.turn = turn;
            }
        }

        if self.turns.last().map(|t| t.turn) != Some(turn) {
            self.turns.push(Turn {
                turn,
                ..Default::default()
            });
            if self.turns.len() > MAX_TURNS {
                self.turns.remove(0);
            }
        }

        self.turns.last_mut().expect("방금 넣었다")
    }

    pub fn observe(&mut self, e: AgentEvent) {
        match e {
            AgentEvent::Thinking { turn } => {
                self.thinking = true;
                self.turn = turn;
                self.thinking_since = Some(std::time::Instant::now());
                self.current();
            }
            AgentEvent::Tool { name, arg } => self.current().push(Entry::Tool { name, arg }),
            AgentEvent::Chose { action_id, say } => {
                self.current().push(Entry::Chose { action_id, say })
            }
            AgentEvent::Settled {
                outcome,
                latency_ms,
            } => {
                self.thinking = false;
                self.thinking_since = None;
                self.last_latency_ms = Some(latency_ms);
                self.current().push(Entry::Settled {
                    outcome: outcome.to_string(),
                    latency_ms,
                });
            }
            AgentEvent::Tokens {
                input,
                output,
                cache_read,
                truncated,
            } => self.current().push(Entry::Tokens {
                input,
                output,
                cache_read,
                truncated,
            }),
            AgentEvent::Memory { changed, lines } => {
                self.current().push(Entry::Memory { changed, lines })
            }
            // Kept even when debug is off, so turning it on after a problem still
            // shows that decision.
            AgentEvent::Raw { thinking, text } => {
                self.current().push(Entry::Raw { thinking, text })
            }
            AgentEvent::Said { who, text } => self.current().push(Entry::Said { who, text }),
            AgentEvent::Published { .. } => {}
            // Handled by `gui::Ui`.
            AgentEvent::Shutdown => {}
        }
    }

    /// Returns text the human wants to send; the caller does the sending.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<String> {
        ui.horizontal(|ui| {
            ui.strong(format!("턴 {}", self.turn));
            ui.separator();
            if self.thinking {
                ui.spinner();
                let secs = self
                    .thinking_since
                    .map(|t| t.elapsed().as_secs_f32())
                    .unwrap_or(0.0);
                ui.label(format!("생각 중 {secs:.1}s"));
            } else if let Some(ms) = self.last_latency_ms {
                ui.weak(format!("쉬는 중 · 직전 {ms}ms"));
            } else {
                ui.weak("쉬는 중");
            }
        });
        ui.separator();

        // Pin the input to the bottom first, then give the rest to the stream.
        // Subtracting a fixed height pushed it off screen, where it was visible but
        // took no input.
        let debug = self.debug;

        let mut send = None;
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.debug, "원문")
                    .on_hover_text("모델이 낸 것을 그대로 본다. 판단은 CHOSEN 줄과 마지막 한 줄만 쓴다.");
                let entry = ui.add(
                    egui::TextEdit::singleline(&mut self.draft)
                        .hint_text("말을 건다…")
                        .desired_width(ui.available_width() - 70.0),
                );
                // Enter sends; focus is restored for the next message.
                let entered = entry.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if (entered || ui.button("보냄").clicked()) && !self.draft.trim().is_empty() {
                    send = Some(self.draft.trim().to_string());
                    self.draft.clear();
                    entry.request_focus();
                }
            });
            ui.separator();

            ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("stream")
                    .stick_to_bottom(true)
                    .auto_shrink([false; 2])
                    .show(ui, |ui| {
                        for t in &self.turns {
                            ui.add_space(6.0);
                            // No header for turn 0 (before combat).
                            if t.turn > 0 {
                                ui.strong(format!("── 턴 {} ──", t.turn));
                            } else {
                                ui.strong("── 전투 전 ──");
                            }

                            for e in &t.entries {
                                match e {
                                    Entry::Tool { name, arg } => {
                                        ui.monospace(format!("  ▸ {name} {arg}"));
                                    }
                                    Entry::Chose { action_id, say } => {
                                        if let Some(s) = say {
                                            ui.label(format!("나: {s}"));
                                        }
                                        ui.weak(format!("     → {action_id}"));
                                    }
                                    Entry::Said { who, text } => {
                                        let label = if who == "human" { "동료" } else { "나" };
                                        ui.label(format!("{label}: {text}"));
                                    }
                                    Entry::Settled {
                                        outcome,
                                        latency_ms,
                                    } => {
                                        ui.weak(format!("     {outcome} · {latency_ms}ms"));
                                    }
                                    Entry::Tokens {
                                        input,
                                        output,
                                        cache_read,
                                        truncated,
                                    } => {
                                        ui.weak(format!(
                                            "     토큰 {input}(+{cache_read})→{output}{}",
                                            if *truncated { " (상한)" } else { "" }
                                        ));
                                    }
                                    Entry::Memory { changed, lines } => {
                                        ui.weak(format!(
                                            "     {} ({lines}줄)",
                                            if *changed {
                                                "기억을 고쳐 썼다"
                                            } else {
                                                "기억은 그대로"
                                            }
                                        ));
                                    }
                                    Entry::Raw { thinking, text } => {
                                        if !debug {
                                            continue;
                                        }
                                        if let Some(t) = thinking {
                                            // Indent every line of the multi-paragraph
                                            // thinking to set it off from the text.
                                            for (i, line) in t.trim().lines().enumerate() {
                                                ui.monospace(if i == 0 {
                                                    format!("  ⟨생각⟩ {line}")
                                                } else {
                                                    format!("         {line}")
                                                });
                                            }
                                        }
                                        if !text.trim().is_empty() {
                                            ui.monospace(format!("  ⟨원문⟩ {}", text.trim()));
                                        }
                                    }
                                }
                            }
                        }
                    });
            });
        });

        send
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chose(id: &str) -> AgentEvent {
        AgentEvent::Chose {
            action_id: id.into(),
            say: Some("그래서".into()),
        }
    }

    fn said(who: &str, text: &str) -> AgentEvent {
        AgentEvent::Said {
            who: who.into(),
            text: text.into(),
        }
    }

    #[test]
    fn tracks_whether_it_is_thinking() {
        let mut v = StreamView::default();
        assert!(!v.thinking);

        v.observe(AgentEvent::Thinking { turn: 3 });
        assert!(v.thinking);
        assert_eq!(v.turn, 3);

        v.observe(chose("a"));
        v.observe(AgentEvent::Settled {
            outcome: "executed",
            latency_ms: 3200,
        });
        assert!(!v.thinking);
        assert_eq!(v.last_latency_ms, Some(3200));
    }

    #[test]
    fn groups_tools_under_the_turn_that_used_them() {
        let mut v = StreamView::default();
        v.observe(AgentEvent::Thinking { turn: 1 });
        v.observe(AgentEvent::Tool {
            name: "shell".into(),
            arg: "cat x".into(),
        });
        v.observe(chose("a"));

        v.observe(AgentEvent::Thinking { turn: 2 });
        v.observe(AgentEvent::Tool {
            name: "shell".into(),
            arg: "cat z".into(),
        });

        assert_eq!(v.turns.len(), 2);
        assert_eq!(v.turns[0].turn, 1);
        assert_eq!(v.turns[1].turn, 2);
    }

    #[test]
    fn folds_conversation_into_the_turn() {
        let mut v = StreamView::default();
        v.observe(AgentEvent::Thinking { turn: 2 });
        v.observe(chose("a"));
        v.observe(said("human", "너 먼저 해"));
        v.observe(said("agent", "그래 먼저 걸게"));
        v.observe(chose("b"));

        assert_eq!(v.turns.len(), 1);
        assert_eq!(v.turns[0].entries.len(), 4);
        assert!(matches!(v.turns[0].entries[1], Entry::Said { .. }));
    }

    #[test]
    fn keeps_one_block_per_game_turn() {
        let mut v = StreamView::default();
        v.observe(AgentEvent::Thinking { turn: 4 });
        v.observe(chose("a"));
        v.observe(AgentEvent::Thinking { turn: 4 });
        v.observe(chose("b"));

        assert_eq!(v.turns.len(), 1);
        assert_eq!(v.turns[0].entries.len(), 2);
    }

    #[test]
    fn keeps_what_was_said_before_any_turn() {
        let mut v = StreamView::default();
        v.observe(said("human", "준비됐어"));

        assert_eq!(v.turns.len(), 1);
        assert_eq!(v.turns[0].entries.len(), 1);
    }

    #[test]
    fn folds_the_pre_combat_word_into_the_first_turn() {
        let mut v = StreamView::default();
        v.observe(said("human", "준비됐어"));
        v.observe(AgentEvent::Thinking { turn: 1 });
        v.observe(chose("a"));

        assert_eq!(v.turns.len(), 1);
        assert_eq!(v.turns[0].turn, 1);
        assert_eq!(v.turns[0].entries.len(), 2);
    }

    #[test]
    fn forgets_old_turns() {
        let mut v = StreamView::default();
        for i in 0..(MAX_TURNS + 5) {
            v.observe(AgentEvent::Thinking { turn: i as i32 });
        }
        assert_eq!(v.turns.len(), MAX_TURNS);
        assert_eq!(v.turns[0].turn, 5);
    }
}
