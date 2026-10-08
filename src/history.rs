//! Version history: older copies of each note, so a bad edit can be undone even hours later.
//! Copies live in `history/<note id>/<unix ms>.md` next to the app's own files (`%APPDATA%\Zima`),
//! so they stay on this PC even when the notes are in a synced folder: OneDrive or Dropbox never
//! upload them, and two devices never clash over them.

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Datelike, Local, TimeZone};
use similar::{ChangeTag, TextDiff};

use crate::model::NoteId;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;
/// Copies older than this are thinned to one per week; younger ones (past the first day) to one per day.
const DAILY_FOR_DAYS: i64 = 30;

/// `%APPDATA%\Zima\history`, or `$ZIMA_DATA_DIR\history`.
pub fn folder() -> io::Result<PathBuf> {
    Ok(crate::store::home_dir()?.join("history"))
}

fn note_dir(dir: &Path, id: NoteId) -> PathBuf {
    dir.join(id.to_string())
}

fn copy_path(dir: &Path, id: NoteId, time: i64) -> PathBuf {
    note_dir(dir, id).join(format!("{time}.md"))
}

/// When each saved copy of a note was made (unix ms), newest first.
pub fn list(dir: &Path, id: NoteId) -> Vec<i64> {
    let Ok(entries) = fs::read_dir(note_dir(dir, id)) else { return Vec::new() };
    let mut times: Vec<i64> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.strip_suffix(".md")?.parse().ok())
        .collect();
    times.sort_unstable_by(|a, b| b.cmp(a));
    times
}

pub fn read(dir: &Path, id: NoteId, time: i64) -> io::Result<String> {
    fs::read_to_string(copy_path(dir, id, time))
}

/// Keep `body` as a copy of note `id` made at `time`, unless it's the same as the newest copy.
/// Then thin out old copies (see [`to_remove`]). Returns whether a copy was written.
pub fn save(dir: &Path, id: NoteId, body: &str, time: i64, now: i64) -> io::Result<bool> {
    let times = list(dir, id);
    if let Some(&newest) = times.first() {
        if read(dir, id, newest).is_ok_and(|text| text == body) {
            return Ok(false);
        }
    }
    fs::create_dir_all(note_dir(dir, id))?;
    // Two copies in the same millisecond: nudge the second one along.
    let mut time = time;
    while times.contains(&time) {
        time += 1;
    }
    crate::store::write_atomic(&copy_path(dir, id, time), body.as_bytes())?;
    let mut all = times;
    all.push(time);
    for old in to_remove(&all, now) {
        let _ = fs::remove_file(copy_path(dir, id, old));
    }
    Ok(true)
}

/// Which copies to delete: keep every copy from the last day, then the newest one of each day for a
/// month, then the newest one of each week.
pub fn to_remove(times: &[i64], now: i64) -> Vec<i64> {
    let mut sorted = times.to_vec();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let mut seen: HashSet<(i32, u32, bool)> = HashSet::new();
    let mut remove = Vec::new();
    for time in sorted {
        let age = now - time;
        if age < DAY_MS {
            continue;
        }
        let Some(date) = Local.timestamp_millis_opt(time).single().map(|t| t.date_naive()) else { continue };
        let bucket = if age < DAILY_FOR_DAYS * DAY_MS {
            (date.year(), date.ordinal(), false)
        } else {
            let week = date.iso_week();
            (week.year(), week.week(), true)
        };
        if !seen.insert(bucket) {
            remove.push(time);
        }
    }
    remove
}

/// Delete a note's copies (the note was deleted for good).
pub fn remove_note(dir: &Path, id: NoteId) -> io::Result<()> {
    match fs::remove_dir_all(note_dir(dir, id)) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// Delete every copy of every note.
pub fn clear(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// "Today 14:30", "Yesterday 09:10", "Mon 5 Oct, 14:30", or with the year if it isn't this one.
pub fn label(time: i64, now: DateTime<Local>) -> String {
    let Some(at) = Local.timestamp_millis_opt(time).single() else { return String::new() };
    let (day, today) = (at.date_naive(), now.date_naive());
    if day == today {
        at.format("Today %H:%M").to_string()
    } else if today.pred_opt() == Some(day) {
        at.format("Yesterday %H:%M").to_string()
    } else if day.year() == today.year() {
        at.format("%a %-d %b, %H:%M").to_string()
    } else {
        at.format("%-d %b %Y, %H:%M").to_string()
    }
}

/// What restoring a copy would do to one line of the note.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Change {
    /// The same in both.
    Same,
    /// Only in the copy: restoring brings it back.
    Back,
    /// Only in the note now: restoring removes it.
    Gone,
}

/// The note now and a saved copy, line by line.
pub fn compare(now: &str, copy: &str) -> Vec<(Change, String)> {
    TextDiff::from_lines(now, copy)
        .iter_all_changes()
        .map(|change| {
            let kind = match change.tag() {
                ChangeTag::Equal => Change::Same,
                ChangeTag::Insert => Change::Back,
                ChangeTag::Delete => Change::Gone,
            };
            (kind, change.value().trim_end_matches(['\n', '\r']).to_string())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty history folder for one test.
    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("zima-history-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn at(day: u32, hour: u32) -> i64 {
        Local.with_ymd_and_hms(2026, 10, day, hour, 0, 0).unwrap().timestamp_millis()
    }

    #[test]
    fn saves_and_lists_newest_first() {
        let dir = temp("list");
        assert!(save(&dir, 7, "first", 1000, 1000).unwrap());
        assert!(save(&dir, 7, "second", 2000, 2000).unwrap());
        assert_eq!(list(&dir, 7), vec![2000, 1000]);
        assert_eq!(read(&dir, 7, 1000).unwrap(), "first");
        // Other notes have their own copies.
        assert!(list(&dir, 8).is_empty());
        clear(&dir).unwrap();
    }

    #[test]
    fn same_text_as_newest_copy_is_not_saved_again() {
        let dir = temp("same");
        assert!(save(&dir, 1, "hello", 1000, 1000).unwrap());
        assert!(!save(&dir, 1, "hello", 5000, 5000).unwrap());
        assert_eq!(list(&dir, 1), vec![1000]);
        // Going back to an older text is a change, so it's kept.
        save(&dir, 1, "changed", 6000, 6000).unwrap();
        assert!(save(&dir, 1, "hello", 7000, 7000).unwrap());
        assert_eq!(list(&dir, 1).len(), 3);
        clear(&dir).unwrap();
    }

    #[test]
    fn same_millisecond_gets_its_own_file() {
        let dir = temp("tie");
        save(&dir, 1, "a", 1000, 1000).unwrap();
        save(&dir, 1, "b", 1000, 1000).unwrap();
        assert_eq!(list(&dir, 1), vec![1001, 1000]);
        clear(&dir).unwrap();
    }

    #[test]
    fn remove_note_and_clear() {
        let dir = temp("remove");
        save(&dir, 1, "a", 1000, 1000).unwrap();
        save(&dir, 2, "b", 1000, 1000).unwrap();
        remove_note(&dir, 1).unwrap();
        assert!(list(&dir, 1).is_empty());
        assert_eq!(list(&dir, 2), vec![1000]);
        // Removing what isn't there is fine.
        remove_note(&dir, 99).unwrap();
        clear(&dir).unwrap();
        assert!(list(&dir, 2).is_empty());
        clear(&dir).unwrap();
    }

    #[test]
    fn thinning_keeps_the_last_day_whole() {
        let now = at(20, 18);
        let times = [at(20, 9), at(20, 10), at(20, 11), at(19, 20)];
        assert!(to_remove(&times, now).is_empty());
    }

    #[test]
    fn thinning_keeps_one_per_day_for_a_month() {
        let now = at(20, 18);
        // Three copies on the 15th, one on the 14th.
        let times = [at(15, 9), at(15, 12), at(15, 16), at(14, 10)];
        let mut removed = to_remove(&times, now);
        removed.sort();
        assert_eq!(removed, vec![at(15, 9), at(15, 12)]);
    }

    #[test]
    fn thinning_keeps_one_per_week_after_a_month() {
        let now = Local.with_ymd_and_hms(2026, 12, 31, 12, 0, 0).unwrap().timestamp_millis();
        // Mon 5, Wed 7 and Fri 9 Oct are one week; Mon 12 Oct starts the next.
        let times = [at(5, 9), at(7, 9), at(9, 9), at(12, 9)];
        let mut removed = to_remove(&times, now);
        removed.sort();
        assert_eq!(removed, vec![at(5, 9), at(7, 9)]);
    }

    #[test]
    fn thinning_edges() {
        assert!(to_remove(&[], 0).is_empty());
        // A single old copy always stays.
        assert!(to_remove(&[at(1, 9)], at(30, 9)).is_empty());
    }

    #[test]
    fn labels() {
        let now = Local.with_ymd_and_hms(2026, 10, 8, 18, 0, 0).unwrap();
        assert_eq!(label(at(8, 14), now), "Today 14:00");
        assert_eq!(label(at(7, 9), now), "Yesterday 09:00");
        assert_eq!(label(at(5, 9), now), "Mon 5 Oct, 09:00");
        let last_year = Local.with_ymd_and_hms(2025, 12, 31, 23, 30, 0).unwrap().timestamp_millis();
        assert_eq!(label(last_year, now), "31 Dec 2025, 23:30");
    }

    #[test]
    fn compare_marks_what_restoring_changes() {
        let changes = compare("milk\neggs\nbread\n", "milk\nbutter\nbread\n");
        assert_eq!(
            changes,
            vec![
                (Change::Same, "milk".to_string()),
                (Change::Gone, "eggs".to_string()),
                (Change::Back, "butter".to_string()),
                (Change::Same, "bread".to_string()),
            ]
        );
    }

    #[test]
    fn compare_edges() {
        assert!(compare("", "").is_empty());
        assert!(compare("same\n", "same\n").iter().all(|(c, _)| *c == Change::Same));
        assert_eq!(compare("", "back"), vec![(Change::Back, "back".to_string())]);
        assert_eq!(compare("gone", ""), vec![(Change::Gone, "gone".to_string())]);
        // Windows line endings don't leak into the shown text.
        assert_eq!(compare("a\r\n", "a\r\n"), vec![(Change::Same, "a".to_string())]);
    }
}
