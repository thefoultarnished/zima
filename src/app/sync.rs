//! Folder sync: keep the notes in a synced folder (Google Drive first, or any folder such as
//! OneDrive/Dropbox) and pick up changes made on other devices.

use std::path::{Path, PathBuf};
use std::time::Duration;

use slint::{Timer, TimerMode};

use super::{App, display_title};
use crate::model::{Note, now_ms};
use crate::store::{self, Store};

/// How often to look for changes from other devices. Rare on purpose: notes and reminders are
/// assumed to be written by this app only, so changes made elsewhere (another device, `zima add`,
/// `zima remind`) can take up to an hour to show, and saving here before then can overwrite them.
const WATCH_INTERVAL: Duration = Duration::from_secs(60 * 60);

impl App {
    /// Settings shows where notes live and whether Google Drive was found.
    pub fn refresh_sync_ui(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.set_sync_location(self.store.root().display().to_string().into());
        ui.set_sync_active(store::chosen_location().is_some());
        ui.set_google_drive_path(store::find_google_drive().map(|p| p.display().to_string()).unwrap_or_default().into());
    }

    pub fn use_google_drive(&mut self) {
        match store::find_google_drive() {
            Some(drive) => self.move_notes_to(drive.join("Zima")),
            None => self.toast("Google Drive for desktop isn't installed (or isn't signed in)", true),
        }
    }

    pub fn choose_sync_folder(&mut self) {
        let Some(folder) = rfd::FileDialog::new().set_title("Keep notes in this folder").pick_folder() else { return };
        // Use the folder itself if it's already a Zima folder; otherwise a "Zima" folder inside it.
        let target = if folder.join("index.json").exists() || folder.file_name().is_some_and(|n| n.eq_ignore_ascii_case("zima")) {
            folder
        } else {
            folder.join("Zima")
        };
        self.move_notes_to(target);
    }

    /// Back to the local folder (%APPDATA%\Zima), bringing the synced notes along.
    pub fn stop_syncing(&mut self) {
        let Ok(home) = store::home_dir() else { return };
        self.flush();
        if let Err(e) = merge_into(self.store.root(), &home) {
            self.toast(&format!("Couldn't copy notes back: {e}"), true);
            return;
        }
        if let Err(e) = store::set_location(None) {
            self.toast(&format!("Couldn't stop syncing: {e}"), true);
            return;
        }
        self.restart();
    }

    fn move_notes_to(&mut self, target: PathBuf) {
        if target == self.store.root() {
            self.toast("Your notes are already there", false);
            return;
        }
        self.flush();
        // Merge rather than overwrite: the folder may already hold notes from another device.
        if let Err(e) = merge_into(self.store.root(), &target) {
            self.toast(&format!("Couldn't move notes: {e}"), true);
            return;
        }
        if let Err(e) = store::set_location(Some(&target)) {
            self.toast(&format!("Couldn't save the location: {e}"), true);
            return;
        }
        self.restart();
    }

    /// Start a fresh copy of Zima (it reads the new location) and quit this one.
    pub(super) fn restart(&mut self) {
        self.flush();
        // The new copy waits for this one to finish quitting instead of handing over to it.
        let mut args: Vec<String> = std::env::args()
            .skip(1)
            .filter(|a| a != crate::system::HIDDEN_ARG && a != crate::instance::RESTART_ARG)
            .collect();
        args.push(crate::instance::RESTART_ARG.into());
        match std::env::current_exe().and_then(|exe| std::process::Command::new(exe).args(args).spawn()) {
            Ok(_) => {
                let _ = slint::quit_event_loop();
            }
            Err(e) => self.toast(&format!("Restart Zima to finish: {e}"), true),
        }
    }

    /// Check now and then whether another device changed the notes.
    pub fn watch_for_external_changes(&mut self) {
        self.last_index_write = self.store.index_modified();
        self.last_reminders_write = self.store.reminders_modified();
        self.last_tasks_write = self.store.tasks_modified();
        let app = self.this.clone();
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, WATCH_INTERVAL, move || {
            let Some(app) = app.upgrade() else { return };
            let Ok(mut app) = app.try_borrow_mut() else { return };
            let modified = app.store.index_modified();
            if modified.is_some() && modified != app.last_index_write {
                app.last_index_write = modified;
                if !app.reload_external() {
                    app.last_index_write = None;
                }
            }
            // Reminders added elsewhere (another device, or `zima remind`).
            let reminders_modified = app.store.reminders_modified();
            if reminders_modified.is_some() && reminders_modified != app.last_reminders_write {
                // Half-written by a sync app: keep ours and look again next time.
                match app.store.read_reminders() {
                    Some(reminders) => {
                        app.last_reminders_write = reminders_modified;
                        app.reminders = reminders;
                        app.refresh_reminders();
                    }
                    None => app.last_reminders_write = None,
                }
            }
            // `@due` tasks added or ticked on another device.
            let tasks_modified = app.store.tasks_modified();
            if tasks_modified.is_some() && tasks_modified != app.last_tasks_write {
                match app.store.read_tasks() {
                    Some(tasks) => {
                        app.last_tasks_write = tasks_modified;
                        app.quick_tasks = tasks;
                        app.refresh_tasks();
                    }
                    None => app.last_tasks_write = None,
                }
            }
        });
        std::mem::forget(timer);
    }

    /// Merge notes changed elsewhere into what's in memory. Returns false if the note list
    /// couldn't be read (a sync app may still be writing it), so it's tried again later.
    fn reload_external(&mut self) -> bool {
        let Some(disk) = self.store.read_notes() else { return false };
        let mut changed_current = false;
        let mut old_body = String::new();
        let mut replaced = Vec::new();
        let mut conflicts = 0;
        for incoming in disk {
            match self.notes.iter().position(|n| n.id == incoming.id) {
                None => self.notes.push(incoming),
                // Not readable on disk right now: keep what's in memory, try again next time.
                Some(_) if incoming.unreadable => {}
                Some(i) => {
                    let ours = &self.notes[i];
                    let same = ours.body == incoming.body && ours.title == incoming.title && ours.deleted_at == incoming.deleted_at;
                    if same || incoming.modified <= ours.modified {
                        continue;
                    }
                    if self.dirty.contains(&incoming.id) && ours.body != incoming.body {
                        // Both sides edited: keep ours, and save theirs as a separate note.
                        let id = self.next_note_id();
                        let title = format!("{} (conflict from another device)", display_title(&incoming));
                        self.notes.push(Note { id, title, created: now_ms(), modified: now_ms(), daily: None, ..incoming });
                        self.dirty.insert(id);
                        conflicts += 1;
                    } else {
                        if Some(incoming.id) == self.state.current {
                            changed_current = true;
                            old_body = ours.body.clone();
                        }
                        replaced.push(incoming.id);
                        self.notes[i] = incoming;
                    }
                }
            }
        }
        for id in replaced {
            self.sync_sticky(id);
        }
        self.index_dirty = self.index_dirty || conflicts > 0;
        self.refresh_lists();
        self.refresh_reminders();
        // Another device may have added or removed links to the open note.
        self.refresh_backlinks();
        if changed_current {
            if let (Some(ui), Some(note)) = (self.ui.upgrade(), self.current()) {
                ui.set_note_title(note.title.as_str().into());
            }
            self.show_body_and_selection(&old_body, None);
            self.after_body_change();
        }
        if conflicts > 0 {
            self.flush();
            self.toast(&format!("{conflicts} note(s) changed on two devices; both versions kept"), false);
        }
        true
    }
}

impl App {
    /// Import a folder of Markdown files (an Obsidian vault, a notes folder …).
    pub fn import_markdown_folder(&mut self) {
        let Some(folder) = rfd::FileDialog::new().set_title("Import a folder of Markdown notes").pick_folder() else { return };
        let mut next = self.next_note_id();
        let mut next_id = || {
            next += 1;
            next - 1
        };
        let imported = crate::transfer::import_markdown_folder(&folder, &mut next_id);
        let count = imported.len();
        for note in imported {
            self.dirty.insert(note.id);
            self.notes.push(note);
        }
        self.index_dirty = true;
        self.flush();
        self.refresh_lists();
        self.toast(&format!("Imported {count} note{}", if count == 1 { "" } else { "s" }), false);
    }

    /// Every note (except the Bin) as a zip of Markdown files.
    pub fn export_all(&mut self) {
        let name = format!("Notes {}.zip", chrono::Local::now().format("%Y-%m-%d"));
        let Some(path) = rfd::FileDialog::new().set_title("Export all notes").set_file_name(name).add_filter("Zip", &["zip"]).save_file() else { return };
        let notes: Vec<&Note> = self.notes.iter().filter(|n| n.is_live()).collect();
        match crate::transfer::export_zip(&notes, &path) {
            Ok(count) => self.toast(&format!("Exported {count} notes to {}", path.display()), false),
            Err(e) => self.toast(&format!("Export failed: {e}"), true),
        }
    }
}

/// Merge one data folder into another: every note ends up in `to`, the newer copy winning.
fn merge_into(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to.join("notes"))?;
    let source = Store::at(from)?;
    let target = Store::at(to)?;
    let (mut merged, _) = target.load_notes();
    for note in source.load_notes().0 {
        match merged.iter().position(|n| n.id == note.id) {
            Some(i) if merged[i].modified >= note.modified => {}
            Some(i) => merged[i] = note,
            None => merged.push(note),
        }
    }
    for note in &merged {
        target.save_body(note)?;
    }
    target.save_index(&merged)?;
    // Settings, reminders and stats: copy if the target doesn't have them yet.
    store::copy_data(from, to)?;
    Ok(())
}
