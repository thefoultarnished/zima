//! Pinning, archiving, colours, emoji, duplicate/merge, daily notes, templates, tags, links, calendar.

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use slint::{ComponentHandle, ModelRc, VecModel};

use std::cmp::Reverse;

use super::{App, display_title};
use crate::model::{Note, NoteId, now_ms};
use crate::{CalendarDay, LinkPreview, LinkRow, TagRow, Theme};

/// Notes in the Bin longer than this are deleted for good.
const BIN_DAYS: i64 = 30;

impl App {
    pub fn toggle_pin(&mut self, id: NoteId) {
        let Some(note) = self.find_mut(id) else { return };
        note.pinned = !note.pinned;
        self.index_dirty = true;
        self.flush();
        self.refresh_lists();
    }

    /// Sort the note lists (see `UiState::note_order`). Recent always stays by last change.
    pub fn set_note_order(&mut self, order: i32) {
        self.state.note_order = order.clamp(0, 2);
        self.save_state();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_note_order(self.state.note_order);
        }
        self.refresh_lists();
    }

    /// Ctrl+click in the sidebar: add a note to the selection, or take it out.
    pub fn toggle_selected(&mut self, id: NoteId) {
        if let Some(at) = self.selected.iter().position(|&s| s == id) {
            self.selected.remove(at);
        } else if self.find(id).is_some_and(Note::is_live) {
            self.selected.push(id);
        }
        self.refresh_lists();
    }

    pub fn clear_selection(&mut self) {
        if !self.selected.is_empty() {
            self.selected.clear();
            self.refresh_lists();
        }
    }

    /// Pin, favourite, archive or bin every selected note at once (see [`apply_bulk`]).
    pub fn bulk(&mut self, action: &str) {
        let Some(action) = Bulk::from_name(action) else { return };
        let ids = std::mem::take(&mut self.selected);
        let changed = apply_bulk(&mut self.notes, &ids, action, now_ms());
        if changed.is_empty() {
            self.refresh_lists();
            return;
        }
        self.index_dirty = true;
        self.flush();
        // Notes that left the main lists also leave Active, like closing them one by one; binned
        // ones also close their sticky.
        for &id in &changed {
            if !self.find(id).is_some_and(Note::is_listed) {
                self.close(id);
            }
            if !self.find(id).is_some_and(Note::is_live) {
                self.close_sticky(id);
            }
        }
        self.refresh_lists();
        let count = changed.len();
        let notes = if count == 1 { "1 note".to_string() } else { format!("{count} notes") };
        match action {
            Bulk::Bin => self.toast(&format!("Moved {notes} to the Bin."), false),
            Bulk::Archive if self.find(changed[0]).is_some_and(|n| n.archived) => {
                self.toast(&format!("Archived {notes}. Find them in the Archive section."), false)
            }
            _ => {}
        }
    }

    pub fn toggle_archive(&mut self, id: NoteId) {
        let Some(note) = self.find_mut(id) else { return };
        note.archived = !note.archived;
        let archived = note.archived;
        self.index_dirty = true;
        self.flush();
        if archived {
            self.close(id);
            self.toast("Archived. Find it in the Archive section.", false);
        } else {
            self.refresh_lists();
        }
    }

    pub fn set_color(&mut self, id: NoteId, color: Option<u8>) {
        let Some(note) = self.find_mut(id) else { return };
        note.color = color;
        self.index_dirty = true;
        self.flush();
        self.refresh_lists();
        self.sync_sticky(id);
    }

    pub fn set_emoji(&mut self, id: NoteId, emoji: Option<String>) {
        let Some(note) = self.find_mut(id) else { return };
        note.emoji = emoji.as_deref().and_then(crate::emoji::clean);
        self.index_dirty = true;
        self.flush();
        self.refresh_lists();
        self.sync_sticky(id);
    }

    pub fn duplicate(&mut self, id: NoteId) {
        if !self.can_change(id) {
            return;
        }
        let Some(original) = self.find(id).cloned() else { return };
        let new_id = self.next_note_id();
        let title = if original.title.trim().is_empty() { String::new() } else { format!("{} (copy)", original.title.trim()) };
        self.notes.push(Note {
            id: new_id,
            title,
            created: now_ms(),
            modified: now_ms(),
            pinned: false,
            daily: None,
            deleted_at: None,
            ..original
        });
        self.dirty.insert(new_id);
        self.index_dirty = true;
        self.flush();
        self.open(new_id);
    }

    /// Append the current note to `target` and move the current one to the Bin.
    pub fn merge_current_into(&mut self, target: NoteId) {
        let Some(source) = self.current().cloned() else { return };
        if source.id == target || !self.can_change(source.id) || !self.can_change(target) {
            return;
        }
        let Some(note) = self.find_mut(target) else { return };
        let heading = if source.title.trim().is_empty() { String::new() } else { format!("## {}\n\n", source.title.trim()) };
        let separator = if note.body.trim().is_empty() { "" } else { "\n\n" };
        note.body = format!("{}{separator}{heading}{}", note.body.trim_end(), source.body);
        note.modified = now_ms();
        let name = display_title(note);
        self.dirty.insert(target);
        self.sync_sticky(target);
        self.delete(source.id);
        self.open(target);
        self.toast(&format!("Merged into \u{201c}{name}\u{201d}"), false);
    }

    pub fn empty_bin(&mut self) {
        let binned: Vec<NoteId> = self.notes.iter().filter(|n| !n.is_live()).map(|n| n.id).collect();
        let count = binned.len();
        for id in binned {
            self.purge(id);
        }
        self.toast(&match count {
            0 => "The Bin is already empty".to_string(),
            1 => "Deleted 1 note for good".to_string(),
            n => format!("Deleted {n} notes for good"),
        }, false);
    }

    /// Delete notes that have been in the Bin for more than 30 days.
    pub fn auto_purge(&mut self) {
        let cutoff = now_ms() - BIN_DAYS * 24 * 3600 * 1000;
        let old: Vec<NoteId> = self.notes.iter().filter(|n| n.deleted_at.is_some_and(|t| t < cutoff)).map(|n| n.id).collect();
        for id in old {
            self.purge(id);
        }
    }

    /// Open (or create) the daily note for `date`. A note titled "Template: Daily" is used as its template.
    pub fn open_daily(&mut self, date: NaiveDate) {
        let key = date.format("%Y-%m-%d").to_string();
        if let Some(id) = self.notes.iter().find(|n| n.is_live() && n.daily.as_deref() == Some(&key)).map(|n| n.id) {
            if self.find(id).is_some_and(|n| n.archived) {
                self.toggle_archive(id);
            }
            self.open(id);
            return;
        }
        let template = self
            .notes
            .iter()
            .find(|n| n.is_live() && n.title.trim().eq_ignore_ascii_case("template: daily"))
            .map(|n| n.body.clone());
        let body = template.unwrap_or_else(|| "## Plan\n\n- [ ] \n\n## Notes\n\n".to_string());
        let id = self.next_note_id();
        self.notes.push(Note {
            id,
            title: date.format("%A, %-d %B %Y").to_string(),
            body,
            created: now_ms(),
            modified: now_ms(),
            daily: Some(key),
            ..Default::default()
        });
        self.dirty.insert(id);
        self.index_dirty = true;
        self.flush();
        self.open(id);
        self.refresh_calendar();
    }

    pub fn new_from_template(&mut self, template: NoteId) {
        let Some(body) = self.find(template).map(|n| n.body.clone()) else { return };
        let id = self.next_note_id();
        self.notes.push(Note { id, body, created: now_ms(), modified: now_ms(), ..Default::default() });
        self.dirty.insert(id);
        self.index_dirty = true;
        self.flush();
        self.open(id);
        if let Some(ui) = self.ui.upgrade() {
            ui.invoke_focus_title();
        }
    }

    pub fn next_note_id(&self) -> NoteId {
        crate::model::next_id(self.notes.iter().map(|n| n.id), now_ms())
    }

    // ----- Per-note font and width -----

    pub fn set_note_font(&mut self, font: Option<String>) {
        let Some(note) = self.current_mut() else { return };
        note.font = font;
        self.index_dirty = true;
        self.flush();
        self.apply_note_overrides();
    }

    pub fn set_note_wide(&mut self, wide: Option<bool>) {
        let Some(note) = self.current_mut() else { return };
        note.wide = wide;
        self.index_dirty = true;
        self.flush();
        self.apply_note_overrides();
    }

    /// The current note's font/width win over the app settings.
    pub fn apply_note_overrides(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let note = self.current();
        let font = note.and_then(|n| n.font.clone()).unwrap_or_else(|| self.state.font.clone());
        let wide = note.and_then(|n| n.wide).unwrap_or(self.state.wide);
        ui.global::<Theme>().set_font(font.as_str().into());
        ui.set_wide(wide);
    }

    // ----- Tags -----

    pub fn refresh_tags(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
        for note in self.notes.iter().filter(|n| n.is_listed()) {
            for tag in &self.summary(note).tags {
                *counts.entry(tag.clone()).or_default() += 1;
            }
        }
        let rows: Vec<TagRow> = counts.into_iter().map(|(name, count)| TagRow { name: name.into(), count: count as i32 }).collect();
        ui.set_tags(ModelRc::new(VecModel::from(rows)));
    }

    // ----- Links -----

    /// `[[Title]]` clicked: open that note, or create it.
    pub fn open_link(&mut self, title: &str) {
        let title = title.trim();
        let existing = link_target(&self.notes, title).map(|n| n.id);
        match existing {
            Some(id) => {
                if self.find(id).is_some_and(|n| n.archived) {
                    self.toggle_archive(id);
                }
                self.open(id);
            }
            None => {
                let id = self.next_note_id();
                self.notes.push(Note { id, title: title.to_string(), created: now_ms(), modified: now_ms(), ..Default::default() });
                self.dirty.insert(id);
                self.index_dirty = true;
                self.flush();
                self.open(id);
                if let Some(ui) = self.ui.upgrade() {
                    ui.invoke_focus_body();
                }
            }
        }
    }

    /// The pointer rests on a link in a preview block that links to `titles`: show a card for them.
    pub fn link_hover(&mut self, titles: Vec<String>, x: f32, y: f32) {
        if !crate::system::pointer_on_link() {
            self.hide_link_preview();
            return;
        }
        if titles == self.link_preview {
            return;
        }
        let previews: Vec<LinkPreview> = link_previews(&self.notes, &titles)
            .into_iter()
            .map(|p| LinkPreview { title: p.title.into(), snippet: p.snippet.into(), missing: p.missing, emoji: p.emoji.into() })
            .collect();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_link_previews(ModelRc::new(VecModel::from(previews)));
            ui.set_link_preview_x(x);
            ui.set_link_preview_y(y);
        }
        self.link_preview = titles;
    }

    pub fn hide_link_preview(&mut self) {
        if self.link_preview.is_empty() {
            return;
        }
        self.link_preview.clear();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_link_previews(ModelRc::default());
        }
    }

    /// Notes that link to the current one with `[[…]]`.
    pub fn refresh_backlinks(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let rows: Vec<LinkRow> = match self.current() {
            Some(current) if self.state.view_mode != 0 => {
                let names = [display_title(current).to_lowercase(), current.title.trim().to_lowercase()];
                self.notes
                    .iter()
                    .filter(|n| n.is_live() && n.id != current.id)
                    .filter(|n| extract_links(&n.body).iter().any(|l| names.contains(&l.to_lowercase())))
                    .map(|n| LinkRow { id: n.id.to_string().into(), title: display_title(n).into() })
                    .collect()
            }
            _ => Vec::new(),
        };
        ui.set_backlinks(ModelRc::new(VecModel::from(rows)));
    }

    // ----- Calendar -----

    pub fn open_calendar(&mut self) {
        let today = Local::now().date_naive();
        self.calendar_month = today.with_day(1).unwrap_or(today);
        self.refresh_calendar();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_calendar_open(true);
            ui.window().request_redraw();
        }
    }

    pub fn shift_calendar(&mut self, months: i32) {
        let month0 = self.calendar_month.year() * 12 + self.calendar_month.month0() as i32 + months;
        if let Some(date) = NaiveDate::from_ymd_opt(month0.div_euclid(12), month0.rem_euclid(12) as u32 + 1, 1) {
            self.calendar_month = date;
        }
        self.refresh_calendar();
    }

    pub fn pick_calendar_day(&mut self, date: &str) {
        if let Ok(date) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
            if let Some(ui) = self.ui.upgrade() {
                ui.set_calendar_open(false);
            }
            self.open_daily(date);
        }
    }

    /// Six weeks starting on the Monday on or before the 1st.
    pub fn refresh_calendar(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let first = self.calendar_month;
        let start = first - Duration::days(first.weekday().num_days_from_monday() as i64);
        let today = Local::now().date_naive();
        let daily: std::collections::HashSet<&str> = self.notes.iter().filter(|n| n.is_live()).filter_map(|n| n.daily.as_deref()).collect();
        // Reminders and task due dates both show as a blue dot.
        let now = Local::now();
        let reminder_days: std::collections::HashSet<NaiveDate> = self
            .reminders
            .iter()
            .filter_map(|r| Local.timestamp_millis_opt(r.due).single())
            .map(|d| d.date_naive())
            .chain(
                self.notes
                    .iter()
                    .filter(|n| n.is_listed())
                    .flat_map(|n| crate::tasks::extract(&n.body, now))
                    .filter(|t| !t.done)
                    .filter_map(|t| t.due),
            )
            .collect();
        let days: Vec<CalendarDay> = (0..42)
            .map(|i| {
                let date = start + Duration::days(i);
                let key = date.format("%Y-%m-%d").to_string();
                CalendarDay {
                    day: date.day() as i32,
                    in_month: date.month() == first.month(),
                    today: date == today,
                    has_note: daily.contains(key.as_str()),
                    has_reminder: reminder_days.contains(&date),
                    date: key.into(),
                }
            })
            .collect();
        ui.set_calendar_days(ModelRc::new(VecModel::from(days)));
        ui.set_calendar_title(first.format("%B %Y").to_string().into());
    }
}

/// The live note a `[[title]]` points to: its title (or first line, if untitled), ignoring case.
pub fn link_target<'a>(notes: &'a [Note], title: &str) -> Option<&'a Note> {
    let title = title.trim();
    notes.iter().filter(|n| n.is_live()).find(|n| display_title(n).eq_ignore_ascii_case(title) || n.title.trim().eq_ignore_ascii_case(title))
}

/// Most notes shown in one link hover card.
const MAX_LINK_PREVIEWS: usize = 3;

/// What the hover card shows for one linked note.
pub struct LinkPreviewData {
    pub title: String,
    pub snippet: String,
    /// No note has this title yet (clicking the link would create it).
    pub missing: bool,
    pub emoji: String,
}

/// Card entries for the linked `titles` (at most three), looked up in `notes`.
pub fn link_previews(notes: &[Note], titles: &[String]) -> Vec<LinkPreviewData> {
    titles
        .iter()
        .take(MAX_LINK_PREVIEWS)
        .map(|title| match link_target(notes, title) {
            Some(note) => {
                let name = display_title(note);
                LinkPreviewData {
                    snippet: crate::markdown::plain_snippet(&note.body, &name, 3, 220),
                    title: name,
                    missing: false,
                    emoji: note.emoji.as_deref().and_then(crate::emoji::clean).unwrap_or_default(),
                }
            }
            None => LinkPreviewData { title: title.clone(), snippet: String::new(), missing: true, emoji: String::new() },
        })
        .collect()
}

/// `#tags` in a note: `#` at a word start, then a letter, then letters, digits, `-`, `_` or `/`.
/// Headings (`# Title`) aren't tags because of the space.
pub fn extract_tags(body: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut in_code = false;
    for line in body.lines() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code {
            continue;
        }
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let at_word_start = i == 0 || chars[i - 1].is_whitespace() || chars[i - 1] == '(';
            if chars[i] == '#' && at_word_start && chars.get(i + 1).is_some_and(|c| c.is_alphabetic()) {
                let start = i + 1;
                let mut end = start;
                while end < chars.len() && (chars[end].is_alphanumeric() || "-_/".contains(chars[end])) {
                    end += 1;
                }
                tags.push(chars[start..end].iter().collect());
                i = end;
            } else {
                i += 1;
            }
        }
    }
    tags
}

/// `[[Title]]` link targets in a note.
pub fn extract_links(body: &str) -> Vec<String> {
    let mut links = Vec::new();
    let mut rest = body;
    while let Some(start) = rest.find("[[") {
        let after = &rest[start + 2..];
        match after.find("]]") {
            Some(end) if !after[..end].contains('\n') && !after[..end].trim().is_empty() => {
                links.push(after[..end].trim().to_string());
                rest = &after[end + 2..];
            }
            _ => rest = after,
        }
    }
    links
}

/// "3/5" for a note with five checkboxes, three ticked; empty if none.
pub fn checklist_progress(body: &str) -> String {
    let (mut done, mut total) = (0, 0);
    for line in body.lines() {
        let line = line.trim_start();
        for marker in ["- [", "* [", "+ ["] {
            if let Some(rest) = line.strip_prefix(marker) {
                if rest.starts_with("x] ") || rest.starts_with("X] ") {
                    done += 1;
                    total += 1;
                } else if rest.starts_with(" ] ") || rest == " ]" {
                    total += 1;
                }
            }
        }
    }
    if total == 0 { String::new() } else { format!("{done}/{total}") }
}

/// Something done to every selected note at once.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Bulk {
    Pin,
    Favorite,
    Archive,
    Bin,
}

impl Bulk {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "pin" => Some(Self::Pin),
            "favorite" => Some(Self::Favorite),
            "archive" => Some(Self::Archive),
            "bin" => Some(Self::Bin),
            _ => None,
        }
    }
}

/// Apply `action` to the notes in `ids` that are still around and not in the Bin. Pin, favourite and
/// archive work like a switch for the whole group: if every note already has it, it's taken off all
/// of them, otherwise it's put on all of them. Returns the notes that changed.
pub fn apply_bulk(notes: &mut [Note], ids: &[NoteId], action: Bulk, now: i64) -> Vec<NoteId> {
    fn flag(n: &mut Note, action: Bulk) -> Option<&mut bool> {
        match action {
            Bulk::Pin => Some(&mut n.pinned),
            Bulk::Favorite => Some(&mut n.favorite),
            Bulk::Archive => Some(&mut n.archived),
            Bulk::Bin => None,
        }
    }
    let mut targets: Vec<&mut Note> = notes.iter_mut().filter(|n| n.is_live() && ids.contains(&n.id)).collect();
    let on = !targets.iter_mut().all(|n| flag(n, action).is_some_and(|f| *f));
    let mut changed = Vec::new();
    for note in targets {
        match flag(note, action) {
            Some(f) if *f != on => *f = on,
            Some(_) => continue,
            None => note.deleted_at = Some(now),
        }
        changed.push(note.id);
    }
    changed
}

/// Put notes in the list order the user picked (see `UiState::note_order`).
/// Notes that tie (same title, or no creation date) stay newest change first.
pub fn sort_notes(notes: &mut [&Note], order: i32) {
    notes.sort_by_key(|n| Reverse(n.modified));
    match order {
        1 => notes.sort_by_cached_key(|n| display_title(n).to_lowercase()),
        2 => notes.sort_by_key(|n| Reverse(n.created)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags() {
        assert_eq!(extract_tags("# Heading\nidea #work and #home/garden, not a#b"), vec!["work", "home/garden"]);
        assert_eq!(extract_tags("```\n#notatag\n```\n#yes"), vec!["yes"]);
        assert!(extract_tags("#1 is not a tag").is_empty());
    }

    #[test]
    fn links() {
        assert_eq!(extract_links("see [[Groceries]] and [[ Book ideas ]]"), vec!["Groceries", "Book ideas"]);
        assert!(extract_links("[[]] and [[broken").is_empty());
    }

    fn note(id: u64, title: &str, body: &str) -> Note {
        Note { id, title: title.into(), body: body.into(), ..Default::default() }
    }

    #[test]
    fn link_target_matches_ignoring_case() {
        let notes = [note(1, "Groceries", ""), note(2, "Other", "")];
        assert_eq!(link_target(&notes, " groceries ").map(|n| n.id), Some(1));
        assert!(link_target(&notes, "Grocer").is_none());
    }

    #[test]
    fn link_target_uses_first_line_for_untitled() {
        let notes = [note(1, "", "# Book ideas\nmore")];
        assert_eq!(link_target(&notes, "book ideas").map(|n| n.id), Some(1));
    }

    #[test]
    fn link_target_skips_binned() {
        let mut gone = note(1, "Groceries", "");
        gone.deleted_at = Some(5);
        assert!(link_target(&[gone], "Groceries").is_none());
    }

    #[test]
    fn link_previews_marks_missing() {
        let notes = [note(1, "Groceries", "# Groceries\nmilk\neggs")];
        let found = link_previews(&notes, &["groceries".to_string(), "Nope".to_string()]);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].title.as_str(), found[0].snippet.as_str(), found[0].missing), ("Groceries", "milk\neggs", false));
        assert_eq!((found[1].title.as_str(), found[1].missing), ("Nope", true));
    }

    #[test]
    fn link_previews_caps_at_three() {
        let titles: Vec<String> = (0..5).map(|i| format!("n{i}")).collect();
        let found = link_previews(&[], &titles);
        assert_eq!(found.len(), 3);
        assert_eq!(found[2].title, "n2");
    }

    #[test]
    fn link_previews_empty() {
        assert!(link_previews(&[note(1, "A", "")], &[]).is_empty());
        let found = link_previews(&[note(1, "A", "")], &["A".to_string()]);
        assert_eq!((found[0].snippet.as_str(), found[0].missing), ("", false));
    }

    #[test]
    fn progress() {
        assert_eq!(checklist_progress("- [x] a\n- [ ] b\n  - [X] c\ntext"), "2/3");
        assert_eq!(checklist_progress("no tasks"), "");
    }

    fn ids_where(notes: &[Note], test: impl Fn(&Note) -> bool) -> Vec<NoteId> {
        notes.iter().filter(|n| test(n)).map(|n| n.id).collect()
    }

    #[test]
    fn bulk_pins_all_selected() {
        let mut notes = [dated(1, "a", 0, 0), dated(2, "b", 0, 0), dated(3, "c", 0, 0)];
        assert_eq!(apply_bulk(&mut notes, &[1, 3], Bulk::Pin, 0), vec![1, 3]);
        assert_eq!(ids_where(&notes, |n| n.pinned), vec![1, 3]);
    }

    #[test]
    fn bulk_switches_off_when_all_have_it() {
        let mut notes = [dated(1, "a", 0, 0), dated(2, "b", 0, 0)];
        notes[0].favorite = true;
        notes[1].favorite = true;
        apply_bulk(&mut notes, &[1, 2], Bulk::Favorite, 0);
        assert!(ids_where(&notes, |n| n.favorite).is_empty());
    }

    #[test]
    fn bulk_mixed_group_switches_on_and_skips_unchanged() {
        let mut notes = [dated(1, "a", 0, 0), dated(2, "b", 0, 0)];
        notes[0].archived = true;
        // Only the note that wasn't archived yet changes.
        assert_eq!(apply_bulk(&mut notes, &[1, 2], Bulk::Archive, 0), vec![2]);
        assert_eq!(ids_where(&notes, |n| n.archived), vec![1, 2]);
    }

    #[test]
    fn bulk_bin_and_unselected_notes() {
        let mut notes = [dated(1, "a", 0, 0), dated(2, "b", 0, 0), dated(3, "c", 0, 0)];
        assert_eq!(apply_bulk(&mut notes, &[2, 3], Bulk::Bin, 77), vec![2, 3]);
        assert_eq!(notes[1].deleted_at, Some(77));
        // The note that wasn't selected is untouched.
        assert!(notes[0].deleted_at.is_none() && !notes[0].pinned);
    }

    #[test]
    fn bulk_edge_cases() {
        let mut notes = [dated(1, "a", 0, 0), dated(2, "b", 0, 0)];
        notes[1].deleted_at = Some(5);
        // Nothing selected, a note that's gone, and a note already in the Bin: nothing happens.
        assert!(apply_bulk(&mut notes, &[], Bulk::Pin, 0).is_empty());
        assert!(apply_bulk(&mut notes, &[99], Bulk::Bin, 0).is_empty());
        assert!(apply_bulk(&mut notes, &[2], Bulk::Bin, 9).is_empty());
        assert_eq!(notes[1].deleted_at, Some(5));
        assert_eq!(Bulk::from_name("nope"), None);
        assert_eq!(Bulk::from_name("bin"), Some(Bulk::Bin));
    }

    fn dated(id: NoteId, title: &str, modified: i64, created: i64) -> Note {
        Note { id, title: title.into(), modified, created, ..Default::default() }
    }

    fn sorted(notes: &[Note], order: i32) -> Vec<NoteId> {
        let mut list: Vec<&Note> = notes.iter().collect();
        sort_notes(&mut list, order);
        list.iter().map(|n| n.id).collect()
    }

    #[test]
    fn sort_by_title_ignores_case() {
        let notes = [dated(1, "banana", 3, 1), dated(2, "Apple", 1, 2), dated(3, "cherry", 2, 3)];
        assert_eq!(sorted(&notes, 1), vec![2, 1, 3]);
    }

    #[test]
    fn sort_by_created_newest_first() {
        let notes = [dated(1, "a", 30, 100), dated(2, "b", 10, 300), dated(3, "c", 20, 200)];
        assert_eq!(sorted(&notes, 2), vec![2, 3, 1]);
    }

    #[test]
    fn sort_default_is_last_changed() {
        let notes = [dated(1, "a", 10, 3), dated(2, "b", 30, 2), dated(3, "c", 20, 1)];
        assert_eq!(sorted(&notes, 0), vec![2, 3, 1]);
        // Anything unknown (a newer version's setting) falls back to last changed.
        assert_eq!(sorted(&notes, 9), vec![2, 3, 1]);
    }

    #[test]
    fn sort_ties_and_edges() {
        // Same title, or no creation date (old notes): newest change first, and old notes last.
        let notes = [dated(1, "Same", 10, 0), dated(2, "same", 20, 0), dated(3, "x", 5, 50)];
        assert_eq!(sorted(&notes, 1), vec![2, 1, 3]);
        assert_eq!(sorted(&notes, 2), vec![3, 2, 1]);
        assert!(sorted(&[], 1).is_empty());
    }
}
