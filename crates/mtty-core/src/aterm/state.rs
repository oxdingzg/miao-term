//! Exact screen state, for reattaching to a program that kept running while
//! no screen was watching it (ADR 0041).
//!
//! A program that redraws with relative cursor moves (Ink/Claude Code, every
//! full-screen program) needs the screen it last drew on to be rebuilt cell
//! for cell, at the same size and with the cursor in the same place; a
//! reflowed copy would make its next frame paint over the wrong rows. So this
//! keeps cells, cursors and modes, not text.
//!
//! What `alacritty_terminal` 0.25 keeps private cannot be captured: the
//! scroll region, tab stops, the active charset (SO/SI), the title stack and
//! the depth of the kitty keyboard stack. Callers make the program redraw
//! after a restore (a resize), which sets those again.

use alacritty_terminal::grid::{Cursor, Dimensions};
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{CharsetIndex, StandardCharset};
use serde::{Deserialize, Serialize};

use super::ATerm;

/// A screen's contents, cursors and modes, serialisable.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenState {
    pub cols: u16,
    pub rows: u16,
    /// Main-screen history, oldest first.
    history: Vec<Row>,
    main: Screen,
    /// The alternate screen, when it is the active one.
    alt: Option<Screen>,
    /// `TermMode` bits.
    mode: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(super) struct Screen {
    rows: Vec<Row>,
    cursor: CursorState,
    saved_cursor: CursorState,
}

/// A row as runs of cells that share every attribute but the character.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Row(Vec<Run>);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Run {
    /// One character per cell.
    text: String,
    /// The cells' attributes (its own character is unused).
    cell: Cell,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CursorState {
    point: Point,
    template: Cell,
    /// G0..G3: true for the DEC line-drawing set.
    line_drawing: [bool; 4],
    input_needs_wrap: bool,
}

const CHARSETS: [CharsetIndex; 4] = [
    CharsetIndex::G0,
    CharsetIndex::G1,
    CharsetIndex::G2,
    CharsetIndex::G3,
];

impl CursorState {
    fn read(cursor: &Cursor<Cell>) -> Self {
        Self {
            point: cursor.point,
            template: cursor.template.clone(),
            line_drawing: CHARSETS
                .map(|i| cursor.charsets[i] == StandardCharset::SpecialCharacterAndLineDrawing),
            input_needs_wrap: cursor.input_needs_wrap,
        }
    }

    fn write(&self, cursor: &mut Cursor<Cell>) {
        cursor.point = self.point;
        cursor.template = self.template.clone();
        for (i, drawing) in CHARSETS.into_iter().zip(self.line_drawing) {
            cursor.charsets[i] = if drawing {
                StandardCharset::SpecialCharacterAndLineDrawing
            } else {
                StandardCharset::Ascii
            };
        }
        cursor.input_needs_wrap = self.input_needs_wrap;
    }
}

/// DEC private modes restored by sequence (`alacritty_terminal` has no
/// setter). The mouse protocols are exclusive and handled separately.
const PRIVATE_MODES: &[(TermMode, u16)] = &[
    (TermMode::APP_CURSOR, 1),
    (TermMode::ORIGIN, 6),
    (TermMode::LINE_WRAP, 7),
    (TermMode::SHOW_CURSOR, 25),
    (TermMode::FOCUS_IN_OUT, 1004),
    (TermMode::UTF8_MOUSE, 1005),
    (TermMode::SGR_MOUSE, 1006),
    (TermMode::ALTERNATE_SCROLL, 1007),
    (TermMode::URGENCY_HINTS, 1042),
    (TermMode::BRACKETED_PASTE, 2004),
];

/// The sequences that put a fresh screen into `mode` (the alternate screen
/// and the kitty keyboard flags excepted).
fn mode_sequences(mode: TermMode) -> String {
    let mut out = String::new();
    for &(flag, n) in PRIVATE_MODES {
        let set = if mode.contains(flag) { 'h' } else { 'l' };
        out.push_str(&format!("\x1b[?{n}{set}"));
    }
    out.push_str("\x1b[?1000l\x1b[?1002l\x1b[?1003l");
    for (flag, n) in [
        (TermMode::MOUSE_REPORT_CLICK, 1000),
        (TermMode::MOUSE_DRAG, 1002),
        (TermMode::MOUSE_MOTION, 1003),
    ] {
        if mode.contains(flag) {
            out.push_str(&format!("\x1b[?{n}h"));
        }
    }
    out.push_str(if mode.contains(TermMode::APP_KEYPAD) {
        "\x1b="
    } else {
        "\x1b>"
    });
    for (flag, n) in [(TermMode::INSERT, 4), (TermMode::LINE_FEED_NEW_LINE, 20)] {
        let set = if mode.contains(flag) { 'h' } else { 'l' };
        out.push_str(&format!("\x1b[{n}{set}"));
    }
    out
}

impl ATerm {
    /// Capture the screen exactly (ADR 0041), with at most `max_history`
    /// lines of history. Output held back by a synchronized update is applied
    /// first, so the state covers every byte processed so far. Nothing on the
    /// screen changes, including under a full-screen program.
    pub fn snapshot_state(&mut self, max_history: usize) -> ScreenState {
        self.flush_synchronized_output();
        let alt_active = self.term.mode().contains(TermMode::ALT_SCREEN);
        let alt = alt_active.then(|| self.read_screen());
        if alt_active {
            // To the main screen; this keeps the alternate one intact.
            self.term.swap_alt();
        }
        let main = self.read_screen();
        let grid = self.term.grid();
        let lines = grid.history_size().min(max_history) as i32;
        let history = (-lines..0).map(|l| self.read_row(Line(l))).collect();
        if let Some(alt) = &alt {
            // Back again, which clears the alternate screen: write it back.
            // Its cursor is restored too; the main screen's saved cursor is
            // set to its cursor as on the first switch, which it still equals.
            self.term.swap_alt();
            self.write_screen(alt);
        }
        ScreenState {
            cols: self.cols as u16,
            rows: self.rows as u16,
            history,
            main,
            alt,
            mode: self.term.mode().bits(),
        }
    }

    /// A screen rebuilt from `state` at its own size. It has sent no replies:
    /// restoring answers no query.
    pub fn restore_state(state: &ScreenState, scrollback: usize) -> Self {
        let mut term = ATerm::new(state.cols, state.rows, scrollback);
        let rows = state.rows as usize;
        let history = state.history.len().min(scrollback);
        if history > 0 {
            // Scroll blank lines into history, to be overwritten below.
            term.process("\n".repeat(rows - 1 + history).as_bytes());
        }
        let mode = TermMode::from_bits_truncate(state.mode);
        // Modes first: setting the origin mode moves the cursor.
        term.process(mode_sequences(mode).as_bytes());
        let skip = state.history.len() - history;
        for (i, row) in state.history[skip..].iter().enumerate() {
            term.write_row(Line(i as i32 - history as i32), row);
        }
        term.write_screen(&state.main);
        if let Some(alt) = &state.alt {
            term.process(b"\x1b[?1049h");
            term.write_screen(alt);
        }
        let kitty = (mode & TermMode::KITTY_KEYBOARD_PROTOCOL).bits() >> 18;
        if kitty != 0 {
            term.process(format!("\x1b[>{kitty}u").as_bytes());
        }
        term.take_responses(0, 0);
        term
    }

    pub(super) fn read_screen(&self) -> Screen {
        let grid = self.term.grid();
        Screen {
            rows: (0..self.rows as i32)
                .map(|l| self.read_row(Line(l)))
                .collect(),
            cursor: CursorState::read(&grid.cursor),
            saved_cursor: CursorState::read(&grid.saved_cursor),
        }
    }

    pub(super) fn write_screen(&mut self, screen: &Screen) {
        for (l, row) in screen.rows.iter().enumerate() {
            self.write_row(Line(l as i32), row);
        }
        let grid = self.term.grid_mut();
        screen.cursor.write(&mut grid.cursor);
        screen.saved_cursor.write(&mut grid.saved_cursor);
    }

    fn read_row(&self, line: Line) -> Row {
        let row = &self.term.grid()[line];
        let mut runs: Vec<Run> = Vec::new();
        for c in 0..self.term.grid().columns() {
            let cell = &row[Column(c)];
            match runs.last_mut() {
                Some(run)
                    if run.cell.fg == cell.fg
                        && run.cell.bg == cell.bg
                        && run.cell.flags == cell.flags
                        && run.cell.extra == cell.extra =>
                {
                    run.text.push(cell.c);
                }
                _ => runs.push(Run {
                    text: cell.c.to_string(),
                    cell: Cell {
                        c: ' ',
                        ..cell.clone()
                    },
                }),
            }
        }
        Row(runs)
    }

    fn write_row(&mut self, line: Line, row: &Row) {
        let cols = self.cols;
        let grid_row = &mut self.term.grid_mut()[line];
        let cells = row.0.iter().flat_map(|run| {
            run.text.chars().map(move |c| Cell {
                c,
                ..run.cell.clone()
            })
        });
        for (c, cell) in cells.take(cols).enumerate() {
            grid_row[Column(c)] = cell;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLS: u16 = 20;
    const ROWS: u16 = 6;
    const SCROLLBACK: usize = 50;

    /// Restoring a snapshot taken after `a` and then feeding `b` must give the
    /// same screen as feeding `a` and `b` without interruption; taking the
    /// snapshot must not disturb the original either.
    fn assert_equivalent(a: &[u8], b: &[u8]) {
        let mut whole = ATerm::new(COLS, ROWS, SCROLLBACK);
        whole.process(a);
        whole.process(b);
        let expected = whole.snapshot_state(usize::MAX);

        let mut original = ATerm::new(COLS, ROWS, SCROLLBACK);
        original.process(a);
        let state = original.snapshot_state(usize::MAX);
        let json = serde_json::to_string(&state).unwrap();
        let state: ScreenState = serde_json::from_str(&json).unwrap();

        let mut restored = ATerm::restore_state(&state, SCROLLBACK);
        restored.process(b);
        assert_eq!(restored.snapshot_state(usize::MAX), expected, "restored");

        original.process(b);
        assert_eq!(original.snapshot_state(usize::MAX), expected, "original");
    }

    #[test]
    fn shell_output_with_history_colours_wide_and_wrapped_lines() {
        let mut a = Vec::new();
        for i in 0..30 {
            a.extend(format!("\x1b[3{}mline {i}\x1b[0m\r\n", i % 8).as_bytes());
        }
        a.extend("目录 wide e\u{301} and a line long enough to wrap twice over\r\n".as_bytes());
        // An explicit id: generated ids come from a process-wide counter.
        a.extend(b"\x1b]8;id=x;https://example.com\x07link\x1b]8;;\x07 $ ");
        assert_equivalent(&a, b"ls\r\nmore\r\n\x1b[1mbold\x1b[0m $ ");
    }

    #[test]
    fn relative_redraws_land_on_the_same_rows() {
        // Ink-style: a live region redrawn by moving up from the cursor.
        let a = b"out 1\r\nout 2\r\n\x1b[?25l> spinner 1\r\n  status\r\n  hint";
        let b = b"\r\x1b[2A\x1b[2K> spinner 2\r\n\x1b[2K  status 2\r\n\x1b[2K  hint 2";
        assert_equivalent(a, b);
        // The pending-wrap state at the last column decides where `y` goes.
        assert_equivalent(&[b'x'; COLS as usize], b"y");
    }

    #[test]
    fn full_screen_program_and_its_modes() {
        let a = b"$ vim\r\n\x1b[?1049h\x1b[?1h\x1b=\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?1004h\
                  \x1b[H\x1b[2Jline one\r\nline two\x1b[2;3H";
        let b = b"X\x1b[3;1Hthree\x1b[?1049l\x1b[?1l\x1b>after vim";
        assert_equivalent(a, b);
        // Still inside: the next frame and the input modes carry on.
        assert_equivalent(a, b"\x1b[1;1HY");
    }

    #[test]
    fn saved_cursor_charsets_origin_and_insert() {
        assert_equivalent(b"ab\x1b[3;4H\x1b7\x1b[1;1H", b"\x1b8X");
        assert_equivalent(b"\x1b(0", b"qqqx\x1b(Bq");
        assert_equivalent(b"\x1b[?6h\x1b[4h\x1b[20h", b"ab\x1b[1;1Hc\nd");
        assert_equivalent(b"\x1b[?25l\x1b[?7l", b"0123456789012345678901234\x1b[?25h");
    }

    #[test]
    fn keyboard_and_mouse_protocols() {
        assert_equivalent(b"\x1b[>1u\x1b[?1003h\x1b[?1005h", b"\x1b[<u\x1b[?1003l");
        assert_equivalent(b"\x1b[?1000h", b"x");
    }

    #[test]
    fn a_synchronized_update_in_progress_is_kept() {
        assert_equivalent(b"before\r\n\x1b[?2026hpart", b"rest\x1b[?2026l done");
    }

    #[test]
    fn history_is_capped() {
        let mut term = ATerm::new(COLS, ROWS, SCROLLBACK);
        for i in 0..40 {
            term.process(format!("{i}\r\n").as_bytes());
        }
        let state = term.snapshot_state(10);
        let restored = ATerm::restore_state(&state, SCROLLBACK);
        assert_eq!(restored.history_size(), 10);
        assert_eq!(restored.line_text_abs(0), "25");
        let small = ATerm::restore_state(&term.snapshot_state(usize::MAX), 5);
        assert_eq!(small.history_size(), 5);
    }

    #[test]
    fn restoring_answers_no_query() {
        let mut term = ATerm::new(COLS, ROWS, SCROLLBACK);
        term.process(b"\x1b[6n\x1b[c");
        let restored = ATerm::restore_state(&term.snapshot_state(usize::MAX), SCROLLBACK);
        assert!(restored.take_responses(0, 0).is_empty());
    }
}
