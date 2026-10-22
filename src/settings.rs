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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub theme: ThemeChoice,
    #[serde(default = "default_sidebar_width")]
    pub sidebar_width: f32,
    #[serde(default = "default_editor_height")]
    pub editor_height: f32,
    #[serde(default = "default_editor_font_size")]
    pub editor_font_size: f32,
    #[serde(default = "default_page_size")]
    pub page_size: usize,
    /// Show autocomplete suggestions as you type, not only on Ctrl+Space.
    #[serde(default)]
    pub complete_while_typing: bool,
    /// Closing the window hides it to the system tray instead of quitting.
    #[serde(default)]
    pub minimize_to_tray: bool,
}

pub const EDITOR_FONT_SIZES: [f32; 8] = [11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 18.0, 20.0];
pub const PAGE_SIZES: [usize; 4] = [200, 500, 1000, 2000];

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            sidebar_width: default_sidebar_width(),
            editor_height: default_editor_height(),
            editor_font_size: default_editor_font_size(),
            page_size: default_page_size(),
            complete_while_typing: false,
            minimize_to_tray: false,
        }
    }
}

fn default_sidebar_width() -> f32 {
    300.0
}

fn default_editor_height() -> f32 {
    280.0
}

fn default_editor_font_size() -> f32 {
    13.0
}

fn default_page_size() -> usize {
    500
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
    directories::ProjectDirs::from("", "", "litedb").map(|d| d.config_dir().join(name))
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
