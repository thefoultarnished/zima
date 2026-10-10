//! Tasks view: every checkbox from every note, grouped by when it's due.

use chrono::{Local, NaiveDate};
use slint::{ModelRc, VecModel};

use super::{App, display_title};
use crate::model::{NoteId, QuickTask, now_ms};
use crate::{TaskRow, tasks};

impl App {
    pub fn open_tasks(&mut self) {
        self.refresh_tasks();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_tasks_open(true);
        }
    }

    /// Rebuild the Tasks view (and the sidebar's open-task count): checkboxes from notes, and `@due` tasks.
    pub fn refresh_tasks(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let today = Local::now().date_naive();
        let show_done = self.state.tasks_show_done;

        // (group order, due, row)
        let mut rows: Vec<(u8, Option<NaiveDate>, TaskRow)> = Vec::new();
        let mut open = 0;
        let mut add = |done: bool, due: Option<NaiveDate>, every: Option<&str>, row: TaskRow| {
            if !done {
                open += 1;
            }
            if done && !show_done {
                return;
            }
            let (group, label) = when(done, due, every, today);
            rows.push((group, due, TaskRow { due: label.into(), overdue: group == 0, done, ..row }));
        };
        for note in self.notes.iter().filter(|n| n.is_listed()) {
            for task in self.summary(note).tasks.iter() {
                let row = TaskRow {
                    note_id: note.id.to_string().into(),
                    note_title: display_title(note).into(),
                    line: task.line as i32,
                    text: task.text.as_str().into(),
                    ..Default::default()
                };
                add(task.done, task.due, task.every.as_deref(), row);
            }
        }
        for task in &self.quick_tasks {
            let row = TaskRow { task_id: task.id.to_string().into(), text: task.text.as_str().into(), ..Default::default() };
            add(task.done, quick_due(task), None, row);
        }
        rows.sort_by_key(|(group, due, _)| (*group, *due));

        // Group headings between the rows.
        let names = ["Overdue", "Today", "Upcoming", "No date", "Done"];
        let mut out: Vec<TaskRow> = Vec::new();
        let mut last_group = None;
        for (group, _, row) in rows {
            if last_group != Some(group) {
                out.push(TaskRow { header: names[group as usize].into(), ..Default::default() });
                last_group = Some(group);
            }
            out.push(row);
        }
        ui.set_task_rows(ModelRc::new(VecModel::from(out)));
        ui.set_open_task_count(open);
    }

    /// "Show done" in the Tasks view, remembered between runs.
    pub fn set_tasks_show_done(&mut self, on: bool) {
        self.state.tasks_show_done = on;
        self.save_state();
        self.refresh_tasks();
    }

    /// From an `@due` line: a task kept in `tasks.json`, not in the note.
    pub fn add_quick_task(&mut self, text: String, due: Option<NaiveDate>) {
        let id = (now_ms() as u64).max(self.quick_tasks.iter().map(|t| t.id + 1).max().unwrap_or(0));
        let when = match due {
            Some(day) => format!(", due {}", day_label(day, Local::now().date_naive()).to_lowercase()),
            None => String::new(),
        };
        self.toast(&format!("Task added: {text}{when}. See it in Tasks (Ctrl+T)."), false);
        self.quick_tasks.push(QuickTask { id, text, due: due.map(|d| d.format("%Y-%m-%d").to_string()), done: false, created: now_ms() });
        self.save_quick_tasks();
    }

    pub fn toggle_quick_task(&mut self, id: &str) {
        let Some(task) = id.parse::<u64>().ok().and_then(|id| self.quick_tasks.iter_mut().find(|t| t.id == id)) else { return };
        task.done = !task.done;
        self.save_quick_tasks();
    }

    pub fn delete_quick_task(&mut self, id: &str) {
        let Ok(id) = id.parse::<u64>() else { return };
        self.quick_tasks.retain(|t| t.id != id);
        self.save_quick_tasks();
    }

    fn save_quick_tasks(&mut self) {
        if let Err(e) = self.store.save_tasks(&self.quick_tasks) {
            eprintln!("failed to save tasks: {e}");
            self.toast("Couldn't save your tasks", true);
        }
        self.last_tasks_write = self.store.tasks_modified();
        self.refresh_tasks();
    }

    /// Tick a task from the Tasks view (in any note).
    pub fn toggle_task_in(&mut self, note_id: NoteId, line: usize) {
        let Some(body) = self.find(note_id).map(|n| n.body.clone()) else { return };
        let Some(new_body) = tasks::toggle(&body, line, Local::now()) else { return };
        if self.state.current == Some(note_id) {
            self.replace_body(new_body, None);
        } else {
            self.set_body_of(note_id, new_body);
        }
        self.refresh_tasks();
        self.refresh_lists();
    }

    /// Jump to a task's line in its note.
    pub fn open_task(&mut self, note_id: NoteId, line: usize) {
        self.open(note_id);
        let Some(body) = self.current().map(|n| n.body.clone()) else { return };
        let offset: usize = body.split('\n').take(line).map(|l| l.len() + 1).sum();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_tasks_open(false);
            if ui.get_view_mode() == 2 {
                ui.set_view_mode(1);
                self.set_view_mode(1);
            }
        }
        self.show_selection(offset, offset, true);
    }
}

fn quick_due(task: &QuickTask) -> Option<NaiveDate> {
    task.due.as_deref().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
}

/// "Today", "Tomorrow" or "Fri 9 Oct".
fn day_label(day: NaiveDate, today: NaiveDate) -> String {
    if day == today {
        "Today".into()
    } else if Some(day) == today.succ_opt() {
        "Tomorrow".into()
    } else {
        day.format("%a %-d %b").to_string()
    }
}

/// Which group a task goes in (0 Overdue, 1 Today, 2 Upcoming, 3 No date, 4 Done) and its date chip.
fn when(done: bool, due: Option<NaiveDate>, every: Option<&str>, today: NaiveDate) -> (u8, String) {
    let (group, label) = match (done, due) {
        (true, _) => (4, String::new()),
        (false, Some(due)) if due < today => (0, format!("Overdue \u{00b7} {}", due.format("%a %-d %b"))),
        (false, Some(due)) if due == today => (1, "Today".into()),
        (false, Some(due)) => (2, day_label(due, today)),
        (false, None) => (3, String::new()),
    };
    let label = match every {
        Some(every) if label.is_empty() => format!("every {every}"),
        Some(every) => format!("{label} \u{00b7} every {every}"),
        None => label,
    };
    (group, label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 10, d).unwrap()
    }

    #[test]
    fn groups_by_deadline() {
        let today = day(7);
        assert_eq!(when(false, Some(day(5)), None, today), (0, "Overdue \u{00b7} Mon 5 Oct".into()));
        assert_eq!(when(false, Some(day(7)), None, today), (1, "Today".into()));
        assert_eq!(when(false, Some(day(8)), None, today), (2, "Tomorrow".into()));
        assert_eq!(when(false, Some(day(9)), None, today), (2, "Fri 9 Oct".into()));
        assert_eq!(when(false, None, None, today), (3, String::new()));
    }

    #[test]
    fn done_and_repeating() {
        let today = day(7);
        // Done goes last whatever its date.
        assert_eq!(when(true, Some(day(5)), None, today), (4, String::new()));
        assert_eq!(when(false, None, Some("monday"), today), (3, "every monday".into()));
        assert_eq!(when(false, Some(day(7)), Some("day"), today), (1, "Today \u{00b7} every day".into()));
    }

    #[test]
    fn quick_task_dates() {
        let task = |due: Option<&str>| QuickTask { id: 1, text: "x".into(), due: due.map(Into::into), done: false, created: 0 };
        assert_eq!(quick_due(&task(Some("2026-10-09"))), Some(day(9)));
        assert_eq!(quick_due(&task(None)), None);
        // A date mangled by hand in tasks.json just means no deadline.
        assert_eq!(quick_due(&task(Some("soon"))), None);
    }
}
