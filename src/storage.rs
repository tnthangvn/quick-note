//! Local persistence: JSON files under the platform config dir, written atomically.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::model::Workspace;

pub const APP_DIR: &str = "quick-note";
pub const WORKSPACE_FILE: &str = "workspace.json";

pub fn app_dir() -> Result<PathBuf> {
    let base = dirs::config_dir().context("không tìm thấy thư mục config của hệ thống")?;
    let dir = base.join(APP_DIR);
    fs::create_dir_all(&dir).with_context(|| format!("không tạo được {}", dir.display()))?;
    Ok(dir)
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(path).with_context(|| format!("đọc {}", path.display()))?;
    let value = serde_json::from_str(&raw).with_context(|| format!("parse {}", path.display()))?;
    Ok(Some(value))
}

/// Writes to a temp file then renames, so a crash never leaves a half-written file.
/// `private` restricts permissions to the owner (used for OAuth tokens).
pub fn write_json<T: Serialize>(path: &Path, value: &T, private: bool) -> Result<()> {
    let data = serde_json::to_vec_pretty(value)?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = open_for_write(&tmp, private)?;
        file.write_all(&data)?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).with_context(|| format!("ghi {}", path.display()))?;
    Ok(())
}

#[cfg(unix)]
fn open_for_write(path: &Path, private: bool) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let mode = if private { 0o600 } else { 0o644 };
    Ok(fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(mode)
        .open(path)?)
}

#[cfg(not(unix))]
fn open_for_write(path: &Path, _private: bool) -> Result<fs::File> {
    Ok(fs::File::create(path)?)
}

pub fn load_workspace(dir: &Path) -> Result<Workspace> {
    let ws: Option<Workspace> = read_json(&dir.join(WORKSPACE_FILE))?;
    Ok(ws.unwrap_or_default().normalize())
}

pub fn save_workspace(dir: &Path, ws: &Workspace) -> Result<()> {
    write_json(&dir.join(WORKSPACE_FILE), ws, false)
}

/// Copies the workspace to `backups/` (used before destructive replaces).
pub fn backup_workspace(dir: &Path, ws: &Workspace) -> Result<PathBuf> {
    let backups = dir.join("backups");
    fs::create_dir_all(&backups)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let path = backups.join(format!("workspace-{stamp}.json"));
    write_json(&path, ws, false)?;
    Ok(path)
}

/// Writes a page as Markdown into ~/Documents/QuickNote (or the app dir as fallback).
pub fn export_markdown(fallback_dir: &Path, page: &crate::model::Page) -> Result<PathBuf> {
    let dir = dirs::document_dir()
        .map(|d| d.join("QuickNote"))
        .unwrap_or_else(|| fallback_dir.join("export"));
    fs::create_dir_all(&dir)?;
    let path = dir.join(crate::export::safe_file_name(&page.title, "md"));
    fs::write(&path, crate::export::page_to_markdown(page))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backup_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = backup_workspace(dir.path(), &Workspace::default()).unwrap();
        assert!(path.starts_with(dir.path().join("backups")));
        assert!(read_json::<Workspace>(&path).unwrap().is_some());
    }

    #[test]
    fn workspace_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = Workspace::default();
        ws.active_page_mut().add_note([10.0, 20.0]);
        ws.active_page_mut().notes[0].body = "xin chào".into();
        save_workspace(dir.path(), &ws).unwrap();
        let loaded = load_workspace(dir.path()).unwrap();
        assert_eq!(loaded, ws);
    }

    #[test]
    fn missing_file_gives_default() {
        let dir = tempfile::tempdir().unwrap();
        let ws = load_workspace(dir.path()).unwrap();
        assert_eq!(ws.pages.len(), 1);
    }

    #[test]
    fn corrupt_file_is_error_not_silent_reset() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(WORKSPACE_FILE), "{not json").unwrap();
        assert!(load_workspace(dir.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn private_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("token.json");
        write_json(&path, &"secret", true).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
