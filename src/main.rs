// Hide the console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod backup;
mod calc;
mod cli;
mod commands;
mod currency;
mod emoji;
mod format;
mod highlight;
mod history;
mod import;
mod instance;
mod live;
mod markdown;
mod model;
mod palette;
mod reminders;
mod search;
mod spell;
mod store;
mod system;
mod tasks;
mod timezones;
mod transfer;
mod window;

use std::path::PathBuf;

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

    // The window is built when Zima is opened, and dropped again when it goes to the tray.
    let app = app::App::load(store, &import);

    instance.on_show(app::show_from_any_thread);

    let tray = TrayIcon::new()?;
    tray.on_open({
        let app = app.clone();
        move || app::with_app(&app, |a| a.show_window_or_log(false))
    });
    tray.on_new_note({
        let app = app.clone();
        // The window first, so the title box exists to take focus.
        move || {
            app::with_app(&app, |a| {
                a.show_window_or_log(false);
                a.new_note();
            })
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
        app.borrow_mut().show_window(false)?;
    }
    // Keep running in the tray, with no window, until "Quit" in the tray.
    slint::run_event_loop_until_quit()?;

    // Save anything still waiting on the debounce timer, and where the window and cursor are.
    app.borrow_mut().remember_window();
    app.borrow_mut().remember_cursor();
    app.borrow_mut().flush();
    drop(tray);
    Ok(())
}
