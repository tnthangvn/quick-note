//! Infinite-ish canvas: pan by dragging the background, notes are dragged by
//! their header and resized from the bottom-right grip.

use egui::{
    Align, Color32, CornerRadius, CursorIcon, Id, Label, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, TextEdit, Ui, UiBuilder, Vec2, epaint::Shadow, vec2,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use uuid::Uuid;

use super::links::{self, LinkState};
use super::md_highlight;
use crate::model::{MIN_NOTE_SIZE, NOTE_COLORS, Note, Page, ZOOM_STEP, now_ts};

const HEADER_H: f32 = 28.0;
const GRIP: f32 = 16.0;
const GRID_STEP: f32 = 28.0;
const RADIUS: u8 = 8;
const ACCENT: Color32 = Color32::from_rgb(255, 138, 0);
/// Viền sáng sau khi nhảy tới một note qua neo.
const FLASH_TIME: std::time::Duration = std::time::Duration::from_millis(1600);
const INK: Color32 = Color32::from_gray(35);

/// Id của ô soạn nội dung note — cần cố định để đọc lại vùng bôi đen.
fn body_id(note: Uuid) -> Id {
    Id::new(("note-body", note))
}

/// Vị trí con trỏ (theo ký tự) trong phần nội dung của note.
pub fn cursor_index(ctx: &egui::Context, note: Uuid) -> Option<usize> {
    let state = egui::TextEdit::load_state(ctx, body_id(note))?;
    Some(state.cursor.char_range()?.primary.index.0)
}

/// Vùng bôi đen hiện tại trong note, đọc thẳng từ trạng thái ô soạn thảo.
/// Không phụ thuộc thứ tự vẽ, và còn nguyên khi ô mất focus.
pub fn selected_range(ctx: &egui::Context, note: Uuid) -> Option<Selection> {
    let state = egui::TextEdit::load_state(ctx, body_id(note))?;
    let chars = state.cursor.char_range()?.as_sorted_char_range();
    let (start, end) = (chars.start.0, chars.end.0);
    (start < end).then_some(Selection { note, start, end })
}

/// Transient UI state that survives between frames but is not persisted.
#[derive(Default)]
pub struct CanvasState {
    pub editing_title: Option<Uuid>,
    pub focus_body: Option<Uuid>,
    /// Note whose Markdown source is open for editing (others show rendered).
    pub editing_body: Option<Uuid>,
    md_cache: CommonMarkCache,
    /// Đoạn text đang bôi đen trong một note (để tách thành note con).
    pub selection: Option<Selection>,
    /// Note vừa được nhảy tới: viền sáng trong chốc lát.
    pub flash: Option<(Uuid, std::time::Instant)>,
    /// Canvas size from the last frame (for toolbar zoom around the centre).
    pub last_size: Vec2,
    pub links: LinkState,
}

/// Vùng bôi đen trong phần thân một note, tính theo ký tự.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub note: Uuid,
    pub start: usize,
    pub end: usize,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}

pub struct CanvasOptions<'a> {
    pub search: &'a str,
    /// Render note bodies as Markdown when not editing.
    pub markdown: bool,
}

/// Per-note rendering inputs.
struct NoteView {
    rect: Rect,
    zoom: f32,
    /// `Some(false)` = a search is active and this note doesn't match.
    matched: Option<bool>,
    markdown: bool,
}

#[derive(Default)]
pub struct CanvasOutput {
    pub changed: bool,
    /// Dán ảnh từ clipboard vào note này.
    pub paste_image: Option<Uuid>,
    /// Tách phần bôi đen của note này thành note con.
    pub split: Option<Uuid>,
    pub delete: Option<Uuid>,
    pub delete_link: Option<Uuid>,
}

#[derive(Default)]
struct NoteOutput {
    changed: bool,
    delete: bool,
    split: bool,
    paste_image: bool,
    /// Chuyển note sang chế độ sửa (để bôi đen được).
    edit: bool,
}

pub fn show(
    ui: &mut Ui,
    page: &mut Page,
    state: &mut CanvasState,
    opts: &CanvasOptions,
) -> CanvasOutput {
    let canvas = ui.max_rect();
    state.last_size = canvas.size();
    let mut out = CanvasOutput::default();

    // Background first so every note widget sits on top of it for hit-testing.
    let bg = ui.interact(canvas, ui.id().with("canvas-bg"), Sense::click_and_drag());
    if bg.dragged() {
        let d = bg.drag_delta();
        page.pan = [page.pan[0] + d.x, page.pan[1] + d.y];
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        out.changed = true;
    }
    out.changed |= wheel_zoom(ui, canvas, page);

    let z = page.zoom;
    let origin = canvas.min + vec2(page.pan[0], page.pan[1]);
    if bg.double_clicked()
        && let Some(p) = bg.interact_pointer_pos()
    {
        add_note_at_screen(page, canvas, p, state);
        out.changed = true;
    }

    match state.flash {
        Some((_, at)) if at.elapsed() >= FLASH_TIME => state.flash = None,
        Some((_, at)) => ui.ctx().request_repaint_after(FLASH_TIME - at.elapsed()),
        None => {}
    }

    paint_grid(ui, canvas, page.pan, z);
    if page.notes.is_empty() {
        paint_empty_hint(ui, canvas);
    }

    // Arrows go under the notes.
    let rects: std::collections::HashMap<Uuid, Rect> = page
        .notes
        .iter()
        .map(|n| (n.id, note_rect(n, origin, z)))
        .collect();
    let link_out = links::show(ui, &bg, page, &rects, &mut state.links);
    out.changed |= link_out.changed;
    out.delete_link = link_out.delete;
    if let Some(p) = link_out.new_note_at {
        add_note_at_screen(page, canvas, p, state);
        out.changed = true;
    }

    // Nhớ vùng bôi đen gần nhất: khi bấm nút », ô soạn thảo xử lý cú nhấn
    // trước và xoá vùng chọn, nên không thể đọc "sống" tại thời điểm đó.
    if let Some(sel) = state
        .editing_body
        .and_then(|id| selected_range(ui.ctx(), id))
    {
        state.selection = Some(sel);
    }

    let raise = topmost_pressed(ui, page, origin);
    let query = opts.search.trim();
    for note in page.notes.iter_mut() {
        let rect = note_rect(note, origin, z);
        if !rect.intersects(canvas) {
            continue;
        }
        let view = NoteView {
            rect,
            zoom: z,
            matched: (!query.is_empty()).then(|| note.matches(query)),
            markdown: opts.markdown,
        };
        let res = note_ui(ui, canvas, &view, note, state);
        out.changed |= res.changed;
        if res.delete {
            out.delete = Some(note.id);
        }
        if res.split {
            out.split = Some(note.id);
        }
        if res.paste_image {
            out.paste_image = Some(note.id);
        }
        if res.edit {
            state.editing_body = Some(note.id);
            state.focus_body = Some(note.id);
        }
    }
    let under_pointer = ui
        .input(|i| i.pointer.interact_pos())
        .and_then(|p| topmost_at_rects(page, &rects, p));
    if links::finish_connecting(ui, page, &rects, under_pointer, &mut state.links) {
        out.changed = true;
    }
    if let Some(id) = raise {
        page.bring_to_front(id);
    }
    out
}

fn add_note_at_screen(page: &mut Page, canvas: Rect, p: egui::Pos2, state: &mut CanvasState) {
    let [x, y] = page.to_canvas((p - canvas.min).into());
    let id = page.add_note_at([x - 20.0, y - HEADER_H / 2.0]);
    state.focus_body = Some(id);
}

fn topmost_at_rects(
    page: &Page,
    rects: &std::collections::HashMap<Uuid, Rect>,
    pos: egui::Pos2,
) -> Option<Uuid> {
    page.notes
        .iter()
        .rev()
        .find(|n| rects.get(&n.id).is_some_and(|r| r.contains(pos)))
        .map(|n| n.id)
}

/// Ctrl + scroll (or pinch) zooms around the pointer.
fn wheel_zoom(ui: &Ui, canvas: Rect, page: &mut Page) -> bool {
    let (factor, pointer) = ui.input(|i| (i.zoom_delta(), i.pointer.hover_pos()));
    let Some(p) = pointer.filter(|p| canvas.contains(*p)) else {
        return false;
    };
    if (factor - 1.0).abs() < f32::EPSILON {
        return false;
    }
    page.zoom_at((p - canvas.min).into(), page.zoom * factor);
    true
}

/// Toolbar / keyboard zoom: one step in or out around the canvas centre.
pub fn zoom_step(page: &mut Page, state: &CanvasState, steps: i32) {
    let centre = (state.last_size / 2.0).into();
    page.zoom_at(centre, page.zoom * ZOOM_STEP.powi(steps));
}

fn note_rect(note: &Note, origin: egui::Pos2, zoom: f32) -> Rect {
    Rect::from_min_size(
        origin + vec2(note.pos[0], note.pos[1]) * zoom,
        vec2(note.size[0], note.size[1]) * zoom,
    )
}

/// The note under the pointer when the primary button is pressed this frame,
/// unless it is already on top.
fn topmost_pressed(ui: &Ui, page: &Page, origin: egui::Pos2) -> Option<Uuid> {
    let pos = ui.input(|i| {
        i.pointer
            .primary_pressed()
            .then(|| i.pointer.interact_pos())
            .flatten()
    })?;
    topmost_at(page, origin, pos).filter(|id| page.notes.last().map(|n| n.id) != Some(*id))
}

fn topmost_at(page: &Page, origin: egui::Pos2, pos: egui::Pos2) -> Option<Uuid> {
    page.notes
        .iter()
        .rev()
        .find(|n| note_rect(n, origin, page.zoom).contains(pos))
        .map(|n| n.id)
}

fn note_ui(
    parent: &mut Ui,
    canvas: Rect,
    view: &NoteView,
    note: &mut Note,
    state: &mut CanvasState,
) -> NoteOutput {
    let (rect, zoom, matched) = (view.rect, view.zoom, view.matched);
    let mut out = NoteOutput::default();
    let id = Id::new(("note", note.id));
    let mut ui = parent.new_child(
        UiBuilder::new()
            .id_salt(id)
            .max_rect(rect)
            .layout(Layout::top_down(Align::Min)),
    );
    ui.set_clip_rect(canvas);
    ui.style_mut().visuals = egui::Visuals::light();
    // The default fade uses the panel colour, which shows as a dark band on pastel notes.
    ui.style_mut().spacing.scroll.fade.strength = 0.0;
    scale_style(ui.style_mut(), zoom);
    if matched == Some(false) {
        ui.multiply_opacity(0.3);
    }

    // Swallow clicks on the note's padding so notes underneath don't react.
    ui.interact(rect, id.with("block"), Sense::click());
    let flashing = state
        .flash
        .is_some_and(|(id, at)| id == note.id && at.elapsed() < FLASH_TIME);
    paint_note_frame(&ui, rect, zoom, note, matched == Some(true) || flashing);

    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H * zoom));
    header_ui(&mut ui, header, view, note, state, &mut out);

    let body = Rect::from_min_max(
        header.left_bottom() + vec2(10.0, 2.0) * zoom,
        rect.max - vec2(8.0, 8.0) * zoom,
    );
    body_ui(&mut ui, body, view, note, state, &mut out);

    resize_grip(&mut ui, rect, zoom, note, &mut out);
    connect_handle(&mut ui, rect, zoom, note.id, &mut state.links);
    out
}

/// Scales fonts and spacing so a zoomed note looks like a scaled picture of itself.
fn scale_style(style: &mut egui::Style, zoom: f32) {
    if (zoom - 1.0).abs() < f32::EPSILON {
        return;
    }
    for font in style.text_styles.values_mut() {
        font.size *= zoom;
    }
    let spacing = &mut style.spacing;
    spacing.item_spacing *= zoom;
    spacing.button_padding *= zoom;
    spacing.interact_size *= zoom;
    spacing.icon_width *= zoom;
    spacing.icon_width_inner *= zoom;
}

fn paint_note_frame(ui: &Ui, rect: Rect, zoom: f32, note: &Note, highlight: bool) {
    let [r, g, b] = NOTE_COLORS[note.color];
    let fill = Color32::from_rgb(r, g, b);
    let shadow = Shadow {
        offset: [2, 5],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(70),
    };
    let painter = ui.painter();
    painter.add(shadow.as_shape(rect, CornerRadius::same(RADIUS)));
    let stroke = if highlight {
        Stroke::new(2.5, ACCENT)
    } else {
        Stroke::new(1.0, fill.gamma_multiply(0.75))
    };
    painter.rect(
        rect,
        CornerRadius::same(RADIUS),
        fill,
        stroke,
        StrokeKind::Inside,
    );
    // Slightly darker header band.
    let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER_H * zoom));
    let band = CornerRadius {
        nw: RADIUS,
        ne: RADIUS,
        sw: 0,
        se: 0,
    };
    painter.rect_filled(header.shrink(1.0), band, Color32::from_black_alpha(14));
}

fn header_ui(
    ui: &mut Ui,
    header: Rect,
    view: &NoteView,
    note: &mut Note,
    state: &mut CanvasState,
    out: &mut NoteOutput,
) {
    let zoom = view.zoom;
    let drag = ui.interact(header, ui.id().with("drag"), Sense::click_and_drag());
    if !note.pinned {
        if drag.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
        if drag.dragged() {
            let d = drag.drag_delta() / zoom;
            note.pos = [note.pos[0] + d.x, note.pos[1] + d.y];
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            out.changed = true;
        }
    }
    if drag.double_clicked() {
        state.editing_title = Some(note.id);
    }

    let inner = header.shrink2(vec2(8.0, 2.0) * zoom);
    let mut bar = ui.new_child(
        UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::right_to_left(Align::Center)),
    );
    bar.spacing_mut().item_spacing.x = 2.0;
    if bar
        .small_button("×")
        .on_hover_text("Xoá ghi chú (Ctrl+Z để hoàn tác)")
        .clicked()
    {
        out.delete = true;
    }
    let pin_tip = if note.pinned {
        "Đang khoá vị trí — bấm để mở"
    } else {
        "Khoá vị trí & kích thước"
    };
    // Ở chế độ xem thì chưa bôi đen được: bấm » lần đầu để mở chế độ sửa.
    let can_split = selected_range(ui.ctx(), note.id).is_some();
    let editing_now = state.editing_body == Some(note.id) || !view.markdown;
    let tip = if can_split {
        "Tách phần bôi đen thành note con (Ctrl+Shift+T)"
    } else if editing_now {
        "Bôi đen một đoạn trong note rồi bấm nút này"
    } else {
        "Mở chế độ sửa để bôi đen và tách"
    };
    if bar.small_button("»").on_hover_text(tip).clicked() {
        if can_split {
            out.split = true;
        } else {
            out.edit = true;
        }
    }
    if view.markdown {
        let mut editing = state.editing_body == Some(note.id);
        let tip = if editing {
            "Xem dạng Markdown (Esc)"
        } else {
            "Sửa nội dung (hoặc nhấp đúp)"
        };
        if bar
            .toggle_value(&mut editing, "✏")
            .on_hover_text(tip)
            .clicked()
        {
            state.editing_body = editing.then_some(note.id);
            state.focus_body = editing.then_some(note.id);
        }
    }
    if bar
        .toggle_value(&mut note.pinned, "📌")
        .on_hover_text(pin_tip)
        .changed()
    {
        out.changed = true;
    }
    bar.menu_button("🎨", |ui| {
        ui.horizontal(|ui| {
            for (idx, [r, g, b]) in NOTE_COLORS.iter().enumerate() {
                let (swatch, resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                let stroke = if idx == note.color {
                    Stroke::new(2.0, INK)
                } else {
                    Stroke::new(1.0, Color32::GRAY)
                };
                ui.painter()
                    .circle(swatch.center(), 9.0, Color32::from_rgb(*r, *g, *b), stroke);
                if resp.clicked() {
                    note.color = idx;
                    out.changed = true;
                    ui.close();
                }
            }
        });
    });

    bar.with_layout(Layout::left_to_right(Align::Center), |ui| {
        if state.editing_title == Some(note.id) {
            let resp = ui.add(
                TextEdit::singleline(&mut note.title)
                    .hint_text("Tiêu đề…")
                    .desired_width(f32::INFINITY)
                    .text_color(INK),
            );
            if !resp.has_focus() && !resp.lost_focus() {
                resp.request_focus();
            }
            if resp.changed() {
                note.updated_at = now_ts();
                out.changed = true;
            }
            if resp.lost_focus() {
                state.editing_title = None;
            }
        } else {
            let label = match &note.ticket {
                Some(ticket) => format!("{ticket} · {}", note.display_title()),
                None => note.display_title().to_string(),
            };
            let text = RichText::new(label).strong().color(INK);
            // Non-interactive label so dragging the header still works over the title.
            ui.add(
                Label::new(text)
                    .selectable(false)
                    .sense(Sense::hover())
                    .truncate(),
            )
            .on_hover_text("Nhấp đúp để đổi tiêu đề");
        }
    });
}

fn body_ui(
    ui: &mut Ui,
    body: Rect,
    view: &NoteView,
    note: &mut Note,
    state: &mut CanvasState,
    out: &mut NoteOutput,
) {
    let editing_before = !view.markdown || state.editing_body == Some(note.id);
    // Đăng ký TRƯỚC phần nội dung: link và checkbox vẽ sau nên vẫn bắt được
    // cú nhấn của chúng, phần nền còn lại thì mở chế độ sửa.
    let background = (!editing_before)
        .then(|| ui.interact(body, Id::new(("body-click", note.id)), Sense::click()));
    if let Some(resp) = &background {
        if resp.clicked() || resp.double_clicked() {
            state.editing_body = Some(note.id);
            state.focus_body = Some(note.id);
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Text);
        }
    }
    let focus_now = state.focus_body == Some(note.id);
    if focus_now && view.markdown {
        state.editing_body = Some(note.id);
    }
    let editing = !view.markdown || state.editing_body == Some(note.id);

    let mut area = ui.new_child(
        UiBuilder::new()
            .max_rect(body)
            .layout(Layout::top_down(Align::Min)),
    );
    egui::ScrollArea::vertical()
        .id_salt(("note-scroll", note.id))
        .auto_shrink(false)
        .show(&mut area, |ui| {
            if editing {
                edit_body(ui, note, state, view.markdown, focus_now, out);
            } else {
                render_body(ui, note, state, out);
            }
        });
}

fn edit_body(
    ui: &mut Ui,
    note: &mut Note,
    state: &mut CanvasState,
    markdown: bool,
    focus_now: bool,
    out: &mut NoteOutput,
) {
    let hint = if markdown {
        "Ghi chú nhanh… (hỗ trợ Markdown)"
    } else {
        "Ghi chú nhanh…"
    };
    let mut layouter = |ui: &Ui, text: &dyn egui::TextBuffer, wrap: f32| {
        let job = md_highlight::layout(ui, text.as_str(), wrap, INK);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let output = TextEdit::multiline(&mut note.body)
        .id(body_id(note.id))
        .layouter(&mut layouter)
        .frame(egui::Frame::NONE)
        .background_color(Color32::TRANSPARENT)
        .text_color(INK)
        .hint_text(hint)
        .desired_width(f32::INFINITY)
        .min_size(ui.available_size())
        .show(ui);
    let resp = &output.response;
    if focus_now {
        resp.request_focus();
        state.focus_body = None;
    }
    if resp.changed() {
        note.updated_at = now_ts();
        out.changed = true;
    }
    if resp.has_focus() {
        continue_list(ui, note, &output, out);
        // Ctrl+V: ô soạn thảo tự dán chữ. Không "ăn" phím tắt ở đây — chỉ báo
        // cho app kiểm tra xem clipboard có ảnh không.
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::V)) {
            out.paste_image = true;
        }
    }
    if split_button(ui, &output, note.id, state.selection) {
        out.split = true;
    }
    // Click elsewhere or Esc: back to the rendered view.
    if markdown && resp.lost_focus() && state.editing_body == Some(note.id) {
        state.editing_body = None;
    }
}

/// Bấm Enter ở cuối một mục danh sách thì tự thêm dấu đầu dòng cho dòng mới.
fn continue_list(
    ui: &Ui,
    note: &mut Note,
    output: &egui::text_edit::TextEditOutput,
    out: &mut NoteOutput,
) {
    if !ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        return;
    }
    let Some(range) = output.cursor_range else {
        return;
    };
    let at = range.primary.index.0;
    // Con trỏ vừa xuống dòng: dòng phía trên là mục vừa gõ xong.
    let before: String = note.body.chars().take(at).collect();
    let Some(previous) = before.trim_end_matches('\n').rsplit('\n').next() else {
        return;
    };
    let Some(marker) = md_highlight::continued_marker(previous) else {
        return;
    };
    let byte = before.len();
    note.body.insert_str(byte, &marker);
    note.updated_at = now_ts();
    out.changed = true;

    let cursor = egui::text::CCursor::new(at + marker.chars().count());
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), body_id(note.id)) {
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(cursor)));
        state.store(ui.ctx(), body_id(note.id));
    }
}

/// Nút tròn nổi ngay sau đoạn bôi đen: bấm là tách thành note con.
fn split_button(
    ui: &mut Ui,
    output: &egui::text_edit::TextEditOutput,
    note: Uuid,
    remembered: Option<Selection>,
) -> bool {
    let live = output
        .cursor_range
        .map(|r| r.as_sorted_char_range())
        .filter(|r| r.start < r.end)
        .map(|r| r.end.0);
    // Sau cú nhấn, vùng chọn "sống" biến mất — vẫn giữ nút ở chỗ cũ.
    let Some(end) = live.or_else(|| remembered.filter(|s| s.note == note).map(|s| s.end)) else {
        return false;
    };
    let caret = output.galley.pos_from_cursor(egui::text::CCursor::new(end));
    let centre = output.galley_pos + caret.right_center().to_vec2() + vec2(12.0, 0.0);
    let rect = Rect::from_center_size(centre, Vec2::splat(20.0));

    let resp = ui
        .interact(rect, Id::new(("split-btn", note)), Sense::click())
        .on_hover_text("Tách đoạn này thành note con (Ctrl+Shift+T)");
    let fill = if resp.hovered() {
        ACCENT
    } else {
        ACCENT.gamma_multiply(0.75)
    };
    let painter = ui.painter();
    painter.circle(rect.center(), 10.0, fill, Stroke::new(1.0, INK));
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "»",
        egui::FontId::proportional(13.0),
        Color32::WHITE,
    );
    resp.clicked()
}

fn render_body(ui: &mut Ui, note: &mut Note, state: &mut CanvasState, out: &mut NoteOutput) {
    if note.body.trim().is_empty() {
        ui.label(RichText::new("Nhấp đúp để viết…").color(INK.gamma_multiply(0.45)));
        return;
    }
    // `show_mut` lets task-list checkboxes (`- [ ]`) be ticked right in the preview.
    let resp = CommonMarkViewer::new().show_mut(ui, &mut state.md_cache, &mut note.body);
    if resp.response.changed() {
        note.updated_at = now_ts();
        out.changed = true;
    }
}

/// Dot on the right edge; drag it onto another note to draw an arrow.
fn connect_handle(ui: &mut Ui, rect: Rect, zoom: f32, id: Uuid, links: &mut LinkState) {
    let near = ui.rect_contains_pointer(rect.expand(14.0 * zoom));
    let active = links.is_connecting_from(id);
    if !near && !active {
        return;
    }
    let radius = (6.0 * zoom).max(4.0);
    // One dot per side, sitting just outside the note so they never cover the
    // header (which would swallow drags meant to move the note).
    let out = radius * 0.9;
    let sides = [
        ("top", rect.center_top() - vec2(0.0, out)),
        ("right", rect.right_center() + vec2(out, 0.0)),
        ("bottom", rect.center_bottom() + vec2(0.0, out)),
        ("left", rect.left_center() - vec2(out, 0.0)),
    ];
    for (side, centre) in sides {
        let hit = Rect::from_center_size(centre, Vec2::splat(radius * 2.2));
        let resp = ui.interact(hit, ui.id().with(("connect", side)), Sense::drag());
        if resp.drag_started() {
            links.start_connecting(id);
        }
        if resp.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        }
        let resp = resp.on_hover_text("Kéo sang note khác để nối mũi tên");
        let fill = if resp.hovered() || active {
            ACCENT
        } else {
            Color32::WHITE
        };
        ui.painter()
            .circle(centre, radius, fill, Stroke::new(1.5, ACCENT));
    }
}

fn resize_grip(ui: &mut Ui, rect: Rect, zoom: f32, note: &mut Note, out: &mut NoteOutput) {
    if note.pinned {
        return;
    }
    // Grip stays grabbable even when zoomed far out.
    let grip = Rect::from_min_max(rect.max - Vec2::splat((GRIP * zoom).max(10.0)), rect.max);
    let resp = ui.interact(grip, ui.id().with("grip"), Sense::drag());
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeNwSe);
    }
    if resp.dragged() {
        let d = resp.drag_delta() / zoom;
        note.size = [
            (note.size[0] + d.x).max(MIN_NOTE_SIZE[0]),
            (note.size[1] + d.y).max(MIN_NOTE_SIZE[1]),
        ];
        out.changed = true;
    }
    let color = if resp.hovered() {
        INK
    } else {
        Color32::from_black_alpha(90)
    };
    let painter = ui.painter();
    for k in [4.0, 8.0, 12.0] {
        let (k, edge) = (k * zoom, 3.0 * zoom);
        painter.line_segment(
            [rect.max - vec2(k, edge), rect.max - vec2(edge, k)],
            Stroke::new(1.2, color),
        );
    }
}

fn paint_grid(ui: &Ui, canvas: Rect, pan: [f32; 2], zoom: f32) {
    let painter = ui.painter_at(canvas);
    let color = ui.visuals().weak_text_color().gamma_multiply(0.35);
    // Double the spacing when zoomed out so the dots never turn into noise.
    let mut step = GRID_STEP * zoom;
    while step < 14.0 {
        step *= 2.0;
    }
    let start_x = canvas.left() + pan[0].rem_euclid(step);
    let start_y = canvas.top() + pan[1].rem_euclid(step);
    let mut y = start_y;
    while y < canvas.bottom() {
        let mut x = start_x;
        while x < canvas.right() {
            painter.circle_filled(egui::pos2(x, y), 1.1, color);
            x += step;
        }
        y += step;
    }
}

fn paint_empty_hint(ui: &Ui, canvas: Rect) {
    ui.painter_at(canvas).text(
        canvas.center(),
        egui::Align2::CENTER_CENTER,
        "Nhấp đúp vào nền hoặc Ctrl+N để tạo ghi chú\nKéo nền để di chuyển khung nhìn · Ctrl+cuộn chuột để zoom",
        egui::FontId::proportional(16.0),
        ui.visuals().weak_text_color(),
    );
}
