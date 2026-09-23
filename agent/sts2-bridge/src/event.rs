use tokio::sync::broadcast;

/// What the agent is doing. One stream, filtered by each consumer: the game uses
/// a few states, the window shows everything.
#[derive(Debug, Clone)]
pub enum AgentEvent {
    Thinking { turn: i32 },
    /// Window only.
    Tool { name: String, arg: String },
    Chose {
        action_id: String,
        say: Option<String>,
    },
    /// `who` is `human` or `agent`.
    Said { who: String, text: String },
    /// A session file changed (instead of a file watcher).
    Published { snapshot_id: String },
    Settled {
        outcome: &'static str,
        latency_ms: u128,
    },
    /// Raw model output for the window's debug view: the only place the parts of an
    /// answer that decisions don't use are kept.
    Raw {
        /// Empty unless thinking display is `summarized` (the default is `omitted`).
        thinking: Option<String>,
        text: String,
    },

    /// Tokens used by one decision. `cache_read` is not included in `input`
    /// (Anthropic counts them separately). It is only nonzero with the patched ailoy,
    /// which adds prompt-cache breakpoints (`cache_control: {"type": "ephemeral"}` on
    /// the system block and the last message); upstream ailoy sends none.
    Tokens {
        input: u64,
        output: u64,
        cache_read: u64,
        truncated: bool,
    },
    /// Whether the post-combat summary changed the notes, and their length. A
    /// steadily growing count means the agent appends instead of rewriting.
    Memory { changed: bool, lines: usize },

    /// The bridge is exiting: close the window. On macOS the GUI owns the main
    /// thread, so the process lives as long as the window.
    Shutdown,
}

/// Send errors with no subscribers are ignored: the bridge runs without a window.
#[derive(Clone)]
pub struct Events {
    tx: broadcast::Sender<AgentEvent>,
}

impl Default for Events {
    fn default() -> Self {
        Self::new()
    }
}

impl Events {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self { tx }
    }

    pub fn emit(&self, e: AgentEvent) {
        let _ = self.tx.send(e);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emitting_without_listeners_is_fine() {
        let ev = Events::new();
        ev.emit(AgentEvent::Thinking { turn: 3 });
    }

    #[tokio::test]
    async fn a_subscriber_gets_what_comes_after_it() {
        let ev = Events::new();
        ev.emit(AgentEvent::Thinking { turn: 1 });

        let mut rx = ev.subscribe();
        ev.emit(AgentEvent::Thinking { turn: 2 });

        let got = rx.recv().await.expect("이벤트가 와야 한다");
        assert!(matches!(got, AgentEvent::Thinking { turn: 2 }));
    }
}
