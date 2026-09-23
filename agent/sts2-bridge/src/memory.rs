//! What outlives a combat: notes about the partner only. Plain files rather than a
//! search index: the whole list rides along with every decision (a dozen lines),
//! and a human can read and fix a file.

use std::path::Path;

use anyhow::{Context as _, Result};

const HEADER: &str = "# 동료에 대해 알게 된 것";

/// Appends one bullet, creating the file with a header if needed.
pub fn append(path: &Path, line: &str) -> Result<()> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(());
    }

    let mut body = std::fs::read_to_string(path).unwrap_or_default();
    if body.is_empty() {
        body.push_str(HEADER);
        body.push_str("\n\n");
    }
    body.push_str("- ");
    body.push_str(line);
    body.push('\n');

    std::fs::write(path, body).with_context(|| format!("writing {}", path.display()))
}

/// One layer as a prompt section, empty if there is nothing. Never truncated: the
/// human's lines are instructions, and a cut agent layer would differ from what the
/// agent sees when summarizing. The summary rewrites it every combat instead.
fn section(path: &Path, title: &str) -> String {
    let Ok(body) = std::fs::read_to_string(path) else {
        return String::new();
    };

    let lines: Vec<&str> = body.lines().filter(|l| l.starts_with("- ")).collect();
    if lines.is_empty() {
        return String::new();
    }

    format!("\n## {title}\n\n{}\n", lines.join("\n"))
}

/// The human's instructions first, then the agent's observations. Kept apart
/// because in one list the agent overwrote instructions with contradicting
/// observations; the rules say the first section wins.
pub fn read_layers(mine: &Path, learned: &Path) -> String {
    format!(
        "{}{}",
        section(mine, "동료가 시킨 것"),
        section(learned, "동료에 대해 알게 된 것")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> std::path::PathBuf {
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        // Thread id too: nanoseconds alone collide between parallel tests.
        let tid = std::thread::current().id();
        let d = std::env::temp_dir().join(format!("sts2-mem-{ns}-{tid:?}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn writes_a_header_the_first_time() {
        let d = tmp();
        let p = d.join("memory.md");
        append(&p, "방어를 아낀다").unwrap();

        let s = std::fs::read_to_string(&p).unwrap();
        assert!(s.starts_with("# 동료에 대해 알게 된 것"), "{s}");
        assert!(s.contains("- 방어를 아낀다"), "{s}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn appends_without_repeating_the_header() {
        let d = tmp();
        let p = d.join("memory.md");
        append(&p, "첫째").unwrap();
        append(&p, "둘째").unwrap();

        let s = std::fs::read_to_string(&p).unwrap();
        assert_eq!(s.matches("# 동료에 대해").count(), 1, "{s}");
        assert!(s.contains("- 첫째") && s.contains("- 둘째"), "{s}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn reads_two_layers_as_two_sections() {
        let d = tmp();
        let (mine, learned) = (d.join("memory.md"), d.join("learned.md"));
        append(&mine, "오래 생각하는거 싫어함").unwrap();
        append(&learned, "코스트를 꼼꼼히 따진다").unwrap();

        let out = read_layers(&mine, &learned);
        let a = out.find("동료가 시킨 것").expect("사람 층");
        let b = out.find("알게 된 것").expect("에이전트 층");
        assert!(a < b, "사람 층이 먼저: {out}");

        std::fs::remove_dir_all(&d).ok();
    }

    /// An empty section would read as if something were there.
    #[test]
    fn skips_a_layer_that_is_empty() {
        let d = tmp();
        let mine = d.join("memory.md");
        append(&mine, "오래 생각하는거 싫어함").unwrap();

        let out = read_layers(&mine, &d.join("learned.md"));
        assert!(out.contains("동료가 시킨 것"), "{out}");
        assert!(!out.contains("알게 된 것"), "{out}");

        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn adds_nothing_when_both_are_empty() {
        let d = tmp();
        assert_eq!(read_layers(&d.join("a.md"), &d.join("b.md")), "");
    }

    #[test]
    fn keeps_every_line() {
        let d = tmp();
        let mine = d.join("memory.md");
        for i in 0..15 {
            append(&mine, &format!("줄 {i}")).unwrap();
        }

        let out = read_layers(&mine, &d.join("b.md"));
        assert_eq!(
            out.lines().filter(|l| l.starts_with("- ")).count(),
            15,
            "{out}"
        );
        assert!(out.contains("줄 0") && out.contains("줄 14"), "{out}");

        std::fs::remove_dir_all(&d).ok();
    }
}
