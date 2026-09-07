//! Pure types and rules for domux: no IO, no tokio, no ratatui, no process spawning.

/// This build's version. Every crate shares the workspace version, so a client and a server
/// built together agree and a stale one is refused at the hello.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod api;
pub mod config;
pub mod facts;
pub mod ids;
pub mod keymap;
pub mod model;
pub mod names;
pub mod paths;
pub mod proto;
pub mod state_file;
pub mod text;
