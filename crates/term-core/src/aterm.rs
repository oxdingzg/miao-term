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
    pub wide_spacer: bool,
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

    /// The character at a viewport cell (space for empty cells).
    pub fn cell_char(&self, row: u16, col: u16) -> char {
        let off = self.term.grid().display_offset() as i32;
        self.term.grid()[Line(row as i32 - off)][Column(col as usize)].c
    }

    /// A row's text, right-trimmed.
    pub fn line_text(&self, row: u16) -> String {
        let text: String = (0..self.cols)
            .map(|col| self.cell_char(row, col as u16))
            .collect();
        text.trim_end().to_string()
    }

    /// Total buffer lines (scrollback + screen).
    pub fn total_lines(&self) -> usize {
        self.term.grid().total_lines()
    }

    /// Lines of history above the viewport.
    pub fn history_size(&self) -> usize {
        self.total_lines().saturating_sub(self.rows)
    }

    /// Text of buffer line `b` counted from the oldest (0), right-trimmed.
    /// Wide-character spacer cells are skipped, so CJK reads as typed.
    pub fn line_text_abs(&self, b: usize) -> String {
        let text: String = self
            .line_chars_abs(b)
            .into_iter()
            .map(|(_, c, _)| c)
            .collect();
        text.trim_end().to_string()
    }

    /// Characters of buffer line `b` with their starting column and cell
    /// width (2 for wide characters); spacer cells are skipped.
    pub fn line_chars_abs(&self, b: usize) -> Vec<(u16, char, u16)> {
        let line = b as i32 - self.history_size() as i32;
        let row = &self.term.grid()[Line(line)];
        (0..self.cols)
            .filter_map(|c| {
                let cell = &row[Column(c)];
                if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    return None;
                }
                let width = if cell.flags.contains(Flags::WIDE_CHAR) {
                    2
                } else {
                    1
                };
                Some((c as u16, cell.c, width))
            })
            .collect()
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
        let off = self.term.grid().display_offset() as i32;
        let cell = &self.term.grid()[Line(row as i32 - off)][Column(col as usize)];
        Some(CellView {
            ch: cell.c,
            fg: cell.fg,
            bg: cell.bg,
            inverse: cell.flags.contains(Flags::INVERSE),
            bold: cell.flags.contains(Flags::BOLD),
            wide_spacer: cell.flags.contains(Flags::WIDE_CHAR_SPACER),
        })
    }

    /// Text between two cells (inclusive), right-trimmed per line.
    pub fn contents_between(&self, r1: u16, c1: u16, r2: u16, c2: u16) -> String {
        let mut out = String::new();
        for row in r1..=r2 {
            let start = if row == r1 { c1 } else { 0 };
            let end = if row == r2 {
                c2.saturating_add(1).min(self.cols as u16)
            } else {
                self.cols as u16
            };
            let mut line = String::new();
            for col in start..end.max(start) {
                if let Some(cell) = self.cell(row, col) {
                    if !cell.wide_spacer {
                        line.push(cell.ch);
                    }
                }
            }
            out.push_str(line.trim_end());
            if row != r2 {
                out.push('\n');
            }
        }
        out
    }

    /// Selection text with SGR colour codes (for "Copy as ANSI Sequence").
    pub fn contents_ansi_between(&self, r1: u16, c1: u16, r2: u16, c2: u16) -> String {
        let mut out = String::new();
        for row in r1..=r2 {
            let start = if row == r1 { c1 } else { 0 };
            let end = if row == r2 {
                c2.saturating_add(1).min(self.cols as u16)
            } else {
                self.cols as u16
            };
            let mut line = String::new();
            let (mut cur_fg, mut cur_bg) = (None, None);
            for col in start..end.max(start) {
                let Some(cell) = self.cell(row, col) else {
                    continue;
                };
                if cell.wide_spacer {
                    continue;
                }
                if cell.fg != Color::Named(NamedColor::Foreground) && Some(cell.fg) != cur_fg {
                    line.push_str(&sgr(cell.fg, false));
                    cur_fg = Some(cell.fg);
                }
                if cell.bg != Color::Named(NamedColor::Background) && Some(cell.bg) != cur_bg {
                    line.push_str(&sgr(cell.bg, true));
                    cur_bg = Some(cell.bg);
                }
                line.push(cell.ch);
            }
            if cur_fg.is_some() || cur_bg.is_some() {
                line.push_str("\x1b[0m");
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

    /// A full-screen program (vim, less, man) is on the alternate screen and
    /// alternate scroll (DECSET 1007, on by default) is set: the wheel should
    /// send it arrow keys rather than scroll a scrollback it does not have.
    pub fn alternate_scroll(&self) -> bool {
        self.term
            .mode()
            .contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL)
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

    /// Mouse reporting mode the application asked for, if any.
    ///
    /// Returns `(report_clicks, report_motion, report_drag, sgr_encoding)`.
    pub fn mouse_reporting(&self) -> Option<(bool, bool, bool, bool)> {
        let mode = self.term.mode();
        if !mode.intersects(TermMode::MOUSE_MODE) {
            return None;
        }
        Some((
            // Drag and all-motion protocols include button presses/releases;
            // Alacritty stores these protocols as mutually exclusive bits.
            mode.intersects(TermMode::MOUSE_MODE),
            mode.contains(TermMode::MOUSE_MOTION),
            mode.contains(TermMode::MOUSE_DRAG),
            mode.contains(TermMode::SGR_MOUSE),
        ))
    }
}

/// An SGR fragment for a colour (`bg` selects 48/49 instead of 38/39).
fn sgr(c: Color, bg: bool) -> String {
    match c {
        Color::Named(NamedColor::Foreground) => "\x1b[39m".into(),
        Color::Named(NamedColor::Background) => "\x1b[49m".into(),
        Color::Named(n) if (n as usize) < 16 => {
            let base = if bg { 40 } else { 30 };
            let idx = n as usize;
            // Bright colours use the 90/100 range.
            let code = if idx >= 8 {
                base + 60 + (idx - 8)
            } else {
                base + idx
            };
            format!("\x1b[{code}m")
        }
        Color::Indexed(i) => {
            let sel = if bg { 48 } else { 38 };
            format!("\x1b[{sel};5;{i}m")
        }
        Color::Spec(rgb) => {
            let sel = if bg { 48 } else { 38 };
            format!("\x1b[{sel};2;{};{};{}m", rgb.r, rgb.g, rgb.b)
        }
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wheel_goes_to_full_screen_programs_on_the_alternate_screen() {
        let mut term = ATerm::new(20, 3, 100);
        assert!(!term.alternate_scroll(), "the main screen scrolls back");
        term.process(b"\x1b[?1049h");
        assert!(term.alternate_scroll(), "vim/less: on by default");
        term.process(b"\x1b[?1007l");
        assert!(!term.alternate_scroll(), "a program can turn it off");
        term.process(b"\x1b[?1007h\x1b[?1049l");
        assert!(!term.alternate_scroll(), "back on the main screen");
    }

    #[test]
    fn mouse_tracking_protocols_include_clicks() {
        for (protocol, motion, drag) in [
            (1000, false, false),
            (1002, false, true),
            (1003, true, false),
        ] {
            let mut term = ATerm::new(20, 3, 100);
            assert_eq!(term.mouse_reporting(), None);
            term.process(format!("\x1b[?{protocol}h\x1b[?1006h").as_bytes());
            assert_eq!(term.mouse_reporting(), Some((true, motion, drag, true)));
            term.process(format!("\x1b[?{protocol}l").as_bytes());
            assert_eq!(term.mouse_reporting(), None);
        }
    }

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
    fn scrollback_shifts_viewport() {
        let mut t = ATerm::new(10, 3, 100);
        t.process(b"a\r\nb\r\nc\r\nd\r\ne");
        assert_eq!(t.line_text(0), "c");
        assert_eq!(t.line_text(2), "e");
        assert_eq!(t.total_lines(), 5);
        assert_eq!(t.history_size(), 2);
        assert_eq!(t.line_text_abs(0), "a");
        assert_eq!(t.line_text_abs(4), "e");
        t.set_scrollback(2);
        assert_eq!(t.line_text(0), "a");
        assert_eq!(t.line_text(2), "c");
    }

    #[test]
    fn absolute_line_text_skips_wide_spacers() {
        let mut t = ATerm::new(20, 3, 100);
        t.process("ab目录c".as_bytes());
        assert_eq!(t.line_text_abs(0), "ab目录c");
        let cells = t.line_chars_abs(0);
        assert_eq!(
            &cells[..5],
            &[
                (0, 'a', 1),
                (1, 'b', 1),
                (2, '目', 2),
                (4, '录', 2),
                (6, 'c', 1)
            ]
        );
    }

    #[test]
    fn ansi_copy_carries_colours() {
        let mut t = ATerm::new(10, 2, 10);
        t.process(b"\x1b[31mred\x1b[0m");
        let s = t.contents_ansi_between(0, 0, 0, 2);
        assert!(s.contains("\x1b[31m"), "{s:?}");
        assert!(s.contains("red"), "{s:?}");
    }

    #[test]
    fn tracks_modes() {
        let mut term = ATerm::new(20, 3, 100);
        term.process(b"\x1b[?1h"); // DECCKM (application cursor)
        assert!(term.application_cursor());
        term.process(b"\x1b[?2004h"); // bracketed paste
        assert!(term.bracketed_paste());
    }

    #[test]
    fn copying_stops_at_inclusive_selection_end() {
        let mut term = ATerm::new(30, 3, 100);
        term.process(b"prefix selected suffix\r\nsecond line");
        assert_eq!(term.contents_between(0, 7, 0, 14), "selected");
        assert_eq!(term.contents_between(0, 7, 1, 5), "selected suffix\nsecond");
        assert_eq!(term.contents_between(0, 7, 0, 7), "s");
        assert_eq!(term.contents_ansi_between(0, 7, 0, 14), "selected");
    }

    #[test]
    fn copying_cjk_does_not_insert_spacer_cells() {
        let mut term = ATerm::new(30, 3, 100);
        term.process("目录/file.txt suffix".as_bytes());
        assert_eq!(term.contents_between(0, 0, 0, 12), "目录/file.txt");
        assert_eq!(term.contents_ansi_between(0, 0, 0, 12), "目录/file.txt");
    }
}
