//! User settings (Drive OAuth client + target folder, auto sync).

use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::storage;

pub const CONFIG_FILE: &str = "config.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DriveConfig {
    /// OAuth client of type "Desktop app" from Google Cloud Console.
    pub client_id: String,
    pub client_secret: String,
    /// Existing Drive folder ID. When set, the app needs the full `drive` scope.
    pub folder_id: String,
    /// Folder the app creates/uses when `folder_id` is empty (`drive.file` scope).
    pub folder_name: String,
    /// Also upload each page as a readable Markdown file.
    pub export_markdown: bool,
    /// Path to a service-account JSON key. When set, it replaces the browser
    /// login — and `folder_id` must point into a Shared Drive.
    pub service_account_key: String,
    /// Email người dùng để service account "đóng vai" (uỷ quyền toàn miền).
    /// Khi đặt, file do người này sở hữu và nằm trong My Drive của họ.
    pub impersonate: String,
}

impl Default for DriveConfig {
    fn default() -> Self {
        Self {
            client_id: String::new(),
            client_secret: String::new(),
            folder_id: String::new(),
            folder_name: "QuickNote".into(),
            export_markdown: true,
            service_account_key: String::new(),
            impersonate: String::new(),
        }
    }
}

impl DriveConfig {
    pub fn uses_service_account(&self) -> bool {
        !self.service_account_key.trim().is_empty()
    }

    pub fn is_configured(&self) -> bool {
        self.uses_service_account()
            || (!self.client_id.trim().is_empty() && !self.client_secret.trim().is_empty())
    }

    pub fn scope(&self) -> &'static str {
        if self.uses_service_account() {
            return "https://www.googleapis.com/auth/drive";
        }
        if self.folder_id.trim().is_empty() {
            "https://www.googleapis.com/auth/drive.file"
        } else {
            "https://www.googleapis.com/auth/drive"
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub drive: DriveConfig,
    /// Push to Drive every N minutes when there are changes. 0 = off.
    pub auto_sync_minutes: u32,
    pub font_size: f32,
    /// Show note bodies as rendered Markdown when not editing.
    pub markdown: bool,
    pub dock: DockConfig,
    /// Mirrors the XDG autostart entry (see `autostart.rs`).
    pub autostart: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            drive: DriveConfig::default(),
            auto_sync_minutes: 0,
            font_size: 15.0,
            markdown: true,
            dock: DockConfig::default(),
            autostart: false,
        }
    }
}

/// Corner-sticky mode: small handle in the bottom-right, expands on hover.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DockConfig {
    pub enabled: bool,
    /// Width of the expanded panel (points). Height is always the full screen.
    pub width: f32,
    /// Size of the collapsed square handle.
    pub handle_size: f32,
    /// How long the pointer may be outside before collapsing.
    pub collapse_delay_ms: u32,
    /// Open/close animation length. 0 = instant.
    pub animation_ms: u32,
    /// Monitor name (e.g. "HDMI-1"). Empty = the rightmost monitor.
    pub monitor: String,
}

impl DockConfig {
    pub const MIN_WIDTH: f32 = 360.0;
}

impl Default for DockConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            width: 1000.0,
            handle_size: 36.0,
            collapse_delay_ms: 400,
            animation_ms: 180,
            monitor: String::new(),
        }
    }
}

impl Config {
    pub fn load(dir: &Path) -> Result<Self> {
        Ok(storage::read_json(&dir.join(CONFIG_FILE))?.unwrap_or_default())
    }

    /// Private: contains the OAuth client secret.
    pub fn save(&self, dir: &Path) -> Result<()> {
        storage::write_json(&dir.join(CONFIG_FILE), self, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_depends_on_folder_id() {
        let mut cfg = DriveConfig::default();
        assert!(cfg.scope().ends_with("drive.file"));
        cfg.folder_id = "abc".into();
        assert!(cfg.scope().ends_with("/drive"));
    }

    #[test]
    fn service_account_key_switches_auth_mode() {
        let mut cfg = DriveConfig::default();
        assert!(!cfg.uses_service_account() && !cfg.is_configured());
        cfg.service_account_key = "/home/me/key.json".into();
        assert!(cfg.uses_service_account());
        assert!(cfg.is_configured(), "no client id/secret needed");
        assert!(cfg.scope().ends_with("/drive"));
    }

    #[test]
    fn partial_config_file_fills_defaults() {
        let cfg: Config = serde_json::from_str(r#"{"drive":{"client_id":"x"}}"#).unwrap();
        assert_eq!(cfg.drive.client_id, "x");
        assert_eq!(cfg.drive.folder_name, "QuickNote");
        assert!(!cfg.drive.is_configured());
    }
}
