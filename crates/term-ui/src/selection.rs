//! Grid selection helpers (pure geometry over the screen).

use mtty_core::ATerm;

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
            .map(|cell| cell.wide_spacer || cell.ch.is_alphanumeric() || "_-./~".contains(cell.ch))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_click_path_including_wide_character_spacer() {
        let mut screen = ATerm::new(30, 3, 100);
        screen.process("目录/file.txt suffix".as_bytes());
        for col in [0, 1, 2, 3, 8] {
            let s = word_selection(&screen, 0, col, 30);
            let (r1, c1, r2, c2) = s.ordered();
            assert_eq!(screen.contents_between(r1, c1, r2, c2), "目录/file.txt");
        }
    }

    #[test]
    fn backwards_drag_and_highlight_cover_the_same_text() {
        let selection = Selection {
            start: (1, 2),
            end: (0, 3),
        };
        assert_eq!(
            selection.cells(5).collect::<Vec<_>>(),
            vec![(0, 3), (0, 4), (1, 0), (1, 1), (1, 2)]
        );
    }
}
