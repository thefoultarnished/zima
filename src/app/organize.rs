//! Pinning, archiving, colours, duplicate/merge, daily notes, templates, tags, links, calendar.

use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone};
use slint::{ComponentHandle, ModelRc, VecModel};

use super::{App, display_title};
use crate::model::{Note, NoteId, now_ms};
use crate::{CalendarDay, LinkRow, TagRow, Theme};

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

    pub fn duplicate(&mut self, id: NoteId) {
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
        if source.id == target {
            return;
        }
        let Some(note) = self.find_mut(target) else { return };
        let heading = if source.title.trim().is_empty() { String::new() } else { format!("## {}\n\n", source.title.trim()) };
        let separator = if note.body.trim().is_empty() { "" } else { "\n\n" };
        note.body = format!("{}{separator}{heading}{}", note.body.trim_end(), source.body);
        note.modified = now_ms();
        let name = display_title(note);
        self.dirty.insert(target);
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
        let max_id = self.notes.iter().map(|n| n.id).max().unwrap_or(0);
        (now_ms() as NoteId).max(max_id + 1)
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
            let mut seen = std::collections::HashSet::new();
            for tag in extract_tags(&note.body) {
                if seen.insert(tag.to_lowercase()) {
                    *counts.entry(tag.to_lowercase()).or_default() += 1;
                }
            }
        }
        let rows: Vec<TagRow> = counts.into_iter().map(|(name, count)| TagRow { name: name.into(), count: count as i32 }).collect();
        ui.set_tags(ModelRc::new(VecModel::from(rows)));
    }

    // ----- Links -----

    /// `[[Title]]` clicked: open that note, or create it.
    pub fn open_link(&mut self, title: &str) {
        let title = title.trim();
        let existing = self
            .notes
            .iter()
            .filter(|n| n.is_live())
            .find(|n| display_title(n).eq_ignore_ascii_case(title) || n.title.trim().eq_ignore_ascii_case(title))
            .map(|n| n.id);
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

    #[test]
    fn progress() {
        assert_eq!(checklist_progress("- [x] a\n- [ ] b\n  - [X] c\ntext"), "2/3");
        assert_eq!(checklist_progress("no tasks"), "");
    }
}
