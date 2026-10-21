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
            ThemeChoice::System => "System theme",
            ThemeChoice::Light => "Light theme",
            ThemeChoice::Dark => "Dark theme",
        })
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub theme: ThemeChoice,
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
