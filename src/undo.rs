//! Undo and redo for edits made from Rust (formatting, commands), which the text field can't undo
//! itself. Each step keeps only the changed text, with the same limits as Live mode's undo.

use std::hash::{DefaultHasher, Hash, Hasher};

use crate::live::{self, Edit, MAX_BYTES, MAX_STEPS};
use crate::model::NoteId;

/// Length and hash of a whole note body, so a step is only undone when the note is exactly as the
/// step left it. Otherwise Ctrl+Z goes to the text field, which undoes the typing done since.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fingerprint {
    len: usize,
    hash: u64,
}

impl Fingerprint {
    fn of(body: &str) -> Self {
        let mut h = DefaultHasher::new();
        body.hash(&mut h);
        Self { len: body.len(), hash: h.finish() }
    }
}

#[derive(Debug)]
struct Step {
    note: NoteId,
    edit: Edit,
    /// Cursor to restore when undoing.
    cursor: usize,
    before: Fingerprint,
    after: Fingerprint,
}

#[derive(Debug, Default)]
pub struct History {
    undo: Vec<Step>,
    redo: Vec<Step>,
}

impl History {
    /// Records a change from `before` to `after` in `note` and clears redo.
    pub fn record(&mut self, note: NoteId, before: &str, after: &str, cursor: usize) {
        self.redo.clear();
        let step = Step { note, edit: live::diff(before, after), cursor, before: Fingerprint::of(before), after: Fingerprint::of(after) };
        self.undo.push(step);
        self.trim();
    }

    /// Undoes the last change if `body` of `note` is exactly what it left. Returns the old body and
    /// the cursor from before the change. None leaves both lists as they were.
    pub fn undo(&mut self, note: NoteId, body: &str) -> Option<(String, usize)> {
        let top = self.undo.last()?;
        if top.note != note || Fingerprint::of(body) != top.after {
            return None;
        }
        let text = live::replace_checked(body, top.edit.at, &top.edit.inserted, &top.edit.removed)?;
        let step = self.undo.pop()?;
        let cursor = step.cursor.min(text.len());
        self.redo.push(step);
        Some((text, cursor))
    }

    /// Redoes what undo undid if `body` of `note` is exactly what undo left. Returns the new body and
    /// the cursor the change was made at.
    pub fn redo(&mut self, note: NoteId, body: &str) -> Option<(String, usize)> {
        let top = self.redo.last()?;
        if top.note != note || Fingerprint::of(body) != top.before {
            return None;
        }
        let text = live::replace_checked(body, top.edit.at, &top.edit.removed, &top.edit.inserted)?;
        let step = self.redo.pop()?;
        let cursor = step.cursor;
        self.undo.push(step);
        Some((text, cursor))
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    /// Drops the oldest steps until both limits hold.
    fn trim(&mut self) {
        if self.undo.len() > MAX_STEPS {
            self.undo.drain(..self.undo.len() - MAX_STEPS);
        }
        let mut bytes: usize = self.undo.iter().map(step_bytes).sum();
        let mut drop = 0;
        while bytes > MAX_BYTES && drop < self.undo.len() {
            bytes -= step_bytes(&self.undo[drop]);
            drop += 1;
        }
        self.undo.drain(..drop);
    }
}

fn step_bytes(step: &Step) -> usize {
    step.edit.removed.len() + step.edit.inserted.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undo_and_redo_restore_the_text_and_cursor() {
        let mut h = History::default();
        h.record(1, "make this bold", "make **this** bold", 5);
        assert_eq!(h.undo(1, "make **this** bold"), Some(("make this bold".into(), 5)));
        assert_eq!(h.redo(1, "make this bold"), Some(("make **this** bold".into(), 5)));
        assert_eq!(h.undo(1, "make **this** bold"), Some(("make this bold".into(), 5)));
    }

    #[test]
    fn steps_undo_in_reverse_order() {
        let mut h = History::default();
        h.record(1, "a", "ab", 1);
        h.record(1, "ab", "abc", 2);
        assert_eq!(h.undo(1, "abc"), Some(("ab".into(), 2)));
        assert_eq!(h.undo(1, "ab"), Some(("a".into(), 1)));
        assert_eq!(h.undo(1, "a"), None);
    }

    #[test]
    fn steps_keep_only_the_changed_text() {
        let mut h = History::default();
        let before = format!("{}x{}", "a".repeat(100_000), "b".repeat(100_000));
        let after = before.replacen('x', "**x**", 1);
        h.record(1, &before, &after, 0);
        assert_eq!(step_bytes(&h.undo[0]), "x".len() + "**x**".len());
        assert_eq!(h.undo(1, &after), Some((before, 0)));
    }

    #[test]
    fn a_changed_note_is_left_to_the_text_field() {
        let mut h = History::default();
        h.record(1, "ab", "aXb", 1);
        // Typed after the change: the step is kept for when the typing has been undone.
        assert_eq!(h.undo(1, "aXbc"), None);
        // Same text the change inserted, but something else differs: still not undone.
        assert_eq!(h.undo(1, "zaXb"), None);
        assert_eq!(h.undo(1, "aXb"), Some(("ab".into(), 1)));
    }

    #[test]
    fn another_note_is_not_undone() {
        let mut h = History::default();
        h.record(1, "ab", "aXb", 1);
        assert_eq!(h.undo(2, "aXb"), None);
        assert_eq!(h.undo(1, "aXb"), Some(("ab".into(), 1)));
        assert_eq!(h.redo(2, "ab"), None);
    }

    #[test]
    fn redo_needs_the_text_undo_left() {
        let mut h = History::default();
        h.record(1, "ab", "aXb", 1);
        h.undo(1, "aXb");
        assert_eq!(h.redo(1, "abc"), None);
        assert_eq!(h.redo(1, "ab"), Some(("aXb".into(), 1)));
    }

    #[test]
    fn a_new_change_clears_redo() {
        let mut h = History::default();
        h.record(1, "ab", "aXb", 1);
        h.undo(1, "aXb");
        h.record(1, "ab", "abY", 2);
        assert_eq!(h.redo(1, "abY"), None);
    }

    #[test]
    fn cursor_is_clamped_to_the_restored_text() {
        let mut h = History::default();
        h.record(1, "", "a long line", 11);
        assert_eq!(h.undo(1, "a long line"), Some((String::new(), 0)));
    }

    #[test]
    fn unicode_changes_round_trip() {
        let mut h = History::default();
        h.record(1, "héllo wörld", "héllo **wörld**", 7);
        assert_eq!(h.undo(1, "héllo **wörld**"), Some(("héllo wörld".into(), 7)));
        assert_eq!(h.redo(1, "héllo wörld"), Some(("héllo **wörld**".into(), 7)));
    }

    #[test]
    fn empty_history_does_nothing() {
        let mut h = History::default();
        assert_eq!(h.undo(1, ""), None);
        assert_eq!(h.redo(1, ""), None);
    }

    #[test]
    fn limits_drop_the_oldest_steps() {
        let mut h = History::default();
        let mut body = String::new();
        for _ in 0..MAX_STEPS + 10 {
            let next = format!("{body}x");
            h.record(1, &body, &next, 0);
            body = next;
        }
        assert_eq!(h.undo.len(), MAX_STEPS);

        let mut h = History::default();
        let big = "a".repeat(1_000_000);
        for _ in 0..10 {
            h.record(1, "", &big, 0);
        }
        let stored: usize = h.undo.iter().map(step_bytes).sum();
        assert!(stored <= MAX_BYTES);
        assert_eq!(h.undo.len(), 8);
        // The newest step is the one kept on top.
        assert_eq!(h.undo(1, &big), Some((String::new(), 0)));
    }
}
