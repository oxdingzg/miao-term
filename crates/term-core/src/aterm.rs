//! Experimental `alacritty_terminal` backend (architecture target, ADR 0001).
//!
//! Runs in parallel with the `vt100` backend for now; the app still uses
//! `vt100`. This exists to de-risk the eventual swap: the same operations the
//! app needs (cells, cursor, modes) are exercised here with tests.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;

struct Dims {
    cols: usize,
    rows: usize,
}

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// A terminal screen backed by `alacritty_terminal`.
pub struct ATerm {
    term: Term<VoidListener>,
    processor: Processor,
    cols: usize,
    rows: usize,
}

impl ATerm {
    pub fn new(cols: u16, rows: u16, scrollback: usize) -> Self {
        let dims = Dims {
            cols: cols as usize,
            rows: rows as usize,
        };
        let config = Config {
            scrolling_history: scrollback,
            ..Default::default()
        };
        Self {
            term: Term::new(config, &dims, VoidListener),
            processor: Processor::new(),
            cols: cols as usize,
            rows: rows as usize,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    pub fn size(&self) -> (u16, u16) {
        (self.rows as u16, self.cols as u16)
    }

    /// The character at a grid cell (space for empty cells).
    pub fn cell_char(&self, row: u16, col: u16) -> char {
        self.term.grid()[Line(row as i32)][Column(col as usize)].c
    }

    /// A row's text, right-trimmed.
    pub fn line_text(&self, row: u16) -> String {
        let text: String = (0..self.cols)
            .map(|col| self.cell_char(row, col as u16))
            .collect();
        text.trim_end().to_string()
    }

    pub fn cursor(&self) -> (u16, u16) {
        let point = self.term.grid().cursor.point;
        (point.line.0.max(0) as u16, point.column.0 as u16)
    }

    pub fn application_cursor(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_text() {
        let mut term = ATerm::new(20, 3, 100);
        term.process(b"hello");
        assert_eq!(term.line_text(0), "hello");
        assert_eq!(term.cursor(), (0, 5));
    }

    #[test]
    fn handles_newline_and_cursor() {
        let mut term = ATerm::new(20, 3, 100);
        term.process(b"one\r\ntwo");
        assert_eq!(term.line_text(0), "one");
        assert_eq!(term.line_text(1), "two");
        assert_eq!(term.cursor(), (1, 3));
    }

    #[test]
    fn tracks_modes() {
        let mut term = ATerm::new(20, 3, 100);
        term.process(b"\x1b[?1h"); // DECCKM (application cursor)
        assert!(term.application_cursor());
        term.process(b"\x1b[?2004h"); // bracketed paste
        assert!(term.bracketed_paste());
    }
}
