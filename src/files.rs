//! Opening and saving .sql files: native file dialogs and text decoding.

use std::path::{Path, PathBuf};

/// Shows the system's open dialog. Returns the chosen files, or nothing if
/// the dialog was cancelled.
pub async fn pick_open(dir: Option<PathBuf>) -> Vec<PathBuf> {
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_title("Open SQL File")
        .add_filter("SQL files", &["sql"])
        .add_filter("All files", &["*"]);
    if let Some(dir) = dir {
        dialog = dialog.set_directory(dir);
    }
    dialog.pick_files().await.unwrap_or_default().into_iter().map(|f| f.path().to_path_buf()).collect()
}

/// Shows the system's save dialog, suggesting `name` in `dir`.
pub async fn pick_save(dir: Option<PathBuf>, name: String) -> Option<PathBuf> {
    let mut dialog =
        rfd::AsyncFileDialog::new().set_title("Save SQL File").add_filter("SQL files", &["sql"]).set_file_name(name);
    if let Some(dir) = dir {
        dialog = dialog.set_directory(dir);
    }
    dialog.save_file().await.map(|f| f.path().to_path_buf())
}

pub async fn read(path: PathBuf) -> Result<String, String> {
    let bytes = tokio::fs::read(&path).await.map_err(|e| format!("Could not open {}: {e}", path.display()))?;
    decode(&bytes).map_err(|e| format!("Could not open {}: {e}", path.display()))
}

/// Saves as UTF-8 without a byte-order mark.
pub fn write(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| format!("Could not save {}: {e}", path.display()))
}

/// Decodes UTF-8 (with or without a BOM) and UTF-16 with a BOM, which is what
/// SSMS writes by default.
pub fn decode(bytes: &[u8]) -> Result<String, String> {
    let utf16 = |rest: &[u8], from: fn([u8; 2]) -> u16| {
        if !rest.len().is_multiple_of(2) {
            return Err("the UTF-16 text is truncated".to_string());
        }
        let units = rest.as_chunks::<2>().0.iter().map(|c| from(*c));
        char::decode_utf16(units).collect::<Result<String, _>>().map_err(|_| "the UTF-16 text is invalid".to_string())
    };
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).map_err(|_| "the file isn't valid UTF-8".to_string());
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| "it isn't UTF-8 or UTF-16 text".to_string())
}

/// The name a tab shows for a file.
pub fn display_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf8_with_and_without_bom() {
        assert_eq!(decode(b"select 1").unwrap(), "select 1");
        assert_eq!(decode(b"\xEF\xBB\xBFselect '\xC3\xA6'").unwrap(), "select 'æ'");
    }

    #[test]
    fn decodes_utf16_with_bom() {
        let le: Vec<u8> = [0xFF, 0xFE].into_iter().chain("Søk".encode_utf16().flat_map(u16::to_le_bytes)).collect();
        let be: Vec<u8> = [0xFE, 0xFF].into_iter().chain("Søk".encode_utf16().flat_map(u16::to_be_bytes)).collect();
        assert_eq!(decode(&le).unwrap(), "Søk");
        assert_eq!(decode(&be).unwrap(), "Søk");
        assert!(decode(&le[..le.len() - 1]).is_err());
    }

    #[test]
    fn rejects_other_encodings() {
        // "æ" in Windows-1252.
        assert!(decode(b"select '\xE6'").is_err());
    }
}
