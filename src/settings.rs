//! App preferences, stored as JSON next to the saved connections.

use std::fmt;
use std::path::PathBuf;

use iced::Theme;
use iced::theme::Mode;
use serde::{Deserialize, Serialize};

const FILE: &str = "settings.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    /// The theme to use, given what the desktop currently prefers.
    pub fn resolve(self, system: Mode) -> Theme {
        let dark = match self {
            ThemeChoice::System => system != Mode::Light,
            ThemeChoice::Light => false,
            ThemeChoice::Dark => true,
        };
        if dark { crate::style::DARK.clone() } else { crate::style::LIGHT.clone() }
    }
}

impl fmt::Display for ThemeChoice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        })
    }
}

/// Missing fields (from older versions) take their defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: ThemeChoice,
    pub sidebar_width: f32,
    pub editor_height: f32,
    pub editor_font_size: f32,
    /// Show autocomplete suggestions as you type, not only on Ctrl+Space.
    pub complete_while_typing: bool,
    /// Closing the window hides it to the system tray instead of quitting.
    pub minimize_to_tray: bool,
    /// Where the window was when querydeck last closed.
    pub window: WindowGeometry,
}

/// The window's normal (not maximized) size and position, and whether it
/// was maximized. Wayland doesn't tell apps where their window is, so there
/// the position stays unset and the desktop places the window.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowGeometry {
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub maximized: bool,
}

impl Default for WindowGeometry {
    fn default() -> Self {
        Self { width: 1280.0, height: 800.0, x: None, y: None, maximized: false }
    }
}

pub const EDITOR_FONT_SIZES: [f32; 8] = [11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 18.0, 20.0];

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            sidebar_width: 300.0,
            editor_height: 280.0,
            editor_font_size: 13.0,
            complete_while_typing: false,
            minimize_to_tray: false,
            window: WindowGeometry::default(),
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        read_json(FILE).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        write_json(FILE, self)
    }
}

pub fn config_file(name: &str) -> Option<PathBuf> {
    config_dir("querydeck").map(|dir| dir.join(name))
}

fn config_dir(app: &str) -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", app).map(|d| d.config_dir().to_path_buf())
}

/// querydeck used to be called litedb: moves its settings and saved
/// connections over on first start.
pub fn migrate_from_litedb() {
    if let (Some(old), Some(new)) = (config_dir("litedb"), config_dir("querydeck"))
        && old.is_dir()
        && !new.exists()
    {
        if let Some(parent) = new.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::rename(&old, &new);
    }
}

pub fn read_json<T: for<'de> Deserialize<'de>>(name: &str) -> Option<T> {
    let text = std::fs::read_to_string(config_file(name)?).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn write_json<T: Serialize>(name: &str, value: &T) -> Result<(), String> {
    let path = config_file(name).ok_or("Could not find a config directory")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("Could not write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_defaults() {
        let s: Settings = serde_json::from_str(r#"{"theme": "Dark", "window": {"width": 900.0}}"#).unwrap();
        assert_eq!(s.theme, ThemeChoice::Dark);
        assert_eq!(s.editor_font_size, 13.0);
        assert_eq!((s.window.width, s.window.height), (900.0, 800.0));
    }
}
