//! Grid selection helpers (pure geometry over the screen).

use miao_term_core::ATerm;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub start: (u16, u16),
    pub end: (u16, u16),
}

impl Selection {
    pub fn cell(c: (u16, u16)) -> Self {
        Self { start: c, end: c }
    }

    pub fn ordered(&self) -> (u16, u16, u16, u16) {
        if self.end < self.start {
            (self.end.0, self.end.1, self.start.0, self.start.1)
        } else {
            (self.start.0, self.start.1, self.end.0, self.end.1)
        }
    }

    /// All cell indices covered, for highlighting.
    pub fn cells(&self, cols: u16) -> impl Iterator<Item = (u16, u16)> + '_ {
        let (r1, c1, r2, c2) = self.ordered();
        let cols = cols as usize;
        let start = r1 as usize * cols + c1 as usize;
        let end = r2 as usize * cols + c2 as usize;
        (start..=end).map(move |i| ((i / cols) as u16, (i % cols) as u16))
    }
}

/// Select the word (alphanumeric plus `_-./~`) around a cell.
pub fn word_selection(screen: &ATerm, row: u16, col: u16, cols: u16) -> Selection {
    let is_word = |c: u16| -> bool {
        screen
            .cell(row, c)
            .map(|cell| cell.ch.is_alphanumeric() || "_-./~".contains(cell.ch))
            .unwrap_or(false)
    };
    if !is_word(col) {
        return Selection::cell((row, col));
    }
    let mut l = col;
    while l > 0 && is_word(l - 1) {
        l -= 1;
    }
    let mut r = col;
    while r + 1 < cols && is_word(r + 1) {
        r += 1;
    }
    Selection {
        start: (row, l),
        end: (row, r),
    }
}
