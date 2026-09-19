//! App state, frame loop, and the glue between UI actions, storage and Drive.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Key, KeyboardShortcut, Modifiers};
use uuid::Uuid;

use crate::config::Config;
use crate::dock::{self, Dock, DockFrame, DockView};
use crate::drive::{Command, DriveHandle, Event};
use crate::model::{Link, Note, Page, Workspace};
use crate::storage;
use crate::ui::Action;
use crate::ui::canvas::{self, CanvasState};
use crate::ui::dialogs::{self, PullChoice, SettingsResult};
use crate::ui::toolbar::{self, DriveView, Status, ToolbarState};

const SAVE_DEBOUNCE: Duration = Duration::from_millis(800);
const UNDO_LIMIT: usize = 20;
const TOAST_TTL: Duration = Duration::from_secs(5);
const NEW_NOTE_ANCHOR: [f32; 2] = [40.0, 40.0];

enum Deleted {
    Note {
        page_id: Uuid,
        note: Note,
        links: Vec<Link>,
    },
    Link {
        page_id: Uuid,
        link: Link,
    },
    Page {
        idx: usize,
        page: Page,
    },
}

struct Toast {
    text: String,
    is_error: bool,
    at: Instant,
}

#[derive(Default)]
struct DriveState {
    connected: bool,
    busy: Option<String>,
    last_sync: Option<String>,
}

pub struct QuickNoteApp {
    dir: PathBuf,
    ws: Workspace,
    cfg: Config,
    drive: DriveHandle,
    drive_state: DriveState,
    /// Local changes not yet written to disk.
    dirty_since: Option<Instant>,
    /// Changes not yet pushed to Drive.
    unsynced: bool,
    /// Bumped on every edit; lets a finished push know if newer edits exist.
    edit_gen: u64,
    pushed_gen: u64,
    last_push: Instant,
    toast: Option<Toast>,
    /// Recently deleted items, newest last (bounded).
    trash: Vec<Deleted>,
    canvas: CanvasState,
    toolbar: ToolbarState,
    settings_draft: Option<Config>,
    pending_pull: Option<Workspace>,
    /// Some = corner-sticky mode (see `dock.rs`).
    dock: Option<Dock>,
}

impl QuickNoteApp {
    pub fn new(cc: &eframe::CreationContext<'_>, dir: PathBuf, dock_mode: bool) -> Self {
        let mut toast = None;
        let ws = storage::load_workspace(&dir).unwrap_or_else(|e| {
            // Keep the broken file untouched: back it up under a new name before we overwrite it.
            let broken = dir.join(storage::WORKSPACE_FILE);
            let _ = std::fs::copy(&broken, broken.with_extension("json.broken"));
            toast = Some(Toast {
                text: format!("Không đọc được dữ liệu, đã giữ bản lỗi (.broken): {e:#}"),
                is_error: true,
                at: Instant::now(),
            });
            Workspace::default()
        });
        let cfg = Config::load(&dir).unwrap_or_default();
        apply_font_size(&cc.egui_ctx, cfg.font_size);
        // Ctrl +/-/0 zoom the canvas, not the whole UI.
        cc.egui_ctx.options_mut(|o| o.zoom_with_keyboard = false);

        let ctx = cc.egui_ctx.clone();
        let drive = DriveHandle::spawn(cfg.drive.clone(), dir.clone(), move || {
            ctx.request_repaint()
        });

        Self {
            dir,
            ws,
            drive,
            drive_state: DriveState::default(),
            dirty_since: None,
            unsynced: false,
            edit_gen: 0,
            pushed_gen: 0,
            last_push: Instant::now(),
            toast,
            trash: Vec::new(),
            canvas: CanvasState::default(),
            toolbar: ToolbarState::default(),
            settings_draft: None,
            pending_pull: None,
            dock: dock_mode.then(|| Dock::new(cfg.dock.clone())),
            cfg,
        }
    }

    fn notify(&mut self, text: impl Into<String>, is_error: bool) {
        self.toast = Some(Toast {
            text: text.into(),
            is_error,
            at: Instant::now(),
        });
    }

    fn mark_changed(&mut self) {
        self.ws.touch();
        self.dirty_since.get_or_insert_with(Instant::now);
        self.unsynced = true;
        self.edit_gen += 1;
    }

    fn save_now(&mut self) {
        match storage::save_workspace(&self.dir, &self.ws) {
            Ok(()) => self.dirty_since = None,
            Err(e) => {
                // Retry on the next debounce tick instead of spamming every frame.
                self.dirty_since = Some(Instant::now());
                self.notify(format!("Lỗi lưu: {e:#}"), true);
            }
        }
    }

    fn push(&mut self) {
        if !self.drive_state.connected {
            self.notify("Chưa kết nối Google Drive", true);
            return;
        }
        self.drive.send(Command::Push(Box::new(self.ws.clone())));
        self.pushed_gen = self.edit_gen;
        self.last_push = Instant::now();
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::SelectPage(idx) => {
                self.ws.active = idx.min(self.ws.pages.len() - 1);
                self.dirty_since.get_or_insert_with(Instant::now);
            }
            Action::AddPage => {
                self.ws.add_page();
                self.toolbar.renaming = Some((self.ws.active, self.ws.active_page().title.clone()));
                self.mark_changed();
            }
            Action::RenamePage(idx, title) => {
                if let Some(page) = self.ws.pages.get_mut(idx) {
                    page.title = title;
                    self.mark_changed();
                }
            }
            Action::DeletePage(idx) => {
                if let Some(page) = self.ws.remove_page(idx) {
                    self.notify(format!("Đã xoá trang \"{}\"", page.title), false);
                    self.push_trash(Deleted::Page { idx, page });
                    self.mark_changed();
                }
            }
            Action::AddNote => {
                let page = self.ws.active_page_mut();
                let anchor = page.to_canvas(NEW_NOTE_ANCHOR);
                let id = page.add_note(anchor);
                self.canvas.focus_body = Some(id);
                self.mark_changed();
            }
            Action::Tidy => {
                let page = self.ws.active_page_mut();
                let width = ctx.content_rect().width() / page.zoom;
                page.tidy(width);
                self.mark_changed();
            }
            Action::Undo => self.undo(),
            Action::Zoom(steps) => {
                canvas::zoom_step(self.ws.active_page_mut(), &self.canvas, steps);
                self.mark_changed();
            }
            Action::ZoomReset => {
                let page = self.ws.active_page_mut();
                let centre = (self.canvas.last_size / 2.0).into();
                page.zoom_at(centre, 1.0);
                self.mark_changed();
            }
            Action::Save => {
                self.save_now();
                if self.drive_state.connected {
                    self.push();
                } else {
                    self.notify("Đã lưu local", false);
                }
            }
            Action::Push => self.push(),
            Action::Pull => self.drive.send(Command::Pull),
            Action::Login => self.drive.send(Command::Login),
            Action::ToggleDockPin => {
                if let Some(dock) = self.dock.as_mut() {
                    dock.pinned = !dock.pinned;
                }
            }
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
            Action::OpenSettings => self.settings_draft = Some(self.cfg.clone()),
            Action::ExportMarkdown => {
                match storage::export_markdown(&self.dir, self.ws.active_page()) {
                    Ok(path) => self.notify(format!("Đã xuất {}", path.display()), false),
                    Err(e) => self.notify(format!("Xuất lỗi: {e:#}"), true),
                }
            }
        }
    }

    fn delete_note(&mut self, id: Uuid) {
        let page = self.ws.active_page_mut();
        let page_id = page.id;
        if let Some((note, links)) = page.remove_note(id) {
            self.notify(format!("Đã xoá \"{}\"", note.display_title()), false);
            self.push_trash(Deleted::Note {
                page_id,
                note,
                links,
            });
            self.mark_changed();
        }
    }

    fn delete_link(&mut self, id: Uuid) {
        let page = self.ws.active_page_mut();
        let page_id = page.id;
        if let Some(link) = page.remove_link(id) {
            self.notify("Đã xoá mũi tên", false);
            self.push_trash(Deleted::Link { page_id, link });
            self.mark_changed();
        }
    }

    /// Index of the page with `page_id` (falls back to the active page) and makes it active.
    fn activate_page(&mut self, page_id: Uuid) -> usize {
        let idx = self
            .ws
            .pages
            .iter()
            .position(|p| p.id == page_id)
            .unwrap_or(self.ws.active);
        self.ws.active = idx;
        idx
    }

    fn push_trash(&mut self, item: Deleted) {
        if self.trash.len() == UNDO_LIMIT {
            self.trash.remove(0);
        }
        self.trash.push(item);
    }

    fn undo(&mut self) {
        match self.trash.pop() {
            Some(Deleted::Note {
                page_id,
                note,
                links,
            }) => {
                let idx = self.activate_page(page_id);
                let page = &mut self.ws.pages[idx];
                page.insert_note(note);
                // Only arrows whose other end still exists come back.
                for link in links {
                    let (from, to) = (link.from, link.to);
                    if page.connect(from, to)
                        && let Some(restored) = page.links.last_mut()
                    {
                        *restored = link;
                    }
                }
            }
            Some(Deleted::Link { page_id, link }) => {
                let idx = self.activate_page(page_id);
                let page = &mut self.ws.pages[idx];
                if page.connect(link.from, link.to)
                    && let Some(restored) = page.links.last_mut()
                {
                    *restored = link;
                }
            }
            Some(Deleted::Page { idx, page }) => {
                let idx = idx.min(self.ws.pages.len());
                self.ws.pages.insert(idx, page);
                self.ws.active = idx;
            }
            None => return,
        }
        self.notify("Đã hoàn tác", false);
        self.mark_changed();
    }

    fn handle_drive_events(&mut self) {
        for event in self.drive.poll() {
            match event {
                Event::Busy(msg) => self.drive_state.busy = Some(msg),
                Event::LoggedIn => {
                    self.drive_state.busy = None;
                    self.drive_state.connected = true;
                }
                Event::LoggedOut => {
                    self.drive_state.busy = None;
                    self.drive_state.connected = false;
                    self.notify("Đã ngắt kết nối Drive", false);
                }
                Event::Pushed { files } => {
                    self.drive_state.busy = None;
                    self.unsynced = self.edit_gen != self.pushed_gen;
                    let now = chrono::Local::now().format("%H:%M").to_string();
                    self.notify(format!("Đã đồng bộ {files} file lên Drive"), false);
                    self.drive_state.last_sync = Some(now);
                }
                Event::Pulled(ws) => {
                    self.drive_state.busy = None;
                    self.pending_pull = Some(*ws);
                }
                Event::Error(msg) => {
                    self.drive_state.busy = None;
                    self.notify(msg, true);
                }
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context, actions: &mut Vec<Action>) {
        let cmd = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let typing = ctx.memory(|m| m.focused().is_some());
        ctx.input_mut(|i| {
            if i.consume_shortcut(&cmd(Key::N)) {
                actions.push(Action::AddNote);
            }
            if i.consume_shortcut(&cmd(Key::T)) {
                actions.push(Action::AddPage);
            }
            if i.consume_shortcut(&cmd(Key::S)) {
                actions.push(Action::Save);
            }
            if i.consume_shortcut(&cmd(Key::Plus)) || i.consume_shortcut(&cmd(Key::Equals)) {
                actions.push(Action::Zoom(1));
            }
            if i.consume_shortcut(&cmd(Key::Minus)) {
                actions.push(Action::Zoom(-1));
            }
            if i.consume_shortcut(&cmd(Key::Num0)) {
                actions.push(Action::ZoomReset);
            }
            if i.consume_shortcut(&cmd(Key::Q)) {
                actions.push(Action::Quit);
            }
            if i.consume_shortcut(&cmd(Key::F)) {
                self.toolbar.focus_search = true;
            }
            // Only when no text field is focused, so TextEdit keeps its own undo.
            if !typing && !self.trash.is_empty() && i.consume_shortcut(&cmd(Key::Z)) {
                actions.push(Action::Undo);
            }
            let pages = self.ws.pages.len();
            if i.consume_shortcut(&cmd(Key::PageDown)) {
                actions.push(Action::SelectPage((self.ws.active + 1) % pages));
            }
            if i.consume_shortcut(&cmd(Key::PageUp)) {
                actions.push(Action::SelectPage((self.ws.active + pages - 1) % pages));
            }
        });
    }

    fn background_tasks(&mut self, ctx: &egui::Context) {
        if let Some(since) = self.dirty_since {
            if since.elapsed() >= SAVE_DEBOUNCE {
                self.save_now();
            } else {
                ctx.request_repaint_after(SAVE_DEBOUNCE);
            }
        }
        let every = self.cfg.auto_sync_minutes;
        if every > 0
            && self.unsynced
            && self.drive_state.connected
            && self.drive_state.busy.is_none()
        {
            let interval = Duration::from_secs(u64::from(every) * 60);
            if self.last_push.elapsed() >= interval {
                self.push();
            } else {
                ctx.request_repaint_after(interval - self.last_push.elapsed());
            }
        }
        if self
            .toast
            .as_ref()
            .is_some_and(|t| t.at.elapsed() > TOAST_TTL)
        {
            self.toast = None;
        } else if self.toast.is_some() {
            ctx.request_repaint_after(TOAST_TTL);
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(draft) = self.settings_draft.as_mut() {
            let monitors = self
                .dock
                .as_ref()
                .map(|d| d.monitor_names().to_vec())
                .unwrap_or_default();
            match dialogs::settings(ctx, draft, self.drive_state.connected, &monitors) {
                SettingsResult::Keep => {}
                SettingsResult::Cancel => self.settings_draft = None,
                SettingsResult::Logout => {
                    self.drive.send(Command::Logout);
                    self.settings_draft = None;
                }
                SettingsResult::Save(cfg) => {
                    match cfg.save(&self.dir) {
                        Ok(()) => self.notify("Đã lưu cài đặt", false),
                        Err(e) => self.notify(format!("Lỗi lưu cài đặt: {e:#}"), true),
                    }
                    apply_font_size(ctx, cfg.font_size);
                    if let Some(dock) = self.dock.as_mut() {
                        dock.set_config(cfg.dock.clone());
                    }
                    if cfg.dock.enabled != self.cfg.dock.enabled {
                        self.notify("Mở lại app để bật/tắt chế độ sticky góc màn hình", false);
                    }
                    self.drive.send(Command::Configure(cfg.drive.clone()));
                    self.cfg = cfg;
                    self.settings_draft = None;
                }
            }
        }

        if let Some(remote) = self.pending_pull.as_ref() {
            match dialogs::confirm_pull(ctx, remote, &self.ws) {
                PullChoice::Pending => {}
                PullChoice::Cancel => self.pending_pull = None,
                PullChoice::Replace => self.replace_with_remote(),
            }
        }
    }

    fn replace_with_remote(&mut self) {
        let Some(remote) = self.pending_pull.take() else {
            return;
        };
        if let Err(e) = storage::backup_workspace(&self.dir, &self.ws) {
            self.notify(format!("Không backup được, huỷ thay thế: {e:#}"), true);
            return;
        }
        self.ws = remote;
        self.trash.clear();
        self.save_now();
        self.unsynced = false;
        self.notify("Đã tải bản từ Drive (bản cũ ở backups/)", false);
    }

    fn drive_line(&self) -> String {
        match (
            &self.drive_state.busy,
            self.drive_state.connected,
            &self.drive_state.last_sync,
        ) {
            (Some(msg), _, _) => msg.clone(),
            (None, true, Some(at)) => format!("☁ Drive: đồng bộ lúc {at}"),
            (None, true, None) => "☁ Drive: đã kết nối".into(),
            (None, false, _) => "☁ Drive: chưa kết nối".into(),
        }
    }
}

impl eframe::App for QuickNoteApp {
    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        #[cfg(debug_assertions)]
        crate::debug_shot::tick(&ctx);
        self.handle_drive_events();

        if let Some(dock) = self.dock.as_mut() {
            let DockFrame { view, rect } = dock.update(&ctx, frame);
            self.toolbar.dock_pinned = Some(dock.pinned);
            if view != DockView::Panel {
                // Everything outside `rect` stays transparent (see `clear_color`).
                let count = self.ws.active_page().notes.len();
                match view {
                    DockView::Handle => dock::paint_handle(ui, rect, count),
                    _ => dock::paint_transition(ui, rect),
                }
                self.background_tasks(&ctx);
                return;
            }
        }

        let mut actions = Vec::new();
        self.shortcuts(&ctx, &mut actions);

        let drive_view = DriveView {
            connected: self.drive_state.connected,
            configured: self.cfg.drive.is_configured(),
            busy: self.drive_state.busy.as_deref(),
        };
        egui::Panel::top("top-bar").show(ui, |ui| {
            ui.add_space(4.0);
            toolbar::top_bar(ui, &self.ws, &mut self.toolbar, &drive_view, &mut actions);
            ui.add_space(2.0);
        });

        let drive_line = self.drive_line();
        let status = Status {
            saved: self.dirty_since.is_none(),
            unsynced: self.unsynced && self.drive_state.connected,
            drive_line: &drive_line,
            toast: self.toast.as_ref().map(|t| (t.text.as_str(), t.is_error)),
            can_undo: !self.trash.is_empty(),
        };
        egui::Panel::bottom("status-bar").show(ui, |ui| {
            toolbar::status_bar(ui, &self.ws, &status, &mut actions);
        });

        let mut delete = None;
        let mut delete_link = None;
        let canvas_frame = egui::Frame::central_panel(ui.style()).inner_margin(0);
        egui::CentralPanel::default()
            .frame(canvas_frame)
            .show(ui, |ui| {
                let search = self.toolbar.search.clone();
                let opts = canvas::CanvasOptions {
                    search: &search,
                    markdown: self.cfg.markdown,
                };
                let out = canvas::show(ui, self.ws.active_page_mut(), &mut self.canvas, &opts);
                delete = out.delete;
                delete_link = out.delete_link;
                if out.changed {
                    self.mark_changed();
                }
            });
        if let Some(id) = delete {
            self.delete_note(id);
        }
        if let Some(id) = delete_link {
            self.delete_link(id);
        }

        for action in actions {
            self.apply(action, &ctx);
        }
        self.dialogs(&ctx);
        self.background_tasks(&ctx);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        if self.dock.is_some() {
            // Dock window is full-size but mostly invisible; only drawn parts show.
            return [0.0; 4];
        }
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.dirty_since.is_some() {
            self.save_now();
        }
    }
}

fn apply_font_size(ctx: &egui::Context, size: f32) {
    use egui::{FontFamily, FontId, TextStyle};
    ctx.global_style_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(size + 5.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(size, FontFamily::Proportional)),
            (
                TextStyle::Monospace,
                FontId::new(size - 1.0, FontFamily::Monospace),
            ),
            (
                TextStyle::Button,
                FontId::new(size - 1.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(size - 3.0, FontFamily::Proportional),
            ),
        ]
        .into();
    });
}
