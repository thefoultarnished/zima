use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::model::{Note, NoteId, QuickTask, Reminder, UiState};

/// On-disk layout (under `%APPDATA%\Zima`, or `$ZIMA_DATA_DIR` if set):
///
/// ```text
/// notes/<id>.md   note bodies
/// index.json      note metadata
/// state.json      UI state
/// reminders.json  pending reminders
/// tasks.json      tasks added with `@due` lines
/// stats.json      words written per day
/// focus.json      focus-timer minutes per day
/// ```
pub struct Store {
    root: PathBuf,
}

/// Where `location.json` (which says where the notes live) is kept: `%APPDATA%\Zima`, or
/// `$ZIMA_DATA_DIR` so test runs never read or move the real data.
pub fn home_dir() -> io::Result<PathBuf> {
    if let Some(dir) = std::env::var_os("ZIMA_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let data = directories::BaseDirs::new().ok_or_else(|| io::Error::other("no home directory"))?.data_dir().to_path_buf();
    let home = data.join("Zima");
    let old = data.join("Note");
    // The app was called Note before; bring its folder over once. `Note` is left as it was, as a backup.
    // If the copy fails, keep using `Note` so no notes go missing; it's tried again next start.
    if !home.exists() && old.is_dir() && copy_folder_atomically(&old, &home).is_err() {
        return Ok(old);
    }
    Ok(home)
}

/// Copy a whole folder to `to` (which must not exist yet). Copies into a temporary folder first,
/// so `to` never exists half-copied.
fn copy_folder_atomically(from: &Path, to: &Path) -> io::Result<()> {
    let partial = to.with_extension("partial");
    let _ = fs::remove_dir_all(&partial);
    copy_tree(from, &partial)?;
    fs::rename(&partial, to)
}

fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// The notes folder chosen for syncing (e.g. inside Google Drive), if any.
pub fn chosen_location() -> Option<PathBuf> {
    let home = home_dir().ok()?;
    let value: serde_json::Value = read_json(&home.join("location.json"))?;
    value["data_dir"].as_str().map(PathBuf::from)
}

pub fn set_location(dir: Option<&Path>) -> io::Result<()> {
    let home = home_dir()?;
    fs::create_dir_all(&home)?;
    match dir {
        Some(dir) => write_json(&home.join("location.json"), &serde_json::json!({ "data_dir": dir })),
        None => match fs::remove_file(home.join("location.json")) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        },
    }
}

/// Google Drive for desktop's "My Drive" folder, if installed (usually G:\My Drive).
pub fn find_google_drive() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = ('D'..='Z').map(|letter| PathBuf::from(format!("{letter}:\\My Drive"))).collect();
    if let Some(dirs) = directories::UserDirs::new() {
        let home = dirs.home_dir();
        candidates.push(home.join("Google Drive").join("My Drive"));
        candidates.push(home.join("My Drive"));
        candidates.push(home.join("Google Drive"));
    }
    candidates.into_iter().find(|p| p.is_dir())
}

/// Copy every file of a Zima data folder to another (notes/, index.json, …), without overwriting.
pub fn copy_data(from: &Path, to: &Path) -> io::Result<usize> {
    fs::create_dir_all(to.join("notes"))?;
    let mut copied = 0;
    for name in ["index.json", "state.json", "reminders.json", "tasks.json", "stats.json", "focus.json"] {
        let (src, dst) = (from.join(name), to.join(name));
        if src.exists() && !dst.exists() {
            fs::copy(&src, &dst)?;
            copied += 1;
        }
    }
    if let Ok(entries) = fs::read_dir(from.join("notes")) {
        for entry in entries.flatten() {
            let dst = to.join("notes").join(entry.file_name());
            if !dst.exists() {
                fs::copy(entry.path(), dst)?;
                copied += 1;
            }
        }
    }
    Ok(copied)
}

impl Store {
    /// The folder chosen for syncing, else the home folder (`%APPDATA%\Zima` or `$ZIMA_DATA_DIR`).
    pub fn open() -> io::Result<Self> {
        let root = chosen_location().unwrap_or(home_dir()?);
        fs::create_dir_all(root.join("notes"))?;
        Ok(Self { root })
    }

    /// A store in a specific folder (used when moving notes between folders).
    pub fn at(root: &Path) -> io::Result<Self> {
        fs::create_dir_all(root.join("notes"))?;
        Ok(Self { root: root.to_path_buf() })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn reminders_modified(&self) -> Option<std::time::SystemTime> {
        fs::metadata(self.root.join("reminders.json")).and_then(|m| m.modified()).ok()
    }

    pub fn tasks_modified(&self) -> Option<std::time::SystemTime> {
        fs::metadata(self.root.join("tasks.json")).and_then(|m| m.modified()).ok()
    }

    /// When index.json last changed (to notice edits from other devices).
    pub fn index_modified(&self) -> Option<std::time::SystemTime> {
        fs::metadata(self.root.join("index.json")).and_then(|m| m.modified()).ok()
    }

    pub fn notes_dir(&self) -> PathBuf {
        self.root.join("notes")
    }

    /// A note's text as it is on disk, and when it was written: the version about to be replaced.
    pub fn saved_body(&self, id: NoteId) -> Option<(String, std::time::SystemTime)> {
        let path = self.note_path(id);
        let modified = fs::metadata(&path).and_then(|m| m.modified()).ok()?;
        Some((fs::read_to_string(&path).ok()?, modified))
    }

    fn note_path(&self, id: NoteId) -> PathBuf {
        self.root.join("notes").join(format!("{id}.md"))
    }

    pub fn load_notes(&self) -> Vec<Note> {
        let mut notes: Vec<Note> = read_json(&self.root.join("index.json")).unwrap_or_default();
        for note in &mut notes {
            note.body = fs::read_to_string(self.note_path(note.id)).unwrap_or_default();
        }
        notes
    }

    pub fn save_index(&self, notes: &[Note]) -> io::Result<()> {
        write_json(&self.root.join("index.json"), &notes)
    }

    pub fn save_body(&self, note: &Note) -> io::Result<()> {
        write_atomic(&self.note_path(note.id), note.body.as_bytes())
    }

    pub fn delete_body(&self, id: NoteId) -> io::Result<()> {
        match fs::remove_file(self.note_path(id)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    pub fn load_state(&self) -> UiState {
        read_json(&self.root.join("state.json")).unwrap_or_default()
    }

    pub fn save_state(&self, state: &UiState) -> io::Result<()> {
        write_json(&self.root.join("state.json"), state)
    }

    /// Words written per day ("YYYY-MM-DD" → words).
    pub fn load_stats(&self) -> std::collections::BTreeMap<String, u32> {
        read_json(&self.root.join("stats.json")).unwrap_or_default()
    }

    pub fn save_stats(&self, stats: &std::collections::BTreeMap<String, u32>) -> io::Result<()> {
        write_json(&self.root.join("stats.json"), stats)
    }

    /// Focus-timer minutes per day.
    pub fn load_focus(&self) -> std::collections::BTreeMap<String, u32> {
        read_json(&self.root.join("focus.json")).unwrap_or_default()
    }

    pub fn save_focus(&self, focus: &std::collections::BTreeMap<String, u32>) -> io::Result<()> {
        write_json(&self.root.join("focus.json"), focus)
    }

    pub fn load_reminders(&self) -> Vec<Reminder> {
        read_json(&self.root.join("reminders.json")).unwrap_or_default()
    }

    pub fn save_reminders(&self, reminders: &[Reminder]) -> io::Result<()> {
        write_json(&self.root.join("reminders.json"), reminders)
    }

    /// The `@due` tasks. A damaged tasks.json is first copied to `tasks.json.damaged-<unix ms>`, so the
    /// next save (of an empty list) doesn't destroy what could still be rescued from it.
    pub fn load_tasks(&self) -> Vec<QuickTask> {
        let path = self.root.join("tasks.json");
        let Ok(text) = fs::read_to_string(&path) else { return Vec::new() };
        match serde_json::from_str(&text) {
            Ok(tasks) => tasks,
            Err(e) => {
                eprintln!("ignoring unreadable {}: {e}", path.display());
                let backup = self.root.join(format!("tasks.json.damaged-{}", crate::model::now_ms()));
                if let Err(e) = fs::copy(&path, &backup) {
                    eprintln!("couldn't keep a copy of the damaged tasks.json: {e}");
                }
                Vec::new()
            }
        }
    }

    pub fn save_tasks(&self, tasks: &[QuickTask]) -> io::Result<()> {
        write_json(&self.root.join("tasks.json"), tasks)
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let text = fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        Err(e) => {
            eprintln!("ignoring unreadable {}: {e}", path.display());
            None
        }
    }
}

fn write_json<T: serde::Serialize + ?Sized>(path: &Path, value: &T) -> io::Result<()> {
    let json = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    write_atomic(path, &json)
}

/// Write to a temp file, then rename over the target, so a crash never leaves a half-written file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_the_old_data_folder_whole() {
        let base = std::env::temp_dir().join(format!("zima-migrate-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (old, new) = (base.join("Note"), base.join("Zima"));
        fs::create_dir_all(old.join("notes")).unwrap();
        fs::write(old.join("index.json"), "[]").unwrap();
        fs::write(old.join("theme.json"), "{}").unwrap();
        fs::write(old.join("notes").join("1.md"), "hello").unwrap();
        // Leftover from a copy that was interrupted last time.
        fs::create_dir_all(base.join("Zima.partial")).unwrap();
        fs::write(base.join("Zima.partial").join("stale.md"), "x").unwrap();

        copy_folder_atomically(&old, &new).unwrap();

        assert_eq!(fs::read_to_string(new.join("notes").join("1.md")).unwrap(), "hello");
        assert!(new.join("index.json").exists() && new.join("theme.json").exists());
        assert!(!new.join("stale.md").exists());
        assert!(!base.join("Zima.partial").exists());
        // The old folder is kept as a backup.
        assert_eq!(fs::read_to_string(old.join("notes").join("1.md")).unwrap(), "hello");
        let _ = fs::remove_dir_all(&base);
    }

    fn scratch_store(name: &str) -> (Store, PathBuf) {
        let root = std::env::temp_dir().join(format!("zima-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        (Store::at(&root).unwrap(), root)
    }

    #[test]
    fn tasks_save_and_load() {
        let (store, root) = scratch_store("tasks");
        // No file yet: no tasks.
        assert!(store.load_tasks().is_empty());
        let task = QuickTask { id: 1, text: "rent".into(), due: Some("2026-10-09".into()), done: false, created: 5 };
        store.save_tasks(std::slice::from_ref(&task)).unwrap();
        assert_eq!(store.load_tasks(), vec![task]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn damaged_tasks_file_is_kept_aside() {
        let (store, root) = scratch_store("tasks-damaged");
        fs::write(root.join("tasks.json"), "[{\"id\": 1, \"text\": \"rent\"").unwrap();
        assert!(store.load_tasks().is_empty());
        let kept = || -> Vec<String> {
            let mut texts: Vec<String> = fs::read_dir(&root)
                .unwrap()
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("tasks.json.damaged-"))
                .map(|e| fs::read_to_string(e.path()).unwrap())
                .collect();
            texts.sort();
            texts
        };
        assert_eq!(kept(), vec!["[{\"id\": 1, \"text\": \"rent\""]);
        // Saving afterwards leaves the copy alone, and a second damaged file gets its own copy.
        store.save_tasks(&[]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        fs::write(root.join("tasks.json"), "also bad").unwrap();
        store.load_tasks();
        assert_eq!(kept(), vec!["[{\"id\": 1, \"text\": \"rent\"", "also bad"]);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn settings_from_before_interface_size_still_load() {
        let (store, root) = scratch_store("old-state");
        fs::write(root.join("state.json"), r#"{"theme": 2, "zoom": 1.3}"#).unwrap();
        let state = store.load_state();
        // The old settings are kept, and the new one starts at 100%.
        assert_eq!(state.theme, 2);
        assert_eq!(state.zoom, 1.3);
        assert_eq!(state.ui_scale, 1.0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn interface_size_is_saved_and_read_back() {
        let (store, root) = scratch_store("ui-scale");
        let state = UiState { ui_scale: 1.5, ..Default::default() };
        store.save_state(&state).unwrap();
        assert_eq!(store.load_state().ui_scale, 1.5);
        // Text size is a separate setting and isn't changed by it.
        assert_eq!(store.load_state().zoom, 1.0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn keep_running_in_tray_is_on_for_old_settings() {
        // Closing used to always hide to the tray; old settings must keep that behaviour.
        let (store, root) = scratch_store("old-close");
        fs::write(root.join("state.json"), r#"{"tray_hint_shown": true}"#).unwrap();
        let state = store.load_state();
        assert!(state.close_to_tray);
        assert!(state.tray_hint_shown);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn keep_running_in_tray_off_is_saved() {
        let (store, root) = scratch_store("close-off");
        store.save_state(&UiState { close_to_tray: false, ..Default::default() }).unwrap();
        assert!(!store.load_state().close_to_tray);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_settings_file_gives_defaults() {
        let (store, root) = scratch_store("no-state");
        assert_eq!(store.load_state().ui_scale, 1.0);
        assert!(store.load_state().close_to_tray);
        let _ = fs::remove_dir_all(&root);
    }
}
