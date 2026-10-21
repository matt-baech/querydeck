//! Saved connection profiles. Profiles live in a JSON file in the user's config
//! directory; passwords go to the OS keyring (Secret Service on Linux).

use std::path::PathBuf;

use crate::db::Profile;

const KEYRING_SERVICE: &str = "litedb";

fn config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "litedb").map(|d| d.config_dir().join("connections.json"))
}

pub fn load() -> Vec<Profile> {
    let Some(path) = config_path() else { return Vec::new() };
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(profiles: &[Profile]) -> Result<(), String> {
    let path = config_path().ok_or("Could not find a config directory")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(profiles).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| format!("Could not write {}: {e}", path.display()))
}

/// Keyring entries are keyed by the profile's connection identity, so renaming a
/// profile keeps its password but changing host/user/port does not.
fn keyring_account(p: &Profile) -> String {
    format!("{:?}:{}@{}:{}", p.driver, p.user, p.host, p.port)
}

pub fn load_password(p: &Profile) -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, &keyring_account(p)).ok()?.get_password().ok()
}

pub fn store_password(p: &Profile, password: &str) -> Result<(), String> {
    keyring::Entry::new(KEYRING_SERVICE, &keyring_account(p))
        .and_then(|e| e.set_password(password))
        .map_err(|e| format!("Could not save password to keyring: {e}"))
}

pub fn forget_password(p: &Profile) {
    if let Ok(e) = keyring::Entry::new(KEYRING_SERVICE, &keyring_account(p)) {
        let _ = e.delete_credential();
    }
}
