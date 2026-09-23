//! Bridge internals shared by the binary and `sts2-replay`. Anything that must run
//! without a game lives here; the game-facing loop (`decision.rs`) stays in the
//! binary.

pub mod agent;
pub mod console;
pub mod event;
pub mod history;
pub mod memory;
pub mod record;
pub mod session;
