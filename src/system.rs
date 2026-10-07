//! Windows integration: launch at login, Explorer, links, file dialogs.

use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
#[cfg(windows)]
const RUN_VALUE: &str = "Zima";

/// Started by the login entry: show only the tray icon.
pub const HIDDEN_ARG: &str = "--hidden";

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
