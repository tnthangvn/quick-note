//! Background Drive sync worker. The UI sends [`Command`]s and polls [`Event`]s,
//! so network calls never block rendering.

mod api;
mod oauth;
mod service_account;

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::config::DriveConfig;
use crate::export;
use crate::model::Workspace;
use api::Drive;
use oauth::Session;
use service_account::ServiceAccount;

/// Anything that can hand out a Drive access token.
pub(crate) trait TokenSource {
    fn token(&mut self, http: &reqwest::blocking::Client) -> Result<String>;
}

/// How the worker is authenticated right now.
enum Auth {
    /// Browser login as the user (personal accounts).
    User(Session),
    /// Key file; needs a Shared Drive folder (service accounts have no quota).
    Service(Box<ServiceAccount>),
}

impl TokenSource for Auth {
    fn token(&mut self, http: &reqwest::blocking::Client) -> Result<String> {
        match self {
            Self::User(session) => session.access_token(http),
            Self::Service(sa) => sa.access_token(http),
        }
    }
}

pub const REMOTE_WORKSPACE: &str = "quick-note-workspace.json";
/// Dùng khi ô "Tên thư mục" bị bỏ trống.
const DEFAULT_FOLDER_NAME: &str = "QuickNote";
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
    /// Something worth a toast (e.g. which service account was loaded).
    Info(String),
    LoggedIn,
    LoggedOut,
    Pushed {
        files: usize,
    },
    Pulled(Box<Workspace>),
    Error(String),
}

/// Kiểm tra cấu hình Drive mà KHÔNG ghi gì: xác thực rồi liệt kê Shared Drive.
pub fn check(cfg: &DriveConfig, dir: &std::path::Path) -> Result<String> {
    let http = reqwest::blocking::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()?;
    let mut report = String::new();
    let mut auth = if cfg.uses_service_account() {
        let key = std::path::Path::new(cfg.service_account_key.trim());
        let sa = ServiceAccount::load(key, &cfg.impersonate)?;
        report.push_str(&format!("Service account: {}\n", sa.email()));
        match sa.subject() {
            Some(user) => report.push_str(&format!("Đóng vai: {user}\n")),
            None => report.push_str("Đóng vai: (không) — chỉ ghi được vào Shared Drive\n"),
        }
        Auth::Service(Box::new(sa))
    } else {
        let session = Session::restore(cfg, dir)?
            .context("chưa đăng nhập Google (bấm Kết nối Drive trong app)")?;
        report.push_str("Đăng nhập người dùng: đã có token\n");
        Auth::User(session)
    };
    auth.token(&http).context("lấy access token thất bại")?;
    report.push_str("Access token: OK\n");

    let mut drive = Drive::new(&http, &mut auth);
    let drives = drive.shared_drives()?;
    if drives.is_empty() {
        report.push_str("Shared Drive: (không có) — thêm email ở trên vào một Shared Drive\n");
    } else {
        report.push_str("Shared Drive thấy được:\n");
        for (id, name) in &drives {
            report.push_str(&format!("  - {name}  ({id})\n"));
        }
    }
    let folders = drive.visible_folders(10)?;
    if folders.is_empty() {
        report.push_str("Thư mục thấy được: (không có)\n");
    } else {
        report.push_str("Thư mục thấy được:\n");
        for f in &folders {
            let name = f["name"].as_str().unwrap_or("?");
            let id = f["id"].as_str().unwrap_or("?");
            let place = match f["driveId"].as_str() {
                Some(drive_id) => format!("Shared Drive {drive_id}"),
                None => "My Drive (KHÔNG ghi được: service account hết quota)".to_string(),
            };
            report.push_str(&format!("  - {name}  ({id})  [{place}]\n"));
        }
    }
    if !cfg.folder_id.trim().is_empty() {
        report.push_str(&format!(
            "Folder ID đã cấu hình: {}\n",
            cfg.folder_id.trim()
        ));
    }
    Ok(report)
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
    auth: Option<Auth>,
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
            auth: None,
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

    /// Picks up whatever credentials are configured, without user interaction.
    fn restore_session(&mut self) {
        if self.cfg.uses_service_account() {
            let key = std::path::Path::new(self.cfg.service_account_key.trim());
            match ServiceAccount::load(key, &self.cfg.impersonate) {
                Ok(sa) => {
                    self.emit(Event::Info(format!("Dùng service account {}", sa.email())));
                    self.auth = Some(Auth::Service(Box::new(sa)));
                    self.emit(Event::LoggedIn);
                }
                Err(e) => self.emit(Event::Error(format!("service account: {e:#}"))),
            }
            return;
        }
        match Session::restore(&self.cfg, &self.dir) {
            Ok(Some(session)) => {
                self.auth = Some(Auth::User(session));
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
                    self.auth = None;
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
                // Credentials are cached inside the session, so rebuild it when they change.
                let scope_changed = cfg.scope() != self.cfg.scope()
                    || cfg.client_id != self.cfg.client_id
                    || cfg.client_secret != self.cfg.client_secret
                    || cfg.service_account_key != self.cfg.service_account_key;
                let folder_changed =
                    cfg.folder_id != self.cfg.folder_id || cfg.folder_name != self.cfg.folder_name;
                self.cfg = cfg;
                if folder_changed {
                    self.folder_id = None;
                }
                if scope_changed && self.auth.take().is_some() {
                    self.emit(Event::LoggedOut);
                    self.restore_session();
                }
            }
            Command::Login => {
                if self.cfg.uses_service_account() {
                    // Nothing interactive to do: just (re)read the key file.
                    self.folder_id = None;
                    self.restore_session();
                    return Ok(());
                }
                self.emit(Event::Busy(
                    "Đang chờ đăng nhập Google trên trình duyệt…".into(),
                ));
                self.auth = Some(Auth::User(Session::login(
                    &self.cfg, &self.dir, &self.http,
                )?));
                self.folder_id = None;
                self.emit(Event::LoggedIn);
            }
            Command::Logout => {
                self.auth = None;
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
        let folder_id = self.cfg.folder_id.trim();
        if folder_id.contains('/') {
            bail!(
                "Folder ID phải là mã trong URL drive.google.com/drive/folders/<ID>, không phải đường dẫn"
            );
        }
        let service_account = self.cfg.uses_service_account();
        let auth = self.auth.as_mut().context("chưa kết nối Google Drive")?;
        let mut drive = Drive::new(&self.http, auth);
        let folder = match &self.folder_id {
            Some(id) => id.clone(),
            None => {
                // Service account không có My Drive: chứa file trong Shared Drive.
                let impersonating = !self.cfg.impersonate.trim().is_empty();
                let parent = if service_account && folder_id.is_empty() && !impersonating {
                    let drives = drive.shared_drives()?;
                    let (id, name) = drives.into_iter().next().context(
                        "service account chưa được thêm vào Shared Drive nào — thêm email của nó \
                         vào một Shared Drive (quyền Content manager), hoặc nhập Folder ID",
                    )?;
                    self.events
                        .send(Event::Info(format!("Dùng Shared Drive \"{name}\"")))
                        .ok();
                    id
                } else {
                    "root".to_string()
                };
                let name = if self.cfg.folder_name.trim().is_empty() {
                    DEFAULT_FOLDER_NAME
                } else {
                    self.cfg.folder_name.trim()
                };
                let id = drive.ensure_folder(folder_id, name, &parent)?;
                self.folder_id = Some(id.clone());
                id
            }
        };
        Ok((drive, folder))
    }

    fn push(&mut self, ws: &Workspace) -> Result<usize> {
        let export_md = self.cfg.export_markdown;
        // Ảnh dán trong note nằm ở file riêng: đẩy kèm để bản sao đủ dùng.
        let attachments: Vec<std::path::PathBuf> = ws
            .pages
            .iter()
            .flat_map(|p| p.notes.iter())
            .flat_map(|n| crate::attach::referenced_files(&n.body))
            .filter(|p| p.is_file())
            .collect();
        let (mut drive, folder) = self.drive()?;
        let json = serde_json::to_vec_pretty(ws)?;
        drive.upsert(&folder, REMOTE_WORKSPACE, "application/json", json)?;
        let mut files = 1;
        for path in attachments {
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let bytes = std::fs::read(&path)?;
            drive.upsert(&folder, name, "image/png", bytes)?;
            files += 1;
        }
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
