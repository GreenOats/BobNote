//! Keyboard, mouse, and paste-event handling for `App`.
//!
//! These `impl App` methods are kept here so the input-dispatch logic can be
//! read and edited in isolation from the main state-machine and run-loop code.

use crate::{
    app::{App, DragMode, Focus, NotebookPicker, NotebookPickerMode, NoteType, OcclusionDim, SettingsSection},
    colors::{BG_PALETTE, BORDER_PALETTE},
    constants::{BG_SHELL_INSET, MIN_NOTE_H, MIN_NOTE_W, PROMPT_LINES},
    note::NoteKind,
};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::style::Style;
use tui_textarea::TextArea;

impl App {
    // -----------------------------------------------------------------------
    // Keyboard
    // -----------------------------------------------------------------------

    pub(crate) fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if self.splash {
            self.splash = false;
        }

        // Snapshot config to avoid borrow conflicts when mutating App later.
        let cfg = self.config;

        // ── F1 / hint_bar: toggle hint bar — truly global ───────────────────
        if cfg.hint_bar.matches(key) {
            self.show_hints = !self.show_hints;
            return Ok(());
        }

        // ── Alt+Q / quit: global quit ───────────────────────────────────────
        if cfg.quit.matches(key) {
            self.running = false;
            return Ok(());
        }

        // ── Alt+B / corkboard: toggle corkboard — global open/close ────────
        if cfg.corkboard.matches(key) {
            self.corkboard_open = !self.corkboard_open;
            if !self.corkboard_open {
                self.focus = self.focus_for_active_workspace();
            }
            return Ok(());
        }

        // Notebook picker takes priority over everything else.
        if self.notebook_picker.is_some() {
            return self.handle_notebook_picker_key(key);
        }

        if self.corkboard_open {
            if self.corkboard_expanded.is_some() {
                return self.handle_corkboard_expanded_key(key);
            }
            return self.handle_corkboard_key(key);
        }

        // ── Universal Alt+key actions ───────────────────────────────────────
        // These work from both Shell and Note focus without needing to escape
        // back to the shell first.

        // Alt+N: always create a new text note.
        if cfg.new_note.matches(key) {
            let idx = self.new_note();
            self.focus = Focus::Note(idx, NoteType::Text);
            return Ok(());
        }

        // Alt+F: focus (bring to front) the topmost visible text note.
        if cfg.focus_note.matches(key) {
            let target = self.notes.iter().enumerate().rev()
                .find(|(_, n)| {
                    !n.data.is_shell && !n.data.is_photo
                        && !n.data.is_checklist && !n.data.on_corkboard
                        && n.data.notebook_id.is_none()
                        && n.data.workspace_id == self.active_workspace
                })
                .map(|(i, _)| i);
            if let Some(i) = target {
                let i = self.bring_to_front(i);
                self.focus = self.note_focus(i);
            }
            return Ok(());
        }

        // Alt+L: create checklist, or focus the topmost visible one on this workspace.
        if cfg.new_checklist.matches(key) {
            let target = self.notes.iter().enumerate().rev()
                .find(|(_, n)| {
                    n.data.is_checklist && !n.data.on_corkboard
                        && n.data.notebook_id.is_none()
                        && n.data.workspace_id == self.active_workspace
                })
                .map(|(i, _)| i);
            if let Some(i) = target {
                let i = self.bring_to_front(i);
                self.focus = self.note_focus(i);
            } else {
                let idx = self.new_checklist();
                self.focus = Focus::Note(idx, NoteType::CheckList);
            }
            return Ok(());
        }

        // Alt+T: always create a new terminal note.
        if cfg.new_terminal.matches(key) {
            let idx = self.new_shell_note()?;
            self.focus = Focus::Note(idx, NoteType::Shell);
            return Ok(());
        }

        // Alt+G: focus the topmost visible terminal note on this workspace.
        if cfg.focus_terminal.matches(key) {
            let target = self.notes.iter().enumerate().rev()
                .find(|(_, n)| {
                    n.data.is_shell && !n.data.is_background && !n.data.on_corkboard
                        && n.data.notebook_id.is_none()
                        && n.data.workspace_id == self.active_workspace
                })
                .map(|(i, _)| i);
            if let Some(i) = target {
                let i = self.bring_to_front(i);
                self.focus = self.note_focus(i);
            }
            return Ok(());
        }

        // Alt+Space: cycle forward through terminal notes on the active workspace.
        // Works from any context (shell, text note, another terminal note).
        if key.modifiers == KeyModifiers::ALT && key.code == KeyCode::Char(' ') {
            let current_idx = match &self.focus {
                Focus::Note(i, NoteType::Shell) => Some(*i),
                _ => None,
            };
            let terminals: Vec<usize> = self.notes.iter().enumerate()
                .filter(|(_, n)| {
                    n.data.is_shell && !n.data.is_background && !n.data.on_corkboard
                        && n.data.notebook_id.is_none()
                        && n.data.workspace_id == self.active_workspace
                })
                .map(|(i, _)| i)
                .collect();
            if !terminals.is_empty() {
                let next_idx = match current_idx {
                    Some(cur) => {
                        let pos = terminals.iter().position(|&i| i == cur).unwrap_or(0);
                        terminals[(pos + 1) % terminals.len()]
                    }
                    None => terminals[0],
                };
                let next_idx = self.bring_to_front(next_idx);
                self.focus = Focus::Note(next_idx, NoteType::Shell);
            }
            return Ok(());
        }

        // Alt+O: focus an open book page; cycle between open notebooks if already there.
        if cfg.focus_book.matches(key) {
            if !self.notebooks_open.is_empty() {
                // Stable ordering: follow notebooks Vec order.
                let open_nb_ids: Vec<u64> = self.notebooks.iter()
                    .filter(|nb| self.notebooks_open.contains_key(&nb.id) && !nb.note_ids.is_empty())
                    .map(|nb| nb.id)
                    .collect();

                // Is the currently focused note a current book page in any open notebook?
                let current_nb_id: Option<u64> = if let Focus::Note(i, _) = &self.focus {
                    let i = *i;
                    self.notes[i].data.notebook_id.filter(|&nb_id| {
                        self.notebooks_open.get(&nb_id).map_or(false, |&page_idx| {
                            self.notebooks.iter()
                                .find(|nb| nb.id == nb_id)
                                .and_then(|nb| nb.note_ids.get(page_idx))
                                .map_or(false, |&nid| nid == self.notes[i].data.id)
                        })
                    })
                } else { None };

                // Pick the next notebook: cycle if already on a book page, else jump to first.
                let next_nb_id = if let Some(cur_id) = current_nb_id {
                    let pos = open_nb_ids.iter().position(|&id| id == cur_id).unwrap_or(0);
                    open_nb_ids.get((pos + 1) % open_nb_ids.len()).copied()
                } else {
                    open_nb_ids.first().copied()
                };

                if let Some(nb_id) = next_nb_id {
                    if let Some(&page_idx) = self.notebooks_open.get(&nb_id) {
                        if let Some(&note_id) = self.notebooks.iter()
                            .find(|nb| nb.id == nb_id)
                            .and_then(|nb| nb.note_ids.get(page_idx))
                        {
                            if let Some(note_idx) = self.notes.iter().position(|n| n.data.id == note_id) {
                                let note_idx = self.bring_to_front(note_idx);
                                self.focus = self.note_focus(note_idx);
                            }
                        }
                    }
                }
            }
            return Ok(());
        }

        // Alt+V: paste from system clipboard — context-aware universal paste.
        if cfg.paste.matches(key) {
            let text = self.clipboard
                .as_mut()
                .and_then(|cb| cb.get_text().ok())
                .unwrap_or_default();
            if !text.is_empty() {
                enum PasteTarget { BgShell(usize), TextNote(usize), ShellNote(usize), None }
                let target = match &self.focus {
                    Focus::BackgroundShell(i)               => PasteTarget::BgShell(*i),
                    Focus::Note(i, NoteType::Text)      => PasteTarget::TextNote(*i),
                    Focus::Note(i, NoteType::Shell)     => PasteTarget::ShellNote(*i),
                    _                                       => PasteTarget::None,
                };
                match target {
                    PasteTarget::BgShell(idx) => {
                        if let NoteKind::Shell { pty, .. } = &mut self.notes[idx].kind {
                            let _ = pty.write_bytes(text.as_bytes());
                        }
                    }
                    PasteTarget::TextNote(idx) => {
                        if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                            ta.set_yank_text(text);
                            ta.input(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
                        }
                    }
                    PasteTarget::ShellNote(idx) => {
                        if let NoteKind::Shell { pty, .. } = &mut self.notes[idx].kind {
                            let _ = pty.write_bytes(text.as_bytes());
                        }
                    }
                    PasteTarget::None => {}
                }
            }
            return Ok(());
        }

        // Alt+R: rename the active workspace — works from any context.
        if cfg.rename_workspace.matches(key) {
            let current = self.workspace_names
                .get(self.active_workspace as usize)
                .cloned()
                .unwrap_or_default();
            self.focus = Focus::RenamingWorkspace(current);
            return Ok(());
        }

        // Pull focus out to avoid simultaneous borrow issues.
        let focus = std::mem::replace(&mut self.focus, Focus::BackgroundShell(0));

        match focus {
            // ── Background shell mode ───────────────────────────────────────
            // Same controls as Shell but routes input to a specific note's PTY.
            Focus::BackgroundShell(bg_idx) => {
                let alt  = key.modifiers.contains(KeyModifiers::ALT);
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

                // Alt+Left / Alt+H: previous workspace
                if alt && matches!(key.code, KeyCode::Left | KeyCode::Char('h')) {
                    let new_ws = self.active_workspace.saturating_sub(1);
                    self.switch_workspace(new_ws);
                    return Ok(());
                }
                // Alt+Right: next workspace
                if alt && matches!(key.code, KeyCode::Right) {
                    let new_ws = (self.active_workspace + 1).min(self.workspace_count.saturating_sub(1));
                    self.switch_workspace(new_ws);
                    return Ok(());
                }
                // Alt+= : add workspace
                if alt && key.code == KeyCode::Char('=') {
                    let n = self.workspace_count.saturating_add(1);
                    self.workspace_count = n;
                    let new_ws = n - 1;
                    self.workspace_names.push(format!("WS {n}"));
                    // Create a dedicated background shell note for the new workspace.
                    let new_idx = self.new_shell_note()?;
                    self.notes[new_idx].data.workspace_id = new_ws;
                    self.notes[new_idx].data.is_background = true;
                    self.notes[new_idx].data.show_border = false;
                    self.focus = Focus::BackgroundShell(bg_idx);
                    return Ok(());
                }
                // Alt+- : remove the current workspace
                if alt && key.code == KeyCode::Char('-') && self.workspace_count > 1 {
                    let ws = self.active_workspace;
                    // Trash the background shell note for this workspace so it can be revived.
                    let bg_indices: Vec<usize> = self.notes.iter().enumerate()
                        .filter(|(_, n)| n.data.is_background && n.data.workspace_id == ws)
                        .map(|(i, _)| i)
                        .collect();
                    for i in bg_indices.into_iter().rev() {
                        let note = self.notes.remove(i);
                        self.trash_note(note);
                    }
                    // Reassign any remaining notes on this workspace to the adjacent one,
                    // and shift down the workspace_id of all notes with a higher id.
                    let new_ws = ws.saturating_sub(1);
                    for n in self.notes.iter_mut() {
                        if n.data.workspace_id == ws {
                            n.data.workspace_id = new_ws;
                        } else if n.data.workspace_id > ws {
                            n.data.workspace_id -= 1;
                        }
                    }
                    self.workspace_names.remove(ws as usize);
                    self.workspace_count -= 1;
                    self.switch_workspace(new_ws);
                    return Ok(());
                }
                // PageUp: scroll bg note up
                if key.code == KeyCode::PageUp {
                    if let NoteKind::Shell { parser, scroll_offset, .. } = &mut self.notes[bg_idx].kind {
                        if !parser.screen().alternate_screen() {
                            let step = (self.term_size.1 / 2).max(1) as i64;
                            *scroll_offset += step;
                        }
                    }
                    self.focus = Focus::BackgroundShell(bg_idx);
                    return Ok(());
                }
                // PageDown: scroll bg note down
                if key.code == KeyCode::PageDown {
                    if let NoteKind::Shell { parser, scroll_offset, rows, .. } = &mut self.notes[bg_idx].kind {
                        let min = if parser.screen().alternate_screen() {
                            0
                        } else {
                            -(*rows as i64 - PROMPT_LINES)
                        };
                        let step = (self.term_size.1 / 2).max(1) as i64;
                        *scroll_offset = (*scroll_offset - step).max(min);
                    }
                    self.focus = Focus::BackgroundShell(bg_idx);
                    return Ok(());
                }
                // Ctrl+V: enter screenshot mode (same as shell)
                if ctrl && key.code == KeyCode::Char('v') {
                    if let NoteKind::Shell { parser, scroll_offset, .. } = &self.notes[bg_idx].kind {
                        let (live_crow, ccol) = parser.screen().cursor_position();
                        // anchor_row is screen-absolute: parser row + BG_SHELL_INSET offset.
                        // Clamp to the note's visible area (bg note starts at BG_SHELL_INSET,
                        // ends at term_rows - BG_SHELL_INSET - 1).
                        let note_rows = self.notes[bg_idx].data.height as i64;
                        let anchor_row = (live_crow as i64 + *scroll_offset)
                            .clamp(0, note_rows - 1) as u16
                            + BG_SHELL_INSET;
                        self.focus = Focus::Selecting {
                            anchor_col: ccol,
                            anchor_row,
                            cursor_col: ccol,
                            cursor_row: anchor_row,
                            from_bg_shell: Some(bg_idx),
                        };
                    }
                    return Ok(());
                }
                // Shift+Insert: paste from clipboard into bg shell note
                if !alt && !ctrl && key.code == KeyCode::Insert
                    && key.modifiers.contains(KeyModifiers::SHIFT)
                {
                    let text = self.clipboard
                        .as_mut()
                        .and_then(|cb| cb.get_text().ok())
                        .unwrap_or_default();
                    if !text.is_empty() {
                        if let NoteKind::Shell { pty, .. } = &mut self.notes[bg_idx].kind {
                            let _ = pty.write_bytes(text.as_bytes());
                        }
                    }
                    self.focus = Focus::BackgroundShell(bg_idx);
                    return Ok(());
                }
                // Alt+C: re-copy the stored selection without snapping the scroll.
                if alt && !ctrl && key.code == KeyCode::Char('c') {
                    if let Some((sc, sr, ec, er)) = self.text_selection {
                        self.copy_shell_note_stream_selection(
                            bg_idx,
                            sc, sr.saturating_sub(BG_SHELL_INSET),
                            ec, er.saturating_sub(BG_SHELL_INSET),
                        );
                    }
                    self.focus = Focus::BackgroundShell(bg_idx);
                    return Ok(());
                }
                // Ctrl+Alt+I: toggle PTY output logging on/off.
                if cfg.toggle_log.matches(key) {
                    let logging = matches!(&self.notes[bg_idx].kind,
                        NoteKind::Shell { log_file, .. } if log_file.is_some());
                    if logging {
                        if let NoteKind::Shell { log_file, log_path, .. } =
                            &mut self.notes[bg_idx].kind
                        {
                            *log_file = None; // flushes + closes via BufWriter::drop
                            *log_path = None;
                        }
                        self.focus = Focus::BackgroundShell(bg_idx);
                    } else {
                        let default = self.default_log_path(bg_idx);
                        self.focus = Focus::LoggingSetup(bg_idx, default);
                    }
                    return Ok(());
                }
                // Everything else: route to bg note's PTY
                if let NoteKind::Shell { pty, scroll_offset, .. } = &mut self.notes[bg_idx].kind {
                    if *scroll_offset > 0 { *scroll_offset = 0; }
                    pty.write_key(key)?;
                }
                self.focus = Focus::BackgroundShell(bg_idx);
            }

            // ── Note mode ───────────────────────────────────────────────────
            Focus::Note(idx, note_type) => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                let alt  = key.modifiers.contains(KeyModifiers::ALT);

                // ── Book mode Tab cycling ────────────────────────────────────
                // If the focused note is the current page of any open notebook,
                // Tab/Shift+Tab cycles that specific notebook's pages.
                if !ctrl && matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
                    let book_nb_id: Option<u64> = self.notes[idx].data.notebook_id
                        .filter(|&nb_id| {
                            self.notebooks_open.get(&nb_id).map_or(false, |&page_idx| {
                                self.notebooks.iter()
                                    .find(|nb| nb.id == nb_id)
                                    .and_then(|nb| nb.note_ids.get(page_idx))
                                    .map_or(false, |&nid| nid == self.notes[idx].data.id)
                            })
                        });
                    if let Some(nb_id) = book_nb_id {
                        let forward = key.code == KeyCode::Tab;
                        self.cycle_notebook_page(nb_id, forward);
                        return Ok(());
                    }
                }

                match (ctrl, alt, key.code) {
                    // Back to background shell for the active workspace
                    (true, false, KeyCode::Char('e')) => {
                        self.focus = self.focus_for_active_workspace();
                    }
                    // Ctrl+X on checklist: toggle [ ]/[x] prefix.
                    (true, false, KeyCode::Char('x')) if matches!(note_type, NoteType::CheckList) => {
                        if let NoteKind::CheckList(ta, scroll_top) = &mut self.notes[idx].kind {
                            let row = ta.cursor().0;
                            let mut lines: Vec<String> = ta.lines().to_vec();
                            if let Some(line) = lines.get_mut(row) {
                                *line = if line.starts_with("[x] ") {
                                    format!("[ ] {}", &line[4..])
                                } else if line.starts_with("[ ] ") {
                                    format!("[x] {}", &line[4..])
                                } else {
                                    format!("[ ] {line}")
                                };
                            }
                            let prev_scroll = *scroll_top;
                            let mut new_ta = TextArea::from(lines);
                            new_ta.set_cursor_line_style(Style::default());
                            for _ in 0..row {
                                new_ta.input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
                            }
                            *ta = new_ta;
                            *scroll_top = prev_scroll;
                        }
                        self.focus = self.note_focus(idx);
                    }
                    // Rename note
                    (true, false, KeyCode::Char('t')) => {
                        let current = self.notes[idx].data.title.clone();
                        self.focus = Focus::Renaming(idx, current);
                    }
                    // Open settings popup
                    (true, false, KeyCode::Char('s')) => {
                        self.focus = Focus::Settings(idx, SettingsSection::BorderToggle);
                    }
                    // Toggle notebook persistence (Alt+P) when focused on a book page.
                    // Persistent notebooks float across all workspaces (∞ marker in title).
                    // Non-persistent notebooks stay on their own workspace like regular notes.
                    (false, true, KeyCode::Char('p')) if self.notes[idx].data.notebook_id
                        .map_or(false, |nb_id| {
                            self.notebooks_open.get(&nb_id).map_or(false, |&page_idx| {
                                self.notebooks.iter()
                                    .find(|nb| nb.id == nb_id)
                                    .and_then(|nb| nb.note_ids.get(page_idx))
                                    .map_or(false, |&note_id| note_id == self.notes[idx].data.id)
                            })
                        }) => {
                        let nb_id = self.notes[idx].data.notebook_id.unwrap();
                        if let Some(nb) = self.notebooks.iter_mut().find(|nb| nb.id == nb_id) {
                            nb.persistent = !nb.persistent;
                        }
                    }
                    // In book mode: close this note's notebook and open the corkboard
                    // with its folder card selected ("pin back to corkboard").
                    (true, false, KeyCode::Char('p')) if self.notes[idx].data.notebook_id
                        .map_or(false, |nb_id| {
                            self.notebooks_open.get(&nb_id).map_or(false, |&page_idx| {
                                self.notebooks.iter()
                                    .find(|nb| nb.id == nb_id)
                                    .and_then(|nb| nb.note_ids.get(page_idx))
                                    .map_or(false, |&note_id| note_id == self.notes[idx].data.id)
                            })
                        }) => {
                        let nb_id = self.notes[idx].data.notebook_id.unwrap();
                        self.notebooks_open.remove(&nb_id);
                        let items = self.corkboard_items();
                        if let Some(pos) = items.iter().position(
                            |item| matches!(item, crate::app::CorkItem::Notebook(id) if *id == nb_id)
                        ) {
                            self.corkboard_selected = pos;
                        }
                        self.focus = self.focus_for_active_workspace();
                    }
                    // Pin note to corkboard
                    (true, false, KeyCode::Char('p')) => {
                        self.notes[idx].data.on_corkboard = true;
                        self.focus = self.focus_for_active_workspace();
                    }
                    // Assign note to a notebook
                    (true, false, KeyCode::Char('g')) => {
                        if !self.notebooks.is_empty() {
                            self.notebook_picker = Some(NotebookPicker {
                                selected: 0,
                                mode: NotebookPickerMode::AssignToNotebook(idx),
                            });
                        }
                        self.focus = self.note_focus(idx);
                    }
                    // Remove current page from its notebook (only when it is an open book page).
                    (true, false, KeyCode::Char('r')) if self.notes[idx].data.notebook_id
                        .map_or(false, |nb_id| self.notebooks_open.contains_key(&nb_id)) => {
                        let open_nb_id = self.notes[idx].data.notebook_id.unwrap();
                        let current_page_idx = *self.notebooks_open.get(&open_nb_id).unwrap();
                        let note_id = self.notes[idx].data.id;
                        let (x, y, w, h) = {
                            let d = &self.notes[idx].data;
                            (d.x, d.y, d.width, d.height)
                        };
                        self.notes[idx].data.notebook_id = None;
                        if let Some(nb) = self.notebooks.iter_mut().find(|nb| nb.id == open_nb_id) {
                            nb.note_ids.retain(|&id| id != note_id);
                        }
                        let next = {
                            let nb = self.notebooks.iter().find(|nb| nb.id == open_nb_id);
                            nb.and_then(|nb| {
                                if nb.note_ids.is_empty() { return None; }
                                let new_page = current_page_idx.min(nb.note_ids.len() - 1);
                                let target_id = nb.note_ids[new_page];
                                let new_note_idx = self.notes.iter().position(|n| n.data.id == target_id)?;
                                Some((new_page, new_note_idx))
                            })
                        };
                        match next {
                            Some((new_page, new_note_idx)) => {
                                let d = &mut self.notes[new_note_idx].data;
                                d.x = x; d.y = y; d.width = w; d.height = h;
                                self.notebooks_open.insert(open_nb_id, new_page);
                                self.focus = self.note_focus(new_note_idx);
                            }
                            None => {
                                self.notebooks_open.remove(&open_nb_id);
                                self.focus = self.focus_for_active_workspace();
                            }
                        }
                    }
                    // Toggle always-on-top pin (Ctrl+F = "pin to Front")
                    (true, false, KeyCode::Char('f')) => {
                        let new_idx = self.toggle_pin(idx);
                        self.focus = self.note_focus(new_idx);
                    }
                    // Toggle drop-shadows
                    (true, false, KeyCode::Char('d')) => {
                        self.show_shadows = !self.show_shadows;
                        self.focus = self.note_focus(idx);
                    }
                    // Cycle occlusion dimming (Off → On → BlackText → Off)
                    (true, false, KeyCode::Char('o')) => {
                        self.occlusion_dim = match self.occlusion_dim {
                            OcclusionDim::Off       => OcclusionDim::On,
                            OcclusionDim::On        => OcclusionDim::BlackText,
                            OcclusionDim::BlackText => OcclusionDim::Off,
                        };
                        self.focus = self.note_focus(idx);
                    }
                    // Open corkboard (or return to this notebook's sub-grid when on a book page).
                    (true, false, KeyCode::Char('k')) => {
                        let book_nb_id: Option<u64> = self.notes[idx].data.notebook_id
                            .filter(|&nb_id| {
                                self.notebooks_open.get(&nb_id).map_or(false, |&page_idx| {
                                    self.notebooks.iter()
                                        .find(|nb| nb.id == nb_id)
                                        .and_then(|nb| nb.note_ids.get(page_idx))
                                        .map_or(false, |&nid| nid == self.notes[idx].data.id)
                                })
                            });
                        if let Some(nb_id) = book_nb_id {
                            self.notebooks_open.remove(&nb_id);
                            self.corkboard_open = true;
                            self.corkboard_notebook = Some(nb_id);
                        } else {
                            self.corkboard_open = true;
                            self.focus = self.note_focus(idx);
                        }
                    }
                    // Ctrl+C in text notes: copy selection to internal yank + system clipboard.
                    (true, false, KeyCode::Char('c')) if matches!(note_type, NoteType::Text) => {
                        if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                            ta.input(key);
                        }
                        let yanked = match &self.notes[idx].kind {
                            NoteKind::Text(ta, _) => ta.yank_text().to_string(),
                            _ => String::new(),
                        };
                        if !yanked.is_empty() {
                            if let Some(ref mut cb) = self.clipboard {
                                let _ = cb.set_text(yanked);
                            }
                        }
                        self.focus = self.note_focus(idx);
                    }
                    // Ctrl+V in text notes: enter keyboard visual-selection mode.
                    (true, false, KeyCode::Char('v')) if matches!(note_type, NoteType::Text) => {
                        let (anchor_row, anchor_col) = match &self.notes[idx].kind {
                            NoteKind::Text(ta, _) => ta.cursor(),
                            _ => (0, 0),
                        };
                        self.focus = Focus::TextVisual { note_idx: idx, anchor_row, anchor_col };
                    }
                    // Alt+C in text notes: copy selection to system clipboard.
                    (false, true, KeyCode::Char('c')) if matches!(note_type, NoteType::Text) => {
                        if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                            ta.input(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
                        }
                        let yanked = match &self.notes[idx].kind {
                            NoteKind::Text(ta, _) => ta.yank_text().to_string(),
                            _ => String::new(),
                        };
                        if !yanked.is_empty() {
                            if let Some(ref mut cb) = self.clipboard {
                                let _ = cb.set_text(yanked);
                            }
                        }
                        self.focus = self.note_focus(idx);
                    }
                    // Shift+Insert: paste from system clipboard.
                    (false, false, KeyCode::Insert) if key.modifiers.contains(KeyModifiers::SHIFT) => {
                        let text = self.clipboard
                            .as_mut()
                            .and_then(|cb| cb.get_text().ok())
                            .unwrap_or_default();
                        if !text.is_empty() {
                            match note_type {
                                NoteType::Text => {
                                    if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                                        ta.set_yank_text(text);
                                        ta.input(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
                                    }
                                }
                                NoteType::Shell => {
                                    if let NoteKind::Shell { pty, .. } = &mut self.notes[idx].kind {
                                        let _ = pty.write_bytes(text.as_bytes());
                                    }
                                }
                                NoteType::Photo | NoteType::CheckList => {}
                            }
                        }
                        self.focus = self.note_focus(idx);
                    }
                    // Copy photo note content to clipboard.
                    (true, false, KeyCode::Char('c')) if matches!(note_type, NoteType::Photo) => {
                        let text: String = self.notes[idx].data.photo_rows
                            .iter()
                            .map(|row| {
                                let mut line: String = row.iter().map(|cell| cell.sym.as_str()).collect();
                                let trimmed_len = line.trim_end().len();
                                line.truncate(trimmed_len);
                                line
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        if let Some(ref mut clipboard) = self.clipboard {
                            let _ = clipboard.set_text(text);
                        }
                        self.focus = self.note_focus(idx);
                    }
                    // Alt+C in shell notes: re-copy the stored selection.
                    (false, true, KeyCode::Char('c')) if matches!(note_type, NoteType::Shell) => {
                        if let Some((sel_idx, sc, sr, ec, er)) = self.shell_note_selection {
                            if sel_idx == idx {
                                let inner_x = self.notes[idx].data.x + 1;
                                let inner_y = self.notes[idx].data.y + 1;
                                self.copy_shell_note_stream_selection(
                                    idx,
                                    sc.saturating_sub(inner_x),
                                    sr.saturating_sub(inner_y),
                                    ec.saturating_sub(inner_x),
                                    er.saturating_sub(inner_y),
                                );
                            }
                        }
                        self.focus = self.note_focus(idx);
                    }
                    // Ctrl+Y on shell notes: snapshot as a Photo note.
                    (true, false, KeyCode::Char('y')) if matches!(note_type, NoteType::Shell) => {
                        self.snapshot_shell_note(idx);
                    }
                    // Ctrl+Alt+I on shell notes: toggle PTY output logging.
                    _ if matches!(note_type, NoteType::Shell) && cfg.toggle_log.matches(key) => {
                        let logging = matches!(&self.notes[idx].kind,
                            NoteKind::Shell { log_file, .. } if log_file.is_some());
                        if logging {
                            if let NoteKind::Shell { log_file, log_path, .. } =
                                &mut self.notes[idx].kind
                            {
                                *log_file = None;
                                *log_path = None;
                            }
                            self.focus = self.note_focus(idx);
                        } else {
                            let default = self.default_log_path(idx);
                            self.focus = Focus::LoggingSetup(idx, default);
                        }
                    }
                    // Ctrl+B on shell notes: toggle background mode (fills workspace as background shell).
                    (true, false, KeyCode::Char('b')) if matches!(note_type, NoteType::Shell) => {
                        let was_bg = self.notes[idx].data.is_background;
                        self.notes[idx].data.is_background = !was_bg;
                        if !was_bg {
                            // Becoming a background note: remove border, make it fullscreen
                            self.notes[idx].data.show_border = false;
                            self.focus = Focus::BackgroundShell(idx);
                        } else {
                            // Leaving background mode: restore border
                            self.notes[idx].data.show_border = true;
                            self.focus = self.note_focus(idx);
                        }
                    }
                    // Close note → move to recycle bin
                    (true, false, KeyCode::Char('w')) => {
                        // If the shell note is running a TUI app (alternate screen),
                        // pass Ctrl+W through rather than closing the note.
                        let in_alt = matches!(&self.notes[idx].kind,
                            NoteKind::Shell { parser, .. } if parser.screen().alternate_screen()
                        );
                        if in_alt {
                            if let NoteKind::Shell { pty, .. } = &mut self.notes[idx].kind {
                                pty.write_key(key)?;
                            }
                            self.focus = self.note_focus(idx);
                        } else {
                            self.detach_from_notebook(idx);
                            let note = self.notes.remove(idx);
                            self.trash_note(note);
                            self.focus = self.focus_for_active_workspace();
                        }
                    }
                    // Tab: cycle to next text or checklist note.
                    (false, false, KeyCode::Tab)
                        if matches!(note_type, NoteType::Text | NoteType::CheckList) =>
                    {
                        let len = self.notes.len();
                        let next = (1..len)
                            .map(|offset| (idx + offset) % len)
                            .find(|&i| {
                                !self.notes[i].data.is_shell
                                    && !self.notes[i].data.is_photo
                                    && !self.notes[i].data.on_corkboard
                                    && self.notes[i].data.notebook_id.is_none()
                                    && self.notes[i].data.workspace_id == self.active_workspace
                            });
                        if let Some(next) = next {
                            let next = self.bring_to_front(next);
                            self.focus = self.note_focus(next);
                        } else {
                            self.focus = self.note_focus(idx);
                        }
                    }

                    // ── Move: Alt+hjkl / Alt+arrows ───────────────────────────
                    (false, true, KeyCode::Char('h') | KeyCode::Left) => {
                        self.notes[idx].data.x = self.notes[idx].data.x.saturating_sub(1);
                        self.clamp_note(idx);
                        self.focus = self.note_focus(idx);
                    }
                    (false, true, KeyCode::Char('l') | KeyCode::Right) => {
                        self.notes[idx].data.x = self.notes[idx].data.x.saturating_add(1);
                        self.clamp_note(idx);
                        self.focus = self.note_focus(idx);
                    }
                    (false, true, KeyCode::Char('k') | KeyCode::Up) => {
                        self.notes[idx].data.y = self.notes[idx].data.y.saturating_sub(1);
                        self.clamp_note(idx);
                        self.focus = self.note_focus(idx);
                    }
                    (false, true, KeyCode::Char('j') | KeyCode::Down) => {
                        self.notes[idx].data.y = self.notes[idx].data.y.saturating_add(1);
                        self.clamp_note(idx);
                        self.focus = self.note_focus(idx);
                    }

                    // ── Resize: Ctrl+Alt+hjkl / Ctrl+Alt+arrows ───────────────
                    (true, true, KeyCode::Left | KeyCode::Char('H') | KeyCode::Char('h')) => {
                        self.notes[idx].data.width = self.notes[idx]
                            .data.width.saturating_sub(1).max(MIN_NOTE_W);
                        self.focus = self.note_focus(idx);
                    }
                    (true, true, KeyCode::Right | KeyCode::Char('L') | KeyCode::Char('l')) => {
                        self.notes[idx].data.width = self.notes[idx].data.width.saturating_add(1);
                        self.clamp_note(idx);
                        self.focus = self.note_focus(idx);
                    }
                    (true, true, KeyCode::Up | KeyCode::Char('K') | KeyCode::Char('k')) => {
                        self.notes[idx].data.height = self.notes[idx]
                            .data.height.saturating_sub(1).max(MIN_NOTE_H);
                        self.focus = self.note_focus(idx);
                    }
                    (true, true, KeyCode::Down | KeyCode::Char('J') | KeyCode::Char('j')) => {
                        self.notes[idx].data.height = self.notes[idx].data.height.saturating_add(1);
                        self.clamp_note(idx);
                        self.focus = self.note_focus(idx);
                    }

                    // Anything else → text editor or the note's own shell.
                    _ => {
                        let at_last_down = key.code == KeyCode::Down && {
                            match &self.notes[idx].kind {
                                NoteKind::Text(ta, _) | NoteKind::CheckList(ta, _) =>
                                    ta.cursor().0 >= ta.lines().len().saturating_sub(1),
                                _ => false,
                            }
                        };
                        match &mut self.notes[idx].kind {
                            NoteKind::Text(textarea, scroll_top) | NoteKind::CheckList(textarea, scroll_top) => {
                                if at_last_down {
                                    *scroll_top = scroll_top.saturating_add(1);
                                } else {
                                    textarea.input(key);
                                }
                            }
                            NoteKind::Shell { pty, scroll_offset, .. } => {
                                if *scroll_offset > 0 { *scroll_offset = 0; }
                                pty.write_key(key)?;
                            }
                            NoteKind::Photo => {}
                        }
                        if !at_last_down {
                            let vis_h = self.notes[idx].data.height.saturating_sub(2) as usize;
                            if let NoteKind::Text(ta, scroll_top) | NoteKind::CheckList(ta, scroll_top) =
                                &mut self.notes[idx].kind
                            {
                                let cr = ta.cursor().0;
                                if cr < *scroll_top {
                                    *scroll_top = cr;
                                } else if vis_h > 0 && cr >= *scroll_top + vis_h {
                                    *scroll_top = cr + 1 - vis_h;
                                }
                            }
                        }
                        self.focus = self.note_focus(idx);
                    }
                }
            }

            // ── Settings popup ──────────────────────────────────────────────
            Focus::Settings(idx, section) => match key.code {
                KeyCode::Esc | KeyCode::Enter => {
                    self.focus = self.note_focus(idx);
                }
                KeyCode::Char(' ') => {
                    match section {
                        SettingsSection::BorderToggle => {
                            self.notes[idx].data.show_border = !self.notes[idx].data.show_border;
                        }
                        SettingsSection::TextWrap => {
                            self.notes[idx].data.text_wrap = !self.notes[idx].data.text_wrap;
                        }
                        _ => {}
                    }
                    self.focus = Focus::Settings(idx, section);
                }
                KeyCode::Tab | KeyCode::Up | KeyCode::Down => {
                    let show    = self.notes[idx].data.show_border;
                    let is_text = !self.notes[idx].data.is_checklist
                        && !self.notes[idx].data.is_shell
                        && !self.notes[idx].data.is_photo;
                    self.focus = Focus::Settings(
                        idx,
                        match section {
                            SettingsSection::BorderToggle => {
                                if show { SettingsSection::Border } else { SettingsSection::Background }
                            }
                            SettingsSection::Border     => SettingsSection::Background,
                            SettingsSection::Background => {
                                if is_text { SettingsSection::TextWrap } else { SettingsSection::BorderToggle }
                            }
                            SettingsSection::TextWrap   => SettingsSection::BorderToggle,
                        },
                    );
                }
                KeyCode::Left => {
                    match section {
                        SettingsSection::BorderToggle | SettingsSection::TextWrap => {}
                        SettingsSection::Border => {
                            let len = BORDER_PALETTE.len();
                            self.notes[idx].data.border_color_idx =
                                (self.notes[idx].data.border_color_idx + len - 1) % len;
                        }
                        SettingsSection::Background => {
                            let len = BG_PALETTE.len();
                            self.notes[idx].data.bg_color_idx =
                                (self.notes[idx].data.bg_color_idx + len - 1) % len;
                        }
                    }
                    self.focus = Focus::Settings(idx, section);
                }
                KeyCode::Right => {
                    match section {
                        SettingsSection::BorderToggle | SettingsSection::TextWrap => {}
                        SettingsSection::Border => {
                            self.notes[idx].data.border_color_idx =
                                (self.notes[idx].data.border_color_idx + 1) % BORDER_PALETTE.len();
                        }
                        SettingsSection::Background => {
                            self.notes[idx].data.bg_color_idx =
                                (self.notes[idx].data.bg_color_idx + 1) % BG_PALETTE.len();
                        }
                    }
                    self.focus = Focus::Settings(idx, section);
                }
                _ => { self.focus = Focus::Settings(idx, section); }
            },

            // ── Keyboard visual-block selection ─────────────────────────────
            Focus::Selecting { anchor_col, anchor_row, cursor_col, cursor_row, from_bg_shell } => {
                let (term_rows, term_cols) = (self.term_size.1, self.term_size.0);
                // Clamp cursor within the shell area (excluding tab bar at top and hint bar at bottom).
                let row_min = BG_SHELL_INSET;
                let row_max = term_rows.saturating_sub(BG_SHELL_INSET + 1);
                // Focus to restore when exiting screenshot mode.
                let exit_focus = if let Some(idx) = from_bg_shell {
                    Focus::BackgroundShell(idx)
                } else {
                    self.focus_for_active_workspace()
                };
                match key.code {
                    KeyCode::Esc => { self.focus = exit_focus; }
                    KeyCode::Enter | KeyCode::Char('y') => {
                        self.create_photo_note(anchor_col, anchor_row, cursor_col, cursor_row, from_bg_shell);
                        self.focus = exit_focus;
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::ALT) => {
                        self.copy_selection_to_clipboard(anchor_col, anchor_row, cursor_col, cursor_row, from_bg_shell);
                        self.focus = exit_focus;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.focus = Focus::Selecting {
                            anchor_col, anchor_row, cursor_col,
                            cursor_row: cursor_row.saturating_sub(1).max(row_min),
                            from_bg_shell,
                        };
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.focus = Focus::Selecting {
                            anchor_col, anchor_row, cursor_col,
                            cursor_row: (cursor_row + 1).min(row_max),
                            from_bg_shell,
                        };
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        self.focus = Focus::Selecting {
                            anchor_col, anchor_row, cursor_row,
                            cursor_col: cursor_col.saturating_sub(1),
                            from_bg_shell,
                        };
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        self.focus = Focus::Selecting {
                            anchor_col, anchor_row, cursor_row,
                            cursor_col: (cursor_col + 1).min(term_cols.saturating_sub(1)),
                            from_bg_shell,
                        };
                    }
                    _ => {
                        self.focus = Focus::Selecting { anchor_col, anchor_row, cursor_col, cursor_row, from_bg_shell };
                    }
                }
            }

            // ── Text visual-selection mode ───────────────────────────────────
            Focus::TextVisual { note_idx: idx, anchor_row, anchor_col } => {
                let do_copy_and_exit = matches!(
                    (key.modifiers, key.code),
                    (_, KeyCode::Char('y'))
                    | (KeyModifiers::ALT, KeyCode::Char('c'))
                );
                if do_copy_and_exit {
                    let (cur_row, cur_col) = match &self.notes[idx].kind {
                        NoteKind::Text(ta, _) => ta.cursor(),
                        _ => (0, 0),
                    };
                    let (sr, sc, er, ec) = if anchor_row < cur_row
                        || (anchor_row == cur_row && anchor_col <= cur_col)
                    {
                        (anchor_row, anchor_col, cur_row, cur_col)
                    } else {
                        (cur_row, cur_col, anchor_row, anchor_col)
                    };
                    self.copy_text_note_buffer_selection(idx, sr, sc, er, ec);
                    self.focus = self.note_focus(idx);
                } else {
                    match key.code {
                        KeyCode::Esc => {
                            self.focus = self.note_focus(idx);
                        }
                        KeyCode::Left | KeyCode::Char('h') => {
                            if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                                ta.input(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE));
                            }
                            self.focus = Focus::TextVisual { note_idx: idx, anchor_row, anchor_col };
                        }
                        KeyCode::Right | KeyCode::Char('l') => {
                            if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                                ta.input(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
                            }
                            self.focus = Focus::TextVisual { note_idx: idx, anchor_row, anchor_col };
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                                ta.input(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
                            }
                            self.focus = Focus::TextVisual { note_idx: idx, anchor_row, anchor_col };
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                                ta.input(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
                            }
                            self.focus = Focus::TextVisual { note_idx: idx, anchor_row, anchor_col };
                        }
                        KeyCode::Home | KeyCode::Char('0') => {
                            if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                                ta.input(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
                            }
                            self.focus = Focus::TextVisual { note_idx: idx, anchor_row, anchor_col };
                        }
                        KeyCode::End | KeyCode::Char('$') => {
                            if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                                ta.input(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
                            }
                            self.focus = Focus::TextVisual { note_idx: idx, anchor_row, anchor_col };
                        }
                        KeyCode::Char('w') => {
                            if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                                ta.input(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
                            }
                            self.focus = Focus::TextVisual { note_idx: idx, anchor_row, anchor_col };
                        }
                        KeyCode::Char('b') => {
                            if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                                ta.input(KeyEvent::new(KeyCode::Left, KeyModifiers::CONTROL));
                            }
                            self.focus = Focus::TextVisual { note_idx: idx, anchor_row, anchor_col };
                        }
                        _ => {
                            self.focus = self.note_focus(idx);
                        }
                    }
                }
                // Cursor-follow: keep the moving end of the selection visible.
                let vis_h = self.notes[idx].data.height.saturating_sub(2) as usize;
                if let NoteKind::Text(ta, scroll_top) = &mut self.notes[idx].kind {
                    let cr = ta.cursor().0;
                    if cr < *scroll_top {
                        *scroll_top = cr;
                    } else if vis_h > 0 && cr >= *scroll_top + vis_h {
                        *scroll_top = cr + 1 - vis_h;
                    }
                }
            }

            // ── RenamingWorkspace ────────────────────────────────────────────
            Focus::RenamingWorkspace(mut input) => match key.code {
                KeyCode::Enter => {
                    let name = if input.trim().is_empty() {
                        format!("WS {}", self.active_workspace + 1)
                    } else {
                        input
                    };
                    if let Some(slot) = self.workspace_names.get_mut(self.active_workspace as usize) {
                        *slot = name;
                    }
                    self.focus = self.focus_for_active_workspace();
                }
                KeyCode::Esc => { self.focus = self.focus_for_active_workspace(); }
                KeyCode::Backspace => {
                    input.pop();
                    self.focus = Focus::RenamingWorkspace(input);
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    self.focus = Focus::RenamingWorkspace(input);
                }
                _ => { self.focus = Focus::RenamingWorkspace(input); }
            },

            // ── NamingNotebook (safety fallback) ────────────────────────────
            Focus::NamingNotebook(_, _) => {
                self.focus = self.focus_for_active_workspace();
            }

            // ── Logging setup ────────────────────────────────────────────────
            Focus::LoggingSetup(idx, mut input) => match key.code {
                KeyCode::Enter => {
                    let path_str = if input.trim().is_empty() {
                        self.default_log_path(idx)
                    } else {
                        input.clone()
                    };
                    let path = std::path::PathBuf::from(&path_str);
                    // Ensure the logs directory exists.
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&path)
                    {
                        Ok(file) => {
                            if let NoteKind::Shell { log_file, log_path, .. } =
                                &mut self.notes[idx].kind
                            {
                                *log_file = Some(std::io::BufWriter::new(file));
                                *log_path = Some(path);
                            }
                        }
                        Err(_) => {} // silently stay un-logged on file-open failure
                    }
                    self.focus = if self.notes[idx].data.is_background {
                        Focus::BackgroundShell(idx)
                    } else {
                        self.note_focus(idx)
                    };
                }
                KeyCode::Esc => {
                    self.focus = if self.notes[idx].data.is_background {
                        Focus::BackgroundShell(idx)
                    } else {
                        self.note_focus(idx)
                    };
                }
                KeyCode::Backspace => {
                    input.pop();
                    self.focus = Focus::LoggingSetup(idx, input);
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    self.focus = Focus::LoggingSetup(idx, input);
                }
                _ => { self.focus = Focus::LoggingSetup(idx, input); }
            },

            // ── Rename mode ─────────────────────────────────────────────────
            Focus::Renaming(idx, mut input) => match key.code {
                KeyCode::Enter => {
                    if !input.trim().is_empty() {
                        self.notes[idx].data.title = input;
                    }
                    self.focus = self.note_focus(idx);
                }
                KeyCode::Esc => { self.focus = self.note_focus(idx); }
                KeyCode::Backspace => {
                    input.pop();
                    self.focus = Focus::Renaming(idx, input);
                }
                KeyCode::Char(c) => {
                    input.push(c);
                    self.focus = Focus::Renaming(idx, input);
                }
                _ => { self.focus = Focus::Renaming(idx, input); }
            },
        }

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Bracketed-paste
    // -----------------------------------------------------------------------

    pub(crate) fn handle_paste_event(&mut self, text: String) -> Result<()> {
        enum Target { BgShell(usize), TextNote(usize), ShellNote(usize), None }
        let target = match &self.focus {
            Focus::BackgroundShell(i)               => Target::BgShell(*i),
            Focus::Note(i, NoteType::Text)      => Target::TextNote(*i),
            Focus::Note(i, NoteType::Shell)     => Target::ShellNote(*i),
            _                                       => Target::None,
        };
        match target {
            Target::BgShell(idx) => {
                if let NoteKind::Shell { pty, .. } = &mut self.notes[idx].kind {
                    pty.write_bytes(text.as_bytes())?;
                }
            }
            Target::TextNote(idx) => {
                if let NoteKind::Text(ta, _) = &mut self.notes[idx].kind {
                    ta.set_yank_text(text);
                    ta.input(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL));
                }
            }
            Target::ShellNote(idx) => {
                if let NoteKind::Shell { pty, .. } = &mut self.notes[idx].kind {
                    pty.write_bytes(text.as_bytes())?;
                }
            }
            Target::None => {}
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Mouse
    // -----------------------------------------------------------------------

    pub(crate) fn handle_mouse(&mut self, event: MouseEvent) -> Result<()> {
        if self.corkboard_open { return Ok(()); }
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Tab bar click: switch workspace when bar is visible (workspace_count > 1).
                if self.workspace_count > 1 && event.row == 0 {
                    let tab_w = (self.term_size.0 / self.workspace_count as u16).max(1);
                    let ws = (event.column / tab_w).min(self.workspace_count as u16 - 1) as u8;
                    self.switch_workspace(ws);
                    return Ok(());
                }
                self.text_selection = None;
                self.shell_note_selection = None;
                self.text_note_selection = None;
                if let Some(idx) = self.note_at(event.column, event.row) {
                    let is_shell = self.notes[idx].data.is_shell;
                    let in_content = event.column > self.notes[idx].data.x
                        && event.column < self.notes[idx].data.x + self.notes[idx].data.width - 1
                        && event.row    > self.notes[idx].data.y
                        && event.row    < self.notes[idx].data.y + self.notes[idx].data.height - 1;
                    let alt = event.modifiers.contains(KeyModifiers::ALT);
                    let in_alt_screen = is_shell && in_content && !alt && {
                        if let NoteKind::Shell { parser, .. } = &self.notes[idx].kind {
                            parser.screen().alternate_screen()
                        } else { false }
                    };

                    if is_shell && in_content && alt {
                        let idx = self.bring_to_front(idx);
                        self.focus = self.note_focus(idx);
                        self.drag = Some(DragMode::ShellSelecting {
                            note_idx: idx,
                            start_col: event.column,
                            start_row: event.row,
                            cur_col:   event.column,
                            cur_row:   event.row,
                        });
                    } else if !is_shell && in_content && alt && {
                        matches!(self.notes[idx].kind, NoteKind::Text(..))
                    } {
                        let idx = self.bring_to_front(idx);
                        self.focus = self.note_focus(idx);
                        let inner_x = self.notes[idx].data.x + 1;
                        let inner_y = self.notes[idx].data.y + 1;
                        let sc = event.column.saturating_sub(inner_x);
                        let sr = event.row.saturating_sub(inner_y);
                        self.drag = Some(DragMode::TextSelecting {
                            note_idx: idx,
                            start_col: sc, start_row: sr,
                            cur_col:   sc, cur_row:   sr,
                        });
                    } else if in_alt_screen {
                        let idx = self.bring_to_front(idx);
                        self.focus = self.note_focus(idx);
                        self.forward_mouse_to_shell_note(idx, event)?;
                        self.drag = Some(DragMode::ShellPassthrough { note_idx: idx });
                    } else {
                        let idx = self.bring_to_front(idx);
                        let note = &self.notes[idx];
                        self.drag = Some(DragMode::Moving {
                            note_idx: idx,
                            offset_x: event.column.saturating_sub(note.data.x),
                            offset_y: event.row.saturating_sub(note.data.y),
                        });
                        self.focus = self.note_focus(idx);
                    }
                } else if let Some(bg_idx) = self.background_note_idx() {
                    // Route to the active workspace's background shell note.
                    let in_alt = if let NoteKind::Shell { parser, .. } = &self.notes[bg_idx].kind {
                        parser.screen().alternate_screen()
                    } else { false };
                    if in_alt {
                        self.forward_mouse_to_shell_note(bg_idx, event)?;
                        self.drag = Some(DragMode::ShellPassthrough { note_idx: bg_idx });
                    } else {
                        self.drag = Some(DragMode::Selecting {
                            start_col: event.column,
                            start_row: event.row,
                            cur_col:   event.column,
                            cur_row:   event.row,
                        });
                    }
                    if !matches!(self.focus, Focus::Selecting { .. }) {
                        self.focus = Focus::BackgroundShell(bg_idx);
                    }
                }
            }

            MouseEventKind::Down(MouseButton::Right)
                if event.modifiers.contains(KeyModifiers::ALT) =>
            {
                if let Some(idx) = self.note_at(event.column, event.row) {
                    let idx = self.bring_to_front(idx);
                    let note = &self.notes[idx];
                    self.drag = Some(DragMode::Resizing {
                        note_idx: idx,
                        anchor_col: event.column,
                        anchor_row: event.row,
                        orig_w: note.data.width,
                        orig_h: note.data.height,
                        shell_cleared: false,
                    });
                    self.focus = self.note_focus(idx);
                }
            }

            MouseEventKind::Down(MouseButton::Right | MouseButton::Middle)
                if !event.modifiers.contains(KeyModifiers::ALT) =>
            {
                if let Some(idx) = self.note_at(event.column, event.row) {
                    let in_content = event.column > self.notes[idx].data.x
                        && event.column < self.notes[idx].data.x + self.notes[idx].data.width - 1
                        && event.row    > self.notes[idx].data.y
                        && event.row    < self.notes[idx].data.y + self.notes[idx].data.height - 1;
                    if in_content && self.notes[idx].data.is_shell {
                        let in_alt = if let NoteKind::Shell { parser, .. } = &self.notes[idx].kind {
                            parser.screen().alternate_screen()
                        } else { false };
                        if in_alt {
                            self.forward_mouse_to_shell_note(idx, event)?;
                        }
                    }
                }
            }

            MouseEventKind::Drag(MouseButton::Left) => {
                let passthrough_idx = if let Some(DragMode::ShellPassthrough { note_idx }) = &self.drag {
                    Some(*note_idx)
                } else { None };
                if let Some(nidx) = passthrough_idx {
                    self.forward_mouse_to_shell_note(nidx, event)?;
                    return Ok(());
                }
                match &mut self.drag {
                    Some(DragMode::Moving { note_idx, offset_x, offset_y }) => {
                        let (nidx, ox, oy) = (*note_idx, *offset_x, *offset_y);
                        let note = &mut self.notes[nidx];
                        note.data.x = event.column.saturating_sub(ox);
                        note.data.y = event.row.saturating_sub(oy);
                        self.clamp_note(nidx);
                    }
                    Some(DragMode::Selecting { cur_col, cur_row, .. }) => {
                        *cur_col = event.column;
                        *cur_row = event.row;
                    }
                    Some(DragMode::ShellSelecting { note_idx, cur_col, cur_row, .. }) => {
                        let nidx = *note_idx;
                        let x_min = self.notes[nidx].data.x + 1;
                        let x_max = (self.notes[nidx].data.x + self.notes[nidx].data.width)
                            .saturating_sub(2);
                        let y_min = self.notes[nidx].data.y + 1;
                        let y_max = (self.notes[nidx].data.y + self.notes[nidx].data.height)
                            .saturating_sub(2);
                        *cur_col = event.column.clamp(x_min, x_max);
                        *cur_row = event.row.clamp(y_min, y_max);
                    }
                    Some(DragMode::TextSelecting { note_idx, cur_col, cur_row, .. }) => {
                        let nidx = *note_idx;
                        let inner_x = self.notes[nidx].data.x + 1;
                        let inner_y = self.notes[nidx].data.y + 1;
                        let inner_w = self.notes[nidx].data.width.saturating_sub(2);
                        let inner_h = self.notes[nidx].data.height.saturating_sub(2);
                        *cur_col = event.column.saturating_sub(inner_x).min(inner_w.saturating_sub(1));
                        *cur_row = event.row.saturating_sub(inner_y).min(inner_h.saturating_sub(1));
                    }
                    _ => {}
                }
            }

            MouseEventKind::Drag(MouseButton::Right) => {
                if let Some(DragMode::Resizing {
                    note_idx, anchor_col, anchor_row, orig_w, orig_h, ..
                }) = self.drag
                {
                    let dw = event.column as i32 - anchor_col as i32;
                    let dh = event.row    as i32 - anchor_row  as i32;
                    let note = &mut self.notes[note_idx];
                    note.data.width  = ((orig_w as i32 + dw).max(MIN_NOTE_W as i32)) as u16;
                    note.data.height = ((orig_h as i32 + dh).max(MIN_NOTE_H as i32)) as u16;
                    self.clamp_note(note_idx);
                }
            }

            MouseEventKind::Up(_) => {
                let passthrough_idx = if let Some(DragMode::ShellPassthrough { note_idx }) = &self.drag {
                    Some(*note_idx)
                } else { None };
                if let Some(nidx) = passthrough_idx {
                    self.drag = None;
                    self.forward_mouse_to_shell_note(nidx, event)?;
                    return Ok(());
                }
                if let Some(DragMode::ShellSelecting {
                    note_idx, start_col, start_row, cur_col, cur_row,
                }) = self.drag {
                    self.drag = None;
                    if start_col != cur_col || start_row != cur_row {
                        let (sc, sr, ec, er) = if start_row < cur_row
                            || (start_row == cur_row && start_col <= cur_col)
                        {
                            (start_col, start_row, cur_col, cur_row)
                        } else {
                            (cur_col, cur_row, start_col, start_row)
                        };
                        let inner_x = self.notes[note_idx].data.x + 1;
                        let inner_y = self.notes[note_idx].data.y + 1;
                        self.copy_shell_note_stream_selection(
                            note_idx,
                            sc.saturating_sub(inner_x), sr.saturating_sub(inner_y),
                            ec.saturating_sub(inner_x), er.saturating_sub(inner_y),
                        );
                        self.shell_note_selection = Some((note_idx, sc, sr, ec, er));
                    }
                    return Ok(());
                }

                if let Some(DragMode::TextSelecting {
                    note_idx, start_col, start_row, cur_col, cur_row,
                }) = self.drag {
                    self.drag = None;
                    if start_col != cur_col || start_row != cur_row {
                        let (sc, sr, ec, er) = if start_row < cur_row
                            || (start_row == cur_row && start_col <= cur_col)
                        {
                            (start_col, start_row, cur_col, cur_row)
                        } else {
                            (cur_col, cur_row, start_col, start_row)
                        };
                        self.copy_text_note_content_selection(note_idx, sc, sr, ec, er);
                        self.text_note_selection = Some((note_idx, sc, sr, ec, er));
                    }
                    return Ok(());
                }

                if let Some(DragMode::Selecting { start_col, start_row, cur_col, cur_row }) = self.drag {
                    self.drag = None;
                    if let Focus::Selecting { from_bg_shell, .. } = self.focus {
                        if start_col != cur_col || start_row != cur_row {
                            self.create_photo_note(start_col, start_row, cur_col, cur_row, from_bg_shell);
                            self.focus = if let Some(idx) = from_bg_shell {
                                Focus::BackgroundShell(idx)
                            } else {
                                self.focus_for_active_workspace()
                            };
                        }
                    } else if let Focus::BackgroundShell(bg_idx) = self.focus {
                        // Drag on the background shell note: copy as a stream selection.
                        // Coordinates are screen-absolute; the bg note's parser rows
                        // start at BG_SHELL_INSET so subtract that offset.
                        if start_col != cur_col || start_row != cur_row {
                            let (sc, sr, ec, er) = if start_row < cur_row
                                || (start_row == cur_row && start_col <= cur_col)
                            {
                                (start_col, start_row, cur_col, cur_row)
                            } else {
                                (cur_col, cur_row, start_col, start_row)
                            };
                            self.copy_shell_note_stream_selection(
                                bg_idx,
                                sc, sr.saturating_sub(BG_SHELL_INSET),
                                ec, er.saturating_sub(BG_SHELL_INSET),
                            );
                            self.text_selection = Some((sc, sr, ec, er));
                        }
                    }
                    return Ok(());
                }
                let was_note_drag = matches!(
                    self.drag,
                    Some(DragMode::Moving { .. } | DragMode::Resizing { .. })
                );
                if let Some(DragMode::Resizing { note_idx, shell_cleared, .. }) = self.drag {
                    let note = &mut self.notes[note_idx];
                    let new_cols = note.data.width.saturating_sub(2).max(2);
                    let new_rows = note.data.height.saturating_sub(2).max(2);
                    if let NoteKind::Shell { pty, parser, rows, cols, scroll_offset, .. } =
                        &mut note.kind
                    {
                        let size_changed = new_cols != *cols || new_rows != *rows;
                        if size_changed {
                            parser.set_size(new_rows, new_cols);
                            *cols = new_cols;
                            *rows = new_rows;
                            *scroll_offset = 0;
                        }
                        if size_changed || shell_cleared {
                            let _ = pty.resize(*rows, *cols);
                        }
                    }
                }
                self.drag = None;
            }

            MouseEventKind::ScrollUp => {
                // Screenshot mode entered from a background shell note: scroll the bg
                // note's history, not the main PTY.  Must be checked first so it doesn't
                // fall into the generic `Focus::Selecting` arm below.
                if let Focus::Selecting { from_bg_shell: Some(bg_idx), .. } = self.focus {
                    let in_alt = if let NoteKind::Shell { parser, .. } = &self.notes[bg_idx].kind {
                        parser.screen().alternate_screen()
                    } else { false };
                    if !in_alt {
                        let scrolled = if let NoteKind::Shell { scroll_offset, own_scrollback, parser, .. } =
                            &mut self.notes[bg_idx].kind
                        {
                            let max_scroll = (own_scrollback.len() as i64)
                                .max(parser.screen().scrollback() as i64);
                            let before = *scroll_offset;
                            *scroll_offset = (*scroll_offset + 1).min(max_scroll);
                            *scroll_offset != before
                        } else { false };
                        let _ = scrolled;
                    }
                    return Ok(());
                }
                if let Focus::BackgroundShell(bg_idx) = self.focus {
                    let in_alt = if let NoteKind::Shell { parser, .. } = &self.notes[bg_idx].kind {
                        parser.screen().alternate_screen()
                    } else { false };
                    if in_alt {
                        self.forward_mouse_to_shell_note(bg_idx, event)?;
                        return Ok(());
                    }
                    let scrolled = if let NoteKind::Shell { scroll_offset, own_scrollback, parser, .. } =
                        &mut self.notes[bg_idx].kind
                    {
                        let max_scroll = (own_scrollback.len() as i64)
                            .max(parser.screen().scrollback() as i64);
                        let before = *scroll_offset;
                        *scroll_offset = (*scroll_offset + 1).min(max_scroll);
                        *scroll_offset != before
                    } else { false };
                    if scrolled {
                        let term_rows = self.term_size.1 as u16;
                        if let Some((_, ref mut sr, _, ref mut er)) = self.text_selection {
                            *sr = (*sr + 1).min(term_rows - 1);
                            *er = (*er + 1).min(term_rows - 1);
                        }
                    }
                    self.focus = Focus::BackgroundShell(bg_idx);
                    return Ok(());
                }
                if let Focus::Note(idx, NoteType::Shell) = &self.focus {
                    let idx = *idx;
                    let in_alt = if let NoteKind::Shell { parser, .. } = &self.notes[idx].kind {
                        parser.screen().alternate_screen()
                    } else { false };
                    if in_alt {
                        self.forward_mouse_to_shell_note(idx, event)?;
                        return Ok(());
                    }
                }
                match self.focus {
                    Focus::Note(idx, NoteType::Shell) => {
                        let scrolled = if let NoteKind::Shell { scroll_offset, parser, .. } =
                            &mut self.notes[idx].kind
                        {
                            if !parser.screen().alternate_screen() {
                                let before = *scroll_offset;
                                *scroll_offset += 1;
                                *scroll_offset != before
                            } else { false }
                        } else { false };
                        if scrolled {
                            let term_rows = self.term_size.1 as u16;
                            if let Some((sel_idx, _sc, sr, _ec, er)) = &mut self.shell_note_selection {
                                if *sel_idx == idx {
                                    *sr = (*sr + 1).min(term_rows - 1);
                                    *er = (*er + 1).min(term_rows - 1);
                                }
                            }
                        }
                    }
                    Focus::Note(idx, NoteType::Text) | Focus::Note(idx, NoteType::CheckList) => {
                        if let NoteKind::Text(_, scroll_top) | NoteKind::CheckList(_, scroll_top) =
                            &mut self.notes[idx].kind
                        {
                            *scroll_top = scroll_top.saturating_sub(1);
                        }
                    }
                    _ => {}
                }
            }

            MouseEventKind::ScrollDown => {
                // Screenshot mode from a background shell note: scroll the bg note,
                // not the main PTY.  Same early-exit pattern as ScrollUp above.
                if let Focus::Selecting { from_bg_shell: Some(bg_idx), .. } = self.focus {
                    let in_alt = if let NoteKind::Shell { parser, .. } = &self.notes[bg_idx].kind {
                        parser.screen().alternate_screen()
                    } else { false };
                    if !in_alt {
                        let scrolled = if let NoteKind::Shell { scroll_offset, rows, parser, .. } =
                            &mut self.notes[bg_idx].kind
                        {
                            let min = if parser.screen().alternate_screen() {
                                0
                            } else {
                                -(*rows as i64 - PROMPT_LINES)
                            };
                            let before = *scroll_offset;
                            *scroll_offset = (*scroll_offset - 1).max(min);
                            *scroll_offset != before
                        } else { false };
                        let _ = scrolled;
                    }
                    return Ok(());
                }
                if let Focus::BackgroundShell(bg_idx) = self.focus {
                    let in_alt = if let NoteKind::Shell { parser, .. } = &self.notes[bg_idx].kind {
                        parser.screen().alternate_screen()
                    } else { false };
                    if in_alt {
                        self.forward_mouse_to_shell_note(bg_idx, event)?;
                        return Ok(());
                    }
                    let scrolled = if let NoteKind::Shell { scroll_offset, rows, parser, .. } =
                        &mut self.notes[bg_idx].kind
                    {
                        let min = if parser.screen().alternate_screen() {
                            0
                        } else {
                            -(*rows as i64 - PROMPT_LINES)
                        };
                        let before = *scroll_offset;
                        *scroll_offset = (*scroll_offset - 1).max(min);
                        *scroll_offset != before
                    } else { false };
                    if scrolled {
                        if let Some((_, ref mut sr, _, ref mut er)) = self.text_selection {
                            *sr = sr.saturating_sub(1).max(BG_SHELL_INSET);
                            *er = er.saturating_sub(1).max(BG_SHELL_INSET);
                        }
                    }
                    self.focus = Focus::BackgroundShell(bg_idx);
                    return Ok(());
                }
                if let Focus::Note(idx, NoteType::Shell) = &self.focus {
                    let idx = *idx;
                    let in_alt = if let NoteKind::Shell { parser, .. } = &self.notes[idx].kind {
                        parser.screen().alternate_screen()
                    } else { false };
                    if in_alt {
                        self.forward_mouse_to_shell_note(idx, event)?;
                        return Ok(());
                    }
                }
                match self.focus {
                    Focus::Note(idx, NoteType::Shell) => {
                        let scrolled = if let NoteKind::Shell { scroll_offset, rows, parser, .. } =
                            &mut self.notes[idx].kind
                        {
                            let min = if parser.screen().alternate_screen() {
                                0
                            } else {
                                -(*rows as i64 - PROMPT_LINES)
                            };
                            let before = *scroll_offset;
                            *scroll_offset = (*scroll_offset - 1).max(min);
                            *scroll_offset != before
                        } else { false };
                        if scrolled {
                            let inner_y = self.notes[idx].data.y + 1;
                            if let Some((sel_idx, _sc, sr, _ec, er)) = &mut self.shell_note_selection {
                                if *sel_idx == idx {
                                    *sr = sr.saturating_sub(1).max(inner_y);
                                    *er = er.saturating_sub(1).max(inner_y);
                                }
                            }
                        }
                    }
                    Focus::Note(idx, NoteType::Text) | Focus::Note(idx, NoteType::CheckList) => {
                        if let NoteKind::Text(_, scroll_top) | NoteKind::CheckList(_, scroll_top) =
                            &mut self.notes[idx].kind
                        {
                            *scroll_top = scroll_top.saturating_add(1);
                        }
                    }
                    _ => {}
                }
            }

            _ => {}
        }

        Ok(())
    }

    fn forward_mouse_to_shell_note(&mut self, note_idx: usize, event: MouseEvent) -> Result<()> {
        let (mut btn, is_release): (u16, bool) = match event.kind {
            MouseEventKind::Down(MouseButton::Left)   => (0, false),
            MouseEventKind::Down(MouseButton::Middle) => (1, false),
            MouseEventKind::Down(MouseButton::Right)  => (2, false),
            MouseEventKind::Up(MouseButton::Left)     => (0, true),
            MouseEventKind::Up(MouseButton::Middle)   => (1, true),
            MouseEventKind::Up(MouseButton::Right)    => (2, true),
            MouseEventKind::Drag(MouseButton::Left)   => (32, false),
            MouseEventKind::Drag(MouseButton::Middle) => (33, false),
            MouseEventKind::Drag(MouseButton::Right)  => (34, false),
            MouseEventKind::ScrollUp                  => (64, false),
            MouseEventKind::ScrollDown                => (65, false),
            _ => return Ok(()),
        };
        if event.modifiers.contains(KeyModifiers::SHIFT)   { btn |= 4; }
        if event.modifiers.contains(KeyModifiers::ALT)     { btn |= 8; }
        if event.modifiers.contains(KeyModifiers::CONTROL) { btn |= 16; }
        let suffix = if is_release { 'm' } else { 'M' };
        let note_x = self.notes[note_idx].data.x;
        let note_y = self.notes[note_idx].data.y;
        let col = event.column.saturating_sub(note_x).max(1);
        let row = event.row.saturating_sub(note_y).max(1);
        let seq = format!("\x1b[<{btn};{col};{row}{suffix}");
        if let NoteKind::Shell { pty, .. } = &mut self.notes[note_idx].kind {
            pty.write_bytes(seq.as_bytes())?;
        }
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Resize
    // -----------------------------------------------------------------------

    pub(crate) fn handle_resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        let _ = (cols, rows); // term_size is already updated in the event handler
        // Clamp every note so none end up stranded off-screen after a resize.
        for i in 0..self.notes.len() {
            self.clamp_note(i);
        }
        Ok(())
    }

    // ── Notebook picker key handling ─────────────────────────────────────────

    pub(crate) fn handle_notebook_picker_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(ref picker) = self.notebook_picker else {
            return Ok(());
        };
        let mode = match &picker.mode {
            NotebookPickerMode::AssignToNotebook(idx) => NotebookPickerMode::AssignToNotebook(*idx),
            NotebookPickerMode::AddToNotebook(nb_id) => NotebookPickerMode::AddToNotebook(*nb_id),
        };
        let selected = picker.selected;

        match key.code {
            KeyCode::Esc => {
                self.notebook_picker = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(ref mut p) = self.notebook_picker {
                    p.selected = p.selected.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let list_len = match &mode {
                    NotebookPickerMode::AssignToNotebook(_) => self.notebooks.len(),
                    NotebookPickerMode::AddToNotebook(nb_id) => {
                        let nb_id = *nb_id;
                        self.notes
                            .iter()
                            .filter(|n| n.data.on_corkboard && n.data.notebook_id != Some(nb_id))
                            .count()
                    }
                };
                if let Some(ref mut p) = self.notebook_picker {
                    if list_len > 0 {
                        p.selected = (p.selected + 1).min(list_len - 1);
                    }
                }
            }
            KeyCode::Enter => {
                match mode {
                    NotebookPickerMode::AssignToNotebook(note_idx) => {
                        if selected < self.notebooks.len() {
                            let nb_id = self.notebooks[selected].id;
                            self.assign_to_notebook(note_idx, nb_id);
                            self.focus = self.focus_for_active_workspace();
                        }
                    }
                    NotebookPickerMode::AddToNotebook(nb_id) => {
                        let free: Vec<usize> = self.notes
                            .iter()
                            .enumerate()
                            .filter(|(_, n)| n.data.on_corkboard && n.data.notebook_id != Some(nb_id))
                            .map(|(i, _)| i)
                            .collect();
                        if selected < free.len() {
                            let note_idx = free[selected];
                            self.assign_to_notebook(note_idx, nb_id);
                        }
                    }
                }
                self.notebook_picker = None;
            }
            _ => {}
        }
        Ok(())
    }
}
