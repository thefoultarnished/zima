//! Frameless window: backend setup, title bar buttons, drag, double-click maximize, edge resize.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::ComponentHandle;
use slint::platform::WindowEvent;
use slint::winit_030::{WinitWindowAccessor, winit};
use winit::window::ResizeDirection;

use crate::AppWindow;
use crate::model::WindowPlacement;

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

/// Raise the window above other apps and give it the keyboard.
pub fn bring_to_front(ui: &AppWindow) {
    ui.window().with_winit_window(|w| w.focus_window());
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

/// Open the main window where it was last time (call before it's shown). A spot that's no longer
/// on any screen (a monitor was unplugged) is ignored, so the window never opens out of reach.
pub fn restore(ui: &AppWindow, placement: WindowPlacement) {
    let window = ui.window();
    if placement.width > 0 && placement.height > 0 && title_bar_on_screen(&placement) {
        window.set_position(slint::PhysicalPosition::new(placement.x, placement.y));
        window.set_size(slint::PhysicalSize::new(placement.width, placement.height));
    }
    if placement.maximized {
        window.set_maximized(true);
    }
}

/// Where the main window is now, to save. `previous` is kept while the window is minimized or
/// hasn't been shown yet (started in the tray), when there's no real position to read.
pub fn placement(ui: &AppWindow, previous: Option<WindowPlacement>) -> Option<WindowPlacement> {
    ui.window()
        .with_winit_window(|w| {
            if w.is_minimized() == Some(true) {
                return previous;
            }
            let position = w.outer_position().ok()?;
            let size = w.inner_size();
            Some(next_placement(previous, position.x, position.y, size.width, size.height, w.is_maximized()))
        })
        .unwrap_or(previous)
}

/// A maximized window's own size is the whole screen, so keep the last normal size and position
/// for when it's un-maximized.
fn next_placement(previous: Option<WindowPlacement>, x: i32, y: i32, width: u32, height: u32, maximized: bool) -> WindowPlacement {
    if maximized {
        WindowPlacement { maximized: true, ..previous.unwrap_or_default() }
    } else {
        WindowPlacement { x, y, width, height, maximized: false }
    }
}

/// Whether the middle of the title bar is on a monitor, so the window can be seen and dragged.
#[cfg(windows)]
fn title_bar_on_screen(placement: &WindowPlacement) -> bool {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONULL, MonitorFromPoint};
    let point = POINT { x: placement.x.saturating_add((placement.width / 2) as i32), y: placement.y.saturating_add(16) };
    !unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONULL) }.is_invalid()
}

#[cfg(not(windows))]
fn title_bar_on_screen(_placement: &WindowPlacement) -> bool {
    true
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

#[cfg(test)]
mod tests {
    use super::*;

    const SAVED: WindowPlacement = WindowPlacement { x: 200, y: 100, width: 1300, height: 900, maximized: false };

    #[test]
    fn normal_window_saves_its_own_spot() {
        let placement = next_placement(Some(SAVED), 50, 60, 800, 600, false);
        assert_eq!(placement, WindowPlacement { x: 50, y: 60, width: 800, height: 600, maximized: false });
        // First time, nothing saved yet.
        assert_eq!(next_placement(None, -5, 0, 700, 500, false), WindowPlacement { x: -5, y: 0, width: 700, height: 500, maximized: false });
    }

    #[test]
    fn maximized_window_keeps_the_size_to_go_back_to() {
        // The maximized size (the whole screen) isn't saved as the normal size.
        let placement = next_placement(Some(SAVED), -8, -8, 2576, 1416, true);
        assert_eq!(placement, WindowPlacement { maximized: true, ..SAVED });
        // Maximized with no normal size known yet: just remember it was maximized.
        assert_eq!(next_placement(None, 0, 0, 1920, 1080, true), WindowPlacement { maximized: true, ..Default::default() });
    }

    #[cfg(windows)]
    #[test]
    fn a_spot_off_every_screen_is_ignored() {
        // Far beyond any monitor (as after unplugging one).
        // The main screen starts at the top-left corner.
        assert!(title_bar_on_screen(&SAVED));
        let gone = WindowPlacement { x: 100_000, y: 100_000, ..SAVED };
        assert!(!title_bar_on_screen(&gone));
        let above = WindowPlacement { x: 0, y: -100_000, ..SAVED };
        assert!(!title_bar_on_screen(&above));
        // No overflow at the extremes.
        let edge = WindowPlacement { x: i32::MAX, y: i32::MAX, width: u32::MAX, height: 1, maximized: false };
        assert!(!title_bar_on_screen(&edge));
    }
}
