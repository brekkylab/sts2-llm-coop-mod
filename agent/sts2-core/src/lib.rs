//! Talks to the mod's local server and renders its facts as prose ("it takes X to
//! kill this enemy and my hand has Y").

mod client;
mod dto;
pub mod josa;
pub mod render;
pub mod said;


pub use client::SpireError;
pub use client::Sts2Client;
pub use dto::*;
