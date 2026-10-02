//! Changes to a rope, grouped into transactions that can be inverted and that
//! map positions from before an edit to after it.

use ropey::Rope;

/// Replace the chars `start..end` of the document *before* the transaction
/// with `text`. An insertion has `start == end`; a deletion an empty `text`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Change {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

impl Change {
    pub fn insert(at: usize, text: impl Into<String>) -> Self {
        Change {
            start: at,
            end: at,
            text: text.into(),
        }
    }

    pub fn delete(start: usize, end: usize) -> Self {
        Change {
            start,
            end,
            text: String::new(),
        }
    }

    pub fn replace(start: usize, end: usize, text: impl Into<String>) -> Self {
        Change {
            start,
            end,
            text: text.into(),
        }
    }

    fn inserted_chars(&self) -> usize {
        self.text.chars().count()
    }
}

/// Which side a position sticks to when text is inserted exactly there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Assoc {
    /// Stay before the inserted text.
    Before,
    /// Move after the inserted text.
    After,
}

/// Changes applied together, sorted by position and never overlapping (two
/// changes may touch: one ending where the next starts).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Transaction {
    changes: Vec<Change>,
}

impl Transaction {
    /// Sort the changes and drop no-ops. Panics if two of them overlap, which
    /// would make the result depend on the order they were given in.
    pub fn new(mut changes: Vec<Change>) -> Self {
        changes.retain(|c| c.start != c.end || !c.text.is_empty());
        changes.sort_by_key(|c| (c.start, c.end));
        for pair in changes.windows(2) {
            assert!(
                pair[0].end <= pair[1].start,
                "overlapping changes {:?} and {:?}",
                pair[0],
                pair[1]
            );
        }
        Transaction { changes }
    }

    pub fn changes(&self) -> &[Change] {
        &self.changes
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// Apply to `rope` and return the transaction that undoes it.
    pub fn apply(&self, rope: &mut Rope) -> Transaction {
        let mut inverse = Vec::with_capacity(self.changes.len());
        let mut delta: isize = 0;
        for c in &self.changes {
            let removed: String = rope.slice(c.start..c.end).into();
            let new_start = (c.start as isize + delta) as usize;
            let len = c.inserted_chars();
            inverse.push(Change::replace(new_start, new_start + len, removed));
            delta += len as isize - (c.end - c.start) as isize;
        }
        // Back to front, so earlier offsets stay valid.
        for c in self.changes.iter().rev() {
            if c.end > c.start {
                rope.remove(c.start..c.end);
            }
            if !c.text.is_empty() {
                rope.insert(c.start, &c.text);
            }
        }
        Transaction { changes: inverse }
    }

    /// Where `pos` (before the transaction) ends up after it. A position
    /// inside replaced text goes to the start or end of the new text by
    /// `assoc`; one at the end of a replaced range goes to the end of its
    /// replacement.
    pub fn map(&self, pos: usize, assoc: Assoc) -> usize {
        let mut delta: isize = 0;
        for c in &self.changes {
            if pos < c.start {
                break;
            }
            let new_start = (c.start as isize + delta) as usize;
            let len = c.inserted_chars();
            if pos < c.end || pos == c.start {
                return match assoc {
                    Assoc::Before => new_start,
                    Assoc::After => new_start + len,
                };
            }
            delta += len as isize - (c.end - c.start) as isize;
        }
        (pos as isize + delta) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_and_invert_restores_the_text() {
        let mut rope = Rope::from_str("hello world");
        let tx = Transaction::new(vec![
            Change::replace(6, 11, "rope"),
            Change::insert(0, ">> "),
            Change::delete(4, 5),
        ]);
        let inverse = tx.apply(&mut rope);
        assert_eq!(rope.to_string(), ">> hell rope");
        inverse.apply(&mut rope);
        assert_eq!(rope.to_string(), "hello world");
    }

    #[test]
    fn positions_map_through_changes() {
        // "abcdef": insert "XY" at 2, delete 4..5.
        let tx = Transaction::new(vec![Change::insert(2, "XY"), Change::delete(4, 5)]);
        assert_eq!(tx.map(0, Assoc::Before), 0);
        assert_eq!(tx.map(2, Assoc::Before), 2);
        assert_eq!(tx.map(2, Assoc::After), 4);
        assert_eq!(tx.map(3, Assoc::After), 5);
        assert_eq!(tx.map(4, Assoc::Before), 6);
        assert_eq!(tx.map(5, Assoc::Before), 6, "end of a deletion");
        assert_eq!(tx.map(6, Assoc::Before), 7);
    }

    #[test]
    fn touching_changes_both_apply() {
        let mut rope = Rope::from_str("abcd");
        let tx = Transaction::new(vec![Change::replace(1, 2, "B"), Change::insert(2, "_")]);
        tx.apply(&mut rope);
        assert_eq!(rope.to_string(), "aB_cd");
        assert_eq!(tx.map(2, Assoc::After), 3);
    }

    #[test]
    #[should_panic(expected = "overlapping")]
    fn overlapping_changes_are_rejected() {
        Transaction::new(vec![Change::delete(1, 4), Change::delete(3, 5)]);
    }

    #[test]
    fn multibyte_text_counts_chars() {
        let mut rope = Rope::from_str("中文abc");
        let tx = Transaction::new(vec![Change::replace(1, 3, "字")]);
        let inv = tx.apply(&mut rope);
        assert_eq!(rope.to_string(), "中字bc");
        inv.apply(&mut rope);
        assert_eq!(rope.to_string(), "中文abc");
    }
}
