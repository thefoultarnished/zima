//! Version history: keeping copies while notes are edited, and the panel that shows and restores them.

use std::time::UNIX_EPOCH;

use chrono::Local;
use slint::{ModelRc, SharedString, VecModel};

use super::App;
use crate::HistoryLine;
use crate::history::{self, Change};
use crate::model::{NoteId, now_ms};

/// Keep a copy of a note at most this often while it's being edited.
const COPY_EVERY_MS: i64 = 10 * 60 * 1000;

impl App {
    /// Called just before note `id` is written to disk. If its last copy is old enough, the text
    /// still on disk (the version from before these edits) is kept as a copy.
    pub(super) fn keep_copy(&mut self, id: NoteId) {
        if !self.state.version_history {
            return;
        }
        let Ok(dir) = history::folder() else { return };
        let now = now_ms();
        // The newest copy on disk, read once per note per run.
        let last = *self.last_copy.entry(id).or_insert_with(|| history::list(&dir, id).first().copied().unwrap_or(0));
        if now - last < COPY_EVERY_MS {
            return;
        }
        let Some((saved, modified)) = self.store.saved_body(id) else { return };
        if self.find(id).is_none_or(|n| n.body == saved) {
            return;
        }
        // Labelled with when that version was written, which may be days ago.
        let time = modified.duration_since(UNIX_EPOCH).map_or(now, |d| d.as_millis() as i64);
        match history::save(&dir, id, &saved, time, now) {
            Ok(_) => {
                self.last_copy.insert(id, now);
            }
            Err(e) => eprintln!("failed to keep a copy of note {id}: {e}"),
        }
    }

    /// Open the history panel for the note in the editor. Copies are only read now, never at startup.
    pub fn open_history(&mut self) {
        let Some(id) = self.state.current else { return };
        let Ok(dir) = history::folder() else { return };
        let versions = history::list(&dir, id);
        if versions.is_empty() {
            let when = if self.state.version_history {
                "Zima keeps one about every 10 minutes while you edit."
            } else {
                "Turn on \u{201c}Keep version history\u{201d} in Settings to start keeping them."
            };
            self.toast(&format!("No older versions of this note yet. {when}"), false);
            return;
        }
        self.versions = versions;
        let Some(ui) = self.ui.upgrade() else { return };
        let now = Local::now();
        let labels: Vec<SharedString> = self.versions.iter().map(|&t| history::label(t, now).into()).collect();
        ui.set_history_versions(ModelRc::new(VecModel::from(labels)));
        ui.set_history_open(true);
        self.pick_version(0);
    }

    /// From the sidebar's menu: open that note, then its history.
    pub fn open_history_of(&mut self, id: NoteId) {
        self.open(id);
        self.open_history();
    }

    /// Show copy `index`, compared with the note as it is now.
    pub fn pick_version(&mut self, index: usize) {
        let (Some(ui), Some(note), Some(&time)) = (self.ui.upgrade(), self.current(), self.versions.get(index)) else { return };
        let Ok(dir) = history::folder() else { return };
        let copy = match history::read(&dir, note.id, time) {
            Ok(copy) => copy,
            Err(_) => {
                self.toast("Couldn't read that version", true);
                return;
            }
        };
        let changes = history::compare(&note.body, &copy);
        let back = changes.iter().filter(|(c, _)| *c == Change::Back).count();
        let gone = changes.iter().filter(|(c, _)| *c == Change::Gone).count();
        let lines: Vec<HistoryLine> = changes
            .into_iter()
            .map(|(change, text)| HistoryLine {
                text: text.into(),
                kind: match change {
                    Change::Same => 0,
                    Change::Back => 1,
                    Change::Gone => 2,
                },
            })
            .collect();
        ui.set_history_index(index as i32);
        ui.set_history_lines(ModelRc::new(VecModel::from(lines)));
        ui.set_history_summary(restore_summary(back, gone).into());
    }

    /// Bring back the copy on show. The note's current text is kept as a copy first, so nothing is lost,
    /// and Ctrl+Z undoes the restore.
    pub fn restore_version(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let index = ui.get_history_index().max(0) as usize;
        let (Some(note), Some(&time)) = (self.current(), self.versions.get(index)) else { return };
        let (id, body) = (note.id, note.body.clone());
        let Ok(dir) = history::folder() else { return };
        let Ok(copy) = history::read(&dir, id, time) else {
            self.toast("Couldn't read that version", true);
            return;
        };
        ui.set_history_open(false);
        if copy == body {
            self.toast("That version is the same as your note now", false);
            return;
        }
        let now = now_ms();
        if let Err(e) = history::save(&dir, id, &body, now, now) {
            eprintln!("failed to keep a copy of note {id} before restoring: {e}");
            self.toast("Couldn't keep a copy of your note, so nothing was restored", true);
            return;
        }
        self.last_copy.insert(id, now);
        self.replace_body(copy, None);
        self.flush();
        self.refresh_lists();
        self.toast(&format!("Restored the version from {}. Your text from before is kept in the history.", history::label(time, Local::now())), false);
    }

    pub fn set_version_history(&mut self, on: bool) {
        self.state.version_history = on;
        self.save_state();
    }

    /// Settings → Clear history: delete every saved copy of every note.
    pub fn clear_history(&mut self) {
        let Ok(dir) = history::folder() else { return };
        match history::clear(&dir) {
            Ok(()) => {
                self.last_copy.clear();
                self.toast("Version history cleared", false);
            }
            Err(e) => {
                eprintln!("failed to clear version history: {e}");
                self.toast("Couldn't clear the version history", true);
            }
        }
    }

    /// A note deleted for good takes its history with it.
    pub(super) fn forget_history(&mut self, id: NoteId) {
        self.last_copy.remove(&id);
        if let Ok(dir) = history::folder() {
            if let Err(e) = history::remove_note(&dir, id) {
                eprintln!("failed to delete the history of note {id}: {e}");
            }
        }
    }
}

/// The line above the comparison, in plain words.
fn restore_summary(back: usize, gone: usize) -> String {
    let lines = |n: usize| if n == 1 { "1 line".to_string() } else { format!("{n} lines") };
    match (back, gone) {
        (0, 0) => "Same as your note now.".into(),
        (b, 0) => format!("Restoring brings back {}.", lines(b)),
        (0, g) => format!("Restoring removes {}.", lines(g)),
        (b, g) => format!("Restoring brings back {} and removes {}.", lines(b), lines(g)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_words() {
        assert_eq!(restore_summary(3, 1), "Restoring brings back 3 lines and removes 1 line.");
        assert_eq!(restore_summary(1, 0), "Restoring brings back 1 line.");
        assert_eq!(restore_summary(0, 2), "Restoring removes 2 lines.");
        assert_eq!(restore_summary(0, 0), "Same as your note now.");
    }
}
