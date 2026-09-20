//! UI pieces. Each returns [`Action`]s; `app.rs` applies them to the state,
//! which keeps borrowing simple and the widgets free of business logic.

pub mod canvas;
pub mod dialogs;
pub mod links;
pub mod md_highlight;
pub mod toolbar;

pub enum Action {
    SelectPage(usize),
    AddPage,
    RenamePage(usize, String),
    DeletePage(usize),
    AddNote,
    Tidy,
    /// Zoom the canvas by N steps (negative = out).
    Zoom(i32),
    ZoomReset,
    Undo,
    Save,
    Push,
    Pull,
    Login,
    OpenSettings,
    ExportMarkdown,
    ToggleDockPin,
    Quit,
    /// Show the panel and keep it open (from the tray).
    ShowPanel,
    SetAutostart(bool),
    /// Tách phần bôi đen của note này thành note con.
    SplitSelection(uuid::Uuid),
}
