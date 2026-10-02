//! The editor pane (ADR 0034, phase E2): a `term-editor` document shown on the
//! same monospace cell grid as a terminal and drawn by the same renderer.
//!
//! Everything here works in cells (rows and columns of the pane's text area)
//! and is free of window and GPU code, so it is unit-tested; the host turns
//! the cell quads into pixels and feeds the spans to `TermRenderer`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use miao_term_editor::{layout, motion, Document, Motion, Range, Selection};
use miao_term_render::Span;
use miao_term_ui::input::KeyKind;

/// Files larger than this open read-only in the floating viewer instead (the
/// pane would cope, but E2 has no large-file mode yet).
pub const MAX_PANE_BYTES: u64 = 64 << 20;

/// A rectangle on the cell grid, in text-area coordinates (the gutter is at
/// negative columns from the host's point of view, see [`EditorPane::draw`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellRect {
    pub row: usize,
    pub col: usize,
    pub width: usize,
}

/// What the host should paint for this pane, all in cells of the pane.
pub struct EditorDraw {
    /// Glyph rows for `TermRenderer`; columns include the gutter.
    pub rows: Vec<Vec<Span>>,
    /// Selected cells.
    pub selection: Vec<CellRect>,
    /// The current line's band (behind the selection).
    pub current_line: Option<usize>,
    /// Where carets go (row, col including the gutter); a caret is a bar.
    pub carets: Vec<(usize, usize)>,
    /// Width of the gutter in cells.
    pub gutter: usize,
}

/// Colours the pane draws with (taken from the terminal theme).
#[derive(Clone, Copy)]
pub struct Palette {
    pub fg: (u8, u8, u8),
    pub gutter: (u8, u8, u8),
    pub gutter_current: (u8, u8, u8),
}

/// A command a key press maps to. Kept separate from applying it so the
/// keymap is tested on its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Move(Motion, bool),
    Backspace,
    Delete,
    DeleteWordBackward,
    DeleteToLineStart,
    Newline,
    Indent,
    Outdent,
    SelectAll,
    SelectLine,
    SelectNextOccurrence,
    AddCursor(bool),
    Undo,
    Redo,
    Escape,
}

/// The macOS text conventions (ADR 0034): ⌥ moves by word, ⌘ by line or
/// document, ⇧ extends. On Linux/Windows Ctrl plays ⌥'s word role and Home/End
/// cover line moves. Returns `None` for keys the pane leaves to text input or
/// to the app (⌘S, ⌘W, …).
pub fn keymap(key: KeyKind, shift: bool, alt: bool, cmd: bool, ctrl: bool) -> Option<Command> {
    let mac = cfg!(target_os = "macos");
    // The "word" modifier and the "line/document" modifier per platform.
    let word = if mac { alt } else { ctrl };
    let line = mac && cmd;
    let primary = if mac { cmd } else { ctrl };
    let page = 20;
    Some(match key {
        KeyKind::Left if line => Command::Move(Motion::LineStart, shift),
        KeyKind::Right if line => Command::Move(Motion::LineEnd, shift),
        KeyKind::Up if line => Command::Move(Motion::DocStart, shift),
        KeyKind::Down if line => Command::Move(Motion::DocEnd, shift),
        KeyKind::Up if mac && alt && cmd => Command::AddCursor(false),
        KeyKind::Down if mac && alt && cmd => Command::AddCursor(true),
        KeyKind::Left if word => Command::Move(Motion::WordLeft, shift),
        KeyKind::Right if word => Command::Move(Motion::WordRight, shift),
        KeyKind::Left => Command::Move(Motion::Left, shift),
        KeyKind::Right => Command::Move(Motion::Right, shift),
        KeyKind::Up => Command::Move(Motion::Up, shift),
        KeyKind::Down => Command::Move(Motion::Down, shift),
        KeyKind::Home if ctrl && !mac => Command::Move(Motion::DocStart, shift),
        KeyKind::End if ctrl && !mac => Command::Move(Motion::DocEnd, shift),
        KeyKind::Home => Command::Move(Motion::LineStart, shift),
        KeyKind::End => Command::Move(Motion::LineEnd, shift),
        KeyKind::PageUp => Command::Move(Motion::PageUp(page), shift),
        KeyKind::PageDown => Command::Move(Motion::PageDown(page), shift),
        KeyKind::Backspace if line => Command::DeleteToLineStart,
        KeyKind::Backspace if word => Command::DeleteWordBackward,
        KeyKind::Backspace => Command::Backspace,
        KeyKind::Delete => Command::Delete,
        KeyKind::Enter if !primary => Command::Newline,
        KeyKind::Tab if shift => Command::Outdent,
        KeyKind::Tab if !primary => Command::Indent,
        KeyKind::Escape => Command::Escape,
        KeyKind::Char(c) if primary && !alt => match c.to_ascii_lowercase() {
            'a' => Command::SelectAll,
            'l' => Command::SelectLine,
            'd' => Command::SelectNextOccurrence,
            'z' if shift => Command::Redo,
            'z' => Command::Undo,
            'y' if !mac => Command::Redo,
            _ => return None,
        },
        _ => return None,
    })
}

/// A file open in an editor pane.
pub struct EditorPane {
    pub id: String,
    pub doc: Document,
    pub path: PathBuf,
    /// First visible line and cell column of the text area.
    pub scroll_line: usize,
    pub scroll_col: usize,
    /// Text-area size in cells, from the last layout.
    pub rows: usize,
    pub cols: usize,
    /// A drag selection is in progress (the anchor stays put).
    pub dragging: bool,
    /// Closing with unsaved changes was asked once; the next close discards.
    pub close_armed: bool,
    last_click: Option<(Instant, usize, u8)>,
}

impl EditorPane {
    /// Open `path` (UTF-8 text) in a pane.
    pub fn open(id: String, path: &Path) -> Result<Self, String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if meta.len() > MAX_PANE_BYTES {
            return Err(format!(
                "{} MB is too large for the editor pane",
                meta.len() >> 20
            ));
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let doc = Document::from_bytes(&bytes).map_err(|e| e.to_string())?;
        Ok(Self::with_doc(id, path.to_path_buf(), doc))
    }

    pub fn with_doc(id: String, path: PathBuf, doc: Document) -> Self {
        EditorPane {
            id,
            doc,
            path,
            scroll_line: 0,
            scroll_col: 0,
            rows: 1,
            cols: 1,
            dragging: false,
            close_armed: false,
            last_click: None,
        }
    }

    /// Write the document back. Goes through a temporary file in the same
    /// directory and a rename, so a failed write never truncates the file;
    /// the file's permissions are kept.
    pub fn save(&mut self) -> Result<(), String> {
        let bytes = self.doc.to_bytes();
        let dir = self
            .path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".into());
        let tmp = dir.join(format!(".{name}.mtty-save"));
        let result = (|| -> std::io::Result<()> {
            std::fs::write(&tmp, &bytes)?;
            if let Ok(meta) = std::fs::metadata(&self.path) {
                std::fs::set_permissions(&tmp, meta.permissions())?;
            }
            std::fs::rename(&tmp, &self.path)
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(e.to_string());
        }
        self.doc.mark_saved();
        self.close_armed = false;
        Ok(())
    }

    /// The file name, with `●` while there are unsaved changes.
    pub fn title(&self) -> String {
        let name = self
            .path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.display().to_string());
        if self.doc.is_modified() {
            format!("{name} \u{25cf}")
        } else {
            name
        }
    }

    /// Cells taken by line numbers: the widest number plus a space each side.
    pub fn gutter(&self) -> usize {
        let lines = self.doc.rope().len_lines().max(1);
        lines.to_string().len() + 2
    }

    /// Fit to a pane of `cols` × `rows` cells (gutter included).
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols.saturating_sub(self.gutter()).max(1);
        self.rows = rows.max(1);
    }

    /// Scroll so the primary caret is on screen.
    pub fn reveal_cursor(&mut self) {
        let rope = self.doc.rope();
        let head = self.doc.selection().primary().head;
        let line = rope.char_to_line(head);
        if line < self.scroll_line {
            self.scroll_line = line;
        } else if line >= self.scroll_line + self.rows {
            self.scroll_line = line + 1 - self.rows;
        }
        let col = layout::visual_col(
            rope.line(line),
            head - rope.line_to_char(line),
            self.doc.tab_width(),
        );
        if col < self.scroll_col {
            self.scroll_col = col;
        } else if col >= self.scroll_col + self.cols {
            self.scroll_col = col + 1 - self.cols;
        }
    }

    /// Scroll by `lines` (wheel), clamped to the document.
    pub fn scroll_by(&mut self, lines: isize) {
        let last = motion::last_line(self.doc.rope());
        let next = (self.scroll_line as isize + lines).clamp(0, last as isize);
        self.scroll_line = next as usize;
    }

    /// Run a command; returns true when the text or selection changed.
    pub fn run(&mut self, command: Command) -> bool {
        let before = (self.doc.revision(), self.doc.selection().clone());
        match command {
            Command::Move(m, extend) => {
                let m = match m {
                    Motion::PageUp(_) => Motion::PageUp(self.rows.max(2) - 1),
                    Motion::PageDown(_) => Motion::PageDown(self.rows.max(2) - 1),
                    other => other,
                };
                self.doc.move_cursor(m, extend);
            }
            Command::Backspace => self.doc.delete_backward(),
            Command::Delete => self.doc.delete_forward(),
            Command::DeleteWordBackward => self.doc.delete_word_backward(),
            Command::DeleteToLineStart => self.doc.delete_to_line_start(),
            Command::Newline => self.doc.newline(),
            Command::Indent => self.doc.indent(),
            Command::Outdent => self.doc.outdent(),
            Command::SelectAll => self.doc.select_all(),
            Command::SelectLine => self.doc.select_line(),
            Command::SelectNextOccurrence => {
                self.doc.select_next_occurrence();
            }
            Command::AddCursor(below) => self.doc.add_cursor(below),
            Command::Undo => {
                self.doc.undo();
            }
            Command::Redo => {
                self.doc.redo();
            }
            Command::Escape => {
                let sel = self.doc.selection().clone();
                if sel.len() > 1 {
                    self.doc.collapse_to_primary();
                } else {
                    let head = sel.primary().head;
                    self.doc.set_selection(Selection::cursor(head));
                }
            }
        }
        self.reveal_cursor();
        (self.doc.revision(), self.doc.selection().clone()) != before
    }

    /// Typed text (a key's text or an IME commit).
    pub fn type_text(&mut self, text: &str) {
        self.doc.type_text(text);
        self.reveal_cursor();
    }

    pub fn paste(&mut self, text: &str) {
        self.doc.paste(text);
        self.reveal_cursor();
    }

    /// The selection's text for the clipboard (empty without a selection).
    pub fn copy(&self) -> String {
        self.doc.selected_text()
    }

    /// Cut: the selection's text, removed from the document.
    pub fn cut(&mut self) -> String {
        let text = self.doc.selected_text();
        if !text.is_empty() {
            self.doc.delete_backward();
            self.reveal_cursor();
        }
        text
    }

    /// The char index under a cell of the text area (row, col from its
    /// top-left, gutter excluded); past a line's end lands on its end.
    pub fn hit(&self, row: usize, col: usize) -> usize {
        let rope = self.doc.rope();
        let line = (self.scroll_line + row).min(motion::last_line(rope));
        let offset =
            layout::offset_at_col(rope.line(line), self.scroll_col + col, self.doc.tab_width());
        rope.line_to_char(line) + offset
    }

    /// A press at a text-area cell: a click places the caret (⇧ extends),
    /// a double click selects the word, a triple click the line, ⌥ adds a
    /// caret (⌘ on Linux/Windows... kept to ⌥ everywhere for one rule).
    pub fn press(&mut self, row: usize, col: usize, shift: bool, add: bool, now: Instant) {
        let at = self.hit(row, col);
        let count = match self.last_click {
            Some((t, pos, n))
                if now.duration_since(t) < Duration::from_millis(400) && pos == at =>
            {
                n % 3 + 1
            }
            _ => 1,
        };
        self.last_click = Some((now, at, count));
        let rope = self.doc.rope();
        let selection = match count {
            2 => {
                let (a, b) = motion::word_at(rope, at);
                Selection::single(Range::new(a, b))
            }
            3 => {
                let line = rope.char_to_line(at);
                let start = rope.line_to_char(line);
                let end = if line + 1 < rope.len_lines() {
                    rope.line_to_char(line + 1)
                } else {
                    rope.len_chars()
                };
                Selection::single(Range::new(start, end))
            }
            _ if add => self.doc.selection().push(Range::cursor(at)),
            _ if shift => {
                let p = self.doc.selection().primary();
                Selection::single(Range::new(p.anchor, at))
            }
            _ => Selection::cursor(at),
        };
        self.doc.set_selection(selection);
        self.dragging = count == 1 && !add;
    }

    /// Pointer moved with the button held: extend the primary selection.
    pub fn drag_to(&mut self, row: usize, col: usize) {
        if !self.dragging {
            return;
        }
        let at = self.hit(row, col);
        let p = self.doc.selection().primary();
        if p.head != at {
            self.doc
                .set_selection(Selection::single(Range::new(p.anchor, at)));
        }
    }

    /// The visible rows: glyph spans (gutter numbers, then text clipped to
    /// the text area), selection and caret cells.
    pub fn draw(&self, palette: Palette, focused: bool, carets_on: bool) -> EditorDraw {
        let rope = self.doc.rope();
        let tab = self.doc.tab_width();
        let gutter = self.gutter();
        let last = motion::last_line(rope);
        let selection = self.doc.selection();
        let head_line = rope.char_to_line(selection.primary().head);
        let mut rows = Vec::with_capacity(self.rows);
        let mut sel_cells = Vec::new();
        let mut carets = Vec::new();
        for row in 0..self.rows {
            let line = self.scroll_line + row;
            if line > last {
                rows.push(Vec::new());
                continue;
            }
            let mut spans = Vec::new();
            let number = (line + 1).to_string();
            let color = if line == head_line {
                palette.gutter_current
            } else {
                palette.gutter
            };
            spans.push(Span::new((gutter - 1 - number.len()) as u16, number, color));
            let slice = rope.line(line);
            let line_start = rope.line_to_char(line);
            let content = layout::content_len(slice);
            let mut run = String::new();
            let mut run_col = 0usize;
            let flush = |run: &mut String, run_col: usize, spans: &mut Vec<Span>| {
                if !run.is_empty() {
                    spans.push(Span::new(
                        (gutter + run_col) as u16,
                        std::mem::take(run),
                        palette.fg,
                    ));
                }
            };
            for g in layout::glyphs(slice, tab, self.scroll_col + self.cols) {
                if g.col + g.width <= self.scroll_col {
                    continue;
                }
                let col = g.col.saturating_sub(self.scroll_col);
                if col + g.width > self.cols {
                    break;
                }
                if g.ch == '\t' || g.ch.is_control() {
                    flush(&mut run, run_col, &mut spans);
                    continue;
                }
                if g.width != 1 {
                    // Wide characters get their own span at an exact cell.
                    flush(&mut run, run_col, &mut spans);
                    if g.width == 2 {
                        spans.push(Span::new(
                            (gutter + col) as u16,
                            g.ch.to_string(),
                            palette.fg,
                        ));
                    }
                    continue;
                }
                if run.is_empty() {
                    run_col = col;
                } else if run_col + run.chars().count() != col {
                    flush(&mut run, run_col, &mut spans);
                    run_col = col;
                }
                run.push(g.ch);
            }
            flush(&mut run, run_col, &mut spans);
            rows.push(spans);

            // Selection cells on this line; a selection running past the
            // line's end shows one cell for its line break.
            for r in selection.ranges() {
                if r.is_empty() {
                    continue;
                }
                let from = r.from().max(line_start);
                let to = r.to().min(line_start + slice.len_chars());
                if from >= to && !(r.from() <= line_start && r.to() > line_start + content) {
                    continue;
                }
                let c0 = layout::visual_col(slice, from.saturating_sub(line_start), tab);
                let mut c1 = layout::visual_col(slice, (to - line_start).min(content), tab);
                if r.to() > line_start + content {
                    c1 += 1;
                }
                let c0 = c0.saturating_sub(self.scroll_col);
                let c1 = c1.saturating_sub(self.scroll_col).min(self.cols);
                if c1 > c0 {
                    sel_cells.push(CellRect {
                        row,
                        col: gutter + c0,
                        width: c1 - c0,
                    });
                }
            }
            if focused && carets_on {
                for r in selection.ranges() {
                    if rope.char_to_line(r.head) != line {
                        continue;
                    }
                    let c = layout::visual_col(slice, r.head - line_start, tab);
                    if c >= self.scroll_col && c - self.scroll_col <= self.cols {
                        carets.push((row, gutter + c - self.scroll_col));
                    }
                }
            }
        }
        let current_line = (head_line >= self.scroll_line
            && head_line < self.scroll_line + self.rows)
            .then(|| head_line - self.scroll_line);
        EditorDraw {
            rows,
            selection: sel_cells,
            current_line,
            carets,
            gutter,
        }
    }

    /// The primary caret as 1-based line and column (columns in cells).
    pub fn caret_line_col(&self) -> (usize, usize) {
        let rope = self.doc.rope();
        let head = self.doc.selection().primary().head;
        let line = rope.char_to_line(head);
        let col = layout::visual_col(
            rope.line(line),
            head - rope.line_to_char(line),
            self.doc.tab_width(),
        );
        (line + 1, col + 1)
    }

    /// `LF` or `CRLF`.
    pub fn line_ending_name(&self) -> &'static str {
        match self.doc.line_ending() {
            miao_term_editor::LineEnding::Lf => "LF",
            miao_term_editor::LineEnding::CrLf => "CRLF",
        }
    }

    /// The primary caret's cell (row, col including the gutter), when on
    /// screen: where the IME candidate window goes.
    pub fn caret_cell(&self) -> Option<(usize, usize)> {
        let rope = self.doc.rope();
        let head = self.doc.selection().primary().head;
        let line = rope.char_to_line(head);
        if line < self.scroll_line || line >= self.scroll_line + self.rows {
            return None;
        }
        let c = layout::visual_col(
            rope.line(line),
            head - rope.line_to_char(line),
            self.doc.tab_width(),
        );
        (c >= self.scroll_col)
            .then(|| (line - self.scroll_line, self.gutter() + c - self.scroll_col))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(text: &str) -> EditorPane {
        let mut p =
            EditorPane::with_doc("e1".into(), "/tmp/x.rs".into(), Document::from_text(text));
        p.resize(40, 5);
        p
    }

    fn palette() -> Palette {
        Palette {
            fg: (200, 200, 200),
            gutter: (90, 90, 90),
            gutter_current: (150, 150, 150),
        }
    }

    fn row_text(spans: &[Span]) -> String {
        let mut cells = vec![' '; 60];
        for s in spans {
            for (i, c) in s.text.chars().enumerate() {
                if let Some(cell) = cells.get_mut(s.col as usize + i) {
                    *cell = c;
                }
            }
        }
        cells.into_iter().collect::<String>().trim_end().to_string()
    }

    #[test]
    fn keymap_follows_platform_text_conventions() {
        let mac = cfg!(target_os = "macos");
        let (word_alt, word_ctrl) = if mac { (true, false) } else { (false, true) };
        assert_eq!(
            keymap(KeyKind::Left, false, word_alt, false, word_ctrl),
            Some(Command::Move(Motion::WordLeft, false))
        );
        assert_eq!(
            keymap(KeyKind::Right, true, false, false, false),
            Some(Command::Move(Motion::Right, true))
        );
        assert_eq!(
            keymap(KeyKind::Enter, false, false, false, false),
            Some(Command::Newline)
        );
        assert_eq!(
            keymap(KeyKind::Tab, true, false, false, false),
            Some(Command::Outdent)
        );
        let (cmd, ctrl) = if mac { (true, false) } else { (false, true) };
        assert_eq!(
            keymap(KeyKind::Char('z'), false, false, cmd, ctrl),
            Some(Command::Undo)
        );
        assert_eq!(
            keymap(KeyKind::Char('Z'), true, false, cmd, ctrl),
            Some(Command::Redo)
        );
        assert_eq!(
            keymap(KeyKind::Char('d'), false, false, cmd, ctrl),
            Some(Command::SelectNextOccurrence)
        );
        assert_eq!(
            keymap(KeyKind::Char('s'), false, false, cmd, ctrl),
            None,
            "save is the app's"
        );
        assert_eq!(
            keymap(KeyKind::Char('x'), false, false, false, false),
            None,
            "text input"
        );
        if mac {
            assert_eq!(
                keymap(KeyKind::Left, false, false, true, false),
                Some(Command::Move(Motion::LineStart, false))
            );
            assert_eq!(
                keymap(KeyKind::Backspace, false, false, true, false),
                Some(Command::DeleteToLineStart)
            );
        }
    }

    #[test]
    fn draws_numbers_text_and_clips_long_lines() {
        let mut p = pane("fn main() {\n\tlet 中 = 1;\n}\n");
        p.resize(14, 5);
        let d = p.draw(palette(), true, true);
        assert_eq!(d.gutter, 3);
        assert_eq!(row_text(&d.rows[0]), " 1 fn main() {");
        assert_eq!(row_text(&d.rows[1]), " 2     let 中");
        assert_eq!(row_text(&d.rows[3]), " 4");
        assert_eq!(d.rows[4].len(), 0, "past the end of the document");
        assert_eq!(d.carets, vec![(0, 3)]);
        assert_eq!(p.caret_line_col(), (1, 1));
        assert_eq!(p.line_ending_name(), "LF");
        assert_eq!(d.current_line, Some(0));
        let wide = d.rows[1].iter().find(|s| s.text == "中").unwrap();
        assert_eq!(wide.col, 11);
    }

    #[test]
    fn selection_cells_include_the_line_break() {
        let mut p = pane("abc\ndef\n");
        p.doc.set_selection(Selection::single(Range::new(1, 6)));
        let d = p.draw(palette(), true, false);
        assert_eq!(
            d.selection,
            vec![
                CellRect {
                    row: 0,
                    col: 4,
                    width: 3
                },
                CellRect {
                    row: 1,
                    col: 3,
                    width: 2
                },
            ]
        );
        assert!(d.carets.is_empty(), "blink off");
    }

    #[test]
    fn typing_scrolls_to_keep_the_caret_visible() {
        let mut p = pane(&"line\n".repeat(50));
        p.run(Command::Move(Motion::DocEnd, false));
        assert_eq!(p.scroll_line, 46, "last line at the bottom of 5 rows");
        p.run(Command::Move(Motion::DocStart, false));
        assert_eq!(p.scroll_line, 0);
        p.type_text(&"x".repeat(60));
        assert_eq!(p.scroll_col, 60 + 1 - p.cols);
    }

    #[test]
    fn clicks_place_select_words_and_lines() {
        let mut p = pane("let value = 1;\nnext\n");
        let t = Instant::now();
        p.press(0, 6, false, false, t);
        assert_eq!(p.doc.selection().primary(), Range::cursor(6));
        p.press(0, 6, false, false, t + Duration::from_millis(100));
        assert_eq!(p.copy(), "value");
        p.press(0, 6, false, false, t + Duration::from_millis(200));
        assert_eq!(p.copy(), "let value = 1;\n");
        p.press(1, 2, true, false, t + Duration::from_secs(2));
        assert_eq!(
            p.doc.selection().primary(),
            Range::new(0, 17),
            "shift extends from the line selection's anchor"
        );
        p.press(0, 99, false, false, t + Duration::from_secs(4));
        assert_eq!(
            p.doc.selection().primary(),
            Range::cursor(14),
            "past the end"
        );
        p.drag_to(1, 4);
        assert_eq!(p.copy(), "\nnext");
    }

    #[test]
    fn cut_and_paste_round_trip() {
        let mut p = pane("one two");
        p.doc.set_selection(Selection::single(Range::new(0, 4)));
        assert_eq!(p.cut(), "one ");
        assert_eq!(p.doc.rope().to_string(), "two");
        p.paste("one ");
        assert_eq!(p.doc.rope().to_string(), "one two");
    }

    /// Perf gate (ADR 0034, E2): building the visible rows of a 100 MB file,
    /// scrolled to its middle with a selection, stays far below the 4 ms
    /// frame budget. Run with `cargo test --release -- --ignored`.
    #[test]
    #[ignore = "perf gate; run `cargo test --release -- --ignored`"]
    fn drawing_a_screen_of_a_100_mb_file_fits_a_frame() {
        let line = "    let value = compute(index, &table[offset..end]); // 中文注释\n";
        let text = line.repeat((100 << 20) / line.len());
        let mut p =
            EditorPane::with_doc("e".into(), "/tmp/big.rs".into(), Document::from_text(&text));
        p.resize(200, 60);
        let middle = p.doc.rope().len_chars() / 2;
        p.doc
            .set_selection(Selection::single(Range::new(middle, middle + 5000)));
        p.reveal_cursor();
        let runs = 200;
        let start = Instant::now();
        let mut glyphs = 0;
        for _ in 0..runs {
            let d = p.draw(palette(), true, true);
            glyphs += d.rows.len();
        }
        let ms = start.elapsed().as_secs_f64() * 1e3 / runs as f64;
        let scale: f64 = std::env::var("MTTY_PERF_SCALE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);
        eprintln!("editor rows for one screen of a 100 MB file: {ms:.4} ms");
        assert!(glyphs > 0);
        assert!(ms <= 4.0 * scale, "{ms:.4} ms per frame, budget 4 ms");
    }

    #[test]
    fn saves_atomically_and_keeps_permissions() {
        let dir = std::env::temp_dir().join(format!("mtty-editor-pane-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "hello\r\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        let mut p = EditorPane::open("e".into(), &file).unwrap();
        p.doc.set_selection(Selection::cursor(5));
        p.type_text("!");
        assert!(p.title().ends_with('\u{25cf}'));
        p.save().unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "hello!\r\n");
        assert_eq!(p.title(), "a.txt");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o640);
        }
        assert!(!dir.join(".a.txt.mtty-save").exists());
        assert!(EditorPane::open("e".into(), &dir.join("missing")).is_err());
        std::fs::write(dir.join("bin"), b"\0\x01\x02").unwrap();
        assert!(EditorPane::open("e".into(), &dir.join("bin")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
