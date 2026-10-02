//! Vim mode for editor panes (ADR 0034, E6).
//!
//! A small, self-contained state machine over [`Document`]: NORMAL, INSERT,
//! VISUAL and VISUAL LINE, with counts, the common motions and the
//! `d`/`c`/`y` operators, an internal register, and `u` / `Ctrl-r`. Fold
//! commands (`z…`) and the `:` command line and `/` find leave through
//! [`Action`], because the pane owns fold state and the host owns the
//! prompts. The module is pure (no window or key types), so it is unit
//! tested.

use crate::change::{Change, Transaction};
use crate::document::{Document, Motion};
use crate::history::EditKind;
use crate::motion;
use crate::selection::{Range, Selection};

/// Which editing mode the pane is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Visual,
    VisualLine,
}

/// One key as the pane saw it (modifiers other than Ctrl are already folded
/// into the character by the host's keymap).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    /// A control chord, e.g. `Ctrl('r')`.
    Ctrl(char),
    Esc,
    Enter,
    Backspace,
    Delete,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
}

/// A fold command the pane applies (`z…`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fold {
    Toggle,
    Close,
    Open,
    CloseAll,
    OpenAll,
}

/// What the pane must do beyond the document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Open the Find bar (`/`).
    Find,
    /// Open the `:` command line.
    CommandLine,
    /// Fold the caret's region (the pane owns fold state).
    Fold(Fold),
    /// The command was not recognized in this mode.
    Ignored,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Operator {
    Delete,
    Change,
    Yank,
}

impl Operator {
    fn letter(self) -> char {
        match self {
            Operator::Delete => 'd',
            Operator::Change => 'c',
            Operator::Yank => 'y',
        }
    }
}

/// The vim state for one editor pane.
pub struct Vim {
    mode: Mode,
    count: usize,
    operator: Option<Operator>,
    pending_g: bool,
    pending_z: bool,
    register: String,
    /// The register holds whole lines (pasted below/above).
    linewise: bool,
    /// Visual mode's anchor (a char index).
    anchor: usize,
}

impl Default for Vim {
    fn default() -> Self {
        Self {
            mode: Mode::Normal,
            count: 0,
            operator: None,
            pending_g: false,
            pending_z: false,
            register: String::new(),
            linewise: false,
            anchor: 0,
        }
    }
}

impl Vim {
    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn is_insert(&self) -> bool {
        self.mode == Mode::Insert
    }

    /// Enter insert mode before the caret (used when vim is turned on while
    /// the pane already has focus).
    pub fn insert(&mut self) {
        self.mode = Mode::Insert;
        self.reset_pending();
    }

    /// Handle one key. Returns any action for the pane or host.
    pub fn handle(&mut self, key: Key, doc: &mut Document) -> Action {
        match self.mode {
            Mode::Insert => self.inserting(key, doc),
            Mode::Visual | Mode::VisualLine => self.visual(key, doc),
            Mode::Normal => self.normal(key, doc),
        }
    }

    fn reset_pending(&mut self) {
        self.count = 0;
        self.operator = None;
        self.pending_g = false;
        self.pending_z = false;
    }

    fn caret(&self, doc: &Document) -> usize {
        doc.selection().primary().head
    }

    fn insert_here(&mut self, doc: &mut Document, at: usize) {
        doc.set_selection(Selection::cursor(at));
        self.mode = Mode::Insert;
        self.reset_pending();
    }

    /// The line-start / line-end char range `line`, including its newline
    /// when there is one (`dd` removes the break too).
    fn line_range(doc: &Document, line: usize) -> (usize, usize) {
        let rope = doc.rope();
        let last = motion::last_line(rope);
        let start = rope.line_to_char(line.min(last));
        let end = if line < last {
            rope.line_to_char(line + 1)
        } else {
            rope.len_chars()
        };
        (start, end)
    }

    fn apply(&self, doc: &mut Document, tx: Transaction, cursor: usize, kind: EditKind) {
        let after = Selection::cursor(cursor).clamp(doc.rope().len_chars());
        doc.apply(tx, after, kind);
    }

    /// Delete `from..to` (chars) and leave the caret at `keep`.
    fn delete_range(&mut self, doc: &mut Document, from: usize, to: usize, keep: usize) {
        if from >= to {
            return;
        }
        self.apply(
            doc,
            Transaction::new(vec![Change::delete(from, to)]),
            keep,
            EditKind::Other,
        );
    }

    /// Keep `from..to` in the register, deleting it when `delete` is set.
    fn operate(
        &mut self,
        doc: &mut Document,
        from: usize,
        to: usize,
        linewise: bool,
        op: Operator,
    ) -> Action {
        let (from, to) = (from.min(to), from.max(to));
        if from == to {
            return Action::None;
        }
        let text: String = doc.rope().slice(from..to).to_string();
        if op != Operator::Yank {
            self.register = text;
            self.linewise = linewise;
            self.delete_range(doc, from, to, from);
        } else {
            self.register = text;
            self.linewise = linewise;
            doc.set_selection(Selection::cursor(from));
        }
        if op == Operator::Change {
            self.mode = Mode::Insert;
        }
        Action::None
    }

    // ---- NORMAL ----

    fn normal(&mut self, key: Key, doc: &mut Document) -> Action {
        if let Key::Char(c) = key {
            if c.is_ascii_digit() && !(c == '0' && self.count == 0) {
                self.count = (self.count * 10 + (c as usize - '0' as usize)).min(1_000_000);
                return Action::None;
            }
        }
        let count = self.count.max(1);
        self.count = 0;

        // A pending operator takes the next key as its motion.
        if let Some(op) = self.operator {
            if matches!(key, Key::Char(c) if c == op.letter()) {
                self.operator = None;
                return self.operate_linewise(doc, op, count);
            }
            self.operator = None;
            return self.operator_motion(doc, op, key, count);
        }

        if self.pending_g {
            self.pending_g = false;
            return match key {
                Key::Char('g') => {
                    doc.move_cursor(Motion::DocStart, false);
                    if count > 1 {
                        for _ in 1..count {
                            doc.move_cursor(Motion::Down, false);
                        }
                    }
                    Action::None
                }
                _ => Action::Ignored,
            };
        }

        if self.pending_z {
            self.pending_z = false;
            return match key {
                Key::Char('a') => Action::Fold(Fold::Toggle),
                Key::Char('c') => Action::Fold(Fold::Close),
                Key::Char('o') => Action::Fold(Fold::Open),
                Key::Char('R') => Action::Fold(Fold::OpenAll),
                Key::Char('M') => Action::Fold(Fold::CloseAll),
                _ => Action::Ignored,
            };
        }

        match key {
            Key::Esc => {
                doc.set_selection(Selection::cursor(self.caret(doc)));
                Action::None
            }
            Key::Left => self.move_n(doc, Motion::Left, count),
            Key::Right => self.move_n(doc, Motion::Right, count),
            Key::Up => self.move_n(doc, Motion::Up, count),
            Key::Down => self.move_n(doc, Motion::Down, count),
            Key::Home => self.move_n(doc, Motion::LineStart, count),
            Key::End => self.move_n(doc, Motion::LineEnd, count),
            Key::Char('h') => self.move_n(doc, Motion::Left, count),
            Key::Char('j') => self.move_n(doc, Motion::Down, count),
            Key::Char('k') => self.move_n(doc, Motion::Up, count),
            Key::Char('l') => self.move_n(doc, Motion::Right, count),
            Key::Char('w') => {
                for _ in 0..count {
                    let to = vim_word_forward(doc.rope(), self.caret(doc));
                    doc.set_selection(Selection::cursor(to));
                }
                Action::None
            }
            Key::Char('b') => {
                for _ in 0..count {
                    let to = vim_word_backward(doc.rope(), self.caret(doc));
                    doc.set_selection(Selection::cursor(to));
                }
                Action::None
            }
            Key::Char('e') => {
                for _ in 0..count {
                    let to = vim_word_end(doc.rope(), self.caret(doc));
                    doc.set_selection(Selection::cursor(to));
                }
                Action::None
            }
            Key::Char('0') => {
                doc.move_cursor(Motion::LineStart, false);
                Action::None
            }
            Key::Char('^') => {
                first_non_blank(doc);
                Action::None
            }
            Key::Char('$') => {
                let line = doc.rope().char_to_line(self.caret(doc));
                doc.move_cursor(Motion::LineEnd, false);
                let _ = line;
                Action::None
            }
            Key::Char('g') => {
                self.pending_g = true;
                Action::None
            }
            Key::Char('G') => {
                if count > 1 {
                    for _ in 1..count {
                        doc.move_cursor(Motion::Down, false);
                    }
                    first_non_blank(doc);
                } else {
                    doc.move_cursor(Motion::DocEnd, false);
                }
                Action::None
            }
            Key::Char('i') => {
                self.insert_here(doc, self.caret(doc));
                Action::None
            }
            Key::Char('a') => {
                let at = motion::next_grapheme(doc.rope(), self.caret(doc));
                self.insert_here(doc, at);
                Action::None
            }
            Key::Char('I') => {
                first_non_blank(doc);
                self.insert_here(doc, self.caret(doc));
                Action::None
            }
            Key::Char('A') => {
                doc.move_cursor(Motion::LineEnd, false);
                self.insert_here(doc, self.caret(doc));
                Action::None
            }
            Key::Char('o') => {
                doc.move_cursor(Motion::LineEnd, false);
                doc.newline();
                self.insert_here(doc, self.caret(doc));
                Action::None
            }
            Key::Char('O') => {
                doc.move_cursor(Motion::LineStart, false);
                doc.newline();
                doc.move_cursor(Motion::Up, false);
                self.insert_here(doc, self.caret(doc));
                Action::None
            }
            Key::Char('x') => {
                let head = self.caret(doc);
                let mut to = head;
                for _ in 0..count {
                    to = motion::next_grapheme(doc.rope(), to);
                }
                let to = to.min(motion::line_end(doc.rope(), doc.rope().char_to_line(head)));
                self.delete_range(doc, head, to, head);
                Action::None
            }
            Key::Char('d') | Key::Char('c') | Key::Char('y') => {
                let op = match key {
                    Key::Char('d') => Operator::Delete,
                    Key::Char('c') => Operator::Change,
                    _ => Operator::Yank,
                };
                self.operator = Some(op);
                self.count = count;
                Action::None
            }
            Key::Char('p') => {
                self.paste(doc, true);
                Action::None
            }
            Key::Char('P') => {
                self.paste(doc, false);
                Action::None
            }
            Key::Char('u') => {
                doc.undo();
                Action::None
            }
            Key::Ctrl('r') => {
                doc.redo();
                Action::None
            }
            Key::Char('r') => {
                self.pending_g = false;
                self.pending_z = false;
                self.count = 0;
                Action::Ignored
            }
            Key::Char('J') => {
                join_lines(doc);
                Action::None
            }
            Key::Char('v') => {
                self.anchor = self.caret(doc);
                self.mode = Mode::Visual;
                Action::None
            }
            Key::Char('V') => {
                self.anchor = self.caret(doc);
                let line = doc.rope().char_to_line(self.anchor);
                let (start, _) = Self::line_range(doc, line);
                doc.set_selection(Selection::single(Range::new(self.anchor, start)));
                self.mode = Mode::VisualLine;
                Action::None
            }
            Key::Char(':') => Action::CommandLine,
            Key::Char('/') => Action::Find,
            Key::Char('z') => {
                self.pending_z = true;
                Action::None
            }
            _ => Action::Ignored,
        }
    }

    fn move_n(&mut self, doc: &mut Document, motion: Motion, count: usize) -> Action {
        for _ in 0..count {
            doc.move_cursor(motion, false);
        }
        Action::None
    }

    fn operate_linewise(&mut self, doc: &mut Document, op: Operator, count: usize) -> Action {
        let rope = doc.rope();
        let line = rope.char_to_line(self.caret(doc));
        let last = motion::last_line(rope);
        let end_line = (line + count - 1).min(last);
        let (from, _) = Self::line_range(doc, line);
        let (_, to) = Self::line_range(doc, end_line);
        self.operate(doc, from, to, true, op)
    }

    fn operator_motion(
        &mut self,
        doc: &mut Document,
        op: Operator,
        key: Key,
        count: usize,
    ) -> Action {
        let head = self.caret(doc);
        let rope = doc.rope();
        let line = rope.char_to_line(head);
        let (from, to, linewise) = match key {
            Key::Char('w') => {
                let mut at = head;
                for _ in 0..count {
                    // Vim's `cw` changes to the end of the word (like `ce`).
                    at = if matches!(op, Operator::Change) {
                        vim_word_end(rope, at)
                    } else {
                        vim_word_forward(rope, at)
                    };
                }
                (head, at, false)
            }
            Key::Char('b') => {
                let mut at = head;
                for _ in 0..count {
                    at = vim_word_backward(rope, at);
                }
                (at, head, false)
            }
            Key::Char('e') => {
                let mut at = head;
                for _ in 0..count {
                    at = vim_word_end(rope, at);
                }
                (head, at, false)
            }
            Key::Char('$') => (head, motion::line_end(rope, line), false),
            Key::Char('0') | Key::Char('^') => (motion::smart_home(rope, head), head, false),
            Key::Char('j') => {
                let end_line = (line + count).min(motion::last_line(rope));
                let (from, _) = Self::line_range(doc, line);
                let (_, to) = Self::line_range(doc, end_line);
                (from, to, true)
            }
            Key::Char('k') => {
                let start_line = line.saturating_sub(count);
                let (from, _) = Self::line_range(doc, start_line);
                let (_, to) = Self::line_range(doc, line);
                (from, to, true)
            }
            _ => return Action::Ignored,
        };
        self.operate(doc, from, to, linewise, op)
    }

    fn paste(&mut self, doc: &mut Document, after: bool) {
        if self.register.is_empty() {
            return;
        }
        let text = self.register.clone();
        if self.linewise {
            let rope = doc.rope();
            let line = rope.char_to_line(self.caret(doc));
            let last = rope.len_lines().saturating_sub(1);
            let at = if after {
                if line + 1 >= rope.len_lines() {
                    rope.len_chars()
                } else {
                    rope.line_to_char(line + 1)
                }
            } else {
                rope.line_to_char(line.min(last))
            };
            let tx = Transaction::new(vec![Change::insert(at, text)]);
            self.apply(doc, tx, at, EditKind::Other);
            return;
        }
        let head = self.caret(doc);
        let at = if after {
            motion::next_grapheme(doc.rope(), head)
        } else {
            head
        };
        let tx = Transaction::new(vec![Change::insert(at, text)]);
        self.apply(doc, tx, at, EditKind::Other);
    }

    // ---- VISUAL ----

    fn visual(&mut self, key: Key, doc: &mut Document) -> Action {
        let linewise = self.mode == Mode::VisualLine;
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                doc.set_selection(Selection::cursor(self.caret(doc)));
                Action::None
            }
            Key::Char('d') | Key::Char('x') => {
                let (from, to) = self.selection_bounds(doc, linewise);
                self.mode = Mode::Normal;
                self.operate(doc, from, to, linewise, Operator::Delete)
            }
            Key::Char('c') => {
                let (from, to) = self.selection_bounds(doc, linewise);
                self.mode = Mode::Normal;
                self.operate(doc, from, to, linewise, Operator::Change)
            }
            Key::Char('y') => {
                let (from, to) = self.selection_bounds(doc, linewise);
                self.mode = Mode::Normal;
                self.operate(doc, from, to, linewise, Operator::Yank)
            }
            Key::Char('h') | Key::Left => {
                doc.move_cursor(Motion::Left, true);
                Action::None
            }
            Key::Char('l') | Key::Right => {
                doc.move_cursor(Motion::Right, true);
                Action::None
            }
            Key::Char('j') | Key::Down => {
                doc.move_cursor(Motion::Down, true);
                Action::None
            }
            Key::Char('k') | Key::Up => {
                doc.move_cursor(Motion::Up, true);
                Action::None
            }
            Key::Char('w') => {
                doc.move_cursor(Motion::WordRight, true);
                Action::None
            }
            Key::Char('b') => {
                doc.move_cursor(Motion::WordLeft, true);
                Action::None
            }
            Key::Char('$') => {
                doc.move_cursor(Motion::LineEnd, true);
                Action::None
            }
            Key::Char('0') => {
                doc.move_cursor(Motion::LineStart, true);
                Action::None
            }
            _ => Action::Ignored,
        }
    }

    fn selection_bounds(&self, doc: &Document, linewise: bool) -> (usize, usize) {
        let sel = doc.selection().primary();
        let (mut from, mut to) = (sel.from().min(sel.to()), sel.from().max(sel.to()));
        if linewise {
            let rope = doc.rope();
            let a = rope.char_to_line(from);
            let b = rope.char_to_line(to);
            let (start, _) = Self::line_range(doc, a);
            let (_, end) = Self::line_range(doc, b);
            from = start;
            to = end;
        } else if to < doc.rope().len_chars() {
            // Vim's visual selection is inclusive.
            to = motion::next_grapheme(doc.rope(), to);
        }
        (from, to)
    }

    // ---- INSERT ----

    fn inserting(&mut self, key: Key, doc: &mut Document) -> Action {
        match key {
            Key::Esc => {
                self.mode = Mode::Normal;
                self.reset_pending();
                doc.move_cursor(Motion::Left, false);
            }
            Key::Enter => doc.newline(),
            Key::Backspace => doc.delete_backward(),
            Key::Delete => doc.delete_forward(),
            Key::Left => doc.move_cursor(Motion::Left, false),
            Key::Right => doc.move_cursor(Motion::Right, false),
            Key::Up => doc.move_cursor(Motion::Up, false),
            Key::Down => doc.move_cursor(Motion::Down, false),
            Key::Home => doc.move_cursor(Motion::LineStart, false),
            Key::End => doc.move_cursor(Motion::LineEnd, false),
            Key::Char(c) => doc.type_text(&c.to_string()),
            Key::Ctrl('r') => {
                doc.redo();
            }
            Key::Ctrl('w') => doc.delete_word_backward(),
            Key::Ctrl('u') => doc.delete_to_line_start(),
            _ => {}
        }
        Action::None
    }
}

/// Vim word characters: letters, digits and `_`.
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Vim's `w`: the start of the next word (or punctuation run), skipping
/// whitespace and line breaks.
fn vim_word_forward(rope: &ropey::Rope, at: usize) -> usize {
    let len = rope.len_chars();
    let mut i = at.min(len);
    if i < len && !rope.char(i).is_whitespace() {
        let word = is_word_char(rope.char(i));
        while i < len && !rope.char(i).is_whitespace() && is_word_char(rope.char(i)) == word {
            i += 1;
        }
    }
    while i < len && rope.char(i).is_whitespace() {
        i += 1;
    }
    i
}

/// Vim's `b`: the start of the previous word.
fn vim_word_backward(rope: &ropey::Rope, at: usize) -> usize {
    let mut i = at.min(rope.len_chars());
    if i == 0 {
        return 0;
    }
    i -= 1;
    while i > 0 && rope.char(i).is_whitespace() {
        i -= 1;
    }
    if i > 0 {
        let word = is_word_char(rope.char(i));
        while i > 0 && !rope.char(i - 1).is_whitespace() && is_word_char(rope.char(i - 1)) == word {
            i -= 1;
        }
    }
    i
}

/// Vim's `e`: the end of the current or next word (the char after its last
/// one, matching the editor's insertion-point cursor).
fn vim_word_end(rope: &ropey::Rope, at: usize) -> usize {
    let len = rope.len_chars();
    let mut i = at.min(len);
    if i < len {
        i = motion::next_grapheme(rope, i);
    }
    while i < len && rope.char(i).is_whitespace() {
        i += 1;
    }
    if i < len {
        let word = is_word_char(rope.char(i));
        while i + 1 < len
            && !rope.char(i + 1).is_whitespace()
            && is_word_char(rope.char(i + 1)) == word
        {
            i += 1;
        }
        i += 1;
    }
    i.min(len)
}

/// Move the caret to the first non-blank character of its line.
fn first_non_blank(doc: &mut Document) {
    let head = doc.selection().primary().head;
    let rope = doc.rope();
    let line = rope.char_to_line(head);
    let start = rope.line_to_char(line);
    let slice = rope.line(line);
    let indent = slice
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .count();
    doc.set_selection(Selection::cursor(start + indent));
}

/// Join the caret's line with the next, one space between (vim's `J`).
fn join_lines(doc: &mut Document) {
    let head = doc.selection().primary().head;
    let rope = doc.rope();
    let line = rope.char_to_line(head);
    if line >= motion::last_line(rope) {
        return;
    }
    let end = motion::line_end(rope, line);
    let next_start = rope.line_to_char(line + 1);
    let next_indent = rope
        .line(line + 1)
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .count();
    let from = end.min(rope.len_chars());
    let to = (next_start + next_indent).min(rope.len_chars());
    doc.apply_external(Transaction::new(vec![Change::replace(from, to, " ")]));
    doc.set_selection(Selection::cursor(from));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Document;

    fn doc(text: &str) -> Document {
        let mut d = Document::from_text(text);
        d.set_selection(Selection::cursor(0));
        d
    }

    fn text(d: &Document) -> String {
        d.rope().to_string()
    }

    fn head(d: &Document) -> usize {
        d.selection().primary().head
    }

    fn press(vim: &mut Vim, keys: &str, d: &mut Document) {
        for c in keys.chars() {
            vim.handle(Key::Char(c), d);
        }
    }

    #[test]
    fn motions_and_counts_move_the_caret() {
        let mut d = doc("one two three\nfour\n");
        let mut vim = Vim::default();
        press(&mut vim, "w", &mut d);
        assert_eq!(head(&d), 4);
        press(&mut vim, "2w", &mut d);
        assert_eq!(head(&d), 14, "one line down from three");
        press(&mut vim, "0", &mut d);
        assert_eq!(head(&d), 14);
        press(&mut vim, "G", &mut d);
        assert_eq!(d.rope().char_to_line(head(&d)), 2);
    }

    #[test]
    fn insert_commands_edit_and_return_to_normal() {
        let mut d = doc("abc\n");
        let mut vim = Vim::default();
        vim.handle(Key::Char('i'), &mut d);
        assert_eq!(vim.mode(), Mode::Insert);
        press(&mut vim, "X", &mut d);
        vim.handle(Key::Esc, &mut d);
        assert_eq!(text(&d), "Xabc\n");
        assert_eq!(vim.mode(), Mode::Normal);

        vim.handle(Key::Char('A'), &mut d);
        press(&mut vim, "!", &mut d);
        vim.handle(Key::Esc, &mut d);
        assert_eq!(text(&d), "Xabc!\n");
    }

    #[test]
    fn dd_and_p_move_lines() {
        let mut d = doc("one\ntwo\nthree\n");
        let mut vim = Vim::default();
        press(&mut vim, "dd", &mut d);
        assert_eq!(text(&d), "two\nthree\n");
        assert_eq!(d.rope().char_to_line(head(&d)), 0);
        press(&mut vim, "p", &mut d);
        assert_eq!(text(&d), "two\none\nthree\n");
    }

    #[test]
    fn yank_stores_and_pastes_a_line() {
        let mut d = doc("one\ntwo\n");
        let mut vim = Vim::default();
        press(&mut vim, "yy", &mut d);
        assert_eq!(vim.register, "one\n");
        assert!(vim.linewise);
        press(&mut vim, "p", &mut d);
        assert_eq!(text(&d), "one\none\ntwo\n");
    }

    #[test]
    fn change_word_enters_insert() {
        let mut d = doc("one two\n");
        let mut vim = Vim::default();
        press(&mut vim, "cw", &mut d);
        assert_eq!(vim.mode(), Mode::Insert);
        // Vim's `cw` changes to the end of the word, leaving the space.
        assert_eq!(text(&d), " two\n");
    }

    #[test]
    fn command_and_find_keys_become_actions() {
        let mut d = doc("x\n");
        let mut vim = Vim::default();
        assert_eq!(vim.handle(Key::Char(':'), &mut d), Action::CommandLine);
        assert_eq!(vim.handle(Key::Char('/'), &mut d), Action::Find);
        assert_eq!(vim.handle(Key::Char('z'), &mut d), Action::None);
        assert_eq!(
            vim.handle(Key::Char('a'), &mut d),
            Action::Fold(Fold::Toggle)
        );
    }
}
