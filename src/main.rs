// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod backup;
mod calc;
mod clip;
mod cli;
mod commands;
mod emoji;
mod format;
mod highlight;
mod import;
mod instance;
mod markdown;
mod model;
mod palette;
mod reminders;
mod search;
mod spell;
mod store;
mod system;
mod tasks;
mod transfer;
mod window;

use std::path::PathBuf;

use slint::ComponentHandle;

slint::include_modules!();

/// Command line:
/// - `--hidden`: start in the tray only (used by "Launch at login").
/// - `--import <files…>`: import notes from the old Zima app (JSON export or .txt backups), then open.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // `zima add …`, `zima remind …`, `zima --help`: do it and exit without a window.
    if let Some(code) = cli::run(&args) {
        std::process::exit(code);
    }
    let hidden = args.iter().any(|a| a == system::HIDDEN_ARG);
    let import: Vec<PathBuf> = match args.iter().position(|a| a == "--import") {
        Some(i) => args[i + 1..].iter().take_while(|a| !a.starts_with("--")).map(PathBuf::from).collect(),
        None => Vec::new(),
    };

    let store = store::Store::open()?;

    // One Zima per data folder: a second launch shows the first one's window and exits.
    let key = instance::instance_key(store.root());
    let wait = if args.iter().any(|a| a == instance::RESTART_ARG) { instance::RESTART_WAIT } else { std::time::Duration::ZERO };
    let instance = match instance::claim(&key, wait) {
        instance::Claim::First(instance) => instance,
        instance::Claim::Taken => {
            if !import.is_empty() {
                // Show it right away: the process ends before a background notification would.
                let _ = system::notification()
                    .summary("Zima is already open")
                    .body("Quit it from the tray icon, then run the import again.")
                    .show();
            }
            // Launch at login starts hidden; if Zima is already running there's nothing to show.
            if !hidden {
                instance::signal_show(&key);
            }
            return Ok(());
        }
    };

    instance::mark_running();
    // Before any window, so the taskbar groups Zima with its Start-menu shortcut.
    system::set_app_id();
    window::select_backend(store.load_state().software_rendering)?;

    let ui = AppWindow::new()?;
    window::install(&ui);

    let app = app::App::load(store, &ui, &import);
    app::wire(&app, &ui);

    // Closing the window (close button or Alt+F4) hides it to the tray, or quits if
    // "Keep running in the tray" is off.
    ui.on_close_window({
        let ui = ui.as_weak();
        let app = app.clone();
        move || {
            if let Some(ui) = ui.upgrade() {
                let _ = ui.hide();
            }
            app.borrow_mut().window_closed();
        }
    });
    ui.window().on_close_requested({
        let app = app.clone();
        move || {
            app.borrow_mut().window_closed();
            slint::CloseRequestResponse::HideWindow
        }
    });

    // Another launch of Zima on this folder asked for the window.
    instance.on_show({
        let ui = ui.as_weak();
        move || {
            let _ = ui.upgrade_in_event_loop(|ui| {
                let _ = ui.show();
                ui.window().set_minimized(false);
                window::bring_to_front(&ui);
            });
        }
    });

    let tray = TrayIcon::new()?;
    let show_window = {
        let ui = ui.as_weak();
        move || {
            if let Some(ui) = ui.upgrade() {
                let _ = ui.show();
                ui.window().set_minimized(false);
            }
        }
    };
    tray.on_open(show_window.clone());
    tray.on_new_note({
        let app = app.clone();
        move || {
            show_window();
            app.borrow_mut().new_note();
        }
    });
    tray.on_quit({
        let app = app.clone();
        move || {
            app.borrow_mut().flush();
            let _ = slint::quit_event_loop();
        }
    });

    if !hidden {
        ui.show()?;
    }
    // Keep running with the window hidden, until "Quit" in the tray.
    slint::run_event_loop_until_quit()?;

    // Save anything still waiting on the debounce timer, and where the window and cursor are.
    app.borrow_mut().remember_window();
    app.borrow_mut().remember_cursor();
    app.borrow_mut().flush();
    drop(tray);
    Ok(())
}
