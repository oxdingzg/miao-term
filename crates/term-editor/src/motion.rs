//! Cursor motions over a rope: graphemes, words, lines.

use ropey::{Rope, RopeSlice};
use unicode_segmentation::{GraphemeCursor, GraphemeIncomplete};

use crate::layout;

/// The char index after the grapheme at `at` (a user-perceived character:
/// `\r\n`, a letter with combining marks, an emoji sequence).
pub fn next_grapheme(rope: &Rope, at: usize) -> usize {
    if at >= rope.len_chars() {
        return rope.len_chars();
    }
    let byte = rope.char_to_byte(at);
    rope.byte_to_char(next_boundary(rope.slice(..), byte))
}

/// The char index where the grapheme before `at` starts.
pub fn prev_grapheme(rope: &Rope, at: usize) -> usize {
    if at == 0 {
        return 0;
    }
    let byte = rope.char_to_byte(at.min(rope.len_chars()));
    rope.byte_to_char(prev_boundary(rope.slice(..), byte))
}

fn next_boundary(slice: RopeSlice, byte: usize) -> usize {
    let (mut chunk, mut chunk_start, _, _) = slice.chunk_at_byte(byte);
    let mut cursor = GraphemeCursor::new(byte, slice.len_bytes(), true);
    loop {
        match cursor.next_boundary(chunk, chunk_start) {
            Ok(None) => return slice.len_bytes(),
            Ok(Some(n)) => return n,
            Err(GraphemeIncomplete::NextChunk) => {
                chunk_start += chunk.len();
                chunk = slice.chunk_at_byte(chunk_start).0;
            }
            Err(GraphemeIncomplete::PreContext(n)) => {
                let context = slice.chunk_at_byte(n - 1).0;
                cursor.provide_context(context, n - context.len());
            }
            // The cursor only asks for the chunks above.
            Err(_) => return (byte + 1).min(slice.len_bytes()),
        }
    }
}

fn prev_boundary(slice: RopeSlice, byte: usize) -> usize {
    let (mut chunk, mut chunk_start, _, _) = slice.chunk_at_byte(byte);
    let mut cursor = GraphemeCursor::new(byte, slice.len_bytes(), true);
    loop {
        match cursor.prev_boundary(chunk, chunk_start) {
            Ok(None) => return 0,
            Ok(Some(n)) => return n,
            Err(GraphemeIncomplete::PrevChunk) => {
                let (c, start, _, _) = slice.chunk_at_byte(chunk_start - 1);
                chunk = c;
                chunk_start = start;
            }
            Err(GraphemeIncomplete::PreContext(n)) => {
                let context = slice.chunk_at_byte(n - 1).0;
                cursor.provide_context(context, n - context.len());
            }
            Err(_) => return byte.saturating_sub(1),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Punct,
    Space,
    Newline,
}

fn class(c: char) -> Class {
    if c == '\n' || c == '\r' {
        Class::Newline
    } else if c.is_whitespace() {
        Class::Space
    } else if c.is_alphanumeric() || c == '_' {
        Class::Word
    } else {
        Class::Punct
    }
}

/// `⌥→`: past any spaces, then to the end of the word (or punctuation run).
/// At a line end it moves to the next line's start.
pub fn word_right(rope: &Rope, at: usize) -> usize {
    let len = rope.len_chars();
    let mut i = at;
    if i < len && class(rope.char(i)) == Class::Newline {
        return next_grapheme(rope, i);
    }
    while i < len && class(rope.char(i)) == Class::Space {
        i += 1;
    }
    if i < len {
        let kind = class(rope.char(i));
        if kind == Class::Newline {
            return i;
        }
        while i < len && class(rope.char(i)) == kind {
            i += 1;
        }
    }
    i
}

/// `⌥←`: back over spaces, then to the start of the word (or punctuation run).
/// At a line start it moves to the previous line's end.
pub fn word_left(rope: &Rope, at: usize) -> usize {
    let mut i = at.min(rope.len_chars());
    if i > 0 && class(rope.char(i - 1)) == Class::Newline {
        return prev_grapheme(rope, i);
    }
    while i > 0 && class(rope.char(i - 1)) == Class::Space {
        i -= 1;
    }
    if i > 0 {
        let kind = class(rope.char(i - 1));
        if kind == Class::Newline {
            return i;
        }
        while i > 0 && class(rope.char(i - 1)) == kind {
            i -= 1;
        }
    }
    i
}

/// The word (letters, digits, `_`) around `at`, as a char range; empty when
/// `at` is not next to one.
pub fn word_at(rope: &Rope, at: usize) -> (usize, usize) {
    let len = rope.len_chars();
    let is_word = |i: usize| i < len && class(rope.char(i)) == Class::Word;
    let mut start = at.min(len);
    let mut end = start;
    while start > 0 && is_word(start - 1) {
        start -= 1;
    }
    while is_word(end) {
        end += 1;
    }
    (start, end)
}

/// The first char of `line`.
pub fn line_start(rope: &Rope, line: usize) -> usize {
    rope.line_to_char(line)
}

/// Where `line`'s content ends (before its line break).
pub fn line_end(rope: &Rope, line: usize) -> usize {
    rope.line_to_char(line) + layout::content_len(rope.line(line))
}

/// `⌘←` / Home: to the first non-blank char, or to column 0 when already there.
pub fn smart_home(rope: &Rope, at: usize) -> usize {
    let line = rope.char_to_line(at);
    let start = line_start(rope, line);
    let end = line_end(rope, line);
    let mut first = start;
    while first < end && matches!(rope.char(first), ' ' | '\t') {
        first += 1;
    }
    if at == first {
        start
    } else {
        first
    }
}

/// Up (`delta < 0`) or down by `delta` lines, aiming at `goal_col`. Returns
/// the new position and the goal to keep. Moving up from the first line goes
/// to its start; down from the last line to its end.
pub fn vertical(
    rope: &Rope,
    at: usize,
    delta: isize,
    goal_col: Option<usize>,
    tab_width: usize,
) -> (usize, usize) {
    let line = rope.char_to_line(at);
    let col = goal_col.unwrap_or_else(|| {
        layout::visual_col(rope.line(line), at - line_start(rope, line), tab_width)
    });
    let last = last_line(rope);
    let target = line as isize + delta;
    if target < 0 {
        return (0, col);
    }
    if target as usize > last {
        return (rope.len_chars(), col);
    }
    let target = target as usize;
    let offset = layout::offset_at_col(rope.line(target), col, tab_width);
    (line_start(rope, target) + offset, col)
}

/// The index of the last line (a trailing line break does not open a line
/// with nothing in it for the cursor to be denied: it is the last line).
pub fn last_line(rope: &Rope) -> usize {
    rope.len_lines().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graphemes_keep_crlf_and_clusters_whole() {
        let rope = Rope::from_str("a\r\ne\u{301}👍🏽!");
        assert_eq!(next_grapheme(&rope, 1), 3, "CRLF is one step");
        assert_eq!(next_grapheme(&rope, 3), 5, "e + combining accent");
        assert_eq!(next_grapheme(&rope, 5), 7, "emoji with skin tone");
        assert_eq!(prev_grapheme(&rope, 7), 5);
        assert_eq!(prev_grapheme(&rope, 3), 1);
        assert_eq!(prev_grapheme(&rope, 0), 0);
        assert_eq!(next_grapheme(&rope, rope.len_chars()), rope.len_chars());
    }

    #[test]
    fn word_motions_skip_spaces_then_a_run() {
        let rope = Rope::from_str("let foo_bar = x.y;\nnext");
        assert_eq!(word_right(&rope, 0), 3);
        assert_eq!(word_right(&rope, 3), 11);
        assert_eq!(word_right(&rope, 11), 13, "punctuation is its own run");
        assert_eq!(word_right(&rope, 18), 19, "line end goes to the next line");
        assert_eq!(word_left(&rope, 11), 4);
        assert_eq!(
            word_left(&rope, 19),
            18,
            "line start goes to the previous end"
        );
        assert_eq!(word_at(&rope, 6), (4, 11));
        assert_eq!(word_at(&rope, 12), (12, 12));
    }

    #[test]
    fn home_toggles_between_indent_and_column_zero() {
        let rope = Rope::from_str("    code\n");
        assert_eq!(smart_home(&rope, 7), 4);
        assert_eq!(smart_home(&rope, 4), 0);
        assert_eq!(line_end(&rope, 0), 8);
    }

    #[test]
    fn vertical_moves_keep_the_goal_column() {
        let rope = Rope::from_str("long line here\nab\nanother long\n");
        let (p, goal) = vertical(&rope, 10, 1, None, 4);
        assert_eq!((p, goal), (17, 10), "clamped to the short line's end");
        let (p, _) = vertical(&rope, p, 1, Some(goal), 4);
        assert_eq!(p, 28, "back to column 10 on a long line");
        assert_eq!(
            vertical(&rope, 3, -1, None, 4).0,
            0,
            "up from the first line"
        );
        assert_eq!(vertical(&rope, 20, 9, None, 4).0, rope.len_chars());
    }
}
