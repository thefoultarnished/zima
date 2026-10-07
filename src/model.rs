use serde::{Deserialize, Serialize};

pub type NoteId = u64;

/// A note. Metadata is stored in `index.json`; the body lives in `notes/<id>.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Note {
    pub id: NoteId,
    pub title: String,
    #[serde(skip)]
    pub body: String,
    /// Unix time in milliseconds.
    pub modified: i64,
    /// Unix time in milliseconds; 0 for notes from before this was tracked.
    pub created: i64,
    pub favorite: bool,
    /// Set when the note is in the Bin.
    pub deleted_at: Option<i64>,
    pub pinned: bool,
    /// Hidden from the main lists without deleting.
    pub archived: bool,
    /// Index into the note colour palette.
    pub color: Option<u8>,
    /// Emoji shown before the title in the sidebar and sticky note.
    pub emoji: Option<String>,
    /// Word goal for this note.
    pub goal: Option<u32>,
    /// The date (YYYY-MM-DD) this note is the daily note for.
    pub daily: Option<String>,
    /// Optional notebook (folder) name.
    pub notebook: Option<String>,
    /// Per-note overrides of the global font and wide layout.
    pub font: Option<String>,
    pub wide: Option<bool>,
}

impl Note {
    pub fn is_live(&self) -> bool {
        self.deleted_at.is_none()
    }

    /// In the main lists: not in the Bin and not archived.
    pub fn is_listed(&self) -> bool {
        self.deleted_at.is_none() && !self.archived
    }
}

/// Which sidebar sections are expanded.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Sections {
    pub active: bool,
    pub pinned: bool,
    pub favorites: bool,
    pub recent: bool,
    pub all: bool,
    pub tags: bool,
    pub archive: bool,
    pub bin: bool,
    pub saved: bool,
    pub notebooks: bool,
}

impl Default for Sections {
    fn default() -> Self {
        Self { active: true, pinned: true, favorites: true, recent: true, all: true, tags: true, archive: false, bin: false, saved: true, notebooks: true }
    }
}

/// UI state persisted in `state.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    /// Notes in the "Active" section, in the order they were opened.
    pub open_ids: Vec<NoteId>,
    pub current: Option<NoteId>,
    pub sections: Sections,
    pub sidebar_collapsed: bool,
    /// 0 = follow system, 1 = Light, 2 = Dark, 3 = Ethereal, 4 = Zima Blue, 5 = Blue White.
    pub theme: i32,
    pub font: String,
    /// Opacity of the window surfaces, 0.3–1.0. Below 1 the Mica backdrop shows through.
    pub transparency: f32,
    pub wide: bool,
    /// 0 = edit, 1 = split, 2 = preview.
    pub view_mode: i32,
    pub format_bar: bool,
    /// Keep the cursor line centred in zen mode.
    pub typewriter: bool,
    /// Text size multiplier for notes (Ctrl + / Ctrl −).
    pub zoom: f32,
    /// Size of the whole interface (icons, buttons, spacing and text), on top of Windows' own scaling.
    pub ui_scale: f32,
    /// Line height factor for note text.
    pub line_spacing: f32,
    /// In zen mode, fade the text above and below the line you're writing.
    pub spotlight: bool,
    /// Custom accent colour ("#rrggbb"); the theme's own when unset.
    pub accent: Option<String>,
    /// Secret the browser clipper must send (shown in Settings).
    pub clip_token: String,
    /// Saved searches shown in the sidebar.
    pub saved_searches: Vec<String>,
    /// Notes popped out as sticky notes, reopened at launch.
    pub stickies: Vec<NoteId>,
    /// Closing the window hides it to the tray (reminders keep firing). Off: closing quits Zima.
    pub close_to_tray: bool,
    /// Whether we've told the user that closing the window keeps Zima in the tray.
    pub tray_hint_shown: bool,
    /// Rebound keyboard shortcuts: command id -> "Ctrl+Shift+K" ("" = none).
    pub shortcuts: std::collections::BTreeMap<String, String>,
    /// Draw with the CPU instead of the GPU: much less memory, less smooth animations. Read at startup.
    pub software_rendering: bool,
    /// Where the main window was last time, so it opens there again. `None`: centred at the default size.
    pub window: Option<WindowPlacement>,
    /// Where the cursor was in each note (byte offset), so reopening a note goes back there.
    /// Notes left with the cursor at the very start aren't listed.
    pub cursors: std::collections::BTreeMap<NoteId, usize>,
    /// Order of the note lists: 0 = last changed first, 1 = title A to Z, 2 = newest created first.
    pub note_order: i32,
}

/// The main window's position and size in physical pixels.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// When maximized, the position and size above are the ones to go back to on un-maximize.
    pub maximized: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            open_ids: Vec::new(),
            current: None,
            sections: Sections::default(),
            sidebar_collapsed: false,
            theme: 0,
            font: "Inter".into(),
            transparency: 1.0,
            wide: false,
            view_mode: 0,
            format_bar: false,
            typewriter: true,
            zoom: 1.0,
            ui_scale: 1.0,
            line_spacing: 1.25,
            spotlight: true,
            accent: None,
            stickies: Vec::new(),
            saved_searches: Vec::new(),
            clip_token: String::new(),
            close_to_tray: true,
            tray_hint_shown: false,
            shortcuts: Default::default(),
            software_rendering: false,
            window: None,
            cursors: Default::default(),
            note_order: 0,
        }
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// A scheduled `@remind`, stored in `reminders.json`. Removed once it fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reminder {
    pub id: u64,
    pub note_id: NoteId,
    pub text: String,
    /// Unix time in milliseconds.
    pub due: i64,
    /// Repeating reminders are re-scheduled instead of removed when they fire.
    #[serde(default)]
    pub repeat: Option<crate::reminders::Repeat>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn note_without_emoji_loads() {
        let note: Note = serde_json::from_str(r#"{"id":1,"title":"x","color":2}"#).unwrap();
        assert_eq!(note.emoji, None);
        assert_eq!(note.color, Some(2));
    }

    #[test]
    fn emoji_round_trips() {
        let note = Note { id: 7, emoji: Some("\u{1F389}".into()), ..Default::default() };
        let json = serde_json::to_string(&note).unwrap();
        let back: Note = serde_json::from_str(&json).unwrap();
        assert_eq!(back.emoji.as_deref(), Some("\u{1F389}"));
    }

    #[test]
    fn cursors_round_trip() {
        let mut state = UiState::default();
        state.cursors.insert(1_700_000_000_000, 42);
        let json = serde_json::to_string(&state).unwrap();
        let back: UiState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.cursors.get(&1_700_000_000_000), Some(&42));
    }

    #[test]
    fn state_without_cursors_loads() {
        let state: UiState = serde_json::from_str(r#"{"theme":2}"#).unwrap();
        assert!(state.cursors.is_empty());
        assert_eq!(state.theme, 2);
    }
}
