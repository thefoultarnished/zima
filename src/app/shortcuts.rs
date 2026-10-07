//! Keyboard shortcuts: every quick-switcher command can have one, and they can be rebound in Settings.

use slint::{ModelRc, VecModel};

use super::App;
use crate::ShortcutRow;
use crate::palette::COMMANDS;

/// Commands that edit the note's text: only when the cursor is in the note.
const EDITING: &[&str] = &["bold", "italic", "underline"];

impl App {
    /// The shortcut for a command: the user's override, else the default.
    fn binding(&self, id: &str) -> String {
        match self.state.shortcuts.get(id) {
            Some(custom) => custom.clone(),
            None => COMMANDS.iter().find(|c| c.id == id).map_or(String::new(), |c| c.shortcut.to_string()),
        }
    }

    /// A key combination like "Ctrl+Shift+K" was pressed; run its command. Returns true if it was used.
    pub fn shortcut(&mut self, combo: &str) -> bool {
        let Some(command) = COMMANDS.iter().find(|c| !combo.is_empty() && self.binding(c.id).eq_ignore_ascii_case(combo)) else {
            return false;
        };
        let Some(ui) = self.ui.upgrade() else { return false };
        if EDITING.contains(&command.id) && !ui.get_body_focused() {
            return false;
        }
        let has_note = self.state.current.is_some();
        match command.id {
            "find" | "replace" if !has_note || ui.get_view_mode() == 2 => return true,
            "outline" | "lookup" | "spell" if !has_note => return true,
            _ => {}
        }
        self.run_command(command.id);
        true
    }

    pub fn refresh_shortcuts(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let filter = self.shortcut_filter.to_lowercase();
        let mut rows: Vec<ShortcutRow> = COMMANDS
            .iter()
            .filter(|c| c.title.to_lowercase().contains(&filter) || self.binding(c.id).to_lowercase().contains(&filter))
            .map(|c| ShortcutRow {
                id: c.id.into(),
                title: c.title.into(),
                combo: self.binding(c.id).into(),
                custom: self.state.shortcuts.contains_key(c.id),
            })
            .collect();
        // Commands with shortcuts first.
        rows.sort_by_key(|r| (r.combo.is_empty(), r.title.to_lowercase()));
        ui.set_shortcut_rows(ModelRc::new(VecModel::from(rows)));
    }

    pub fn filter_shortcuts(&mut self, filter: String) {
        self.shortcut_filter = filter;
        self.refresh_shortcuts();
    }

    pub fn open_shortcuts(&mut self) {
        self.shortcut_filter.clear();
        self.refresh_shortcuts();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_settings_open(false);
            ui.set_shortcuts_open(true);
        }
    }

    /// Bind `combo` to a command (taking it away from any other command that had it).
    pub fn set_shortcut(&mut self, id: &str, combo: &str) {
        for command in COMMANDS {
            if command.id != id && self.binding(command.id).eq_ignore_ascii_case(combo) {
                self.state.shortcuts.insert(command.id.to_string(), String::new());
            }
        }
        let default = COMMANDS.iter().find(|c| c.id == id).map_or("", |c| c.shortcut);
        if combo == default {
            self.state.shortcuts.remove(id);
        } else {
            self.state.shortcuts.insert(id.to_string(), combo.to_string());
        }
        self.save_state();
        self.refresh_shortcuts();
    }

    pub fn reset_shortcut(&mut self, id: &str) {
        self.state.shortcuts.remove(id);
        self.save_state();
        self.refresh_shortcuts();
    }
}
