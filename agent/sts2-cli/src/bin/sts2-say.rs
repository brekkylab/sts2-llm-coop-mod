use std::process::ExitCode;

use clap::Parser;

/// Says something to the AI teammate by writing a file to the session directory;
/// the bridge picks it up, even while the game isn't asking for a decision.
#[derive(Parser)]
#[command(
    name = "sts2-say",
    about = "Say something to the AI teammate",
    long_about = "Writes to the session directory, bypassing the game server. \
                  The bridge picks it up and wakes the agent."
)]
struct Args {
    text: Vec<String>,

    /// Speaker; leave as `human`.
    #[arg(long, default_value = "human")]
    who: String,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let text = args.text.join(" ");
    if text.trim().is_empty() {
        eprintln!("sts2-say: 할 말이 비어 있다");
        return ExitCode::FAILURE;
    }

    // Must match the bridge's default.
    let root = std::env::var("STS2_SESSION").unwrap_or_else(|_| "/tmp/sts2-session".into());

    match sts2_core::said::say(std::path::Path::new(&root), &args.who, &text) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sts2-say: {e:#}");
            ExitCode::FAILURE
        }
    }
}
