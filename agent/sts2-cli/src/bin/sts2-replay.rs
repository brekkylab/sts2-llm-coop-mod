use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use sts2_bridge::agent::Decider;
use sts2_bridge::console;
use sts2_bridge::event::Events;
use sts2_core::CombatState;

/// Replays decisions against saved combat states, without the game. Use it to
/// check whether a prompt change made choices worse or less consistent.
#[derive(Parser)]
#[command(name = "sts2-replay", about = "Replay decisions against saved combat states")]
struct Args {
    /// Sample JSON files (from /state/combat), run in order.
    samples: Vec<PathBuf>,

    /// Runs per sample.
    #[arg(long, default_value_t = 3)]
    runs: usize,

    /// Model to compare against (defaults to STS2_MODEL).
    #[arg(long)]
    model: Option<String>,
}

#[derive(Debug, PartialEq)]
struct Spread {
    /// Distinct plans; 1 means consistent.
    distinct: usize,
    failures: usize,
    top: Option<String>,
    top_count: usize,
}

fn spread(picks: &[Option<String>]) -> Spread {
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut failures = 0;
    for p in picks {
        match p {
            Some(id) => *counts.entry(id.as_str()).or_default() += 1,
            None => failures += 1,
        }
    }
    // Ties break by name, so the same input gives the same report.
    let (top, top_count) = counts
        .iter()
        .max_by_key(|(id, n)| (**n, std::cmp::Reverse(**id)))
        .map(|(id, n)| (Some(id.to_string()), *n))
        .unwrap_or((None, 0));

    Spread {
        distinct: counts.len(),
        failures,
        top,
        top_count,
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();
    let args = Args::parse();
    if args.samples.is_empty() {
        eprintln!("sts2-replay: 표본을 하나 이상 주십시오");
        return ExitCode::FAILURE;
    }
    if let Some(m) = &args.model {
        // Decider reads STS2_MODEL.
        unsafe { std::env::set_var("STS2_MODEL", m) };
    }

    // A private session, so leftovers in the live directory can't leak into the
    // prompts and make results depend on hidden state.
    let ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let root = std::env::temp_dir().join(format!("sts2-replay-{ns}"));
    let mount = std::env::temp_dir().join(format!("sts2-replay-mnt-{ns}"));
    let session = match sts2_bridge::session::Session::create(&root) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("sts2-replay: 세션: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    let console = match console::open(&session.notes_dir(), &mount).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("sts2-replay: 콘솔: {e:#}");
            return ExitCode::FAILURE;
        }
    };
    // No game, so default settings.
    let mut decider = Decider::new(
        std::sync::Arc::new(tokio::sync::Mutex::new(Some(console))),
        &sts2_core::CoopConfig::default(),
        Events::new(),
        // No prompt log: every sample would overwrite it.
        None,
    );
    println!("모델: {}\n", decider.model());

    let mut bad = false;
    for path in &args.samples {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: {e}", path.display());
                bad = true;
                continue;
            }
        };
        let state: CombatState = match serde_json::from_str(&raw) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("{}: 파싱 실패: {e}", path.display());
                bad = true;
                continue;
            }
        };

        if let Err(e) = session.publish(&state) {
            eprintln!("{}: 공개 실패: {e:#}", path.display());
            bad = true;
            continue;
        }

        let mut picks = Vec::new();
        let mut lats = Vec::new();
        for _ in 0..args.runs {
            let t0 = std::time::Instant::now();
            // Fresh history each run, or later runs see earlier answers and the
            // spread means nothing.
            decider.end_combat();
            match decider
                .decide(
                    &session.brief_text(&state),
                    &state.legal_actions,
                    &state,
                    &[],
                )
                .await
            {
                Ok(c) => {
                    // Count whole plans, not just first moves.
                    let plan = c.action_ids.join(" → ");
                    println!("  {} · {}", plan, c.say.as_deref().unwrap_or("-"));
                    picks.push(Some(plan));
                }
                Err(e) => {
                    println!("  실패: {e:#}");
                    picks.push(None);
                }
            }
            lats.push(t0.elapsed().as_millis());
        }

        let s = spread(&picks);
        lats.sort_unstable();
        let med = lats[lats.len() / 2];
        println!(
            "{}: 갈래 {} · 실패 {} · 최다 {}({}/{}) · 지연 중앙값 {med}ms\n",
            path.file_name().unwrap_or_default().to_string_lossy(),
            s.distinct,
            s.failures,
            s.top.as_deref().unwrap_or("-"),
            s.top_count,
            args.runs,
        );
        if s.failures > 0 {
            bad = true;
        }
    }

    // Unmount before deleting the directory FUSE still holds.
    drop(decider);
    let _ = std::process::Command::new("umount").arg(&mount).status();
    let _ = std::fs::remove_dir_all(&root);
    let _ = std::fs::remove_dir_all(&mount);

    if bad {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_a_unanimous_run() {
        let s = spread(&[
            Some("a".into()),
            Some("a".into()),
            Some("a".into()),
            Some("a".into()),
            Some("a".into()),
        ]);
        assert_eq!(s.distinct, 1);
        assert_eq!(s.failures, 0);
        assert_eq!(s.top.as_deref(), Some("a"));
        assert_eq!(s.top_count, 5);
    }

    #[test]
    fn counts_a_split_run() {
        let s = spread(&[
            Some("a".into()),
            Some("b".into()),
            Some("a".into()),
            None,
            Some("c".into()),
        ]);
        assert_eq!(s.distinct, 3);
        assert_eq!(s.failures, 1);
        assert_eq!(s.top.as_deref(), Some("a"));
        assert_eq!(s.top_count, 2);
    }

    /// Must not divide by zero.
    #[test]
    fn survives_an_all_failed_run() {
        let s = spread(&[None, None]);
        assert_eq!(s.distinct, 0);
        assert_eq!(s.failures, 2);
        assert!(s.top.is_none());
    }
}
