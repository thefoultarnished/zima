//! Automatic daily backups and "Back up now" (the zipping is in `crate::backup`).

use chrono::Local;

use super::{App, ToastAction, show_notification};
use crate::{backup, system};

impl App {
    /// Make today's backup if it isn't made yet. Runs on a background thread so typing never waits.
    pub fn auto_backup(&mut self) {
        let today = Local::now().date_naive();
        if self.backed_up_on == Some(today) {
            return;
        }
        self.backed_up_on = Some(today);
        self.flush();
        let data = self.store.root().to_path_buf();
        std::thread::spawn(move || {
            if let Err(e) = backup::folder().and_then(|dir| backup::back_up(&data, &dir, today, false)) {
                eprintln!("daily backup failed: {e}");
                show_notification("Zima couldn't back up your notes", &format!("It will try again tomorrow. ({e})"));
            }
        });
    }

    /// The "Back up now" command: back up right away, even if today's backup exists.
    pub fn backup_now(&mut self) {
        self.flush();
        let today = Local::now().date_naive();
        match backup::folder().and_then(|dir| backup::back_up(self.store.root(), &dir, today, true)) {
            Ok(_) => {
                self.backed_up_on = Some(today);
                self.toast_with("Notes backed up", false, vec![("Show".into(), ToastAction::ShowBackups)]);
            }
            Err(e) => self.toast(&format!("Backup failed: {e}"), true),
        }
    }

    pub fn open_backups_folder(&mut self) {
        match backup::folder().and_then(|dir| std::fs::create_dir_all(&dir).map(|()| dir)) {
            Ok(dir) => system::reveal(&dir),
            Err(e) => self.toast(&format!("Couldn't open the backups folder: {e}"), true),
        }
    }
}
