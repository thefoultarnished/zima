//! Windows integration: launch at login, Explorer, links, file dialogs.

use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "Zima";

/// Started by the login entry: show only the tray icon.
pub const HIDDEN_ARG: &str = "--hidden";

/// Windows' id for Zima (AppUserModelID). The installer puts the same id on the Start-menu
/// shortcut (`installer/zima.iss`); notifications and taskbar grouping use it.
#[cfg(windows)]
const APP_ID: &str = "Zima.Notes";

/// Tell Windows this process is Zima, so the taskbar groups it with Zima's Start-menu shortcut.
pub fn set_app_id() {
    #[cfg(windows)]
    if let Err(e) = unsafe { windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&windows::core::HSTRING::from(APP_ID)) } {
        eprintln!("couldn't set the app id: {e}");
    }
}

/// Whether the mouse cursor is the "hand" right now. Slint shows the hand only over a link, so this tells the
/// link hover card that the pointer is on a link and not just somewhere in the paragraph.
pub fn pointer_on_link() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::UI::WindowsAndMessaging::{CURSORINFO, GetCursorInfo, IDC_HAND, LoadCursorW};
        let mut info = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
        // SAFETY: `info` is a valid CURSORINFO with its size set; the hand is a shared system cursor.
        unsafe {
            GetCursorInfo(&mut info).is_ok() && LoadCursorW(None, IDC_HAND).is_ok_and(|hand| hand == info.hCursor)
        }
    }
    #[cfg(not(windows))]
    true
}

/// A notification from Zima. Windows only shows "Zima" on it when the installer's Start-menu
/// shortcut exists; without one, Windows silently drops notifications with Zima's own id, so
/// they borrow PowerShell's (and say "Windows PowerShell").
pub fn notification() -> notify_rust::Notification {
    let mut notification = notify_rust::Notification::new();
    notification.appname("Zima");
    #[cfg(windows)]
    if start_menu_shortcut().is_some_and(|path| path.exists()) {
        notification.app_id(APP_ID);
    }
    notification
}

/// `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Zima.lnk`, where the installer puts it.
#[cfg(windows)]
fn start_menu_shortcut() -> Option<PathBuf> {
    let roaming = directories::BaseDirs::new()?.data_dir().to_path_buf();
    Some(roaming.join("Microsoft").join("Windows").join("Start Menu").join("Programs").join("Zima.lnk"))
}

#[cfg(windows)]
pub fn launch_at_login() -> bool {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(RUN_KEY)
        .and_then(|key| key.get_value::<String, _>(RUN_VALUE))
        .is_ok()
}

#[cfg(windows)]
pub fn set_launch_at_login(enabled: bool) -> std::io::Result<()> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
    let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)?;
    if enabled {
        let exe = std::env::current_exe()?;
        key.set_value(RUN_VALUE, &format!("\"{}\" {HIDDEN_ARG}", exe.display()))
    } else {
        match key.delete_value(RUN_VALUE) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

#[cfg(not(windows))]
pub fn launch_at_login() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set_launch_at_login(_enabled: bool) -> std::io::Result<()> {
    Err(std::io::Error::other("launch at login is only supported on Windows"))
}

pub fn reveal(path: &Path) {
    if let Err(e) = Command::new("explorer").arg(path).spawn() {
        eprintln!("failed to open {}: {e}", path.display());
    }
}

/// Open a file in its default app.
pub fn open_file(path: &Path) {
    #[cfg(windows)]
    let result = Command::new("rundll32").arg("url.dll,FileProtocolHandler").arg(path).spawn();
    #[cfg(target_os = "macos")]
    let result = Command::new("open").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let result = Command::new("xdg-open").arg(path).spawn();
    if let Err(e) = result {
        eprintln!("failed to open {}: {e}", path.display());
    }
}

/// Open http(s) and mailto links in the default handler; ignore anything else.
pub fn open_url(url: &str) {
    let allowed = ["http://", "https://", "mailto:"].iter().any(|p| url.to_ascii_lowercase().starts_with(p));
    if !allowed {
        return;
    }
    if let Err(e) = Command::new("rundll32").args(["url.dll,FileProtocolHandler", url]).spawn() {
        eprintln!("failed to open {url}: {e}");
    }
}

pub fn pick_import_files() -> Vec<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Import from Zima")
        .add_filter("Zima export or backups", &["json", "txt"])
        .pick_files()
        .unwrap_or_default()
}

pub fn pick_export_path(title: &str) -> Option<PathBuf> {
    let name: String = title.chars().map(|c| if "<>:\"/\\|?*".contains(c) { '_' } else { c }).collect();
    let name = if name.trim().is_empty() { "Untitled".to_string() } else { name.trim().to_string() };
    rfd::FileDialog::new()
        .set_title("Export note")
        .set_file_name(format!("{name}.md"))
        .add_filter("Markdown", &["md"])
        .save_file()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    /// The installer's settings (`installer/zima.iss`), read as data.
    fn installer_script() -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("installer").join("zima.iss")).unwrap()
    }

    #[test]
    fn installer_shortcut_has_the_apps_id() {
        // A different id on the shortcut and Windows drops every notification Zima sends.
        let script = installer_script();
        let shortcuts: Vec<&str> = script.lines().filter(|l| l.starts_with("Name: ") && l.contains("zima.exe")).collect();
        assert_eq!(shortcuts.len(), 2);
        for line in shortcuts {
            assert!(line.contains(&format!("AppUserModelID: \"{APP_ID}\"")), "{line}");
        }
    }

    #[test]
    fn installer_waits_for_the_running_app() {
        let script = installer_script();
        assert!(script.lines().any(|l| l == format!("AppMutex={}", crate::instance::RUNNING_NAME)));
    }

    #[test]
    fn shortcut_is_looked_for_in_the_users_start_menu() {
        let path = start_menu_shortcut().unwrap();
        assert!(path.ends_with(r"Microsoft\Windows\Start Menu\Programs\Zima.lnk"), "{}", path.display());
        assert!(path.starts_with(directories::BaseDirs::new().unwrap().data_dir()));
    }
}
