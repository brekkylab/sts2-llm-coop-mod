use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Decision {
    pub t: String,
    pub turn: i32,
    /// `turn_start` (the game asked) or `human_said` (the partner spoke).
    pub trigger: &'static str,
    pub latency_ms: u128,
    /// Only what the bridge can observe. `too_late` vs `illegal_action` comes from
    /// the game's rejection reason: one calls for faster polling, the other for a
    /// better prompt.
    pub outcome: &'static str,
    pub said: Option<String>,
    pub action_id: Option<String>,
    pub error: Option<String>,
}

pub struct Recorder {
    path: PathBuf,
}

impl Recorder {
    pub fn new(dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self {
            path: dir.join("decisions.jsonl"),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append-only, so a crash keeps what was written.
    pub fn write(&self, d: &Decision) -> Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(d)?)?;
        Ok(())
    }
}

/// Classifies a rejection by the mod's message (Sts2CoopDecisionEndpoint); keep the
/// two in sync.
pub fn classify_refusal(message: &str) -> &'static str {
    if message.contains("nothing is waiting") || message.contains("stale requestId") {
        "too_late"
    } else {
        "illegal_action"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tells_being_late_from_choosing_wrong() {
        assert_eq!(
            classify_refusal("stale requestId: waiting on 6d9d01d8"),
            "too_late"
        );
        assert_eq!(
            classify_refusal("nothing is waiting for a decision"),
            "too_late"
        );
        assert_eq!(
            classify_refusal("no such action right now: play_card_combat_14"),
            "illegal_action"
        );
    }
}
