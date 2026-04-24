use crate::{
    colors::{BG_PALETTE, BORDER_PALETTE},
    trash::{self, TrashedNote},
    config::Config,
    constants::{BG_SHELL_INSET, CARD_H, CARD_W, PROMPT_LINES},
    note::{self, Note, NoteKind, PhotoCell, PhotoRow, SerColor},
    notebook::{self, NotebookData},
    pty::PtySession,
    terminal::{CapturedRow, capture_region, capture_screen_before_resize, capture_scrollback_rows, map_color},
    ui,
    workspace,
};
use anyhow::Result;
use ratatui::style::Color;
use crossterm::event::{self, Event};
use ratatui::{Terminal, backend::Backend};
use std::{collections::{HashMap, VecDeque}, time::{Duration, Instant}};

/// How the occlusion-dim effect treats text colour on dimmed (behind) notes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum OcclusionDim {
    /// Effect is disabled.
    Off,
    /// Saturated background; text colour chosen automatically for contrast.
    On,
    /// Saturated background; text colour forced to black regardless of background.
    BlackText,
}

/// Discriminant carried by `Focus::Note` — describes which kind of note is focused
/// without holding any runtime data. Extend this when new note types are added.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NoteType {
    Text,
    Shell,
    Photo,
    CheckList,
}

/// An item in the corkboard grid — either a regular note or a notebook folder.
#[derive(Clone, Copy)]
pub enum CorkItem {
    /// Index into `App::notes`.
    Note(usize),
    /// Notebook ID.
    Notebook(u64),
    /// The recycle-bin card (always last).
    Trash,
}

/// What the notebook-picker overlay is doing.
pub enum NotebookPickerMode {
    /// Choosing a notebook to assign a note (at `App::notes[note_idx]`) to.
    AssignToNotebook(usize),
    /// Choosing a free corkboard note to add as a page to this notebook.
    AddToNotebook(u64),
}

/// Active notebook-picker overlay state.
pub struct NotebookPicker {
    pub selected: usize,
    pub mode: NotebookPickerMode,
}

/// Which section of the settings popup has keyboard focus.
pub enum SettingsSection {
    BorderToggle,
    Border,
    Background,
    /// Text-wrap toggle — only reachable for Text notes.
    TextWrap,
}

/// What currently has keyboard focus.
pub enum Focus {
    Shell,
    /// A background shell note for the active workspace has focus.
    /// The `usize` is the index into `App::notes`.
    BackgroundShell(usize),
    /// A note at this Vec index has focus; `NoteType` says which kind.
    Note(usize, NoteType),
    /// Editing the title of the note at this index; String is the live input.
    Renaming(usize, String),
    /// Colour settings popup for the note at this index.
    Settings(usize, SettingsSection),
    /// Editing the name of the active workspace; String is the live input.
    RenamingWorkspace(String),
    /// Typing the title for a new notebook (id, live input buffer).
    NamingNotebook(u64, String),
    /// Keyboard visual-block selection mode (vim Ctrl+V style).
    /// `anchor` is the fixed corner, `cursor` is the moving corner.
    Selecting {
        anchor_col: u16, anchor_row: u16,
        cursor_col: u16, cursor_row: u16,
    },
    /// Keyboard visual-selection mode inside a text note (entered via Ctrl+V).
    /// `anchor_row/col` mark where selection started (buffer coordinates).
    /// The moving end tracks the textarea's cursor position.
    TextVisual {
        note_idx: usize,
        anchor_row: usize,
        anchor_col: usize,
    },
}

/// Active mouse dragging
pub enum DragMode {
    Moving {
        note_idx: usize,
        offset_x: u16,
        offset_y: u16,
    },
    Resizing {
        note_idx: usize,
        anchor_col: u16,
        anchor_row: u16,
        orig_w: u16,
        orig_h: u16,
        /// True once the initial capture+clear has been done for this drag.
        shell_cleared: bool,
    },
    /// Mouse-drag screenshot selection on the background terminal.
    Selecting {
        start_col: u16,
        start_row: u16,
        cur_col: u16,
        cur_row: u16,
    },
    /// Text-selection drag inside a focused shell note.
    /// Coordinates are screen-absolute (raw event col/row).
    ShellSelecting {
        note_idx: usize,
        start_col: u16,
        start_row: u16,
        cur_col: u16,
        cur_row: u16,
    },
    /// Mouse events are being forwarded to a shell note's PTY (TUI passthrough).
    /// Active while the left button is held down on the note's content area when
    /// a TUI application (alternate screen) is running inside the note.
    ShellPassthrough {
        note_idx: usize,
    },
    /// Text-selection drag inside a focused text note (Alt+click&drag).
    /// Coordinates are content-relative: 0-based inside the note's inner area.
    TextSelecting {
        note_idx: usize,
        start_col: u16,
        start_row: u16,
        cur_col: u16,
        cur_row: u16,
    },
}

pub struct App {
    /// User key-binding configuration loaded from `~/.bobrc`.
    pub config: Config,
    pub pty: PtySession,
    /// vt100 parser — maintains the full virtual terminal grid from PTY output.
    pub parser: vt100::Parser,
    pub notes: Vec<Note>,
    pub focus: Focus,
    pub drag: Option<DragMode>,
    pub running: bool,
    next_id: u64,
    /// Whether the corkboard overlay is currently open.
    pub corkboard_open: bool,
    /// Index of the highlighted card in the corkboard grid.
    pub corkboard_selected: usize,
    /// When `Some(idx)`, a shell note is expanded full-screen inside the corkboard.
    /// `idx` is the index into `App::notes`.
    pub corkboard_expanded: Option<usize>,
    /// Foreground application currently running in the main background terminal (None = shell).
    pub active_app: Option<String>,
    /// Background colour sampled from the main terminal while an app is active.
    /// Cleared when the foreground process returns to the shell.
    pub detected_bg: Option<Color>,
    /// Scroll position for the main background terminal (i64, matches shell-note semantics):
    ///   > 0  →  scrolled up into history
    ///   = 0  →  live view (prompt at bottom)
    ///   < 0  →  scrolled down past live view (blank rows below prompt, Alacritty-style)
    pub scroll_offset: i64,
    /// Show the splash screen until the first keypress.
    pub splash: bool,
    /// Whether the hint bar at the bottom is visible (toggled with Alt+H).
    pub show_hints: bool,
    /// Whether drop-shadows are drawn on overlapping notes (toggled with Ctrl+D).
    pub show_shadows: bool,
    /// Occlusion-dim mode, cycled with Ctrl+O.
    pub occlusion_dim: OcclusionDim,
    /// Captured scrollback lines for the main background terminal.
    /// Holds rows that have scrolled past vt100's accessible window (capped to
    /// the parser's visible row count by the vt100 API).
    pub own_scrollback: VecDeque<CapturedRow>,
    /// Fingerprints of the previously accessible vt100 scrollback rows for the
    /// main terminal — used by `capture_scrollback_rows` each frame.
    pub sb_prev_fps: Vec<u64>,
    /// Cached terminal dimensions — refreshed only on resize events to avoid
    /// a syscall every frame.
    pub term_size: (u16, u16),
    /// Persistent clipboard handle — kept alive so clipboard managers have time
    /// to observe the contents before the handle is dropped.
    pub(crate) clipboard: Option<arboard::Clipboard>,
    /// Last text-selection drag on the main terminal, stored in text order:
    /// `(start_col, start_row, end_col, end_row)` where (start_row, start_col)
    /// ≤ (end_row, end_col).  Displayed until the user clicks again.
    pub text_selection: Option<(u16, u16, u16, u16)>,
    /// Persistent text selection inside a shell note: `(note_idx, sc, sr, ec, er)`
    /// in screen-absolute coordinates, text order.  Cleared on the next click.
    pub shell_note_selection: Option<(usize, u16, u16, u16, u16)>,
    /// Persistent text selection inside a text note: `(note_idx, sc, sr, ec, er)`
    /// in content-relative coordinates (0-based inside inner area), text order.
    /// Cleared on the next left-click.
    pub text_note_selection: Option<(usize, u16, u16, u16, u16)>,

    // ── Workspace state ─────────────────────────────────────────────────────
    /// Currently visible workspace (0-based index).
    pub active_workspace: u8,
    /// Total number of workspaces.
    pub workspace_count: u8,
    /// Display names for each workspace.
    pub workspace_names: Vec<String>,

    // ── Notebook state ──────────────────────────────────────────────────────
    /// All notebooks defined by the user.
    pub notebooks: Vec<NotebookData>,
    /// Next ID to assign when a new notebook is created.
    pub next_notebook_id: u64,
    /// When `Some(nb_id)`, the corkboard is showing that notebook's page sub-grid
    /// instead of the top-level mixed grid.
    pub corkboard_notebook: Option<u64>,
    /// Selection index within the notebook page sub-grid.
    pub corkboard_nb_selected: usize,
    /// Notebooks currently open in "book mode": maps notebook_id → current page_idx.
    /// Each entry causes that notebook's current page to be rendered in the main view
    /// with a spine decoration; Tab/Shift+Tab cycle through pages of the focused notebook.
    pub notebooks_open: HashMap<u64, usize>,
    /// Active notebook-picker overlay (assign / add-page flows).
    pub notebook_picker: Option<NotebookPicker>,

    // ── Recycle bin ─────────────────────────────────────────────────────────
    /// Notes that have been trashed (loaded from ~/.local/share/bobnote/trash/).
    pub trash: Vec<TrashedNote>,
    /// Whether the trash sub-grid is open inside the corkboard.
    pub corkboard_trash_open: bool,
    /// Currently selected card inside the trash sub-grid.
    pub corkboard_trash_selected: usize,
    /// Monotonically increasing frame counter — used to throttle expensive
    /// per-note checks (e.g. `/proc` reads for foreground-app detection) so
    /// they run every N frames instead of every frame.
    frame_count: u64,
}

// ---------------------------------------------------------------------------
// Per-frame helpers: foreground process detection + vt100 colour sampling
// ---------------------------------------------------------------------------

/// Returns `true` when the byte slice contains a "clear screen" escape sequence
/// that the `clear` or `reset` commands would produce on a plain (non-alternate)
/// screen.  The check is skipped when the same chunk also contains an
/// "enter alternate screen" marker (`\x1b[?1049h` / `\x1b[?47h`) so that TUI
/// apps launching (nvim, htop, …) do not falsely wipe the scrollback buffer.
fn is_clear_screen_output(bytes: &[u8]) -> bool {
    let enters_alt = bytes.windows(8).any(|w| w == b"\x1b[?1049h")
        || bytes.windows(6).any(|w| w == b"\x1b[?47h");
    if enters_alt { return false; }
    // \x1b[2J — Erase Display   (`clear`)
    // \x1b[3J — Erase Scrollback (sent by some `clear` / `reset` builds)
    bytes.windows(4).any(|w| w == b"\x1b[2J" || w == b"\x1b[3J")
}

/// Return the name of the foreground application running inside the shell with
/// the given PID, or `None` if the shell itself is the foreground process.
///
/// Reads `/proc/<pid>/task/<pid>/children` to find direct child processes,
/// skips zombies, and returns the `comm` name of the last live child.
fn foreground_app(shell_pid: u32) -> Option<String> {
    let children = std::fs::read_to_string(
        format!("/proc/{shell_pid}/task/{shell_pid}/children"),
    )
    .ok()?;

    // Find the last alive (non-zombie) child process.
    let alive_pid = children
        .split_whitespace()
        .filter_map(|s| s.parse::<u32>().ok())
        .filter(|&pid| {
            // A zombie has State: Z in its status file.
            std::fs::read_to_string(format!("/proc/{pid}/status"))
                .map(|s| !s.lines().any(|l| l.starts_with("State:") && l.contains('Z')))
                .unwrap_or(false)
        })
        .last()?;

    let comm = std::fs::read_to_string(format!("/proc/{alive_pid}/comm")).ok()?;
    let name = comm.trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

/// Sample the most prominent background colour from the vt100 screen.
///
/// Checks the cell at the screen origin (0, 0): full-screen applications
/// (nvim, htop, lazygit, …) always paint this cell in their theme colour.
/// Returns `None` when the background is the terminal default, which is the
/// case for a plain shell prompt.
fn sample_bg_color(parser: &vt100::Parser) -> Option<Color> {
    let bg = parser.screen().cell(0, 0)?.bgcolor();
    match bg {
        vt100::Color::Default => None,
        other => Some(map_color(other)),
    }
}

impl App {
    pub fn new() -> Result<Self> {
        let (cols, rows) = crossterm::terminal::size()?;
        // Load config first so shell_scrollback is available for note/parser creation.
        let config = Config::load();

        // The main PTY is inset by BG_SHELL_INSET rows on each side so the
        // workspace tab bar (top) and hint bar (bottom) are never obscured.
        let shell_rows = rows.saturating_sub(2 * BG_SHELL_INSET);

        let pty = PtySession::new(shell_rows, cols, None)?;
        // 10 000-line scrollback for the main terminal.
        let parser = vt100::Parser::new(shell_rows, cols, 10_000);

        // Load any previously saved notes and notebooks from disk.
        let notes = note::load_notes(config.shell_scrollback).unwrap_or_default();
        let notebooks = notebook::load_notebooks().unwrap_or_default();
        let trash = trash::load_trash();
        // If a crash left a note in both the active notes dir and the trash dir,
        // trust the trash version and drop it from the active list.
        let trash_ids: std::collections::HashSet<u64> =
            trash.iter().map(|t| t.data.id).collect();
        let notes: Vec<_> = notes.into_iter().filter(|n| !trash_ids.contains(&n.data.id)).collect();
        // Re-derive next_id after filtering.
        let next_id = notes.iter().map(|n| n.data.id + 1).max()
            .unwrap_or(1)
            .max(trash.iter().map(|t| t.data.id + 1).max().unwrap_or(1));
        let next_notebook_id = notebooks.iter().map(|nb| nb.id + 1).max().unwrap_or(1);

        // Restore book-mode session from the last run (if any).
        // Validate that each notebook and page still exist before trusting the session.
        let (notebooks_open, initial_focus) = {
            let mut map: HashMap<u64, usize> = HashMap::new();
            let mut focus = Focus::Shell;
            for (nb_id, page_idx) in notebook::load_session() {
                let valid = notebooks.iter()
                    .find(|nb| nb.id == nb_id)
                    .and_then(|nb| nb.note_ids.get(page_idx))
                    .and_then(|&note_id| notes.iter().position(|n| n.data.id == note_id));
                if let Some(note_idx) = valid {
                    map.insert(nb_id, page_idx);
                    // Focus the last restored notebook's page (arbitrary but consistent).
                    let note_type = if notes[note_idx].data.is_shell {
                        NoteType::Shell
                    } else if notes[note_idx].data.is_photo {
                        NoteType::Photo
                    } else if notes[note_idx].data.is_checklist {
                        NoteType::CheckList
                    } else {
                        NoteType::Text
                    };
                    focus = Focus::Note(note_idx, note_type);
                }
            }
            (map, focus)
        };

        let (workspace_count, workspace_names) = workspace::load_workspaces();

        // Determine initial focus: if there's a background note for workspace 0
        // and no notebook session restored a note focus, switch to BackgroundShell.
        let initial_focus = {
            let bg_idx = notes.iter().position(|n| {
                n.data.is_shell && n.data.is_background && n.data.workspace_id == 0
            });
            match (initial_focus, bg_idx) {
                (Focus::Shell, Some(idx)) => Focus::BackgroundShell(idx),
                (f, _) => f,
            }
        };

        Ok(Self {
            config,
            pty,
            parser,
            notes,
            focus: initial_focus,
            drag: None,
            running: true,
            next_id,
            corkboard_open: false,
            corkboard_selected: 0,
            corkboard_expanded: None,
            active_app: None,
            detected_bg: None,
            scroll_offset: 0,
            splash: true,
            show_hints: true,
            show_shadows: false,
            occlusion_dim: OcclusionDim::BlackText,
            own_scrollback: VecDeque::new(),
            sb_prev_fps: Vec::new(),
            term_size: (cols, rows),
            clipboard: arboard::Clipboard::new().ok(),
            text_selection: None,
            shell_note_selection: None,
            text_note_selection: None,
            active_workspace: 0,
            workspace_count,
            workspace_names,
            notebooks,
            next_notebook_id,
            corkboard_notebook: None,
            corkboard_nb_selected: 0,
            notebooks_open,
            notebook_picker: None,
            trash,
            corkboard_trash_open: false,
            corkboard_trash_selected: 0,
            frame_count: 0,
        })
    }

    pub fn run<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        // Target ~60 fps; crossterm events are polled with a short timeout so
        // PTY output is drained every frame even when the user isn't typing.
        let frame_budget = Duration::from_millis(16);

        while self.running {
            let frame_start = Instant::now();
            let mut dirty = false;
            self.frame_count = self.frame_count.wrapping_add(1);

            // Drain main PTY output into the parser, capturing scrollback after
            // every small chunk.  vt100's scrollback peek is capped to `shell_rows`
            // lines per call, so if we drain all pending bytes in one go and then
            // capture, any burst longer than ~shell_rows lines loses the oldest rows.
            // Processing in ≤512-byte chunks (~6 lines) keeps each capture window
            // well within the vt100 limit.
            // The main PTY is inset; derive its actual row count from the vt100 parser
            // (set at startup and updated by handle_resize, not from term_size directly).
            let shell_rows = self.parser.screen().size().0;
            let mut main_new_lines: i64 = 0;
            while let Ok(bytes) = self.pty.output.try_recv() {
                dirty = true;
                for chunk in bytes.chunks(512) {
                    // When `clear` (or `reset`) is run, wipe the scrollback buffer so
                    // the user cannot scroll back into history that has been cleared.
                    // The alternate-screen guard inside is_clear_screen_output prevents
                    // this from firing when a TUI app (nvim, htop …) is starting up.
                    if !self.parser.screen().alternate_screen()
                        && is_clear_screen_output(chunk)
                    {
                        self.own_scrollback.clear();
                        self.sb_prev_fps.clear();
                        self.scroll_offset = 0;
                    }
                    self.parser.process(chunk);
                    let (_, n) = capture_scrollback_rows(
                        &mut self.parser,
                        &mut self.own_scrollback,
                        &mut self.sb_prev_fps,
                        shell_rows,
                        10_000,
                    );
                    main_new_lines += n as i64;
                }
            }

            // Final capture: idempotent, returns current depth for scroll-cap below.
            let (vt100_depth_main, final_n) = capture_scrollback_rows(
                &mut self.parser,
                &mut self.own_scrollback,
                &mut self.sb_prev_fps,
                shell_rows,
                10_000,
            );
            main_new_lines += final_n as i64;

            // Alacritty-style fill: advance negative scroll_offset as output scrolls in,
            // so output appears to flow into the blank space below the prompt.
            if self.scroll_offset < 0 && main_new_lines > 0 {
                self.scroll_offset = (self.scroll_offset + main_new_lines).min(0);
            }

            // Apply scroll offset to the main background terminal.
            // vt100 overflow guard: visible_rows() does `rows_len - scrollback_offset`
            // as usize — panics when offset > rows_len.  Cap what we hand to vt100;
            // deeper history is served by own_scrollback in the renderer.
            let term_rows_i = shell_rows as i64;
            let main_vt100_off = self.scroll_offset.clamp(0, term_rows_i) as usize;
            self.parser.set_scrollback(main_vt100_off);
            let main_actual = self.parser.screen().scrollback() as i64;

            // own_scrollback and vt100 scrollback overlap: rows are captured into
            // own_scrollback as they enter vt100's window and stay there for up to
            // max_safe frames.  The reachable depth is therefore max(L, D), not L+D.
            let main_max = (self.own_scrollback.len() as i64).max(vt100_depth_main as i64);
            if self.scroll_offset > main_max {
                self.scroll_offset = main_max;
            } else if self.scroll_offset <= term_rows_i && self.own_scrollback.is_empty()
                && main_actual < self.scroll_offset
            {
                // In vt100 territory with no own_scrollback: sync down if buffer isn't full.
                self.scroll_offset = main_actual;
            }

            // Drain shell-note PTYs; resize their parser if the note was resized.
            // During a resize drag we update the parser size for correct rendering but
            // defer pty.resize() (SIGWINCH) until the drag ends — sending it every frame
            // causes the shell to redraw its prompt over the scrollback history.
            let drag_resize_note = match &self.drag {
                Some(DragMode::Resizing { note_idx, .. }) => Some(*note_idx),
                _ => None,
            };

            // First-frame drag-clear: the very first time the note's dimensions
            // change during a drag, capture everything visible into own_scrollback
            // and reset the vt100 parser to a blank screen.  This is done once per
            // drag so that (a) no history is lost, and (b) the shell can redraw a
            // clean prompt into the fresh parser when SIGWINCH fires at drag-end.
            // We use two sequential borrows to avoid holding &mut self.drag and
            // &mut self.notes at the same time.
            let first_clear_idx: Option<usize> = {
                if let Some(DragMode::Resizing { note_idx, shell_cleared, .. }) = &self.drag {
                    let idx = *note_idx;
                    if !shell_cleared {
                        if let NoteKind::Shell { rows, cols, .. } = &self.notes[idx].kind {
                            let new_cols = self.notes[idx].data.width.saturating_sub(2).max(2);
                            let new_rows = self.notes[idx].data.height.saturating_sub(2).max(2);
                            if new_cols != *cols || new_rows != *rows { Some(idx) } else { None }
                        } else { None }
                    } else { None }
                } else { None }
            };
            if let Some(idx) = first_clear_idx {
                if let NoteKind::Shell { parser, rows, cols, own_scrollback, sb_prev_fps, .. } =
                    &mut self.notes[idx].kind
                {
                    capture_screen_before_resize(parser, own_scrollback, sb_prev_fps, self.config.shell_scrollback);
                    // Replace the parser with a blank one at the same size — the shell
                    // will draw a fresh prompt into it when SIGWINCH arrives at drag-end.
                    *parser = vt100::Parser::new(*rows, *cols, self.config.shell_scrollback);
                }
                if let Some(DragMode::Resizing { shell_cleared, .. }) = &mut self.drag {
                    *shell_cleared = true;
                }
            }

            // Sync background shell notes to fill the inset area each frame.
            // BG_SHELL_INSET rows are reserved at the top (workspace tab bar) and
            // bottom (hint bar) so those overlays are never obscured by the PTY.
            {
                let (tc, tr) = self.term_size;
                let bg_h = tr.saturating_sub(2 * BG_SHELL_INSET);
                for note in self.notes.iter_mut() {
                    if note.data.is_background {
                        note.data.x = 0;
                        note.data.y = BG_SHELL_INSET;
                        note.data.width = tc;
                        note.data.height = bg_h;
                    }
                }
            }

            for (note_vec_idx, note) in self.notes.iter_mut().enumerate() {
                if let NoteKind::Shell {
                    pty, parser, rows, cols, scroll_offset, own_scrollback, sb_prev_fps,
                    startup_clear_pending, ..
                } = &mut note.kind
                {
                    // Background notes have no border — PTY size = full note dimensions.
                    let (new_cols, new_rows) = if note.data.is_background {
                        (note.data.width.max(2), note.data.height.max(2))
                    } else {
                        (note.data.width.saturating_sub(2).max(2), note.data.height.saturating_sub(2).max(2))
                    };
                    if new_cols != *cols || new_rows != *rows {
                        // Defer BOTH parser.set_size AND pty.resize during a resize drag.
                        // Calling either every frame floods the shell with SIGWINCH redraws
                        // that corrupt scrollback history.  We apply one clean resize at
                        // drag-end in MouseEventKind::Up instead.
                        if drag_resize_note != Some(note_vec_idx) {
                            // Non-drag resize: capture, clear, then immediately apply.
                            capture_screen_before_resize(parser, own_scrollback, sb_prev_fps, self.config.shell_scrollback);
                            *parser = vt100::Parser::new(new_rows, new_cols, self.config.shell_scrollback);
                            let _ = pty.resize(new_rows, new_cols);
                            *cols = new_cols;
                            *rows = new_rows;
                            *scroll_offset = 0;
                        }
                        // During drag: dimensions have changed but we leave the (now blank)
                        // parser at its pre-drag size; out-of-bounds cells return None.
                    }
                    // Count vt100 scroll events so we can advance a negative
                    // scroll_offset — this makes output flow naturally into the
                    // blank space below the prompt (Alacritty-style) instead of
                    // the blank gap staying static while content scrolls above.
                    let mut new_lines: i64 = 0;
                    let mut note_received_output = false;
                    while let Ok(bytes) = pty.output.try_recv() {
                        dirty = true;
                        note_received_output = true;
                        for chunk in bytes.chunks(512) {
                            if !parser.screen().alternate_screen()
                                && is_clear_screen_output(chunk)
                            {
                                own_scrollback.clear();
                                sb_prev_fps.clear();
                                *scroll_offset = 0;
                            }
                            parser.process(chunk);
                            let (_, n) = capture_scrollback_rows(
                                parser,
                                own_scrollback,
                                sb_prev_fps,
                                *rows,
                                self.config.shell_scrollback,
                            );
                            new_lines += n as i64;
                        }
                    }

                    // For restored shell notes: wipe the visible screen every frame that
                    // output is flowing (hiding startup noise, cd echo, motd, prompt draws).
                    // Once the shell goes quiet (no output this frame) it has settled at the
                    // prompt — stop clearing so the user sees a clean, live prompt.
                    // ESC[2J ESC[H is injected directly into the parser: no shell command is
                    // issued, so shell history is unaffected and own_scrollback is preserved.
                    if *startup_clear_pending {
                        if note_received_output {
                            parser.process(b"\x1b[2J\x1b[H");
                        } else {
                            *startup_clear_pending = false;
                        }
                    }

                    // Final capture: idempotent here, returns current depth for scroll cap.
                    let (vt100_depth, final_n) = capture_scrollback_rows(
                        parser,
                        own_scrollback,
                        sb_prev_fps,
                        *rows,
                        self.config.shell_scrollback,
                    );
                    new_lines += final_n as i64;

                    // Advance negative scroll_offset by the number of lines that
                    // just scrolled into vt100's history.  Each such line "fills"
                    // one of the blank rows below the prompt, so the view tracks
                    // the output rather than drifting further away from it.
                    if *scroll_offset < 0 && new_lines > 0 {
                        *scroll_offset = (*scroll_offset + new_lines).min(0);
                    }

                    // scroll_offset semantics (i64):
                    //   > 0  →  scrolled up into history (vt100 ≤ rows, own_scrollback beyond)
                    //   = 0  →  live view (prompt at bottom)
                    //   < 0  →  scrolled down below live view (prompt moves toward top)
                    //
                    // Dynamic cap: whichever is larger — vt100's accessible depth or the
                    // lines we've captured ourselves.
                    let rows_i = *rows as i64;
                    // own_scrollback overlaps with vt100's window (rows are captured as
                    // they enter vt100, before they leave it).  Use max, not sum.
                    let max_scroll = (own_scrollback.len() as i64).max(vt100_depth as i64);
                    *scroll_offset = (*scroll_offset).clamp(-(rows_i - PROMPT_LINES), max_scroll);

                    // vt100 overflow guard: visible_rows() panics when offset > rows_len.
                    // Cap what we pass to vt100; deeper history renders from own_scrollback.
                    let vt100_off = (*scroll_offset).clamp(0, rows_i) as usize;
                    parser.set_scrollback(vt100_off);
                    // Sync down from vt100 only while in vt100 territory AND
                    // own_scrollback has no content to fall back on. When
                    // own_scrollback is populated (e.g. after a parser reset)
                    // we must not clamp scroll_offset to vt100's depth of 0 —
                    // that would prevent the user from scrolling into history.
                    if *scroll_offset <= rows_i && own_scrollback.is_empty() {
                        let actual = parser.screen().scrollback() as i64;
                        if actual < *scroll_offset {
                            *scroll_offset = actual;
                        }
                    }
                }
            }

            // Foreground-app detection + background-colour sampling.
            // Runs after the output drain so the vt100 screen reflects the
            // latest frame before we sample it.
            //
            // `/proc` reads are expensive at 60 fps when many shell notes are open.
            // Two optimisations are applied:
            //   1. Throttle: only poll every 10 frames (~6 Hz) — processes don't
            //      start or exit faster than that in practice.
            //   2. Skip invisible notes: notes on other workspaces or hidden on the
            //      corkboard are never rendered, so knowing their foreground app
            //      has no user-visible effect.
            // The alternate-screen snap is cheap (reads a bool) and must still
            // happen every frame to avoid showing the wrong buffer on switch.
            let check_fg = self.frame_count % 10 == 0;
            let active_ws = self.active_workspace;
            for note in self.notes.iter_mut() {
                if let NoteKind::Shell { pty, parser, active_app, detected_bg, scroll_offset, .. } =
                    &mut note.kind
                {
                    // Only run the expensive /proc check when throttle allows AND
                    // the note is visible to the user on the current workspace.
                    let note_visible = note.data.workspace_id == active_ws
                        && !note.data.on_corkboard;
                    if check_fg && note_visible {
                        if let Some(pid) = pty.shell_pid {
                            let new_app = foreground_app(pid);
                            if new_app != *active_app {
                                // App exited → clear the detected colour so the
                                // border reverts to the user's chosen palette colour.
                                if new_app.is_none() {
                                    *detected_bg = None;
                                }
                                *active_app = new_app;
                            }
                            // While an app is active, keep the sampled colour fresh.
                            // sample_bg_color returns None for plain terminal backgrounds,
                            // so we only overwrite when we get a real colour reading.
                            if active_app.is_some() {
                                if let Some(color) = sample_bg_color(parser) {
                                    *detected_bg = Some(color);
                                }
                            }
                        }
                    }
                    // Snap to the live view whenever a TUI app owns the alternate
                    // screen — any residual scroll offset would show the wrong buffer.
                    if parser.screen().alternate_screen() {
                        *scroll_offset = 0;
                    }
                }
            }

            // Same detection for the main background terminal.
            if check_fg {
                if let Some(pid) = self.pty.shell_pid {
                    let new_app = foreground_app(pid);
                    if new_app != self.active_app {
                        if new_app.is_none() {
                            self.detected_bg = None;
                        }
                        self.active_app = new_app;
                    }
                    if self.active_app.is_some() {
                        if let Some(color) = sample_bg_color(&self.parser) {
                            self.detected_bg = Some(color);
                        }
                    }
                }
            }
            // Snap to the live view whenever a TUI app owns the alternate screen.
            if self.parser.screen().alternate_screen() {
                self.scroll_offset = 0;
            }

            // Poll for input events before drawing so the same frame that receives
            // an event also renders the updated state (lower latency).
            let elapsed = frame_start.elapsed();
            let remaining = frame_budget.saturating_sub(elapsed);
            if event::poll(remaining)? {
                // Snapshot which note (if any) currently has focus so we can
                // detect when the user navigates away and save it immediately.
                let focused_before: Option<usize> = match &self.focus {
                    Focus::Note(i, _) | Focus::Renaming(i, _) | Focus::Settings(i, _) => Some(*i),
                    _ => None,
                };

                match event::read()? {
                    Event::Key(key) => { self.handle_key(key)?; dirty = true; }
                    Event::Mouse(mouse) => { self.handle_mouse(mouse)?; dirty = true; }
                    Event::Resize(cols, rows) => {
                        self.term_size = (cols, rows);
                        self.handle_resize(cols, rows)?;
                        dirty = true;
                    }
                    // Bracketed-paste event from the outer terminal (e.g. kitty, gnome-terminal).
                    // Route the raw text to whichever surface currently has focus.
                    Event::Paste(text) => { self.handle_paste_event(text)?; dirty = true; }
                    _ => {}
                }

                // If a note had focus before the event but no longer does, save it now.
                if let Some(idx) = focused_before {
                    let still_focused = matches!(&self.focus,
                        Focus::Note(i, _) | Focus::Renaming(i, _) | Focus::Settings(i, _)
                        if *i == idx
                    );
                    if !still_focused {
                        if let Some(note) = self.notes.get_mut(idx) {
                            let _ = note::save_one(note);
                        }
                    }
                }
            }

            // Only rerender when PTY output arrived, a user event was handled, or the
            // splash screen is still up (needs at least one draw before first keypress).
            if dirty || self.splash {
                terminal.draw(|f| ui::render(f, self))?;
            }
        }

        // Rescue any note files on disk that are not tracked in App::notes and
        // not already in the trash (e.g. notes dropped by a bug, or a discrepancy
        // between memory and disk).  Move them to the recycle bin rather than
        // silently deleting them — save_notes will then clean up the stale files.
        {
            let active_ids: std::collections::HashSet<u64> =
                self.notes.iter().map(|n| n.data.id).collect();
            let trash_ids: std::collections::HashSet<u64> =
                self.trash.iter().map(|t| t.data.id).collect();
            let now = trash::now_secs();
            for data in note::find_orphan_notes(&active_ids, &trash_ids) {
                let trashed = TrashedNote { deleted_at: now, data };
                let _ = trash::save_trash_note(&trashed);
                self.trash.push(trashed);
            }
        }

        note::save_notes(&mut self.notes)?;
        notebook::save_notebooks(&self.notebooks)?;
        notebook::save_session(&self.notebooks_open)?;
        workspace::save_workspaces(self.workspace_count, &self.workspace_names)?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Helpers (pub(crate) so input.rs and corkboard.rs can call them)
    // -----------------------------------------------------------------------

    /// Return the correct `Focus::Note` for the note at `idx`.
    pub(crate) fn note_focus(&self, idx: usize) -> Focus {
        let note_type = if self.notes[idx].data.is_shell {
            NoteType::Shell
        } else if self.notes[idx].data.is_photo {
            NoteType::Photo
        } else if self.notes[idx].data.is_checklist {
            NoteType::CheckList
        } else {
            NoteType::Text
        };
        Focus::Note(idx, note_type)
    }

    /// Create a new centred note and return its index.
    pub(crate) fn new_note(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let (cols, rows) = self.term_size;
        let x = (cols - CARD_W) / 2;
        let y = (rows - CARD_H) / 2;
        self.notes.push(Note::new(id, x, y, CARD_W, CARD_H));
        let idx = self.notes.len() - 1;
        self.notes[idx].data.workspace_id = self.active_workspace;
        idx
    }

    /// Create a new Checklist note
    pub(crate) fn new_checklist(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let (cols, rows) = self.term_size;
        let x = (cols - CARD_W) /2;
        let y = (rows - CARD_H) /2;
        self.notes.push(Note::new_checklist(id, x, y, CARD_W, CARD_H));
        let idx = self.notes.len() - 1;
        self.notes[idx].data.workspace_id = self.active_workspace;
        idx
    }

    /// Copy the text content of a rectangular region of the background terminal
    /// to the system clipboard. Coordinates are in terminal cell space (col, row).
    pub(crate) fn copy_selection_to_clipboard(&mut self, col1: u16, row1: u16, col2: u16, row2: u16) {
        let rows = capture_region(self.parser.screen(), col1, row1, col2, row2);
        let text: String = rows
            .iter()
            .map(|row| {
                let line: String = row
                    .iter()
                    .map(|cell| if cell.sym.is_empty() { " " } else { cell.sym.as_str() })
                    .collect();
                line.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        if !text.is_empty() {
            if let Some(ref mut cb) = self.clipboard {
                let _ = cb.set_text(text);
            }
        }
    }

    /// Copy a stream (text-flow) selection to the clipboard.
    /// Coordinates must already be in text order: (sc, sr) ≤ (ec, er).
    ///   - First row : cells sc ..= term_width
    ///   - Middle rows: full lines
    ///   - Last row  : cells 0 ..= ec
    ///   - Single row: cells sc ..= ec
    pub(crate) fn copy_stream_selection(&mut self, sc: u16, sr: u16, ec: u16, er: u16) {
        let term_cols = self.term_size.0 as usize;
        let rows = capture_region(
            self.parser.screen(),
            0, sr,
            (term_cols as u16).saturating_sub(1), er,
        );
        let mut lines: Vec<String> = rows.iter().enumerate().map(|(i, row)| {
            let (from, to) = if sr == er {
                (sc as usize, (ec as usize + 1).min(row.len()))
            } else if i == 0 {
                (sc as usize, row.len())
            } else if i == rows.len() - 1 {
                (0, (ec as usize + 1).min(row.len()))
            } else {
                (0, row.len())
            };
            let s: String = row[from.min(row.len())..to.min(row.len())]
                .iter()
                .map(|cell| if cell.sym.is_empty() { " " } else { cell.sym.as_str() })
                .collect();
            s.trim_end().to_string()
        }).collect();
        // Drop trailing blank lines produced by empty terminal rows.
        while lines.last().map_or(false, |l: &String| l.is_empty()) {
            lines.pop();
        }
        let text = lines.join("\n");
        if !text.is_empty() {
            if let Some(ref mut cb) = self.clipboard {
                let _ = cb.set_text(text);
            }
        }
    }

    /// Copy a stream selection from a shell note's visible content to the clipboard.
    /// `sc/sr/ec/er` are **content-relative** (0-based inside the note's inner area),
    /// already in text order (start_row ≤ end_row).
    pub(crate) fn copy_shell_note_stream_selection(
        &mut self,
        note_idx: usize,
        sc: u16, sr: u16,
        ec: u16, er: u16,
    ) {
        let note_inner_w = self.notes[note_idx].data.width.saturating_sub(2);
        if let NoteKind::Shell { parser, .. } = &self.notes[note_idx].kind {
            let rows = capture_region(
                parser.screen(),
                0, sr,
                note_inner_w.saturating_sub(1), er,
            );
            let mut lines: Vec<String> = rows.iter().enumerate().map(|(i, row)| {
                let (from, to) = if sr == er {
                    (sc as usize, (ec as usize + 1).min(row.len()))
                } else if i == 0 {
                    (sc as usize, row.len())
                } else if i == rows.len() - 1 {
                    (0, (ec as usize + 1).min(row.len()))
                } else {
                    (0, row.len())
                };
                let s: String = row[from.min(row.len())..to.min(row.len())]
                    .iter()
                    .map(|cell| if cell.sym.is_empty() { " " } else { cell.sym.as_str() })
                    .collect();
                s.trim_end().to_string()
            }).collect();
            while lines.last().map_or(false, |l: &String| l.is_empty()) {
                lines.pop();
            }
            let text = lines.join("\n");
            if !text.is_empty() {
                if let Some(ref mut cb) = self.clipboard {
                    let _ = cb.set_text(text);
                }
            }
        }
    }

    /// Copy a text-note stream selection (content-relative coords) to the clipboard.
    /// `(sc, sr, ec, er)` are 0-based inside the note's inner area, in text order.
    /// Handles both wrap and no-wrap modes.
    pub(crate) fn copy_text_note_content_selection(
        &mut self,
        note_idx: usize,
        sc: u16, sr: u16,
        ec: u16, er: u16,
    ) {
        let text_wrap = self.notes[note_idx].data.text_wrap;
        let inner_w = self.notes[note_idx].data.width.saturating_sub(2) as usize;
        if let NoteKind::Text(ta, scroll_top) = &self.notes[note_idx].kind {
            let lines = ta.lines();
            let st = *scroll_top;
            let text = if text_wrap && inner_w > 0 {
                // Build a visual-row index: each entry is (buf_row, char_offset_start).
                let mut vrows: Vec<(usize, usize)> = Vec::new();
                for (br, line) in lines.iter().enumerate().skip(st) {
                    let cc = line.chars().count();
                    if cc == 0 {
                        vrows.push((br, 0));
                    } else {
                        let mut off = 0usize;
                        while off < cc { vrows.push((br, off)); off += inner_w; }
                    }
                }
                let sv = sr as usize;
                let ev = er as usize;
                if sv >= vrows.len() { return; }
                let ev = ev.min(vrows.len().saturating_sub(1));
                let mut result = String::new();
                for vr in sv..=ev {
                    let (br, cs_off) = vrows[vr];
                    let chars: Vec<char> = lines[br].chars().collect();
                    let cs = if vr == sv { sc as usize } else { 0 };
                    let ce = if vr == ev { (ec as usize + 1).min(inner_w) } else { inner_w };
                    let abs_s = (cs_off + cs).min(chars.len());
                    let abs_e = (cs_off + ce).min(chars.len());
                    if vr > sv { result.push('\n'); }
                    result.push_str(chars[abs_s..abs_e].iter().collect::<String>().trim_end());
                }
                result
            } else {
                let sr_b = st + sr as usize;
                let er_b = (st + er as usize).min(lines.len().saturating_sub(1));
                if sr_b >= lines.len() { return; }
                let mut out: Vec<String> = Vec::new();
                for (i, br) in (sr_b..=er_b).enumerate() {
                    let chars: Vec<char> = lines[br].chars().collect();
                    let cs = if i == 0 { (sc as usize).min(chars.len()) } else { 0 };
                    let ce = if br == er_b { (ec as usize + 1).min(chars.len()) } else { chars.len() };
                    out.push(chars[cs..ce].iter().collect::<String>().trim_end().to_string());
                }
                while out.last().map_or(false, |l: &String| l.is_empty()) { out.pop(); }
                out.join("\n")
            };
            if !text.is_empty() {
                if let Some(ref mut cb) = self.clipboard { let _ = cb.set_text(text); }
            }
        }
    }

    /// Copy a visual-mode buffer-coordinate selection from a text note to the clipboard.
    /// `(sr, sc)` / `(er, ec)` are buffer (row, col) pairs, already in text order.
    pub(crate) fn copy_text_note_buffer_selection(
        &mut self,
        note_idx: usize,
        sr: usize, sc: usize,
        er: usize, ec: usize,
    ) {
        if let NoteKind::Text(ta, _) = &self.notes[note_idx].kind {
            let lines = ta.lines();
            if sr >= lines.len() { return; }
            let er = er.min(lines.len().saturating_sub(1));
            let mut out: Vec<String> = Vec::new();
            for (i, br) in (sr..=er).enumerate() {
                let chars: Vec<char> = lines[br].chars().collect();
                let cs = if i == 0 { sc.min(chars.len()) } else { 0 };
                let ce = if br == er { (ec + 1).min(chars.len()) } else { chars.len() };
                out.push(chars[cs..ce].iter().collect::<String>().trim_end().to_string());
            }
            while out.last().map_or(false, |l: &String| l.is_empty()) { out.pop(); }
            let text = out.join("\n");
            if !text.is_empty() {
                if let Some(ref mut cb) = self.clipboard { let _ = cb.set_text(text); }
            }
        }
    }

    /// Capture a rectangular region of the background terminal and create a
    /// Photo note positioned exactly over the captured area.
    pub(crate) fn create_photo_note(&mut self, col1: u16, row1: u16, col2: u16, row2: u16) {
        let c1 = col1.min(col2);
        let c2 = col1.max(col2);
        let r1 = row1.min(row2);
        let r2 = row1.max(row2);
        if c1 == c2 && r1 == r2 { return; } // single cell — ignore

        // Capture the live vt100 screen (scroll_offset was snapped to 0 on entry).
        let rows = capture_region(self.parser.screen(), c1, r1, c2, r2);

        // Also copy the captured region as plain text to the system clipboard so the
        // user can paste the content immediately without opening the photo note.
        let plain_text: String = rows
            .iter()
            .map(|row| {
                let line: String = row
                    .iter()
                    .map(|cell| if cell.sym.is_empty() { " " } else { cell.sym.as_str() })
                    .collect();
                line.trim_end().to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(ref mut cb) = self.clipboard {
            let _ = cb.set_text(plain_text);
        }

        // Place the note so its inner content aligns exactly with the selection.
        let note_x = c1.saturating_sub(1);
        let note_y = r1.saturating_sub(1);
        let note_w = (c2 - c1 + 1) + 2;
        let note_h = (r2 - r1 + 1) + 2;

        let id = self.next_id;
        self.next_id += 1;
        self.notes.push(Note::new_photo(id, note_x, note_y, note_w, note_h, rows));
        let new_idx = self.notes.len() - 1;
        self.notes[new_idx].data.workspace_id = self.active_workspace;
        self.focus = self.note_focus(new_idx);
    }

    /// Move a note to the highest z-order position it is allowed to occupy:
    /// - Pinned notes are never moved (they always stay at the very top).
    /// - Non-pinned notes are moved to just below the first pinned note
    ///   (or to the end if there are no pinned notes).
    /// Returns the new index.
    pub(crate) fn bring_to_front(&mut self, idx: usize) -> usize {
        if self.notes[idx].data.pinned {
            return idx;
        }
        // Target: just before the first pinned note, or end of vec if none.
        let first_pinned = self.notes.iter().position(|n| n.data.pinned);
        let insert_at = first_pinned.unwrap_or(self.notes.len());
        // After removing idx the insertion point may shift left by one.
        let insert_at = if idx < insert_at { insert_at - 1 } else { insert_at };
        if idx == insert_at {
            return idx; // already the topmost non-pinned note
        }
        let note = self.notes.remove(idx);
        self.notes.insert(insert_at, note);
        insert_at
    }

    /// Toggle the "pinned to top" flag on a note and reposition it accordingly:
    /// - Pinning   → move to the end of the vec (rendered last = on top of everything).
    /// - Unpinning → move to just below the remaining pinned notes.
    /// Returns the new index.
    pub(crate) fn toggle_pin(&mut self, idx: usize) -> usize {
        if self.notes[idx].data.pinned {
            self.notes[idx].data.pinned = false;
            // Place at the top of the non-pinned layer (just before remaining pinned notes).
            let first_pinned = self.notes.iter().position(|n| n.data.pinned);
            let insert_at = first_pinned.unwrap_or(self.notes.len());
            let insert_at = if idx < insert_at { insert_at - 1 } else { insert_at };
            if idx == insert_at {
                return idx;
            }
            let note = self.notes.remove(idx);
            self.notes.insert(insert_at, note);
            insert_at
        } else {
            self.notes[idx].data.pinned = true;
            let note = self.notes.remove(idx);
            self.notes.push(note);
            self.notes.len() - 1
        }
    }

    /// Capture a shell note's current visual state (border + content) as a Photo note
    /// placed at the same position. The resulting photo includes synthesised border
    /// characters (matching what ratatui renders) and the vt100 content inside.
    pub(crate) fn snapshot_shell_note(&mut self, note_idx: usize) {
        let (x, y, w, h) = {
            let d = &self.notes[note_idx].data;
            (d.x, d.y, d.width, d.height)
        };
        let inner_w = w.saturating_sub(2) as usize;
        let inner_h = h.saturating_sub(2) as usize;
        if inner_w == 0 || inner_h == 0 { return; }

        let border_color = {
            let d = &self.notes[note_idx].data;
            if let NoteKind::Shell { detected_bg: Some(bg), .. } = &self.notes[note_idx].kind {
                *bg
            } else {
                BORDER_PALETTE[d.border_color_idx].0
            }
        };
        let bg_color = BG_PALETTE[self.notes[note_idx].data.bg_color_idx].0;
        let pin_prefix = if self.notes[note_idx].data.pinned { "▲ " } else { "" };
        let title_text = format!(" {}{} ", pin_prefix, self.notes[note_idx].data.title);

        // Capture inner content from the shell note's vt100 parser at its current
        // scroll position (set_scrollback is applied each frame so this reflects
        // exactly what the user sees).
        let content_rows: Vec<PhotoRow> = if let NoteKind::Shell { parser, scroll_offset, .. } =
            &self.notes[note_idx].kind
        {
            let (rows, cols) = parser.screen().size();
            let cap_w = (inner_w as u16).min(cols).saturating_sub(1);
            let cap_h = (inner_h as u16).min(rows).saturating_sub(1);
            // When scrolled into negative territory, shift down by row_offset.
            let row_offset = (*scroll_offset).min(0).unsigned_abs() as u16;
            capture_region(parser.screen(), 0, row_offset, cap_w, cap_h + row_offset)
        } else {
            return;
        };

        // Helpers for building PhotoCell values.
        let bfg = SerColor::from(border_color);
        let bbg = SerColor::from(bg_color);
        let border_cell = |sym: &str| -> PhotoCell {
            PhotoCell {
                sym: sym.to_string(),
                fg: bfg.clone(), bg: bbg.clone(),
                bold: false, italic: false, underline: false, reversed: false,
            }
        };
        let empty_cell = || -> PhotoCell {
            PhotoCell {
                sym: " ".to_string(),
                fg: SerColor::Default, bg: bbg.clone(),
                bold: false, italic: false, underline: false, reversed: false,
            }
        };

        let mut rows: Vec<PhotoRow> = Vec::with_capacity(h as usize);

        // ── Top border row ──────────────────────────────────────────────────
        {
            let mut row: PhotoRow = Vec::with_capacity(w as usize);
            row.push(border_cell("┌"));
            // Embed the title; fill remainder with horizontal rule.
            let title_chars: Vec<char> = title_text.chars().collect();
            let avail = inner_w.saturating_sub(title_chars.len());
            for ch in &title_chars {
                row.push(PhotoCell {
                    sym: ch.to_string(),
                    fg: bfg.clone(), bg: bbg.clone(),
                    bold: false, italic: false, underline: false, reversed: false,
                });
            }
            for _ in 0..avail { row.push(border_cell("─")); }
            row.push(border_cell("┐"));
            rows.push(row);
        }

        // ── Content rows ────────────────────────────────────────────────────
        for cr in 0..inner_h {
            let mut row: PhotoRow = Vec::with_capacity(w as usize);
            row.push(border_cell("│"));
            let src = content_rows.get(cr);
            for cc in 0..inner_w {
                let cell = src.and_then(|r| r.get(cc)).cloned();
                row.push(cell.unwrap_or_else(empty_cell));
            }
            row.push(border_cell("│"));
            rows.push(row);
        }

        // ── Bottom border row ───────────────────────────────────────────────
        {
            let mut row: PhotoRow = Vec::with_capacity(w as usize);
            row.push(border_cell("└"));
            for _ in 0..inner_w { row.push(border_cell("─")); }
            row.push(border_cell("┘"));
            rows.push(row);
        }

        let id = self.next_id;
        self.next_id += 1;
        self.notes.push(Note::new_photo(id, x, y, w, h, rows));
        let new_idx = self.notes.len() - 1;
        self.notes[new_idx].data.workspace_id = self.active_workspace;
        self.focus = self.note_focus(new_idx);
    }

    /// Create a new centred shell-note and return its index.
    pub(crate) fn new_shell_note(&mut self) -> Result<usize> {
        let id = self.next_id;
        self.next_id += 1;
        let (cols, rows) = self.term_size;
        let w = CARD_W * 2;
        let h = CARD_H * 2;
        let x = (cols - w) / 2;
        let y = (rows - h) / 2;
        self.notes.push(Note::new_shell(id, x, y, w, h, self.config.shell_scrollback)?);
        let idx = self.notes.len() - 1;
        self.notes[idx].data.workspace_id = self.active_workspace;
        Ok(idx)
    }

    /// Return the topmost note index whose bounding rect contains (col, row).
    /// Notes currently on the corkboard are invisible in the main view,
    /// EXCEPT for the currently-open book page which is rendered over the terminal.
    pub(crate) fn note_at(&self, col: u16, row: u16) -> Option<usize> {
        // Collect the IDs of all notes currently shown as book pages.
        // Split into two sets:
        //   all_book_ids     — every open book page (exempts from on_corkboard filter)
        //   persistent_ids   — only pages whose notebook is persistent (exempts from workspace filter)
        let (all_book_ids, persistent_ids): (
            std::collections::HashSet<u64>,
            std::collections::HashSet<u64>,
        ) = {
            let mut all = std::collections::HashSet::new();
            let mut pers = std::collections::HashSet::new();
            for (&nb_id, &page_idx) in &self.notebooks_open {
                if let Some(nb) = self.notebooks.iter().find(|nb| nb.id == nb_id) {
                    if let Some(&note_id) = nb.note_ids.get(page_idx) {
                        all.insert(note_id);
                        if nb.persistent {
                            pers.insert(note_id);
                        }
                    }
                }
            }
            (all, pers)
        };

        self.notes
            .iter()
            .enumerate()
            .rev() // last rendered = topmost
            .find(|(_, n)| {
                // Background notes are rendered behind everything; they're not
                // hit-testable as floating notes (mouse goes through to the bg layer).
                if n.data.is_background { return false; }
                let is_book_page = all_book_ids.contains(&n.data.id);
                let is_workspace_exempt = persistent_ids.contains(&n.data.id);
                // Only hit-test notes on the active workspace.
                // Persistent book pages are an exception — they float across all workspaces.
                if n.data.workspace_id != self.active_workspace && !is_workspace_exempt {
                    return false;
                }
                let visible = !n.data.on_corkboard || is_book_page;
                visible
                    && col >= n.data.x
                    && col < n.data.x + n.data.width
                    && row >= n.data.y
                    && row < n.data.y + n.data.height
            })
            .map(|(i, _)| i)
    }

    /// Clamp note position so it never escapes the terminal boundary.
    pub(crate) fn clamp_note(&mut self, idx: usize) {
        let (cols, rows) = self.term_size;
        let n = &mut self.notes[idx];
        n.data.x = n.data.x.min(cols.saturating_sub(n.data.width));
        n.data.y = n.data.y.min(rows.saturating_sub(n.data.height));
    }

    // ── Notebook helpers ────────────────────────────────────────────────────

    /// Build the ordered list of items shown in the top-level corkboard grid:
    /// regular corkboard notes (no notebook) followed by one folder card per notebook.
    pub(crate) fn corkboard_items(&self) -> Vec<CorkItem> {
        let mut items = Vec::new();
        for nb in &self.notebooks {
            items.push(CorkItem::Notebook(nb.id));
        }
        for (i, n) in self.notes.iter().enumerate() {
            if n.data.on_corkboard && n.data.notebook_id.is_none() {
                items.push(CorkItem::Note(i));
            }
        }
        items.push(CorkItem::Trash);
        items
    }

    /// Find the `App::notes` index for the note at `page_idx` in `notebook_id`.
    #[allow(dead_code)]
    pub(crate) fn notebook_page_note_idx(&self, nb_id: u64, page_idx: usize) -> Option<usize> {
        let note_id = {
            let nb = self.notebooks.iter().find(|nb| nb.id == nb_id)?;
            *nb.note_ids.get(page_idx)?
        };
        self.notes.iter().position(|n| n.data.id == note_id)
    }

    /// Advance a specific open notebook by one page forward or backward (wraps around).
    /// The incoming page inherits the outgoing page's size and position so the
    /// notebook window stays fixed while only the content changes (book feel).
    pub(crate) fn cycle_notebook_page(&mut self, nb_id: u64, forward: bool) {
        let page_idx = match self.notebooks_open.get(&nb_id) {
            Some(&p) => p,
            None => return,
        };

        // Collect what we need from notebooks before mutably touching notes.
        let (new_page, old_note_id, target_note_id) = {
            let nb = match self.notebooks.iter().find(|nb| nb.id == nb_id) {
                Some(nb) => nb,
                None => return,
            };
            let total = nb.note_ids.len();
            if total == 0 { return; }
            let new_page = if forward {
                (page_idx + 1) % total
            } else {
                (page_idx + total - 1) % total
            };
            (new_page, nb.note_ids[page_idx], nb.note_ids[new_page])
        };

        // Read the outgoing page's geometry.
        let (x, y, w, h) = {
            if let Some(old_idx) = self.notes.iter().position(|n| n.data.id == old_note_id) {
                let d = &self.notes[old_idx].data;
                (d.x, d.y, d.width, d.height)
            } else {
                return;
            }
        };

        // Apply geometry to the incoming page, bring it to the top, and focus it.
        if let Some(note_idx) = self.notes.iter().position(|n| n.data.id == target_note_id) {
            {
                let d = &mut self.notes[note_idx].data;
                d.x = x;
                d.y = y;
                d.width = w;
                d.height = h;
            }
            self.notebooks_open.insert(nb_id, new_page);
            let note_idx = self.bring_to_front(note_idx);
            self.focus = self.note_focus(note_idx);
        }
    }

    /// Remove a note from whichever notebook it belongs to (if any).
    /// Does NOT remove the note from `App::notes`.
    pub(crate) fn detach_from_notebook(&mut self, note_idx: usize) {
        let note_id = self.notes[note_idx].data.id;
        if let Some(nb_id) = self.notes[note_idx].data.notebook_id {
            if let Some(nb) = self.notebooks.iter_mut().find(|nb| nb.id == nb_id) {
                nb.note_ids.retain(|&id| id != note_id);
            }
            // Close book mode for this specific notebook if it is currently open.
            self.notebooks_open.remove(&nb_id);
        }
        self.notes[note_idx].data.notebook_id = None;
        self.notes[note_idx].data.on_corkboard = false;
    }

    // ── Workspace helpers ───────────────────────────────────────────────────

    /// Return the index of the background shell note for the active workspace, if any.
    pub(crate) fn background_note_idx(&self) -> Option<usize> {
        self.notes.iter().position(|n| {
            n.data.is_shell && n.data.is_background && n.data.workspace_id == self.active_workspace
        })
    }

    /// Return the appropriate shell focus for the active workspace:
    /// `Focus::BackgroundShell(idx)` when a background note exists, else `Focus::Shell`.
    pub(crate) fn focus_for_active_workspace(&self) -> Focus {
        if let Some(idx) = self.background_note_idx() {
            Focus::BackgroundShell(idx)
        } else {
            Focus::Shell
        }
    }

    /// Switch to workspace `ws`, cancelling any drag and resetting focus.
    pub(crate) fn switch_workspace(&mut self, ws: u8) {
        if ws >= self.workspace_count { return; }
        self.active_workspace = ws;
        self.drag = None;
        self.focus = self.focus_for_active_workspace();
    }

    /// Assign a note to a notebook (and move it to the corkboard).
    pub(crate) fn assign_to_notebook(&mut self, note_idx: usize, nb_id: u64) {
        // Detach from any current notebook first.
        self.detach_from_notebook(note_idx);
        let note_id = self.notes[note_idx].data.id;
        self.notes[note_idx].data.notebook_id = Some(nb_id);
        self.notes[note_idx].data.on_corkboard = true;
        if let Some(nb) = self.notebooks.iter_mut().find(|nb| nb.id == nb_id) {
            if !nb.note_ids.contains(&note_id) {
                nb.note_ids.push(note_id);
            }
        }
    }

    // ── Trash helpers ────────────────────────────────────────────────────────

    /// Move a note to the recycle bin: write to trash dir, remove from notes dir.
    pub(crate) fn trash_note(&mut self, mut note: Note) {
        note.sync();
        let trashed = TrashedNote {
            deleted_at: trash::now_secs(),
            data: note.data,
        };
        let _ = trash::save_trash_note(&trashed);
        note::delete_note_file(trashed.data.id);
        self.trash.push(trashed);
    }

    /// Restore a trashed note back to the corkboard on the active workspace.
    pub(crate) fn restore_from_trash(&mut self, trash_idx: usize) -> Result<()> {
        if trash_idx >= self.trash.len() { return Ok(()); }
        let trashed = self.trash.remove(trash_idx);
        trash::delete_trash_note(trashed.data.id);
        let mut data = trashed.data;
        data.on_corkboard = true;
        data.workspace_id = self.active_workspace;
        // Background shell notes come back as regular floating terminal notes so
        // the run-loop doesn't immediately override their position.  The user can
        // re-background them with Ctrl+B after picking them up from the corkboard.
        if data.is_background {
            data.is_background = false;
            data.show_border = true;
            let (cols, rows) = self.term_size;
            let w = CARD_W * 2;
            let h = CARD_H * 2;
            data.width  = w;
            data.height = h;
            data.x = (cols - w) / 2;
            data.y = (rows - h) / 2;
        }
        let note = Note::from_data(data, self.config.shell_scrollback)?;
        self.notes.push(note);
        Ok(())
    }

    /// Permanently delete one note from the trash (no recovery possible).
    pub(crate) fn permanently_delete_trash(&mut self, trash_idx: usize) {
        if trash_idx >= self.trash.len() { return; }
        let trashed = self.trash.remove(trash_idx);
        trash::delete_trash_note(trashed.data.id);
    }

    /// Empty the entire trash.
    pub(crate) fn empty_trash(&mut self) {
        self.trash.clear();
        trash::clear_trash_dir();
    }
}
