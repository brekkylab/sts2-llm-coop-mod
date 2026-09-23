use std::process::ExitCode;

use sts2_core::{SpireError, Sts2Client};

#[tokio::main]
async fn main() -> ExitCode {
    let client = Sts2Client::from_env();

    let state = match client.combat_state().await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("sts2-end-turn: {e:#}");
            return ExitCode::FAILURE;
        }
    };

    let Some(action) = state.end_turn_action() else {
        eprintln!("sts2-end-turn: 지금 턴을 끝낼 수 없다");
        return ExitCode::FAILURE;
    };

    match client.act(&action.action_id, &state.snapshot_id).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(SpireError::Refused(why)) => {
            eprintln!("sts2-end-turn: {why}");
            ExitCode::FAILURE
        }
        Err(SpireError::Unreachable(e)) => {
            eprintln!("sts2-end-turn: {e:#}");
            ExitCode::FAILURE
        }
    }
}
