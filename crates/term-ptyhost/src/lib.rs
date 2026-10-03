//! `miao-term-ptyhost` — the per-pane PTY host (ADR 0041).
//!
//! A host owns one pane's PTY and program and outlives the app, so updating
//! or restarting mtty does not end what runs in its panes. The app is a
//! client: it attaches, replays output from an offset, and streams.
//!
//! The protocol, scanner and ring are platform-neutral; the host, client and
//! launcher are Unix-only for now (Windows is ADR 0041 phase P3).

pub mod proto;
pub mod ring;
pub mod scanner;

#[cfg(unix)]
pub mod client;
#[cfg(unix)]
pub mod host;
#[cfg(unix)]
pub mod launch;

/// Host and client version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
