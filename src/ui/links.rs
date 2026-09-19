//! Diagram arrows between notes: painting, hover/selection, context menu, and
//! the drag-to-connect gesture.

use std::collections::HashMap;

use egui::{Color32, Pos2, Rect, Response, Shape, Stroke, TextEdit, Ui, Vec2};
use uuid::Uuid;

use crate::model::Page;

/// Pointer must be this close (screen px) to an arrow to hover/select it.
const HIT_DISTANCE: f32 = 6.0;
const ACCENT: Color32 = Color32::from_rgb(255, 138, 0);

#[derive(Default)]
pub struct LinkState {
    /// Note an arrow is being dragged from.
    pub connecting: Option<Uuid>,
    pub selected: Option<Uuid>,
    /// Arrow (if any) the open context menu belongs to, and where it was opened.
    menu: Option<(Option<Uuid>, Pos2)>,
}

#[derive(Default)]
pub struct LinkOutput {
    pub changed: bool,
    pub delete: Option<Uuid>,
    /// Background menu: create a note at this screen position.
    pub new_note_at: Option<Pos2>,
}

/// Paints arrows (call before notes so they sit underneath) and handles
/// clicks / keys / right-click menu on them via the canvas background response.
pub fn show(
    ui: &mut Ui,
    bg: &Response,
    page: &mut Page,
    rects: &HashMap<Uuid, Rect>,
    state: &mut LinkState,
) -> LinkOutput {
    let mut out = LinkOutput::default();
    let zoom = page.zoom;
    let segments: Vec<(Uuid, Pos2, Pos2)> = page
        .links
        .iter()
        .filter_map(|l| {
            let (a, b) = segment(*rects.get(&l.from)?, *rects.get(&l.to)?)?;
            Some((l.id, a, b))
        })
        .collect();

    let hovered = bg
        .hover_pos()
        .and_then(|p| nearest(&segments, p))
        .map(|(id, _, _)| id);
    if hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    if bg.clicked() {
        state.selected = hovered;
    }
    if bg.secondary_clicked() {
        let pos = bg.interact_pointer_pos().unwrap_or_default();
        state.menu = Some((hovered, pos));
        state.selected = hovered;
    }
    let typing = ui.ctx().memory(|m| m.focused().is_some());
    if let Some(id) = state.selected
        && !typing
        && ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
    {
        out.delete = Some(id);
        state.selected = None;
    }

    let base = ui.visuals().text_color().gamma_multiply(0.9);
    let painter = ui.painter();
    for link in &page.links {
        let Some(&(_, a, b)) = segments.iter().find(|(id, _, _)| *id == link.id) else {
            continue;
        };
        let active = Some(link.id) == hovered || Some(link.id) == state.selected;
        let color = if active { ACCENT } else { base };
        let width = if active { 2.8 } else { 2.0 } * zoom.max(0.6);
        paint_arrow(painter, a, b, Stroke::new(width, color), link.dashed, zoom);
        if !link.label.trim().is_empty() {
            paint_label(ui, a.lerp(b, 0.5), link.label.trim(), color, zoom);
        }
    }

    context_menu(bg, page, state, &mut out);
    out
}

fn context_menu(bg: &Response, page: &mut Page, state: &mut LinkState, out: &mut LinkOutput) {
    let Some((target, pos)) = state.menu else {
        return;
    };
    bg.context_menu(|ui| {
        let link = target.and_then(|id| page.links.iter_mut().find(|l| l.id == id));
        let Some(link) = link else {
            if ui.button("+ Ghi chú mới tại đây").clicked() {
                out.new_note_at = Some(pos);
                ui.close();
            }
            ui.label(
                egui::RichText::new("Kéo chấm tròn ở mép note để nối mũi tên")
                    .small()
                    .weak(),
            );
            return;
        };
        ui.horizontal(|ui| {
            ui.label("Nhãn");
            if ui
                .add(TextEdit::singleline(&mut link.label).desired_width(140.0))
                .changed()
            {
                out.changed = true;
            }
        });
        if ui.checkbox(&mut link.dashed, "Nét đứt").changed() {
            out.changed = true;
        }
        if ui.button("⇄ Đổi chiều").clicked() {
            std::mem::swap(&mut link.from, &mut link.to);
            out.changed = true;
            ui.close();
        }
        if ui.button("🗑 Xoá mũi tên (Delete)").clicked() {
            out.delete = Some(link.id);
            ui.close();
        }
    });
    if !bg.context_menu_opened() {
        state.menu = None;
    }
}

/// Draws the arrow being dragged out of a note and connects it on release.
/// Call after the notes so the preview is on top.
pub fn finish_connecting(
    ui: &Ui,
    page: &mut Page,
    rects: &HashMap<Uuid, Rect>,
    under_pointer: Option<Uuid>,
    state: &mut LinkState,
) -> bool {
    let Some(from) = state.connecting else {
        return false;
    };
    let (pointer, released, down) = ui.input(|i| {
        (
            i.pointer.interact_pos(),
            i.pointer.primary_released(),
            i.pointer.primary_down(),
        )
    });
    let (Some(src), Some(p)) = (rects.get(&from), pointer) else {
        state.connecting = None;
        return false;
    };
    let target = under_pointer.filter(|id| *id != from);
    let end = target
        .and_then(|id| rects.get(&id))
        .map_or(p, |r| border_point(*r, src.center()));
    if let Some(r) = target.and_then(|id| rects.get(&id)) {
        ui.painter()
            .rect_stroke(*r, 8, Stroke::new(2.0, ACCENT), egui::StrokeKind::Outside);
    }
    let start = border_point(*src, end);
    paint_arrow(
        ui.painter(),
        start,
        end,
        Stroke::new(2.0, ACCENT),
        true,
        page.zoom,
    );

    if released || !down {
        state.connecting = None;
        return target.is_some_and(|to| page.connect(from, to));
    }
    ui.ctx().request_repaint();
    false
}

/// Edge-to-edge segment between two note rects; `None` when they overlap too much.
fn segment(from: Rect, to: Rect) -> Option<(Pos2, Pos2)> {
    let a = border_point(from, to.center());
    let b = border_point(to, from.center());
    // Arrow would point backwards if the rects overlap along the line.
    ((b - a).dot(to.center() - from.center()) > 1.0).then_some((a, b))
}

/// Where the ray from `rect`'s centre toward `toward` leaves the rect.
pub fn border_point(rect: Rect, toward: Pos2) -> Pos2 {
    let c = rect.center();
    let d = toward - c;
    let half = rect.size() / 2.0;
    let tx = if d.x.abs() > f32::EPSILON {
        half.x / d.x.abs()
    } else {
        f32::INFINITY
    };
    let ty = if d.y.abs() > f32::EPSILON {
        half.y / d.y.abs()
    } else {
        f32::INFINITY
    };
    let t = tx.min(ty);
    if !t.is_finite() || t >= 1.0 {
        return toward;
    }
    c + d * t
}

fn nearest(segments: &[(Uuid, Pos2, Pos2)], p: Pos2) -> Option<(Uuid, Pos2, Pos2)> {
    segments
        .iter()
        .map(|s| (distance_to_segment(p, s.1, s.2), *s))
        .filter(|(d, _)| *d <= HIT_DISTANCE)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, s)| s)
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let len2 = ab.length_sq();
    if len2 <= f32::EPSILON {
        return p.distance(a);
    }
    let t = ((p - a).dot(ab) / len2).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

fn paint_arrow(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke, dashed: bool, zoom: f32) {
    let dir = (b - a).normalized();
    if !dir.is_finite() || dir == Vec2::ZERO {
        return;
    }
    let head = 11.0 * zoom.max(0.6);
    let shaft_end = b - dir * head * 0.8;
    if dashed {
        let (dash, gap) = (7.0 * zoom.max(0.6), 5.0 * zoom.max(0.6));
        painter.extend(Shape::dashed_line(&[a, shaft_end], stroke, dash, gap));
    } else {
        painter.line_segment([a, shaft_end], stroke);
    }
    let side = dir.rot90() * head * 0.45;
    let back = b - dir * head;
    painter.add(Shape::convex_polygon(
        vec![b, back + side, back - side],
        stroke.color,
        Stroke::NONE,
    ));
}

fn paint_label(ui: &Ui, at: Pos2, text: &str, color: Color32, zoom: f32) {
    let font = egui::FontId::proportional(12.0 * zoom.clamp(0.7, 2.0));
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, color);
    let rect = Rect::from_center_size(at, galley.size() + Vec2::new(10.0, 4.0));
    ui.painter().rect_filled(rect, 4, ui.visuals().panel_fill);
    ui.painter()
        .galley(rect.min + Vec2::new(5.0, 2.0), galley, color);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    fn r(x: f32, y: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), egui::vec2(100.0, 50.0))
    }

    #[test]
    fn border_point_exits_on_the_facing_side() {
        let p = border_point(r(0.0, 0.0), pos2(500.0, 25.0));
        assert_eq!(p, pos2(100.0, 25.0));
        let p = border_point(r(0.0, 0.0), pos2(50.0, 500.0));
        assert_eq!(p, pos2(50.0, 50.0));
    }

    #[test]
    fn segment_connects_facing_edges() {
        let (a, b) = segment(r(0.0, 0.0), r(300.0, 0.0)).unwrap();
        assert_eq!((a, b), (pos2(100.0, 25.0), pos2(300.0, 25.0)));
    }

    #[test]
    fn overlapping_notes_have_no_segment() {
        assert!(segment(r(0.0, 0.0), r(10.0, 5.0)).is_none());
    }

    #[test]
    fn distance_to_segment_clamps_to_endpoints() {
        let (a, b) = (pos2(0.0, 0.0), pos2(10.0, 0.0));
        assert_eq!(distance_to_segment(pos2(5.0, 3.0), a, b), 3.0);
        assert_eq!(distance_to_segment(pos2(-4.0, 3.0), a, b), 5.0);
    }
}
