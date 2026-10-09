//! App state: owns the notes, keeps the UI in sync, and saves to disk.

mod backups;
mod capture;
mod custom_theme;
mod exchange;
use custom_theme::CUSTOM;
/// Highest theme number (8 Sakura, 9 Cyberpunk, 10 Expedition 33).
const LAST_THEME: i32 = 10;
mod finding;
pub use organize::extract_tags;
mod later;
mod present;
mod shortcuts;
mod navigate;
mod organize;
mod stats;
mod sticky;
mod summary;
mod sync;
mod tasks_view;
mod versions;
mod words;

use std::cell::RefCell;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::{self, Rc};
use std::time::Duration;

use chrono::{Local, TimeZone};
use slint::{ComponentHandle, Model, ModelRc, StyledText, Timer, TimerMode, VecModel, Weak};

use crate::markdown::{self, Kind};
use crate::model::{Note, NoteId, Reminder, UiState, now_ms};
use crate::store::Store;
use crate::{AppWindow, ColourDot, MdBlock, NoteRow, ReminderRow, Suggestion, Theme};
use crate::{commands, format, import, reminders, system, window};
use navigate::{PaletteEntry, PaletteMode};

/// How long to wait after the last keystroke before writing to disk.
const SAVE_DELAY: Duration = Duration::from_millis(500);
/// How soon to try again after a save failed (disk full, sync folder gone, file locked).
const SAVE_RETRY: Duration = Duration::from_secs(10);
/// How long typing must pause before the preview is redrawn and words are counted. Doing either
/// on every key made typing in a long note lag.
const PAUSE_DELAY: Duration = Duration::from_millis(150);
const RECENT_COUNT: usize = 5;
const UI_SCALE_MIN: f32 = 0.8;
const UI_SCALE_MAX: f32 = 2.0;
const TOAST_DURATION: Duration = Duration::from_millis(3500);
/// Wait a little after launch before the daily backup, so it doesn't slow down startup.
const BACKUP_DELAY: Duration = Duration::from_secs(10);
/// How often to check for due reminders.
const REMINDER_CHECK: Duration = Duration::from_secs(5);
/// Colour of `==highlighted==` text in Preview: amber, readable on light and dark themes.
const HIGHLIGHT_LIGHT: &str = "#b45309";
const HIGHLIGHT_DARK: &str = "#fbbf24";

pub struct App {
    store: Store,
    notes: Vec<Note>,
    state: UiState,
    search: String,
    /// Notes whose body changed since the last save.
    dirty: HashSet<NoteId>,
    index_dirty: bool,
    reminders: Vec<Reminder>,
    /// Tasks added with `@due` lines (`tasks.json`).
    quick_tasks: Vec<crate::model::QuickTask>,
    /// Open sticky-note windows.
    stickies: Vec<sticky::Sticky>,
    /// The quick-capture window, only while it's open.
    capture: Option<crate::CaptureWindow>,
    /// Keeps the global Ctrl+Alt+N hotkey registered.
    hotkeys: Option<global_hotkey::GlobalHotKeyManager>,
    /// Words written per day.
    stats: std::collections::BTreeMap<String, u32>,
    stats_dirty: bool,
    /// Focus-timer minutes per day.
    focus: std::collections::BTreeMap<String, u32>,
    /// Modification times of index.json / reminders.json as we last wrote or read them, to notice
    /// changes made by another device (folder sync) or the command line.
    last_index_write: Option<std::time::SystemTime>,
    last_reminders_write: Option<std::time::SystemTime>,
    last_tasks_write: Option<std::time::SystemTime>,
    /// Windows spell checker, created on first use.
    #[cfg(windows)]
    spell: Option<crate::spell::Checker>,
    /// Byte offset of the `@` the suggestion popup is showing for.
    suggestion_at: Option<usize>,
    /// An `@` whose suggestions were dismissed with Esc.
    dismissed_at: Option<usize>,
    cursor: usize,
    /// The note the editor shows, whose cursor `cursor` is.
    shown: Option<NoteId>,
    /// Notes picked with Ctrl+click in the sidebar, for doing something to all of them at once.
    selected: Vec<NoteId>,
    /// Find bar: the query, every match (byte ranges) and the selected one.
    find_query: String,
    find_matches: Vec<(usize, usize)>,
    find_index: usize,
    /// What each of the toast's buttons does.
    toast_actions: Vec<ToastAction>,
    timer: Option<FocusTimer>,
    /// Undo/redo for edits made from Rust (formatting, commands), which the text field can't undo itself.
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    palette_mode: PaletteMode,
    palette_entries: Vec<PaletteEntry>,
    /// Titles on the link hover card now showing (empty: none).
    link_preview: Vec<String>,
    /// Closed notes, most recent last (Ctrl+Shift+T).
    recently_closed: Vec<NoteId>,
    /// First day of the month the calendar shows.
    calendar_month: chrono::NaiveDate,
    /// What the keyboard shortcuts list is filtered by.
    pub(crate) shortcut_filter: String,
    /// The last good theme.json, and when it was last changed.
    custom_theme: custom_theme::CustomTheme,
    custom_theme_modified: Option<std::time::SystemTime>,
    /// Presentation mode: the slides, and which one is showing.
    slides: Vec<Vec<MdBlock>>,
    slide: usize,
    restore_to: Option<crate::window::Placement>,
    /// The day the automatic backup last ran (or found nothing to do).
    backed_up_on: Option<chrono::NaiveDate>,
    save_timer: Timer,
    pause_timer: Timer,
    /// Words in the open note at the last count; words typed beyond it go into the writing stats.
    word_base: usize,
    /// What the lists need from each note's text, kept until the note changes.
    summaries: RefCell<summary::Summaries>,
    /// When each note's newest history copy was made (or found on disk), so copies are kept only every few minutes.
    last_copy: std::collections::HashMap<NoteId, i64>,
    /// The history panel's copies of the open note (unix ms, newest first).
    versions: Vec<i64>,
    toast_timer: Timer,
    /// Exchange rates for `@curr`, and their download.
    exchange: exchange::Exchange,
    /// The last save failed and the user was told; cleared (with a message) once saving works again.
    save_failed: bool,
    this: rc::Weak<RefCell<App>>,
    ui: Weak<AppWindow>,
}

#[derive(Clone)]
enum ToastAction {
    /// Re-schedule a fired reminder this many minutes from now (`None` = tomorrow 9am).
    Snooze(Reminder, Option<i64>),
    StartTimer(u32, bool),
    /// Turn a line of text into a reminder.
    Remind(String, i64),
    OpenNote(NoteId),
    Restart,
    ShowBackups,
    /// The folder holding index.json (and the copy of a damaged one).
    ShowDataFolder,
}

struct FocusTimer {
    /// Unix ms.
    ends: i64,
    minutes: u32,
    is_break: bool,
}

struct Snapshot {
    note: NoteId,
    before: String,
    after: String,
    /// Cursor to restore when undoing.
    cursor: usize,
}

impl App {
    /// `import`: Zima files to import before showing anything (from `--import`).
    pub fn load(store: Store, ui: &AppWindow, import: &[PathBuf]) -> Rc<RefCell<Self>> {
        let (notes, rebuilt) = store.load_notes();
        let state = store.load_state();
        let reminders = store.load_reminders();
        let quick_tasks = store.load_tasks();
        let stats = store.load_stats();
        let focus = store.load_focus();
        let app = Rc::new(RefCell::new(Self {
            store,
            notes,
            state,
            search: String::new(),
            dirty: HashSet::new(),
            index_dirty: false,
            reminders,
            quick_tasks,
            stickies: Vec::new(),
            capture: None,
            hotkeys: None,
            stats,
            stats_dirty: false,
            focus,
            last_index_write: None,
            last_reminders_write: None,
            last_tasks_write: None,
            #[cfg(windows)]
            spell: None,
            suggestion_at: None,
            dismissed_at: None,
            cursor: 0,
            shown: None,
            selected: Vec::new(),
            find_query: String::new(),
            find_matches: Vec::new(),
            find_index: 0,
            toast_actions: Vec::new(),
            timer: None,
            undo: Vec::new(),
            redo: Vec::new(),
            palette_mode: PaletteMode::All,
            palette_entries: Vec::new(),
            link_preview: Vec::new(),
            recently_closed: Vec::new(),
            calendar_month: Local::now().date_naive(),
            shortcut_filter: String::new(),
            custom_theme: Default::default(),
            custom_theme_modified: None,
            slides: Vec::new(),
            slide: 0,
            restore_to: None,
            backed_up_on: None,
            save_timer: Timer::default(),
            pause_timer: Timer::default(),
            word_base: 0,
            summaries: Default::default(),
            last_copy: Default::default(),
            versions: Vec::new(),
            toast_timer: Timer::default(),
            save_failed: false,
            exchange: Default::default(),
            this: rc::Weak::new(),
            ui: ui.as_weak(),
        }));
        let mut this = app.borrow_mut();
        this.this = Rc::downgrade(&app);

        // Drop stale references, e.g. to notes deleted outside the app.
        let live: HashSet<NoteId> = this.notes.iter().filter(|n| n.is_live()).map(|n| n.id).collect();
        this.state.open_ids.retain(|id| live.contains(id));
        this.state.cursors.retain(|id, _| live.contains(id));
        if !this.state.current.is_some_and(|id| this.state.open_ids.contains(&id)) {
            this.state.current = this.state.open_ids.last().copied();
        }
        this.state.transparency = this.state.transparency.clamp(0.3, 1.0);

        let sections = &this.state.sections;
        ui.set_show_active(sections.active);
        ui.set_show_favorites(sections.favorites);
        ui.set_show_recent(sections.recent);
        ui.set_show_all(sections.all);
        ui.set_show_bin(sections.bin);
        ui.set_show_pinned(sections.pinned);
        ui.set_show_tags(sections.tags);
        ui.set_show_archive(sections.archive);
        ui.set_show_saved(sections.saved);
        ui.set_show_notebooks(sections.notebooks);
        ui.set_sidebar_open(!this.state.sidebar_collapsed);
        ui.set_sidebar_width(clamp_sidebar_width(this.state.sidebar_width));
        ui.set_split_ratio(clamp_split_ratio(this.state.split_ratio));
        ui.set_launch_at_login(system::launch_at_login());
        ui.set_software_rendering(this.state.software_rendering);
        ui.set_close_to_tray(this.state.close_to_tray);
        ui.set_note_order(this.state.note_order);
        ui.set_version_history(this.state.version_history);
        ui.set_tasks_show_done(this.state.tasks_show_done);
        if let Some(placement) = this.state.window {
            window::restore(ui, placement);
        }
        this.apply_appearance();

        if !import.is_empty() {
            this.import_paths(import);
        }
        if this.notes.is_empty() {
            this.new_note();
        } else {
            this.refresh_lists();
            this.load_editor();
        }
        this.auto_purge();
        this.restore_stickies();
        this.register_capture_hotkey();
        this.watch_for_external_changes();
        this.watch_custom_theme();
        this.refresh_sync_ui();
        this.refresh_reminders();
        // Anything that came due while the app was closed.
        this.fire_due_reminders(true);
        if rebuilt > 0 {
            let notes = if rebuilt == 1 { "1 note".to_string() } else { format!("{rebuilt} notes") };
            this.toast_with(
                &format!("Zima's list of notes was damaged, so it was rebuilt from your {notes}. Titles show as each note's first line; a copy of the old list is kept in Zima\u{2019}s data folder."),
                true,
                vec![("Open folder".to_string(), ToastAction::ShowDataFolder)],
            );
        }
        drop(this);
        app
    }

    fn find(&self, id: NoteId) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// `note`'s words, tags, checklist count and tasks, read again only if it changed.
    fn summary(&self, note: &Note) -> Rc<summary::Summary> {
        self.summaries.borrow_mut().get(note, Local::now())
    }

    fn find_mut(&mut self, id: NoteId) -> Option<&mut Note> {
        self.notes.iter_mut().find(|n| n.id == id)
    }

    fn current(&self) -> Option<&Note> {
        self.find(self.state.current?)
    }

    fn current_mut(&mut self) -> Option<&mut Note> {
        let id = self.state.current?;
        self.find_mut(id)
    }

    // ----- Notes -----

    pub fn new_note(&mut self) {
        // Millisecond timestamps, like Zima, so imported notes keep their ids.
        let id = self.next_note_id();
        self.notes.push(Note {
            id,
            title: String::new(),
            body: String::new(),
            modified: now_ms(),
            created: now_ms(),
            ..Default::default()
        });
        self.dirty.insert(id);
        self.index_dirty = true;
        self.flush();
        self.open(id);
        if let Some(ui) = self.ui.upgrade() {
            ui.invoke_focus_title();
        }
    }

    pub fn open(&mut self, id: NoteId) {
        if !self.find(id).is_some_and(Note::is_live) {
            return;
        }
        if !self.state.open_ids.contains(&id) {
            self.state.open_ids.push(id);
        }
        self.state.current = Some(id);
        // Opening a note with a plain click ends picking notes.
        self.selected.clear();
        self.save_state();
        self.refresh_lists();
        self.load_editor();
    }

    /// Remove from the Active section. Closing the current note switches to the last opened one.
    pub fn close(&mut self, id: NoteId) {
        if self.state.open_ids.contains(&id) {
            self.recently_closed.retain(|&c| c != id);
            self.recently_closed.push(id);
        }
        self.state.open_ids.retain(|&open| open != id);
        if self.state.current == Some(id) {
            self.state.current = self.state.open_ids.last().copied();
            self.load_editor();
        }
        self.save_state();
        self.refresh_lists();
    }

    /// Move to the Bin.
    pub fn delete(&mut self, id: NoteId) {
        let Some(note) = self.find_mut(id) else { return };
        note.deleted_at = Some(now_ms());
        self.index_dirty = true;
        self.flush();
        self.close(id);
        self.close_sticky(id);
    }

    pub fn restore(&mut self, id: NoteId) {
        let Some(note) = self.find_mut(id) else { return };
        note.deleted_at = None;
        self.index_dirty = true;
        self.flush();
        self.open(id);
    }

    /// Delete permanently.
    pub fn purge(&mut self, id: NoteId) {
        self.notes.retain(|n| n.id != id);
        self.dirty.remove(&id);
        self.selected.retain(|&s| s != id);
        self.state.cursors.remove(&id);
        self.close_sticky(id);
        self.forget_history(id);
        if let Err(e) = self.store.delete_body(id) {
            eprintln!("failed to delete note {id}: {e}");
        }
        self.index_dirty = true;
        self.flush();
        self.refresh_lists();
    }

    pub fn toggle_favorite(&mut self, id: NoteId) {
        let Some(note) = self.find_mut(id) else { return };
        note.favorite = !note.favorite;
        self.index_dirty = true;
        self.flush();
        self.refresh_lists();
    }

    /// Called on every keystroke in the title.
    pub fn set_title(&mut self, title: String) {
        // The title box wraps but holds one line: a pasted line break becomes a space.
        let title = match one_line(&title) {
            Some(fixed) => {
                if let Some(ui) = self.ui.upgrade() {
                    ui.set_note_title(fixed.as_str().into());
                }
                fixed
            }
            None => title,
        };
        let Some(note) = self.current_mut() else { return };
        note.title = title;
        note.modified = now_ms();
        let id = note.id;
        self.index_dirty = true;
        // Just this note's rows for now; the save after typing pauses rebuilds the lists (order, search).
        if let Some(ui) = self.ui.upgrade() {
            let title = self.find(id).map(display_title).unwrap_or_default();
            for rows in [ui.get_active_notes(), ui.get_pinned_notes(), ui.get_favorite_notes(), ui.get_recent_notes(), ui.get_all_notes(), ui.get_archived_notes()] {
                retitle_rows(&rows, id, &title);
            }
        }
        self.schedule_save();
        self.sync_sticky(id);
    }

    /// Called on every keystroke in the body. `cursor` is the cursor's byte offset in `body`.
    pub fn set_body(&mut self, body: String, cursor: usize) {
        let Some(note) = self.current_mut() else { return };
        let grew = body.len() > note.body.len();
        note.body = body;
        note.modified = now_ms();
        let id = note.id;
        self.dirty.insert(id);
        self.index_dirty = true;
        self.cursor = cursor;
        self.schedule_save();
        if grew && !self.check_line_command(id, cursor) {
            self.after_typing(cursor);
        }
        self.after_edit();
        self.schedule_pause();
        self.sync_sticky(id);
    }

    /// Small automatic edits after typing: `@today` expansion and smart lists.
    fn after_typing(&mut self, cursor: usize) {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let Some(last) = body.get(..cursor).and_then(|b| b.chars().next_back()) else { return };
        if last == ' ' || last == '\n' {
            if let Some((start, name)) = commands::date_token_before(&body, cursor - 1) {
                let value = commands::expand(name, Local::now()).unwrap_or_default();
                let mut new = body.clone();
                new.replace_range(start..cursor - 1, &value);
                let at = start + value.len() + 1;
                self.replace_body(new, Some((at, at)));
                return;
            }
        }
        if last == '\n' {
            if let Some(edit) = format::continue_list(&body, cursor) {
                self.replace_body(edit.text, Some((edit.anchor, edit.cursor)));
            }
        }
    }

    /// Replace the current note's body from Rust (formatting, commands) and update the editor.
    /// Recorded for Ctrl+Z.
    fn replace_body(&mut self, body: String, selection: Option<(usize, usize)>) {
        self.set_body_from_rust(body, selection, true);
    }

    fn set_body_from_rust(&mut self, body: String, selection: Option<(usize, usize)>, record: bool) {
        if self.state.current.is_some_and(|id| !self.can_change(id)) {
            return;
        }
        // Typing so far counts for the stats; this edit, made by Zima, doesn't.
        self.count_words();
        let cursor_before = self.cursor;
        let Some(note) = self.current_mut() else { return };
        let before = std::mem::replace(&mut note.body, body);
        note.modified = now_ms();
        let id = note.id;
        if record {
            let after = note.body.clone();
            self.undo.push(Snapshot { note: id, before, after, cursor: cursor_before });
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
            self.redo.clear();
        }
        self.dirty.insert(id);
        self.index_dirty = true;
        self.schedule_save();
        if let (Some(ui), Some(note)) = (self.ui.upgrade(), self.find(id)) {
            ui.set_note_body(note.body.as_str().into());
            if let Some((anchor, cursor)) = selection {
                ui.invoke_set_body_selection(anchor as i32, cursor as i32);
                self.cursor = cursor;
            }
        }
        self.after_body_change();
        self.sync_sticky(id);
    }

    /// Ctrl+Z: undo the last Rust-made edit if the text is still exactly as it left it.
    /// Returns false to let the text field handle ordinary typing undo.
    pub fn undo(&mut self) -> bool {
        let (Some(note), Some(top)) = (self.current(), self.undo.last()) else { return false };
        if top.note != note.id || top.after != note.body {
            return false;
        }
        let snapshot = self.undo.pop().unwrap();
        let cursor = snapshot.cursor.min(snapshot.before.len());
        self.set_body_from_rust(snapshot.before.clone(), Some((cursor, cursor)), false);
        self.redo.push(snapshot);
        true
    }

    /// Ctrl+Y: redo what Ctrl+Z undid.
    pub fn redo(&mut self) -> bool {
        let (Some(note), Some(top)) = (self.current(), self.redo.last()) else { return false };
        if top.note != note.id || top.before != note.body {
            return false;
        }
        let snapshot = self.redo.pop().unwrap();
        let end = snapshot.after.len();
        self.set_body_from_rust(snapshot.after.clone(), Some((end.min(snapshot.cursor + 1), end.min(snapshot.cursor + 1))), false);
        self.undo.push(snapshot);
        true
    }

    fn after_body_change(&mut self) {
        self.refresh_preview();
        self.after_edit();
        // Not typed (opened, synced, edited by Zima): words from here on count as written.
        let words = self.current().map_or(0, |n| n.body.split_whitespace().count());
        self.word_base = words;
        if let Some(ui) = self.ui.upgrade() {
            ui.set_note_words(words as i32);
        }
    }

    /// What has to keep up with every key: `@` suggestions and find matches.
    fn after_edit(&mut self) {
        self.update_suggestions();
        if !self.find_query.is_empty() {
            self.recompute_matches();
            self.update_find_ui();
        }
        if let (Some(ui), Some(note)) = (self.ui.upgrade(), self.current()) {
            ui.set_note_goal(note.goal.unwrap_or(0) as i32);
        }
    }

    // ----- Typing helpers -----

    fn selection(&self) -> (usize, usize) {
        match self.ui.upgrade() {
            Some(ui) => (ui.get_sel_anchor().max(0) as usize, ui.get_sel_cursor().max(0) as usize),
            None => (self.cursor, self.cursor),
        }
    }

    /// Tab in a table moves between cells; returns false when not in a table.
    pub fn table_tab(&mut self, back: bool) -> bool {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return false };
        let (_, cursor) = self.selection();
        match format::table_tab(&body, cursor, back) {
            Some(edit) => {
                self.replace_body(edit.text, Some((edit.anchor, edit.cursor)));
                true
            }
            None => false,
        }
    }

    /// Enter at the end of a table row adds a row; returns false otherwise.
    pub fn table_enter(&mut self) -> bool {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return false };
        let (anchor, cursor) = self.selection();
        if anchor != cursor {
            return false;
        }
        match format::table_enter(&body, cursor) {
            Some(edit) => {
                self.replace_body(edit.text, Some((edit.anchor, edit.cursor)));
                true
            }
            None => false,
        }
    }

    /// Ctrl + / Ctrl − / Ctrl 0.
    pub fn zoom(&mut self, step: i32) {
        self.state.zoom = step_scale(self.state.zoom, step, 0.7, 2.0);
        self.save_state();
        self.apply_appearance();
        self.toast(&format!("Text size {}%", (self.state.zoom * 100.0).round()), false);
    }

    /// Settings → Interface size: −, +, or 0 to reset.
    pub fn step_ui_scale(&mut self, step: i32) {
        self.state.ui_scale = step_scale(self.state.ui_scale, step, UI_SCALE_MIN, UI_SCALE_MAX);
        self.save_state();
        self.apply_ui_scale();
    }

    fn apply_ui_scale(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let scale = self.state.ui_scale.clamp(UI_SCALE_MIN, UI_SCALE_MAX);
        ui.global::<Theme>().set_ui_scale(scale);
        window::apply_ui_scale(&ui, scale);
    }

    pub fn set_line_spacing(&mut self, spacing: f32) {
        self.state.line_spacing = spacing.clamp(1.0, 2.0);
        self.save_state();
        self.apply_appearance();
    }

    pub fn set_spotlight(&mut self, on: bool) {
        self.state.spotlight = on;
        self.save_state();
    }

    /// Tab / Shift+Tab.
    pub fn indent(&mut self, outdent: bool) {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let (anchor, cursor) = self.selection();
        if let Some(edit) = format::indent(&body, anchor, cursor, outdent) {
            self.replace_body(edit.text, Some((edit.anchor, edit.cursor)));
        }
    }

    /// Brackets and backticks; returns true if handled (the key is then swallowed).
    pub fn auto_pair(&mut self, typed: &str) -> bool {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return false };
        let (anchor, cursor) = self.selection();
        match format::auto_pair(&body, anchor, cursor, typed) {
            Some(edit) if edit.text == body => {
                // Just stepping over a closer.
                if let Some(ui) = self.ui.upgrade() {
                    ui.invoke_set_body_selection(edit.anchor as i32, edit.cursor as i32);
                }
                self.cursor = edit.cursor;
                true
            }
            Some(edit) => {
                self.replace_body(edit.text, Some((edit.anchor, edit.cursor)));
                true
            }
            None => false,
        }
    }

    /// Ctrl+V with text selected and a URL on the clipboard makes a link; returns true if handled.
    pub fn paste_link(&mut self) -> bool {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return false };
        let (anchor, cursor) = self.selection();
        if anchor == cursor {
            return false;
        }
        let Ok(clipboard) = arboard::Clipboard::new().and_then(|mut c| c.get_text()) else { return false };
        match format::paste_link(&body, anchor, cursor, &clipboard) {
            Some(edit) => {
                self.replace_body(edit.text, Some((edit.anchor, edit.cursor)));
                true
            }
            None => false,
        }
    }

    // ----- Find & replace -----

    pub fn find_text(&mut self, query: String) {
        self.find_query = query;
        self.recompute_matches();
        // Start at the first match after the cursor.
        self.find_index = self.find_matches.iter().position(|(start, _)| *start >= self.cursor).unwrap_or(0);
        self.select_match();
        self.update_find_ui();
    }

    pub fn find_step(&mut self, delta: i32) {
        if self.find_matches.is_empty() {
            return;
        }
        let n = self.find_matches.len() as i32;
        self.find_index = ((self.find_index as i32 + delta).rem_euclid(n)) as usize;
        self.select_match();
        self.update_find_ui();
    }

    pub fn replace_one(&mut self, replacement: String) {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let Some(&(start, end)) = self.find_matches.get(self.find_index) else { return };
        let mut new = body;
        new.replace_range(start..end, &replacement);
        let at = start + replacement.len();
        self.replace_body(new, Some((at, at)));
        self.find_index = self.find_matches.iter().position(|(s, _)| *s >= at).unwrap_or(0);
        self.select_match();
        self.update_find_ui();
    }

    pub fn replace_all(&mut self, replacement: String) {
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let count = self.find_matches.len();
        if count == 0 {
            return;
        }
        let mut new = body;
        for &(start, end) in self.find_matches.iter().rev() {
            new.replace_range(start..end, &replacement);
        }
        self.replace_body(new, None);
        self.toast(&format!("Replaced {count} {}", if count == 1 { "match" } else { "matches" }), false);
    }

    pub fn close_find(&mut self) {
        self.find_query.clear();
        self.find_matches.clear();
        if let Some(ui) = self.ui.upgrade() {
            ui.invoke_focus_body();
        }
    }

    fn recompute_matches(&mut self) {
        self.find_matches.clear();
        let Some(note) = self.current() else { return };
        if self.find_query.is_empty() {
            return;
        }
        // Case-insensitive when lowercasing keeps byte offsets (true for nearly all text).
        let (haystack, needle) = (note.body.to_lowercase(), self.find_query.to_lowercase());
        let (haystack, needle) = if haystack.len() == note.body.len() && needle.len() == self.find_query.len() {
            (haystack, needle)
        } else {
            (note.body.clone(), self.find_query.clone())
        };
        let mut from = 0;
        while let Some(i) = haystack[from..].find(&needle) {
            let start = from + i;
            self.find_matches.push((start, start + needle.len()));
            from = start + needle.len().max(1);
        }
    }

    fn select_match(&self) {
        if let (Some(ui), Some(&(start, end))) = (self.ui.upgrade(), self.find_matches.get(self.find_index)) {
            ui.invoke_select_in_body(start as i32, end as i32);
        }
    }

    fn update_find_ui(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let status = match (self.find_query.is_empty(), self.find_matches.len()) {
            (true, _) => String::new(),
            (false, 0) => "No results".into(),
            (false, n) => format!("{} of {n}", self.find_index.min(n - 1) + 1),
        };
        ui.set_find_status(status.into());
    }

    // ----- Focus timer -----

    pub fn start_timer(&mut self, minutes: u32, is_break: bool) {
        self.timer = Some(FocusTimer { ends: now_ms() + minutes as i64 * 60_000, minutes, is_break });
        self.tick_timer();
        let label = if is_break { "Break" } else { "Focus" };
        self.toast(&format!("{label} timer: {minutes} min"), false);
    }

    pub fn stop_timer(&mut self) {
        self.timer = None;
        self.tick_timer();
    }

    /// Called every second.
    pub fn tick_timer(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some(timer) = &self.timer else {
            ui.set_timer_text("".into());
            return;
        };
        let left = (timer.ends - now_ms()).max(0) / 1000;
        ui.set_timer_text(format!("{}:{:02}", left / 60, left % 60).into());
        ui.set_timer_break(timer.is_break);
        if left == 0 {
            let (minutes, is_break) = (timer.minutes, timer.is_break);
            self.timer = None;
            ui.set_timer_text("".into());
            if is_break {
                show_notification("Break's over", "Ready for another focus session?");
                self.toast_with("Break's over", false, vec![("Focus 25 min".into(), ToastAction::StartTimer(25, false))]);
            } else {
                self.record_focus(minutes);
                show_notification("Focus session done", &format!("{minutes} minutes. Time for a break."));
                self.toast_with("Focus session done", false, vec![("5-min break".into(), ToastAction::StartTimer(5, true))]);
            }
        }
    }

    pub fn set_search(&mut self, query: String) {
        self.search = query;
        self.refresh_lists();
    }

    pub fn toggle_section(&mut self, name: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let s = &mut self.state.sections;
        match name {
            "active" => { s.active = !s.active; ui.set_show_active(s.active) }
            "favorites" => { s.favorites = !s.favorites; ui.set_show_favorites(s.favorites) }
            "recent" => { s.recent = !s.recent; ui.set_show_recent(s.recent) }
            "all" => { s.all = !s.all; ui.set_show_all(s.all) }
            "bin" => { s.bin = !s.bin; ui.set_show_bin(s.bin) }
            "pinned" => { s.pinned = !s.pinned; ui.set_show_pinned(s.pinned) }
            "tags" => { s.tags = !s.tags; ui.set_show_tags(s.tags) }
            "archive" => { s.archive = !s.archive; ui.set_show_archive(s.archive) }
            "saved" => { s.saved = !s.saved; ui.set_show_saved(s.saved) }
            "notebooks" => { s.notebooks = !s.notebooks; ui.set_show_notebooks(s.notebooks) }
            _ => return,
        }
        self.save_state();
    }

    /// The notes list's edge was dragged (or double-clicked back to the default).
    pub fn set_sidebar_width(&mut self, width: f32) {
        let width = clamp_sidebar_width(width);
        if width != self.state.sidebar_width {
            self.state.sidebar_width = width;
            self.save_state();
        }
    }

    /// The Split view's divider was dragged (or double-clicked back to half and half).
    pub fn set_split_ratio(&mut self, ratio: f32) {
        let ratio = clamp_split_ratio(ratio);
        if ratio != self.state.split_ratio {
            self.state.split_ratio = ratio;
            self.save_state();
        }
    }

    pub fn set_sidebar_open(&mut self, open: bool) {
        self.state.sidebar_collapsed = !open;
        self.save_state();
    }

    // ----- Formatting and commands -----

    pub fn format(&mut self, kind: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let Some(note) = self.current() else { return };
        let (anchor, cursor) = (ui.get_sel_anchor().max(0) as usize, ui.get_sel_cursor().max(0) as usize);
        if let Some(edit) = format::apply(kind, &note.body, anchor, cursor) {
            self.replace_body(edit.text, Some((edit.anchor, edit.cursor)));
        }
    }

    /// A checkbox clicked in the preview; repeating tasks (`@every`) move to their next date.
    pub fn toggle_task(&mut self, line: usize) {
        let Some(note) = self.current() else { return };
        if let Some(body) = crate::tasks::toggle(&note.body, line, Local::now()) {
            self.replace_body(body, None);
        }
    }

    pub fn cursor_moved(&mut self, cursor: usize) {
        self.cursor = cursor;
        self.update_suggestions();
    }

    /// `@` commands anywhere, or the `/` menu at the start of a line: (offset, rows).
    fn find_suggestions(&self) -> Option<(usize, Vec<Suggestion>)> {
        let note = self.current()?;
        if let Some((at, matches)) = commands::suggestions(&note.body, self.cursor) {
            let rows = matches.iter().map(|c| Suggestion { name: format!("@{}", c.name).into(), hint: c.hint.into() }).collect();
            return Some((at, rows));
        }
        let (at, matches) = commands::slash_suggestions(&note.body, self.cursor)?;
        let rows = matches.iter().map(|s| Suggestion { name: format!("/{}", s.name).into(), hint: s.hint.into() }).collect();
        Some((at, rows))
    }

    fn update_suggestions(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let found = self.find_suggestions().filter(|(at, _)| self.dismissed_at != Some(*at));
        match found {
            Some((at, rows)) => {
                if self.suggestion_at != Some(at) {
                    ui.set_suggestion_index(0);
                }
                self.suggestion_at = Some(at);
                ui.set_suggestion_index(ui.get_suggestion_index().clamp(0, rows.len() as i32 - 1));
                ui.set_suggestions(ModelRc::new(VecModel::from(rows)));
            }
            None => {
                if self.suggestion_at.take().is_some() {
                    ui.set_suggestions(ModelRc::default());
                }
            }
        }
    }

    pub fn accept_suggestion(&mut self, index: usize) {
        let Some(note) = self.current() else { return };
        // The `/` menu inserts a snippet.
        if let Some((at, matches)) = commands::slash_suggestions(&note.body, self.cursor) {
            let Some(item) = matches.get(index).or(matches.first()) else { return };
            let (insert, cursor_in) = match item.name {
                "date" => {
                    let date = commands::expand("today", Local::now()).unwrap_or_default();
                    let len = date.len();
                    (date, len)
                }
                _ => (item.insert.to_string(), item.cursor),
            };
            let mut body = note.body.clone();
            body.replace_range(at..self.cursor, &insert);
            let cursor = at + cursor_in;
            self.replace_body(body, Some((cursor, cursor)));
            return;
        }
        let Some((at, matches)) = commands::suggestions(&note.body, self.cursor) else { return };
        let Some(command) = matches.get(index).or(matches.first()) else { return };
        // Date commands insert their value; the rest insert "@name " ready for arguments.
        let insert = commands::expand(command.name, Local::now()).unwrap_or_else(|| format!("@{} ", command.name));
        let mut body = note.body.clone();
        body.replace_range(at..self.cursor, &insert);
        let cursor = at + insert.len();
        self.replace_body(body, Some((cursor, cursor)));
    }

    pub fn dismiss_suggestions(&mut self) {
        self.dismissed_at = self.suggestion_at;
        self.update_suggestions();
    }

    /// Pressing Enter after an `@` command line runs it. Returns true if the line was a command.
    fn check_line_command(&mut self, note_id: NoteId, cursor: usize) -> bool {
        let Some(note) = self.find(note_id) else { return false };
        let body = note.body.clone();
        if cursor == 0 || !body.is_char_boundary(cursor) || body.as_bytes()[cursor - 1] != b'\n' {
            return false;
        }
        let line_end = cursor - 1;
        let line_start = body[..line_end].rfind('\n').map_or(0, |i| i + 1);
        let line = body[line_start..line_end].to_string();
        let now = Local::now();
        // Remove the command line (and its newline), leaving the cursor where the line was.
        let remove_line = |app: &mut Self| {
            let mut new = body.clone();
            new.replace_range(line_start..cursor, "");
            app.replace_body(new, Some((line_start, line_start)));
        };

        if let Some(task) = crate::tasks::parse_due_line(&line, now) {
            if task.text.is_empty() {
                self.toast("Say what\u{2019}s due, like \u{201c}@due rent tomorrow\u{201d}.", true);
                return true;
            }
            self.add_quick_task(task.text, task.due);
            remove_line(self);
        } else if let Some(result) = reminders::parse_command(&line, now) {
            let Ok(parsed) = result else {
                self.toast("Couldn't tell when. Try \u{201c}at 5pm\u{201d} or \u{201c}in 20 min\u{201d}.", true);
                return true;
            };
            self.add_reminder(note_id, parsed.text, parsed.due.timestamp_millis(), parsed.repeat);
            remove_line(self);
        } else if let Some(result) = commands::parse_table(&line) {
            let Ok((rows, cols)) = result else {
                self.toast("Try \u{201c}@table 3,4\u{201d}: up to 20 rows and 10 columns.", true);
                return true;
            };
            let (mut table, mut first_cell) = commands::table_markdown(rows, cols);
            // A table directly under text would be read as part of that paragraph.
            let previous_line = body[..line_start].trim_end_matches('\n').rsplit('\n').next().unwrap_or("");
            if line_start > 0 && !previous_line.trim().is_empty() && !body[..line_start].ends_with("\n\n") {
                table.insert(0, '\n');
                first_cell = first_cell.start + 1..first_cell.end + 1;
            }
            let mut new = body.clone();
            new.replace_range(line_start..cursor, &table);
            // Select "Column 1" so typing replaces it.
            self.replace_body(new, Some((line_start + first_cell.start, line_start + first_cell.end)));
        } else if let Some(result) = commands::parse_time(&line, chrono::Utc::now()) {
            let Ok(replacement) = result else {
                self.toast("Couldn\u{2019}t tell those time zones. Try \u{201c}@time 3pm IST to PST\u{201d} or \u{201c}@time India to Estonia\u{201d}.", true);
                return true;
            };
            let mut new = body.clone();
            new.replace_range(line_start..line_end, &replacement);
            let at = line_start + replacement.len() + 1;
            self.replace_body(new, Some((at, at)));
        } else if let Some(request) = commands::currency_request(&line) {
            if let Some(replacement) = self.currency_line(note_id, &line, request) {
                let mut new = body.clone();
                new.replace_range(line_start..line_end, &replacement);
                let at = line_start + replacement.len() + 1;
                self.replace_body(new, Some((at, at)));
            }
        } else if let Some(result) = commands::parse_calc(&line) {
            let Ok(replacement) = result else {
                self.toast("Couldn't calculate that. Try \u{201c}@calc 12*3.5 + 8\u{201d}.", true);
                return true;
            };
            let mut new = body.clone();
            new.replace_range(line_start..line_end, &replacement);
            let at = line_start + replacement.len() + 1;
            self.replace_body(new, Some((at, at)));
        } else if let Some(result) = commands::parse_goal(&line) {
            let Ok(goal) = result else {
                self.toast("Try \u{201c}@goal 500\u{201d} or \u{201c}@goal off\u{201d}.", true);
                return true;
            };
            if let Some(note) = self.find_mut(note_id) {
                note.goal = goal;
            }
            self.index_dirty = true;
            remove_line(self);
            self.toast(
                &match goal {
                    Some(n) => format!("Word goal: {n}"),
                    None => "Word goal cleared".into(),
                },
                false,
            );
        } else if let Some(result) = commands::parse_timer(&line) {
            let Ok(minutes) = result else {
                self.toast("Try \u{201c}@timer 25\u{201d} (1\u{2013}240 minutes) or \u{201c}@timer stop\u{201d}.", true);
                return true;
            };
            remove_line(self);
            match minutes {
                Some(m) => self.start_timer(m, false),
                None => self.stop_timer(),
            }
        } else {
            // Not a command; maybe the line mentions a time ("call Sam tomorrow at 5"): offer a reminder.
            if let Some(parsed) = reminders::spot(&line, now) {
                let label = format!("Remind me {}", reminders::format_due(parsed.due, now));
                let action = ToastAction::Remind(parsed.text.clone(), parsed.due.timestamp_millis());
                self.toast_with(&format!("\u{201c}{}\u{201d}", parsed.text), false, vec![(label, action)]);
            }
            return false;
        }
        true
    }

    fn add_reminder(&mut self, note_id: NoteId, text: String, due: i64, repeat: Option<reminders::Repeat>) {
        let id = crate::model::next_id(self.reminders.iter().map(|r| r.id), now_ms());
        let now = Local::now();
        let due_text = Local.timestamp_millis_opt(due).single().map(|d| reminders::format_due(d, now)).unwrap_or_default();
        let repeat_text = repeat.as_ref().map(|r| format!(" \u{00b7} {}", r.describe())).unwrap_or_default();
        self.toast(&format!("Reminder set: {text} \u{2014} {due_text}{repeat_text}"), false);
        self.reminders.push(Reminder { id, note_id, text, due, repeat });
        self.save_reminders();
        self.refresh_reminders();
    }

    // ----- Appearance -----

    fn apply_appearance(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let theme = ui.global::<Theme>();
        // 6 = Auto: Light from 7am to 7pm, Dark otherwise.
        self.apply_custom_theme();
        let choice = match self.state.theme.clamp(0, LAST_THEME) {
            6 => {
                let hour = chrono::Timelike::hour(&Local::now());
                if (7..19).contains(&hour) { 1 } else { 2 }
            }
            CUSTOM => {
                if self.custom_theme.dark { 2 } else { 1 }
            }
            other => other,
        };
        theme.set_choice(choice);
        theme.set_selected_theme(self.state.theme.clamp(0, LAST_THEME));
        match self.state.accent.as_deref().and_then(parse_hex_color) {
            Some(color) => {
                theme.set_custom_accent(color);
                theme.set_has_custom_accent(true);
            }
            None => theme.set_has_custom_accent(false),
        }
        theme.set_surface_alpha(self.state.transparency);
        theme.set_font(self.state.font.as_str().into());
        theme.set_text_scale(self.state.zoom.clamp(0.7, 2.0));
        theme.set_ui_scale(self.state.ui_scale.clamp(UI_SCALE_MIN, UI_SCALE_MAX));
        theme.set_line_spacing(self.state.line_spacing.clamp(1.0, 2.0));
        ui.set_spotlight(self.state.spotlight);
        ui.set_view_mode(self.state.view_mode.clamp(0, 2));
        ui.set_wide(self.state.wide);
        ui.set_format_bar(self.state.format_bar);
        ui.set_typewriter(self.state.typewriter);
        window::apply_mica(&ui, theme.get_dark());
        self.apply_note_overrides();
        self.sync_theme_to_stickies();
        // Code colours depend on light/dark.
        self.refresh_preview();
    }

    /// Re-apply the Auto theme as the day goes on.
    pub fn tick_auto_theme(&mut self) {
        if self.state.theme == 6 {
            self.apply_appearance();
        }
    }

    /// `None` resets to the theme's own accent.
    pub fn set_accent(&mut self, accent: Option<String>) {
        self.state.accent = accent;
        self.save_state();
        self.apply_appearance();
    }

    pub fn set_theme(&mut self, theme: i32) {
        self.state.theme = theme.clamp(0, LAST_THEME);
        self.save_state();
        self.apply_appearance();
    }

    pub fn set_font(&mut self, font: String) {
        self.state.font = font;
        self.save_state();
        self.apply_appearance();
    }

    /// Called on every slider step, so only the opacity changes: a full `apply_appearance`
    /// would re-apply Mica (the backdrop flickers) and re-render the preview each step.
    pub fn set_transparency(&mut self, value: f32) {
        self.state.transparency = value.clamp(0.3, 1.0);
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Theme>().set_surface_alpha(self.state.transparency);
        }
        self.schedule_save(); // saves state too; avoids a write per slider step
    }

    pub fn set_wide(&mut self, wide: bool) {
        self.state.wide = wide;
        self.save_state();
    }

    pub fn set_view_mode(&mut self, mode: i32) {
        self.state.view_mode = mode.clamp(0, 2);
        self.save_state();
        self.refresh_preview();
        self.refresh_backlinks();
    }

    pub fn set_typewriter(&mut self, on: bool) {
        self.state.typewriter = on;
        self.save_state();
    }

    pub fn set_format_bar(&mut self, shown: bool) {
        self.state.format_bar = shown;
        self.save_state();
    }

    /// Takes effect at the next start, so offer to restart.
    pub fn set_software_rendering(&mut self, on: bool) {
        self.state.software_rendering = on;
        self.save_state();
        let message = if on { "Low-memory rendering is on after a restart" } else { "GPU rendering is back after a restart" };
        self.toast_with(message, false, vec![("Restart now".into(), ToastAction::Restart)]);
    }

    pub fn set_close_to_tray(&mut self, on: bool) {
        self.state.close_to_tray = on;
        self.save_state();
    }

    pub fn set_launch_at_login(&mut self, enabled: bool) {
        if let Err(e) = system::set_launch_at_login(enabled) {
            self.toast(&format!("Couldn't change launch at login: {e}"), true);
        }
        if let Some(ui) = self.ui.upgrade() {
            ui.set_launch_at_login(system::launch_at_login());
        }
    }

    // ----- Files -----

    pub fn export_note(&mut self) {
        let Some(note) = self.current() else { return };
        let title = display_title(note);
        let Some(path) = system::pick_export_path(&title) else { return };
        let text = if note.title.trim().is_empty() {
            note.body.clone()
        } else {
            format!("# {}\n\n{}", note.title.trim(), note.body)
        };
        match std::fs::write(&path, text) {
            Ok(()) => self.toast(&format!("Exported to {}", path.display()), false),
            Err(e) => self.toast(&format!("Export failed: {e}"), true),
        }
    }

    pub fn import_zima(&mut self) {
        let paths = system::pick_import_files();
        if !paths.is_empty() {
            self.import_paths(&paths);
        }
    }

    pub fn import_paths(&mut self, paths: &[PathBuf]) {
        match import::import(paths, &self.notes) {
            Ok(imported) => {
                let count = imported.len();
                for note in imported {
                    self.dirty.insert(note.id);
                    self.notes.push(note);
                }
                self.index_dirty = true;
                self.flush();
                self.refresh_lists();
                let message = match count {
                    0 => "No new notes to import".to_string(),
                    1 => "Imported 1 note".to_string(),
                    n => format!("Imported {n} notes"),
                };
                self.toast(&message, false);
            }
            Err(e) => self.toast(&format!("Import failed: {e}"), true),
        }
    }

    pub fn open_notes_folder(&self) {
        system::reveal(&self.store.notes_dir());
    }

    // ----- Reminders -----

    pub fn delete_reminder(&mut self, id: u64) {
        self.reminders.retain(|r| r.id != id);
        self.save_reminders();
        self.refresh_reminders();
    }

    /// Notify about and remove every reminder that is due. `missed` = it came due while the app was closed.
    pub fn fire_due_reminders(&mut self, missed: bool) {
        let now = now_ms();
        let (due, pending): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.reminders).into_iter().partition(|r| r.due <= now);
        self.reminders = pending;
        if due.is_empty() {
            return;
        }
        let title = if missed { "Missed reminder" } else { "Reminder" };
        for reminder in due {
            show_notification(title, &reminder.text);
            let actions = vec![
                ("10 min".to_string(), ToastAction::Snooze(reminder.clone(), Some(10))),
                ("1 hour".to_string(), ToastAction::Snooze(reminder.clone(), Some(60))),
                ("Tomorrow".to_string(), ToastAction::Snooze(reminder.clone(), None)),
                ("Open note".to_string(), ToastAction::OpenNote(reminder.note_id)),
            ];
            self.toast_with(&format!("{title}: {}", reminder.text), false, actions);
            // Repeating reminders come back at their next time.
            if let Some(next) = reminder.repeat.as_ref().and_then(|r| r.next_after(Local::now())) {
                self.reminders.push(Reminder { due: next.timestamp_millis(), ..reminder });
            }
        }
        self.save_reminders();
        self.refresh_reminders();
    }

    /// Clicking a reminder opens the note it was written in.
    pub fn open_reminder(&mut self, id: u64) {
        if let Some(note) = self.reminders.iter().find(|r| r.id == id).map(|r| r.note_id) {
            self.open(note);
        }
    }

    fn save_reminders(&mut self) {
        if let Err(e) = self.store.save_reminders(&self.reminders) {
            eprintln!("failed to save reminders: {e}");
        }
        self.last_reminders_write = self.store.reminders_modified();
    }

    fn refresh_reminders(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let now = Local::now();
        let mut sorted: Vec<&Reminder> = self.reminders.iter().collect();
        sorted.sort_by_key(|r| r.due);
        let rows: Vec<ReminderRow> = sorted
            .into_iter()
            .map(|r| ReminderRow {
                id: r.id.to_string().into(),
                text: r.text.as_str().into(),
                due: {
                    let due = Local.timestamp_millis_opt(r.due).single().map(|d| reminders::format_due(d, now)).unwrap_or_default();
                    match &r.repeat {
                        Some(repeat) => format!("{due}  \u{00b7}  {}", repeat.describe()),
                        None => due,
                    }
                }
                .into(),
            })
            .collect();
        ui.set_reminders(ModelRc::new(VecModel::from(rows)));
    }

    // ----- Window and tray -----

    /// The window's close button or Alt+F4: hide to the tray, or quit if that setting is off.
    /// The first time it hides, say so.
    pub fn window_closed(&mut self) {
        self.remember_window();
        self.remember_cursor();
        self.flush();
        if !self.state.close_to_tray {
            let _ = slint::quit_event_loop();
            return;
        }
        if !self.state.tray_hint_shown {
            self.state.tray_hint_shown = true;
            self.save_state();
            show_notification("Zima is still running", "Reminders will still fire. Quit from the tray icon.");
        }
    }

    /// Save where the main window is, so it opens there next time. Not while presenting, when it
    /// covers the screen (the spot to go back to is saved once presenting ends).
    pub fn remember_window(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        if self.restore_to.is_some() {
            return;
        }
        let placement = window::placement(&ui, self.state.window);
        if placement != self.state.window {
            self.state.window = placement;
            self.save_state();
        }
    }

    fn toast(&mut self, message: &str, error: bool) {
        self.toast_with(message, error, Vec::new());
    }

    /// Show a message in the top-right; with buttons it stays up longer.
    fn toast_with(&mut self, message: &str, error: bool, actions: Vec<(String, ToastAction)>) {
        let Some(ui) = self.ui.upgrade() else { return };
        ui.set_toast_text(message.into());
        ui.set_toast_error(error);
        let labels: Vec<slint::SharedString> = actions.iter().map(|(label, _)| label.as_str().into()).collect();
        ui.set_toast_actions(ModelRc::new(VecModel::from(labels)));
        ui.set_toast_visible(true);
        let duration = if actions.is_empty() { TOAST_DURATION } else { TOAST_DURATION * 3 };
        self.toast_actions = actions.into_iter().map(|(_, action)| action).collect();
        let ui = self.ui.clone();
        self.toast_timer.start(TimerMode::SingleShot, duration, move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_toast_visible(false);
            }
        });
    }

    /// A toast button was clicked.
    pub fn toast_action(&mut self, index: usize) {
        let Some(action) = self.toast_actions.get(index).cloned() else { return };
        self.toast_actions.clear();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_toast_visible(false);
        }
        match action {
            ToastAction::Snooze(reminder, minutes) => {
                let now = Local::now();
                let due = match minutes {
                    Some(m) => now + chrono::Duration::minutes(m),
                    None => {
                        let tomorrow = now.date_naive() + chrono::Duration::days(1);
                        let nine = chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap();
                        Local.from_local_datetime(&tomorrow.and_time(nine)).earliest().unwrap_or(now)
                    }
                };
                // Its own id: a repeating reminder is already back in the list under the old one.
                let id = crate::model::next_id(self.reminders.iter().map(|r| r.id), now_ms());
                self.reminders.push(Reminder { id, due: due.timestamp_millis(), repeat: None, ..reminder });
                self.save_reminders();
                self.refresh_reminders();
                self.toast(&format!("Snoozed until {}", reminders::format_due(due, now)), false);
            }
            ToastAction::StartTimer(minutes, is_break) => self.start_timer(minutes, is_break),
            ToastAction::Remind(text, due) => {
                let note = self.state.current.unwrap_or(0);
                self.add_reminder(note, text, due, None);
            }
            ToastAction::OpenNote(id) => {
                self.open(id);
                if let Some(ui) = self.ui.upgrade() {
                    let _ = ui.show();
                }
            }
            ToastAction::Restart => self.restart(),
            ToastAction::ShowBackups => self.open_backups_folder(),
            ToastAction::ShowDataFolder => system::reveal(self.store.root()),
        }
    }

    // ----- Saving -----

    /// Save shortly after the last change; every call restarts the timer.
    fn schedule_save(&self) {
        self.schedule_save_in(SAVE_DELAY);
    }

    fn schedule_save_in(&self, delay: Duration) {
        let this = self.this.clone();
        self.save_timer.start(TimerMode::SingleShot, delay, move || {
            if let Some(app) = this.upgrade() {
                let mut app = app.borrow_mut();
                app.flush();
                app.save_state();
                app.refresh_lists();
                // A title or [[link]] edit can change which notes link here.
                app.refresh_backlinks();
            }
        });
    }

    /// Write pending changes to disk. Anything that fails stays pending and is tried again.
    pub fn flush(&mut self) {
        self.save_stats();
        let mut error = None;
        for id in std::mem::take(&mut self.dirty) {
            self.keep_copy(id);
            if let Some(Err(e)) = self.find(id).map(|note| self.store.save_body(note)) {
                eprintln!("failed to save note {id}: {e}");
                self.dirty.insert(id);
                error = Some(e);
            }
        }
        if self.index_dirty {
            match self.store.save_index(&self.notes) {
                Ok(()) => {
                    self.index_dirty = false;
                    self.last_index_write = self.store.index_modified();
                }
                Err(e) => {
                    eprintln!("failed to save index: {e}");
                    error = Some(e);
                }
            }
        }
        self.after_save(error);
    }

    /// Say so when saving starts failing (once, not on every retry) and when it works again.
    fn after_save(&mut self, error: Option<std::io::Error>) {
        match error {
            Some(e) => {
                if !self.save_failed {
                    self.save_failed = true;
                    self.toast(&format!("Couldn't save your notes ({e}). Zima keeps trying; don't quit until it says they're saved."), true);
                }
                self.schedule_save_in(SAVE_RETRY);
            }
            None if self.save_failed => {
                self.save_failed = false;
                self.toast("Your notes are saved again.", false);
            }
            None => {}
        }
    }

    /// The open note's file couldn't be read before: try again (it may have finished syncing).
    fn read_again_if_unreadable(&mut self) {
        let Some(id) = self.state.current else { return };
        if !self.find(id).is_some_and(|n| n.unreadable) {
            return;
        }
        if let (Some(body), Some(note)) = (self.store.read_body(id), self.find_mut(id)) {
            note.body = body;
            note.unreadable = false;
        }
    }

    /// Whether note `id` can be changed; if its file couldn't be read, say so and refuse.
    fn can_change(&mut self, id: NoteId) -> bool {
        if !self.find(id).is_some_and(|n| n.unreadable) {
            return true;
        }
        self.toast("This note\u{2019}s file couldn\u{2019}t be read, so Zima won\u{2019}t change it. Close any app using it or let it finish syncing, then open the note again.", true);
        false
    }

    fn save_state(&self) {
        if let Err(e) = self.store.save_state(&self.state) {
            eprintln!("failed to save state: {e}");
        }
    }

    // ----- UI sync -----

    /// Push the current note into the editor. Only called when switching notes, so typing never resets the cursor.
    fn load_editor(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        // Words just typed in the note being left still count.
        self.count_words();
        self.note_cursor();
        self.read_again_if_unreadable();
        match self.current() {
            Some(note) => {
                ui.set_has_note(true);
                ui.set_note_unreadable(note.unreadable);
                ui.set_current_id(note.id.to_string().into());
                ui.set_note_title(note.title.as_str().into());
                ui.set_note_body(note.body.as_str().into());
            }
            None => {
                ui.set_has_note(false);
                ui.set_note_unreadable(false);
                ui.set_current_id("".into());
                ui.set_note_title("".into());
                ui.set_note_body("".into());
            }
        }
        self.shown = self.state.current;
        self.cursor = self.current().map_or(0, |n| saved_cursor(&n.body, self.state.cursors.get(&n.id).copied()));
        self.dismissed_at = None;
        self.undo.clear();
        self.redo.clear();
        self.after_body_change();
        self.refresh_backlinks();
        self.apply_note_overrides();
        ui.invoke_focus_default();
        // Back to where the cursor was. After this turn, once the editor has scrolled the new note to the top.
        if self.cursor > 0 && self.state.view_mode != 2 {
            let (id, at) = (self.shown, self.cursor);
            let this = self.this.clone();
            Timer::single_shot(Duration::ZERO, move || {
                let Some(app) = this.upgrade() else { return };
                with_app(&app, move |a| {
                    if let (Some(ui), true) = (a.ui.upgrade(), a.shown == id) {
                        ui.invoke_set_body_selection(at as i32, at as i32);
                    }
                });
            });
        }
    }

    /// Note where the cursor is in the note on screen, for when it's opened again.
    fn note_cursor(&mut self) {
        let Some(id) = self.shown else { return };
        if self.cursor == 0 {
            self.state.cursors.remove(&id);
        } else {
            self.state.cursors.insert(id, self.cursor);
        }
    }

    /// Save where the cursor is, if it moved since the last save (quitting, hiding to the tray).
    pub fn remember_cursor(&mut self) {
        let before = self.state.cursors.clone();
        self.note_cursor();
        if self.state.cursors != before {
            self.save_state();
        }
    }

    /// Re-render the Markdown preview (only when it's visible).
    fn refresh_preview(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let blocks: Vec<MdBlock> = match self.current() {
            Some(note) if self.state.view_mode != 0 => {
                let ink = Ink::of(&ui);
                markdown::parse(&note.body).into_iter().map(|b| md_block(b, &ink)).collect()
            }
            _ => Vec::new(),
        };
        ui.set_blocks(ModelRc::new(VecModel::from(blocks)));
    }

    /// Redraw the preview and count words once typing pauses.
    fn schedule_pause(&self) {
        let this = self.this.clone();
        self.pause_timer.start(TimerMode::SingleShot, PAUSE_DELAY, move || {
            if let Some(app) = this.upgrade() {
                with_app(&app, |a| {
                    if a.state.view_mode != 0 {
                        a.refresh_preview();
                    }
                    a.count_words();
                });
            }
        });
    }

    /// Show the open note's word count, and add any words typed since the last count to today's stats.
    fn count_words(&mut self) {
        let Some(words) = self.shown.and_then(|id| self.find(id)).map(|n| n.body.split_whitespace().count()) else { return };
        self.record_words(self.word_base, words);
        self.word_base = words;
        if let Some(ui) = self.ui.upgrade() {
            ui.set_note_words(words as i32);
        }
    }

    /// Rebuild every sidebar section.
    pub fn refresh_lists(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        // Words plus filters: #tag, notebook:, created:, modified:, has:, is: (see search.rs).
        let search = crate::search::parse(&self.search, Local::now().date_naive());
        let query = search.text();
        let matches = |n: &Note| search.matches(n);
        // The matching line, when the match is in the body rather than the title.
        let first_word = search.words.first().cloned().unwrap_or_default();
        let snippet = |n: &Note| -> String {
            if first_word.is_empty() || n.title.to_lowercase().contains(&query) {
                return String::new();
            }
            n.body
                .lines()
                .find(|l| l.to_lowercase().contains(&first_word))
                .map(|l| l.trim().chars().take(80).collect())
                .unwrap_or_default()
        };
        // Sorted by creation date, the times shown are creation times too.
        let by_created = self.state.note_order == 2;
        let row = |n: &Note| NoteRow {
            id: n.id.to_string().into(),
            title: display_title(n).into(),
            meta: relative_time(if by_created && n.created > 0 { n.created } else { n.modified }).into(),
            favorite: n.favorite,
            current: self.state.current == Some(n.id),
            dim: !matches(n),
            pinned: n.pinned,
            color: n.color.map_or(-1, |c| c as i32),
            emoji: n.emoji.as_deref().and_then(crate::emoji::clean).unwrap_or_default().into(),
            progress: self.summary(n).progress.as_str().into(),
            snippet: snippet(n).into(),
            selected: self.selected.contains(&n.id),
        };

        let mut listed: Vec<&Note> = self.notes.iter().filter(|n| n.is_listed()).collect();
        listed.sort_by_key(|n| Reverse(n.modified));
        let recent: Vec<&Note> = listed.iter().copied().filter(|n| matches(n)).take(RECENT_COUNT).collect();
        organize::sort_notes(&mut listed, self.state.note_order);
        let mut binned: Vec<&Note> = self.notes.iter().filter(|n| !n.is_live()).collect();
        binned.sort_by_key(|n| Reverse(n.deleted_at));
        let mut archived: Vec<&Note> = self.notes.iter().filter(|n| n.is_live() && n.archived).collect();
        organize::sort_notes(&mut archived, self.state.note_order);

        // Active keeps every open note and fades non-matches (like Zima); other sections filter.
        let active = self.state.open_ids.iter().filter_map(|&id| self.find(id)).map(row);
        let pinned = listed.iter().copied().filter(|n| n.pinned && matches(n)).map(row);
        let favorites = listed.iter().copied().filter(|n| n.favorite && matches(n)).map(row);
        let recent = recent.into_iter().map(row);
        let all = listed.iter().copied().filter(|n| matches(n)).map(row);
        let bin = binned.iter().copied().filter(|n| matches(n)).map(row);
        let archive = archived.iter().copied().filter(|n| matches(n)).map(row);

        ui.set_active_notes(model(active));
        ui.set_pinned_notes(model(pinned));
        ui.set_favorite_notes(model(favorites));
        ui.set_recent_notes(model(recent));
        ui.set_all_notes(model(all));
        ui.set_bin_notes(model(bin));
        ui.set_archived_notes(model(archive));
        ui.set_note_count(listed.len() as i32);
        ui.set_selected_count(self.selected.len() as i32);
        ui.set_word_count(listed.iter().map(|n| self.summary(n).words).sum::<usize>() as i32);
        let ids: HashSet<NoteId> = self.notes.iter().map(|n| n.id).collect();
        self.summaries.borrow_mut().retain(|id| ids.contains(&id));
        self.refresh_tasks();
        self.refresh_finding();
        self.refresh_tags();
    }
}

/// A saved cursor spot that still fits the note: within the text and not inside a character.
/// The note may have been edited elsewhere since (synced folder).
fn saved_cursor(body: &str, saved: Option<usize>) -> usize {
    let mut at = saved.unwrap_or(0).min(body.len());
    while !body.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Give note `id`'s rows in a sidebar list a new title, leaving the other rows alone.
fn retitle_rows(rows: &ModelRc<NoteRow>, id: NoteId, title: &str) {
    let id = id.to_string();
    for i in 0..rows.row_count() {
        if let Some(mut row) = rows.row_data(i).filter(|r| r.id == id.as_str() && r.title != title) {
            row.title = title.into();
            rows.set_row_data(i, row);
        }
    }
}

fn model(rows: impl Iterator<Item = NoteRow>) -> ModelRc<NoteRow> {
    ModelRc::new(VecModel::from(rows.collect::<Vec<_>>()))
}

/// Colours the preview needs from the theme.
pub struct Ink {
    dark: bool,
    /// For `@words`: the accent, as "#rrggbb".
    keyword: String,
}

impl Ink {
    pub fn of(ui: &AppWindow) -> Self {
        let theme = ui.global::<Theme>();
        let accent = theme.get_accent();
        Self { dark: theme.get_dark(), keyword: format!("#{:02x}{:02x}{:02x}", accent.red(), accent.green(), accent.blue()) }
    }
}

fn md_block(block: markdown::Block, ink: &Ink) -> MdBlock {
    let dark = ink.dark;
    let mark = if dark { HIGHLIGHT_DARK } else { HIGHLIGHT_LIGHT };
    // A highlight that crosses bold or italic can't be drawn; show its `==` as typed instead.
    let styled = |text: &str| {
        StyledText::from_markdown(&markdown::keywords(&markdown::highlights(text, mark, dark), &ink.keyword))
            .or_else(|_| StyledText::from_markdown(&markdown::plain_marks(text)))
            .unwrap_or_else(|_| StyledText::from_plain_text(&markdown::plain_marks(text)))
    };
    let cells: Vec<StyledText> = if block.kind == Kind::Code {
        // Code blocks: one highlighted line per cell.
        crate::highlight::highlight(&block.text, &block.marker, dark).iter().map(|l| styled(l)).collect()
    } else {
        block
            .cells
            .iter()
            // Header cells are bold.
            .map(|cell| if block.level == 1 && !cell.is_empty() { styled(&format!("**{cell}**")) } else { styled(cell) })
            .collect()
    };
    // Notes this block links to (code never links), for the hover card.
    let links: Vec<slint::SharedString> = if block.kind == Kind::Code {
        Vec::new()
    } else {
        let mut titles = markdown::note_links(&block.text);
        for cell in &block.cells {
            for title in markdown::note_links(cell) {
                if !titles.iter().any(|t| t.eq_ignore_ascii_case(&title)) {
                    titles.push(title);
                }
            }
        }
        titles.into_iter().map(Into::into).collect()
    };
    MdBlock {
        kind: block.kind.name().into(),
        text: if block.kind == Kind::Code { StyledText::default() } else { styled(&block.text) },
        plain: if block.kind == Kind::Code { block.text.as_str().into() } else { Default::default() },
        level: block.level as i32,
        marker: block.marker.as_str().into(),
        indent: block.indent as i32,
        checked: block.checked,
        cells: ModelRc::new(VecModel::from(cells)),
        line: block.line as i32,
        space: block.space as i32,
        note_links: ModelRc::new(VecModel::from(links)),
    }
}

/// The title, or the first line of the body, or "Untitled".
/// For other modules (export).
pub fn display_title_of(note: &Note) -> String {
    display_title(note)
}

fn display_title(note: &Note) -> String {
    let title = note.title.trim();
    if !title.is_empty() {
        return title.to_string();
    }
    // First non-empty line, without Markdown symbols ("# ", "- [ ] ", "**", …).
    let plain = |line: &str| -> String { markdown::strip_markers(line).chars().take(60).collect::<String>().trim().to_string() };
    note.body
        .lines()
        .map(plain)
        .find(|line| !line.is_empty())
        .unwrap_or_else(|| "Untitled".to_string())
}

fn relative_time(ms: i64) -> String {
    let Some(then) = Local.timestamp_millis_opt(ms).single() else { return String::new() };
    let now = Local::now();
    let secs = (now - then).num_seconds();
    if secs < 60 {
        "now".into()
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if then.date_naive() == now.date_naive() {
        then.format("%-I:%M %p").to_string()
    } else if now.date_naive().pred_opt() == Some(then.date_naive()) {
        "Yesterday".into()
    } else if then.format("%Y").to_string() == now.format("%Y").to_string() {
        then.format("%b %-d").to_string()
    } else {
        then.format("%b %-d, %Y").to_string()
    }
}

/// Windows toast notification. Runs on a thread so the UI never waits on it.
fn show_notification(title: &str, body: &str) {
    let (title, body) = (title.to_string(), body.to_string());
    std::thread::spawn(move || {
        if let Err(e) = crate::system::notification().summary(&title).body(&body).show() {
            eprintln!("failed to show notification: {e}");
        }
    });
}

/// `text` with its line breaks turned into spaces, or `None` if it has none.
fn one_line(text: &str) -> Option<String> {
    text.contains(['\n', '\r']).then(|| text.replace("\r\n", " ").replace(['\r', '\n'], " "))
}

/// The notes list's width, kept between 180 and 480 px (the same limits as dragging in `ui/app.slint`).
fn clamp_sidebar_width(width: f32) -> f32 {
    if width.is_finite() { width.clamp(180.0, 480.0) } else { 260.0 }
}

/// The text's share in Split view, kept between 25% and 75% (as in `ui/editor.slint`).
fn clamp_split_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() { ratio.clamp(0.25, 0.75) } else { 0.5 }
}

/// One step of a size setting (text size, interface size): ±10% per step, 0 resets to 100%.
/// Rounded to whole tens so repeated steps don't drift (0.1 isn't exact in floating point).
fn step_scale(current: f32, step: i32, min: f32, max: f32) -> f32 {
    match step {
        0 => 1.0,
        s => ((current + s as f32 * 0.1) * 10.0).round() / 10.0,
    }
    .clamp(min, max)
}

/// "#rrggbb" → colour.
/// "#rrggbb" or "#rrggbbaa".
fn parse_hex_color(hex: &str) -> Option<slint::Color> {
    let hex = hex.trim().strip_prefix('#')?;
    let value = u32::from_str_radix(hex, 16).ok()?;
    match hex.len() {
        6 => Some(slint::Color::from_rgb_u8((value >> 16) as u8, (value >> 8) as u8, value as u8)),
        8 => Some(slint::Color::from_argb_u8(value as u8, (value >> 24) as u8, (value >> 16) as u8, (value >> 8) as u8)),
        _ => None,
    }
}

/// The toolbar's colour dots: the plain highlight, then the named colours Preview knows.
fn colour_dots() -> Vec<ColourDot> {
    let dot = |name: &str, light: &str, dark: &str| ColourDot {
        title: if name.is_empty() { "Highlight".into() } else { format!("{}{}", name[..1].to_uppercase(), &name[1..]).into() },
        name: name.into(),
        light: parse_hex_color(light).unwrap_or_default(),
        dark: parse_hex_color(dark).unwrap_or_default(),
    };
    std::iter::once(dot("", HIGHLIGHT_LIGHT, HIGHLIGHT_DARK))
        .chain(markdown::TEXT_COLOURS.iter().map(|c| dot(c.name, c.light, c.dark)))
        .collect()
}

fn parse_id(id: &str) -> Option<NoteId> {
    id.parse().ok()
}

/// Connect UI callbacks to the app.
pub fn wire(app: &Rc<RefCell<App>>, ui: &AppWindow) {
    macro_rules! on_id {
        ($setter:ident, $method:ident) => {{
            let app = app.clone();
            ui.$setter(move |id| {
                if let Some(id) = parse_id(&id) {
                    with_app(&app, move |app| app.$method(id));
                }
            });
        }};
    }
    on_id!(on_open_note, open);
    on_id!(on_close_note, close);
    on_id!(on_delete_note, delete);
    on_id!(on_restore_note, restore);
    on_id!(on_purge_note, purge);
    on_id!(on_toggle_favorite, toggle_favorite);
    on_id!(on_delete_reminder, delete_reminder);

    macro_rules! on {
        ($setter:ident, |$app:ident $(, $arg:ident)*| $body:expr) => {{
            let app = app.clone();
            ui.$setter(move |$($arg),*| {
                with_app(&app, move |$app| $body);
            });
        }};
    }
    on!(on_new_note, |a| a.new_note());
    on!(on_title_edited, |a, title| a.set_title(title.into()));
    on!(on_body_edited, |a, body, cursor| a.set_body(body.into(), cursor.max(0) as usize));
    on!(on_cursor_moved, |a, cursor| a.cursor_moved(cursor.max(0) as usize));
    on!(on_accept_suggestion, |a, index| a.accept_suggestion(index.max(0) as usize));
    on!(on_dismiss_suggestions, |a| a.dismiss_suggestions());
    on!(on_format, |a, kind| a.format(&kind));
    ui.set_text_colours(ModelRc::new(VecModel::from(colour_dots())));
    on!(on_toggle_task, |a, line| a.toggle_task(line.max(0) as usize));
    on!(on_search_edited, |a, query| a.set_search(query.into()));
    on!(on_toggle_section, |a, name| a.toggle_section(&name));
    on!(on_sidebar_toggled, |a, open| a.set_sidebar_open(open));
    on!(on_set_theme, |a, theme| a.set_theme(theme));
    on!(on_set_font, |a, font| a.set_font(font.into()));
    on!(on_set_transparency, |a, value| a.set_transparency(value));
    on!(on_set_wide, |a, wide| a.set_wide(wide));
    on!(on_set_view_mode, |a, mode| a.set_view_mode(mode));
    on!(on_set_format_bar, |a, shown| a.set_format_bar(shown));
    on!(on_sidebar_resized, |a, width| a.set_sidebar_width(width));
    on!(on_split_resized, |a, ratio| a.set_split_ratio(ratio));
    on!(on_set_launch_at_login, |a, enabled| a.set_launch_at_login(enabled));
    on!(on_set_software_rendering, |a, on| a.set_software_rendering(on));
    on!(on_set_close_to_tray, |a, on| a.set_close_to_tray(on));
    on!(on_export_note, |a| a.export_note());
    on!(on_indent, |a, outdent| a.indent(outdent));
    on!(on_find_edited, |a, query| a.find_text(query.into()));
    on!(on_find_step, |a, delta| a.find_step(delta));
    on!(on_replace_one, |a, text| a.replace_one(text.into()));
    on!(on_replace_all, |a, text| a.replace_all(text.into()));
    on!(on_find_closed, |a| a.close_find());
    on!(on_toast_action, |a, index| a.toast_action(index.max(0) as usize));
    on!(on_stop_timer, |a| a.stop_timer());
    on!(on_set_typewriter, |a, on| a.set_typewriter(on));
    on!(on_set_accent, |a, hex| a.set_accent((!hex.is_empty()).then(|| hex.to_string())));
    on!(on_open_stats, |a| a.open_stats());
    on!(on_open_spelling, |a| a.open_spelling());
    on!(on_spell_replace, |a, word, with| a.spell_replace(word.into(), with.into()));
    on!(on_spell_ignore, |a, word| a.spell_ignore(word.into(), false));
    on!(on_spell_add, |a, word| a.spell_ignore(word.into(), true));
    on!(on_look_up_word, |a| a.look_up_word());
    on!(on_open_tasks, |a| a.open_tasks());
    on!(on_use_google_drive, |a| a.use_google_drive());
    on!(on_choose_sync_folder, |a| a.choose_sync_folder());
    on!(on_stop_syncing, |a| a.stop_syncing());
    on!(on_import_markdown_folder, |a| a.import_markdown_folder());
    on!(on_export_all, |a| a.export_all());
    on!(on_save_search, |a| a.save_search());
    on!(on_remove_saved_search, |a, query| a.remove_saved_search(&query));
    on!(on_apply_search, |a, query| a.apply_search(query.into()));
    on!(on_open_on_this_day, |a| a.open_palette(PaletteMode::OnThisDay));
    on_id!(on_move_to_notebook, pick_notebook);
    on_id!(on_pick_emoji, pick_emoji);
    on!(on_link_hover, |a, titles, x, y| {
        let titles: Vec<String> = slint::Model::iter(&titles).map(|t| t.to_string()).collect();
        a.link_hover(titles, x, y);
    });
    on!(on_link_hover_end, |a| a.hide_link_preview());
    on!(on_tasks_show_done_changed, |a, on| a.set_tasks_show_done(on));
    on!(on_quick_task_toggle, |a, id| a.toggle_quick_task(&id));
    on!(on_quick_task_delete, |a, id| a.delete_quick_task(&id));
    {
        let app = app.clone();
        ui.on_task_toggle(move |note, line| {
            if let Some(note) = parse_id(&note) {
                with_app(&app, move |a| a.toggle_task_in(note, line.max(0) as usize));
            }
        });
    }
    {
        let app = app.clone();
        ui.on_task_open(move |note, line| {
            if let Some(note) = parse_id(&note) {
                with_app(&app, move |a| a.open_task(note, line.max(0) as usize));
            }
        });
    }
    on_id!(on_open_reminder, open_reminder);

    // These decide synchronously whether a key press is swallowed, so they can't be deferred.
    macro_rules! on_now {
        ($setter:ident, |$app:ident $(, $arg:ident)*| $body:expr) => {{
            let app = app.clone();
            ui.$setter(move |$($arg),*| match app.try_borrow_mut() {
                Ok(mut $app) => $body,
                Err(_) => false,
            });
        }};
    }
    on_now!(on_auto_pair, |a, typed| a.auto_pair(&typed));
    on_now!(on_paste_link, |a| a.paste_link());
    on_now!(on_undo, |a| a.undo());
    on_now!(on_table_tab, |a, back| a.table_tab(back));
    on_now!(on_table_enter, |a| a.table_enter());
    on!(on_zoom, |a, step| a.zoom(step));
    on!(on_set_line_spacing, |a, spacing| a.set_line_spacing(spacing));
    on!(on_step_ui_scale, |a, step| a.step_ui_scale(step));
    on!(on_set_spotlight, |a, on| a.set_spotlight(on));
    on_now!(on_redo, |a| a.redo());
    on_now!(on_handle_shortcut, |a, combo| a.shortcut(&combo));
    on!(on_slide_step, |a, delta| a.step_slide(delta));
    on!(on_stop_presenting, |a| a.stop_presenting());
    on!(on_set_shortcut, |a, id, combo| a.set_shortcut(&id, &combo));
    on!(on_reset_shortcut, |a, id| a.reset_shortcut(&id));
    on!(on_shortcut_filter_edited, |a, text| a.filter_shortcuts(text.into()));

    let timer_tick = Timer::default();
    timer_tick.start(TimerMode::Repeated, Duration::from_secs(1), {
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                if let Ok(mut app) = app.try_borrow_mut() {
                    app.tick_timer();
                }
            }
        }
    });
    std::mem::forget(timer_tick);
    on!(on_import_zima, |a| a.import_zima());
    on!(on_open_notes_folder, |a| a.open_notes_folder());
    on!(on_open_backups_folder, |a| a.open_backups_folder());
    // `[[note]]` links open (or create) notes; everything else goes to the browser.
    on!(on_link_clicked, |a, url| match url.strip_prefix("note:") {
        Some(title) => a.open_link(&title.replace("%20", " ")),
        None => system::open_url(&url),
    });
    on!(on_open_palette, |a, mode| a.open_palette(match mode {
        1 => PaletteMode::Outline,
        2 => PaletteMode::Template,
        _ => PaletteMode::All,
    }));
    on!(on_palette_query, |a, query| a.palette_query(query.into()));
    on!(on_palette_run, |a, index| a.palette_run(index.max(0) as usize));
    on!(on_palette_closed, |a| a.close_palette());
    on!(on_run_command, |a, id| a.run_command(&id));
    on!(on_navigate, |a, delta| a.navigate(delta));
    on!(on_open_tag, |a, name| {
        let query = format!("#{name}");
        if let Some(ui) = a.ui.upgrade() {
            ui.set_search_text(query.as_str().into());
        }
        a.set_search(query);
    });
    on!(on_calendar_shift, |a, months| a.shift_calendar(months));
    on!(on_calendar_pick, |a, date| a.pick_calendar_day(&date));
    on!(on_open_calendar, |a| a.open_calendar());
    on_id!(on_toggle_pin, toggle_pin);
    on!(on_set_note_order, |a, order| a.set_note_order(order));
    on_id!(on_select_note, toggle_selected);
    on_id!(on_open_history, open_history_of);
    on!(on_history_pick, |a, index| a.pick_version(index.max(0) as usize));
    on!(on_history_restore, |a| a.restore_version());
    on!(on_set_version_history, |a, on| a.set_version_history(on));
    on!(on_bulk_action, |a, action| a.bulk(&action));
    on!(on_clear_selection, |a| a.clear_selection());
    on_id!(on_toggle_archive, toggle_archive);
    on_id!(on_duplicate_note, duplicate);
    on_id!(on_open_sticky, open_sticky);
    {
        let app = app.clone();
        ui.on_set_color(move |id, color| {
            if let Some(id) = parse_id(&id) {
                with_app(&app, move |a| a.set_color(id, (color >= 0).then_some(color as u8)));
            }
        });
    }

    // Mica can only be applied once the native window exists.
    retry_mica(Rc::downgrade(app));

    // "System" theme: Windows reports light/dark only once the window is up, so re-render then
    // (and whenever it changes).
    {
        let app_weak = Rc::downgrade(app);
        let last_dark = std::cell::Cell::new(None::<bool>);
        let watch = Timer::default();
        watch.start(TimerMode::Repeated, Duration::from_millis(500), move || {
            let Some(app) = app_weak.upgrade() else { return };
            let Ok(app) = app.try_borrow() else { return };
            let Some(ui) = app.ui.upgrade() else { return };
            let dark = ui.global::<Theme>().get_dark();
            if last_dark.replace(Some(dark)) != Some(dark) {
                app.refresh_preview();
                window::apply_mica(&ui, dark);
            }
            // The window only exists after startup, and moving to a monitor with different
            // Windows scaling resets the interface size, so keep re-applying it (a no-op when unchanged).
            app.apply_ui_scale();
        });
        std::mem::forget(watch);
    }

    let check = Timer::default();
    check.start(TimerMode::Repeated, REMINDER_CHECK, {
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                app.borrow_mut().fire_due_reminders(false);
            }
        }
    });
    std::mem::forget(check); // lives for the whole app

    // Clock in the title bar.
    let update_clock = {
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_clock(Local::now().format("%a %-d %b  \u{00b7}  %-I:%M %p").to_string().into());
            }
        }
    };
    update_clock();
    let clock = Timer::default();
    clock.start(TimerMode::Repeated, Duration::from_secs(5), update_clock);
    std::mem::forget(clock);

    // Keep relative times ("5m", "Yesterday", "Today 5:00 PM") fresh.
    let tick = Timer::default();
    tick.start(TimerMode::Repeated, Duration::from_secs(60), {
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                let Ok(mut app) = app.try_borrow_mut() else { return };
                app.refresh_lists();
                app.refresh_reminders();
                app.tick_auto_theme();
                app.auto_backup();
                // Also catches the window's spot if Zima is ended without quitting (Windows shutting down).
                app.remember_window();
                app.remember_cursor();
            }
        }
    });
    std::mem::forget(tick);

    // Today's backup, shortly after launch (and then by the minute tick above, after midnight).
    Timer::single_shot(BACKUP_DELAY, {
        let app = Rc::downgrade(app);
        move || {
            if let Some(app) = app.upgrade() {
                with_app(&app, |a| a.auto_backup());
            }
        }
    });
}

/// Run `f` on the app now, or, if the app is busy (Slint can call back into Rust synchronously,
/// e.g. moving the cursor from Rust fires `cursor-moved`), right after the current call finishes.
fn with_app(app: &Rc<RefCell<App>>, f: impl FnOnce(&mut App) + 'static) {
    match app.try_borrow_mut() {
        Ok(mut app) => f(&mut app),
        Err(_) => {
            let app = Rc::downgrade(app);
            Timer::single_shot(Duration::ZERO, move || {
                if let Some(app) = app.upgrade() {
                    f(&mut app.borrow_mut());
                }
            });
        }
    }
}

fn retry_mica(app: rc::Weak<RefCell<App>>) {
    Timer::single_shot(Duration::from_millis(100), move || {
        let Some(strong) = app.upgrade() else { return };
        let Some(ui) = strong.borrow().ui.upgrade() else { return };
        if !window::apply_mica(&ui, ui.global::<Theme>().get_dark()) {
            retry_mica(app);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_stay_on_one_line() {
        assert_eq!(one_line("Trip to Lisbon"), None);
        assert_eq!(one_line(""), None);
        assert_eq!(one_line("Trip\nto Lisbon").as_deref(), Some("Trip to Lisbon"));
        assert_eq!(one_line("Trip\r\nto\rLisbon\n").as_deref(), Some("Trip to Lisbon "));
    }

    #[test]
    fn divider_sizes_stay_in_range() {
        assert_eq!(clamp_sidebar_width(300.0), 300.0);
        assert_eq!(clamp_sidebar_width(50.0), 180.0);
        assert_eq!(clamp_sidebar_width(2000.0), 480.0);
        assert_eq!(clamp_sidebar_width(f32::NAN), 260.0);
        assert_eq!(clamp_split_ratio(0.6), 0.6);
        assert_eq!(clamp_split_ratio(0.0), 0.25);
        assert_eq!(clamp_split_ratio(1.0), 0.75);
        assert_eq!(clamp_split_ratio(f32::INFINITY), 0.5);
    }

    #[test]
    fn colour_dots_match_preview_colours() {
        let dots = colour_dots();
        assert_eq!(dots.len(), markdown::TEXT_COLOURS.len() + 1);
        assert_eq!((dots[0].name.as_str(), dots[0].title.as_str()), ("", "Highlight"));
        assert_eq!((dots[1].name.as_str(), dots[1].title.as_str()), ("red", "Red"));
        // Every colour is valid (a bad hex would fall back to transparent).
        for dot in &dots {
            assert_ne!(dot.light, slint::Color::default(), "{}", dot.name);
            assert_ne!(dot.dark, slint::Color::default(), "{}", dot.name);
        }
        // Every named dot is one the toolbar can apply.
        for dot in dots.iter().skip(1) {
            assert!(format::apply(&format!("colour:{}", dot.name), "x", 0, 1).is_some(), "{}", dot.name);
        }
    }

    #[test]
    fn step_scale_moves_by_ten_percent() {
        assert_eq!(step_scale(1.0, 1, 0.8, 2.0), 1.1);
        assert_eq!(step_scale(1.0, -1, 0.8, 2.0), 0.9);
        assert_eq!(step_scale(1.5, 1, UI_SCALE_MIN, UI_SCALE_MAX), 1.6);
    }

    fn rows(items: &[(&str, &str)]) -> ModelRc<NoteRow> {
        model(items.iter().map(|(id, title)| NoteRow { id: (*id).into(), title: (*title).into(), meta: "1d".into(), ..Default::default() }))
    }

    fn titles(rows: &ModelRc<NoteRow>) -> Vec<String> {
        rows.iter().map(|r| r.title.to_string()).collect()
    }

    #[test]
    fn retitle_changes_only_that_note() {
        let list = rows(&[("1", "Apples"), ("2", "Pears"), ("3", "Plums")]);
        retitle_rows(&list, 2, "Pears and figs");
        assert_eq!(titles(&list), vec!["Apples", "Pears and figs", "Plums"]);
        // The rest of the row is kept.
        assert_eq!(list.row_data(1).unwrap().meta, "1d");
    }

    #[test]
    fn retitle_leaves_other_lists_alone() {
        let list = rows(&[("1", "Apples"), ("3", "Plums")]);
        retitle_rows(&list, 2, "Pears");
        assert_eq!(titles(&list), vec!["Apples", "Plums"]);
        // An id that only starts the same is a different note.
        let list = rows(&[("12", "Twelve")]);
        retitle_rows(&list, 1, "One");
        assert_eq!(titles(&list), vec!["Twelve"]);
    }

    #[test]
    fn retitle_edges() {
        let empty = rows(&[]);
        retitle_rows(&empty, 1, "x");
        assert_eq!(empty.row_count(), 0);
        let list = rows(&[("1", "Old")]);
        retitle_rows(&list, 1, "");
        assert_eq!(titles(&list), vec![""]);
    }

    #[test]
    fn saved_cursor_goes_back() {
        assert_eq!(saved_cursor("hello world", Some(6)), 6);
        assert_eq!(saved_cursor("hello", Some(5)), 5);
    }

    #[test]
    fn saved_cursor_none_is_start() {
        assert_eq!(saved_cursor("hello", None), 0);
        assert_eq!(saved_cursor("", None), 0);
    }

    #[test]
    fn saved_cursor_fits_a_changed_note() {
        // The note got shorter on another device.
        assert_eq!(saved_cursor("hi", Some(40)), 2);
        assert_eq!(saved_cursor("", Some(3)), 0);
        // "é" is two bytes; never land between them.
        assert_eq!(saved_cursor("café!", Some(4)), 3);
        assert_eq!(saved_cursor("café!", Some(5)), 5);
    }

    #[test]
    fn step_scale_zero_resets() {
        assert_eq!(step_scale(1.7, 0, 0.8, 2.0), 1.0);
        assert_eq!(step_scale(0.8, 0, 0.8, 2.0), 1.0);
    }

    #[test]
    fn step_scale_stops_at_the_limits() {
        assert_eq!(step_scale(2.0, 1, 0.8, 2.0), 2.0);
        assert_eq!(step_scale(0.8, -1, 0.8, 2.0), 0.8);
        // A hand-edited value outside the range comes back inside it.
        assert_eq!(step_scale(5.0, 1, 0.8, 2.0), 2.0);
        assert_eq!(step_scale(0.1, -1, 0.8, 2.0), 0.8);
    }

    #[test]
    fn step_scale_does_not_drift() {
        // Ten steps up and back must land exactly where they started.
        let mut scale = 1.0;
        for _ in 0..10 {
            scale = step_scale(scale, 1, 0.8, 2.0);
        }
        assert_eq!(scale, 2.0);
        for _ in 0..10 {
            scale = step_scale(scale, -1, 0.8, 2.0);
        }
        assert_eq!(scale, 1.0);
    }
}
