//! Placeholders for features built in later batches; each is replaced as it lands.

use super::App;

impl App {
    fn not_yet(&mut self) {
        self.toast("Coming soon", false);
    }


    pub fn open_history(&mut self) {
        self.not_yet();
    }

    pub fn export_html(&mut self) {
        self.not_yet();
    }

    pub fn print_note(&mut self) {
        self.not_yet();
    }

    pub fn share_image(&mut self) {
        self.not_yet();
    }

    pub fn toggle_lock(&mut self) {
        self.not_yet();
    }
}
