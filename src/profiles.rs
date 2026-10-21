//! Saved connection profiles. Profiles live in a JSON file in the user's config
//! directory; passwords go to the OS keyring (Secret Service on Linux).

use crate::db::Profile;
use crate::settings::{read_json, write_json};

const KEYRING_SERVICE: &str = "litedb";
const FILE: &str = "connections.json";

pub fn load() -> Vec<Profile> {
    read_json(FILE).unwrap_or_default()
}

pub fn save(profiles: &[Profile]) -> Result<(), String> {
    write_json(FILE, &profiles)
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
