//! Data model: a workspace holds pages, a page holds freely placed notes.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MIN_NOTE_SIZE: [f32; 2] = [140.0, 90.0];
pub const DEFAULT_NOTE_SIZE: [f32; 2] = [240.0, 180.0];

/// Sticky-note palette (RGB). Index stored on the note.
pub const NOTE_COLORS: [[u8; 3]; 6] = [
    [255, 241, 168], // yellow
    [255, 205, 210], // pink
    [200, 230, 201], // green
    [187, 222, 251], // blue
    [225, 190, 231], // purple
    [236, 239, 241], // grey
];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: Uuid,
    pub title: String,
    pub body: String,
    /// Top-left corner in canvas coordinates.
    pub pos: [f32; 2],
    pub size: [f32; 2],
    #[serde(default)]
    pub color: usize,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub updated_at: i64,
}

impl Note {
    pub fn new(pos: [f32; 2], color: usize) -> Self {
        Self {
            id: Uuid::new_v4(),
            title: String::new(),
            body: String::new(),
            pos,
            size: DEFAULT_NOTE_SIZE,
            color: color % NOTE_COLORS.len(),
            pinned: false,
            updated_at: now_ts(),
        }
    }

    pub fn display_title(&self) -> &str {
        if !self.title.trim().is_empty() {
            return self.title.trim();
        }
        // Markdown heading markers are not part of the title.
        self.body
            .lines()
            .map(|l| l.trim().trim_start_matches('#').trim())
            .find(|l| !l.is_empty())
            .unwrap_or("(trống)")
    }

    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.title.to_lowercase().contains(&q) || self.body.to_lowercase().contains(&q)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page {
    pub id: Uuid,
    pub title: String,
    /// Draw order: last note is on top.
    pub notes: Vec<Note>,
    /// Canvas pan offset.
    #[serde(default)]
    pub pan: [f32; 2],
    /// Canvas zoom factor (1.0 = 100%).
    #[serde(default = "default_zoom")]
    pub zoom: f32,
    /// Arrows between notes (diagram edges).
    #[serde(default)]
    pub links: Vec<Link>,
}

/// A directed arrow from one note to another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    pub id: Uuid,
    pub from: Uuid,
    pub to: Uuid,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub dashed: bool,
}

impl Link {
    pub fn new(from: Uuid, to: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            from,
            to,
            label: String::new(),
            dashed: false,
        }
    }

    pub fn touches(&self, note: Uuid) -> bool {
        self.from == note || self.to == note
    }
}

fn default_zoom() -> f32 {
    1.0
}

pub const ZOOM_MIN: f32 = 0.3;
pub const ZOOM_MAX: f32 = 3.0;
pub const ZOOM_STEP: f32 = 1.15;

impl Page {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            title: title.into(),
            notes: Vec::new(),
            pan: [0.0, 0.0],
            zoom: 1.0,
            links: Vec::new(),
        }
    }

    /// Adds a note, cascading its position so new notes don't stack exactly.
    pub fn add_note(&mut self, anchor: [f32; 2]) -> Uuid {
        let step = (self.notes.len() % 8) as f32 * 24.0;
        self.add_note_at([anchor[0] + step, anchor[1] + step])
    }

    pub fn add_note_at(&mut self, pos: [f32; 2]) -> Uuid {
        let note = Note::new(pos, self.notes.len());
        let id = note.id;
        self.notes.push(note);
        id
    }

    pub fn insert_note(&mut self, note: Note) {
        self.notes.push(note);
    }

    pub fn bring_to_front(&mut self, id: Uuid) {
        if let Some(idx) = self.notes.iter().position(|n| n.id == id)
            && idx + 1 != self.notes.len()
        {
            let note = self.notes.remove(idx);
            self.notes.push(note);
        }
    }

    /// Removes a note and the arrows attached to it (returned for undo).
    pub fn remove_note(&mut self, id: Uuid) -> Option<(Note, Vec<Link>)> {
        let idx = self.notes.iter().position(|n| n.id == id)?;
        let note = self.notes.remove(idx);
        let (gone, kept) = std::mem::take(&mut self.links)
            .into_iter()
            .partition(|l| l.touches(id));
        self.links = kept;
        Some((note, gone))
    }

    /// Adds an arrow unless it would be a self-loop or a duplicate of an existing one
    /// (in either direction). Returns whether one was added.
    pub fn connect(&mut self, from: Uuid, to: Uuid) -> bool {
        let exists = self.links.iter().any(|l| l.touches(from) && l.touches(to));
        let known = |id| self.notes.iter().any(|n| n.id == id);
        if from == to || exists || !known(from) || !known(to) {
            return false;
        }
        self.links.push(Link::new(from, to));
        true
    }

    pub fn remove_link(&mut self, id: Uuid) -> Option<Link> {
        let idx = self.links.iter().position(|l| l.id == id)?;
        Some(self.links.remove(idx))
    }

    /// Converts a point relative to the canvas top-left into canvas coordinates.
    pub fn to_canvas(&self, screen: [f32; 2]) -> [f32; 2] {
        [
            (screen[0] - self.pan[0]) / self.zoom,
            (screen[1] - self.pan[1]) / self.zoom,
        ]
    }

    /// Zooms keeping the canvas point under `anchor` (relative to canvas top-left) fixed.
    pub fn zoom_at(&mut self, anchor: [f32; 2], zoom: f32) {
        let fixed = self.to_canvas(anchor);
        self.zoom = zoom.clamp(ZOOM_MIN, ZOOM_MAX);
        self.pan = [
            anchor[0] - fixed[0] * self.zoom,
            anchor[1] - fixed[1] * self.zoom,
        ];
    }

    /// Lays notes out in a left-to-right grid, keeping their sizes.
    pub fn tidy(&mut self, max_width: f32) {
        const GAP: f32 = 16.0;
        let (mut x, mut y, mut row_h) = (GAP, GAP, 0.0_f32);
        for note in &mut self.notes {
            if x + note.size[0] > max_width && x > GAP {
                x = GAP;
                y += row_h + GAP;
                row_h = 0.0;
            }
            note.pos = [x, y];
            x += note.size[0] + GAP;
            row_h = row_h.max(note.size[1]);
        }
        self.pan = [0.0, 0.0];
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub version: u32,
    pub pages: Vec<Page>,
    pub active: usize,
    #[serde(default)]
    pub updated_at: i64,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            version: 1,
            pages: vec![Page::new("Trang 1")],
            active: 0,
            updated_at: now_ts(),
        }
    }
}

impl Workspace {
    pub fn active_page(&self) -> &Page {
        &self.pages[self.active.min(self.pages.len() - 1)]
    }

    pub fn active_page_mut(&mut self) -> &mut Page {
        let idx = self.active.min(self.pages.len() - 1);
        &mut self.pages[idx]
    }

    pub fn add_page(&mut self) -> usize {
        let title = format!("Trang {}", self.pages.len() + 1);
        self.pages.push(Page::new(title));
        self.active = self.pages.len() - 1;
        self.active
    }

    /// Removes a page; a workspace always keeps at least one page.
    pub fn remove_page(&mut self, idx: usize) -> Option<Page> {
        if self.pages.len() <= 1 || idx >= self.pages.len() {
            return None;
        }
        let page = self.pages.remove(idx);
        if self.active >= self.pages.len() || self.active > idx {
            self.active = self.active.saturating_sub(1);
        }
        Some(page)
    }

    /// Repairs invariants after loading from disk / Drive.
    pub fn normalize(mut self) -> Self {
        if self.pages.is_empty() {
            self.pages.push(Page::new("Trang 1"));
        }
        self.active = self.active.min(self.pages.len() - 1);
        for page in &mut self.pages {
            if !page.zoom.is_finite() {
                page.zoom = 1.0;
            }
            page.zoom = page.zoom.clamp(ZOOM_MIN, ZOOM_MAX);
            let ids: std::collections::HashSet<Uuid> = page.notes.iter().map(|n| n.id).collect();
            page.links
                .retain(|l| l.from != l.to && ids.contains(&l.from) && ids.contains(&l.to));
        }
        for note in self.pages.iter_mut().flat_map(|p| p.notes.iter_mut()) {
            note.size[0] = note.size[0].max(MIN_NOTE_SIZE[0]);
            note.size[1] = note.size[1].max(MIN_NOTE_SIZE[1]);
            note.color %= NOTE_COLORS.len();
        }
        self
    }

    pub fn touch(&mut self) {
        self.updated_at = now_ts();
    }
}

pub fn now_ts() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_page_keeps_at_least_one() {
        let mut ws = Workspace::default();
        assert!(ws.remove_page(0).is_none());
        ws.add_page();
        assert!(ws.remove_page(0).is_some());
        assert_eq!(ws.pages.len(), 1);
        assert_eq!(ws.active, 0);
    }

    #[test]
    fn remove_page_before_active_shifts_active() {
        let mut ws = Workspace::default();
        ws.add_page();
        ws.add_page();
        assert_eq!(ws.active, 2);
        ws.remove_page(0);
        assert_eq!(ws.active, 1);
        assert_eq!(ws.active_page().title, "Trang 3");
    }

    #[test]
    fn bring_to_front_moves_note_last() {
        let mut page = Page::new("p");
        let a = page.add_note([0.0, 0.0]);
        let b = page.add_note([0.0, 0.0]);
        page.bring_to_front(a);
        assert_eq!(page.notes.last().unwrap().id, a);
        assert_eq!(page.notes[0].id, b);
    }

    #[test]
    fn normalize_repairs_bad_data() {
        let mut ws = Workspace {
            version: 1,
            pages: vec![],
            active: 9,
            updated_at: 0,
        };
        ws = ws.normalize();
        assert_eq!(ws.pages.len(), 1);
        assert_eq!(ws.active, 0);

        let mut page = Page::new("p");
        page.add_note([0.0, 0.0]);
        page.notes[0].size = [1.0, 1.0];
        page.notes[0].color = 99;
        let ws = Workspace {
            version: 1,
            pages: vec![page],
            active: 0,
            updated_at: 0,
        }
        .normalize();
        let note = &ws.pages[0].notes[0];
        assert_eq!(note.size, MIN_NOTE_SIZE);
        assert!(note.color < NOTE_COLORS.len());
    }

    #[test]
    fn display_title_falls_back_to_first_body_line() {
        let mut note = Note::new([0.0, 0.0], 0);
        assert_eq!(note.display_title(), "(trống)");
        note.body = "\n  mua sữa \nabc".into();
        assert_eq!(note.display_title(), "mua sữa");
        note.body = "## Kế hoạch\n- a".into();
        assert_eq!(note.display_title(), "Kế hoạch");
        note.title = "Việc".into();
        assert_eq!(note.display_title(), "Việc");
    }

    #[test]
    fn matches_is_case_insensitive() {
        let mut note = Note::new([0.0, 0.0], 0);
        note.body = "Họp Team lúc 9h".into();
        assert!(note.matches("team"));
        assert!(!note.matches("khách"));
    }

    #[test]
    fn zoom_at_keeps_anchor_point_fixed() {
        let mut page = Page::new("p");
        page.pan = [30.0, -10.0];
        let anchor = [200.0, 150.0];
        let before = page.to_canvas(anchor);
        page.zoom_at(anchor, 2.0);
        assert_eq!(page.zoom, 2.0);
        let after = page.to_canvas(anchor);
        assert!((before[0] - after[0]).abs() < 1e-4 && (before[1] - after[1]).abs() < 1e-4);
    }

    #[test]
    fn zoom_is_clamped() {
        let mut page = Page::new("p");
        page.zoom_at([0.0, 0.0], 100.0);
        assert_eq!(page.zoom, ZOOM_MAX);
        page.zoom_at([0.0, 0.0], 0.0);
        assert_eq!(page.zoom, ZOOM_MIN);
    }

    #[test]
    fn old_files_without_zoom_load_at_100_percent() {
        let page: Page = serde_json::from_str(
            r#"{"id":"00000000-0000-0000-0000-000000000000","title":"p","notes":[]}"#,
        )
        .unwrap();
        assert_eq!(page.zoom, 1.0);
    }

    #[test]
    fn connect_rejects_self_loops_duplicates_and_unknown_notes() {
        let mut page = Page::new("p");
        let a = page.add_note([0.0, 0.0]);
        let b = page.add_note([0.0, 0.0]);
        assert!(page.connect(a, b));
        assert!(!page.connect(a, b), "duplicate");
        assert!(!page.connect(b, a), "reverse duplicate");
        assert!(!page.connect(a, a), "self loop");
        assert!(!page.connect(a, Uuid::new_v4()), "unknown note");
        assert_eq!(page.links.len(), 1);
    }

    #[test]
    fn removing_note_takes_its_links() {
        let mut page = Page::new("p");
        let a = page.add_note([0.0, 0.0]);
        let b = page.add_note([0.0, 0.0]);
        let c = page.add_note([0.0, 0.0]);
        page.connect(a, b);
        page.connect(b, c);
        page.connect(a, c);
        let (_, gone) = page.remove_note(b).unwrap();
        assert_eq!(gone.len(), 2);
        assert_eq!(page.links.len(), 1);
    }

    #[test]
    fn normalize_drops_dangling_links() {
        let mut page = Page::new("p");
        let a = page.add_note([0.0, 0.0]);
        page.links.push(Link::new(a, Uuid::new_v4()));
        let ws = Workspace {
            version: 1,
            pages: vec![page],
            active: 0,
            updated_at: 0,
        }
        .normalize();
        assert!(ws.pages[0].links.is_empty());
    }

    #[test]
    fn tidy_wraps_rows() {
        let mut page = Page::new("p");
        for _ in 0..3 {
            page.add_note([500.0, 500.0]);
        }
        page.tidy(600.0);
        assert_eq!(page.notes[0].pos, [16.0, 16.0]);
        assert_eq!(page.notes[1].pos[1], 16.0);
        assert!(page.notes[2].pos[1] > 16.0, "third note wraps to next row");
    }
}
