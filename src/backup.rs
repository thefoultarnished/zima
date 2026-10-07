//! Daily safety copies: the whole data folder zipped to `backups/zima-YYYY-MM-DD.zip`, keeping the
//! newest 14. Backups live next to the app's own files (`%APPDATA%\Zima\backups`), so they stay on
//! this PC even when the notes are in a synced folder. To restore one, unzip it into the data folder.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use chrono::NaiveDate;

/// How many backups to keep.
pub const KEEP: usize = 14;

/// Only one backup at a time (the daily one runs on a background thread).
static RUNNING: Mutex<()> = Mutex::new(());

/// `%APPDATA%\Zima\backups`, or `$ZIMA_DATA_DIR\backups`.
pub fn folder() -> io::Result<PathBuf> {
    Ok(crate::store::home_dir()?.join("backups"))
}

/// Zip `data` into `backups`, then delete all but the newest [`KEEP`] backups.
/// Without `force` (the daily backup), nothing happens if today's backup exists or nothing changed
/// since the last one, so a quiet week doesn't push older backups out. Returns the new zip, if any.
pub fn back_up(data: &Path, backups: &Path, today: NaiveDate, force: bool) -> io::Result<Option<PathBuf>> {
    let _running = RUNNING.lock().unwrap_or_else(|e| e.into_inner());
    fs::create_dir_all(backups)?;
    let path = backups.join(file_name(today));
    if !force && (path.exists() || !changed_since_last_backup(data, backups)) {
        return Ok(None);
    }
    // Write next to it first, so a crash never leaves a half-written zip with a backup's name.
    let partial = path.with_extension("zip.tmp");
    if let Err(e) = write_zip(data, backups, &partial) {
        let _ = fs::remove_file(&partial);
        return Err(e);
    }
    fs::rename(&partial, &path)?;
    for name in to_remove(&backup_names(backups)) {
        if let Err(e) = fs::remove_file(backups.join(&name)) {
            eprintln!("couldn't remove old backup {name}: {e}");
        }
    }
    Ok(Some(path))
}

fn file_name(day: NaiveDate) -> String {
    format!("zima-{}.zip", day.format("%Y-%m-%d"))
}

/// Our backup files only (`zima-YYYY-MM-DD.zip`), so nothing else in the folder is ever deleted.
fn is_backup_name(name: &str) -> bool {
    name.strip_prefix("zima-")
        .and_then(|rest| rest.strip_suffix(".zip"))
        .is_some_and(|day| NaiveDate::parse_from_str(day, "%Y-%m-%d").is_ok())
}

fn backup_names(backups: &Path) -> Vec<String> {
    let Ok(entries) = fs::read_dir(backups) else { return Vec::new() };
    entries.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| is_backup_name(n)).collect()
}

/// The backups to delete: all but the newest [`KEEP`]. The date in the name sorts as text.
fn to_remove(names: &[String]) -> Vec<String> {
    let mut names: Vec<String> = names.iter().filter(|n| is_backup_name(n)).cloned().collect();
    names.sort_unstable_by(|a, b| b.cmp(a));
    names.into_iter().skip(KEEP).collect()
}

/// Whether any note or data file changed after the newest backup was made. Settings (`state.json`)
/// don't count: the window position alone changes them every day.
fn changed_since_last_backup(data: &Path, backups: &Path) -> bool {
    let newest_backup = backup_names(backups).iter().filter_map(|n| modified(&backups.join(n))).max();
    let Some(newest_backup) = newest_backup else { return true };
    let mut newest_change = None;
    walk(data, backups, &mut |path, _| {
        if path.file_name().is_some_and(|n| n != "state.json") {
            newest_change = newest_change.max(modified(path));
        }
        Ok(())
    })
    .map_or(true, |()| newest_change.is_some_and(|t| t > newest_backup))
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn write_zip(data: &Path, backups: &Path, to: &Path) -> io::Result<()> {
    let mut zip = zip::ZipWriter::new(fs::File::create(to)?);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    walk(data, backups, &mut |path, name| {
        let bytes = fs::read(path)?;
        zip.start_file(name, options).map_err(io::Error::other)?;
        zip.write_all(&bytes)
    })?;
    zip.finish().map_err(io::Error::other)?.sync_all()
}

/// Call `f(path, name inside the zip)` for every file under `data`, skipping the backups folder
/// (it's inside `data` when the notes aren't synced) and half-written `.tmp` files.
fn walk(data: &Path, backups: &Path, f: &mut dyn FnMut(&Path, &str) -> io::Result<()>) -> io::Result<()> {
    let mut stack = vec![(data.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            if entry.file_type()?.is_dir() {
                if path != backups {
                    stack.push((path, format!("{name}/")));
                }
            } else if !name.ends_with(".tmp") {
                f(&path, &name)?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    /// A data folder with one note, with the backups folder inside it (as when notes aren't synced).
    fn scratch(name: &str) -> (PathBuf, PathBuf) {
        let data = std::env::temp_dir().join(format!("zima-backup-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&data);
        fs::create_dir_all(data.join("notes")).unwrap();
        fs::write(data.join("index.json"), "[]").unwrap();
        fs::write(data.join("notes").join("1.md"), "hello").unwrap();
        (data.clone(), data.join("backups"))
    }

    fn zip_names(path: &Path) -> Vec<String> {
        let mut archive = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
        let mut names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().to_string()).collect();
        names.sort();
        names
    }

    #[test]
    fn zips_the_whole_data_folder_but_not_backups_or_temp_files() {
        let (data, backups) = scratch("contents");
        fs::write(data.join("state.json"), "{}").unwrap();
        fs::write(data.join("index.tmp"), "half written").unwrap();
        fs::create_dir_all(&backups).unwrap();
        fs::write(backups.join("zima-2026-10-01.zip"), "old").unwrap();

        let path = back_up(&data, &backups, day(7), true).unwrap().unwrap();
        assert_eq!(path, backups.join("zima-2026-10-07.zip"));
        assert_eq!(zip_names(&path), vec!["index.json", "notes/1.md", "state.json"]);
        let mut archive = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
        let mut note = String::new();
        io::Read::read_to_string(&mut archive.by_name("notes/1.md").unwrap(), &mut note).unwrap();
        assert_eq!(note, "hello");
        // No leftover temp file.
        assert!(!backups.join("zima-2026-10-07.zip.tmp").exists());
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn daily_backup_happens_once_a_day() {
        let (data, backups) = scratch("once");
        assert!(back_up(&data, &backups, day(7), false).unwrap().is_some());
        fs::write(data.join("notes").join("2.md"), "new note").unwrap();
        // Today's backup exists: the daily one doesn't run again...
        assert!(back_up(&data, &backups, day(7), false).unwrap().is_none());
        // ...but "Back up now" replaces it with the latest notes.
        let path = back_up(&data, &backups, day(7), true).unwrap().unwrap();
        assert_eq!(zip_names(&path), vec!["index.json", "notes/1.md", "notes/2.md"]);
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn no_daily_backup_when_nothing_changed() {
        let (data, backups) = scratch("unchanged");
        assert!(back_up(&data, &backups, day(6), false).unwrap().is_some());
        // Changing only settings (window position etc.) doesn't count as a change.
        std::thread::sleep(std::time::Duration::from_millis(20));
        fs::write(data.join("state.json"), "{}").unwrap();
        assert!(back_up(&data, &backups, day(7), false).unwrap().is_none());
        // Editing a note does.
        fs::write(data.join("notes").join("1.md"), "edited").unwrap();
        assert!(back_up(&data, &backups, day(7), false).unwrap().is_some());
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn first_backup_happens_even_with_no_notes() {
        let data = std::env::temp_dir().join(format!("zima-backup-empty-{}", std::process::id()));
        let _ = fs::remove_dir_all(&data);
        fs::create_dir_all(&data).unwrap();
        let backups = data.join("backups");
        let path = back_up(&data, &backups, day(7), false).unwrap().unwrap();
        assert!(zip_names(&path).is_empty());
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn keeps_the_newest_fourteen() {
        let (data, backups) = scratch("keep");
        fs::create_dir_all(&backups).unwrap();
        for d in 1..=20 {
            fs::write(backups.join(file_name(day(d))), "old").unwrap();
        }
        // Not ours: never deleted.
        fs::write(backups.join("my-copy.zip"), "mine").unwrap();
        fs::write(backups.join("zima-notes.zip"), "mine").unwrap();
        back_up(&data, &backups, day(21), true).unwrap();
        let mut left = backup_names(&backups);
        left.sort();
        assert_eq!(left.len(), KEEP);
        assert_eq!(left.first().unwrap(), "zima-2026-10-08.zip");
        assert_eq!(left.last().unwrap(), "zima-2026-10-21.zip");
        assert!(backups.join("my-copy.zip").exists() && backups.join("zima-notes.zip").exists());
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn picks_old_backups_by_date_not_by_folder_order() {
        let names: Vec<String> = ["zima-2025-12-31.zip", "zima-2026-01-02.zip", "zima-2026-01-01.zip"].map(String::from).to_vec();
        // Fewer than KEEP: nothing to remove.
        assert!(to_remove(&names).is_empty());
        let many: Vec<String> = (3..=15).map(|d| file_name(NaiveDate::from_ymd_opt(2026, 1, d).unwrap())).chain(names).collect();
        assert_eq!(to_remove(&many), vec!["zima-2026-01-01.zip", "zima-2025-12-31.zip"]);
        assert!(to_remove(&[]).is_empty());
    }

    #[test]
    fn only_our_file_names_count_as_backups() {
        assert!(is_backup_name("zima-2026-10-07.zip"));
        assert!(!is_backup_name("zima-2026-10-07.zip.tmp"));
        assert!(!is_backup_name("zima-2026-13-07.zip"));
        assert!(!is_backup_name("zima-notes.zip"));
        assert!(!is_backup_name("notes-2026-10-07.zip"));
        assert!(!is_backup_name(""));
    }
}
