//! Dán ảnh từ clipboard vào note: lưu PNG ra thư mục `attachments/` rồi chèn
//! `![](file://…)` vào nội dung. Ảnh nằm ngoài file JSON để workspace không phình.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use base64::Engine;

pub const ATTACH_DIR: &str = "attachments";

/// Ảnh RGBA lấy từ clipboard.
pub struct Clipped {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Nội dung ảnh lấy được từ clipboard.
pub enum Pasted {
    /// Ảnh bitmap (từ clipboard hoặc PNG đã giải mã).
    Image(Clipped),
    /// File ảnh có sẵn trên đĩa (copy file trong Nautilus, kéo-thả…).
    File(PathBuf),
}

/// Thử mọi cách lấy ảnh: bitmap trong clipboard → PNG qua `wl-paste`
/// → đường dẫn file ảnh (`text/uri-list`). `None` khi clipboard chỉ có chữ.
pub fn clipboard_image() -> Option<Pasted> {
    if let Some(image) = arboard_image() {
        return Some(Pasted::Image(image));
    }
    if let Some(png) = wl_paste(&["--type", "image/png"])
        && let Some(image) = decode(&png)
    {
        return Some(Pasted::Image(image));
    }
    if let Some(uris) = wl_paste(&["--type", "text/uri-list"])
        && let Some(path) = image_from_uris(&String::from_utf8_lossy(&uris))
    {
        return Some(Pasted::File(path));
    }
    // Copy từ trình duyệt hay chỉ cho `text/html` kèm <img src="…">.
    let html = wl_paste(&["--type", "text/html"])?;
    let src = img_src(&String::from_utf8_lossy(&html))?;
    fetch_image(&src)
}

/// Giá trị `src` của thẻ `<img>` đầu tiên trong đoạn HTML.
pub fn img_src(html: &str) -> Option<String> {
    let at = html.find("<img")?;
    let rest = &html[at..];
    let end = rest.find('>')?;
    let tag = &rest[..end];
    let src_at = tag.find("src=")?;
    let value = tag[src_at + 4..].trim_start();
    let quote = value.chars().next()?;
    let value = if quote == '"' || quote == '\'' {
        let inner = &value[1..];
        &inner[..inner.find(quote)?]
    } else {
        value.split_whitespace().next()?
    };
    (!value.is_empty()).then(|| value.to_string())
}

/// Lấy ảnh từ `src`: `data:` (base64), `file://`, hoặc tải qua HTTP.
fn fetch_image(src: &str) -> Option<Pasted> {
    if let Some(rest) = src.strip_prefix("data:") {
        let (_, data) = rest.split_once("base64,")?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data.trim())
            .ok()?;
        return decode(&bytes).map(Pasted::Image);
    }
    if let Some(path) = src.strip_prefix("file://") {
        let path = PathBuf::from(percent_decode(path));
        return is_image(&path).then_some(Pasted::File(path));
    }
    if src.starts_with("http://") || src.starts_with("https://") {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .ok()?;
        let bytes = client.get(src).send().ok()?.bytes().ok()?;
        return decode(&bytes).map(Pasted::Image);
    }
    None
}

/// Các kiểu dữ liệu clipboard đang có (dùng cho `--clipboard-check`).
pub fn clipboard_types() -> Vec<String> {
    wl_paste(&["--list-types"])
        .map(|out| {
            String::from_utf8_lossy(&out)
                .lines()
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn arboard_image() -> Option<Clipped> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let image = clipboard.get_image().ok()?;
    Some(Clipped {
        width: image.width,
        height: image.height,
        rgba: image.bytes.into_owned(),
    })
}

/// `wl-paste` đọc clipboard của compositor — dùng được cả khi app chạy qua
/// XWayland, và xử lý được kiểu dữ liệu mà arboard bỏ qua.
fn wl_paste(args: &[&str]) -> Option<Vec<u8>> {
    let out = std::process::Command::new("wl-paste")
        .arg("--no-newline")
        .args(args)
        .output()
        .ok()?;
    (out.status.success() && !out.stdout.is_empty()).then_some(out.stdout)
}

fn decode(bytes: &[u8]) -> Option<Clipped> {
    let image = image::load_from_memory(bytes).ok()?.to_rgba8();
    Some(Clipped {
        width: image.width() as usize,
        height: image.height() as usize,
        rgba: image.into_raw(),
    })
}

/// File ảnh đầu tiên trong danh sách URI (`file:///…`).
pub fn image_from_uris(uris: &str) -> Option<PathBuf> {
    uris.lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("file://"))
        .map(|path| PathBuf::from(percent_decode(path)))
        .find(|path| is_image(path) && path.is_file())
}

pub fn is_image(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);
    matches!(
        ext.as_deref(),
        Some("png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp")
    )
}

/// `%20` → khoảng trắng (URI từ trình quản lý file được mã hoá).
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1), bytes.get(i + 2)) {
            (b'%', Some(a), Some(b)) => {
                let hex = [*a, *b];
                match u8::from_str_radix(&String::from_utf8_lossy(&hex), 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            _ => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Lưu ảnh thành PNG trong `<dir>/attachments/` và trả về đường dẫn file.
pub fn save_png(dir: &Path, image: &Clipped) -> Result<PathBuf> {
    let attachments = dir.join(ATTACH_DIR);
    std::fs::create_dir_all(&attachments)
        .with_context(|| format!("tạo {}", attachments.display()))?;
    let path = attachments.join(file_name());
    let (w, h) = (u32::try_from(image.width)?, u32::try_from(image.height)?);
    let buffer = image::RgbaImage::from_raw(w, h, image.rgba.clone())
        .context("dữ liệu ảnh trong clipboard không hợp lệ")?;
    buffer
        .save_with_format(&path, image::ImageFormat::Png)
        .with_context(|| format!("ghi {}", path.display()))?;
    Ok(path)
}

fn file_name() -> String {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    format!("anh-{stamp}-{}.png", uuid::Uuid::new_v4().simple())
}

/// Đoạn Markdown chèn vào note. Dùng `file://` để bộ tải ảnh của egui đọc được.
pub fn markdown_for(path: &Path) -> String {
    format!("\n![ảnh](file://{})\n", path.display())
}

/// Đường dẫn các ảnh được note tham chiếu (để đồng bộ/ dọn dẹp).
pub fn referenced_files(body: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(at) = rest.find("](file://") {
        let after = &rest[at + "](file://".len()..];
        match after.find(')') {
            Some(end) => {
                out.push(PathBuf::from(&after[..end]));
                rest = &after[end + 1..];
            }
            None => break,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_image() -> Clipped {
        Clipped {
            width: 2,
            height: 1,
            rgba: vec![255, 0, 0, 255, 0, 255, 0, 255],
        }
    }

    #[test]
    fn luu_png_vao_thu_muc_attachments() {
        let dir = tempfile::tempdir().unwrap();
        let path = save_png(dir.path(), &tiny_image()).unwrap();
        assert!(path.starts_with(dir.path().join(ATTACH_DIR)));
        assert_eq!(path.extension().unwrap(), "png");

        let decoded = image::open(&path).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 1));
    }

    #[test]
    fn bao_loi_khi_kich_thuoc_khong_khop_du_lieu() {
        let dir = tempfile::tempdir().unwrap();
        let broken = Clipped {
            width: 10,
            height: 10,
            rgba: vec![0; 8],
        };
        assert!(save_png(dir.path(), &broken).is_err());
    }

    #[test]
    fn lay_duoc_file_anh_tu_uri_list() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ảnh có dấu.png");
        save_png(dir.path(), &tiny_image()).unwrap();
        std::fs::copy(
            std::fs::read_dir(dir.path().join(ATTACH_DIR))
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path(),
            &path,
        )
        .unwrap();
        let uris = format!(
            "file://{}\r\n",
            path.display().to_string().replace(' ', "%20")
        );
        assert_eq!(image_from_uris(&uris).as_deref(), Some(path.as_path()));
    }

    #[test]
    fn bo_qua_uri_khong_phai_anh() {
        assert_eq!(image_from_uris("file:///etc/hostname\n"), None);
        assert_eq!(image_from_uris("https://x.dev/a.png"), None);
    }

    #[test]
    fn nhan_dien_duoi_file_anh() {
        assert!(is_image(Path::new("/a/b.PNG")));
        assert!(is_image(Path::new("/a/b.jpeg")));
        assert!(!is_image(Path::new("/a/b.txt")));
        assert!(!is_image(Path::new("/a/b")));
    }

    #[test]
    fn lay_duoc_src_cua_the_img() {
        let html = r#"<meta charset='utf-8'><img src="https://x.dev/a.png" alt="x">"#;
        assert_eq!(img_src(html).as_deref(), Some("https://x.dev/a.png"));

        let single = "<div><img class='big' src='file:///tmp/a.png'/></div>";
        assert_eq!(img_src(single).as_deref(), Some("file:///tmp/a.png"));

        assert_eq!(img_src("<p>không có ảnh</p>"), None);
        assert_eq!(img_src("<img alt='thiếu src'>"), None);
    }

    #[test]
    fn doc_duoc_anh_data_uri() {
        let mut png = Vec::new();
        image::RgbaImage::from_raw(2, 1, tiny_image().rgba)
            .unwrap()
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&png);
        let html = format!("<img src=\"data:image/png;base64,{b64}\">");
        let src = img_src(&html).unwrap();
        match fetch_image(&src) {
            Some(Pasted::Image(img)) => assert_eq!((img.width, img.height), (2, 1)),
            _ => panic!("phải giải mã được ảnh trong data URI"),
        }
    }

    #[test]
    fn markdown_tro_dung_duong_dan() {
        let md = markdown_for(Path::new("/home/me/.config/quick-note/attachments/a.png"));
        assert!(md.contains("![ảnh](file:///home/me/.config/quick-note/attachments/a.png)"));
    }

    #[test]
    fn doc_lai_duoc_danh_sach_anh_trong_note() {
        let body = "trước\n![ảnh](file:///tmp/a.png)\ngiữa ![x](file:///tmp/b.png) sau\n[link](https://x.dev)";
        assert_eq!(
            referenced_files(body),
            [PathBuf::from("/tmp/a.png"), PathBuf::from("/tmp/b.png")]
        );
    }

    #[test]
    fn bo_qua_doan_thieu_dau_dong() {
        assert!(referenced_files("![ảnh](file:///tmp/a.png").is_empty());
    }
}
