//! Quick switcher (Ctrl+K), outline, reopening closed notes, keyboard navigation.

use slint::{ModelRc, VecModel};

use super::{App, display_title};
use crate::model::NoteId;
use crate::palette::{self, fuzzy_score};
use crate::PaletteItem;

/// What the quick switcher is listing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum PaletteMode {
    /// Notes and commands.
    All,
    /// Headings of the current note (outline).
    Outline,
    /// Pick the note to merge the current one into.
    MergeTarget,
    /// Pick a template for a new note.
    Template,
    /// Pick (or name) a notebook for a note.
    Notebook(NoteId),
    /// Pick (or paste) an emoji for a note.
    Emoji(NoteId),
    /// Notes from this day in earlier weeks, months and years.
    OnThisDay,
}

#[derive(Clone)]
pub enum PaletteEntry {
    Note(NoteId),
    Command(&'static str),
    /// Byte offset of a heading in the current note.
    Heading(usize),
    MergeInto(NoteId),
    Template(NoteId),
    /// Put the note in this notebook (`None` = no notebook).
    SetNotebook(NoteId, Option<String>),
    /// Give the note this emoji (`None` = no emoji).
    SetEmoji(NoteId, Option<String>),
}

const MAX_RESULTS: usize = 50;

impl App {
    pub fn open_palette(&mut self, mode: PaletteMode) {
        self.palette_mode = mode;
        if let Some(ui) = self.ui.upgrade() {
            let placeholder = match mode {
                PaletteMode::All => "Search notes and commands\u{2026}",
                PaletteMode::Outline => "Jump to a heading\u{2026}",
                PaletteMode::MergeTarget => "Merge this note into\u{2026}",
                PaletteMode::Template => "New note from template\u{2026}",
                PaletteMode::Notebook(_) => "Move to notebook\u{2026} (type a new name to create one)",
                PaletteMode::Emoji(_) => "Pick an emoji\u{2026} (Win+. to type any emoji)",
            PaletteMode::OnThisDay => "On this day\u{2026}",
            };
            ui.set_palette_placeholder(placeholder.into());
            ui.set_palette_open(true);
            ui.invoke_focus_palette();
        }
        self.palette_query(String::new());
    }

    /// Recompute results for the typed query.
    pub fn palette_query(&mut self, query: String) {
        let query = query.trim().to_string();
        let mut scored: Vec<(i32, PaletteEntry, PaletteItem)> = Vec::new();
        let item = |title: String, subtitle: String, kind: &str, shortcut: &str| PaletteItem {
            title: title.into(),
            subtitle: subtitle.into(),
            kind: kind.into(),
            shortcut: shortcut.into(),
            emoji: Default::default(),
            icon: Default::default(),
        };
        let note_emoji = |note: &crate::model::Note| -> slint::SharedString { note.emoji.as_deref().and_then(crate::emoji::clean).unwrap_or_default().into() };

        match self.palette_mode {
            PaletteMode::All => {
                let mut notes: Vec<_> = self.notes.iter().filter(|n| n.is_live()).collect();
                notes.sort_by_key(|n| std::cmp::Reverse(n.modified));
                for note in notes {
                    let title = display_title(note);
                    let score = if query.is_empty() {
                        Some(0)
                    } else {
                        // Title matches rank above body matches.
                        fuzzy_score(&query, &title)
                            .map(|s| s + 30)
                            .or_else(|| note.body.to_lowercase().contains(&query.to_lowercase()).then_some(0))
                    };
                    if let Some(score) = score {
                        let subtitle = if note.archived { "Archived".to_string() } else { super::relative_time(note.modified) };
                        scored.push((score, PaletteEntry::Note(note.id), PaletteItem { emoji: note_emoji(note), ..item(title, subtitle, "note", "") }));
                    }
                }
                // Commands only appear once you type.
                if !query.is_empty() {
                    for command in palette::COMMANDS {
                        if let Some(score) = fuzzy_score(&query, command.title) {
                            let entry = PaletteItem { icon: palette::icon(command.id).into(), ..item(command.title.into(), String::new(), "command", command.shortcut) };
                            scored.push((score + 10, PaletteEntry::Command(command.id), entry));
                        }
                    }
                }
            }
            PaletteMode::Outline => {
                if let Some(note) = self.current() {
                    let mut offset = 0;
                    for line in note.body.split('\n') {
                        let hashes = line.bytes().take_while(|&b| b == b'#').count();
                        if (1..=6).contains(&hashes) && line[hashes..].starts_with(' ') {
                            let text = line[hashes + 1..].trim().to_string();
                            let indent = "    ".repeat(hashes - 1);
                            if let Some(score) = fuzzy_score(&query, &text) {
                                // Keep document order: score only filters.
                                let _ = score;
                                scored.push((-(offset as i32), PaletteEntry::Heading(offset), item(format!("{indent}{text}"), format!("H{hashes}"), "heading", "")));
                            }
                        }
                        offset += line.len() + 1;
                    }
                }
            }
            PaletteMode::Notebook(target) => {
                let mut names: Vec<String> = self.notebooks().into_iter().map(|(name, _)| name).collect();
                names.sort_by_key(|n| n.to_lowercase());
                let typed = query.trim().to_string();
                if !typed.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(&typed)) {
                    scored.push((1000, PaletteEntry::SetNotebook(target, Some(typed.clone())), item(format!("Create notebook \u{201c}{typed}\u{201d}"), String::new(), "command", "")));
                }
                for name in names {
                    if let Some(score) = fuzzy_score(&query, &name) {
                        scored.push((score, PaletteEntry::SetNotebook(target, Some(name.clone())), item(name, String::new(), "note", "")));
                    }
                }
                scored.push((-1000, PaletteEntry::SetNotebook(target, None), item("No notebook".into(), String::new(), "command", "")));
            }
            PaletteMode::Emoji(target) => {
                for choice in crate::emoji::choices(&query) {
                    let emoji = choice.emoji.clone();
                    scored.push((choice.score, PaletteEntry::SetEmoji(target, Some(choice.emoji)), PaletteItem { emoji: emoji.into(), ..item(choice.label, String::new(), "command", "") }));
                }
                scored.push((-1000, PaletteEntry::SetEmoji(target, None), item("No emoji".into(), String::new(), "command", "")));
            }
            PaletteMode::OnThisDay => {
                for (id, label) in self.on_this_day() {
                    if let Some(note) = self.find(id) {
                        let title = display_title(note);
                        if fuzzy_score(&query, &title).is_some() {
                            scored.push((0, PaletteEntry::Note(id), PaletteItem { emoji: note_emoji(note), ..item(title, label, "note", "") }));
                        }
                    }
                }
                if scored.is_empty() && query.is_empty() {
                    self.toast("Nothing from this day yet", false);
                }
            }
            PaletteMode::MergeTarget | PaletteMode::Template => {
                let current = self.state.current;
                for note in self.notes.iter().filter(|n| n.is_live() && Some(n.id) != current) {
                    let title = display_title(note);
                    let is_template = title.to_lowercase().starts_with("template");
                    if self.palette_mode == PaletteMode::Template && !is_template {
                        continue;
                    }
                    if let Some(score) = fuzzy_score(&query, &title) {
                        let entry = if self.palette_mode == PaletteMode::Template {
                            PaletteEntry::Template(note.id)
                        } else {
                            PaletteEntry::MergeInto(note.id)
                        };
                        scored.push((score, entry, PaletteItem { emoji: note_emoji(note), ..item(title, super::relative_time(note.modified), "note", "") }));
                    }
                }
            }
        }

        if self.palette_mode != PaletteMode::Outline || !query.is_empty() {
            scored.sort_by_key(|(score, _, _)| std::cmp::Reverse(*score));
        }
        scored.truncate(MAX_RESULTS);
        if self.palette_mode == PaletteMode::Template && scored.is_empty() && query.is_empty() {
            self.toast("No templates yet. Any note titled \u{201c}Template: \u{2026}\u{201d} becomes one.", false);
        }
        let (entries, items): (Vec<_>, Vec<_>) = scored.into_iter().map(|(_, e, i)| (e, i)).unzip();
        self.palette_entries = entries;
        if let Some(ui) = self.ui.upgrade() {
            ui.set_palette_index(0);
            ui.set_palette_items(ModelRc::new(VecModel::from(items)));
        }
    }

    pub fn palette_run(&mut self, index: usize) {
        let Some(entry) = self.palette_entries.get(index).cloned() else { return };
        self.close_palette();
        match entry {
            PaletteEntry::Note(id) => {
                if self.find(id).is_some_and(|n| n.archived) {
                    self.toggle_archive(id);
                }
                self.open(id);
            }
            PaletteEntry::Command(id) => self.run_command(id),
            PaletteEntry::Heading(offset) => {
                if let Some(ui) = self.ui.upgrade() {
                    if ui.get_view_mode() == 2 {
                        ui.set_view_mode(1);
                        self.set_view_mode(1);
                    }
                }
                self.show_selection(offset, offset, true);
                self.cursor = offset;
            }
            PaletteEntry::MergeInto(target) => self.merge_current_into(target),
            PaletteEntry::Template(template) => self.new_from_template(template),
            PaletteEntry::SetNotebook(id, notebook) => self.set_notebook(id, notebook),
            PaletteEntry::SetEmoji(id, emoji) => self.set_emoji(id, emoji),
        }
    }

    pub fn close_palette(&mut self) {
        self.palette_entries.clear();
        if let Some(ui) = self.ui.upgrade() {
            // Move focus first: destroying the focused text box leaves the next click stranded.
            ui.invoke_focus_default();
            ui.set_palette_open(false);
            ui.set_palette_items(ModelRc::default());
        }
    }

    /// Commands from the quick switcher (and a few shortcuts).
    pub fn run_command(&mut self, id: &str) {
        let Some(ui) = self.ui.upgrade() else { return };
        let current = self.state.current;
        match id {
            "new-note" => self.new_note(),
            "palette" => self.open_palette(PaletteMode::All),
            "close-note" => {
                if let Some(id) = current {
                    self.close(id);
                }
            }
            "cycle-view" => {
                let mode = (ui.get_view_mode() + 1) % 4;
                ui.set_view_mode(mode);
                self.set_view_mode(mode);
            }
            "bold" | "italic" | "underline" => self.format(id),
            "shortcuts" => ui.invoke_open_settings("shortcuts".into()),
            "edit-theme" => self.edit_theme(),
            "present" => self.start_presenting(),
            "daily" => self.open_daily(chrono::Local::now().date_naive()),
            "reopen" => self.reopen_closed(),
            "find" => ui.invoke_open_find(false),
            "replace" => ui.invoke_open_find(true),
            "outline" => self.open_palette(PaletteMode::Outline),
            "mode-edit" | "mode-split" | "mode-preview" | "mode-live" => {
                let mode = match id {
                    "mode-edit" => 0,
                    "mode-split" => 1,
                    "mode-live" => 3,
                    _ => 2,
                };
                ui.set_view_mode(mode);
                self.set_view_mode(mode);
            }
            "sidebar" => {
                let open = !ui.get_sidebar_open();
                ui.set_sidebar_open(open);
                self.set_sidebar_open(open);
            }
            "zen" => ui.set_zen(!ui.get_zen()),
            "format-bar" => {
                let shown = !ui.get_format_bar();
                ui.set_format_bar(shown);
                self.set_format_bar(shown);
            }
            "wide" => {
                let wide = !self.state.wide;
                self.set_wide(wide);
                self.apply_note_overrides();
            }
            "pin" | "favorite" | "archive" | "duplicate" | "delete" | "sticky" => {
                let Some(id_) = current else { return };
                match id {
                    "pin" => self.toggle_pin(id_),
                    "favorite" => self.toggle_favorite(id_),
                    "archive" => self.toggle_archive(id_),
                    "duplicate" => self.duplicate(id_),
                    "sticky" => self.open_sticky(id_),
                    _ => self.delete(id_),
                }
            }
            "merge" => {
                if current.is_some() {
                    self.open_palette(PaletteMode::MergeTarget);
                }
            }
            "empty-bin" => self.empty_bin(),
            "timer" => self.start_timer(25, false),
            "timer-stop" => self.stop_timer(),
            "reminders" => ui.set_reminders_open(true),
            "calendar" => self.open_calendar(),
            "stats" => self.open_stats(),
            "random" => self.open_random(),
            "on-this-day" => self.open_palette(PaletteMode::OnThisDay),
            "notebook" => {
                if let Some(id) = current {
                    self.pick_notebook(id);
                }
            }
            "emoji" => {
                if let Some(id) = current {
                    self.pick_emoji(id);
                }
            }
            "save-search" => self.save_search(),
            "import-folder" => self.import_markdown_folder(),
            "export-all" => self.export_all(),
            "google-drive" => self.use_google_drive(),
            "sync-folder" => self.choose_sync_folder(),
            "spell" => self.open_spelling(),
            "tasks" => self.open_tasks(),
            "lookup" => self.look_up_word(),
            "history" => self.open_history(),
            "sort-changed" => self.set_note_order(0),
            "sort-title" => self.set_note_order(1),
            "sort-created" => self.set_note_order(2),
            "settings" => ui.invoke_open_settings("".into()),
            "export" => self.export_note(),
            "export-html" => self.export_html(),
            "print" => self.print_note(),
            "share-image" => self.share_image(),
            "lock" => self.toggle_lock(),
            "import" => self.import_zima(),
            "backup" => self.backup_now(),
            // From Settings only (it deletes every copy, so it isn't in the quick switcher).
            "clear-history" => self.clear_history(),
            "open-backups" => self.open_backups_folder(),
            "open-folder" => self.open_notes_folder(),
            "quit" => {
                self.flush();
                let _ = slint::quit_event_loop();
            }
            theme if theme.starts_with("theme-") => {
                if let Ok(index) = theme["theme-".len()..].parse() {
                    self.set_theme(index);
                }
            }
            font if font.starts_with("font-note-") => {
                let font = &font["font-note-".len()..];
                self.set_note_font((font != "default").then(|| font.to_string()));
            }
            width if width.starts_with("width-note-") => {
                let wide = match &width["width-note-".len()..] {
                    "wide" => Some(true),
                    "normal" => Some(false),
                    _ => None,
                };
                self.set_note_wide(wide);
            }
            _ => {}
        }
    }

    pub fn reopen_closed(&mut self) {
        while let Some(id) = self.recently_closed.pop() {
            if self.find(id).is_some_and(|n| n.is_live()) {
                self.open(id);
                return;
            }
        }
        self.toast("No recently closed notes", false);
    }

    /// Up/Down in the sidebar order (pinned, then most recent).
    pub fn navigate(&mut self, delta: i32) {
        let order = self.list_order();
        if order.is_empty() {
            return;
        }
        let position = self.state.current.and_then(|id| order.iter().position(|&o| o == id));
        let next = match position {
            Some(p) => (p as i32 + delta).clamp(0, order.len() as i32 - 1) as usize,
            None => 0,
        };
        self.open(order[next]);
    }

    /// Listed notes in sidebar order: pinned first, then most recently modified.
    pub fn list_order(&self) -> Vec<NoteId> {
        let mut notes: Vec<_> = self.notes.iter().filter(|n| n.is_listed()).collect();
        notes.sort_by_key(|n| (!n.pinned, std::cmp::Reverse(n.modified)));
        notes.into_iter().map(|n| n.id).collect()
    }
}
