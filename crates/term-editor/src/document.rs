//! A document: text, selections and history, edited through commands that
//! act on every selection at once.

use ropey::Rope;

use crate::change::{Assoc, ByteEdit, Change, Transaction};
use crate::history::{EditKind, History};
use crate::layout;
use crate::motion;
use crate::search::{self, SearchQuery};
use crate::selection::{Range, Selection};
use crate::text::{self, DecodeError, LineEnding};

/// How one indentation level is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Indent {
    Tab,
    Spaces(usize),
}

impl Indent {
    fn unit(self) -> String {
        match self {
            Indent::Tab => "\t".into(),
            Indent::Spaces(n) => " ".repeat(n),
        }
    }

    /// Tabs if more indented lines start with a tab than with spaces;
    /// otherwise the smallest space indent seen (2–8), defaulting to 4.
    pub fn detect(rope: &Rope) -> Indent {
        let mut tabs = 0usize;
        let mut spaces = 0usize;
        let mut smallest = usize::MAX;
        for line in rope.lines().take(2000) {
            match line.chars().next() {
                Some('\t') => tabs += 1,
                Some(' ') => {
                    let n = line.chars().take_while(|c| *c == ' ').count();
                    // A lone space or 1-space indents are alignment, not levels.
                    if n >= 2 && line.chars().nth(n).is_some_and(|c| !c.is_whitespace()) {
                        spaces += 1;
                        smallest = smallest.min(n);
                    }
                }
                _ => {}
            }
        }
        if tabs > spaces {
            Indent::Tab
        } else if spaces > 0 {
            Indent::Spaces(smallest.clamp(2, 8))
        } else {
            Indent::Spaces(4)
        }
    }
}

/// Where a cursor command moves each selection's head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Up,
    Down,
    /// Smart home: first non-blank char, then column 0.
    LineStart,
    LineEnd,
    DocStart,
    DocEnd,
    /// Up/down by this many lines (the view's height).
    PageUp(usize),
    PageDown(usize),
}

#[derive(Clone, Debug)]
pub struct Document {
    rope: Rope,
    selection: Selection,
    history: History,
    line_ending: LineEnding,
    bom: bool,
    indent: Indent,
    tab_width: usize,
    saved_state: u64,
    /// Bumped on every change to the text, for caches (render rows, search).
    revision: u64,
    /// Byte edits since the last `take_edits`, for a syntax tree.
    edits: Vec<ByteEdit>,
}

impl Default for Document {
    fn default() -> Self {
        Document::from_rope(Rope::new(), false, LineEnding::Lf)
    }
}

impl Document {
    fn from_rope(rope: Rope, bom: bool, line_ending: LineEnding) -> Self {
        let indent = Indent::detect(&rope);
        Document {
            rope,
            selection: Selection::cursor(0),
            history: History::default(),
            line_ending,
            bom,
            indent,
            tab_width: 4,
            saved_state: 0,
            revision: 0,
            edits: Vec::new(),
        }
    }

    pub fn from_text(text: &str) -> Self {
        Document::from_rope(Rope::from_str(text), false, LineEnding::detect(text))
    }

    /// A file's bytes (UTF-8, optional BOM).
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let (rope, bom, ending) = text::decode(bytes)?;
        Ok(Document::from_rope(rope, bom, ending))
    }

    /// The bytes to save, with the original BOM and line endings.
    pub fn to_bytes(&self) -> Vec<u8> {
        text::encode(&self.rope, self.bom)
    }

    pub fn rope(&self) -> &Rope {
        &self.rope
    }

    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    pub fn line_ending(&self) -> LineEnding {
        self.line_ending
    }

    pub fn indent_style(&self) -> Indent {
        self.indent
    }

    pub fn set_indent_style(&mut self, indent: Indent) {
        self.indent = indent;
    }

    pub fn tab_width(&self) -> usize {
        self.tab_width
    }

    pub fn set_tab_width(&mut self, width: usize) {
        self.tab_width = width.max(1);
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The byte edits made since the last call, in order, for a syntax tree.
    pub fn take_edits(&mut self) -> Vec<ByteEdit> {
        std::mem::take(&mut self.edits)
    }

    /// Unsaved changes: the text differs from the last save (undoing back to
    /// it counts as unmodified).
    pub fn is_modified(&self) -> bool {
        self.history.state_id() != self.saved_state
    }

    pub fn mark_saved(&mut self) {
        self.saved_state = self.history.state_id();
        self.history.break_group();
    }

    /// Replace the selection (a click, a search hit). Ends the typing group.
    pub fn set_selection(&mut self, selection: Selection) {
        self.selection = selection.clamp(self.rope.len_chars());
        self.history.break_group();
    }

    /// Apply a transaction and record it; the selection becomes `after`.
    pub fn apply(&mut self, tx: Transaction, after: Selection, kind: EditKind) {
        if tx.is_empty() {
            self.selection = after;
            return;
        }
        let before = self.selection.clone();
        self.edits.extend(tx.byte_edits(&self.rope));
        let inverse = tx.apply(&mut self.rope);
        self.revision += 1;
        self.selection = after.clamp(self.rope.len_chars());
        self.history
            .record(kind, tx, inverse, before, self.selection.clone());
    }

    /// An edit from outside the cursor (an agent, a formatter, replace-all):
    /// one undo step, selections carried along.
    pub fn apply_external(&mut self, tx: Transaction) {
        let after = self.selection.map(&tx);
        self.apply(tx, after, EditKind::Other);
    }

    /// Replace every selection with `text` and put each cursor after it.
    fn replace_selections(&mut self, text: &str, kind: EditKind) {
        let tx = Transaction::new(
            self.selection
                .ranges()
                .iter()
                .map(|r| Change::replace(r.from(), r.to(), text))
                .collect(),
        );
        let after = self.selection.transform(|r| {
            let at = tx.map(r.to(), Assoc::After);
            Range::cursor(at)
        });
        self.apply(tx, after, kind);
    }

    /// Typed text (no line breaks): one undo group per run of typing.
    pub fn type_text(&mut self, text: &str) {
        if text.contains('\n') || text.contains('\r') {
            self.paste(text);
        } else {
            self.replace_selections(text, EditKind::Typing);
        }
    }

    /// Pasted text, line breaks written in the document's ending. With one
    /// line per cursor, each cursor gets its own line (as copy put them).
    pub fn paste(&mut self, text: &str) {
        let text = self.line_ending.normalize(text);
        let ending = self.line_ending.as_str();
        let pieces: Vec<&str> = text
            .strip_suffix(ending)
            .unwrap_or(&text)
            .split(ending)
            .collect();
        if self.selection.len() > 1 && pieces.len() == self.selection.len() {
            let tx = Transaction::new(
                self.selection
                    .ranges()
                    .iter()
                    .zip(&pieces)
                    .map(|(r, p)| Change::replace(r.from(), r.to(), *p))
                    .collect(),
            );
            let after = self
                .selection
                .transform(|r| Range::cursor(tx.map(r.to(), Assoc::After)));
            self.apply(tx, after, EditKind::Other);
        } else {
            self.replace_selections(&text, EditKind::Other);
        }
    }

    /// Enter: a line break, then the current line's leading whitespace.
    pub fn newline(&mut self) {
        let ending = self.line_ending.as_str();
        let tx = Transaction::new(
            self.selection
                .ranges()
                .iter()
                .map(|r| {
                    let line = self.rope.char_to_line(r.from());
                    let start = motion::line_start(&self.rope, line);
                    let indent: String = self
                        .rope
                        .slice(start..r.from())
                        .chars()
                        .take_while(|c| *c == ' ' || *c == '\t')
                        .collect();
                    Change::replace(r.from(), r.to(), format!("{ending}{indent}"))
                })
                .collect(),
        );
        let after = self
            .selection
            .transform(|r| Range::cursor(tx.map(r.to(), Assoc::After)));
        self.apply(tx, after, EditKind::Other);
    }

    /// Backspace: the selection, or the grapheme before each cursor — or back
    /// to the previous indent stop inside leading spaces.
    pub fn delete_backward(&mut self) {
        let unit = match self.indent {
            Indent::Spaces(n) => n,
            Indent::Tab => 0,
        };
        self.delete_ranges(EditKind::DeleteBackward, |doc, r| {
            if !r.is_empty() {
                return (r.from(), r.to());
            }
            let at = r.head;
            let line = doc.rope.char_to_line(at);
            let start = motion::line_start(&doc.rope, line);
            let before = doc.rope.slice(start..at);
            if unit > 0 && at > start && before.chars().all(|c| c == ' ') {
                let col = at - start;
                let stop = (col - 1) / unit * unit;
                return (start + stop, at);
            }
            (motion::prev_grapheme(&doc.rope, at), at)
        });
    }

    /// Delete (forward): the selection, or the grapheme after each cursor.
    pub fn delete_forward(&mut self) {
        self.delete_ranges(EditKind::DeleteForward, |doc, r| {
            if r.is_empty() {
                (r.head, motion::next_grapheme(&doc.rope, r.head))
            } else {
                (r.from(), r.to())
            }
        });
    }

    /// `⌥⌫`: the selection, or back to the start of the word.
    pub fn delete_word_backward(&mut self) {
        self.delete_ranges(EditKind::Other, |doc, r| {
            if r.is_empty() {
                (motion::word_left(&doc.rope, r.head), r.head)
            } else {
                (r.from(), r.to())
            }
        });
    }

    /// `⌘⌫`: the selection, or back to the start of the line.
    pub fn delete_to_line_start(&mut self) {
        self.delete_ranges(EditKind::Other, |doc, r| {
            if !r.is_empty() {
                return (r.from(), r.to());
            }
            let line = doc.rope.char_to_line(r.head);
            let start = motion::line_start(&doc.rope, line);
            if start == r.head {
                (motion::prev_grapheme(&doc.rope, r.head), r.head)
            } else {
                (start, r.head)
            }
        });
    }

    fn delete_ranges(
        &mut self,
        kind: EditKind,
        span: impl Fn(&Document, &Range) -> (usize, usize),
    ) {
        let mut spans: Vec<(usize, usize)> = self
            .selection
            .ranges()
            .iter()
            .map(|r| span(self, r))
            .collect();
        // Cursors whose deletions overlap (two cursors one char apart) delete
        // the union once.
        spans.sort();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (a, b) in spans {
            match merged.last_mut() {
                Some(last) if a <= last.1 => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        let tx = Transaction::new(merged.iter().map(|(a, b)| Change::delete(*a, *b)).collect());
        let after = self
            .selection
            .transform(|r| Range::cursor(tx.map(r.from().min(r.head), Assoc::Before)));
        let after = Selection::new(
            merged
                .iter()
                .map(|(a, _)| Range::cursor(tx.map(*a, Assoc::Before)))
                .collect(),
            after.primary_index().min(merged.len().saturating_sub(1)),
        );
        self.apply(tx, after, kind);
    }

    /// The lines any selection touches (a selection ending at column 0 does
    /// not include that line).
    fn selected_lines(&self) -> Vec<usize> {
        let mut lines = Vec::new();
        for r in self.selection.ranges() {
            let first = self.rope.char_to_line(r.from());
            let mut last = self.rope.char_to_line(r.to());
            if last > first && motion::line_start(&self.rope, last) == r.to() {
                last -= 1;
            }
            for l in first..=last {
                if lines.last() != Some(&l) {
                    lines.push(l);
                }
            }
        }
        lines.dedup();
        lines
    }

    /// Tab: with a selection spanning lines, indent them; otherwise insert an
    /// indent unit (spaces up to the next stop) at each cursor.
    pub fn indent(&mut self) {
        let multi_line = self
            .selection
            .ranges()
            .iter()
            .any(|r| self.rope.char_to_line(r.from()) != self.rope.char_to_line(r.to()));
        if !multi_line {
            let unit = self.indent;
            let tx = Transaction::new(
                self.selection
                    .ranges()
                    .iter()
                    .map(|r| {
                        let text = match unit {
                            Indent::Tab => "\t".to_string(),
                            Indent::Spaces(n) => {
                                let line = self.rope.char_to_line(r.from());
                                let col = layout::visual_col(
                                    self.rope.line(line),
                                    r.from() - motion::line_start(&self.rope, line),
                                    self.tab_width,
                                );
                                " ".repeat(n - col % n)
                            }
                        };
                        Change::replace(r.from(), r.to(), text)
                    })
                    .collect(),
            );
            let after = self
                .selection
                .transform(|r| Range::cursor(tx.map(r.to(), Assoc::After)));
            self.apply(tx, after, EditKind::Other);
            return;
        }
        let unit = self.indent.unit();
        let tx = Transaction::new(
            self.selected_lines()
                .into_iter()
                .filter(|l| layout::content_len(self.rope.line(*l)) > 0)
                .map(|l| Change::insert(motion::line_start(&self.rope, l), unit.clone()))
                .collect(),
        );
        let after = self.selection.map(&tx);
        self.apply(tx, after, EditKind::Other);
    }

    /// Shift-Tab: remove one indent level from every selected line.
    pub fn outdent(&mut self) {
        let unit = self.indent;
        let tx = Transaction::new(
            self.selected_lines()
                .into_iter()
                .filter_map(|l| {
                    let start = motion::line_start(&self.rope, l);
                    let line = self.rope.line(l);
                    let remove = match line.chars().next() {
                        Some('\t') => 1,
                        Some(' ') => {
                            let max = match unit {
                                Indent::Spaces(n) => n,
                                Indent::Tab => self.tab_width,
                            };
                            line.chars().take(max).take_while(|c| *c == ' ').count()
                        }
                        _ => 0,
                    };
                    (remove > 0).then(|| Change::delete(start, start + remove))
                })
                .collect(),
        );
        let after = self.selection.transform(|r| Range {
            anchor: tx.map(r.anchor, Assoc::Before),
            head: tx.map(r.head, Assoc::Before),
            goal_col: None,
        });
        self.apply(tx, after, EditKind::Other);
    }

    /// Move every head; `extend` keeps the anchors (shift held). Without
    /// `extend`, Left/Right on a selection collapse it to that side.
    pub fn move_cursor(&mut self, motion: Motion, extend: bool) {
        let rope = &self.rope;
        let tab = self.tab_width;
        let sel = self.selection.transform(|r| {
            if !extend && !r.is_empty() {
                match motion {
                    Motion::Left => return Range::cursor(r.from()),
                    Motion::Right => return Range::cursor(r.to()),
                    _ => {}
                }
            }
            match motion {
                Motion::Up | Motion::Down | Motion::PageUp(_) | Motion::PageDown(_) => {
                    let delta = match motion {
                        Motion::Up => -1,
                        Motion::Down => 1,
                        Motion::PageUp(n) => -(n.max(1) as isize),
                        Motion::PageDown(n) => n.max(1) as isize,
                        _ => 0,
                    };
                    let (head, goal) = motion::vertical(rope, r.head, delta, r.goal_col, tab);
                    Range {
                        anchor: if extend { r.anchor } else { head },
                        head,
                        goal_col: Some(goal),
                    }
                }
                _ => {
                    let head = match motion {
                        Motion::Left => motion::prev_grapheme(rope, r.head),
                        Motion::Right => motion::next_grapheme(rope, r.head),
                        Motion::WordLeft => motion::word_left(rope, r.head),
                        Motion::WordRight => motion::word_right(rope, r.head),
                        Motion::LineStart => motion::smart_home(rope, r.head),
                        Motion::LineEnd => motion::line_end(rope, rope.char_to_line(r.head)),
                        Motion::DocStart => 0,
                        Motion::DocEnd => rope.len_chars(),
                        _ => r.head,
                    };
                    r.moved_to(head, extend)
                }
            }
        });
        self.selection = sel;
        self.history.break_group();
    }

    pub fn select_all(&mut self) {
        self.set_selection(Selection::single(Range::new(0, self.rope.len_chars())));
    }

    /// `⌘L`: extend each selection to whole lines (repeat to add the next).
    pub fn select_line(&mut self) {
        let rope = &self.rope;
        let sel = self.selection.transform(|r| {
            let first = rope.char_to_line(r.from());
            let last = rope.char_to_line(r.to());
            let start = motion::line_start(rope, first);
            let mut end = if last + 1 < rope.len_lines() {
                rope.line_to_char(last + 1)
            } else {
                rope.len_chars()
            };
            if r.from() == start && r.to() == end && last + 1 < rope.len_lines() {
                // Already whole lines: take the next one too.
                end = if last + 2 < rope.len_lines() {
                    rope.line_to_char(last + 2)
                } else {
                    rope.len_chars()
                };
            }
            Range::new(start, end)
        });
        self.set_selection(sel);
    }

    /// `⌥⌘↓` / `⌥⌘↑`: a new cursor on the line below the last (or above the
    /// first) cursor, at the same column.
    pub fn add_cursor(&mut self, below: bool) {
        let ranges = self.selection.ranges();
        let edge = if below {
            ranges[ranges.len() - 1]
        } else {
            ranges[0]
        };
        let line = self.rope.char_to_line(edge.head);
        if (below && line >= motion::last_line(&self.rope)) || (!below && line == 0) {
            return;
        }
        let (head, goal) = motion::vertical(
            &self.rope,
            edge.head,
            if below { 1 } else { -1 },
            edge.goal_col,
            self.tab_width,
        );
        let sel = self.selection.push(Range {
            anchor: head,
            head,
            goal_col: Some(goal),
        });
        self.set_selection(sel);
    }

    /// `⌘D`: with a bare cursor, select the word under it; otherwise add the
    /// next occurrence of the primary selection's text. False when there is
    /// nothing (more) to add.
    pub fn select_next_occurrence(&mut self) -> bool {
        let primary = self.selection.primary();
        if primary.is_empty() {
            let (start, end) = motion::word_at(&self.rope, primary.head);
            if start == end {
                return false;
            }
            let sel = self.selection.transform(|r| {
                if r == primary {
                    Range::new(start, end)
                } else {
                    r
                }
            });
            self.set_selection(sel);
            return true;
        }
        let needle: String = self.rope.slice(primary.from()..primary.to()).into();
        let Ok(hits) = search::find_all(&self.rope, &SearchQuery::literal(needle)) else {
            return false;
        };
        let last = self
            .selection
            .ranges()
            .iter()
            .map(|r| r.to())
            .max()
            .unwrap_or(0);
        let taken = |h: &(usize, usize)| {
            self.selection
                .ranges()
                .iter()
                .any(|r| r.from() == h.0 && r.to() == h.1)
        };
        let next = hits
            .iter()
            .find(|h| h.0 >= last && !taken(h))
            .or_else(|| hits.iter().find(|h| !taken(h)));
        match next {
            Some(&(a, b)) => {
                let sel = self.selection.push(Range::new(a, b));
                self.set_selection(sel);
                true
            }
            None => false,
        }
    }

    /// `⇧⌘L`: select every occurrence of the primary selection's text, or of
    /// the word under a bare cursor (whole words only, as `⌘D` picks words).
    /// False when there is nothing to select.
    pub fn select_all_occurrences(&mut self) -> bool {
        let primary = self.selection.primary();
        let query = if primary.is_empty() {
            let (start, end) = motion::word_at(&self.rope, primary.head);
            if start == end {
                return false;
            }
            SearchQuery {
                whole_word: true,
                ..SearchQuery::literal(String::from(self.rope.slice(start..end)))
            }
        } else {
            SearchQuery::literal(String::from(self.rope.slice(primary.from()..primary.to())))
        };
        match search::find_all(&self.rope, &query) {
            Ok(hits) if !hits.is_empty() => {
                self.select_ranges(&hits, primary.head);
                true
            }
            _ => false,
        }
    }

    /// Select each of `ranges` (search matches), the primary being the one
    /// at or after `near` (wrapping to the first). Nothing for none.
    pub fn select_ranges(&mut self, ranges: &[(usize, usize)], near: usize) {
        if ranges.is_empty() {
            return;
        }
        let primary = ranges.iter().position(|&(_, end)| end >= near).unwrap_or(0);
        let sel = Selection::new(
            ranges.iter().map(|&(a, b)| Range::new(a, b)).collect(),
            primary,
        );
        self.set_selection(sel);
    }

    /// `⇧⌥I`: a cursor at the end of every line a selection covers (the last
    /// line's at the selection's end). A selection within one line becomes a
    /// cursor at its end.
    pub fn cursors_at_line_ends(&mut self) {
        let rope = &self.rope;
        let mut ranges = Vec::new();
        let mut primary = 0;
        for (i, r) in self.selection.ranges().iter().enumerate() {
            let first = rope.char_to_line(r.from());
            let mut last = rope.char_to_line(r.to());
            if last > first && motion::line_start(rope, last) == r.to() {
                last -= 1;
            }
            if i == self.selection.primary_index() {
                primary = ranges.len();
            }
            for line in first..last {
                ranges.push(Range::cursor(motion::line_end(rope, line)));
            }
            ranges.push(Range::cursor(r.to().min(motion::line_end(rope, last))));
        }
        let sel = Selection::new(ranges, primary);
        self.set_selection(sel);
    }

    /// Replace the match `start..end` of `query` with `replacement` (groups
    /// expanded), one undo step, and put the caret after it. `Ok(None)` when
    /// the range is no longer a match.
    pub fn replace_match(
        &mut self,
        query: &SearchQuery,
        start: usize,
        end: usize,
        replacement: &str,
    ) -> Result<Option<usize>, search::SearchError> {
        let Some(text) = search::replacement_at(&self.rope, query, start, end, replacement)? else {
            return Ok(None);
        };
        let tx = Transaction::new(vec![Change::replace(start, end, text)]);
        let after = tx.map(end, Assoc::After);
        self.apply(tx, Selection::cursor(after), EditKind::Other);
        Ok(Some(after))
    }

    /// Escape with several cursors: keep the primary only.
    pub fn collapse_to_primary(&mut self) {
        let sel = self.selection.clone().into_primary();
        self.set_selection(sel);
    }

    /// The selected text, ranges joined by the document's line ending (what
    /// copy puts on the clipboard).
    pub fn selected_text(&self) -> String {
        self.selection
            .ranges()
            .iter()
            .filter(|r| !r.is_empty())
            .map(|r| String::from(self.rope.slice(r.from()..r.to())))
            .collect::<Vec<_>>()
            .join(self.line_ending.as_str())
    }

    /// Replace every match of `query`; one undo step. Returns how many.
    pub fn replace_all(
        &mut self,
        query: &SearchQuery,
        replacement: &str,
    ) -> Result<usize, search::SearchError> {
        let tx = search::replace_all(&self.rope, query, replacement)?;
        let n = tx.changes().len();
        self.apply_external(tx);
        Ok(n)
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo(&mut self) -> bool {
        let Some((steps, selection)) = self.history.undo() else {
            return false;
        };
        for tx in &steps {
            self.edits.extend(tx.byte_edits(&self.rope));
            tx.apply(&mut self.rope);
        }
        self.revision += 1;
        self.selection = selection.clamp(self.rope.len_chars());
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some((steps, selection)) = self.history.redo() else {
            return false;
        };
        for tx in &steps {
            self.edits.extend(tx.byte_edits(&self.rope));
            tx.apply(&mut self.rope);
        }
        self.revision += 1;
        self.selection = selection.clamp(self.rope.len_chars());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(text: &str, cursor: usize) -> Document {
        let mut d = Document::from_text(text);
        d.set_selection(Selection::cursor(cursor));
        d
    }

    fn text(d: &Document) -> String {
        d.rope().to_string()
    }

    fn heads(d: &Document) -> Vec<usize> {
        d.selection().ranges().iter().map(|r| r.head).collect()
    }

    #[test]
    fn typing_groups_into_one_undo_step() {
        let mut d = doc("", 0);
        for c in ["h", "e", "l", "l", "o"] {
            d.type_text(c);
        }
        assert_eq!(text(&d), "hello");
        assert!(d.is_modified());
        assert!(d.undo());
        assert_eq!(text(&d), "");
        assert!(!d.is_modified(), "back at the saved state");
        assert!(d.redo());
        assert_eq!(text(&d), "hello");
        assert_eq!(heads(&d), vec![5]);
    }

    #[test]
    fn moving_the_cursor_ends_a_typing_group() {
        let mut d = doc("", 0);
        d.type_text("ab");
        d.move_cursor(Motion::Left, false);
        d.type_text("X");
        assert_eq!(text(&d), "aXb");
        d.undo();
        assert_eq!(text(&d), "ab");
        d.undo();
        assert_eq!(text(&d), "");
    }

    #[test]
    fn saved_state_survives_undo_and_redo() {
        let mut d = doc("x", 1);
        d.type_text("y");
        d.mark_saved();
        assert!(!d.is_modified());
        d.undo();
        assert!(d.is_modified());
        d.redo();
        assert!(!d.is_modified());
    }

    #[test]
    fn newline_keeps_the_indent_and_line_ending() {
        let mut d = doc("    fn a() {\r\n}\r\n", 12);
        d.newline();
        assert_eq!(text(&d), "    fn a() {\r\n    \r\n}\r\n");
        assert_eq!(heads(&d), vec![18]);
    }

    #[test]
    fn backspace_in_leading_spaces_goes_to_the_indent_stop() {
        let mut d = doc("fn a() {\n    if x {\n        x\n    }\n}\n", 28);
        assert_eq!(d.indent_style(), Indent::Spaces(4));
        d.delete_backward();
        assert_eq!(text(&d), "fn a() {\n    if x {\n    x\n    }\n}\n");
        d.delete_backward();
        d.delete_backward();
        assert_eq!(text(&d), "fn a() {\n    if x {x\n    }\n}\n");
    }

    #[test]
    fn multiple_cursors_type_and_delete_together() {
        let mut d = doc("a\nb\nc\n", 1);
        d.add_cursor(true);
        d.add_cursor(true);
        assert_eq!(heads(&d), vec![1, 3, 5]);
        d.type_text(";");
        assert_eq!(text(&d), "a;\nb;\nc;\n");
        d.delete_backward();
        assert_eq!(text(&d), "a\nb\nc\n");
        assert_eq!(heads(&d), vec![1, 3, 5]);
    }

    #[test]
    fn select_next_occurrence_then_type_replaces_all() {
        let mut d = doc("let x = x + x;", 4);
        assert!(d.select_next_occurrence(), "selects the word");
        assert!(d.select_next_occurrence());
        assert!(d.select_next_occurrence());
        assert!(!d.select_next_occurrence(), "nothing left");
        d.type_text("y");
        assert_eq!(text(&d), "let y = y + y;");
    }

    #[test]
    fn paste_splits_lines_across_cursors() {
        let mut d = doc("a\nb\n", 1);
        d.add_cursor(true);
        d.paste("1\n2\n");
        assert_eq!(text(&d), "a1\nb2\n");
        let mut d = doc("ab", 1);
        d.paste("x\ny");
        assert_eq!(text(&d), "ax\nyb");
    }

    #[test]
    fn indent_and_outdent_lines() {
        let mut d = doc("a\nb\nc\n", 0);
        d.set_selection(Selection::single(Range::new(0, 3)));
        d.indent();
        assert_eq!(text(&d), "    a\n    b\nc\n");
        d.outdent();
        assert_eq!(text(&d), "a\nb\nc\n");
        let mut d = doc("ab", 1);
        d.indent();
        assert_eq!(text(&d), "a   b", "spaces to the next stop");
    }

    #[test]
    fn word_and_line_deletes() {
        let mut d = doc("let foo_bar", 11);
        d.delete_word_backward();
        assert_eq!(text(&d), "let ");
        let mut d = doc("abc def", 7);
        d.delete_to_line_start();
        assert_eq!(text(&d), "");
    }

    #[test]
    fn select_line_grows_by_lines() {
        let mut d = doc("one\ntwo\nthree", 1);
        d.select_line();
        assert_eq!(d.selected_text(), "one\n");
        d.select_line();
        assert_eq!(d.selected_text(), "one\ntwo\n");
    }

    #[test]
    fn external_edits_carry_cursors_and_undo_in_one_step() {
        let mut d = doc("hello world", 6);
        d.apply_external(Transaction::new(vec![Change::insert(0, ">> ")]));
        assert_eq!(text(&d), ">> hello world");
        assert_eq!(heads(&d), vec![9]);
        d.undo();
        assert_eq!(text(&d), "hello world");
    }

    #[test]
    fn select_all_occurrences_of_a_word_or_the_selection() {
        let mut d = doc("let x = x + xx; x", 8);
        assert!(d.select_all_occurrences());
        let ranges: Vec<_> = d
            .selection()
            .ranges()
            .iter()
            .map(|r| (r.from(), r.to()))
            .collect();
        assert_eq!(
            ranges,
            vec![(4, 5), (8, 9), (16, 17)],
            "whole words from a cursor"
        );
        assert_eq!(d.selection().primary(), Range::new(8, 9));
        d.type_text("y");
        assert_eq!(text(&d), "let y = y + xx; y");
        let mut d = doc("ab ab-ab", 0);
        d.set_selection(Selection::single(Range::new(0, 2)));
        assert!(d.select_all_occurrences());
        assert_eq!(d.selection().len(), 3, "a selection matches anywhere");
        let mut d = doc("   ", 1);
        assert!(!d.select_all_occurrences(), "no word under the cursor");
    }

    #[test]
    fn cursors_go_to_the_ends_of_selected_lines() {
        let mut d = doc("one\ntwo\nthree\n", 1);
        d.set_selection(Selection::single(Range::new(1, 10)));
        d.cursors_at_line_ends();
        assert_eq!(heads(&d), vec![3, 7, 10]);
        d.type_text(";");
        assert_eq!(text(&d), "one;\ntwo;\nth;ree\n");
        // A selection ending at a line start does not take that line.
        let mut d = doc("a\nb\nc", 0);
        d.set_selection(Selection::single(Range::new(0, 4)));
        d.cursors_at_line_ends();
        assert_eq!(heads(&d), vec![1, 3]);
    }

    #[test]
    fn replacing_one_match_is_an_undo_step_and_moves_past_it() {
        let mut d = doc("a-b-c", 0);
        let q = SearchQuery::literal("-");
        assert_eq!(d.replace_match(&q, 3, 4, "+").unwrap(), Some(4));
        assert_eq!(text(&d), "a-b+c");
        assert_eq!(heads(&d), vec![4]);
        assert_eq!(d.replace_match(&q, 3, 4, "+").unwrap(), None, "stale");
        d.undo();
        assert_eq!(text(&d), "a-b-c");
    }

    #[test]
    fn replace_all_is_one_undo_step() {
        let mut d = doc("a-b-c", 0);
        let n = d.replace_all(&SearchQuery::literal("-"), "+").unwrap();
        assert_eq!((n, text(&d)), (2, "a+b+c".to_string()));
        d.undo();
        assert_eq!(text(&d), "a-b-c");
    }

    #[test]
    fn left_right_collapse_a_selection_and_shift_extends() {
        let mut d = doc("hello", 1);
        d.move_cursor(Motion::WordRight, true);
        assert_eq!(d.selected_text(), "ello");
        d.move_cursor(Motion::Left, false);
        assert_eq!(heads(&d), vec![1]);
    }

    #[test]
    fn bytes_round_trip() {
        let bytes = "\u{feff}line\r\n".as_bytes();
        let mut d = Document::from_bytes(bytes).unwrap();
        assert_eq!(d.line_ending(), LineEnding::CrLf);
        assert_eq!(d.to_bytes(), bytes);
        d.set_selection(Selection::cursor(4));
        d.newline();
        assert_eq!(d.to_bytes(), "\u{feff}line\r\n\r\n".as_bytes());
    }
}
