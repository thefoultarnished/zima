//! Global quick capture: Ctrl+Alt+N from any app opens a small box to jot a note or an @remind.

use std::time::Duration;

use chrono::Local;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use slint::{ComponentHandle, Timer, TimerMode};
use slint::winit_030::WinitWindowAccessor;

use super::{App, show_notification, with_app};
use crate::model::{Note, now_ms};
use crate::{CaptureWindow, Theme, reminders};

impl App {
    /// Register Ctrl+Alt+N. The manager must stay alive, so the app keeps it.
    pub fn register_capture_hotkey(&mut self) {
        let manager = match GlobalHotKeyManager::new() {
            Ok(m) => m,
            Err(e) => {
                eprintln!("global hotkeys unavailable: {e}");
                return;
            }
        };
        let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyN);
        if let Err(e) = manager.register(hotkey) {
            eprintln!("couldn't register Ctrl+Alt+N: {e}");
            return;
        }
        self.hotkeys = Some(manager);

        // Hotkey events arrive on a channel; check it from the UI thread.
        let app = self.this.clone();
        let poll = Timer::default();
        poll.start(TimerMode::Repeated, Duration::from_millis(100), move || {
            while let Ok(event) = GlobalHotKeyEvent::receiver().try_recv() {
                if event.state == HotKeyState::Pressed {
                    if let Some(app) = app.upgrade() {
                        with_app(&app, |a| a.show_capture());
                    }
                }
            }
        });
        std::mem::forget(poll);
    }

    pub fn show_capture(&mut self) {
        if self.capture.is_none() {
            let Ok(window) = CaptureWindow::new() else { return };
            let app = self.this.clone();
            window.on_save(move |text| {
                if let Some(app) = app.upgrade() {
                    with_app(&app, move |a| a.save_capture(text.into()));
                }
            });
            let app = self.this.clone();
            window.on_cancel(move || {
                if let Some(app) = app.upgrade() {
                    with_app(&app, |a| a.close_capture());
                }
            });
            let app = self.this.clone();
            window.window().on_close_requested(move || {
                if let Some(app) = app.upgrade() {
                    with_app(&app, |a| a.close_capture());
                }
                slint::CloseRequestResponse::HideWindow
            });
            self.capture = Some(window);
        }
        let Some(window) = &self.capture else { return };
        if let Some(ui) = self.ui.upgrade() {
            super::custom_theme::copy_theme(&ui.global::<Theme>(), &window.global::<Theme>());
        }
        let _ = window.show();
        window.invoke_reset();
        // Centre near the top of the screen once the window has its real size, and take focus.
        let weak = window.as_weak();
        Timer::single_shot(Duration::from_millis(30), move || {
            let Some(window) = weak.upgrade() else { return };
            window.window().with_winit_window(|w| {
                if let Some(monitor) = w.current_monitor() {
                    let (screen, size) = (monitor.size(), w.outer_size());
                    let x = monitor.position().x + (screen.width as i32 - size.width as i32) / 2;
                    let y = monitor.position().y + screen.height as i32 / 4;
                    w.set_outer_position(slint::winit_030::winit::dpi::PhysicalPosition::new(x, y));
                }
                w.focus_window();
            });
        });
    }

    /// Close the quick-capture box and free its window (about 10 MB); Ctrl+Alt+N builds a new one.
    fn close_capture(&mut self) {
        if let Some(window) = self.capture.take() {
            let _ = window.hide();
            // Dropped after this turn, not inside one of its own callbacks.
            Timer::single_shot(Duration::ZERO, move || drop(window));
        }
    }

    fn save_capture(&mut self, text: String) {
        self.close_capture();
        let text = text.trim().to_string();
        if text.is_empty() {
            return;
        }
        // "@remind …" sets a reminder instead of a note.
        if let Some(result) = reminders::parse_command(&text, Local::now()) {
            match result {
                Ok(parsed) => {
                    let note = self.state.current.unwrap_or(0);
                    self.add_reminder(note, parsed.text.clone(), parsed.due.timestamp_millis(), parsed.repeat);
                    show_notification("Reminder set", &format!("{} \u{2014} {}", parsed.text, reminders::format_due(parsed.due, Local::now())));
                }
                Err(()) => show_notification("Couldn't set that reminder", "Try \u{201c}@remind me to stretch at 5pm\u{201d}."),
            }
            return;
        }
        // First line is the title (if there's more), the rest is the body.
        let (title, body) = match text.split_once('\n') {
            Some((first, rest)) if first.chars().count() <= 80 => (first.trim().to_string(), rest.trim_start().to_string()),
            _ => (String::new(), text.clone()),
        };
        let id = self.next_note_id();
        self.notes.push(Note { id, title, body, created: now_ms(), modified: now_ms(), ..Default::default() });
        self.dirty.insert(id);
        self.index_dirty = true;
        self.flush();
        self.refresh_lists();
        show_notification("Saved to Zima", text.lines().next().unwrap_or(""));
    }
}
