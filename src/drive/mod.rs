//! Background Drive sync worker. The UI sends [`Command`]s and polls [`Event`]s,
//! so network calls never block rendering.

mod api;
mod oauth;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use crate::config::DriveConfig;
use crate::export;
use crate::model::Workspace;
use api::Drive;
use oauth::Session;

pub const REMOTE_WORKSPACE: &str = "quick-note-workspace.json";
const HTTP_TIMEOUT: Duration = Duration::from_secs(30);

pub enum Command {
    Configure(DriveConfig),
    Login,
    Logout,
    Push(Box<Workspace>),
    Pull,
}

pub enum Event {
    Busy(String),
    LoggedIn,
    LoggedOut,
    Pushed { files: usize },
    Pulled(Box<Workspace>),
    Error(String),
}

pub struct DriveHandle {
    tx: Sender<Command>,
    rx: Receiver<Event>,
}

impl DriveHandle {
    /// Spawns the worker. `notify` is called after each event (to wake the UI).
    pub fn spawn(cfg: DriveConfig, dir: PathBuf, notify: impl Fn() + Send + 'static) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (evt_tx, evt_rx) = mpsc::channel();
        thread::Builder::new()
            .name("drive-sync".into())
            .spawn(move || Worker::new(cfg, dir, evt_tx, notify).run(cmd_rx))
            .expect("spawn drive thread");
        Self {
            tx: cmd_tx,
            rx: evt_rx,
        }
    }

    pub fn send(&self, cmd: Command) {
        // Worker only exits when the handle is dropped, so this cannot fail in practice.
        let _ = self.tx.send(cmd);
    }

    pub fn poll(&self) -> Vec<Event> {
        self.rx.try_iter().collect()
    }
}

struct Worker<N: Fn()> {
    cfg: DriveConfig,
    dir: PathBuf,
    http: reqwest::blocking::Client,
    session: Option<Session>,
    folder_id: Option<String>,
    events: Sender<Event>,
    notify: N,
}

impl<N: Fn()> Worker<N> {
    fn new(cfg: DriveConfig, dir: PathBuf, events: Sender<Event>, notify: N) -> Self {
        let http = reqwest::blocking::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent(concat!("quick-note/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("build http client");
        let mut worker = Self {
            cfg,
            dir,
            http,
            session: None,
            folder_id: None,
            events,
            notify,
        };
        worker.restore_session();
        worker
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
        (self.notify)();
    }

    fn restore_session(&mut self) {
        match Session::restore(&self.cfg, &self.dir) {
            Ok(Some(session)) => {
                self.session = Some(session);
                self.emit(Event::LoggedIn);
            }
            Ok(None) => {}
            Err(e) => self.emit(Event::Error(format!("đọc token Drive: {e:#}"))),
        }
    }

    fn run(mut self, commands: Receiver<Command>) {
        for cmd in commands {
            if let Err(e) = self.handle(cmd) {
                if e.downcast_ref::<oauth::InvalidGrant>().is_some() {
                    self.session = None;
                    self.folder_id = None;
                    if let Err(forget) = oauth::forget_token(&self.dir) {
                        self.emit(Event::Error(format!("xoá token cũ: {forget:#}")));
                    }
                    self.emit(Event::LoggedOut);
                }
                self.emit(Event::Error(format!("{e:#}")));
            }
        }
    }

    fn handle(&mut self, cmd: Command) -> Result<()> {
        match cmd {
            Command::Configure(cfg) => {
                // The session caches the client credentials, so rebuild it when they change.
                let scope_changed = cfg.scope() != self.cfg.scope()
                    || cfg.client_id != self.cfg.client_id
                    || cfg.client_secret != self.cfg.client_secret;
                let folder_changed =
                    cfg.folder_id != self.cfg.folder_id || cfg.folder_name != self.cfg.folder_name;
                self.cfg = cfg;
                if folder_changed {
                    self.folder_id = None;
                }
                if scope_changed && self.session.take().is_some() {
                    self.emit(Event::LoggedOut);
                    self.restore_session();
                }
            }
            Command::Login => {
                self.emit(Event::Busy(
                    "Đang chờ đăng nhập Google trên trình duyệt…".into(),
                ));
                self.session = Some(Session::login(&self.cfg, &self.dir, &self.http)?);
                self.folder_id = None;
                self.emit(Event::LoggedIn);
            }
            Command::Logout => {
                self.session = None;
                self.folder_id = None;
                oauth::forget_token(&self.dir)?;
                self.emit(Event::LoggedOut);
            }
            Command::Push(ws) => {
                self.emit(Event::Busy("Đang đẩy lên Drive…".into()));
                let files = self.push(&ws)?;
                self.emit(Event::Pushed { files });
            }
            Command::Pull => {
                self.emit(Event::Busy("Đang tải từ Drive…".into()));
                let ws = self.pull()?;
                self.emit(Event::Pulled(Box::new(ws)));
            }
        }
        Ok(())
    }

    fn drive(&mut self) -> Result<(Drive<'_>, String)> {
        let session = self.session.as_mut().context("chưa kết nối Google Drive")?;
        let mut drive = Drive::new(&self.http, session);
        let folder = match &self.folder_id {
            Some(id) => id.clone(),
            None => {
                let id = drive.ensure_folder(&self.cfg.folder_id, &self.cfg.folder_name)?;
                self.folder_id = Some(id.clone());
                id
            }
        };
        Ok((drive, folder))
    }

    fn push(&mut self, ws: &Workspace) -> Result<usize> {
        let export_md = self.cfg.export_markdown;
        let (mut drive, folder) = self.drive()?;
        let json = serde_json::to_vec_pretty(ws)?;
        drive.upsert(&folder, REMOTE_WORKSPACE, "application/json", json)?;
        let mut files = 1;
        if export_md {
            for (page, name) in ws.pages.iter().zip(export::unique_file_names(&ws.pages)) {
                drive.upsert(
                    &folder,
                    &name,
                    "text/markdown",
                    export::page_to_markdown(page).into_bytes(),
                )?;
                files += 1;
            }
        }
        Ok(files)
    }

    fn pull(&mut self) -> Result<Workspace> {
        let (mut drive, folder) = self.drive()?;
        let file = drive
            .find_in_folder(&folder, REMOTE_WORKSPACE)?
            .with_context(|| {
                format!("chưa có {REMOTE_WORKSPACE} trên Drive — hãy đẩy lên trước")
            })?;
        let bytes = drive.download(&file.id)?;
        let ws: Workspace = serde_json::from_slice(&bytes).context("file trên Drive hỏng")?;
        Ok(ws.normalize())
    }
}
