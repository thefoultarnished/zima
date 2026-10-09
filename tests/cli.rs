//! End-to-end tests for the command line (`zima add`, `zima remind`, `zima --help`): they run the
//! real program against a throwaway data folder and check what it wrote to disk.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

/// A fresh, empty data folder, deleted when the test ends.
struct DataDir(PathBuf);

impl DataDir {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("zima-cli-test-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn zima(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_zima"))
            .args(args)
            // Never touch the real notes.
            .env("ZIMA_DATA_DIR", self.path())
            .output()
            .expect("run zima")
    }

    fn json(&self, file: &str) -> Value {
        serde_json::from_str(&std::fs::read_to_string(self.path().join(file)).unwrap()).unwrap()
    }
}

impl Drop for DataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn help_prints_usage() {
    let data = DataDir::new("help");
    let output = data.zima(&["--help"]);
    assert!(output.status.success());
    assert!(stdout(&output).contains("zima add"));
    assert!(stdout(&output).contains("zima remind"));
    // Help doesn't create any data.
    assert!(!data.path().join("index.json").exists());
}

#[test]
fn add_saves_a_note_and_its_index_entry() {
    let data = DataDir::new("add");
    let output = data.zima(&["add", "buy milk"]);
    assert!(output.status.success(), "{output:?}");

    let index = data.json("index.json");
    let notes = index.as_array().unwrap();
    assert_eq!(notes.len(), 1);
    let id = notes[0]["id"].as_i64().unwrap();
    // A single line has no separate title; it's all body.
    assert_eq!(notes[0]["title"], "");
    let body = std::fs::read_to_string(data.path().join("notes").join(format!("{id}.md"))).unwrap();
    assert_eq!(body, "buy milk");
}

#[test]
fn add_with_several_lines_uses_the_first_as_title() {
    let data = DataDir::new("add-title");
    let output = data.zima(&["add", "Groceries\\nmilk\\neggs"]);
    assert!(output.status.success(), "{output:?}");

    let notes = data.json("index.json");
    assert_eq!(notes[0]["title"], "Groceries");
    let id = notes[0]["id"].as_i64().unwrap();
    let body = std::fs::read_to_string(data.path().join("notes").join(format!("{id}.md"))).unwrap();
    assert_eq!(body, "milk\neggs");
}

#[test]
fn add_twice_keeps_both_notes() {
    let data = DataDir::new("add-twice");
    assert!(data.zima(&["add", "first"]).status.success());
    assert!(data.zima(&["add", "second"]).status.success());

    let notes = data.json("index.json");
    let notes = notes.as_array().unwrap();
    assert_eq!(notes.len(), 2);
    assert_ne!(notes[0]["id"], notes[1]["id"]);
}

#[test]
fn add_with_no_text_fails_and_writes_nothing() {
    let data = DataDir::new("add-empty");
    let output = data.zima(&["add", "   "]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("nothing to add"));
    assert!(!data.path().join("index.json").exists());
}

#[test]
fn remind_saves_the_reminder_with_its_due_time() {
    let data = DataDir::new("remind");
    let before = now_ms();
    let output = data.zima(&["remind", "me to eat in 2 mins"]);
    let after = now_ms();
    assert!(output.status.success(), "{output:?}");
    assert!(stdout(&output).contains("Reminder set: eat"));

    let reminders = data.json("reminders.json");
    let reminders = reminders.as_array().unwrap();
    assert_eq!(reminders.len(), 1);
    assert_eq!(reminders[0]["text"], "eat");
    assert!(reminders[0]["repeat"].is_null());
    // Two minutes from when it ran (the parser works in whole seconds).
    let due = reminders[0]["due"].as_i64().unwrap();
    assert!(due >= before + 120_000 - 1_000 && due <= after + 120_000 + 1_000, "due {due}, ran {before}..{after}");
}

#[test]
fn remind_with_a_repeat_saves_the_repeat() {
    let data = DataDir::new("remind-repeat");
    let output = data.zima(&["remind", "vitamins every day at 8am"]);
    assert!(output.status.success(), "{output:?}");

    let reminders = data.json("reminders.json");
    assert_eq!(reminders[0]["text"], "vitamins");
    assert_eq!(reminders[0]["repeat"]["kind"], "daily");
    assert!(reminders[0]["due"].as_i64().unwrap() > now_ms());
}

#[test]
fn remind_keeps_earlier_reminders() {
    let data = DataDir::new("remind-twice");
    assert!(data.zima(&["remind", "stretch in 20 min"]).status.success());
    assert!(data.zima(&["remind", "drink water in 1 hour"]).status.success());

    let reminders = data.json("reminders.json");
    let texts: Vec<&str> = reminders.as_array().unwrap().iter().map(|r| r["text"].as_str().unwrap()).collect();
    assert_eq!(texts, ["stretch", "drink water"]);
}

#[test]
fn remind_without_a_time_fails_and_writes_nothing() {
    let data = DataDir::new("remind-bad");
    let output = data.zima(&["remind", "eat something"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("couldn't tell when"));
    assert!(!data.path().join("reminders.json").exists());
}

#[test]
fn add_keeps_existing_note_emoji() {
    let data = DataDir::new("add-emoji");
    // One note saved before emoji existed (no "emoji" key) and one with an emoji.
    std::fs::write(data.path().join("index.json"), r#"[{"id":1,"title":"old","modified":5},{"id":2,"title":"party","emoji":"🎉","modified":6}]"#).unwrap();
    let output = data.zima(&["add", "new"]);
    assert!(output.status.success(), "{output:?}");

    let index = data.json("index.json");
    let notes = index.as_array().unwrap();
    assert_eq!(notes.len(), 3);
    let by_id = |id: i64| notes.iter().find(|n| n["id"] == id).unwrap();
    assert_eq!(by_id(2)["emoji"], "🎉");
    assert!(by_id(1)["emoji"].is_null());
}

#[test]
fn add_with_a_damaged_note_list_keeps_every_note() {
    let data = DataDir::new("damaged-index");
    std::fs::create_dir_all(data.path().join("notes")).unwrap();
    std::fs::write(data.path().join("notes").join("1.md"), "old note").unwrap();
    std::fs::write(data.path().join("index.json"), "[{\"id\": 1, \"ti").unwrap();

    assert!(data.zima(&["add", "buy milk"]).status.success());

    // The old note is still listed next to the new one, and the damaged list was kept.
    let index = data.json("index.json");
    let ids: Vec<u64> = index.as_array().unwrap().iter().map(|n| n["id"].as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&1));
    let kept = std::fs::read_dir(data.path())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("index.json.damaged-"))
        .count();
    assert_eq!(kept, 1);
}
