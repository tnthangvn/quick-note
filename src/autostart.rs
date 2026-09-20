//! Start-with-session support: an XDG autostart desktop entry.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const ENTRY_FILE: &str = "quick-note.desktop";

/// `~/.config/autostart` (XDG spec: `$XDG_CONFIG_HOME/autostart`).
pub fn autostart_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("autostart"))
}

fn entry(exec: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Quick Note\n\
         Comment=Ghi chú nhanh dạng sticky note\n\
         Exec={}\n\
         Icon=quick-note\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n",
        exec.display()
    )
}

pub fn is_enabled() -> bool {
    autostart_dir().is_some_and(|d| d.join(ENTRY_FILE).exists())
}

/// Writes or removes the autostart entry for the currently running binary.
pub fn set(enabled: bool) -> Result<()> {
    let dir = autostart_dir().context("không tìm thấy thư mục config")?;
    let exec = std::env::current_exe().context("không xác định được đường dẫn app")?;
    set_in(&dir, enabled, &exec)
}

/// Same as [`set`], with explicit paths (used by tests).
pub fn set_in(dir: &Path, enabled: bool, exec: &Path) -> Result<()> {
    let path = dir.join(ENTRY_FILE);
    if !enabled {
        return match fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("xoá {}", path.display())),
        };
    }
    fs::create_dir_all(dir)?;
    fs::write(&path, entry(exec)).with_context(|| format!("ghi {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enable_then_disable_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let exec = Path::new("/home/me/.local/bin/quick-note");
        set_in(dir.path(), true, exec).unwrap();
        set_in(dir.path(), true, exec).unwrap();
        let body = fs::read_to_string(dir.path().join(ENTRY_FILE)).unwrap();
        assert!(body.contains("Exec=/home/me/.local/bin/quick-note"));
        assert!(body.contains("X-GNOME-Autostart-enabled=true"));

        set_in(dir.path(), false, exec).unwrap();
        set_in(dir.path(), false, exec).unwrap();
        assert!(!dir.path().join(ENTRY_FILE).exists());
    }

    #[test]
    fn entry_is_a_valid_desktop_file() {
        let text = entry(Path::new("/usr/bin/quick-note"));
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.ends_with('\n'));
        assert!(text.contains("Type=Application"));
    }
}
