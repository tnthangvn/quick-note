//! Tô màu Markdown ngay trong ô soạn thảo (kiểu "source mode" của Obsidian):
//! văn bản vẫn là Markdown thô, chỉ khác là được hiển thị có định dạng.
//!
//! Phần quét cú pháp tách riêng khỏi egui để test được bằng chuỗi thường.

use std::ops::Range;

use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, FontFamily, FontId, Stroke, TextStyle, Ui};

/// Kiểu hiển thị cho một đoạn trong văn bản.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Span {
    Heading(u8),
    Bold,
    Italic,
    Code,
    Link,
    /// Dấu đầu dòng `-`, `*`, `1.`, hoặc ô `[ ]` / `[x]`.
    Marker,
    Quote,
}

/// Các đoạn có định dạng, theo thứ tự xuất hiện và không chồng nhau.
pub fn spans(text: &str) -> Vec<(Range<usize>, Span)> {
    let mut out = Vec::new();
    for (start, line) in line_offsets(text) {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let body_start = start + indent;

        if let Some(level) = heading_level(trimmed) {
            out.push((body_start..start + line.len(), Span::Heading(level)));
            continue;
        }
        if trimmed.starts_with('>') {
            out.push((body_start..start + line.len(), Span::Quote));
            continue;
        }
        let mut inline_from = body_start;
        if let Some(len) = list_marker(trimmed) {
            out.push((body_start..body_start + len, Span::Marker));
            inline_from = body_start + len;
        }
        inline_spans(text, inline_from..start + line.len(), &mut out);
    }
    out
}

fn line_offsets(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut offset = 0;
    text.split_inclusive('\n').map(move |line| {
        let start = offset;
        offset += line.len();
        (start, line.trim_end_matches('\n'))
    })
}

fn heading_level(line: &str) -> Option<u8> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    (1..=6).contains(&hashes).then_some(hashes as u8)?;
    line[hashes..].starts_with(' ').then_some(hashes as u8)
}

/// Độ dài dấu đầu dòng, kể cả ô checkbox: `- `, `* `, `1. `, `- [x] `.
fn list_marker(line: &str) -> Option<usize> {
    let bullet = if line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ") {
        2
    } else {
        let digits = line.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && line[digits..].starts_with(". ") {
            digits + 2
        } else {
            return None;
        }
    };
    let rest = &line[bullet..];
    let checkbox = ["[ ] ", "[x] ", "[X] "]
        .iter()
        .find(|b| rest.starts_with(**b))
        .map_or(0, |b| b.len());
    Some(bullet + checkbox)
}

/// Quét `**đậm**`, `*nghiêng*`, `` `code` `` và `[nhãn](link)` trong một dòng.
fn inline_spans(text: &str, range: Range<usize>, out: &mut Vec<(Range<usize>, Span)>) {
    let line = &text[range.clone()];
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &line[i..];
        let found = delimited(rest, "**", Span::Bold)
            .or_else(|| delimited(rest, "*", Span::Italic))
            .or_else(|| delimited(rest, "`", Span::Code))
            .or_else(|| link(rest));
        match found {
            Some((len, span)) => {
                out.push((range.start + i..range.start + i + len, span));
                i += len;
            }
            None => i += next_char_len(rest),
        }
    }
}

/// Đoạn mở/đóng bằng cùng một dấu, ví dụ `**đậm**`. Trả về độ dài cả dấu.
fn delimited(rest: &str, mark: &str, span: Span) -> Option<(usize, Span)> {
    let inner = rest.strip_prefix(mark)?;
    let end = inner.find(mark)?;
    (end > 0).then_some((mark.len() * 2 + end, span))
}

fn link(rest: &str) -> Option<(usize, Span)> {
    let after_label = rest.strip_prefix('[')?;
    let label_end = after_label.find(']')?;
    let after = &after_label[label_end + 1..];
    if !after.starts_with('(') {
        return None;
    }
    let url_end = after.find(')')?;
    Some((1 + label_end + 1 + url_end + 1, Span::Link))
}

fn next_char_len(rest: &str) -> usize {
    rest.chars().next().map_or(1, char::len_utf8)
}

/// Dựng bố cục có định dạng cho `TextEdit` (gọi trong `layouter`).
pub fn layout(ui: &Ui, text: &str, wrap_width: f32, ink: Color32) -> LayoutJob {
    let base = ui
        .style()
        .text_styles
        .get(&TextStyle::Body)
        .cloned()
        .unwrap_or(FontId::proportional(14.0));
    let mut job = LayoutJob {
        wrap: egui::text::TextWrapping {
            max_width: wrap_width,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut cursor = 0;
    for (range, span) in spans(text) {
        if range.start > cursor {
            job.append(&text[cursor..range.start], 0.0, plain(&base, ink));
        }
        job.append(&text[range.clone()], 0.0, format(span, &base, ink));
        cursor = range.end;
    }
    if cursor < text.len() {
        job.append(&text[cursor..], 0.0, plain(&base, ink));
    }
    job
}

fn plain(base: &FontId, ink: Color32) -> TextFormat {
    TextFormat {
        font_id: base.clone(),
        color: ink,
        ..Default::default()
    }
}

fn format(span: Span, base: &FontId, ink: Color32) -> TextFormat {
    let mut fmt = plain(base, ink);
    match span {
        Span::Heading(level) => {
            let scale = 1.0 + 0.45 / f32::from(level);
            fmt.font_id = FontId::new(base.size * scale, FontFamily::Proportional);
            fmt.color = ink;
        }
        Span::Bold => fmt.color = Color32::BLACK,
        Span::Italic => fmt.italics = true,
        Span::Code => {
            fmt.font_id = FontId::new(base.size * 0.95, FontFamily::Monospace);
            fmt.background = Color32::from_black_alpha(20);
        }
        Span::Link => {
            fmt.color = Color32::from_rgb(20, 90, 190);
            fmt.underline = Stroke::new(1.0, Color32::from_rgb(20, 90, 190));
        }
        Span::Marker => fmt.color = Color32::from_rgb(190, 100, 0),
        Span::Quote => fmt.color = ink.gamma_multiply(0.7),
    }
    fmt
}

/// Dấu đầu dòng cần lặp lại khi bấm Enter ở cuối một mục danh sách.
/// `- x` → `- `, `- [x] x` → `- [ ] `, `3. x` → `4. `. `None` nếu dòng trống
/// hoặc không phải danh sách.
pub fn continued_marker(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let len = list_marker(trimmed)?;
    let (marker, rest) = (&trimmed[..len], trimmed[len..].trim());
    if rest.is_empty() {
        return None; // mục rỗng: Enter để thoát danh sách
    }
    let marker = match marker.split_once(". ") {
        Some((digits, tail)) => {
            let next: u32 = digits.parse().ok()?;
            format!("{}. {tail}", next + 1)
        }
        None => marker.replace("[x]", "[ ]").replace("[X]", "[ ]"),
    };
    Some(format!("{indent}{marker}"))
}

/// Dọn đoạn chữ dán từ trình duyệt: `<br>` thành xuống dòng, ký tự HTML thành
/// ký tự thật. Giữ nguyên mọi thứ khác (không phải bộ chuyển HTML đầy đủ).
pub fn clean_pasted(text: &str) -> String {
    const BREAKS: [&str; 4] = ["<br>", "<br/>", "<br />", "<BR>"];
    let mut out = text.to_string();
    for tag in BREAKS {
        out = out.replace(tag, "\n");
    }
    out.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(text: &str) -> Vec<(&str, Span)> {
        spans(text)
            .into_iter()
            .map(|(r, s)| (&text[r], s))
            .collect()
    }

    #[test]
    fn nhan_dien_tieu_de() {
        assert_eq!(kinds("## Hôm nay"), [("## Hôm nay", Span::Heading(2))]);
        assert!(kinds("#khong-phai").is_empty(), "thiếu dấu cách");
        assert!(kinds("####### quá nhiều").is_empty());
    }

    #[test]
    fn nhan_dien_dau_dau_dong_va_checkbox() {
        assert_eq!(kinds("- việc")[0], ("- ", Span::Marker));
        assert_eq!(kinds("- [x] xong")[0], ("- [x] ", Span::Marker));
        assert_eq!(kinds("12. mục")[0], ("12. ", Span::Marker));
        assert!(kinds("-khong-phai").is_empty());
    }

    #[test]
    fn nhan_dien_dinh_dang_trong_dong() {
        assert_eq!(kinds("họp **9h** nhé")[0], ("**9h**", Span::Bold));
        assert_eq!(kinds("*nghiêng*")[0], ("*nghiêng*", Span::Italic));
        assert_eq!(kinds("chạy `cargo test`")[0], ("`cargo test`", Span::Code));
        assert_eq!(
            kinds("[egui](https://docs.rs)")[0],
            ("[egui](https://docs.rs)", Span::Link)
        );
    }

    #[test]
    fn dam_duoc_uu_tien_hon_nghieng() {
        assert_eq!(kinds("**x**")[0].1, Span::Bold);
    }

    #[test]
    fn khong_cat_nham_chu_tieng_viet() {
        // Nếu tính theo byte thì "đ" (2 byte) sẽ làm lệch chỉ số.
        let text = "đơn đội **step 2** của phong trào";
        assert_eq!(kinds(text)[0], ("**step 2**", Span::Bold));
    }

    #[test]
    fn cac_doan_khong_chong_nhau_va_dung_thu_tu() {
        let text = "# Tiêu đề\n- [ ] làm `x` và **y**\n> trích";
        let found = spans(text);
        assert!(
            found
                .windows(2)
                .all(|w| w[0].1 == w[1].1 || w[0].0.end <= w[1].0.start)
        );
        let labels: Vec<Span> = found.iter().map(|(_, s)| *s).collect();
        assert_eq!(
            labels,
            [
                Span::Heading(1),
                Span::Marker,
                Span::Code,
                Span::Bold,
                Span::Quote
            ]
        );
    }

    #[test]
    fn don_the_br_va_ky_tu_html_khi_dan() {
        assert_eq!(clean_pasted("A<br>B<br />C"), "A\nB\nC");
        assert_eq!(clean_pasted("a &amp; b&nbsp;c"), "a & b c");
        assert_eq!(clean_pasted("&lt;div&gt;"), "<div>");
        assert_eq!(
            clean_pasted("giữ nguyên chữ thường"),
            "giữ nguyên chữ thường"
        );
    }

    #[test]
    fn giai_ma_amp_sau_cung_de_khong_dich_hai_lan() {
        // "&amp;lt;" phải ra "&lt;", không phải "<".
        assert_eq!(clean_pasted("&amp;lt;"), "&lt;");
    }

    #[test]
    fn tu_noi_dau_dau_dong() {
        assert_eq!(continued_marker("- việc").as_deref(), Some("- "));
        assert_eq!(continued_marker("  * việc").as_deref(), Some("  * "));
        assert_eq!(continued_marker("- [x] xong").as_deref(), Some("- [ ] "));
        assert_eq!(continued_marker("3. ba").as_deref(), Some("4. "));
        assert_eq!(
            continued_marker("- ").as_deref(),
            None,
            "mục rỗng thì thoát"
        );
        assert_eq!(continued_marker("văn bản thường"), None);
    }

    #[test]
    fn dau_le_loi_khong_gay_hoang_loan() {
        assert!(kinds("chỉ có một * dấu sao").is_empty());
        assert!(kinds("`chưa đóng").is_empty());
        assert!(kinds("[nhãn](chưa đóng").is_empty());
        assert!(kinds("****").is_empty(), "không có nội dung bên trong");
    }
}
