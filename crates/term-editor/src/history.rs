//! Undo/redo history. Every edit is an entry; consecutive typing joins one
//! undo group so `⌘Z` removes a word, not a letter.

use crate::change::Transaction;
use crate::selection::Selection;

/// What kind of edit an entry was, for grouping.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    /// Typing text without line breaks: joins the previous typing group.
    Typing,
    /// Deleting one character backwards/forwards: joins the previous group of
    /// the same kind.
    DeleteBackward,
    DeleteForward,
    /// Anything else (paste, newline, replace, an agent's edit…): its own group.
    Other,
}

#[derive(Clone, Debug)]
struct Entry {
    id: u64,
    group: u64,
    kind: EditKind,
    tx: Transaction,
    inverse: Transaction,
    before: Selection,
    after: Selection,
}

/// Applied entries (undo stack) and undone ones (redo stack).
#[derive(Clone, Debug, Default)]
pub struct History {
    done: Vec<Entry>,
    undone: Vec<Entry>,
    next_id: u64,
    /// The next edit starts a new group even if it could join the last one
    /// (set by cursor moves, saves and undo).
    break_group: bool,
}

impl History {
    /// Record an applied edit.
    pub fn record(
        &mut self,
        kind: EditKind,
        tx: Transaction,
        inverse: Transaction,
        before: Selection,
        after: Selection,
    ) {
        self.next_id += 1;
        let joins = !self.break_group
            && kind != EditKind::Other
            && self
                .done
                .last()
                .is_some_and(|last| last.kind == kind && last.after == before);
        let group = match self.done.last() {
            Some(last) if joins => last.group,
            _ => self.next_id,
        };
        self.done.push(Entry {
            id: self.next_id,
            group,
            kind,
            tx,
            inverse,
            before,
            after,
        });
        self.undone.clear();
        self.break_group = false;
    }

    /// Make the next edit start its own undo group.
    pub fn break_group(&mut self) {
        self.break_group = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.done.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.undone.is_empty()
    }

    /// The inverses of the last group, newest first, and the selection to
    /// restore. The caller applies them in the order given.
    pub fn undo(&mut self) -> Option<(Vec<Transaction>, Selection)> {
        let group = self.done.last()?.group;
        let mut steps = Vec::new();
        let mut selection = None;
        while self.done.last().is_some_and(|e| e.group == group) {
            let entry = self.done.pop().unwrap();
            steps.push(entry.inverse.clone());
            selection = Some(entry.before.clone());
            self.undone.push(entry);
        }
        self.break_group = true;
        Some((steps, selection.unwrap()))
    }

    /// The transactions of the last undone group, oldest first, and the
    /// selection after them.
    pub fn redo(&mut self) -> Option<(Vec<Transaction>, Selection)> {
        let group = self.undone.last()?.group;
        let mut steps = Vec::new();
        let mut selection = None;
        while self.undone.last().is_some_and(|e| e.group == group) {
            let entry = self.undone.pop().unwrap();
            steps.push(entry.tx.clone());
            selection = Some(entry.after.clone());
            self.done.push(entry);
        }
        self.break_group = true;
        Some((steps, selection.unwrap()))
    }

    /// An id for the current state: equal ids mean equal text, so undoing
    /// back to the saved state reads as unmodified.
    pub fn state_id(&self) -> u64 {
        self.done.last().map_or(0, |e| e.id)
    }
}
