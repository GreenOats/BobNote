use crate::{
    colors::BG_PALETTE,
    pty::PtySession,
    terminal::{CapturedRow, capture_scrollback_rows, capture_screen_before_resize},
};
use anyhow::Result;
use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Serialisable colour / cell types used by Photo notes
// ---------------------------------------------------------------------------

/// A lossless, serde-friendly mirror of `ratatui::style::Color`.
/// Only the three variants produced by `terminal::map_color` are needed.
#[derive(Serialize, Deserialize, Clone, Default)]
pub enum SerColor {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl From<Color> for SerColor {
    fn from(c: Color) -> Self {
        match c {
            Color::Reset           => SerColor::Default,
            Color::Indexed(i)      => SerColor::Indexed(i),
            Color::Rgb(r, g, b)   => SerColor::Rgb(r, g, b),
            _                      => SerColor::Default,
        }
    }
}

impl From<SerColor> for Color {
    fn from(c: SerColor) -> Self {
        match c {
            SerColor::Default        => Color::Reset,
            SerColor::Indexed(i)     => Color::Indexed(i),
            SerColor::Rgb(r, g, b)  => Color::Rgb(r, g, b),
        }
    }
}

/// One captured terminal cell, serialisable for persistence.
#[derive(Serialize, Deserialize, Clone, Default)]
pub struct PhotoCell {
    pub sym: String,
    pub fg: SerColor,
    pub bg: SerColor,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub reversed: bool,
}

/// One row of captured cells.
pub type PhotoRow = Vec<PhotoCell>;
use std::{
    collections::VecDeque,
    fs,
    io::BufWriter,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use tui_textarea::TextArea;

/// Returns a random index into BG_PALETTE, always skipping index 0 (transparent).
fn random_bg_color() -> usize {
    let non_transparent = BG_PALETTE.len() - 1; // indices 1..len
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize % non_transparent + 1)
        .unwrap_or(1)
}

fn default_true() -> bool {
    true
}

/// The live content of a note — text editor, embedded shell, or captured screenshot.
pub enum NoteKind {
    /// The `usize` is the scroll-top offset used only when `NoteData::text_wrap`
    /// is enabled (the Paragraph path tracks its own viewport like CheckList).
    Text(TextArea<'static>, usize),
    /// A frozen screenshot of a terminal region.  All data lives in
    /// `NoteData.photo_rows`; this variant just marks the note kind.
    Photo,
    /// WIP A basic Checklist type note!
    /// The `usize` is the scroll-top offset (first visible line index),
    /// updated each render frame to mirror tui_textarea's lazy-scroll logic.
    CheckList(TextArea<'static>, usize),
    Shell {
        pty: PtySession,
        parser: vt100::Parser,
        /// Tracked so that only call is resize() when dimensions actually change.
        rows: u16,
        cols: u16,
        /// Scroll position.
        /// Positive → scrolled up into history (lines above live view).
        /// Zero     → live view (prompt at bottom).
        /// Negative → scrolled down below live view (prompt moving toward top).
        /// Range: [-(rows-1), own_scrollback.len()]
        scroll_offset: i64,
        /// Captured scrollback lines that have scrolled past vt100's accessible
        /// window (which is limited to `rows` lines by the vt100 API).
        own_scrollback: VecDeque<CapturedRow>,
        /// Fingerprints of the previously accessible vt100 scrollback rows, used
        /// by `capture_scrollback_rows` to detect which rows are new each frame.
        sb_prev_fps: Vec<u64>,
        /// Name of the foreground process currently running in this PTY (None = shell).
        /// Updated each frame by reading /proc/<shell_pid>/task/<shell_pid>/children.
        active_app: Option<String>,
        /// Background colour sampled from the vt100 screen while an app is active.
        /// Used to tint the note border to match the running application's theme.
        /// Cleared when the foreground process returns to the shell.
        detected_bg: Option<Color>,
        /// When true, the first batch of PTY output triggers a VT100 screen clear
        /// (ESC[2J ESC[H) injected directly into the parser — no shell command is
        /// issued, so history is unaffected and own_scrollback is preserved.
        /// Set on restored shell notes to hide startup noise (cd, prompts, motd).
        startup_clear_pending: bool,
        /// Active log file writer — `Some` means logging is on for this shell.
        /// Dropped (flushed + closed) automatically when set back to `None`.
        log_file: Option<BufWriter<fs::File>>,
        /// Path of the currently active log file; `None` when not logging.
        log_path: Option<PathBuf>,
    },
}

/// The persisted portion of a note (saved to disk as JSON).
#[derive(Serialize, Deserialize, Clone)]
pub struct NoteData {
    pub id: u64,
    pub title: String,
    pub content: Vec<String>, // one entry per line (text notes only)
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
    /// Index into `colors::BORDER_PALETTE`. Defaults to 0 (Yellow).
    #[serde(default)]
    pub border_color_idx: usize,
    /// Index into `colors::BG_PALETTE`. Defaults to 0 (None / transparent).
    #[serde(default)]
    pub bg_color_idx: usize,
    /// Whether to draw a full border box around the note. Shell notes always
    /// use a border; text notes default to borderless (just a title bar).
    #[serde(default = "default_true")]
    pub show_border: bool,
    /// If true this note hosts its own shell session instead of a text area.
    #[serde(default)]
    pub is_shell: bool,
    /// If true this note is a frozen terminal screenshot (Photo note).
    #[serde(default)]
    pub is_photo: bool,
    /// Captured cell data for Photo notes (empty for Text/Shell notes).
    #[serde(default)]
    pub photo_rows: Vec<PhotoRow>,
    /// If true this note is a checklist note
    #[serde(default)]
    pub is_checklist: bool,
    /// If true the note is stored on the corkboard, not the main view.
    #[serde(default)]
    pub on_corkboard: bool,
    /// If true, text-note content wraps inside the note width (Paragraph mode).
    #[serde(default)]
    pub text_wrap: bool,
    /// If true, this note is always rendered on top of all non-pinned notes.
    #[serde(default)]
    pub pinned: bool,
    /// ID of the notebook this note belongs to, if any.
    #[serde(default)]
    pub notebook_id: Option<u64>,
    /// Workspace this note belongs to (0 = default workspace).
    #[serde(default)]
    pub workspace_id: u8,
    /// If true, this shell note fills the full screen as the background of its workspace,
    /// replacing App.pty for that workspace. Has no border.
    #[serde(default)]
    pub is_background: bool,
    /// Saved working directory of the shell process at last quit.
    /// Restored via `cd` into the fresh PTY on next launch.
    /// Only populated for non-background shell notes.
    #[serde(default)]
    pub saved_cwd: Option<String>,
    /// Captured terminal rows (scrollback + last visible screen), saved on quit.
    /// Restored into `own_scrollback` on next launch so history is immediately browsable.
    /// Only populated for non-background shell notes.
    #[serde(default)]
    pub saved_scrollback: Vec<PhotoRow>,
}

/// Runtime note — combines persisted data with live content.
pub struct Note {
    pub data: NoteData,
    pub kind: NoteKind,
}

impl Note {
    /// Create a new text note.
    pub fn new(id: u64, x: u16, y: u16, width: u16, height: u16) -> Self {
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(Style::default());
        Self {
            data: NoteData {
                id,
                title: format!("Note {id}"),
                content: vec![],
                x,
                y,
                width,
                height,
                border_color_idx: 0,
                bg_color_idx: random_bg_color(),
                show_border: false,
                is_shell: false,
                is_photo: false,
                photo_rows: vec![],
                is_checklist: false,
                on_corkboard: false,
                text_wrap: true,
                pinned: false,
                notebook_id: None,
                workspace_id: 0,
                is_background: false,
                saved_cwd: None,
                saved_scrollback: vec![],
            },
            kind: NoteKind::Text(textarea, 0),
        }
    }

    pub fn new_checklist(id: u64, x: u16, y:u16, width: u16, height: u16) -> Self {
        
        let mut textarea = TextArea::default();
        textarea.set_cursor_line_style(Style::default());
 
        Self {
                data: NoteData { id,
                title: format!("Checklist {id}"), 
                content: vec![], 
                x, 
                y, 
               width, 
                height, 
                border_color_idx: 0, 
                bg_color_idx: random_bg_color(), 
                show_border: false, 
                is_shell: false, 
                is_photo: false,
                photo_rows: vec![], 
                is_checklist: true,
                on_corkboard: false,
                text_wrap: false,
                pinned: false,
                notebook_id: None,
                workspace_id: 0,
                is_background: false,
                saved_cwd: None,
                saved_scrollback: vec![],
            },
            kind: NoteKind::CheckList(textarea, 0),
        }
    }

    /// Create a new shell note with its own PTY session.
    pub fn new_shell(id: u64, x: u16, y: u16, width: u16, height: u16, scrollback: usize) -> Result<Self> {
        let cols = width.saturating_sub(2).max(2);
        let rows = height.saturating_sub(2).max(2);
        let pty = PtySession::new(rows, cols, None)?;
        let parser = vt100::Parser::new(rows, cols, scrollback);
        Ok(Self {
            data: NoteData {
                id,
                title: format!("Shell {id}"),
                content: vec![],
                x,
                y,
                width,
                height,
                border_color_idx: 0,
                bg_color_idx: 0,
                show_border: true,
                is_shell: true,
                is_photo: false,
                photo_rows: vec![],
                is_checklist: false,
                on_corkboard: false,
                text_wrap: false,
                pinned: false,
                notebook_id: None,
                workspace_id: 0,
                is_background: false,
                saved_cwd: None,
                saved_scrollback: vec![],
            },
            kind: NoteKind::Shell {
                pty,
                parser,
                rows,
                cols,
                scroll_offset: 0,
                own_scrollback: VecDeque::new(),
                sb_prev_fps: Vec::new(),
                active_app: None,
                detected_bg: None,
                startup_clear_pending: false,
                log_file: None,
                log_path: None,
            },
        })
    }

    /// Restore a saved note from disk.
    /// Shell notes get a fresh PTY (their session state is ephemeral).
    /// Photo notes restore their captured rows directly from NoteData.
    pub fn from_data(data: NoteData, scrollback: usize) -> Result<Self> {
        if data.is_photo {
            return Ok(Self { kind: NoteKind::Photo, data });
        }
        if data.is_shell {
            let cols = data.width.saturating_sub(2).max(2);
            let rows = data.height.saturating_sub(2).max(2);
            let cwd = data.saved_cwd.as_deref();
            let mut pty = PtySession::new(rows, cols, cwd)?;
            let parser = vt100::Parser::new(rows, cols, scrollback);
            // Restore saved scrollback so the user can scroll up into prior history.
            let own_scrollback: VecDeque<CapturedRow> = data.saved_scrollback.iter()
                .map(|row| {
                    row.iter().map(|cell| {
                        let mut modifier = Modifier::empty();
                        if cell.bold      { modifier |= Modifier::BOLD; }
                        if cell.italic    { modifier |= Modifier::ITALIC; }
                        if cell.underline { modifier |= Modifier::UNDERLINED; }
                        if cell.reversed  { modifier |= Modifier::REVERSED; }
                        (cell.sym.clone(), Color::from(cell.fg.clone()), Color::from(cell.bg.clone()), modifier)
                    }).collect()
                })
                .collect();
            Ok(Self {
                kind: NoteKind::Shell {
                    pty,
                    parser,
                    rows,
                    cols,
                    scroll_offset: 0,
                    own_scrollback,
                    sb_prev_fps: Vec::new(),
                    active_app: None,
                    detected_bg: None,
                    startup_clear_pending: true,
                    log_file: None,
                    log_path: None,
                },
                data,
            })
        } else if data.is_checklist {
            let lines = data.content.clone();
            let mut textarea = TextArea::from(lines);
            textarea.set_cursor_line_style(Style::default());
            Ok(Self {
                kind: NoteKind::CheckList(textarea, 0),
                data,
            })
        } else {
            let lines = data.content.clone();
            let mut textarea = TextArea::from(lines);
            textarea.set_cursor_line_style(Style::default());
            Ok(Self {
                kind: NoteKind::Text(textarea, 0),
                data,
            })
        }
    }

    /// Create a new Photo note from a captured region.
    pub fn new_photo(id: u64, x: u16, y: u16, width: u16, height: u16, rows: Vec<PhotoRow>) -> Self {
        Self {
            data: NoteData {
                id,
                title: format!("Photo {id}"),
                content: vec![],
                x,
                y,
                width,
                height,
                border_color_idx: 0,
                bg_color_idx: 0,
                show_border: true,
                is_shell: false,
                is_photo: true,
                photo_rows: rows,
                is_checklist: false,
                on_corkboard: false,
                text_wrap: false,
                pinned: false,
                notebook_id: None,
                workspace_id: 0,
                is_background: false,
                saved_cwd: None,
                saved_scrollback: vec![],
            },
            kind: NoteKind::Photo,
        }
    }

    /// Sync textarea contents back into NoteData before saving.
    /// Shell and Photo notes have no mutable runtime content to sync.
    pub fn sync(&mut self) {
        match &self.kind {
            NoteKind::Text(ta, _) | NoteKind::CheckList(ta, _) => {
                self.data.content = ta.lines().to_vec();
            }
            _ => {}
        }
    }

    /// Snapshot shell-note runtime state into NoteData for persistence.
    ///
    /// Saves the current working directory and the full scrollback (including the
    /// rows still inside vt100's accessible window and the live visible screen)
    /// so that the next session can restore history and `cd` to the right directory.
    ///
    /// Background shell notes are skipped — they are ephemeral workspace terminals.
    pub fn sync_shell(&mut self) {

        // Collect data from NoteKind::Shell while mutably borrowing `kind`,
        // then write results to `data` after the borrow ends.
        let result: Option<(Option<String>, Vec<PhotoRow>)> =
            if let NoteKind::Shell { pty, parser, own_scrollback, sb_prev_fps, rows, .. } =
                &mut self.kind
            {
                // Save CWD via /proc/<pid>/cwd symlink (Linux).
                let saved_cwd = pty.shell_pid
                    .and_then(|pid| std::fs::read_link(format!("/proc/{pid}/cwd")).ok())
                    .and_then(|p| p.to_str().map(String::from));

                // Flush vt100's accessible scrollback window into own_scrollback.
                capture_scrollback_rows(parser, own_scrollback, sb_prev_fps, *rows, usize::MAX);
                // Reset to the live view and capture the visible screen too,
                // so no rows are lost between the last render frame and quit.
                parser.set_scrollback(0);
                capture_screen_before_resize(parser, own_scrollback, sb_prev_fps, usize::MAX);

                // Convert CapturedRow → PhotoRow for serde serialisation.
                let saved_scrollback = own_scrollback.iter()
                    .map(|row| {
                        row.iter().map(|(sym, fg, bg, modifier)| PhotoCell {
                            sym: sym.clone(),
                            fg: SerColor::from(*fg),
                            bg: SerColor::from(*bg),
                            bold:      modifier.contains(Modifier::BOLD),
                            italic:    modifier.contains(Modifier::ITALIC),
                            underline: modifier.contains(Modifier::UNDERLINED),
                            reversed:  modifier.contains(Modifier::REVERSED),
                        }).collect()
                    })
                    .collect();

                Some((saved_cwd, saved_scrollback))
            } else {
                None
            };

        if let Some((cwd, scrollback)) = result {
            self.data.saved_cwd = cwd;
            self.data.saved_scrollback = scrollback;
        }
    }
}

///******************************************************************************Give Me PERSISTENCE!

fn notes_dir() -> Result<PathBuf> {
    let base = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("bobnote");
    fs::create_dir_all(&base)?;
    Ok(base)
}

/// Save a single note immediately — syncs text/checklist content but skips the
/// expensive shell scrollback capture (that still happens on quit).  Called on
/// note deselect so edits are not lost if the process crashes mid-session.
pub fn save_one(note: &mut Note) -> Result<()> {
    let dir = notes_dir()?;
    note.sync();
    let path = dir.join(format!("{}.json", note.data.id));
    let json = serde_json::to_string_pretty(&note.data)?;
    fs::write(path, json)?;
    Ok(())
}

pub fn save_notes(notes: &mut [Note]) -> Result<()> {
    let dir = notes_dir()?;

    // Delete files for notes that no longer exist (e.g. closed with Ctrl+W).
    let active_ids: std::collections::HashSet<u64> = notes.iter().map(|n| n.data.id).collect();
    for entry in fs::read_dir(&dir)?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Some(id) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u64>().ok()) {
                if !active_ids.contains(&id) {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    for note in notes.iter_mut() {
        note.sync();
        note.sync_shell();
        let path = dir.join(format!("{}.json", note.data.id));
        let json = serde_json::to_string_pretty(&note.data)?;
        fs::write(path, json)?;
    }
    Ok(())
}

/// Scan the notes directory and return `NoteData` for every file whose ID is
/// not in `active_ids` (currently loaded notes) or `trash_ids` (already trashed).
/// Does not delete or move anything — pure read.
pub fn find_orphan_notes(
    active_ids: &std::collections::HashSet<u64>,
    trash_ids: &std::collections::HashSet<u64>,
) -> Vec<NoteData> {
    let dir = match notes_dir() {
        Ok(d) => d,
        Err(_) => return vec![],
    };
    let mut orphans = Vec::new();
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") { continue; }
            let id = match path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
            {
                Some(id) => id,
                None => continue,
            };
            if active_ids.contains(&id) || trash_ids.contains(&id) { continue; }
            if let Ok(json) = fs::read_to_string(&path) {
                if let Ok(data) = serde_json::from_str::<NoteData>(&json) {
                    orphans.push(data);
                }
            }
        }
    }
    orphans
}

/// Delete the persisted JSON file for a single note ID.
/// Used by the trash system to immediately remove a note from the active notes dir.
pub fn delete_note_file(id: u64) {
    if let Ok(dir) = notes_dir() {
        let _ = fs::remove_file(dir.join(format!("{}.json", id)));
    }
}

pub fn load_notes(scrollback: usize) -> Result<Vec<Note>> {
    let dir = notes_dir()?;
    let mut notes = Vec::new();

    for entry in fs::read_dir(dir)?.flatten() {
        let path = entry.path();
        // Only process numerically-named files (e.g. "42.json").
        // Other JSON files in the same directory (e.g. workspaces.json) are skipped.
        let is_note_file = path.extension().and_then(|e| e.to_str()) == Some("json")
            && path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.parse::<u64>().ok()).is_some();
        if is_note_file {
            let json = fs::read_to_string(&path)?;
            let data: NoteData = serde_json::from_str(&json)?;
            notes.push(Note::from_data(data, scrollback)?);
        }
    }

    // Stable ordering by id
    notes.sort_by_key(|n| n.data.id);
    Ok(notes)
}
