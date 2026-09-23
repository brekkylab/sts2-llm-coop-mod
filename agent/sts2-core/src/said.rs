//! The conversation, one file per utterance (shared by the bridge and the CLI).
//! Separate files let readers detect new speech by counting, and the epoch-millis
//! prefix makes lexical order chronological.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

pub fn dir(session_root: &Path) -> PathBuf {
    session_root.join("said")
}

/// `who` is `human` or `agent`.
pub fn say(session_root: &Path, who: &str, text: &str) -> Result<()> {
    let d = dir(session_root);
    std::fs::create_dir_all(&d).with_context(|| format!("creating {}", d.display()))?;

    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_millis())
        .unwrap_or(0);
    let dest = d.join(format!("{ms:013}-{who}.txt"));
    std::fs::write(&dest, format!("{}\n", text.trim()))
        .with_context(|| format!("writing {}", dest.display()))?;
    Ok(())
}

/// Both speakers.
pub fn count(session_root: &Path) -> usize {
    files(session_root).len()
}

/// Used to detect new partner speech; counting both speakers would make the agent
/// answer itself.
pub fn count_by(session_root: &Path, who: &str) -> usize {
    let suffix = format!("-{who}.txt");
    files(session_root)
        .iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with(&suffix))
        })
        .count()
}

/// The last `n`, oldest first, as `(who, text)`.
pub fn recent(session_root: &Path, n: usize) -> Vec<(String, String)> {
    let fs_ = files(session_root);
    let start = fs_.len().saturating_sub(n);
    fs_[start..]
        .iter()
        .map(|f| {
            let who = f
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.split_once('-'))
                .map(|(_, who)| who.to_string())
                .unwrap_or_else(|| "?".into());
            let text = std::fs::read_to_string(f).unwrap_or_default();
            (who, text.trim().to_string())
        })
        .collect()
}

/// Sorted, i.e. chronological.
fn files(session_root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = match std::fs::read_dir(dir(session_root)) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "txt"))
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let d = std::env::temp_dir().join(format!("sts2-said-{ns}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn counts_and_orders_what_was_said() {
        let root = tmp();
        assert_eq!(count(&root), 0);

        say(&root, "agent", "14밖에 안 나와").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        say(&root, "human", "나 12 있어").unwrap();

        assert_eq!(count(&root), 2);
        let r = recent(&root, 6);
        assert_eq!(r[0], ("agent".into(), "14밖에 안 나와".into()));
        assert_eq!(r[1], ("human".into(), "나 12 있어".into()));

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn counts_only_one_speaker() {
        let root = tmp();
        say(&root, "agent", "이번 턴은 넘길게").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        say(&root, "agent", "손패가 비었어").unwrap();

        assert_eq!(count(&root), 2);
        assert_eq!(count_by(&root, "human"), 0, "사람은 말한 적이 없다");
        assert_eq!(count_by(&root, "agent"), 2);

        std::fs::remove_dir_all(&root).ok();
    }

}
