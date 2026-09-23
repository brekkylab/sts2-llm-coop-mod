use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use sts2_core::{render, said, CombatState};

/// The session directory, shared by the agent and the human. Files are replaced
/// atomically, one snapshot at a time.
pub struct Session {
    root: PathBuf,
}

impl Session {
    pub fn create(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        for sub in ["said", "turns", "notes"] {
            std::fs::create_dir_all(root.join(sub))
                .with_context(|| format!("creating {}", root.join(sub).display()))?;
        }
        let s = Self { root };
        s.ensure_notes()?;
        Ok(s)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Write to a temp file and rename: `fs::write` truncates first, so a reader
    /// (especially through FUSE) could see an empty file.
    fn put(&self, rel: &str, body: &str) -> Result<()> {
        let dest = self.root.join(rel);
        let tmp = dest.with_extension("tmp");
        std::fs::write(&tmp, body).with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &dest).with_context(|| format!("renaming into {}", dest.display()))?;
        Ok(())
    }

    pub fn brief_text(&self, s: &CombatState) -> String {
        render::brief(s)
    }

    /// Publishes one point in time as a single file.
    pub fn publish(&self, s: &CombatState) -> Result<()> {
        self.put("brief.md", &self.brief_text(s))
    }

    /// The prompt just sent, the only record of it. Outside `notes/` so the agent
    /// can't read its own prompts back.
    pub fn put_last_prompt(&self, body: &str) -> Result<()> {
        self.put("last_prompt.md", body)
    }

    /// The agent's own layer; the only directory mounted into its console.
    pub fn notes_dir(&self) -> std::path::PathBuf {
        self.root.join("notes")
    }

    pub fn learned_path(&self) -> std::path::PathBuf {
        self.notes_dir().join("learned.md")
    }

    /// Only if missing: `create()` runs on every bridge start.
    fn ensure_notes(&self) -> Result<()> {
        let p = self.learned_path();
        if p.exists() {
            return Ok(());
        }
        std::fs::write(&p, "# 동료에 대해 알게 된 것\n\n")
            .with_context(|| format!("creating {}", p.display()))
    }

    /// The human's instructions to the agent. Never written by the bridge or agent.
    pub fn memory_path(&self) -> std::path::PathBuf {
        self.root.join("memory.md")
    }

    /// Everything in `said/`. No offset parameter on purpose: the directory is
    /// cleared when a combat ends, so it only ever holds the current stretch.
    pub fn said_all(&self) -> Vec<(String, String)> {
        said::recent(&self.root, usize::MAX)
    }

    /// Partner utterances after the first `from`; same unit as `human_said_count()`.
    /// The agent's own lines are already in the history as Assistant messages.
    pub fn human_said_since(&self, from: usize) -> Vec<(String, String)> {
        said::recent(&self.root, usize::MAX)
            .into_iter()
            .filter(|(who, _)| who == "human")
            .skip(from)
            .collect()
    }

    /// The human side writes through `sts2-say` or the window.
    pub fn say(&self, who: &str, text: &str) -> Result<()> {
        said::say(&self.root, who, text)
    }

    /// Partner only: counting the agent too would make it reply to itself.
    pub fn human_said_count(&self) -> usize {
        said::count_by(&self.root, "human")
    }

    /// Clears the conversation when the run changes, keeps it across a bridge
    /// restart. A continued run keeps its seed; an empty seed changes nothing.
    /// (What was said is still in `decisions.jsonl`.)
    pub fn start_run(&self, seed: &str) -> Result<bool> {
        if seed.is_empty() {
            return Ok(false);
        }

        let marker = self.root.join("run_seed");
        if std::fs::read_to_string(&marker).is_ok_and(|p| p.trim() == seed) {
            return Ok(false);
        }

        self.clear_said()?;
        std::fs::write(&marker, format!("{seed}\n"))
            .with_context(|| format!("writing {}", marker.display()))?;
        Ok(true)
    }

    /// Clears the conversation. What should outlive a combat belongs in the notes;
    /// leftover instructions ("attack first!") otherwise leak into the next combat.
    pub fn end_combat(&self) -> Result<()> {
        self.clear_said()
    }

    fn clear_said(&self) -> Result<()> {
        let d = said::dir(&self.root);
        let Ok(rd) = std::fs::read_dir(&d) else {
            return Ok(());
        };
        for e in rd.flatten() {
            if e.path().extension().is_some_and(|x| x == "txt") {
                std::fs::remove_file(e.path())
                    .with_context(|| format!("removing {}", e.path().display()))?;
            }
        }
        Ok(())
    }

    /// The file wins, so a hand edit in the window becomes the prompt; falls back to
    /// rendering when there is no file (the replay tool).
    pub fn brief_for_prompt(&self, s: &CombatState) -> String {
        let brief = std::fs::read_to_string(self.root.join("brief.md"))
            .unwrap_or_else(|_| self.brief_text(s));

        // Appended here, not in brief.md, which the next publish overwrites.
        format!(
            "{brief}{}",
            crate::memory::read_layers(&self.memory_path(), &self.learned_path())
        )
    }

    pub fn last_said(&self) -> Option<(String, String)> {
        said::recent(&self.root, 1).into_iter().next()
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thread id too: nanoseconds alone collide between parallel tests.
    fn tmp_session() -> Session {
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tid = std::thread::current().id();
        Session::create(std::env::temp_dir().join(format!("sts2-brief-{ns}-{tid:?}"))).unwrap()
    }

    #[test]
    fn memory_rides_along_with_the_brief() {
        let s = tmp_session();
        let state = sample();
        s.publish(&state).unwrap();
        crate::memory::append(&s.memory_path(), "방어를 아낀다").unwrap();

        let p = s.brief_for_prompt(&state);
        assert!(p.contains("방어를 아낀다"), "{p}");
        assert!(p.contains("턴"), "브리핑 본문이 사라졌다");

        std::fs::remove_dir_all(s.root()).ok();
    }

    #[test]
    fn the_last_prompt_lands_outside_the_mount() {
        let s = tmp_session();
        s.put_last_prompt("### system\nRULES\n\n### user\n턴 3\n").unwrap();

        let p = s.root().join("last_prompt.md");
        assert!(p.exists(), "파일이 있어야 한다");
        assert!(!p.starts_with(s.notes_dir()), "notes/ 안이면 안 된다");
        assert!(std::fs::read_to_string(&p).unwrap().contains("턴 3"));

        std::fs::remove_dir_all(s.root()).ok();
    }

    /// A partner-only count must slice partner-only utterances.
    #[test]
    fn counting_the_partner_only_slices_the_partner_only() {
        let s = tmp_session();
        s.say("human", "방어 챙겨").unwrap();
        s.say("agent", "그럴게").unwrap();
        s.say("human", "이제 공격").unwrap();

        let mine = s.human_said_since(1);
        assert_eq!(mine.len(), 1, "{mine:?}");
        assert_eq!(mine[0].1, "이제 공격");

        assert_eq!(s.said_all().len(), 3);

        std::fs::remove_dir_all(s.root()).ok();
    }

    /// The conversation lives in the history; in the brief it would be sent twice.
    #[test]
    fn the_brief_no_longer_carries_the_talk() {
        let s = tmp_session();
        s.say("human", "방어 챙겨").unwrap();

        let brief = s.brief_text(&sample());
        assert!(!brief.contains("주고받은 말"), "{brief}");
        assert!(!brief.contains("방어 챙겨"), "{brief}");

        std::fs::remove_dir_all(s.root()).ok();
    }

    #[test]
    fn no_memory_adds_nothing() {
        let s = tmp_session();
        let state = sample();
        s.publish(&state).unwrap();

        let p = s.brief_for_prompt(&state);
        assert!(!p.contains("동료가 시킨 것"), "{p}");
        assert!(!p.contains("알게 된 것"), "{p}");
        std::fs::remove_dir_all(s.root()).ok();
    }

    fn sample() -> CombatState {
        serde_json::from_str(include_str!("../../sts2-core/src/sample_state.json")).unwrap()
    }

    #[test]
    fn a_new_run_starts_with_no_leftover_talk() {
        let s = tmp_session();
        s.say("human", "지난 런에서 한 말").unwrap();

        assert!(s.start_run("seed-a").unwrap(), "처음 보는 런이다");
        assert_eq!(s.human_said_count(), 0);

        std::fs::remove_dir_all(s.root()).ok();
    }

    #[test]
    fn a_finished_combat_leaves_no_talk_behind() {
        let s = tmp_session();
        s.say("human", "너가 지금 먼저 처치해").unwrap();
        crate::memory::append(&s.memory_path(), "딜을 우선한다").unwrap();
        assert_eq!(s.human_said_count(), 1);

        s.end_combat().unwrap();

        assert_eq!(s.human_said_count(), 0, "대화가 남았다");
        let state = sample();
        assert!(
            s.brief_for_prompt(&state).contains("딜을 우선한다"),
            "기억까지 지우면 안 된다"
        );

        std::fs::remove_dir_all(s.root()).ok();
    }

    #[test]
    fn the_same_run_keeps_its_talk() {
        let s = tmp_session();
        s.start_run("seed-a").unwrap();
        s.say("human", "이번 런에서 한 말").unwrap();

        assert!(!s.start_run("seed-a").unwrap(), "같은 런이다");
        assert_eq!(s.human_said_count(), 1);

        std::fs::remove_dir_all(s.root()).ok();
    }

    #[test]
    fn an_unknown_run_touches_nothing() {
        let s = tmp_session();
        s.say("human", "남아 있어야 한다").unwrap();

        assert!(!s.start_run("").unwrap());
        assert_eq!(s.human_said_count(), 1);

        std::fs::remove_dir_all(s.root()).ok();
    }

    #[test]
    fn a_hand_edited_brief_becomes_the_prompt() {
        let s = tmp_session();
        let state = sample();
        s.publish(&state).unwrap();

        std::fs::write(s.root().join("brief.md"), "사람이 고친 것\n").unwrap();
        assert_eq!(s.brief_for_prompt(&state), "사람이 고친 것\n");

        std::fs::remove_dir_all(s.root()).ok();
    }

    #[test]
    fn falls_back_when_there_is_no_file() {
        let s = tmp_session();
        let state = sample();
        assert_eq!(s.brief_for_prompt(&state), s.brief_text(&state));
        std::fs::remove_dir_all(s.root()).ok();
    }
}
