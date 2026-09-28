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

mod term;

pub use term::Terminal;

/// Re-exported so the app can reach the screen model without naming the parser
/// crate directly (R1 swaps the backend behind this).
pub use vt100;

/// Engine crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
