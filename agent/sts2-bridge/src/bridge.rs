//! The loop that polls the game and carries decisions. Runs in the background
//! when the window is up.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use sts2_core::{CombatState, Sts2Client};
use tokio::sync::broadcast;
use tokio::sync::Mutex;

use sts2_bridge::agent::Decider;
use sts2_bridge::event::{AgentEvent, Events};
use sts2_bridge::session::Session;
use sts2_bridge::{console, record};

use crate::decision;

/// One directory per run, named by time (the bridge doesn't know the character).
fn run_result_dir() -> std::path::PathBuf {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    std::path::PathBuf::from(std::env::var("STS2_RUN_DIR").unwrap_or_else(|_| "run_result".into()))
        .join(format!("run-{secs}"))
}

/// `diskutil` does the work: FUSE-T sits on NFS, so `umount` reports "not currently
/// mounted". `umount` is still tried first, as in `run-bridge.sh`. A mount left
/// behind hangs anything that touches the path.
fn unmount(mount: &str) {
    let by_umount = std::process::Command::new("umount")
        .arg("-f")
        .arg(mount)
        .status()
        .is_ok_and(|s| s.success());
    if by_umount {
        return;
    }

    let ok = std::process::Command::new("diskutil")
        .args(["unmount", "force", mount])
        .status()
        .is_ok_and(|s| s.success());
    if !ok {
        eprintln!("  마운트를 못 풀었다 — 다음 실행의 run-bridge.sh 가 치운다");
    }
}

/// Consecutive failed polls before the game counts as gone (5 s at 200 ms).
const GAME_GONE_TICKS: usize = 25;

pub async fn run(events: Events) -> Result<()> {
    let root = std::env::var("STS2_SESSION").unwrap_or_else(|_| "/tmp/sts2-session".into());
    let session = Session::create(&root)?;
    let client = Sts2Client::from_env();
    println!("session at {}", session.root().display());


    let mut latest: Option<CombatState> = None;
    let mut last_published = String::new();
    let mut complained = false;

    // Set only when the mod started us; started by hand, the game may come later.
    let exit_with_game = std::env::var("STS2_EXIT_WITH_GAME").is_ok_and(|v| v == "1");
    let mut unreachable_ticks = 0usize;

    // One console (and FUSE-T mount) for the bridge's lifetime.
    let mount = std::env::var("STS2_MOUNT").unwrap_or_else(|_| "/tmp/sts2-mnt".into());
    let console = Arc::new(Mutex::new(Some(
        console::open(&session.notes_dir(), std::path::Path::new(&mount)).await?,
    )));
    // Initial values only: the mod applies budget and call cap itself, and the
    // Decider re-reads the language on every decision.
    let config = client.config().await;
    println!("대화 언어: {}", config.language);

    tokio::spawn(push_status_to_game(
        Sts2Client::from_env(),
        events.subscribe(),
    ));

    let decider = Decider::new(
        console.clone(),
        &config,
        events.clone(),
        Some(session.root().join("last_prompt.md")),
    );
    println!("모델: {}", decider.model());

    let recorder = record::Recorder::new(&run_result_dir())?;
    println!("기록: {}", recorder.path().display());
    let mut decisions =
        decision::DecisionLoop::new(recorder, decider, events.clone(), session.human_said_count());

    let mut ticker = tokio::time::interval(Duration::from_millis(200));

    loop {
        ticker.tick().await;
        match client.combat_state().await {
            Ok(state) => {
                complained = false;
                unreachable_ticks = 0;

                // Checked here, not at startup: the bridge may start before
                // the game, when the seed isn't known yet.
                match session.start_run(&state.run_seed) {
                    Ok(true) => println!("새 런({}) — 지난 대화를 치웠다", state.run_seed),
                    Ok(false) => {}
                    Err(e) => eprintln!("run: {e:#}"),
                }
                // Combat entry before decisions.tick(), or begin_combat
                // would overwrite the first decision's history after a
                // restart (latest starts as None). After start_run, so a new
                // run's stale talk isn't used.
                if !latest.as_ref().is_some_and(|l| l.in_combat) && state.in_combat {
                    decisions.begin_combat(&session.said_all());
                }

                // Don't rewrite an unchanged snapshot; the file view would
                // flicker.
                if state.snapshot_id != last_published
                    || state.in_combat != latest.as_ref().is_some_and(|l| l.in_combat)
                {
                    session.publish(&state)?;
                    last_published = state.snapshot_id.clone();
                    events.emit(AgentEvent::Published {
                        snapshot_id: state.snapshot_id.clone(),
                    });
                }

                // State first, then the question, so the state is the newer.
                if let Err(e) = decisions.tick(&client, &state, &session).await {
                    eprintln!("decision: {e:#}");
                }

                let left_combat =
                    latest.as_ref().is_some_and(|l| l.in_combat) && !state.in_combat;
                if left_combat {
                    // Same range the history saw, including the reward and
                    // map screens.
                    let said = session.said_all();
                    println!("전투가 끝났다 — 오간 말 {}마디를 돌아본다", said.len());

                    // Clear right after the snapshot, before the summary. The
                    // summary only uses `said`; clearing afterwards would
                    // silently delete whatever the partner typed while it ran
                    // (the reward screen, a natural time to talk). Those words
                    // now carry over into the next combat's history.
                    if let Err(e) = session.end_combat() {
                        eprintln!("said: {e:#}");
                    }
                    // The cursors can't detect a clear on their own.
                    decisions.forget_said();

                    // Backup before the summary rewrites the whole file. Kept
                    // outside notes/, which keeps it out of the agent's way but
                    // doesn't protect it (see console.rs). One slot only: a bad
                    // rewrite not noticed before the next combat is overwritten.
                    let learned = session.learned_path();
                    let before =
                        std::fs::read_to_string(&learned).unwrap_or_default();
                    if !before.is_empty() {
                        let _ = std::fs::write(
                            session.root().join("learned.md.bak"),
                            &before,
                        );
                    }

                    // A failed summary must not stop the game.
                    if let Err(e) = decisions.summarize_combat(&said).await {
                        eprintln!("memory: {e:#}");
                    }

                    // Diff before/after: the agent may just say it tidied up
                    // without calling a tool.
                    let after =
                        std::fs::read_to_string(&learned).unwrap_or_default();
                    let changed = before != after;
                    let lines =
                        after.lines().filter(|l| l.starts_with("- ")).count();
                    println!(
                        "  기억: {} ({lines}줄)",
                        if changed { "고쳐 썼다" } else { "손대지 않았다" }
                    );
                    events.emit(AgentEvent::Memory { changed, lines });

                    decisions.end_combat();
                }

                latest = Some(state);
            }
            Err(e) => {
                if !complained {
                    eprintln!("state: {e:#}");
                    complained = true;
                }

                // The poll doubles as the game's liveness check; a few misses
                // are normal (scene changes, heavy frames), 5 s isn't.
                unreachable_ticks += 1;
                if exit_with_game && unreachable_ticks >= GAME_GONE_TICKS {
                    println!("게임이 사라졌다 — 브리지도 끝낸다");

                    // Tell the window first so it can close itself.
                    events.emit(AgentEvent::Shutdown);

                    // Drop the console on a separate task, without waiting:
                    // doing it inline was seen to never return (cause not
                    // investigated). If it fails, the next run-bridge.sh
                    // cleans up.
                    let c = console.clone();
                    tokio::spawn(async move {
                        c.lock().await.take();
                    });

                    // Returning isn't enough: on macOS the GUI owns the main
                    // thread, so `main` lives as long as the window.
                    tokio::time::sleep(Duration::from_millis(500)).await;

                    // The kernel-side mount outlives the console process and
                    // hangs anything that touches it (an `ls` took 2 min).
                    unmount(&mount);

                    std::process::exit(0);
                }
            }
        }
    }
}

/// Reduces events to the two states the game draws (tool calls are noise there).
async fn push_status_to_game(client: Sts2Client, mut rx: broadcast::Receiver<AgentEvent>) {
    let mut last = String::new();
    loop {
        let e = match rx.recv().await {
            Ok(e) => e,
            // Only the latest state matters.
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        };

        let (state, bubble) = match e {
            AgentEvent::Thinking { .. } => ("thinking", None),
            AgentEvent::Chose { say, .. } => ("idle", say),
            AgentEvent::Said { who, text } if who == "agent" => ("idle", Some(text)),
            // Also clears the thought bubble after a failed decision.
            AgentEvent::Settled { .. } => ("idle", None),
            _ => continue,
        };

        // New text is sent even when the state is unchanged.
        if state == last && bubble.is_none() {
            continue;
        }
        last = state.to_string();

        if let Err(e) = client.push_status(state, bubble.as_deref()).await {
            eprintln!("status: {e}");
        }
    }
}
