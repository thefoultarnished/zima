use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::model::{Note, NoteId, Reminder, UiState};

/// On-disk layout (under `%APPDATA%\Zima`, or `$ZIMA_DATA_DIR` if set):
///
/// ```text
/// notes/<id>.md   note bodies
/// index.json      note metadata
/// state.json      UI state
/// reminders.json  pending reminders
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
    for name in ["index.json", "state.json", "reminders.json", "stats.json", "focus.json"] {
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

    /// When index.json last changed (to notice edits from other devices).
    pub fn index_modified(&self) -> Option<std::time::SystemTime> {
        fs::metadata(self.root.join("index.json")).and_then(|m| m.modified()).ok()
    }

    pub fn notes_dir(&self) -> PathBuf {
        self.root.join("notes")
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
fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
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
