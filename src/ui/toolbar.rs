//! Top bar (page tabs, actions, search) and bottom status bar.

use egui::{Align, Color32, Layout, RichText, TextEdit, Ui};

use super::Action;
use crate::model::Workspace;

#[derive(Default)]
pub struct ToolbarState {
    pub renaming: Option<(usize, String)>,
    pub search: String,
    pub focus_search: bool,
    /// Some(pinned) when running in dock mode.
    pub dock_pinned: Option<bool>,
}

pub struct DriveView<'a> {
    pub connected: bool,
    pub configured: bool,
    pub busy: Option<&'a str>,
}

pub fn top_bar(
    ui: &mut Ui,
    ws: &Workspace,
    state: &mut ToolbarState,
    drive: &DriveView,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("📝 Quick Note").strong());
        ui.separator();

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if let Some(pinned) = state.dock_pinned {
                if ui.button("×").on_hover_text("Thoát app (Ctrl+Q)").clicked() {
                    actions.push(Action::Quit);
                }
                let tip = if pinned {
                    "Đang giữ mở — bấm để tự thu gọn khi rời chuột"
                } else {
                    "Giữ panel mở"
                };
                if ui
                    .selectable_label(pinned, "📌")
                    .on_hover_text(tip)
                    .clicked()
                {
                    actions.push(Action::ToggleDockPin);
                }
            }
            if ui.button("⚙").on_hover_text("Cài đặt").clicked() {
                actions.push(Action::OpenSettings);
            }
            drive_buttons(ui, drive, actions);
            ui.separator();
            let search = ui.add(
                TextEdit::singleline(&mut state.search)
                    .hint_text("🔍 Tìm (Ctrl+F)")
                    .desired_width(160.0),
            );
            if std::mem::take(&mut state.focus_search) {
                search.request_focus();
            }
            zoom_buttons(ui, ws.active_page().zoom, actions);
            if ui
                .button("Sắp xếp")
                .on_hover_text("Sắp xếp gọn ghi chú")
                .clicked()
            {
                actions.push(Action::Tidy);
            }
            if ui.button("+ Ghi chú").on_hover_text("Ctrl+N").clicked() {
                actions.push(Action::AddNote);
            }
            ui.separator();

            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("page-tabs")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| page_tabs(ui, ws, state, actions));
                    });
            });
        });
    });
}

fn zoom_buttons(ui: &mut Ui, zoom: f32, actions: &mut Vec<Action>) {
    // Right-to-left layout: added in reverse visual order.
    if ui
        .small_button("+")
        .on_hover_text("Phóng to (Ctrl+=, Ctrl+cuộn)")
        .clicked()
    {
        actions.push(Action::Zoom(1));
    }
    let label = format!("{:.0}%", zoom * 100.0);
    if ui
        .small_button(label)
        .on_hover_text("Về 100% (Ctrl+0)")
        .clicked()
    {
        actions.push(Action::ZoomReset);
    }
    if ui
        .small_button("−")
        .on_hover_text("Thu nhỏ (Ctrl+-, Ctrl+cuộn)")
        .clicked()
    {
        actions.push(Action::Zoom(-1));
    }
}

fn page_tabs(ui: &mut Ui, ws: &Workspace, state: &mut ToolbarState, actions: &mut Vec<Action>) {
    for (idx, page) in ws.pages.iter().enumerate() {
        if let Some((rename_idx, text)) = state.renaming.as_mut().filter(|(i, _)| *i == idx) {
            let resp = ui.add(TextEdit::singleline(text).desired_width(110.0));
            if !resp.has_focus() && !resp.lost_focus() {
                resp.request_focus();
            }
            if resp.lost_focus() {
                let cancelled = ui.input(|i| i.key_pressed(egui::Key::Escape));
                if !cancelled && !text.trim().is_empty() {
                    actions.push(Action::RenamePage(*rename_idx, text.trim().to_string()));
                }
                state.renaming = None;
            }
            continue;
        }

        let label = format!("{}  {}", page.title, page.notes.len());
        let tab = ui
            .selectable_label(idx == ws.active, label)
            .on_hover_text("Nhấp đúp để đổi tên · chuột phải để xoá");
        if tab.clicked() {
            actions.push(Action::SelectPage(idx));
        }
        if tab.double_clicked() {
            state.renaming = Some((idx, page.title.clone()));
        }
        tab.context_menu(|ui| {
            if ui.button("✏ Đổi tên").clicked() {
                state.renaming = Some((idx, page.title.clone()));
                ui.close();
            }
            let can_delete = ws.pages.len() > 1;
            if ui
                .add_enabled(can_delete, egui::Button::new("🗑 Xoá trang"))
                .clicked()
            {
                actions.push(Action::DeletePage(idx));
                ui.close();
            }
        });
    }
    if ui
        .button(" + ")
        .on_hover_text("Trang mới (Ctrl+T)")
        .clicked()
    {
        actions.push(Action::AddPage);
    }
}

fn drive_buttons(ui: &mut Ui, drive: &DriveView, actions: &mut Vec<Action>) {
    if let Some(msg) = drive.busy {
        ui.spinner().on_hover_text(msg);
        return;
    }
    if drive.connected {
        if ui
            .button("⬇ Tải về")
            .on_hover_text("Lấy bản trên Drive thay bản local")
            .clicked()
        {
            actions.push(Action::Pull);
        }
        if ui
            .button("⬆ Đồng bộ")
            .on_hover_text("Đẩy lên Google Drive (Ctrl+S)")
            .clicked()
        {
            actions.push(Action::Push);
        }
    } else {
        let tip = if drive.configured {
            "Đăng nhập Google để lưu lên Drive"
        } else {
            "Nhập OAuth client trong Cài đặt trước"
        };
        if ui.button("☁ Kết nối Drive").on_hover_text(tip).clicked() {
            actions.push(if drive.configured {
                Action::Login
            } else {
                Action::OpenSettings
            });
        }
    }
}

pub struct Status<'a> {
    pub saved: bool,
    pub unsynced: bool,
    pub drive_line: &'a str,
    pub toast: Option<(&'a str, bool)>,
    pub can_undo: bool,
}

pub fn status_bar(ui: &mut Ui, ws: &Workspace, status: &Status, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let (text, color) = if status.saved {
            ("✔ Đã lưu", Color32::from_rgb(76, 175, 80))
        } else {
            ("… Đang lưu", Color32::from_rgb(255, 170, 0))
        };
        ui.label(RichText::new(text).color(color).small());
        ui.separator();
        let total: usize = ws.pages.iter().map(|p| p.notes.len()).sum();
        ui.label(RichText::new(format!("{} trang · {} ghi chú", ws.pages.len(), total)).small());
        ui.separator();
        let sync = if status.unsynced {
            format!("{} · có thay đổi chưa đồng bộ", status.drive_line)
        } else {
            status.drive_line.to_string()
        };
        ui.label(RichText::new(sync).small().weak());

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .small_button("⬇ Xuất .md")
                .on_hover_text("Xuất trang hiện tại ra Markdown")
                .clicked()
            {
                actions.push(Action::ExportMarkdown);
            }
            if status.can_undo && ui.small_button("↶ Hoàn tác xoá").clicked() {
                actions.push(Action::Undo);
            }
            if let Some((msg, is_error)) = status.toast {
                let color = if is_error {
                    Color32::from_rgb(229, 57, 53)
                } else {
                    ui.visuals().text_color()
                };
                ui.label(RichText::new(msg).small().color(color));
            }
        });
    });
}
