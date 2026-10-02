//! Selections: one or more ranges, each with an anchor and a moving head.

use crate::change::{Assoc, Transaction};

/// A selected span (or a bare cursor when `anchor == head`). `head` is where
/// the caret is; `goal_col` remembers the visual column vertical moves aim
/// for, so moving through a short line does not lose it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub anchor: usize,
    pub head: usize,
    pub goal_col: Option<usize>,
}

impl Range {
    pub fn cursor(at: usize) -> Self {
        Range {
            anchor: at,
            head: at,
            goal_col: None,
        }
    }

    pub fn new(anchor: usize, head: usize) -> Self {
        Range {
            anchor,
            head,
            goal_col: None,
        }
    }

    pub fn from(&self) -> usize {
        self.anchor.min(self.head)
    }

    pub fn to(&self) -> usize {
        self.anchor.max(self.head)
    }

    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }

    /// Move the head; with `extend` the anchor stays, otherwise the range
    /// collapses to the new position. Clears the goal column.
    pub fn moved_to(&self, head: usize, extend: bool) -> Range {
        Range {
            anchor: if extend { self.anchor } else { head },
            head,
            goal_col: None,
        }
    }

    fn overlaps_or_touches(&self, other: &Range) -> bool {
        // Two cursors at the same spot, or ranges sharing chars, merge;
        // non-empty ranges that only touch stay separate.
        if self.is_empty() || other.is_empty() {
            self.from() <= other.to() && other.from() <= self.to()
        } else {
            self.from() < other.to() && other.from() < self.to()
        }
    }
}

/// One or more ranges, sorted and merged, with one of them primary (the one
/// the view follows).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    ranges: Vec<Range>,
    primary: usize,
}

impl Selection {
    pub fn single(range: Range) -> Self {
        Selection {
            ranges: vec![range],
            primary: 0,
        }
    }

    pub fn cursor(at: usize) -> Self {
        Selection::single(Range::cursor(at))
    }

    /// Ranges in any order; overlapping ones merge. `primary` indexes `ranges`.
    pub fn new(ranges: Vec<Range>, primary: usize) -> Self {
        assert!(!ranges.is_empty(), "a selection has at least one range");
        let primary_range = ranges[primary.min(ranges.len() - 1)];
        let mut sorted = ranges;
        sorted.sort_by_key(|r| (r.from(), r.to()));
        let mut merged: Vec<Range> = Vec::with_capacity(sorted.len());
        for r in sorted {
            match merged.last_mut() {
                Some(last) if last.overlaps_or_touches(&r) => {
                    let from = last.from().min(r.from());
                    let to = last.to().max(r.to());
                    // Keep the direction of the range that was there first.
                    *last = if last.head < last.anchor {
                        Range::new(to, from)
                    } else {
                        Range::new(from, to)
                    };
                }
                _ => merged.push(r),
            }
        }
        let primary = merged
            .iter()
            .position(|r| r.from() <= primary_range.head && primary_range.head <= r.to())
            .unwrap_or(0);
        Selection {
            ranges: merged,
            primary,
        }
    }

    pub fn ranges(&self) -> &[Range] {
        &self.ranges
    }

    pub fn primary(&self) -> Range {
        self.ranges[self.primary]
    }

    pub fn primary_index(&self) -> usize {
        self.primary
    }

    pub fn len(&self) -> usize {
        self.ranges.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Apply `f` to every range, then re-sort and merge.
    pub fn transform(&self, mut f: impl FnMut(Range) -> Range) -> Selection {
        Selection::new(self.ranges.iter().map(|r| f(*r)).collect(), self.primary)
    }

    /// Keep only the primary range.
    pub fn into_primary(self) -> Selection {
        Selection::single(self.primary())
    }

    /// Add a range and make it primary.
    pub fn push(&self, range: Range) -> Selection {
        let mut ranges = self.ranges.clone();
        ranges.push(range);
        let primary = ranges.len() - 1;
        Selection::new(ranges, primary)
    }

    /// The selection after `tx`: each end mapped, sticking after inserted text.
    pub fn map(&self, tx: &Transaction) -> Selection {
        self.transform(|r| Range {
            anchor: tx.map(r.anchor, Assoc::After),
            head: tx.map(r.head, Assoc::After),
            goal_col: None,
        })
    }

    /// Clamp every position to `len` chars (after the text shrank).
    pub fn clamp(&self, len: usize) -> Selection {
        self.transform(|r| Range {
            anchor: r.anchor.min(len),
            head: r.head.min(len),
            goal_col: r.goal_col,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::Change;

    #[test]
    fn overlapping_ranges_merge_and_keep_the_primary() {
        let sel = Selection::new(
            vec![Range::new(10, 14), Range::new(0, 2), Range::new(12, 20)],
            2,
        );
        assert_eq!(sel.ranges(), &[Range::new(0, 2), Range::new(10, 20)]);
        assert_eq!(sel.primary(), Range::new(10, 20));
    }

    #[test]
    fn equal_cursors_merge_but_touching_ranges_do_not() {
        let sel = Selection::new(vec![Range::cursor(5), Range::cursor(5)], 0);
        assert_eq!(sel.len(), 1);
        let sel = Selection::new(vec![Range::new(0, 3), Range::new(3, 6)], 0);
        assert_eq!(sel.len(), 2);
    }

    #[test]
    fn selections_follow_an_edit() {
        let sel = Selection::new(vec![Range::cursor(1), Range::cursor(4)], 0);
        let tx = Transaction::new(vec![Change::insert(1, "x"), Change::insert(4, "y")]);
        let mapped = sel.map(&tx);
        assert_eq!(mapped.ranges(), &[Range::cursor(2), Range::cursor(6)]);
    }
}
