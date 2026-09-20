//! System tray icon (StatusNotifierItem over D-Bus — works with GNOME's
//! AppIndicator extension, KDE, and most other desktops).

use std::sync::mpsc::{self, Receiver, Sender};

use ksni::blocking::TrayMethods;
use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{MenuItem, Tray};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayCommand {
    /// Show the notes panel (and keep it open).
    Open,
    NewNote,
    Sync,
    Settings,
    SetAutostart(bool),
    Quit,
}

struct QuickNoteTray {
    tx: Sender<TrayCommand>,
    ctx: egui::Context,
    autostart: bool,
    notes: usize,
}

impl QuickNoteTray {
    fn send(&self, cmd: TrayCommand) {
        let _ = self.tx.send(cmd);
        self.ctx.request_repaint();
    }
}

impl Tray for QuickNoteTray {
    fn id(&self) -> String {
        "quick-note".into()
    }

    fn title(&self) -> String {
        "Quick Note".into()
    }

    fn icon_name(&self) -> String {
        // Installed by scripts/install.sh into the hicolor icon theme.
        "quick-note".into()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            title: "Quick Note".into(),
            description: format!("{} ghi chú ở trang hiện tại", self.notes),
            icon_name: "quick-note".into(),
            icon_pixmap: Vec::new(),
        }
    }

    /// Left click.
    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::Open);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Mở bảng ghi chú".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayCommand::Open)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Ghi chú mới".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayCommand::NewNote)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Đồng bộ Google Drive".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayCommand::Sync)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Khởi động cùng máy".into(),
                checked: self.autostart,
                activate: Box::new(|t: &mut Self| {
                    // The app flips the real state and calls `set_autostart` back.
                    t.send(TrayCommand::SetAutostart(!t.autostart));
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Cài đặt…".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayCommand::Settings)),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Thoát".into(),
                activate: Box::new(|t: &mut Self| t.send(TrayCommand::Quit)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub struct TrayHandle {
    rx: Receiver<TrayCommand>,
    handle: ksni::blocking::Handle<QuickNoteTray>,
}

impl TrayHandle {
    /// `None` when the desktop has no StatusNotifier host (no tray to show).
    pub fn spawn(ctx: egui::Context, autostart: bool, notes: usize) -> Option<Self> {
        let (tx, rx) = mpsc::channel();
        let tray = QuickNoteTray {
            tx,
            ctx,
            autostart,
            notes,
        };
        match tray.spawn() {
            Ok(handle) => Some(Self { rx, handle }),
            Err(e) => {
                eprintln!("quick-note: không tạo được biểu tượng khay hệ thống: {e}");
                None
            }
        }
    }

    pub fn poll(&self) -> Vec<TrayCommand> {
        self.rx.try_iter().collect()
    }

    pub fn set_autostart(&self, enabled: bool) {
        self.handle.update(|t| t.autostart = enabled);
    }

    pub fn set_note_count(&self, notes: usize) {
        self.handle.update(|t| t.notes = notes);
    }
}
