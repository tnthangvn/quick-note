//! Command-line flags: help, version, window mode, uninstall.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

pub const HELP: &str = "\
Quick Note — ghi chú nhanh dạng sticky note

Cách dùng: quick-note [TUỲ CHỌN]

Tuỳ chọn:
  -h, --help        Hiện trợ giúp này
  -V, --version     Hiện phiên bản
  -w, --window      Mở dạng cửa sổ thường (bỏ qua chế độ sticky góc màn hình)
      --drive-check Kiểm tra cấu hình Google Drive (chỉ đọc, không ghi gì)
      --clipboard-check  Xem clipboard đang có kiểu dữ liệu gì (gỡ lỗi dán ảnh)
      --uninstall   Gỡ app đã cài bằng scripts/install.sh (giữ lại ghi chú)
      --purge       Dùng kèm --uninstall: xoá luôn ghi chú + cài đặt (có hỏi xác nhận)

Dữ liệu: ~/.config/quick-note/
";

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub help: bool,
    pub version: bool,
    pub window: bool,
    pub uninstall: bool,
    pub drive_check: bool,
    pub clipboard_check: bool,
    pub purge: bool,
}

pub fn parse(args: impl IntoIterator<Item = String>) -> Result<Args> {
    let mut out = Args::default();
    for arg in args {
        match arg.as_str() {
            "-h" | "--help" => out.help = true,
            "-V" | "--version" => out.version = true,
            "-w" | "--window" => out.window = true,
            "--uninstall" => out.uninstall = true,
            "--drive-check" => out.drive_check = true,
            "--clipboard-check" => out.clipboard_check = true,
            "--purge" => out.purge = true,
            other => bail!(
                "tuỳ chọn không hợp lệ: {other}\nChạy `quick-note --help` để xem các tuỳ chọn."
            ),
        }
    }
    if out.purge && !out.uninstall {
        bail!("--purge chỉ dùng kèm --uninstall");
    }
    Ok(out)
}

/// Files placed by `scripts/install.sh`, relative to $HOME.
fn installed_files(home: &Path, data_home: &Path) -> Vec<PathBuf> {
    vec![
        home.join(".local/bin/quick-note"),
        data_home.join("applications/quick-note.desktop"),
        data_home.join("icons/hicolor/scalable/apps/quick-note.svg"),
    ]
}

pub fn uninstall(purge: bool, data_dir: Option<&Path>) -> Result<()> {
    let exe = std::env::current_exe()?;
    if exe.starts_with("/usr") {
        bail!("App được cài bằng gói .deb — gỡ bằng: sudo apt remove quick-note");
    }
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("không tìm thấy thư mục home"))?;
    let data_home = dirs::data_dir().unwrap_or_else(|| home.join(".local/share"));

    let mut removed = 0;
    for path in installed_files(&home, &data_home) {
        match std::fs::remove_file(&path) {
            Ok(()) => {
                println!("Đã xoá {}", path.display());
                removed += 1;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("Không xoá được {}: {e}", path.display()),
        }
    }
    if removed == 0 {
        println!("Không thấy file cài đặt nào trong ~/.local (có thể đang chạy bằng cargo run).");
    }

    match (purge, data_dir) {
        (true, Some(dir)) if dir.exists() => purge_data(dir)?,
        (_, Some(dir)) if dir.exists() => {
            println!("Giữ lại ghi chú ở {} (thêm --purge để xoá).", dir.display())
        }
        _ => {}
    }
    Ok(())
}

fn purge_data(dir: &Path) -> Result<()> {
    print!(
        "Xoá VĨNH VIỄN toàn bộ ghi chú, cài đặt và token Drive trong {}? Gõ \"yes\" để xác nhận: ",
        dir.display()
    );
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if answer.trim() != "yes" {
        println!("Đã huỷ, dữ liệu được giữ nguyên.");
        return Ok(());
    }
    std::fs::remove_dir_all(dir)?;
    println!("Đã xoá {}", dir.display());
    println!("Bản trên Google Drive (nếu có) không bị xoá.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Args> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn parses_short_and_long_flags() {
        assert!(p(&["-h"]).unwrap().help);
        assert!(p(&["--version"]).unwrap().version);
        assert!(p(&["-w"]).unwrap().window);
        let a = p(&["--uninstall", "--purge"]).unwrap();
        assert!(a.uninstall && a.purge);
    }

    #[test]
    fn parses_drive_check() {
        assert!(p(&["--drive-check"]).unwrap().drive_check);
    }

    #[test]
    fn rejects_unknown_flag() {
        assert!(p(&["--nope"]).unwrap_err().to_string().contains("--nope"));
    }

    #[test]
    fn purge_requires_uninstall() {
        assert!(p(&["--purge"]).is_err());
    }

    #[test]
    fn installed_files_match_install_script() {
        let files = installed_files(Path::new("/h"), Path::new("/h/.local/share"));
        assert_eq!(files[0], Path::new("/h/.local/bin/quick-note"));
        assert!(files[1].ends_with("applications/quick-note.desktop"));
    }
}
