//! Import notes from Zima.
//!
//! Accepts either:
//! - a JSON export of Zima's localStorage: `{"notes": "<json>", "deleted": "<json>"}` (values may also be arrays), or
//! - Zima's `.txt` backups (`<title>_<id>.txt`).

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::model::{Note, NoteId, now_ms};

/// Read every path and return the notes that aren't already present (by id).
pub fn import(paths: &[impl AsRef<Path>], existing: &[Note]) -> Result<Vec<Note>, String> {
    let mut taken: HashSet<NoteId> = existing.iter().map(|n| n.id).collect();
    let mut next_id = (now_ms() as NoteId).max(taken.iter().max().map_or(0, |m| m + 1));
    let mut fresh_id = |taken: &mut HashSet<NoteId>| {
        while taken.contains(&next_id) {
            next_id += 1;
        }
        taken.insert(next_id);
        next_id
    };

    let mut out = Vec::new();
    for path in paths {
        let path = path.as_ref();
        let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let is_json = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("json"));
        if is_json {
            let value: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
            let (notes, deleted) = match &value {
                Value::Array(_) => (value.clone(), Value::Null),
                Value::Object(map) => (
                    map.get("notes").cloned().unwrap_or(Value::Null),
                    map.get("deleted").or_else(|| map.get("deletedNotes")).cloned().unwrap_or(Value::Null),
                ),
                _ => return Err(format!("{}: not a Zima export", path.display())),
            };
            for (list, in_bin) in [(notes, false), (deleted, true)] {
                for item in as_array(list) {
                    if let Some(mut note) = zima_note(&item, in_bin) {
                        if taken.contains(&note.id) {
                            continue;
                        }
                        if note.id == 0 {
                            note.id = fresh_id(&mut taken);
                        } else {
                            taken.insert(note.id);
                        }
                        out.push(note);
                    }
                }
            }
        } else {
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Imported");
            // "<title>_<id>": drop the trailing tab id.
            let title = match stem.rsplit_once('_') {
                Some((title, id)) if id.chars().all(|c| c.is_ascii_digit()) => title,
                _ => stem,
            };
            let modified = fs::metadata(path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or_else(now_ms, |d| d.as_millis() as i64);
            out.push(Note {
                id: fresh_id(&mut taken),
                title: title.trim().to_string(),
                body: text.replace("\r\n", "\n"),
                modified,
                created: modified,
                ..Default::default()
            });
        }
    }
    Ok(out)
}

/// localStorage values are JSON strings holding JSON; accept both that and plain arrays.
fn as_array(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        Value::String(s) => match serde_json::from_str(&s) {
            Ok(Value::Array(items)) => items,
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

fn zima_note(item: &Value, in_bin: bool) -> Option<Note> {
    let obj = item.as_object()?;
    let str_field = |key: &str| obj.get(key).and_then(Value::as_str).unwrap_or("").trim().to_string();
    // Zima had a tab label (`title`, usually an automatic "Note 3") and a real title (`noteTitle`).
    // Skip automatic labels so the sidebar shows the note's first line instead.
    let label = str_field("title");
    let automatic = label.strip_prefix("Note ").is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()));
    let title = Some(str_field("noteTitle"))
        .filter(|t| !t.is_empty())
        .unwrap_or(if automatic { String::new() } else { label });
    let html = str_field("content");
    let body = htmd::convert(&html).unwrap_or(html).trim().to_string();
    let modified = obj.get("lastModified").and_then(Value::as_i64).unwrap_or_else(now_ms);
    Some(Note {
        id: obj.get("id").and_then(Value::as_u64).unwrap_or(0),
        title,
        body,
        modified,
        created: obj.get("id").and_then(Value::as_i64).filter(|id| *id > 1_000_000_000_000).unwrap_or(modified),
        favorite: obj.get("isFavorite").and_then(Value::as_bool).unwrap_or(false),
        deleted_at: in_bin.then(now_ms),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_localstorage_export() {
        let dir = std::env::temp_dir().join(format!("zima-import-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let notes = r#"[{"id":5,"title":"Note 1","noteTitle":"Shopping","content":"<div><b>Eggs</b></div><div>Milk</div>","lastModified":1000,"isFavorite":true},
                        {"id":6,"title":"Note 2","noteTitle":"","content":"plain","lastModified":2000,"isFavorite":false}]"#;
        let export = serde_json::json!({ "notes": notes, "deleted": "[{\"id\":7,\"title\":\"Old\",\"content\":\"x\"}]" });
        let json = dir.join("zima-export.json");
        fs::write(&json, export.to_string()).unwrap();
        let txt = dir.join("Groceries_3.txt");
        fs::write(&txt, "a\r\nb").unwrap();

        let existing = [Note { id: 6, ..Default::default() }];
        let imported = import(&[json, txt], &existing).unwrap();
        fs::remove_dir_all(&dir).ok();

        assert_eq!(imported.len(), 3); // id 6 already exists
        assert_eq!((imported[0].id, imported[0].title.as_str(), imported[0].favorite), (5, "Shopping", true));
        assert!(imported[0].body.contains("**Eggs**") && imported[0].body.contains("Milk"), "{}", imported[0].body);
        assert_eq!((imported[1].id, imported[1].deleted_at.is_some()), (7, true));
        assert_eq!((imported[2].title.as_str(), imported[2].body.as_str()), ("Groceries", "a\nb"));
    }
}
