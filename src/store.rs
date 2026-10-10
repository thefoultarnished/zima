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

    /// Every note, plus how many had to be rebuilt from their files (0 normally). If index.json
    /// can't be read, or is missing while note files exist, a copy of it is kept as
    /// `index.json.damaged-<unix ms>` and the list is rebuilt from `notes/` and saved, so no note
    /// goes missing. Rebuilt notes have no title (the first line shows instead).
    pub fn load_notes(&self) -> (Vec<Note>, usize) {
        let path = self.root.join("index.json");
        if let Some(notes) = self.read_notes() {
            return (notes, 0);
        }
        if path.exists() {
            keep_damaged(&path);
        }
        let notes = self.notes_from_files();
        if !notes.is_empty() {
            if let Err(e) = self.save_index(&notes) {
                eprintln!("couldn't save the rebuilt note list: {e}");
            }
        }
        let count = notes.len();
        (notes, count)
    }

    /// Every note, or `None` when index.json is missing or can't be read (for example while a
    /// sync app is still writing it). Changes nothing on disk.
    pub fn read_notes(&self) -> Option<Vec<Note>> {
        let mut notes: Vec<Note> = read_json(&self.root.join("index.json"))?;
        for note in &mut notes {
            match self.read_body(note.id) {
                Some(body) => note.body = body,
                None => note.unreadable = true,
            }
        }
        Some(notes)
    }

    /// A note's text, `""` if it has no file yet, or `None` if the file is there but can't be read.
    pub fn read_body(&self, id: NoteId) -> Option<String> {
        match fs::read_to_string(self.note_path(id)) {
            Ok(body) => Some(body),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Some(String::new()),
            Err(e) => {
                eprintln!("couldn't read note {id}: {e}");
                None
            }
        }
    }

    /// A note for every `notes/<id>.md` file, dated by when the file last changed.
    fn notes_from_files(&self) -> Vec<Note> {
        let Ok(entries) = fs::read_dir(self.notes_dir()) else { return Vec::new() };
        let mut notes: Vec<Note> = entries
            .flatten()
            .filter_map(|entry| {
                let id: NoteId = entry.file_name().to_str()?.strip_suffix(".md")?.parse().ok()?;
                let body = self.read_body(id);
                let modified = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or_else(crate::model::now_ms, |d| d.as_millis() as i64);
                Some(Note { id, unreadable: body.is_none(), body: body.unwrap_or_default(), modified, created: modified, ..Default::default() })
            })
            .collect();
        notes.sort_by_key(|n| n.id);
        notes
    }

    pub fn save_index(&self, notes: &[Note]) -> io::Result<()> {
        write_json(&self.root.join("index.json"), &notes)
    }

    /// Write a note's text. A note whose file couldn't be read is skipped: its real text is still
    /// in that file, and the empty stand-in must never replace it.
    pub fn save_body(&self, note: &Note) -> io::Result<()> {
        if note.unreadable {
            return Ok(());
        }
        write_atomic(&self.note_path(note.id), note.body.as_bytes())
    }

    pub fn delete_body(&self, id: NoteId) -> io::Result<()> {
        match fs::remove_file(self.note_path(id)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    pub fn load_state(&self) -> UiState {
        self.load_or_keep("state.json")
    }

    pub fn save_state(&self, state: &UiState) -> io::Result<()> {
        write_json(&self.root.join("state.json"), state)
    }

    /// Words written per day ("YYYY-MM-DD" → words).
    pub fn load_stats(&self) -> std::collections::BTreeMap<String, u32> {
        self.load_or_keep("stats.json")
    }

    pub fn save_stats(&self, stats: &std::collections::BTreeMap<String, u32>) -> io::Result<()> {
        write_json(&self.root.join("stats.json"), stats)
    }

    /// Focus-timer minutes per day.
    pub fn load_focus(&self) -> std::collections::BTreeMap<String, u32> {
        self.load_or_keep("focus.json")
    }

    pub fn save_focus(&self, focus: &std::collections::BTreeMap<String, u32>) -> io::Result<()> {
        write_json(&self.root.join("focus.json"), focus)
    }

    pub fn load_reminders(&self) -> Vec<Reminder> {
        self.load_or_keep("reminders.json")
    }

    /// The reminders, or `None` if reminders.json can't be read right now (a sync app may still be
    /// writing it). Changes nothing on disk.
    pub fn read_reminders(&self) -> Option<Vec<Reminder>> {
        read_json(&self.root.join("reminders.json"))
    }

    pub fn save_reminders(&self, reminders: &[Reminder]) -> io::Result<()> {
        write_json(&self.root.join("reminders.json"), reminders)
    }

    /// The `@due` tasks.
    pub fn load_tasks(&self) -> Vec<QuickTask> {
        self.load_or_keep("tasks.json")
    }

    /// The tasks, or `None` if tasks.json can't be read right now. Changes nothing on disk.
    pub fn read_tasks(&self) -> Option<Vec<QuickTask>> {
        read_json(&self.root.join("tasks.json"))
    }

    /// A data file, or its defaults if it's missing or damaged. A damaged file is first copied to
    /// `<name>.damaged-<unix ms>` (see [`keep_damaged`]), so the next save of the defaults doesn't
    /// destroy what could still be rescued from it.
    fn load_or_keep<T: serde::de::DeserializeOwned + Default>(&self, name: &str) -> T {
        let path = self.root.join(name);
        if !path.exists() {
            return T::default();
        }
        read_json(&path).unwrap_or_else(|| {
            keep_damaged(&path);
            T::default()
        })
    }

    pub fn save_tasks(&self, tasks: &[QuickTask]) -> io::Result<()> {
        write_json(&self.root.join("tasks.json"), tasks)
    }
}

/// Copy a damaged data file to `<name>.damaged-<unix ms>` next to it, unless an earlier copy already
/// holds exactly the same bytes (it's read at every start until a good file replaces it).
fn keep_damaged(path: &Path) {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else { return };
    let Ok(bytes) = fs::read(path) else { return };
    let prefix = format!("{name}.damaged-");
    let already_kept = fs::read_dir(dir).into_iter().flatten().flatten().any(|entry| {
        entry.file_name().to_str().is_some_and(|n| n.starts_with(&prefix)) && fs::read(entry.path()).is_ok_and(|kept| kept == bytes)
    });
    if already_kept {
        return;
    }
    let backup = dir.join(format!("{prefix}{}", crate::model::now_ms()));
    if let Err(e) = fs::write(&backup, &bytes) {
        eprintln!("couldn't keep a copy of the damaged {name}: {e}");
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

    #[test]
    fn live_view_mode_is_saved_and_loaded() {
        let (store, root) = scratch_store("view-mode-live");
        fs::write(root.join("state.json"), r#"{"view_mode": 3}"#).unwrap();
        assert_eq!(store.load_state().view_mode, 3);
        let state = UiState { view_mode: 3, ..Default::default() };
        store.save_state(&state).unwrap();
        assert_eq!(store.load_state().view_mode, 3);
        let _ = fs::remove_dir_all(&root);
    }

    fn scratch_store(name: &str) -> (Store, PathBuf) {
        let root = std::env::temp_dir().join(format!("zima-store-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        (Store::at(&root).unwrap(), root)
    }

    fn write_note(root: &Path, id: NoteId, body: &str) {
        fs::write(root.join("notes").join(format!("{id}.md")), body).unwrap();
    }

    #[test]
    fn damaged_note_list_is_kept_and_rebuilt() {
        let (store, root) = scratch_store("index-damaged");
        write_note(&root, 2, "# Second
more");
        write_note(&root, 1, "first");
        let damaged = r#"[{"id": 1, "title": "Shopping", "modi"#;
        fs::write(root.join("index.json"), damaged).unwrap();

        let (notes, rebuilt) = store.load_notes();
        assert_eq!(rebuilt, 2);
        let found: Vec<(NoteId, &str)> = notes.iter().map(|n| (n.id, n.body.as_str())).collect();
        assert_eq!(found, vec![(1, "first"), (2, "# Second
more")]);
        // The damaged list is kept as it was, and a readable one is saved in its place.
        let kept: Vec<String> = fs::read_dir(&root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with("index.json.damaged-"))
            .map(|e| fs::read_to_string(e.path()).unwrap())
            .collect();
        assert_eq!(kept, vec![damaged]);
        assert_eq!(store.read_notes().unwrap().len(), 2);
        // Next start loads normally.
        assert_eq!(store.load_notes().1, 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_note_list_is_rebuilt_from_files() {
        let (store, root) = scratch_store("index-missing");
        write_note(&root, 5, "hello");
        // Not notes: left alone.
        fs::write(root.join("notes").join("readme.md"), "x").unwrap();
        fs::write(root.join("notes").join("6.tmp"), "x").unwrap();
        let (notes, rebuilt) = store.load_notes();
        assert_eq!((notes.len(), rebuilt), (1, 1));
        assert_eq!((notes[0].id, notes[0].title.as_str(), notes[0].is_live()), (5, "", true));
        assert!(notes[0].modified > 0);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn good_or_empty_note_list_is_left_alone() {
        let (store, root) = scratch_store("index-good");
        // Fresh start: nothing to rebuild, nothing written.
        let (notes, rebuilt) = store.load_notes();
        assert_eq!((notes.len(), rebuilt), (0, 0));
        assert!(!root.join("index.json").exists());
        write_note(&root, 1, "body");
        let note = Note { id: 1, title: "Kept".into(), pinned: true, ..Default::default() };
        store.save_index(std::slice::from_ref(&note)).unwrap();
        let (notes, rebuilt) = store.load_notes();
        assert_eq!(rebuilt, 0);
        assert_eq!((notes[0].title.as_str(), notes[0].pinned, notes[0].body.as_str()), ("Kept", true, "body"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn reading_a_damaged_list_changes_nothing() {
        let (store, root) = scratch_store("index-read");
        write_note(&root, 1, "body");
        fs::write(root.join("index.json"), "[{").unwrap();
        assert!(store.read_notes().is_none());
        assert_eq!(fs::read_to_string(root.join("index.json")).unwrap(), "[{");
        assert_eq!(fs::read_dir(&root).unwrap().count(), 2);
        let _ = fs::remove_dir_all(&root);
    }

    /// Not valid UTF-8, so it can't be read as text (like a note saved as "ANSI" with an accent).
    const UNREADABLE: &[u8] = b"caf\xe9 notes";

    #[test]
    fn unreadable_note_is_marked_and_never_saved_over() {
        let (store, root) = scratch_store("unreadable");
        fs::write(root.join("notes").join("1.md"), UNREADABLE).unwrap();
        write_note(&root, 2, "fine");
        // Note 3 has no file yet: that's just an empty note, not an unreadable one.
        let index: Vec<Note> = (1..=3).map(|id| Note { id, ..Default::default() }).collect();
        store.save_index(&index).unwrap();

        let notes = store.read_notes().unwrap();
        let seen: Vec<(NoteId, &str, bool)> = notes.iter().map(|n| (n.id, n.body.as_str(), n.unreadable)).collect();
        assert_eq!(seen, vec![(1, "", true), (2, "fine", false), (3, "", false)]);

        // Saving the stand-in leaves the real file alone; normal notes still save.
        store.save_body(&Note { body: "typed".into(), ..notes[0].clone() }).unwrap();
        assert_eq!(fs::read(root.join("notes").join("1.md")).unwrap(), UNREADABLE);
        store.save_body(&Note { body: "new".into(), ..notes[2].clone() }).unwrap();
        assert_eq!(store.read_body(3).as_deref(), Some("new"));
        // The unreadable flag is never written to the note list.
        store.save_index(&notes).unwrap();
        assert!(!fs::read_to_string(root.join("index.json")).unwrap().contains("unreadable"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rebuilt_list_keeps_unreadable_notes() {
        let (store, root) = scratch_store("rebuild-unreadable");
        fs::write(root.join("notes").join("4.md"), UNREADABLE).unwrap();
        let (notes, rebuilt) = store.load_notes();
        assert_eq!(rebuilt, 1);
        assert!(notes[0].unreadable && notes[0].body.is_empty());
        assert_eq!(fs::read(root.join("notes").join("4.md")).unwrap(), UNREADABLE);
        let _ = fs::remove_dir_all(&root);
    }

    /// The contents of every `<name>.damaged-*` copy in `root`, sorted.
    fn kept_copies(root: &Path, name: &str) -> Vec<String> {
        let mut texts: Vec<String> = fs::read_dir(root)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(&format!("{name}.damaged-")))
            .map(|e| fs::read_to_string(e.path()).unwrap())
            .collect();
        texts.sort();
        texts
    }

    #[test]
    fn damaged_data_files_are_kept_before_defaults_are_used() {
        let (store, root) = scratch_store("damaged-files");
        let names = ["reminders.json", "state.json", "stats.json", "focus.json"];
        for name in names {
            fs::write(root.join(name), format!("{{\"broken {name}")).unwrap();
        }
        assert!(store.load_reminders().is_empty());
        assert_eq!(store.load_state().theme, UiState::default().theme);
        assert!(store.load_stats().is_empty());
        assert!(store.load_focus().is_empty());
        for name in names {
            assert_eq!(kept_copies(&root, name), vec![format!("{{\"broken {name}")], "{name}");
        }
        // Read again (the next start, or settings read twice at startup): no second copy.
        store.load_state();
        store.load_reminders();
        assert_eq!(kept_copies(&root, "state.json").len(), 1);
        assert_eq!(kept_copies(&root, "reminders.json").len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn good_or_missing_data_files_make_no_copies() {
        let (store, root) = scratch_store("good-files");
        let reminder = Reminder { id: 1, note_id: 0, text: "stretch".into(), due: 5, repeat: None };
        store.save_reminders(std::slice::from_ref(&reminder)).unwrap();
        assert_eq!(store.load_reminders().len(), 1);
        assert!(store.load_stats().is_empty());
        let names: Vec<String> = fs::read_dir(&root).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert!(names.iter().all(|n| !n.contains("damaged")), "{names:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn rereading_a_damaged_file_changes_nothing() {
        let (store, root) = scratch_store("reread");
        fs::write(root.join("reminders.json"), "[{").unwrap();
        fs::write(root.join("tasks.json"), "[{").unwrap();
        assert!(store.read_reminders().is_none());
        assert!(store.read_tasks().is_none());
        assert!(kept_copies(&root, "reminders.json").is_empty() && kept_copies(&root, "tasks.json").is_empty());
        let _ = fs::remove_dir_all(&root);
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
    fn settings_from_removed_features_still_load() {
        // Older versions saved fields that no longer exist; the rest must still load.
        let (store, root) = scratch_store("removed-field");
        fs::write(root.join("state.json"), r#"{"theme": 3, "old_setting": "abc", "zoom": 1.2}"#).unwrap();
        let state = store.load_state();
        assert_eq!((state.theme, state.zoom), (3, 1.2));
        // The next save drops the old field.
        store.save_state(&state).unwrap();
        assert!(!fs::read_to_string(root.join("state.json")).unwrap().contains("old_setting"));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn divider_sizes_are_saved_and_default_for_old_settings() {
        let (store, root) = scratch_store("dividers");
        fs::write(root.join("state.json"), r#"{"theme": 2}"#).unwrap();
        let state = store.load_state();
        assert_eq!((state.sidebar_width, state.split_ratio), (260.0, 0.5));
        store.save_state(&UiState { sidebar_width: 320.0, split_ratio: 0.4, ..state }).unwrap();
        let state = store.load_state();
        assert_eq!((state.sidebar_width, state.split_ratio, state.theme), (320.0, 0.4, 2));
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
