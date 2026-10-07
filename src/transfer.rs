//! Importing a folder of Markdown files (e.g. an Obsidian vault) and exporting everything as a zip.

use std::fs;
use std::io::Write;
use std::path::Path;

use chrono::{Local, TimeZone};

use crate::model::{Note, NoteId, now_ms};

/// Read every `.md` file under `folder`. Subfolders become notebooks; YAML front-matter `title:` and
/// `tags:` are used (tags are appended as #tags so they keep working).
pub fn import_markdown_folder(folder: &Path, next_id: &mut impl FnMut() -> NoteId) -> Vec<Note> {
    let mut notes = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                // Skip app folders like .obsidian, .trash and our own data.
                if !name.starts_with('.') && name != "notes" {
                    stack.push(path);
                }
                continue;
            }
            if !path.extension().is_some_and(|e| e.eq_ignore_ascii_case("md")) {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else { continue };
            let text = text.replace("\r\n", "\n");
            let (front, body) = split_front_matter(&text);
            let title = front_value(&front, "title").unwrap_or_else(|| path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default());
            let mut body = body.to_string();
            let tags = front_list(&front, "tags");
            if !tags.is_empty() {
                let line = tags.iter().map(|t| format!("#{}", t.replace(' ', "-"))).collect::<Vec<_>>().join(" ");
                body = format!("{}\n\n{line}\n", body.trim_end());
            }
            let modified = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or_else(now_ms, |d| d.as_millis() as i64);
            let relative = dir.strip_prefix(folder).ok().filter(|r| !r.as_os_str().is_empty());
            notes.push(Note {
                id: next_id(),
                title,
                body,
                modified,
                created: modified,
                notebook: relative.map(|r| r.to_string_lossy().replace('\\', "/")),
                ..Default::default()
            });
        }
    }
    notes
}

/// ("key: value" lines, body)
fn split_front_matter(text: &str) -> (String, &str) {
    if let Some(rest) = text.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---") {
            let body = rest[end + 4..].trim_start_matches(|c| c == '\n' || c == '-');
            return (rest[..end].to_string(), body);
        }
    }
    (String::new(), text)
}

fn front_value(front: &str, key: &str) -> Option<String> {
    front.lines().find_map(|line| {
        let (k, v) = line.split_once(':')?;
        (k.trim() == key && !v.trim().is_empty()).then(|| v.trim().trim_matches('"').trim_matches('\'').to_string())
    })
}

/// `tags: [a, b]`, `tags: a, b` or a YAML list on the following lines.
fn front_list(front: &str, key: &str) -> Vec<String> {
    let lines: Vec<&str> = front.lines().collect();
    let Some(i) = lines.iter().position(|l| l.split_once(':').is_some_and(|(k, _)| k.trim() == key)) else { return Vec::new() };
    let inline = lines[i].split_once(':').map(|(_, v)| v.trim()).unwrap_or("");
    let clean = |s: &str| s.trim().trim_matches('"').trim_matches('\'').trim_start_matches('#').to_string();
    if !inline.is_empty() {
        return inline.trim_matches(|c| c == '[' || c == ']').split(',').map(clean).filter(|s| !s.is_empty()).collect();
    }
    lines[i + 1..].iter().take_while(|l| l.trim_start().starts_with("- ")).map(|l| clean(&l.trim_start()[2..])).collect()
}

/// Write all notes into a zip of Markdown files, one folder per notebook, with front-matter.
pub fn export_zip(notes: &[&Note], path: &Path) -> std::io::Result<usize> {
    let file = fs::File::create(path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut used = std::collections::HashSet::new();
    let date = |ms: i64| Local.timestamp_millis_opt(ms).single().map(|d| d.to_rfc3339()).unwrap_or_default();
    for note in notes {
        let title = if note.title.trim().is_empty() { crate::app::display_title_of(note) } else { note.title.trim().to_string() };
        let safe: String = title.chars().map(|c| if "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
        let folder = note.notebook.as_deref().map(|n| format!("{}/", n.replace(['\\', ':'], "_"))).unwrap_or_default();
        // Unique file names.
        let mut name = format!("{folder}{safe}.md");
        let mut n = 2;
        while !used.insert(name.clone()) {
            name = format!("{folder}{safe} ({n}).md");
            n += 1;
        }
        let tags = crate::app::extract_tags(&note.body);
        let mut front = format!("---\ntitle: \"{}\"\ncreated: {}\nmodified: {}\n", title.replace('"', "'"), date(if note.created > 0 { note.created } else { note.modified }), date(note.modified));
        if !tags.is_empty() {
            front.push_str(&format!("tags: [{}]\n", tags.join(", ")));
        }
        if note.favorite {
            front.push_str("favorite: true\n");
        }
        if note.pinned {
            front.push_str("pinned: true\n");
        }
        front.push_str("---\n\n");
        zip.start_file(name, options).map_err(std::io::Error::other)?;
        zip.write_all(front.as_bytes())?;
        zip.write_all(note.body.as_bytes())?;
    }
    zip.finish().map_err(std::io::Error::other)?;
    Ok(notes.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_a_vault() {
        let dir = std::env::temp_dir().join(format!("zima-vault-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Projects")).unwrap();
        fs::create_dir_all(dir.join(".obsidian")).unwrap();
        fs::write(dir.join("Ideas.md"), "Plain note").unwrap();
        fs::write(dir.join("Projects").join("Atlas.md"), "---\ntitle: Atlas plan\ntags: [work, q3]\n---\n# Goals\nShip it").unwrap();
        fs::write(dir.join(".obsidian").join("config.md"), "skip me").unwrap();
        let mut id = 100;
        let mut next = || {
            id += 1;
            id
        };
        let mut notes = import_markdown_folder(&dir, &mut next);
        notes.sort_by_key(|n| n.title.clone());
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(notes.len(), 2);
        assert_eq!((notes[0].title.as_str(), notes[0].notebook.as_deref()), ("Atlas plan", Some("Projects")));
        assert!(notes[0].body.starts_with("# Goals") && notes[0].body.contains("#work #q3"), "{}", notes[0].body);
        assert_eq!((notes[1].title.as_str(), notes[1].notebook.as_deref()), ("Ideas", None));
    }

    #[test]
    fn exports_and_reads_back() {
        let path = std::env::temp_dir().join(format!("zima-export-{}.zip", std::process::id()));
        let a = Note { id: 1, title: "Same".into(), body: "one #x".into(), notebook: Some("Work".into()), ..Default::default() };
        let b = Note { id: 2, title: "Same".into(), body: "two".into(), notebook: Some("Work".into()), ..Default::default() };
        assert_eq!(export_zip(&[&a, &b], &path).unwrap(), 2);
        let mut archive = zip::ZipArchive::new(fs::File::open(&path).unwrap()).unwrap();
        let names: Vec<String> = (0..archive.len()).map(|i| archive.by_index(i).unwrap().name().to_string()).collect();
        let _ = fs::remove_file(&path);
        assert_eq!(names, vec!["Work/Same.md", "Work/Same (2).md"]);
    }
}
