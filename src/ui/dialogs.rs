//! Settings window and pull-confirmation modal.

use egui::{Context, Grid, Id, Modal, RichText, TextEdit};

use crate::config::Config;
use crate::model::Workspace;

pub enum SettingsResult {
    Keep,
    Save(Config),
    Cancel,
    Logout,
}

const CONSOLE_URL: &str = "https://console.cloud.google.com/apis/credentials";

/// `draft` is edited in place; nothing is applied until "Lưu".
pub fn settings(
    ctx: &Context,
    draft: &mut Config,
    connected: bool,
    monitors: &[String],
) -> SettingsResult {
    let mut result = SettingsResult::Keep;
    let mut open = true;
    egui::Window::new("⚙ Cài đặt")
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(460.0)
        .show(ctx, |ui| {
            ui.heading("Google Drive");
            ui.label(RichText::new(
                "Tạo OAuth client loại \"Desktop app\" trong Google Cloud Console, bật Google Drive API, rồi dán vào đây.",
            ).small());
            ui.hyperlink_to("Mở Google Cloud Console ↗", CONSOLE_URL);
            ui.add_space(6.0);
            Grid::new("drive-grid").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                ui.label("Client ID");
                ui.add(TextEdit::singleline(&mut draft.drive.client_id).desired_width(300.0));
                ui.end_row();
                ui.label("Client secret");
                ui.add(TextEdit::singleline(&mut draft.drive.client_secret).password(true).desired_width(300.0));
                ui.end_row();
                ui.label("Tên thư mục");
                ui.add(TextEdit::singleline(&mut draft.drive.folder_name).desired_width(300.0));
                ui.end_row();
                ui.label("Folder ID (tuỳ chọn)");
                ui.add(
                    TextEdit::singleline(&mut draft.drive.folder_id)
                        .hint_text("để trống = app tự tạo thư mục")
                        .desired_width(300.0),
                );
                ui.end_row();
            });
            ui.label(RichText::new(
                "Folder ID lấy từ URL drive.google.com/drive/folders/<ID>. Dùng folder có sẵn cần quyền Drive đầy đủ; để trống thì app chỉ thấy file nó tạo (an toàn hơn).",
            ).small().weak());
            ui.checkbox(&mut draft.drive.export_markdown, "Kèm file .md cho từng trang (đọc được trên Drive/điện thoại)");

            ui.add_space(8.0);
            ui.heading("Chung");
            ui.horizontal(|ui| {
                ui.label("Tự đồng bộ mỗi");
                ui.add(egui::DragValue::new(&mut draft.auto_sync_minutes).range(0..=240).suffix(" phút"));
                ui.label(RichText::new("(0 = tắt)").weak());
            });
            ui.checkbox(&mut draft.markdown, "Hiển thị Markdown (tiêu đề, **đậm**, danh sách, - [ ] checkbox, link)");
            ui.horizontal(|ui| {
                ui.label("Cỡ chữ");
                ui.add(egui::Slider::new(&mut draft.font_size, 11.0..=24.0));
            });

            ui.add_space(8.0);
            ui.heading("Sticky góc màn hình");
            ui.checkbox(
                &mut draft.dock.enabled,
                "Thu nhỏ thành sticky ở góc dưới phải, hover để mở (cần mở lại app)",
            );
            ui.add_enabled_ui(draft.dock.enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Rộng khi mở");
                    ui.add(
                        egui::DragValue::new(&mut draft.dock.width)
                            .range(crate::config::DockConfig::MIN_WIDTH..=3840.0)
                            .suffix(" px"),
                    );
                    ui.label("Cỡ sticky");
                    ui.add(egui::DragValue::new(&mut draft.dock.handle_size).range(20.0..=120.0).suffix(" px"));
                });
                ui.horizontal(|ui| {
                    ui.label("Màn hình");
                    monitor_picker(ui, &mut draft.dock.monitor, monitors);
                });
                ui.horizontal(|ui| {
                    ui.label("Thu gọn sau khi rời chuột");
                    ui.add(egui::DragValue::new(&mut draft.dock.collapse_delay_ms).range(0..=5000).suffix(" ms"));
                    ui.label("Hiệu ứng");
                    ui.add(egui::DragValue::new(&mut draft.dock.animation_ms).range(0..=1000).suffix(" ms"));
                });
            });
            ui.label(RichText::new("Chạy `quick-note -w` để mở dạng cửa sổ thường.").small().weak());

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("💾 Lưu").clicked() {
                    result = SettingsResult::Save(draft.clone());
                }
                if ui.button("Huỷ").clicked() {
                    result = SettingsResult::Cancel;
                }
                if connected && ui.button("Ngắt kết nối Drive").clicked() {
                    result = SettingsResult::Logout;
                }
            });
        });
    if !open {
        return SettingsResult::Cancel;
    }
    result
}

const AUTO_MONITOR: &str = "Tự động (màn ngoài cùng bên phải)";

fn monitor_picker(ui: &mut egui::Ui, selected: &mut String, monitors: &[String]) {
    let current = if selected.is_empty() {
        AUTO_MONITOR
    } else {
        selected.as_str()
    };
    egui::ComboBox::from_id_salt("dock-monitor")
        .selected_text(current.to_string())
        .show_ui(ui, |ui| {
            ui.selectable_value(selected, String::new(), AUTO_MONITOR);
            for name in monitors {
                ui.selectable_value(selected, name.clone(), name);
            }
        });
}

pub enum PullChoice {
    Pending,
    Replace,
    Cancel,
}

pub fn confirm_pull(ctx: &Context, remote: &Workspace, local: &Workspace) -> PullChoice {
    let mut choice = PullChoice::Pending;
    let modal = Modal::new(Id::new("confirm-pull")).show(ctx, |ui| {
        ui.set_width(380.0);
        ui.heading("Thay bằng bản trên Drive?");
        let count = |ws: &Workspace| ws.pages.iter().map(|p| p.notes.len()).sum::<usize>();
        ui.label(format!(
            "Drive: {} trang, {} ghi chú — sửa lúc {}",
            remote.pages.len(),
            count(remote),
            fmt_ts(remote.updated_at)
        ));
        ui.label(format!(
            "Local: {} trang, {} ghi chú — sửa lúc {}",
            local.pages.len(),
            count(local),
            fmt_ts(local.updated_at)
        ));
        ui.label(
            RichText::new("Bản local sẽ được backup vào thư mục backups/ trước khi thay.")
                .small()
                .weak(),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui.button("Thay bằng bản Drive").clicked() {
                choice = PullChoice::Replace;
            }
            if ui.button("Huỷ").clicked() {
                choice = PullChoice::Cancel;
            }
        });
    });
    if modal.should_close()
        && let PullChoice::Pending = choice
    {
        return PullChoice::Cancel;
    }
    choice
}

pub fn fmt_ts(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|t| {
            t.with_timezone(&chrono::Local)
                .format("%H:%M %d/%m/%Y")
                .to_string()
        })
        .unwrap_or_else(|| "?".into())
}
