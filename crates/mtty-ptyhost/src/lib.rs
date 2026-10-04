//! `mtty-ptyhost` — the per-pane PTY host (ADR 0041).
//!
//! A host owns one pane's PTY and program and outlives the app, so updating
//! or restarting mtty does not end what runs in its panes. The app is a
//! client: it attaches, replays output from an offset, and streams.
//!
//! Platform differences live in [`sys`]; Windows hosts use ConPTY and
//! AF_UNIX sockets (ADR 0041 P3).

pub mod client;
pub mod host;
pub mod launch;
pub mod proto;
pub mod ring;
pub mod scanner;
pub mod sys;

/// Host and client version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
