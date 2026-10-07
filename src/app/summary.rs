//! What the lists need from each note's text (words, tags, checklist count, tasks), worked out once
//! per change and reused, so the sidebar, tags and Tasks lists don't re-read every note after each
//! pause in typing.

use std::collections::HashMap;
use std::rc::Rc;

use chrono::{DateTime, Local, NaiveDate};

use super::organize::{checklist_progress, extract_tags};
use crate::model::{Note, NoteId};
use crate::tasks::{self, Task};

pub struct Summary {
    pub words: usize,
    /// Lowercase, each once.
    pub tags: Vec<String>,
    /// Like "3/5"; empty without checkboxes.
    pub progress: String,
    pub tasks: Vec<Task>,
}

impl Summary {
    pub fn of(body: &str, now: DateTime<Local>) -> Self {
        let mut tags: Vec<String> = Vec::new();
        for tag in extract_tags(body) {
            let tag = tag.to_lowercase();
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }
        Self {
            words: body.split_whitespace().count(),
            tags,
            progress: checklist_progress(body),
            tasks: tasks::extract(body, now),
        }
    }
}

/// When a note was last changed, and its length: any edit (here, from another device or the command
/// line) moves the first, so a summary with the same stamp is still right.
type Stamp = (i64, usize);

/// Every note's summary, kept until the note changes.
#[derive(Default)]
pub struct Summaries {
    /// The day the summaries were made for: "@due fri" means a different date once the day changes.
    day: Option<NaiveDate>,
    notes: HashMap<NoteId, (Stamp, Rc<Summary>)>,
}

impl Summaries {
    /// `note`'s summary, worked out again only if the note (or the day) changed since.
    pub fn get(&mut self, note: &Note, now: DateTime<Local>) -> Rc<Summary> {
        let today = now.date_naive();
        if self.day != Some(today) {
            self.notes.clear();
            self.day = Some(today);
        }
        let stamp = (note.modified, note.body.len());
        match self.notes.get(&note.id) {
            Some((kept, summary)) if *kept == stamp => summary.clone(),
            _ => {
                let summary = Rc::new(Summary::of(&note.body, now));
                self.notes.insert(note.id, (stamp, summary.clone()));
                summary
            }
        }
    }

    /// Forget notes that are gone (deleted for good, or removed by another device).
    pub fn retain(&mut self, keep: impl Fn(NoteId) -> bool) {
        self.notes.retain(|&id, _| keep(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(day: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 10, day, 9, 0, 0).unwrap()
    }

    fn note(id: NoteId, body: &str, modified: i64) -> Note {
        Note { id, body: body.into(), modified, ..Default::default() }
    }

    #[test]
    fn summary_reads_words_tags_and_checkboxes() {
        let s = Summary::of("Shop #Home and #home #errands\n- [x] milk\n- [ ] eggs @due fri", at(7));
        assert_eq!(s.words, 14);
        assert_eq!(s.tags, vec!["home", "errands"]);
        assert_eq!(s.progress, "1/2");
        assert_eq!(s.tasks.len(), 2);
        assert!(s.tasks[0].done && !s.tasks[1].done);
        assert!(s.tasks[1].due.is_some());
    }

    #[test]
    fn summary_of_empty_note() {
        let s = Summary::of("", at(7));
        assert_eq!(s.words, 0);
        assert!(s.tags.is_empty() && s.tasks.is_empty());
        assert_eq!(s.progress, "");
    }

    #[test]
    fn unchanged_note_reuses_its_summary() {
        let mut cache = Summaries::default();
        let n = note(1, "- [ ] a", 100);
        let first = cache.get(&n, at(7));
        // Later the same day: no re-reading.
        assert!(Rc::ptr_eq(&first, &cache.get(&n, Local.with_ymd_and_hms(2026, 10, 7, 23, 0, 0).unwrap())));
    }

    #[test]
    fn edited_note_is_read_again() {
        let mut cache = Summaries::default();
        let first = cache.get(&note(1, "- [ ] a", 100), at(7));
        // Same length, but changed later (ticking the box): new summary.
        let ticked = cache.get(&note(1, "- [x] a", 200), at(7));
        assert!(!Rc::ptr_eq(&first, &ticked));
        assert_eq!(ticked.progress, "1/1");
        // Same time but different text (a synced copy that kept its time): also new.
        let longer = cache.get(&note(1, "- [x] a #tag", 200), at(7));
        assert_eq!(longer.tags, vec!["tag"]);
    }

    #[test]
    fn other_notes_keep_their_summaries() {
        let mut cache = Summaries::default();
        let other = note(2, "#work", 50);
        let kept = cache.get(&other, at(7));
        cache.get(&note(1, "a", 100), at(7));
        cache.get(&note(1, "ab", 300), at(7));
        assert!(Rc::ptr_eq(&kept, &cache.get(&other, at(7))));
    }

    #[test]
    fn new_day_reads_again() {
        let mut cache = Summaries::default();
        let n = note(1, "- [ ] pay rent @due fri", 100);
        let wednesday = cache.get(&n, at(7));
        let thursday = cache.get(&n, at(8));
        assert!(!Rc::ptr_eq(&wednesday, &thursday));
    }

    #[test]
    fn gone_notes_are_forgotten() {
        let mut cache = Summaries::default();
        let gone = note(1, "x", 100);
        let first = cache.get(&gone, at(7));
        cache.get(&note(2, "y", 100), at(7));
        cache.retain(|id| id == 2);
        assert_eq!(cache.notes.len(), 1);
        // Coming back (say, restored by sync) reads it afresh.
        assert!(!Rc::ptr_eq(&first, &cache.get(&gone, at(7))));
    }
}
