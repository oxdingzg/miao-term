//! `alacritty_terminal` backend (ADR 0001) — the screen model behind
//! [`crate::Terminal`]. Exposes the cells/colors/cursor/modes/scrollback the
//! app needs, plus tests.

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;

pub use alacritty_terminal::vte::ansi::{Color, NamedColor};

/// A snapshot of a grid cell (owned, so the caller doesn't borrow the term).
#[derive(Clone)]
pub struct CellView {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub inverse: bool,
    pub bold: bool,
}

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

    pub fn cursor_position(&self) -> (u16, u16) {
        self.cursor()
    }

    /// True when the cursor should not be drawn.
    pub fn hide_cursor(&self) -> bool {
        !self.term.mode().contains(TermMode::SHOW_CURSOR)
    }

    pub fn cell(&self, row: u16, col: u16) -> Option<CellView> {
        if row as usize >= self.rows || col as usize >= self.cols {
            return None;
        }
        let cell = &self.term.grid()[Line(row as i32)][Column(col as usize)];
        Some(CellView {
            ch: cell.c,
            fg: cell.fg,
            bg: cell.bg,
            inverse: cell.flags.contains(Flags::INVERSE),
            bold: cell.flags.contains(Flags::BOLD),
        })
    }

    /// Text between two cells (inclusive), right-trimmed per line.
    pub fn contents_between(&self, r1: u16, c1: u16, r2: u16, c2: u16) -> String {
        let mut out = String::new();
        for row in r1..=r2 {
            let (start, end) = if row == r1 {
                (c1, self.cols as u16)
            } else if row == r2 {
                (0, c2)
            } else {
                (0, self.cols as u16)
            };
            let mut line = String::new();
            for col in start..end.max(start) {
                if let Some(cell) = self.cell(row, col) {
                    line.push(cell.ch);
                }
            }
            out.push_str(line.trim_end());
            if row != r2 {
                out.push('\n');
            }
        }
        out
    }

    /// Current scrollback offset (lines scrolled up from the bottom).
    pub fn scroll_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Lines of history above the viewport (for a scrollbar).
    pub fn scrollback_len(&self) -> usize {
        self.term
            .grid()
            .total_lines()
            .saturating_sub(self.term.grid().screen_lines())
    }

    /// Scroll the viewport `n` lines back from the bottom.
    pub fn set_scrollback(&mut self, n: usize) {
        let current = self.term.grid().display_offset() as i32;
        self.term.scroll_display(Scroll::Delta(n as i32 - current));
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols as usize == self.cols && rows as usize == self.rows {
            return;
        }
        self.cols = cols as usize;
        self.rows = rows as usize;
        self.term.resize(Dims {
            cols: self.cols,
            rows: self.rows,
        });
    }

    pub fn application_cursor(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    pub fn bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// Kitty keyboard protocol: disambiguate escape codes (CSI-u).
    pub fn kitty_disambiguate(&self) -> bool {
        self.term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES)
    }

    /// Kitty keyboard protocol: report key event types (press/repeat/release).
    pub fn kitty_report_event_types(&self) -> bool {
        self.term.mode().contains(TermMode::REPORT_EVENT_TYPES)
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
