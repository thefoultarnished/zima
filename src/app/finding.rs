//! Finding things again: saved searches, notebooks, on this day, random note.

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use slint::{ModelRc, VecModel};

use super::App;
use super::navigate::PaletteMode;
use crate::TagRow;
use crate::model::NoteId;

impl App {
    /// Sidebar parts that depend on all notes: saved searches, notebooks, on-this-day count.
    pub fn refresh_finding(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let saved: Vec<slint::SharedString> = self.state.saved_searches.iter().map(|s| s.as_str().into()).collect();
        ui.set_saved_searches(ModelRc::new(VecModel::from(saved)));
        let current = self.search.trim();
        ui.set_search_saved(!current.is_empty() && self.state.saved_searches.iter().any(|s| s == current));
        let notebooks: Vec<TagRow> = self.notebooks().into_iter().map(|(name, count)| TagRow { name: name.into(), count: count as i32 }).collect();
        ui.set_notebooks(ModelRc::new(VecModel::from(notebooks)));
        ui.set_on_this_day_count(self.on_this_day().len() as i32);
    }

    /// Notebook names with their note counts.
    pub fn notebooks(&self) -> Vec<(String, usize)> {
        let mut counts: Vec<(String, usize)> = Vec::new();
        for name in self.notes.iter().filter(|n| n.is_listed()).filter_map(|n| n.notebook.clone()) {
            match counts.iter_mut().find(|(n, _)| *n == name) {
                Some((_, count)) => *count += 1,
                None => counts.push((name, 1)),
            }
        }
        counts.sort_by_key(|(n, _)| n.to_lowercase());
        counts
    }

    pub fn pick_notebook(&mut self, id: NoteId) {
        self.open_palette(PaletteMode::Notebook(id));
    }

    pub fn pick_emoji(&mut self, id: NoteId) {
        self.open_palette(PaletteMode::Emoji(id));
    }

    pub fn set_notebook(&mut self, id: NoteId, notebook: Option<String>) {
        let Some(note) = self.find_mut(id) else { return };
        note.notebook = notebook.clone();
        self.index_dirty = true;
        self.flush();
        self.refresh_lists();
        self.toast(&match notebook {
            Some(name) => format!("Moved to \u{201c}{name}\u{201d}"),
            None => "Removed from its notebook".into(),
        }, false);
    }

    pub fn save_search(&mut self) {
        let query = self.search.trim().to_string();
        if query.is_empty() {
            self.toast("Type a search first", false);
            return;
        }
        if !self.state.saved_searches.contains(&query) {
            self.state.saved_searches.push(query);
            self.save_state();
        }
        self.refresh_finding();
    }

    pub fn remove_saved_search(&mut self, query: &str) {
        self.state.saved_searches.retain(|s| s != query);
        self.save_state();
        self.refresh_finding();
    }

    /// Run a search (from a saved search or a notebook) and show it in the search box.
    pub fn apply_search(&mut self, query: String) {
        if let Some(ui) = self.ui.upgrade() {
            ui.set_search_text(query.as_str().into());
        }
        self.set_search(query);
    }

    /// Notes created on this day a week, a month, or years ago: (id, "1 year ago" …).
    pub fn on_this_day(&self) -> Vec<(NoteId, String)> {
        let today = Local::now().date_naive();
        let mut targets: Vec<(NaiveDate, String)> = vec![(today - Duration::days(7), "A week ago".into())];
        if let Some(month_ago) = today.checked_sub_months(chrono::Months::new(1)) {
            targets.push((month_ago, "A month ago".into()));
        }
        for years in 1..=10 {
            if let Some(date) = today.with_year(today.year() - years) {
                targets.push((date, if years == 1 { "A year ago".into() } else { format!("{years} years ago") }));
            }
        }
        let mut found = Vec::new();
        for note in self.notes.iter().filter(|n| n.is_live()) {
            let ms = if note.created > 0 { note.created } else { note.modified };
            let Some(date) = Local.timestamp_millis_opt(ms).single().map(|d| d.date_naive()) else { continue };
            if let Some((_, label)) = targets.iter().find(|(t, _)| *t == date) {
                found.push((note.id, label.clone()));
            }
        }
        found
    }

    pub fn open_random(&mut self) {
        let listed: Vec<NoteId> = self.notes.iter().filter(|n| n.is_listed() && Some(n.id) != self.state.current).map(|n| n.id).collect();
        if listed.is_empty() {
            self.toast("No other notes yet", false);
            return;
        }
        // Good enough randomness without another dependency.
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos() as usize);
        self.open(listed[seed % listed.len()]);
    }
}
