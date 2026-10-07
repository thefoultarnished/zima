//! One Zima per data folder. Two copies writing the same files would overwrite each other's
//! notes, so a second launch asks the first one to show its window and then exits.
//!
//! Uses a named Windows mutex (held for the app's life) and a named event (the "show yourself"
//! signal). A waiting thread costs nothing until the event fires: no timers, no files.

use std::path::Path;
use std::time::Duration;

/// Passed by "Restart now": the old copy is still quitting, so wait for it instead of handing over.
pub const RESTART_ARG: &str = "--restarted";

/// How long a restarted copy waits for the old one to finish saving and exit.
pub const RESTART_WAIT: Duration = Duration::from_secs(15);

/// Exists while any Zima window app runs. The installer checks for it (`AppMutex` in
/// `installer/zima.iss`) and asks to quit Zima before replacing or removing zima.exe.
#[cfg(windows)]
pub const RUNNING_NAME: &str = "ZimaRunning";

/// The name both copies agree on for one data folder. Stable across builds (FNV-1a, not Rust's
/// hasher, whose output may change between versions) and case-insensitive like Windows paths.
pub fn instance_key(data_dir: &Path) -> String {
    let path = std::fs::canonicalize(data_dir).unwrap_or_else(|_| data_dir.to_path_buf());
    let text = path.to_string_lossy().to_lowercase().replace('/', "\\");
    let text = text.trim_end_matches('\\');
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("Zima-{hash:016x}")
}

pub enum Claim {
    /// This is the only copy for this folder.
    First(Instance),
    /// Another copy already has this folder.
    Taken,
}

/// Held for the app's whole life; Windows releases it when the process exits.
pub struct Instance {
    #[cfg(windows)]
    event: windows::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError, WAIT_ABANDONED, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, EVENT_MODIFY_STATE, INFINITE, OpenEventW, SetEvent, WaitForSingleObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    use windows::core::HSTRING;

    // `Local\` keeps it to this Windows session, so other users on the PC have their own.
    fn names(key: &str) -> (HSTRING, HSTRING) {
        (HSTRING::from(format!("Local\\{key}")), HSTRING::from(format!("Local\\{key}-show")))
    }

    /// `wait`: how long to wait for a copy that's on its way out (after a restart).
    pub fn claim(key: &str, wait: Duration) -> Claim {
        let (mutex_name, event_name) = names(key);
        // If anything here fails, run anyway: one copy too many beats Zima not starting.
        let Ok(mutex) = (unsafe { CreateMutexW(None, true, &mutex_name) }) else { return Claim::First(Instance::none()) };
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            let millis = wait.as_millis().min(u32::MAX as u128) as u32;
            let result = unsafe { WaitForSingleObject(mutex, millis) };
            // Abandoned = the old copy exited without letting go, which is how a process exit looks.
            if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED {
                return Claim::Taken;
            }
        }
        // Auto-reset, so each signal wakes the waiting thread once.
        match unsafe { CreateEventW(None, false, false, &event_name) } {
            Ok(event) => Claim::First(Instance { event }),
            Err(_) => Claim::First(Instance::none()),
        }
    }

    /// Ask the copy that owns `key` to show its window. Returns false if there's none to ask.
    pub fn signal_show(key: &str) -> bool {
        let (_, event_name) = names(key);
        let Ok(event) = (unsafe { OpenEventW(EVENT_MODIFY_STATE, false, &event_name) }) else { return false };
        // We were just launched by the user, so we may bring a window to the front; pass that on.
        let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
        unsafe { SetEvent(event) }.is_ok()
    }

    /// Let the installer see that Zima is running. Never closed, so it lasts until the process exits.
    pub fn mark_running() {
        let _ = unsafe { CreateMutexW(None, false, &HSTRING::from(RUNNING_NAME)) };
    }

    impl Instance {
        fn none() -> Self {
            Self { event: Default::default() }
        }

        /// Run `show` (from a background thread) every time another launch asks for the window.
        pub fn on_show(&self, show: impl Fn() + Send + 'static) {
            if self.event.is_invalid() {
                return;
            }
            // HANDLE isn't Send; the raw value is fine to share, the event lives as long as the process.
            let event = self.event.0 as usize;
            std::thread::spawn(move || {
                let event = windows::Win32::Foundation::HANDLE(event as *mut _);
                while unsafe { WaitForSingleObject(event, INFINITE) } == WAIT_OBJECT_0 {
                    show();
                }
            });
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn claim(_key: &str, _wait: Duration) -> Claim {
        Claim::First(Instance {})
    }

    pub fn signal_show(_key: &str) -> bool {
        false
    }

    pub fn mark_running() {}

    impl Instance {
        pub fn on_show(&self, _show: impl Fn() + Send + 'static) {}
    }
}

pub use imp::{claim, mark_running, signal_show};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_folder_gives_the_same_key() {
        let a = instance_key(Path::new(r"C:\Users\Someone\Zima"));
        assert_eq!(a, instance_key(Path::new(r"c:\users\someone\zima")));
        assert_eq!(a, instance_key(Path::new(r"C:\Users\Someone\Zima\")));
        assert_eq!(a, instance_key(Path::new("C:/Users/Someone/Zima")));
    }

    #[test]
    fn different_folders_give_different_keys() {
        assert_ne!(instance_key(Path::new(r"C:\Data\Zima")), instance_key(Path::new(r"C:\Data\Zima2")));
        assert_ne!(instance_key(Path::new(r"C:\Data\Zima")), instance_key(Path::new(r"D:\Data\Zima")));
    }

    #[test]
    fn key_is_stable_between_builds() {
        // An old and a new build must agree, or both would think they're first.
        assert_eq!(instance_key(Path::new(r"Z:\no-such-folder\zima")), "Zima-41c0316747a66dde");
    }

    #[test]
    fn key_is_a_valid_object_name() {
        // Windows object names can't contain a backslash after the `Local\` prefix.
        let key = instance_key(Path::new(r"C:\Users\Someone\My Notes"));
        assert!(key.starts_with("Zima-") && key.len() == 21);
        assert!(key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }

    #[test]
    fn empty_path_still_gives_a_key() {
        assert!(instance_key(Path::new("")).starts_with("Zima-"));
    }
}
