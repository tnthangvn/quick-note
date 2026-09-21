//! Top bar (page tabs, actions, search) and bottom status bar.

use egui::{
    Align, CollapsingHeader, Color32, Id, Layout, Popup, PopupCloseBehavior, RichText, TextEdit, Ui,
};

use super::Action;
use crate::model::Workspace;

/// Dấu phân cấp trong tên trang: "RV/Deploy" nằm dưới nhóm "RV".
const TREE_SEP: char = '/';

#[derive(Default)]
pub struct ToolbarState {
    pub renaming: Option<(usize, String)>,
    pub search: String,
    pub focus_search: bool,
    /// Từ khoá lọc trong menu chọn trang (khác với `search` là tìm ghi chú).
    pub page_filter: String,
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

            page_menu(ui, ws, state, actions);
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

/// Nút bật/tắt danh sách trang: gom tất cả trang vào một popup cây thư mục có
/// ô tìm kiếm, thay cho hàng tab ngang dễ tràn.
fn page_menu(ui: &mut Ui, ws: &Workspace, state: &mut ToolbarState, actions: &mut Vec<Action>) {
    let active_leaf = ws
        .active_page()
        .title
        .rsplit(TREE_SEP)
        .next()
        .unwrap_or("")
        .trim();
    let label = if active_leaf.is_empty() {
        "🗂 Trang".to_string()
    } else {
        format!("🗂 {active_leaf}  ({})", ws.pages.len())
    };
    let btn = ui
        .button(label)
        .on_hover_text("Danh sách trang (tìm kiếm, cây thư mục)");

    Popup::menu(&btn)
        .id(Id::new("page-tree-popup"))
        .close_behavior(PopupCloseBehavior::CloseOnClickOutside)
        .width(280.0)
        .show(|ui| page_tree_popup(ui, ws, state, actions));
}

fn page_tree_popup(ui: &mut Ui, ws: &Workspace, state: &mut ToolbarState, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let field = ui.add(
            TextEdit::singleline(&mut state.page_filter)
                .hint_text("🔍 Tìm trang")
                .desired_width(200.0),
        );
        field.request_focus();
        if !state.page_filter.is_empty() && ui.small_button("✕").clicked() {
            state.page_filter.clear();
        }
    });
    ui.separator();

    let filter = state.page_filter.trim().to_lowercase();
    let tree = build_tree(ws, &filter);

    egui::ScrollArea::vertical()
        .max_height(360.0)
        .show(ui, |ui| {
            if tree.is_empty() {
                ui.weak("Không có trang khớp.");
            } else {
                render_tree(ui, ws, &tree, !filter.is_empty(), state, actions);
            }
        });

    ui.separator();
    if ui
        .button("➕ Trang mới")
        .on_hover_text("Ctrl+T")
        .clicked()
    {
        actions.push(Action::AddPage);
        Popup::close_all(ui.ctx());
    }
}

/// Một nhánh của cây trang: các nhóm con (theo thứ tự xuất hiện) và các trang lá.
#[derive(Default)]
struct TreeNode {
    groups: Vec<(String, TreeNode)>,
    /// Chỉ số trang trong `ws.pages`.
    leaves: Vec<usize>,
}

impl TreeNode {
    fn is_empty(&self) -> bool {
        self.groups.is_empty() && self.leaves.is_empty()
    }

    fn group_mut(&mut self, seg: &str) -> &mut TreeNode {
        if let Some(pos) = self.groups.iter().position(|(s, _)| s == seg) {
            return &mut self.groups[pos].1;
        }
        self.groups.push((seg.to_string(), TreeNode::default()));
        &mut self.groups.last_mut().unwrap().1
    }
}

/// Dựng cây từ tên trang; `filter` rỗng nghĩa là lấy hết, ngược lại chỉ giữ
/// trang có tên chứa từ khoá (đã lowercase).
fn build_tree(ws: &Workspace, filter: &str) -> TreeNode {
    let mut root = TreeNode::default();
    for (idx, page) in ws.pages.iter().enumerate() {
        if !filter.is_empty() && !page.title.to_lowercase().contains(filter) {
            continue;
        }
        let mut segs: Vec<&str> = page
            .title
            .split(TREE_SEP)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        // Đoạn cuối là lá; các đoạn trước là nhóm. Tên rỗng → coi như lá gốc.
        let _leaf = segs.pop();
        let mut node = &mut root;
        for seg in segs {
            node = node.group_mut(seg);
        }
        node.leaves.push(idx);
    }
    root
}

fn render_tree(
    ui: &mut Ui,
    ws: &Workspace,
    node: &TreeNode,
    force_open: bool,
    state: &mut ToolbarState,
    actions: &mut Vec<Action>,
) {
    for idx in &node.leaves {
        leaf_row(ui, ws, *idx, state, actions);
    }
    for (seg, child) in &node.groups {
        let count = subtree_count(child);
        // Id nằm trong Ui con của nhóm cha nên chỉ cần seg là đủ phân biệt.
        let header = CollapsingHeader::new(format!("📁 {seg}  ({count})"))
            .id_salt(seg.as_str())
            .default_open(true);
        let header = if force_open {
            header.open(Some(true))
        } else {
            header
        };
        header.show(ui, |ui| {
            render_tree(ui, ws, child, force_open, state, actions);
        });
    }
}

fn subtree_count(node: &TreeNode) -> usize {
    node.leaves.len() + node.groups.iter().map(|(_, c)| subtree_count(c)).sum::<usize>()
}

fn leaf_row(
    ui: &mut Ui,
    ws: &Workspace,
    idx: usize,
    state: &mut ToolbarState,
    actions: &mut Vec<Action>,
) {
    let Some(page) = ws.pages.get(idx) else { return };

    if let Some((rename_idx, text)) = state.renaming.as_mut().filter(|(i, _)| *i == idx) {
        let resp = ui.add(TextEdit::singleline(text).desired_width(f32::INFINITY));
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
        return;
    }

    let leaf = page.title.rsplit(TREE_SEP).next().unwrap_or(&page.title).trim();
    let label = format!("{leaf}  ·  {}", page.notes.len());
    let row = ui
        .selectable_label(idx == ws.active, label)
        .on_hover_text("Chuột phải để đổi tên / xoá");
    if row.clicked() {
        actions.push(Action::SelectPage(idx));
        Popup::close_all(ui.ctx());
    }
    if row.double_clicked() {
        state.renaming = Some((idx, page.title.clone()));
    }
    row.context_menu(|ui| {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Page, Workspace};

    fn ws_with(titles: &[&str]) -> Workspace {
        let mut ws = Workspace::default();
        ws.pages = titles.iter().map(|t| Page::new(*t)).collect();
        ws.active = 0;
        ws
    }

    fn leaf_titles<'a>(ws: &'a Workspace, node: &TreeNode, out: &mut Vec<&'a str>) {
        for &idx in &node.leaves {
            out.push(ws.pages[idx].title.as_str());
        }
        for (_, child) in &node.groups {
            leaf_titles(ws, child, out);
        }
    }

    #[test]
    fn groups_by_slash_and_keeps_flat_pages_at_root() {
        let ws = ws_with(&["Temp", "RV/Deploy", "RV/Build", "F3S/api/v1"]);
        let tree = build_tree(&ws, "");
        // Root: one leaf "Temp" + groups RV, F3S.
        assert_eq!(tree.leaves.len(), 1);
        assert_eq!(ws.pages[tree.leaves[0]].title, "Temp");
        let names: Vec<&str> = tree.groups.iter().map(|(s, _)| s.as_str()).collect();
        assert_eq!(names, vec!["RV", "F3S"]);
        // RV holds two leaves; F3S nests api → v1.
        let rv = &tree.groups[0].1;
        assert_eq!(rv.leaves.len(), 2);
        let f3s = &tree.groups[1].1;
        assert_eq!(f3s.groups[0].0, "api");
        assert_eq!(subtree_count(f3s), 1);
    }

    #[test]
    fn filter_keeps_only_matching_pages_case_insensitive() {
        let ws = ws_with(&["Temp", "RV/Deploy", "RV/Build"]);
        let tree = build_tree(&ws, "build");
        let mut got = Vec::new();
        leaf_titles(&ws, &tree, &mut got);
        assert_eq!(got, vec!["RV/Build"]);
    }
}
