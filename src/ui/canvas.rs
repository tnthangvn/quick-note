//! Infinite-ish canvas: pan by dragging the background, notes are dragged by
//! their header and resized from the bottom-right grip.

use egui::{
    Align, Color32, CornerRadius, CursorIcon, Id, Label, Layout, Rect, RichText, Sense, Stroke,
    StrokeKind, TextEdit, Ui, UiBuilder, Vec2, epaint::Shadow, vec2,
};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use uuid::Uuid;

use super::links::{self, LinkState};
use crate::model::{MIN_NOTE_SIZE, NOTE_COLORS, Note, Page, ZOOM_STEP, now_ts};

const HEADER_H: f32 = 28.0;
const GRIP: f32 = 16.0;
const GRID_STEP: f32 = 28.0;
const RADIUS: u8 = 8;
const ACCENT: Color32 = Color32::from_rgb(255, 138, 0);
const INK: Color32 = Color32::from_gray(35);

/// Transient UI state that survives between frames but is not persisted.
#[derive(Default)]
pub struct CanvasState {
    pub editing_title: Option<Uuid>,
    pub focus_body: Option<Uuid>,
    /// Note whose Markdown source is open for editing (others show rendered).
    pub editing_body: Option<Uuid>,
    md_cache: CommonMarkCache,
    /// Canvas size from the last frame (for toolbar zoom around the centre).
    pub last_size: Vec2,
    pub links: LinkState,
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
    /// The user double-clicked this note (and it was the topmost one there).
    double_clicked: bool,
}

#[derive(Default)]
pub struct CanvasOutput {
    pub changed: bool,
    pub delete: Option<Uuid>,
    pub delete_link: Option<Uuid>,
}

#[derive(Default)]
struct NoteOutput {
    changed: bool,
    delete: bool,
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

    let raise = topmost_pressed(ui, page, origin);
    let double_clicked = ui
        .input(|i| {
            i.pointer
                .button_double_clicked(egui::PointerButton::Primary)
                .then(|| i.pointer.interact_pos())
                .flatten()
        })
        .and_then(|pos| topmost_at(page, origin, pos));
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
            double_clicked: double_clicked == Some(note.id),
        };
        let res = note_ui(ui, canvas, &view, note, state);
        out.changed |= res.changed;
        if res.delete {
            out.delete = Some(note.id);
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
    paint_note_frame(&ui, rect, zoom, note, matched == Some(true));

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
            let text = RichText::new(note.display_title()).strong().color(INK);
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
    if view.markdown && view.double_clicked && ui.rect_contains_pointer(body) {
        state.editing_body = Some(note.id);
        state.focus_body = Some(note.id);
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
    let resp = ui.add(
        TextEdit::multiline(&mut note.body)
            .frame(egui::Frame::NONE)
            .background_color(Color32::TRANSPARENT)
            .text_color(INK)
            .hint_text(hint)
            .desired_width(f32::INFINITY)
            .min_size(ui.available_size()),
    );
    if focus_now {
        resp.request_focus();
        state.focus_body = None;
    }
    if resp.changed() {
        note.updated_at = now_ts();
        out.changed = true;
    }
    // Click elsewhere or Esc: back to the rendered view.
    if markdown && resp.lost_focus() && state.editing_body == Some(note.id) {
        state.editing_body = None;
    }
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
    let active = links.connecting == Some(id);
    if !near && !active {
        return;
    }
    let radius = (6.0 * zoom).max(4.0);
    let centre = egui::pos2(rect.right(), rect.center().y);
    let hit = Rect::from_center_size(centre, Vec2::splat(radius * 3.0));
    let resp = ui.interact(hit, ui.id().with("connect"), Sense::drag());
    if resp.drag_started() {
        links.connecting = Some(id);
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
