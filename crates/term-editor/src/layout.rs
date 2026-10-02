//! Lines on the monospace cell grid: tabs expand to the next tab stop, wide
//! (CJK, emoji) characters take two cells, combining marks none.

use ropey::RopeSlice;
use unicode_width::UnicodeWidthChar;

/// Cells a character takes when it is not a tab.
pub fn char_width(c: char) -> usize {
    if c.is_control() {
        // Shown as a replacement glyph by the renderer.
        1
    } else {
        c.width().unwrap_or(0)
    }
}

/// The chars of `line` without its trailing line break (`\n` or `\r\n`).
pub fn content_len(line: RopeSlice) -> usize {
    let mut len = line.len_chars();
    if len > 0 && line.char(len - 1) == '\n' {
        len -= 1;
        if len > 0 && line.char(len - 1) == '\r' {
            len -= 1;
        }
    }
    len
}

/// The cell column where the char at `offset` (within `line`) starts.
pub fn visual_col(line: RopeSlice, offset: usize, tab_width: usize) -> usize {
    let mut col = 0;
    for c in line.chars().take(offset) {
        col = advance(col, c, tab_width);
    }
    col
}

/// The char offset in `line` whose cell is at or before `col`, never past the
/// line's content (vertical moves land there).
pub fn offset_at_col(line: RopeSlice, col: usize, tab_width: usize) -> usize {
    let end = content_len(line);
    let mut at = 0;
    for (i, c) in line.chars().take(end).enumerate() {
        let next = advance(at, c, tab_width);
        if next > col {
            // Round to the nearer edge of a wide char or tab.
            return if col - at >= next - col { i + 1 } else { i };
        }
        at = next;
        if at == col {
            return i + 1;
        }
    }
    end
}

fn advance(col: usize, c: char, tab_width: usize) -> usize {
    if c == '\t' {
        let tab = tab_width.max(1);
        (col / tab + 1) * tab
    } else {
        col + char_width(c)
    }
}

/// One drawn character of a line: its char offset, cell column and width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Glyph {
    pub offset: usize,
    pub col: usize,
    pub width: usize,
    pub ch: char,
}

/// The glyphs of `line` up to `max_cols` cells (a tab is one glyph spanning
/// its cells; the line break is not drawn).
pub fn glyphs(line: RopeSlice, tab_width: usize, max_cols: usize) -> Vec<Glyph> {
    let end = content_len(line);
    let mut out = Vec::new();
    let mut col = 0;
    for (offset, ch) in line.chars().take(end).enumerate() {
        if col >= max_cols {
            break;
        }
        let next = advance(col, ch, tab_width);
        out.push(Glyph {
            offset,
            col,
            width: next - col,
            ch,
        });
        col = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ropey::Rope;

    #[test]
    fn tabs_and_wide_chars_take_their_cells() {
        let rope = Rope::from_str("a\tb中c\n");
        let line = rope.line(0);
        assert_eq!(visual_col(line, 1, 4), 1);
        assert_eq!(visual_col(line, 2, 4), 4, "tab to the next stop");
        assert_eq!(visual_col(line, 4, 4), 7, "wide char is two cells");
        assert_eq!(content_len(line), 5);
        let g = glyphs(line, 4, 80);
        assert_eq!(g.len(), 5);
        assert_eq!((g[1].col, g[1].width), (1, 3));
        assert_eq!((g[3].ch, g[3].col, g[3].width), ('中', 5, 2));
    }

    #[test]
    fn columns_map_back_to_offsets() {
        let rope = Rope::from_str("中文ab\r\n");
        let line = rope.line(0);
        assert_eq!(content_len(line), 4, "CRLF is not content");
        assert_eq!(offset_at_col(line, 0, 4), 0);
        assert_eq!(offset_at_col(line, 2, 4), 1);
        assert_eq!(
            offset_at_col(line, 3, 4),
            2,
            "middle of a wide char rounds up"
        );
        assert_eq!(
            offset_at_col(line, 99, 4),
            4,
            "past the end stops at content end"
        );
    }
}
