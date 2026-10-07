//! Pop-out sticky notes: a note in its own small always-on-top window, kept in sync with the editor.

use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;

use super::{App, display_title, with_app};
use crate::model::{NoteId, now_ms};
use crate::{StickyWindow, Theme};

pub struct Sticky {
    pub note: NoteId,
    pub window: StickyWindow,
}

impl App {
    pub fn open_sticky(&mut self, id: NoteId) {
        if let Some(sticky) = self.stickies.iter().find(|s| s.note == id) {
            let _ = sticky.window.show();
            return;
        }
        let Some(note) = self.find(id).cloned() else { return };
        let Ok(window) = StickyWindow::new() else { return };
        window.set_note_title(display_title(&note).into());
        window.set_body(note.body.as_str().into());
        window.set_note_color(note.color.map_or(-1, |c| c as i32));
        self.sync_theme_to_sticky(&window);

        let app = self.this.clone();
        window.on_body_edited(move |text| {
            if let Some(app) = app.upgrade() {
                with_app(&app, move |a| a.set_body_of(id, text.into()));
            }
        });
        let app = self.this.clone();
        window.on_close_sticky(move || {
            if let Some(app) = app.upgrade() {
                with_app(&app, move |a| a.close_sticky(id));
            }
        });
        let app = self.this.clone();
        window.on_open_in_app(move || {
            if let Some(app) = app.upgrade() {
                with_app(&app, move |a| {
                    a.open(id);
                    if let Some(ui) = a.ui.upgrade() {
                        let _ = ui.show();
                        ui.window().set_minimized(false);
                    }
                });
            }
        });
        let weak = window.as_weak();
        window.on_drag(move || {
            if let Some(w) = weak.upgrade() {
                w.window().with_winit_window(|w| {
                    let _ = w.drag_window();
                });
            }
        });
        let weak = window.as_weak();
        window.on_resize_window(move |_| {
            if let Some(w) = weak.upgrade() {
                w.window().with_winit_window(|w| {
                    let _ = w.drag_resize_window(slint::winit_030::winit::window::ResizeDirection::SouthEast);
                });
            }
        });

        let _ = window.show();
        self.stickies.push(Sticky { note: id, window });
        if !self.state.stickies.contains(&id) {
            self.state.stickies.push(id);
            self.save_state();
        }
    }

    pub fn close_sticky(&mut self, id: NoteId) {
        if let Some(index) = self.stickies.iter().position(|s| s.note == id) {
            let sticky = self.stickies.remove(index);
            let _ = sticky.window.hide();
        }
        self.state.stickies.retain(|&s| s != id);
        self.save_state();
    }

    /// Reopen the sticky notes that were open last time.
    pub fn restore_stickies(&mut self) {
        let ids = self.state.stickies.clone();
        self.state.stickies.clear();
        for id in ids {
            if self.find(id).is_some_and(|n| n.is_live()) {
                self.open_sticky(id);
            }
        }
        self.save_state();
    }

    /// Edit from a sticky window (which may not be the note open in the editor).
    pub fn set_body_of(&mut self, id: NoteId, body: String) {
        let Some(note) = self.find_mut(id) else { return };
        let before_words = note.body.split_whitespace().count();
        note.body = body;
        note.modified = now_ms();
        let after_words = note.body.split_whitespace().count();
        self.record_words(before_words, after_words);
        self.dirty.insert(id);
        self.index_dirty = true;
        self.schedule_save();
        if self.state.current == Some(id) {
            if let (Some(ui), Some(note)) = (self.ui.upgrade(), self.find(id)) {
                ui.set_note_body(note.body.as_str().into());
            }
            self.after_body_change();
        }
    }

    /// The editor changed a note: update its sticky window, if open.
    pub fn sync_sticky(&self, id: NoteId) {
        if let (Some(sticky), Some(note)) = (self.stickies.iter().find(|s| s.note == id), self.find(id)) {
            if sticky.window.get_body() != note.body.as_str() {
                sticky.window.set_body(note.body.as_str().into());
            }
            sticky.window.set_note_title(display_title(note).into());
            sticky.window.set_note_color(note.color.map_or(-1, |c| c as i32));
        }
    }

    pub fn sync_theme_to_stickies(&self) {
        for sticky in &self.stickies {
            self.sync_theme_to_sticky(&sticky.window);
        }
    }

    fn sync_theme_to_sticky(&self, window: &StickyWindow) {
        let Some(ui) = self.ui.upgrade() else { return };
        super::custom_theme::copy_theme(&ui.global::<Theme>(), &window.global::<Theme>());
    }
}
