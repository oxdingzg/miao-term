//! `miao-term-core` — the terminal core.
//!
//! Owns the hot path `pty → vte → grid → term` and nothing else: no GPU, no
//! windowing, no miaotty business logic.
//!
//! Planned modules (see the architecture doc):
//! - `pty`   spawn/write/resize (Unix + ConPTY)
//! - `vt`    `vte` processor → term mutations
//! - `term`  grid + scrollback + cursor + modes, selection, search, damage
//! - `osc`   title / cwd (7) / hyperlink (8) / progress (9;4) / shell-integration (133)
//! - `input` key/mouse → PTY bytes (incl. kitty keyboard / CSI u, bracketed paste)
//! - `event` [`EventSink`] for OSC-driven notifications to the host

pub mod aterm;
pub mod graphics;
pub mod perfgate;
mod shell;
mod term;

pub use term::Terminal;

/// The screen model the app renders, backed by `alacritty_terminal` (ADR 0001).
pub use aterm::ATerm;

/// Engine crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
