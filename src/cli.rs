//! Command-line quick add, without opening the window:
//!
//! ```text
//! zima add "buy milk"                 a new note (first line = title when there are several)
//! zima remind "stretch in 20 min"     a reminder
//! zima --help
//! ```
//! A running Zima picks these up at its next hourly check, or when it's restarted.

use chrono::Local;

use crate::model::{Note, NoteId, Reminder, now_ms};
use crate::store::Store;

/// Handle a command-line action. `None` = not a CLI command (open the app normally).
pub fn run(args: &[String]) -> Option<i32> {
    let command = args.first()?.as_str();
    let text = args[1..].join(" ");
    let code = match command {
        "add" => add(&text),
        "remind" => remind(&text),
        "--help" | "-h" | "help" => {
            println!("{}", HELP);
            Ok(())
        }
        _ => return None,
    };
    Some(match code {
        Ok(()) => 0,
        Err(message) => {
            eprintln!("zima: {message}");
            1
        }
    })
}

const HELP: &str = "Zima: a calm notes app.

Usage:
  zima                         open Zima
  zima add \"text\"              add a note (first line is the title if there are several)
  zima remind \"what when\"      add a reminder, e.g. \"stretch in 20 min\", \"call Sam tomorrow at 5\"
  zima --import <files>        import notes from the old Zima app (JSON export or .txt backups)
  zima --hidden                start in the tray";

fn add(text: &str) -> Result<(), String> {
    let text = text.replace("\\n", "\n");
    let text = text.trim();
    if text.is_empty() {
        return Err("nothing to add. Try: zima add \"buy milk\"".into());
    }
    let store = Store::open().map_err(|e| e.to_string())?;
    let mut notes = store.load_notes();
    let id = next_id(notes.iter().map(|n| n.id));
    let (title, body) = match text.split_once('\n') {
        Some((first, rest)) if first.chars().count() <= 80 => (first.trim().to_string(), rest.trim_start().to_string()),
        _ => (String::new(), text.to_string()),
    };
    let note = Note { id, title, body, created: now_ms(), modified: now_ms(), ..Default::default() };
    store.save_body(&note).map_err(|e| e.to_string())?;
    notes.push(note);
    store.save_index(&notes).map_err(|e| e.to_string())?;
    println!("Added to Zima.");
    Ok(())
}

fn remind(text: &str) -> Result<(), String> {
    let now = Local::now();
    let parsed = crate::reminders::parse(text, now).ok_or("couldn't tell when. Try: zima remind \"stretch in 20 min\"")?;
    let store = Store::open().map_err(|e| e.to_string())?;
    let mut reminders = store.load_reminders();
    let id = next_id(reminders.iter().map(|r| r.id)) as u64;
    reminders.push(Reminder { id, note_id: 0, text: parsed.text.clone(), due: parsed.due.timestamp_millis(), repeat: parsed.repeat.clone() });
    store.save_reminders(&reminders).map_err(|e| e.to_string())?;
    let repeat = parsed.repeat.map(|r| format!(" ({})", r.describe())).unwrap_or_default();
    println!("Reminder set: {}: {}{repeat}", parsed.text, crate::reminders::format_due(parsed.due, now));
    Ok(())
}

fn next_id(existing: impl Iterator<Item = NoteId>) -> NoteId {
    (now_ms() as NoteId).max(existing.max().unwrap_or(0) + 1)
}
