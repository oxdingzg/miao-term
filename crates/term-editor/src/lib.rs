//! Text editing core for mtty's editor pane (ADR 0034, phase E1).
//!
//! Everything here is GPU- and UI-free: a [`Document`] is a rope plus
//! multiple selections and an undo history, edited through
//! [`Transaction`]s. Positions are **char indices** into the rope, so they
//! line up with ropey's API; the renderer converts lines to cell columns with
//! [`layout`].

pub mod change;
pub mod document;
pub mod fallback;
pub mod history;
pub mod large;
pub mod layout;
pub mod motion;
pub mod search;
pub mod selection;
pub mod syntax;
pub mod text;

pub use change::{Assoc, ByteEdit, Change, Transaction};
pub use document::{Document, Motion};
pub use search::{SearchError, SearchQuery};
pub use selection::{Range, Selection};
pub use syntax::{set_parse_waker, Highlight, Syntax};
pub use text::{DecodeError, LineEnding};

pub use ropey::Rope;
