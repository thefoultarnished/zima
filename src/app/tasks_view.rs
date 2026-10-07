//! Tasks view: every checkbox from every note, grouped by when it's due.

use chrono::Local;
use slint::{ModelRc, VecModel};

use super::{App, display_title};
use crate::model::NoteId;
use crate::{TaskRow, tasks};

impl App {
    pub fn open_tasks(&mut self) {
        self.refresh_tasks();
        if let Some(ui) = self.ui.upgrade() {
            ui.set_tasks_open(true);
        }
    }

    /// Rebuild the Tasks view (and the sidebar's open-task count).
    pub fn refresh_tasks(&self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let now = Local::now();
        let today = now.date_naive();
        let show_done = ui.get_tasks_show_done();

        // (group order, due, row)
        let mut rows: Vec<(u8, Option<chrono::NaiveDate>, TaskRow)> = Vec::new();
        let mut open = 0;
        for note in self.notes.iter().filter(|n| n.is_listed()) {
            for task in tasks::extract(&note.body, now) {
                if !task.done {
                    open += 1;
                }
                if task.done && !show_done {
                    continue;
                }
                let (group, label) = match (task.done, task.due) {
                    (true, _) => (4, String::new()),
                    (false, Some(due)) if due < today => (0, format!("Overdue \u{00b7} {}", due.format("%a %-d %b"))),
                    (false, Some(due)) if due == today => (1, "Today".into()),
                    (false, Some(due)) if due == today.succ_opt().unwrap_or(due) => (2, "Tomorrow".into()),
                    (false, Some(due)) => (2, due.format("%a %-d %b").to_string()),
                    (false, None) => (3, String::new()),
                };
                let label = match &task.every {
                    Some(every) if label.is_empty() => format!("every {every}"),
                    Some(every) => format!("{label} \u{00b7} every {every}"),
                    None => label,
                };
                rows.push((
                    group,
                    task.due,
                    TaskRow {
                        note_id: note.id.to_string().into(),
                        note_title: display_title(note).into(),
                        line: task.line as i32,
                        text: task.text.into(),
                        done: task.done,
                        due: label.into(),
                        overdue: group == 0,
                        header: Default::default(),
                    },
                ));
            }
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
            ui.invoke_set_body_selection(offset as i32, offset as i32);
        }
    }
}
