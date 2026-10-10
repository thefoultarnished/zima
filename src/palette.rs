//! Quick switcher (Ctrl+K): fuzzy matching and the list of commands.

/// Score how well `query` matches `text` (higher is better); `None` if the letters don't all appear in order.
/// Consecutive letters, word starts and an early first match score higher.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i32> {
    let query: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    if query.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = text.to_lowercase().chars().collect();
    let mut score = 0;
    let mut qi = 0;
    let mut previous: Option<usize> = None;
    for (i, c) in chars.iter().enumerate() {
        if qi < query.len() && *c == query[qi] {
            score += 10;
            if previous == Some(i.wrapping_sub(1)) {
                score += 15; // consecutive
            }
            if i == 0 || !chars[i - 1].is_alphanumeric() {
                score += 20; // start of a word
            }
            if qi == 0 {
                score -= i.min(20) as i32; // earlier is better
            }
            previous = Some(i);
            qi += 1;
        }
    }
    if qi < query.len() {
        return None;
    }
    // The query appearing as-is ("notebook" in "…a notebook") beats scattered letters.
    let needle: String = query.iter().collect();
    let haystack: String = chars.iter().collect();
    if haystack.contains(&needle) {
        score += 60;
    }
    Some(score - (chars.len() as i32 / 8))
}

pub struct Command {
    pub id: &'static str,
    pub title: &'static str,
    pub shortcut: &'static str,
}

/// The icon a command shows in the quick switcher: a name the switcher turns into an `Icons` path
/// (`icon-for` in `ui/palette.slint`). Unknown or empty names show a small arrow.
pub fn icon(id: &str) -> &'static str {
    match id {
        "new-note" | "duplicate" => "plus",
        "palette" | "find" | "save-search" | "lookup" => "search",
        "replace" => "replace",
        "close-note" | "quit" => "close",
        "reopen" | "history" => "restore",
        "bold" | "italic" | "underline" | "format-bar" => "type",
        "shortcuts" => "keyboard",
        "outline" | "sort-changed" | "sort-title" => "bullets",
        "sort-created" => "numbers",
        "mode-edit" | "mode-live" | "sticky" | "daily" | "emoji" => "edit",
        "mode-split" | "cycle-view" | "wide" | "width-note-default" | "width-note-wide" | "width-note-normal" => "split",
        "mode-preview" | "zen" | "present" | "on-this-day" | "random" => "eye",
        "sidebar" => "sidebar",
        "pin" | "favorite" => "star",
        "archive" | "notebook" | "open-folder" | "google-drive" | "sync-folder" | "open-backups" => "folder",
        "merge" | "import" | "import-folder" => "import",
        "delete" | "empty-bin" => "trash",
        "timer" => "timer",
        "timer-stop" => "stop",
        "reminders" | "calendar" => "bell",
        "tasks" => "task",
        "spell" => "check",
        "settings" | "edit-theme" | "font-note-default" => "settings",
        "export" | "export-html" | "export-all" | "backup" | "print" | "share-image" => "export",
        id if id.starts_with("theme-") => "settings",
        _ => "",
    }
}

pub const COMMANDS: &[Command] = &[
    Command { id: "new-note", title: "New note", shortcut: "Ctrl+N" },
    Command { id: "palette", title: "Quick switcher", shortcut: "Ctrl+K" },
    Command { id: "close-note", title: "Close this note", shortcut: "Ctrl+W" },
    Command { id: "cycle-view", title: "View: next (edit, split, preview, live)", shortcut: "Ctrl+E" },
    Command { id: "bold", title: "Bold", shortcut: "Ctrl+B" },
    Command { id: "italic", title: "Italic", shortcut: "Ctrl+I" },
    Command { id: "underline", title: "Underline", shortcut: "Ctrl+U" },
    Command { id: "shortcuts", title: "Keyboard shortcuts\u{2026}", shortcut: "" },
    Command { id: "daily", title: "Today's note", shortcut: "Ctrl+D" },
    Command { id: "reopen", title: "Reopen closed note", shortcut: "Ctrl+Shift+T" },
    Command { id: "find", title: "Find in note", shortcut: "Ctrl+F" },
    Command { id: "replace", title: "Find and replace", shortcut: "Ctrl+H" },
    Command { id: "outline", title: "Outline: jump to a heading", shortcut: "Ctrl+Shift+O" },
    Command { id: "mode-edit", title: "View: Edit", shortcut: "" },
    Command { id: "mode-split", title: "View: Split", shortcut: "" },
    Command { id: "mode-preview", title: "View: Preview", shortcut: "" },
    Command { id: "mode-live", title: "View: Live", shortcut: "" },
    Command { id: "sidebar", title: "Toggle sidebar", shortcut: "Ctrl+\\" },
    Command { id: "zen", title: "Zen mode", shortcut: "Ctrl+Shift+Z" },
    Command { id: "present", title: "Present as slides (split on ---)", shortcut: "F5" },
    Command { id: "format-bar", title: "Toggle formatting toolbar", shortcut: "" },
    Command { id: "wide", title: "Toggle wide layout", shortcut: "" },
    Command { id: "pin", title: "Pin / unpin this note", shortcut: "" },
    Command { id: "favorite", title: "Favorite / unfavorite this note", shortcut: "" },
    Command { id: "archive", title: "Archive / unarchive this note", shortcut: "" },
    Command { id: "duplicate", title: "Duplicate this note", shortcut: "" },
    Command { id: "merge", title: "Merge this note into\u{2026}", shortcut: "" },
    Command { id: "sticky", title: "Pop out as sticky note", shortcut: "" },
    Command { id: "delete", title: "Delete this note (move to Bin)", shortcut: "" },
    Command { id: "empty-bin", title: "Empty the Bin", shortcut: "" },
    Command { id: "timer", title: "Start focus timer (25 min)", shortcut: "" },
    Command { id: "timer-stop", title: "Stop focus timer", shortcut: "" },
    Command { id: "reminders", title: "Show reminders", shortcut: "" },
    Command { id: "calendar", title: "Calendar", shortcut: "" },
    Command { id: "stats", title: "Writing stats", shortcut: "" },
    Command { id: "tasks", title: "Tasks (all checkboxes)", shortcut: "Ctrl+T" },
    Command { id: "random", title: "Random note", shortcut: "" },
    Command { id: "on-this-day", title: "On this day", shortcut: "" },
    Command { id: "notebook", title: "Move this note to a notebook\u{2026}", shortcut: "" },
    Command { id: "emoji", title: "Set this note's emoji\u{2026}", shortcut: "" },
    Command { id: "save-search", title: "Save the current search", shortcut: "" },
    Command { id: "spell", title: "Check spelling", shortcut: "F7" },
    Command { id: "lookup", title: "Look up word (definition)", shortcut: "Ctrl+Shift+L" },
    Command { id: "history", title: "Version history of this note", shortcut: "" },
    Command { id: "sort-changed", title: "Sort notes: last changed first", shortcut: "" },
    Command { id: "sort-title", title: "Sort notes: by title (A to Z)", shortcut: "" },
    Command { id: "sort-created", title: "Sort notes: newest created first", shortcut: "" },
    Command { id: "settings", title: "Settings", shortcut: "Ctrl+," },
    Command { id: "theme-0", title: "Theme: System", shortcut: "" },
    Command { id: "theme-1", title: "Theme: Light", shortcut: "" },
    Command { id: "theme-2", title: "Theme: Dark", shortcut: "" },
    Command { id: "theme-3", title: "Theme: Ethereal", shortcut: "" },
    Command { id: "theme-4", title: "Theme: Zima Blue", shortcut: "" },
    Command { id: "theme-5", title: "Theme: Blue White", shortcut: "" },
    Command { id: "theme-7", title: "Theme: Custom (theme.json)", shortcut: "" },
    Command { id: "theme-8", title: "Theme: Sakura", shortcut: "" },
    Command { id: "theme-9", title: "Theme: Cyberpunk", shortcut: "" },
    Command { id: "theme-10", title: "Theme: Expedition 33", shortcut: "" },
    Command { id: "theme-11", title: "Theme: Hollow Knight", shortcut: "" },
    Command { id: "theme-12", title: "Theme: Hades", shortcut: "" },
    Command { id: "theme-13", title: "Theme: Elden Ring", shortcut: "" },
    Command { id: "theme-14", title: "Theme: Zelda", shortcut: "" },
    Command { id: "theme-15", title: "Theme: Stardew Valley", shortcut: "" },
    Command { id: "theme-16", title: "Theme: Celeste", shortcut: "" },
    Command { id: "theme-17", title: "Theme: Persona 5", shortcut: "" },
    Command { id: "theme-18", title: "Theme: Journey", shortcut: "" },
    Command { id: "theme-19", title: "Theme: Outer Wilds", shortcut: "" },
    Command { id: "theme-20", title: "Theme: Ghost of Tsushima", shortcut: "" },
    Command { id: "edit-theme", title: "Edit custom theme", shortcut: "" },
    Command { id: "font-note-default", title: "This note's font: same as app", shortcut: "" },
    Command { id: "font-note-Inter", title: "This note's font: Sans", shortcut: "" },
    Command { id: "font-note-Newsreader", title: "This note's font: Serif", shortcut: "" },
    Command { id: "font-note-Literata", title: "This note's font: Book", shortcut: "" },
    Command { id: "font-note-JetBrains Mono", title: "This note's font: Mono", shortcut: "" },
    Command { id: "width-note-default", title: "This note's width: same as app", shortcut: "" },
    Command { id: "width-note-wide", title: "This note's width: wide", shortcut: "" },
    Command { id: "width-note-normal", title: "This note's width: normal", shortcut: "" },
    Command { id: "export", title: "Export note as Markdown\u{2026}", shortcut: "" },
    Command { id: "export-html", title: "Export note as HTML\u{2026}", shortcut: "" },
    Command { id: "print", title: "Print / save as PDF", shortcut: "" },
    Command { id: "share-image", title: "Save note as image\u{2026}", shortcut: "" },
    Command { id: "lock", title: "Lock / unlock this note with a password", shortcut: "" },
    Command { id: "import", title: "Import from Zima\u{2026}", shortcut: "" },
    Command { id: "import-folder", title: "Import a folder of Markdown notes\u{2026}", shortcut: "" },
    Command { id: "export-all", title: "Export all notes as a zip\u{2026}", shortcut: "" },
    Command { id: "google-drive", title: "Sync notes with Google Drive", shortcut: "" },
    Command { id: "sync-folder", title: "Sync notes with a folder\u{2026} (OneDrive, Dropbox\u{2026})", shortcut: "" },
    Command { id: "backup", title: "Back up now", shortcut: "" },
    Command { id: "open-backups", title: "Open backups folder", shortcut: "" },
    Command { id: "open-folder", title: "Open notes folder", shortcut: "" },
    Command { id: "quit", title: "Quit Zima", shortcut: "" },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_have_icons() {
        assert_eq!(icon("new-note"), "plus");
        assert_eq!(icon("theme-9"), "settings");
        assert_eq!(icon("not-a-command"), "");
        // Most commands get a real icon, not the plain arrow.
        let with_icon = COMMANDS.iter().filter(|c| !icon(c.id).is_empty()).count();
        assert!(with_icon * 10 >= COMMANDS.len() * 9, "{with_icon} of {}", COMMANDS.len());
    }

    #[test]
    fn matching() {
        assert!(fuzzy_score("grc", "Groceries").is_some());
        assert!(fuzzy_score("xyz", "Groceries").is_none());
        // Word starts beat scattered letters.
        assert!(fuzzy_score("wp", "Weekend plan").unwrap() > fuzzy_score("wp", "swamp").unwrap());
        // Earlier and consecutive beats later.
        assert!(fuzzy_score("not", "Notes").unwrap() > fuzzy_score("not", "Big notes").unwrap());
        assert_eq!(fuzzy_score("", "anything"), Some(0));
        // Whole-word matches win over letters spread across words.
        assert!(
            fuzzy_score("notebook", "Move this note to a notebook").unwrap()
                > fuzzy_score("notebook", "This note's font: Book").unwrap()
        );
    }

    fn default_shortcut(id: &str) -> &'static str {
        COMMANDS.iter().find(|c| c.id == id).map(|c| c.shortcut).unwrap()
    }

    #[test]
    fn settings_window_opens_with_ctrl_comma() {
        assert_eq!(default_shortcut("settings"), "Ctrl+,");
        // The shortcuts page has no key of its own; it's reached from Settings or Ctrl+K.
        assert_eq!(default_shortcut("shortcuts"), "");
    }

    #[test]
    fn default_shortcuts_are_unique() {
        let mut seen: Vec<String> = Vec::new();
        for command in COMMANDS.iter().filter(|c| !c.shortcut.is_empty()) {
            let combo = command.shortcut.to_lowercase();
            assert!(!seen.contains(&combo), "{} is used twice", command.shortcut);
            seen.push(combo);
        }
    }

    #[test]
    fn command_ids_are_unique() {
        // Ids are the keys of saved custom shortcuts, so two commands must never share one.
        for (i, command) in COMMANDS.iter().enumerate() {
            assert!(COMMANDS[i + 1..].iter().all(|c| c.id != command.id), "{} is used twice", command.id);
        }
    }
}
