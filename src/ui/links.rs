//! Diagram arrows between notes: painting, hover/selection, context menu, and
//! the drag-to-connect gesture.

use std::collections::HashMap;

use egui::{Color32, Pos2, Rect, Response, Shape, Stroke, TextEdit, Ui, Vec2};
use uuid::Uuid;

use crate::model::{LinkStyle, Page};

/// Pointer must be this close (screen px) to an arrow to hover/select it.
const HIT_DISTANCE: f32 = 6.0;
/// Speed of the marching dashes, points per second (positive = toward the target).
const DASH_SPEED: f32 = 26.0;
/// Frame pacing for the dash animation (keeps an idle app cheap).
const DASH_FRAME: f32 = 1.0 / 30.0;
const ACCENT: Color32 = Color32::from_rgb(255, 138, 0);

/// A drag that produced no release event (pointer left the window, panel
/// collapsed mid-drag) is dropped after this long instead of drawing forever.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

#[derive(Default)]
pub struct LinkState {
    /// Note an arrow is being dragged from, and when the drag started.
    connecting: Option<(Uuid, std::time::Instant)>,
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
    let routes: Vec<(Uuid, Vec<Pos2>)> = page
        .links
        .iter()
        .filter_map(|l| {
            Some((
                l.id,
                route(*rects.get(&l.from)?, *rects.get(&l.to)?, l.style)?,
            ))
        })
        .collect();

    let hovered = bg.hover_pos().and_then(|p| nearest(&routes, p));
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
    // Dashes march while the window is in use (hover counts: the dock panel opens
    // on hover without taking focus), so an idle background window stays cheap.
    let (time, in_use) = ui.input(|i| (i.time as f32, i.focused || i.pointer.has_pointer()));
    let animate = in_use && page.links.iter().any(|l| l.dashed);
    if animate {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f32(DASH_FRAME));
    }
    let dash_offset = if animate { -time * DASH_SPEED } else { 0.0 };
    let painter = ui.painter();
    for link in &page.links {
        let Some((_, path)) = routes.iter().find(|(id, _)| *id == link.id) else {
            continue;
        };
        let active = Some(link.id) == hovered || Some(link.id) == state.selected;
        let color = if active { ACCENT } else { base };
        let width = if active { 2.8 } else { 2.0 } * zoom.max(0.6);
        let style = ArrowStyle {
            stroke: Stroke::new(width, color),
            zoom,
            dashed: link.dashed,
            dash_offset,
        };
        paint_arrow(painter, path, &style);
        if !link.label.trim().is_empty() {
            paint_label(ui, midpoint(path), link.label.trim(), color, zoom);
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
        ui.horizontal(|ui| {
            ui.label("Kiểu");
            for style in LinkStyle::ALL {
                if ui
                    .selectable_label(link.style == style, style.label())
                    .clicked()
                {
                    link.style = style;
                    out.changed = true;
                }
            }
        });
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
impl LinkState {
    pub fn start_connecting(&mut self, from: Uuid) {
        self.connecting = Some((from, std::time::Instant::now()));
    }

    pub fn is_connecting_from(&self, note: Uuid) -> bool {
        self.connecting.is_some_and(|(id, _)| id == note)
    }
}

pub fn finish_connecting(
    ui: &Ui,
    page: &mut Page,
    rects: &HashMap<Uuid, Rect>,
    under_pointer: Option<Uuid>,
    state: &mut LinkState,
) -> bool {
    let Some((from, started)) = state.connecting else {
        return false;
    };
    let (pointer, released, down, has_pointer, cancel) = ui.input(|i| {
        (
            i.pointer.interact_pos(),
            i.pointer.primary_released(),
            i.pointer.primary_down(),
            i.pointer.has_pointer(),
            i.key_pressed(egui::Key::Escape) || i.pointer.secondary_pressed(),
        )
    });
    // When the pointer leaves the window egui deliberately keeps `interact_pos`
    // and `primary_down` (so slider drags survive), and the release event never
    // arrives — so `has_pointer` is the only reliable "drag is over" signal.
    let lost = !has_pointer || pointer.is_none() || started.elapsed() > CONNECT_TIMEOUT;
    if cancel || lost {
        state.connecting = None;
        return false;
    }
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
    let path = [start, end];
    let style = ArrowStyle {
        stroke: Stroke::new(2.0, ACCENT),
        zoom: page.zoom,
        dashed: true,
        dash_offset: -ui.input(|i| i.time as f32) * DASH_SPEED,
    };
    paint_arrow(ui.painter(), &path, &style);

    if released || !down {
        state.connecting = None;
        return target.is_some_and(|to| page.connect(from, to));
    }
    ui.ctx().request_repaint();
    false
}

/// Path between two note rects, in the given style. `None` when they overlap.
fn route(from: Rect, to: Rect, style: LinkStyle) -> Option<Vec<Pos2>> {
    match style {
        LinkStyle::Straight => {
            let (a, b) = segment(from, to)?;
            Some(vec![a, b])
        }
        LinkStyle::Elbow => elbow(from, to),
        LinkStyle::Curve => {
            let (a, b) = segment(from, to)?;
            Some(bezier(a, b))
        }
    }
}

/// Edge-to-edge straight segment; `None` when the rects overlap too much.
fn segment(from: Rect, to: Rect) -> Option<(Pos2, Pos2)> {
    let a = border_point(from, to.center());
    let b = border_point(to, from.center());
    // Arrow would point backwards if the rects overlap along the line.
    ((b - a).dot(to.center() - from.center()) > 1.0).then_some((a, b))
}

/// Right-angle route: leaves and enters through facing edge centres, bending
/// halfway. Keeps diagrams readable instead of one long diagonal.
fn elbow(from: Rect, to: Rect) -> Option<Vec<Pos2>> {
    let (c1, c2) = (from.center(), to.center());
    let (dx, dy) = ((c2.x - c1.x), (c2.y - c1.y));
    if from.expand(1.0).intersects(to) {
        return None;
    }
    if dx.abs() >= dy.abs() {
        let (ax, bx) = if dx >= 0.0 {
            (from.right(), to.left())
        } else {
            (from.left(), to.right())
        };
        let (a, b) = (egui::pos2(ax, c1.y), egui::pos2(bx, c2.y));
        let mid = (a.x + b.x) / 2.0;
        Some(vec![a, egui::pos2(mid, a.y), egui::pos2(mid, b.y), b])
    } else {
        let (ay, by) = if dy >= 0.0 {
            (from.bottom(), to.top())
        } else {
            (from.top(), to.bottom())
        };
        let (a, b) = (egui::pos2(c1.x, ay), egui::pos2(c2.x, by));
        let mid = (a.y + b.y) / 2.0;
        Some(vec![a, egui::pos2(a.x, mid), egui::pos2(b.x, mid), b])
    }
}

/// Cubic bezier sampled into a polyline (so dashes and hit-testing work on it).
fn bezier(a: Pos2, b: Pos2) -> Vec<Pos2> {
    const STEPS: usize = 24;
    let d = b - a;
    // Bulge sideways along the dominant axis.
    let (c1, c2) = if d.x.abs() >= d.y.abs() {
        (
            egui::pos2(a.x + d.x * 0.5, a.y),
            egui::pos2(b.x - d.x * 0.5, b.y),
        )
    } else {
        (
            egui::pos2(a.x, a.y + d.y * 0.5),
            egui::pos2(b.x, b.y - d.y * 0.5),
        )
    };
    (0..=STEPS)
        .map(|i| {
            let t = i as f32 / STEPS as f32;
            let (u, t2, u2) = (1.0 - t, t * t, (1.0 - t) * (1.0 - t));
            let w = [u2 * u, 3.0 * u2 * t, 3.0 * u * t2, t2 * t];
            let x = w[0] * a.x + w[1] * c1.x + w[2] * c2.x + w[3] * b.x;
            let y = w[0] * a.y + w[1] * c1.y + w[2] * c2.y + w[3] * b.y;
            egui::pos2(x, y)
        })
        .collect()
}

/// Point halfway along the path (by length), for the label.
fn midpoint(path: &[Pos2]) -> Pos2 {
    let total: f32 = path.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut walked = 0.0;
    for w in path.windows(2) {
        let len = w[0].distance(w[1]);
        if walked + len >= total / 2.0 && len > 0.0 {
            return w[0].lerp(w[1], (total / 2.0 - walked) / len);
        }
        walked += len;
    }
    *path.last().unwrap_or(&Pos2::ZERO)
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

fn nearest(routes: &[(Uuid, Vec<Pos2>)], p: Pos2) -> Option<Uuid> {
    routes
        .iter()
        .map(|(id, path)| (distance_to_path(p, path), *id))
        .filter(|(d, _)| *d <= HIT_DISTANCE)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

fn distance_to_path(p: Pos2, path: &[Pos2]) -> f32 {
    path.windows(2)
        .map(|w| distance_to_segment(p, w[0], w[1]))
        .fold(f32::INFINITY, f32::min)
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

struct ArrowStyle {
    stroke: Stroke,
    zoom: f32,
    dashed: bool,
    /// Shifts the dash pattern; animating it makes the dashes march.
    dash_offset: f32,
}

fn paint_arrow(painter: &egui::Painter, path: &[Pos2], style: &ArrowStyle) {
    let (Some(&end), Some(&before)) = (path.last(), path.get(path.len().wrapping_sub(2))) else {
        return;
    };
    let dir = (end - before).normalized();
    if !dir.is_finite() || dir == Vec2::ZERO {
        return;
    }
    let scale = style.zoom.max(0.6);
    let head = 11.0 * scale;
    // Stop the shaft just short of the arrow head.
    let mut shaft = path.to_vec();
    if let Some(last) = shaft.last_mut() {
        *last = end - dir * head * 0.8;
    }
    if style.dashed {
        let (dash, gap) = (7.0 * scale, 5.0 * scale);
        painter.extend(Shape::dashed_line_with_offset(
            &shaft,
            style.stroke,
            &[dash],
            &[gap],
            wrapped_offset(style.dash_offset, dash + gap),
        ));
    } else {
        painter.add(Shape::line(shaft, style.stroke));
    }
    let side = dir.rot90() * head * 0.45;
    let back = end - dir * head;
    painter.add(Shape::convex_polygon(
        vec![end, back + side, back - side],
        style.stroke.color,
        Stroke::NONE,
    ));
}

/// epaint starts laying dashes at `dash_offset` **along** the path, so a negative
/// (or ever-growing) offset paints dashes before the first point — a tail that
/// keeps getting longer. Wrapping into one dash+gap period keeps the marching
/// animation but never draws outside the path.
fn wrapped_offset(offset: f32, period: f32) -> f32 {
    if period <= 0.0 || !offset.is_finite() {
        return 0.0;
    }
    offset.rem_euclid(period)
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
    fn elbow_leaves_and_enters_through_facing_edges() {
        let path = elbow(r(0.0, 0.0), r(300.0, 200.0)).unwrap();
        assert_eq!(path.len(), 4);
        assert_eq!(path[0], pos2(100.0, 25.0), "exits the right edge");
        assert_eq!(
            *path.last().unwrap(),
            pos2(300.0, 225.0),
            "enters the left edge"
        );
        // Middle segment is vertical at the halfway x.
        assert_eq!(path[1].x, path[2].x);
        assert_eq!(path[1].x, 200.0);
    }

    #[test]
    fn elbow_uses_vertical_route_when_mostly_below() {
        let path = elbow(r(0.0, 0.0), r(20.0, 400.0)).unwrap();
        assert_eq!(path[0], pos2(50.0, 50.0), "exits the bottom edge");
        assert_eq!(path[1].y, path[2].y, "bends horizontally halfway");
    }

    #[test]
    fn overlapping_notes_have_no_elbow() {
        assert!(elbow(r(0.0, 0.0), r(10.0, 5.0)).is_none());
    }

    #[test]
    fn bezier_starts_and_ends_on_the_anchors() {
        let (a, b) = (pos2(0.0, 0.0), pos2(100.0, 50.0));
        let path = bezier(a, b);
        assert_eq!(path.first(), Some(&a));
        assert_eq!(path.last(), Some(&b));
        assert!(path.len() > 10);
    }

    #[test]
    fn wrapped_offset_is_never_negative() {
        assert_eq!(wrapped_offset(-1.0, 12.0), 11.0);
        assert_eq!(wrapped_offset(-1200.5, 12.0), wrapped_offset(-0.5, 12.0));
        assert!((0.0..12.0).contains(&wrapped_offset(-99999.0, 12.0)));
        assert_eq!(wrapped_offset(5.0, 12.0), 5.0);
    }

    #[test]
    fn wrapped_offset_handles_degenerate_input() {
        assert_eq!(wrapped_offset(-5.0, 0.0), 0.0);
        assert_eq!(wrapped_offset(f32::NAN, 12.0), 0.0);
    }

    #[test]
    fn midpoint_of_an_l_shape_is_on_the_bend() {
        let path = [pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(10.0, 10.0)];
        assert_eq!(midpoint(&path), pos2(10.0, 0.0));
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
