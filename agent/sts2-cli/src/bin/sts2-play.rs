use std::process::ExitCode;

use clap::Parser;
use sts2_core::{SpireError, Sts2Client};

/// Plays a card from the AI teammate's hand, by name rather than position
/// (positions shift, and a shifted index still passes the game's checks).
#[derive(Parser)]
#[command(name = "sts2-play", about = "play a card from the AI teammate's hand")]
struct Args {
    /// Card name or id
    card: String,

    /// Target enemy name or id; omit for untargeted cards.
    #[arg(short, long)]
    target: Option<String>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let client = Sts2Client::from_env();

    // This read also yields the snapshot id, so a board change in between is
    // rejected rather than silently acted on.
    let state = match client.combat_state().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("sts2-play: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    if !state.in_combat {
        eprintln!("sts2-play: 전투 중이 아니다");
        return ExitCode::FAILURE;
    }
    if !state.can_act {
        eprintln!("sts2-play: 지금은 동료가 둘 차례가 아니다 (이미 턴을 끝냈다)");
        return ExitCode::FAILURE;
    }

    let Some(action) = state.find_action(&args.card, args.target.as_deref()) else {
        eprintln!("sts2-play: 지금 낼 수 있는 행동에 '{}' 이(가) 없다", args.card);
        let playable: Vec<&str> = state
            .hand
            .iter()
            .filter(|c| c.playable)
            .map(|c| c.name.as_str())
            .collect();
        if !playable.is_empty() {
            eprintln!("           낼 수 있는 것: {}", playable.join(", "));
        }
        return ExitCode::FAILURE;
    };

    match client.act(&action.action_id, &state.snapshot_id).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(SpireError::Refused(why)) => {
            eprintln!("sts2-play: {why}");
            ExitCode::FAILURE
        }
        Err(SpireError::Unreachable(e)) => {
            eprintln!("sts2-play: {e:#}");
            ExitCode::FAILURE
        }
    }
}
