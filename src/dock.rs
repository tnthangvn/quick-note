//! Dock mode: the window sits as a small sticky in the bottom-right screen corner
//! and expands to a full-height panel on the right edge while hovered.
//!
//! Needs X11 (XWayland on GNOME): Wayland clients may not position themselves
//! or stay on top, so `main.rs` forces the X11 backend when dock mode is on.
//!
//! The OS window never resizes while animating (that made Mutter reconfigure it
//! every frame and looked jerky). It always has the full panel size and a
//! transparent background; the open/close effect is drawn inside it, and an X11
//! input shape limits clicks to the visible part so the rest passes through.

use std::time::{Duration, Instant};

use egui::{Rect, Vec2, pos2, vec2};
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::window::WindowLevel;

use crate::config::DockConfig;

/// Whoever hides us keeps sending `SIGUSR1` as a heartbeat; we come back this
/// long after the last one (tool closed, crashed, or was killed).
const HIDE_TIMEOUT: Duration = Duration::from_secs(6);
const ARM_DELAY: Duration = Duration::from_millis(400);
const RESEND_INTERVAL: Duration = Duration::from_millis(100);

/// What the app should draw this frame, and where (window-local points).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DockFrame {
    pub view: DockView,
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockView {
    /// Tiny sticky in the corner.
    Handle,
    /// Growing/shrinking: draw a cheap placeholder, not the full UI.
    Animating,
    /// Fully open panel: draw the normal app.
    Panel,
}

pub struct Dock {
    cfg: DockConfig,
    expanded: bool,
    /// Keep expanded even when the pointer leaves.
    pub pinned: bool,
    left_at: Option<Instant>,
    applied: Option<Rect>,
    /// When the window first reached its corner; hover is ignored before that
    /// (the WM maps new windows under the pointer, which would pop the panel open).
    placed_at: Option<Instant>,
    sent_at: Option<Instant>,
    workarea: Option<Rect>,
    monitor_names: Vec<String>,
    /// Input region currently set on the X window (window-local physical px).
    input_region: Option<Rect>,
    /// Whether the window currently accepts keyboard focus.
    focusable: Option<bool>,
    /// Hidden on request (see `signals.rs`), and since when.
    hidden: Option<Instant>,
}

impl Dock {
    pub fn new(cfg: DockConfig) -> Self {
        Self {
            cfg,
            expanded: false,
            pinned: false,
            left_at: None,
            applied: None,
            placed_at: None,
            sent_at: None,
            workarea: None,
            monitor_names: Vec::new(),
            input_region: None,
            focusable: None,
            hidden: None,
        }
    }

    pub fn set_config(&mut self, cfg: DockConfig) {
        self.cfg = cfg;
        self.applied = None;
    }

    /// Hides/shows the whole window (a fullscreen overlay from another app
    /// would otherwise end up underneath it).
    pub fn set_hidden(&mut self, ctx: &egui::Context, hidden: bool) {
        if self.hidden.is_some() == hidden {
            // Repeated hide = heartbeat: restart the countdown.
            if hidden {
                self.hidden = Some(Instant::now());
            }
            return;
        }
        self.hidden = hidden.then(Instant::now);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(!hidden));
        if !hidden {
            self.applied = None;
            self.input_region = None;
        }
    }

    pub fn is_hidden(&self) -> bool {
        self.hidden.is_some()
    }

    /// Comes back on its own if whoever hid us never asked for it back
    /// (crashed screenshot tool, killed process…).
    pub fn poll_hidden(&mut self, ctx: &egui::Context) {
        let Some(since) = self.hidden else { return };
        match HIDE_TIMEOUT.checked_sub(since.elapsed()) {
            Some(left) => ctx.request_repaint_after(left),
            None => self.set_hidden(ctx, false),
        }
    }

    /// Opens the panel and keeps it open (tray "show" action).
    pub fn request_open(&mut self) {
        self.expanded = true;
        self.pinned = true;
        self.left_at = None;
    }

    /// Connected monitor names, for the settings dropdown.
    pub fn monitor_names(&self) -> &[String] {
        &self.monitor_names
    }

    /// Updates hover state, keeps the OS window placed, and says what to draw.
    pub fn update(&mut self, ctx: &egui::Context, frame: &eframe::Frame) -> DockFrame {
        if self.hidden.is_some() {
            // Unmapped: nothing to place, nothing to draw.
            return DockFrame {
                view: DockView::Handle,
                rect: Rect::NOTHING,
            };
        }
        self.update_hover(ctx);
        let secs = self.cfg.animation_ms as f32 / 1000.0;
        let t = ctx.animate_bool_with_time_and_easing(
            egui::Id::new("dock-open"),
            self.expanded,
            secs,
            egui::emath::easing::cubic_out,
        );
        self.place_window(ctx, frame);

        let full = ctx.content_rect();
        let handle = handle_rect(full, self.cfg.handle_size);
        if let Some(window) = frame.winit_window() {
            self.update_input_region(window, ctx.pixels_per_point(), handle);
            // While collapsed we are just a sticker: taking keyboard focus would
            // steal Esc/clicks from whatever is actually in front of the user.
            let focusable = self.expanded;
            if self.focusable != Some(focusable) && set_focusable(window, focusable).is_some() {
                self.focusable = Some(focusable);
            }
        }
        let view = match t {
            t if t <= 0.0 => DockView::Handle,
            t if t >= 1.0 => DockView::Panel,
            _ => DockView::Animating,
        };
        DockFrame {
            view,
            rect: lerp_rect(handle, full, t),
        }
    }

    /// Clicks reach us only on the handle while collapsed, everywhere while open.
    fn update_input_region(&mut self, window: &winit::window::Window, ppp: f32, handle: Rect) {
        let size = window.inner_size();
        let full = Rect::from_min_size(pos2(0.0, 0.0), vec2(size.width as f32, size.height as f32));
        let wanted = if self.expanded {
            full
        } else {
            Rect::from_min_max((handle.min.to_vec2() * ppp).to_pos2(), full.max)
        };
        if self.input_region == Some(wanted) {
            return;
        }
        if set_input_region(window, wanted).is_some() {
            self.input_region = Some(wanted);
        }
    }

    fn update_hover(&mut self, ctx: &egui::Context) {
        let armed = self.placed_at.is_some_and(|t| t.elapsed() >= ARM_DELAY);
        if !armed {
            ctx.request_repaint_after(ARM_DELAY);
        }
        // Opening needs real motion: X sends no leave event when we move the window
        // out from under a pointer, so `has_pointer` alone can be stale.
        let (has_pointer, moved) = ctx.input(|i| {
            let moved = i
                .raw
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::PointerMoved(_)));
            (i.pointer.has_pointer(), moved)
        });
        let hovering = armed && has_pointer && (self.expanded || moved);
        let delay = Duration::from_millis(u64::from(self.cfg.collapse_delay_ms));
        if hovering {
            self.expanded = true;
            self.left_at = None;
        } else if self.expanded && !self.pinned {
            let left = *self.left_at.get_or_insert_with(Instant::now);
            if left.elapsed() >= delay {
                self.expanded = false;
                self.left_at = None;
            } else {
                ctx.request_repaint_after(delay.saturating_sub(left.elapsed()));
            }
        }
    }

    fn place_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        let Some(window) = frame.winit_window() else {
            return;
        };
        let monitors: Vec<_> = window.available_monitors().collect();
        let screens: Vec<Screen> = monitors
            .iter()
            .map(|m| Screen {
                name: m.name().unwrap_or_default(),
                rect: Rect::from_min_size(
                    pos2(m.position().x as f32, m.position().y as f32),
                    vec2(m.size().width as f32, m.size().height as f32),
                ),
            })
            .collect();
        self.monitor_names = screens.iter().map(|s| s.name.clone()).collect();
        let Some(idx) = pick_monitor(&screens, &self.cfg.monitor) else {
            ctx.request_repaint_after(Duration::from_millis(100));
            return;
        };
        let monitor = &monitors[idx];
        let monitor_rect = screens[idx].rect;
        let is_primary = window.primary_monitor().is_none_or(|p| p == *monitor);
        if !self.expanded || self.workarea.is_none() {
            // Cheap (one round-trip); refreshed while collapsed so panel changes are picked up.
            self.workarea = x11_workarea();
        }
        // Stay out of the GNOME top bar (primary monitor only): Mutter shoves
        // overlapping windows to another monitor.
        let screen = self
            .workarea
            .filter(|_| is_primary)
            .map(|w| w.intersect(monitor_rect))
            .filter(|r| r.is_positive())
            .unwrap_or(monitor_rect);
        let target = geometry(true, screen, &self.cfg, monitor.scale_factor() as f32);
        let target = Rect::from_min_max(target.min.round(), target.max.round());

        // Trust what the WM reports, not what we asked for: Mutter ignores
        // moves sent before the window is mapped.
        let actual = window.outer_position().ok().map(|p| {
            let s = window.inner_size();
            Rect::from_min_size(
                pos2(p.x as f32, p.y as f32),
                vec2(s.width as f32, s.height as f32),
            )
        });
        if actual.is_some_and(|a| close_enough(a, target)) {
            if self.placed_at.is_none() {
                self.placed_at = Some(Instant::now());
            }
            self.applied = Some(target);
            return;
        }
        let resend_due = self
            .sent_at
            .is_none_or(|at| at.elapsed() >= RESEND_INTERVAL);
        if self.applied == Some(target) && !resend_due {
            ctx.request_repaint_after(RESEND_INTERVAL);
            return;
        }
        window.set_outer_position(PhysicalPosition::new(
            target.min.x as i32,
            target.min.y as i32,
        ));
        let _ = window.request_inner_size(PhysicalSize::new(
            target.width() as u32,
            target.height() as u32,
        ));
        // The builder flag is dropped by Mutter when set before mapping; keep asserting it.
        window.set_window_level(WindowLevel::AlwaysOnTop);
        self.applied = Some(target);
        self.sent_at = Some(Instant::now());
        self.input_region = None; // size changed: re-apply the shape

        ctx.request_repaint_after(RESEND_INTERVAL);
    }
}

/// Tells the window manager whether this window wants keyboard focus (WM_HINTS
/// input flag). Mutter re-reads it when the property changes.
fn set_focusable(window: &winit::window::Window, focusable: bool) -> Option<()> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
    use x11rb::wrapper::ConnectionExt as _;

    let id = match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok()?,
        RawWindowHandle::Xcb(h) => h.window.get(),
        _ => return None,
    };
    let (conn, _) = x11rb::connect(None).ok()?;
    let wm_hints = conn.intern_atom(true, b"WM_HINTS").ok()?.reply().ok()?.atom;
    // ICCCM WM_HINTS: [flags, input, initial_state, ...]; bit 0 of flags = InputHint.
    let mut hints = [0u32; 9];
    hints[0] = 1;
    hints[1] = u32::from(focusable);
    conn.change_property32(PropMode::REPLACE, id, wm_hints, AtomEnum::WM_HINTS, &hints)
        .ok()?
        .check()
        .ok()?;
    conn.flush().ok()
}

/// Sets the X11 input shape of `window` to one rectangle (window-local physical px).
fn set_input_region(window: &winit::window::Window, rect: Rect) -> Option<()> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::connection::Connection;
    use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
    use x11rb::protocol::xproto::{ClipOrdering, Rectangle};

    let id = match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Xlib(h) => u32::try_from(h.window).ok()?,
        RawWindowHandle::Xcb(h) => h.window.get(),
        _ => return None,
    };
    let (conn, _) = x11rb::connect(None).ok()?;
    let r = Rectangle {
        x: rect.min.x.round() as i16,
        y: rect.min.y.round() as i16,
        width: rect.width().round() as u16,
        height: rect.height().round() as u16,
    };
    conn.shape_rectangles(SO::SET, SK::INPUT, ClipOrdering::UNSORTED, id, 0, 0, &[r])
        .ok()?
        .check()
        .ok()?;
    conn.flush().ok()
}

/// The collapsed sticky: bottom-right square of the window (points).
fn handle_rect(window: Rect, size: f32) -> Rect {
    Rect::from_min_max(
        window.max - Vec2::splat(size.min(window.width()).min(window.height())),
        window.max,
    )
}

/// Desktop area minus panels (`_NET_WORKAREA` of the current desktop), in physical pixels.
fn x11_workarea() -> Option<Rect> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{AtomEnum, ConnectionExt};

    let (conn, screen_idx) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen_idx)?.root;
    let atom = |name: &[u8]| {
        conn.intern_atom(true, name)
            .ok()?
            .reply()
            .ok()
            .map(|r| r.atom)
    };
    let cardinals = |prop| {
        conn.get_property(false, root, prop, AtomEnum::CARDINAL, 0, 256)
            .ok()?
            .reply()
            .ok()?
            .value32()
            .map(|v| v.collect::<Vec<u32>>())
    };
    let desktop = atom(b"_NET_CURRENT_DESKTOP")
        .and_then(cardinals)
        .and_then(|v| v.first().copied())
        .unwrap_or(0) as usize;
    let area = cardinals(atom(b"_NET_WORKAREA")?)?;
    let [x, y, w, h] = area.get(desktop * 4..desktop * 4 + 4)?.try_into().ok()?;
    Some(Rect::from_min_size(
        pos2(x as f32, y as f32),
        vec2(w as f32, h as f32),
    ))
}

/// WMs may nudge windows by a pixel or two (struts, rounding); don't fight that.
fn close_enough(a: Rect, b: Rect) -> bool {
    const TOLERANCE: f32 = 3.0;
    (a.min - b.min).abs().max_elem() <= TOLERANCE && (a.max - b.max).abs().max_elem() <= TOLERANCE
}

struct Screen {
    name: String,
    rect: Rect,
}

/// The configured monitor by name, else the rightmost one (bottom-most on ties),
/// so "bottom-right corner" means the corner of the whole desktop.
fn pick_monitor(screens: &[Screen], wanted: &str) -> Option<usize> {
    let wanted = wanted.trim();
    if !wanted.is_empty()
        && let Some(i) = screens.iter().position(|s| s.name == wanted)
    {
        return Some(i);
    }
    screens
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            (a.rect.max.x, a.rect.max.y)
                .partial_cmp(&(b.rect.max.x, b.rect.max.y))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
}

fn lerp_rect(a: Rect, b: Rect, t: f32) -> Rect {
    Rect::from_min_max(a.min.lerp(b.min, t), a.max.lerp(b.max, t))
}

/// Placeholder while animating (laying out the full UI at odd sizes looks broken).
pub fn paint_transition(ui: &egui::Ui, rect: Rect) {
    let painter = ui.painter();
    painter.rect_filled(rect, 6, ui.visuals().panel_fill);
    let band = Rect::from_min_size(rect.min, vec2(rect.width(), 34.0_f32.min(rect.height())));
    painter.rect_filled(band, 6, egui::Color32::from_rgb(242, 220, 122));
}

/// Window rect in physical pixels on `screen`: handle in the bottom-right corner,
/// or a full-height panel anchored to the right edge. Sizes in config are points.
pub fn geometry(expanded: bool, screen: Rect, cfg: &DockConfig, scale: f32) -> Rect {
    if expanded {
        let width = (cfg.width * scale).clamp(DockConfig::MIN_WIDTH * scale, screen.width());
        Rect::from_min_max(pos2(screen.max.x - width, screen.min.y), screen.max)
    } else {
        let size = Vec2::splat(cfg.handle_size * scale);
        Rect::from_min_max(screen.max - size, screen.max)
    }
}

/// Collapsed look: a tiny sticky note with the note count.
pub fn paint_handle(ui: &egui::Ui, rect: Rect, note_count: usize) {
    let painter = ui.painter();
    painter.rect_filled(rect, 4, egui::Color32::from_rgb(255, 241, 168));
    let band = Rect::from_min_size(rect.min, vec2(rect.width(), rect.height() * 0.22));
    painter.rect_filled(band, 4, egui::Color32::from_rgb(242, 220, 122));
    painter.text(
        rect.center() + vec2(0.0, rect.height() * 0.08),
        egui::Align2::CENTER_CENTER,
        note_count.to_string(),
        egui::FontId::proportional(rect.height() * 0.4),
        egui::Color32::from_rgb(109, 93, 30),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> DockConfig {
        DockConfig {
            enabled: true,
            width: 1000.0,
            handle_size: 36.0,
            collapse_delay_ms: 400,
            animation_ms: 180,
            monitor: String::new(),
        }
    }

    fn two_screens() -> Vec<Screen> {
        vec![
            Screen {
                name: "DP-2".into(),
                rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(1920.0, 1080.0)),
            },
            Screen {
                name: "HDMI-1".into(),
                rect: Rect::from_min_size(pos2(1920.0, 0.0), vec2(1920.0, 1080.0)),
            },
        ]
    }

    #[test]
    fn auto_picks_rightmost_monitor() {
        assert_eq!(pick_monitor(&two_screens(), ""), Some(1));
    }

    #[test]
    fn named_monitor_wins_and_unknown_falls_back() {
        assert_eq!(pick_monitor(&two_screens(), "DP-2"), Some(0));
        assert_eq!(pick_monitor(&two_screens(), "gone"), Some(1));
        assert_eq!(pick_monitor(&[], ""), None);
    }

    #[test]
    fn handle_rect_is_bottom_right_square() {
        let w = Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 1048.0));
        assert_eq!(
            handle_rect(w, 36.0),
            Rect::from_min_max(pos2(964.0, 1012.0), pos2(1000.0, 1048.0))
        );
    }

    #[test]
    fn close_enough_tolerates_small_nudges() {
        let a = Rect::from_min_max(pos2(1884.0, 1044.0), pos2(1920.0, 1080.0));
        assert!(close_enough(a, a.translate(vec2(0.0, -1.0))));
        assert!(!close_enough(a, a.translate(vec2(-40.0, 0.0))));
    }

    #[test]
    fn lerp_rect_endpoints_and_midpoint() {
        let a = Rect::from_min_max(pos2(0.0, 0.0), pos2(10.0, 10.0));
        let b = Rect::from_min_max(pos2(10.0, 20.0), pos2(30.0, 40.0));
        assert_eq!(lerp_rect(a, b, 0.0), a);
        assert_eq!(lerp_rect(a, b, 1.0), b);
        assert_eq!(
            lerp_rect(a, b, 0.5),
            Rect::from_min_max(pos2(5.0, 10.0), pos2(20.0, 25.0))
        );
    }

    fn screen() -> Rect {
        // Second monitor of a 2x1920 layout.
        Rect::from_min_size(pos2(1920.0, 0.0), vec2(1920.0, 1080.0))
    }

    #[test]
    fn collapsed_sits_in_bottom_right_corner() {
        let r = geometry(false, screen(), &cfg(), 1.0);
        assert_eq!(
            r,
            Rect::from_min_max(pos2(3804.0, 1044.0), pos2(3840.0, 1080.0))
        );
    }

    #[test]
    fn expanded_is_full_height_right_panel() {
        let r = geometry(true, screen(), &cfg(), 1.0);
        assert_eq!(
            r,
            Rect::from_min_max(pos2(2840.0, 0.0), pos2(3840.0, 1080.0))
        );
    }

    #[test]
    fn hidpi_scales_configured_sizes() {
        let r = geometry(false, screen(), &cfg(), 2.0);
        assert_eq!(r.width(), 72.0);
    }

    #[test]
    fn expanded_width_never_exceeds_screen() {
        let c = DockConfig {
            width: 5000.0,
            ..cfg()
        };
        let r = geometry(true, screen(), &c, 1.0);
        assert_eq!(r, screen());
    }
}
