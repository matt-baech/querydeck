//! Saved connection profiles. Profiles live in a JSON file in the user's config
//! directory; passwords go to the OS keyring (Secret Service on Linux).

use crate::db::Profile;
use crate::settings::{read_json, write_json};

const KEYRING_SERVICE: &str = "querydeck";
/// Where passwords were kept when querydeck was called litedb.
const OLD_KEYRING_SERVICE: &str = "litedb";
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
    let account = keyring_account(p);
    if let Ok(password) = keyring::Entry::new(KEYRING_SERVICE, &account).ok()?.get_password() {
        return Some(password);
    }
    // Saved before the rename: move it to the new name.
    let old = keyring::Entry::new(OLD_KEYRING_SERVICE, &account).ok()?;
    let password = old.get_password().ok()?;
    if store_password(p, &password).is_ok() {
        let _ = old.delete_credential();
    }
    Some(password)
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
