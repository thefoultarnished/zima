//! Custom theme: colours from theme.json in the notes folder, re-applied whenever the file is saved.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};
use slint::{Color, ComponentHandle, Timer, TimerMode};

use super::{App, parse_hex_color};
use crate::{Theme, system};

/// Index of the Custom theme in Settings.
pub const CUSTOM: i32 = 7;

/// theme.json. Colours are "#rrggbb" or "#rrggbbaa"; anything left out keeps its default.
#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(default)]
pub struct CustomTheme {
    pub name: String,
    /// Dark themes get dark window chrome and code colours.
    pub dark: bool,
    pub paper: String,
    pub sidebar: String,
    pub ink: String,
    pub ink_soft: String,
    pub muted: String,
    pub rule: String,
    pub hover: String,
    pub selected: String,
    pub accent: String,
    pub on_accent: String,
    pub popover: String,
    /// Window background: one colour, or two for a top-to-bottom gradient. Empty = `paper`.
    pub backdrop: Vec<String>,
}

impl Default for CustomTheme {
    fn default() -> Self {
        // A calm forest green to start from.
        Self {
            name: "Forest".into(),
            dark: true,
            paper: "#16201b".into(),
            sidebar: "#121a16".into(),
            ink: "#e3ece6".into(),
            ink_soft: "#b3c4b9".into(),
            muted: "#728a7c".into(),
            rule: "#ffffff14".into(),
            hover: "#ffffff0c".into(),
            selected: "#ffffff18".into(),
            accent: "#7fc8a0".into(),
            on_accent: "#10261a".into(),
            popover: "#1d2a23".into(),
            backdrop: vec!["#1b2a22".into(), "#121a16".into()],
        }
    }
}

impl App {
    fn theme_path(&self) -> PathBuf {
        self.store.root().join("theme.json")
    }

    /// Read theme.json, or explain what's wrong with it.
    fn load_custom_theme(&self) -> Result<CustomTheme, String> {
        match std::fs::read_to_string(self.theme_path()) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(CustomTheme::default()),
            Err(e) => Err(e.to_string()),
        }
    }

    /// Set the Custom theme's colours on the main window (called by `apply_appearance`).
    pub(super) fn apply_custom_theme(&mut self) {
        let Some(ui) = self.ui.upgrade() else { return };
        let theme = ui.global::<Theme>();
        if self.state.theme != CUSTOM {
            theme.set_custom(false);
            return;
        }
        match self.load_custom_theme() {
            Ok(custom) => self.custom_theme = custom,
            Err(e) => self.toast(&format!("theme.json has a mistake: {e}"), true),
        }
        set_custom_colours(&theme, &self.custom_theme);
    }

    /// Open theme.json in the default editor (writing the starter theme first), and switch to it.
    pub fn edit_theme(&mut self) {
        let path = self.theme_path();
        if !path.exists() {
            let text = serde_json::to_string_pretty(&CustomTheme::default()).unwrap_or_default();
            if let Err(e) = std::fs::write(&path, text) {
                self.toast(&format!("Couldn't create theme.json: {e}"), true);
                return;
            }
        }
        if self.state.theme != CUSTOM {
            self.set_theme(CUSTOM);
        }
        self.custom_theme_modified = modified(&path);
        system::open_file(&path);
        self.toast("Edit theme.json and save; Zima updates as you go", false);
    }

    /// Re-apply the Custom theme whenever theme.json changes.
    pub fn watch_custom_theme(&mut self) {
        self.custom_theme_modified = modified(&self.theme_path());
        let app = self.this.clone();
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_secs(1), move || {
            let Some(app) = app.upgrade() else { return };
            let Ok(mut app) = app.try_borrow_mut() else { return };
            let now = modified(&app.theme_path());
            if now != app.custom_theme_modified {
                app.custom_theme_modified = now;
                if app.state.theme == CUSTOM {
                    app.apply_appearance();
                }
            }
        });
        std::mem::forget(timer);
    }
}

fn modified(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn set_custom_colours(theme: &Theme, custom: &CustomTheme) {
    let fallback = CustomTheme::default();
    let colour = |value: &str, default: &str| parse_hex_color(value).or_else(|| parse_hex_color(default)).unwrap_or_default();
    theme.set_custom_name(if custom.name.trim().is_empty() { "Custom".into() } else { custom.name.as_str().into() });
    theme.set_custom_dark(custom.dark);
    theme.set_custom_paper(colour(&custom.paper, &fallback.paper));
    theme.set_custom_sidebar(colour(&custom.sidebar, &fallback.sidebar));
    theme.set_custom_ink(colour(&custom.ink, &fallback.ink));
    theme.set_custom_ink_soft(colour(&custom.ink_soft, &fallback.ink_soft));
    theme.set_custom_muted(colour(&custom.muted, &fallback.muted));
    theme.set_custom_rule(colour(&custom.rule, &fallback.rule));
    theme.set_custom_hover(colour(&custom.hover, &fallback.hover));
    theme.set_custom_selected(colour(&custom.selected, &fallback.selected));
    theme.set_custom_theme_accent(colour(&custom.accent, &fallback.accent));
    theme.set_custom_on_accent(colour(&custom.on_accent, &fallback.on_accent));
    theme.set_custom_popover(colour(&custom.popover, &fallback.popover));
    let stops: Vec<Color> = custom.backdrop.iter().filter_map(|c| parse_hex_color(c)).collect();
    let paper = theme.get_custom_paper();
    theme.set_custom_backdrop_top(stops.first().copied().unwrap_or(paper));
    theme.set_custom_backdrop_bottom(stops.last().copied().unwrap_or(paper));
    theme.set_custom(true);
}

/// Give a sticky note or quick-capture window the main window's look.
pub fn copy_theme(main: &Theme, other: &Theme) {
    other.set_choice(main.get_choice());
    other.set_font(main.get_font());
    other.set_has_custom_accent(main.get_has_custom_accent());
    other.set_custom_accent(main.get_custom_accent());
    other.set_custom_dark(main.get_custom_dark());
    other.set_custom_name(main.get_custom_name());
    other.set_custom_paper(main.get_custom_paper());
    other.set_custom_sidebar(main.get_custom_sidebar());
    other.set_custom_ink(main.get_custom_ink());
    other.set_custom_ink_soft(main.get_custom_ink_soft());
    other.set_custom_muted(main.get_custom_muted());
    other.set_custom_rule(main.get_custom_rule());
    other.set_custom_hover(main.get_custom_hover());
    other.set_custom_selected(main.get_custom_selected());
    other.set_custom_theme_accent(main.get_custom_theme_accent());
    other.set_custom_on_accent(main.get_custom_on_accent());
    other.set_custom_popover(main.get_custom_popover());
    other.set_custom_backdrop_top(main.get_custom_backdrop_top());
    other.set_custom_backdrop_bottom(main.get_custom_backdrop_bottom());
    other.set_custom(main.get_custom());
}
