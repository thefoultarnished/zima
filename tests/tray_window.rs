//! End-to-end tests for "no main window while in the tray". They start real copies of Zima
//! against throwaway data folders and look at the windows Windows reports for each process.
//! The ones that open a window are `#[ignore]`d (they flash a window on the desktop); run them
//! with `cargo test --test tray_window -- --ignored`.
#![cfg(windows)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, PostMessageW, WM_CLOSE,
};
use windows::core::BOOL;

/// Time for a copy to start up and claim its folder.
const STARTUP: Duration = Duration::from_secs(3);

struct DataDir(PathBuf);

impl DataDir {
    fn new(name: &str) -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("zima-tray-test-{name}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // No "still running in the tray" notification popping up when a test closes the window.
        std::fs::write(dir.join("state.json"), r#"{"tray_hint_shown": true}"#).unwrap();
        Self(dir)
    }

    /// Settings as saved by the app.
    fn state(&self) -> String {
        std::fs::read_to_string(self.0.join("state.json")).unwrap_or_default()
    }
}

impl Drop for DataDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A copy of Zima this test started; ended when the test ends, pass or fail.
struct Zima(Child);

impl Zima {
    fn start(data: &DataDir, args: &[&str]) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_zima"))
            .args(args)
            .env("ZIMA_DATA_DIR", &data.0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start zima");
        Self(child)
    }

    fn running(&mut self) -> bool {
        self.0.try_wait().unwrap().is_none()
    }

    /// Every visible top-level window of this process whose title is "Zima".
    fn main_windows(&self) -> Vec<HWND> {
        struct Search {
            pid: u32,
            found: Vec<HWND>,
        }
        unsafe extern "system" fn visit(window: HWND, param: LPARAM) -> BOOL {
            let search = unsafe { &mut *(param.0 as *mut Search) };
            let mut pid = 0;
            unsafe { GetWindowThreadProcessId(window, Some(&mut pid)) };
            if pid == search.pid && unsafe { IsWindowVisible(window) }.as_bool() {
                let mut title = [0u16; 64];
                let len = unsafe { GetWindowTextW(window, &mut title) } as usize;
                if String::from_utf16_lossy(&title[..len]) == "Zima" {
                    search.found.push(window);
                }
            }
            true.into()
        }
        let mut search = Search { pid: self.0.id(), found: Vec::new() };
        unsafe {
            let _ = EnumWindows(Some(visit), LPARAM(&mut search as *mut Search as isize));
        }
        search.found
    }

    /// Wait up to `within` until the main window is (or isn't) there.
    fn wait_for_window(&self, shown: bool, within: Duration) -> bool {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if self.main_windows().is_empty() != shown {
                return true;
            }
            sleep(Duration::from_millis(100));
        }
        self.main_windows().is_empty() != shown
    }

    /// Click the window's close button, as far as the app can tell.
    fn close_window(&self) {
        for window in self.main_windows() {
            unsafe {
                let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }

    fn wait_for_exit(&mut self, within: Duration) -> Option<i32> {
        let deadline = Instant::now() + within;
        while Instant::now() < deadline {
            if let Some(status) = self.0.try_wait().unwrap() {
                return status.code();
            }
            sleep(Duration::from_millis(100));
        }
        None
    }
}

impl Drop for Zima {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn hidden_start_builds_no_main_window() {
    let data = DataDir::new("hidden");
    let mut zima = Zima::start(&data, &["--hidden"]);
    sleep(STARTUP);
    assert!(zima.running(), "it should keep running in the tray");
    assert!(zima.main_windows().is_empty(), "no window should exist until Zima is opened");
}

#[test]
#[ignore = "opens a real window"]
fn closing_frees_the_window_and_reopening_rebuilds_it() {
    let data = DataDir::new("reopen");
    let mut zima = Zima::start(&data, &[]);
    assert!(zima.wait_for_window(true, Duration::from_secs(10)), "the window should open");

    zima.close_window();
    assert!(zima.wait_for_window(false, Duration::from_secs(2)), "closing should take the window away");
    assert!(zima.running(), "closing hides to the tray, it doesn't quit");
    assert!(data.state().contains("\"window\""), "where the window was is saved when it closes");

    // Another launch on the same folder asks the running copy to show its window.
    let mut second = Zima::start(&data, &[]);
    assert!(zima.wait_for_window(true, Duration::from_secs(10)), "the window should come back");
    assert_eq!(second.wait_for_exit(Duration::from_secs(10)), Some(0));
}

#[test]
#[ignore = "opens a real window"]
fn closing_with_tray_off_quits() {
    let data = DataDir::new("quit");
    std::fs::write(data.0.join("state.json"), r#"{"tray_hint_shown": true, "close_to_tray": false}"#).unwrap();
    let mut zima = Zima::start(&data, &[]);
    assert!(zima.wait_for_window(true, Duration::from_secs(10)), "the window should open");

    zima.close_window();
    assert_eq!(zima.wait_for_exit(Duration::from_secs(10)), Some(0), "with the tray off, closing quits");
}

#[test]
#[ignore = "opens a real window"]
fn notes_survive_close_and_reopen() {
    let data = DataDir::new("notes");
    let added = Command::new(env!("CARGO_BIN_EXE_zima"))
        .args(["add", "a note that must survive"])
        .env("ZIMA_DATA_DIR", &data.0)
        .output()
        .expect("zima add");
    assert!(added.status.success());
    let notes = || -> Vec<(PathBuf, String)> {
        let mut found: Vec<_> = std::fs::read_dir(data.0.join("notes"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .map(|p| {
                let text = std::fs::read_to_string(&p).unwrap();
                (p, text)
            })
            .collect();
        found.sort();
        found
    };
    let before = notes();
    assert!(before.iter().any(|(_, text)| text.contains("a note that must survive")));

    let zima = Zima::start(&data, &[]);
    assert!(zima.wait_for_window(true, Duration::from_secs(10)));
    zima.close_window();
    assert!(zima.wait_for_window(false, Duration::from_secs(2)));
    let mut second = Zima::start(&data, &[]);
    assert!(zima.wait_for_window(true, Duration::from_secs(10)));
    assert_eq!(second.wait_for_exit(Duration::from_secs(10)), Some(0));
    zima.close_window();
    assert!(zima.wait_for_window(false, Duration::from_secs(2)));

    // Opening and closing windows never rewrites a note (an empty one may be added on first start).
    let after = notes();
    for (path, text) in &before {
        assert!(after.iter().any(|(p, t)| p == path && t == text), "{} changed", path.display());
    }
    let index = std::fs::read_to_string(data.0.join("index.json")).unwrap();
    for (path, _) in &before {
        let id = path.file_stem().unwrap().to_string_lossy();
        assert!(index.contains(id.as_ref()), "note {id} missing from the list");
    }
}
