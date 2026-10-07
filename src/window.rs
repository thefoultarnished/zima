//! Frameless window: backend setup, title bar buttons, drag, double-click maximize, edge resize.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use slint::platform::WindowEvent;
use slint::winit_030::{WinitWindowAccessor, winit};
use winit::window::ResizeDirection;

use crate::AppWindow;

const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// Must run before the first window is created. `software`: draw with the CPU (Settings → Low-memory rendering).
pub fn select_backend(software: bool) -> Result<(), slint::PlatformError> {
    // The custom title bar needs the winit backend for native drag and resize.
    let selector = slint::BackendSelector::new().backend_name("winit".into());
    // SLINT_BACKEND (e.g. winit-software), if set, wins over the setting.
    let selector = if software && std::env::var_os("SLINT_BACKEND").is_none() {
        selector.renderer_name("software".into())
    } else {
        selector
    };
    selector.with_winit_window_attributes_hook(window_attributes).select()
}

// A frameless window loses the native shadow and rounded corners; ask Windows to keep them.
// Transparent, so the transparency setting can let the Mica backdrop show through.
fn window_attributes(attributes: winit::window::WindowAttributes) -> winit::window::WindowAttributes {
    let attributes = attributes.with_transparent(true);
    #[cfg(windows)]
    let attributes = {
        use winit::platform::windows::{CornerPreference, WindowAttributesExtWindows};
        attributes
            .with_undecorated_shadow(true)
            .with_corner_preference(CornerPreference::Round)
    };
    attributes
}

/// Apply the Windows 11 Mica backdrop (light or dark). Returns false if the window doesn't exist yet.
pub fn apply_mica(ui: &AppWindow, dark: bool) -> bool {
    ui.window()
        .with_winit_window(|w| {
            #[cfg(windows)]
            if let Err(e) = window_vibrancy::apply_mica(w, Some(dark)) {
                eprintln!("Mica unavailable: {e}");
            }
            let _ = (w, dark);
        })
        .is_some()
}

/// Title bar buttons, drag and resize. (Closing is handled in main: it hides to the tray.)
pub fn install(ui: &AppWindow) {
    ui.on_minimize_window({
        let ui = ui.as_weak();
        move || ui.unwrap().window().set_minimized(true)
    });

    ui.on_maximize_window({
        let ui = ui.as_weak();
        move || toggle_maximized(&ui.unwrap())
    });

    let last_press = Rc::new(Cell::new(None::<Instant>));
    ui.on_drag_window({
        let ui = ui.as_weak();
        move || {
            let ui = ui.unwrap();
            let now = Instant::now();
            // The OS takes over the mouse during a drag, so detect double-clicks here.
            if last_press.get().is_some_and(|t| now - t < DOUBLE_CLICK) {
                last_press.set(None);
                toggle_maximized(&ui);
            } else {
                last_press.set(Some(now));
                ui.window().with_winit_window(|w| {
                    let _ = w.drag_window();
                });
            }
        }
    });

    ui.on_resize_window({
        let ui = ui.as_weak();
        move |direction| {
            let ui = ui.unwrap();
            if ui.window().is_maximized() {
                return;
            }
            let direction = match direction.as_str() {
                "n" => ResizeDirection::North,
                "s" => ResizeDirection::South,
                "e" => ResizeDirection::East,
                "w" => ResizeDirection::West,
                "ne" => ResizeDirection::NorthEast,
                "nw" => ResizeDirection::NorthWest,
                "se" => ResizeDirection::SouthEast,
                "sw" => ResizeDirection::SouthWest,
                _ => return,
            };
            ui.window().with_winit_window(|w| {
                let _ = w.drag_resize_window(direction);
            });
        }
    });
}

fn toggle_maximized(ui: &AppWindow) {
    let window = ui.window();
    window.set_maximized(!window.is_maximized());
}

/// Draw the whole interface `scale` times bigger than Windows' own scaling for this monitor.
/// Does nothing before the window exists or when the scale is already right.
pub fn apply_ui_scale(ui: &AppWindow, scale: f32) {
    let window = ui.window();
    let Some(system) = window.with_winit_window(|w| w.scale_factor() as f32) else { return };
    let scale_factor = system * scale;
    if (window.scale_factor() - scale_factor).abs() < 0.001 {
        return;
    }
    // Same physical window, so the layout gets a smaller logical size and everything grows.
    window.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor });
    window.dispatch_event(WindowEvent::Resized { size: window.size().to_logical(scale_factor) });
}

/// Where the window was before it covered the screen.
pub struct Placement {
    position: winit::dpi::PhysicalPosition<i32>,
    size: winit::dpi::PhysicalSize<u32>,
    maximized: bool,
}

/// Cover the whole monitor, above the taskbar. (Real full screen mis-restores a frameless window on Windows.)
pub fn cover_screen(ui: &AppWindow) -> Option<Placement> {
    ui.window()
        .with_winit_window(|w| {
            let monitor = w.current_monitor()?;
            let placement = Placement { position: w.outer_position().ok()?, size: w.inner_size(), maximized: w.is_maximized() };
            w.set_maximized(false);
            w.set_window_level(winit::window::WindowLevel::AlwaysOnTop);
            w.set_outer_position(monitor.position());
            let _ = w.request_inner_size(monitor.size());
            Some(placement)
        })
        .flatten()
}

pub fn restore_placement(ui: &AppWindow, placement: Placement) {
    ui.window().with_winit_window(|w| {
        w.set_window_level(winit::window::WindowLevel::Normal);
        w.set_outer_position(placement.position);
        let _ = w.request_inner_size(placement.size);
        w.set_maximized(placement.maximized);
        w.focus_window();
    });
}
