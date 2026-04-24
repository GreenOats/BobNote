//! Corkboard input handling for `App`.
//!
//! These `impl App` methods manage keyboard input when the corkboard overlay is
//! open, including grid navigation, shell expansion, notebook creation/navigation,
//! and picking notes back up to the main view.

use crate::{
    app::{App, CorkItem, Focus, NotebookPicker, NotebookPickerMode},
    constants::{CARD_GAP, CARD_W, PROMPT_LINES},
    note::NoteKind,
    notebook::NotebookData,
    terminal::capture_screen_before_resize,
};
use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    /// Handle all keypresses while the corkboard grid is open.
    pub(crate) fn handle_corkboard_key(&mut self, key: KeyEvent) -> Result<()> {
        // ── Notebook naming prompt ───────────────────────────────────────────
        if let Focus::NamingNotebook(nb_id, ref input) = self.focus {
            let nb_id = nb_id;
            let input = input.clone();
            return self.handle_notebook_naming_key(key, nb_id, input);
        }

        // ── Note renaming prompt (shared by main grid and notebook sub-grid) ─
        if let Focus::Renaming(note_idx, ref input) = self.focus {
            let note_idx = note_idx;
            let input = input.clone();
            return self.handle_corkboard_renaming_key(key, note_idx, input);
        }

        // ── Notebook page sub-grid ───────────────────────────────────────────
        if let Some(nb_id) = self.corkboard_notebook {
            return self.handle_corkboard_notebook_key(key, nb_id);
        }

        // ── Trash sub-grid ───────────────────────────────────────────────────
        if self.corkboard_trash_open {
            return self.handle_trash_key(key);
        }

        // ── Main corkboard grid ──────────────────────────────────────────────
        let items = self.corkboard_items();
        let item_count = items.len();
        if item_count > 0 {
            self.corkboard_selected = self.corkboard_selected.min(item_count - 1);
        }

        let cols = ((self.term_size.0 + CARD_GAP) / (CARD_W + CARD_GAP)).max(1) as usize;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            // Close corkboard
            KeyCode::Esc => {
                self.corkboard_open = false;
            }
            KeyCode::Char('k') if ctrl => {
                self.corkboard_open = false;
            }

            // Navigate
            KeyCode::Right | KeyCode::Char('l') if item_count > 0 => {
                self.corkboard_selected = (self.corkboard_selected + 1).min(item_count - 1);
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.corkboard_selected = self.corkboard_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') if item_count > 0 => {
                self.corkboard_selected = (self.corkboard_selected + cols).min(item_count - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.corkboard_selected = self.corkboard_selected.saturating_sub(cols);
            }

            // Delete selected item (note or notebook)
            KeyCode::Char('w') if ctrl && item_count > 0 => {
                let sel = self.corkboard_selected.min(item_count - 1);
                match items[sel] {
                    CorkItem::Note(note_idx) => {
                        self.detach_from_notebook(note_idx);
                        let note = self.notes.remove(note_idx);
                        self.trash_note(note);
                        if self.corkboard_selected > 0
                            && self.corkboard_selected >= item_count - 1
                        {
                            self.corkboard_selected -= 1;
                        }
                    }
                    CorkItem::Notebook(nb_id) => {
                        // Return all notebook notes to the corkboard as regular notes.
                        for n in self.notes.iter_mut() {
                            if n.data.notebook_id == Some(nb_id) {
                                n.data.notebook_id = None;
                                // leave on_corkboard = true so they stay on the board
                            }
                        }
                        self.notebooks.retain(|nb| nb.id != nb_id);
                        if self.corkboard_selected > 0
                            && self.corkboard_selected >= item_count - 1
                        {
                            self.corkboard_selected -= 1;
                        }
                    }
                    CorkItem::Trash => { /* cannot delete the trash bin */ }
                }
            }

            // Create a new notebook (prompts for title)
            KeyCode::Char('n') if !ctrl => {
                let nb_id = self.next_notebook_id;
                self.next_notebook_id += 1;
                self.notebooks.push(NotebookData {
                    id: nb_id,
                    title: String::new(),
                    note_ids: Vec::new(),
                    persistent: false,
                });
                self.focus = Focus::NamingNotebook(nb_id, String::new());
            }

            // Rename selected note
            KeyCode::Char('t') if !ctrl && item_count > 0 => {
                let sel = self.corkboard_selected.min(item_count - 1);
                if let CorkItem::Note(note_idx) = items[sel] {
                    let current = self.notes[note_idx].data.title.clone();
                    self.focus = Focus::Renaming(note_idx, current);
                }
            }

            // Assign selected note to a notebook
            KeyCode::Char('a') if !ctrl && item_count > 0 => {
                let sel = self.corkboard_selected.min(item_count - 1);
                if let CorkItem::Note(note_idx) = items[sel] {
                    if !self.notebooks.is_empty() {
                        self.notebook_picker = Some(NotebookPicker {
                            selected: 0,
                            mode: NotebookPickerMode::AssignToNotebook(note_idx),
                        });
                    }
                }
            }

            // Enter: open shell note expanded / pick up text note / open notebook folder
            KeyCode::Enter if item_count > 0 => {
                let sel = self.corkboard_selected.min(item_count - 1);
                match items[sel] {
                    CorkItem::Note(note_idx) => {
                        if matches!(self.notes[note_idx].kind, NoteKind::Shell { .. }) {
                            let (rows, cols) = self.corkboard_shell_size();
                            self.resize_note_pty(note_idx, rows, cols);
                            self.corkboard_expanded = Some(note_idx);
                        } else {
                            self.pickup_from_corkboard_idx(note_idx);
                        }
                    }
                    CorkItem::Notebook(nb_id) => {
                        self.corkboard_notebook = Some(nb_id);
                        self.corkboard_nb_selected = 0;
                    }
                    CorkItem::Trash => {
                        self.corkboard_trash_open = true;
                    }
                }
            }

            _ => {}
        }
        Ok(())
    }

    // ── Notebook naming prompt ───────────────────────────────────────────────

    fn handle_notebook_naming_key(
        &mut self,
        key: KeyEvent,
        nb_id: u64,
        mut input: String,
    ) -> Result<()> {
        match key.code {
            KeyCode::Enter => {
                let title = input.trim().to_string();
                if title.is_empty() {
                    // User pressed Enter with nothing typed — remove the empty notebook.
                    self.notebooks.retain(|nb| nb.id != nb_id);
                } else if let Some(nb) = self.notebooks.iter_mut().find(|nb| nb.id == nb_id) {
                    nb.title = title;
                }
                self.focus = self.focus_for_active_workspace();
            }
            KeyCode::Esc => {
                // Cancel — remove the empty notebook that was pre-created.
                self.notebooks.retain(|nb| nb.id != nb_id);
                self.focus = self.focus_for_active_workspace();
            }
            KeyCode::Backspace => {
                input.pop();
                self.focus = Focus::NamingNotebook(nb_id, input);
            }
            KeyCode::Char(c) => {
                input.push(c);
                self.focus = Focus::NamingNotebook(nb_id, input);
            }
            _ => {
                self.focus = Focus::NamingNotebook(nb_id, input);
            }
        }
        Ok(())
    }

    // ── Notebook page sub-grid ───────────────────────────────────────────────

    fn handle_corkboard_notebook_key(&mut self, key: KeyEvent, nb_id: u64) -> Result<()> {
        let page_note_indices: Vec<usize> = {
            // Collect ordered page indices according to notebook.note_ids.
            let nb = match self.notebooks.iter().find(|nb| nb.id == nb_id) {
                Some(nb) => nb,
                None => {
                    self.corkboard_notebook = None;
                    return Ok(());
                }
            };
            nb.note_ids
                .iter()
                .filter_map(|&nid| self.notes.iter().position(|n| n.data.id == nid))
                .collect()
        };

        let page_count = page_note_indices.len();
        if page_count > 0 {
            self.corkboard_nb_selected = self.corkboard_nb_selected.min(page_count - 1);
        }

        let cols = ((self.term_size.0 + CARD_GAP) / (CARD_W + CARD_GAP)).max(1) as usize;
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => {
                self.corkboard_notebook = None;
            }

            // Ctrl+arrows: reorder the selected page within the notebook.
            // Swaps the selected page with its neighbour in the displayed grid order,
            // then moves the selection cursor to follow the card.
            KeyCode::Left if ctrl && page_count > 0 && self.corkboard_nb_selected > 0 => {
                self.swap_notebook_pages(nb_id, &page_note_indices,
                    self.corkboard_nb_selected, self.corkboard_nb_selected - 1);
                self.corkboard_nb_selected -= 1;
            }
            KeyCode::Right if ctrl && page_count > 0
                && self.corkboard_nb_selected + 1 < page_count =>
            {
                self.swap_notebook_pages(nb_id, &page_note_indices,
                    self.corkboard_nb_selected, self.corkboard_nb_selected + 1);
                self.corkboard_nb_selected += 1;
            }
            KeyCode::Up if ctrl && page_count > 0 && self.corkboard_nb_selected >= cols => {
                let new_pos = self.corkboard_nb_selected - cols;
                self.swap_notebook_pages(nb_id, &page_note_indices,
                    self.corkboard_nb_selected, new_pos);
                self.corkboard_nb_selected = new_pos;
            }
            KeyCode::Down if ctrl && page_count > 0
                && self.corkboard_nb_selected + cols < page_count =>
            {
                let new_pos = self.corkboard_nb_selected + cols;
                self.swap_notebook_pages(nb_id, &page_note_indices,
                    self.corkboard_nb_selected, new_pos);
                self.corkboard_nb_selected = new_pos;
            }

            // Plain arrows: navigate.
            KeyCode::Right | KeyCode::Char('l') if page_count > 0 => {
                self.corkboard_nb_selected =
                    (self.corkboard_nb_selected + 1).min(page_count - 1);
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.corkboard_nb_selected = self.corkboard_nb_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') if page_count > 0 => {
                self.corkboard_nb_selected =
                    (self.corkboard_nb_selected + cols).min(page_count - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.corkboard_nb_selected = self.corkboard_nb_selected.saturating_sub(cols);
            }

            // Enter: open selected page as book in the main view.
            // Inserts into the open map — a previously open page of the same
            // notebook is simply replaced; other notebooks stay open.
            KeyCode::Enter if page_count > 0 => {
                let note_idx = page_note_indices[self.corkboard_nb_selected];
                // Adopt the current workspace so the page is visible here.
                // (Non-persistent notebooks filter pages by workspace_id.)
                self.notes[note_idx].data.workspace_id = self.active_workspace;
                self.notebooks_open.insert(nb_id, self.corkboard_nb_selected);
                self.corkboard_open = false;
                self.corkboard_notebook = None;
                let note_idx = self.bring_to_front(note_idx);
                self.focus = self.note_focus(note_idx);
            }

            // 'r' or Ctrl+W: remove selected page from the notebook (returns it to corkboard grid).
            KeyCode::Char('r') if !ctrl && page_count > 0 => {
                self.remove_notebook_page(nb_id, page_note_indices[self.corkboard_nb_selected], page_count);
            }
            KeyCode::Char('w') if ctrl && page_count > 0 => {
                self.remove_notebook_page(nb_id, page_note_indices[self.corkboard_nb_selected], page_count);
            }

            // 't': rename the selected page note.
            KeyCode::Char('t') if !ctrl && page_count > 0 => {
                let note_idx = page_note_indices[self.corkboard_nb_selected];
                let current = self.notes[note_idx].data.title.clone();
                self.focus = Focus::Renaming(note_idx, current);
            }

            // 'a': open picker to add a free corkboard note to this notebook.
            KeyCode::Char('a') if !ctrl => {
                // Check if there are any free corkboard notes to add.
                let has_free = self
                    .notes
                    .iter()
                    .any(|n| n.data.on_corkboard && n.data.notebook_id.is_none());
                if has_free {
                    self.notebook_picker = Some(NotebookPicker {
                        selected: 0,
                        mode: NotebookPickerMode::AddToNotebook(nb_id),
                    });
                }
            }

            _ => {}
        }
        Ok(())
    }

    // ── Corkboard note rename prompt ─────────────────────────────────────────

    fn handle_corkboard_renaming_key(
        &mut self,
        key: KeyEvent,
        note_idx: usize,
        mut input: String,
    ) -> Result<()> {
        match key.code {
            KeyCode::Enter => {
                if !input.trim().is_empty() {
                    self.notes[note_idx].data.title = input;
                }
                self.focus = self.focus_for_active_workspace();
            }
            KeyCode::Esc => {
                self.focus = self.focus_for_active_workspace();
            }
            KeyCode::Backspace => {
                input.pop();
                self.focus = Focus::Renaming(note_idx, input);
            }
            KeyCode::Char(c) => {
                input.push(c);
                self.focus = Focus::Renaming(note_idx, input);
            }
            _ => {
                self.focus = Focus::Renaming(note_idx, input);
            }
        }
        Ok(())
    }

    // ── Shell expansion (unchanged) ──────────────────────────────────────────

    /// Handle keypresses while a shell note is expanded full-screen on the corkboard.
    pub(crate) fn handle_corkboard_expanded_key(&mut self, key: KeyEvent) -> Result<()> {
        let Some(idx) = self.corkboard_expanded else {
            return Ok(());
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match (ctrl, key.code) {
            (false, KeyCode::Esc) => {
                self.collapse_corkboard_shell(idx);
            }
            (true, KeyCode::Char('e')) => {
                self.collapse_corkboard_shell(idx);
                let items = self.corkboard_items();
                let sel = self.corkboard_selected.min(items.len().saturating_sub(1));
                if let Some(CorkItem::Note(note_idx)) = items.get(sel) {
                    self.pickup_from_corkboard_idx(*note_idx);
                }
            }
            (false, KeyCode::PageUp) => {
                if let NoteKind::Shell { scroll_offset, parser, .. } = &mut self.notes[idx].kind {
                    if !parser.screen().alternate_screen() {
                        *scroll_offset += 10;
                    }
                }
            }
            (false, KeyCode::PageDown) => {
                if let NoteKind::Shell {
                    scroll_offset, rows, parser, ..
                } = &mut self.notes[idx].kind
                {
                    let min = if parser.screen().alternate_screen() {
                        0
                    } else {
                        -(*rows as i64 - PROMPT_LINES)
                    };
                    *scroll_offset = (*scroll_offset - 10).max(min);
                }
            }
            _ => {
                if let NoteKind::Shell { pty, scroll_offset, .. } = &mut self.notes[idx].kind {
                    if *scroll_offset > 0 {
                        *scroll_offset = 0;
                    }
                    pty.write_key(key)?;
                }
            }
        }
        Ok(())
    }

    fn collapse_corkboard_shell(&mut self, idx: usize) {
        let rows = self.notes[idx].data.height.saturating_sub(2).max(2);
        let cols = self.notes[idx].data.width.saturating_sub(2).max(2);
        self.resize_note_pty(idx, rows, cols);
        self.corkboard_expanded = None;
    }

    fn resize_note_pty(&mut self, idx: usize, rows: u16, cols: u16) {
        if let NoteKind::Shell {
            pty,
            parser,
            rows: r,
            cols: c,
            scroll_offset,
            own_scrollback,
            sb_prev_fps,
            ..
        } = &mut self.notes[idx].kind
        {
            capture_screen_before_resize(parser, own_scrollback, sb_prev_fps, self.config.shell_scrollback);
            parser.set_size(rows, cols);
            let _ = pty.resize(rows, cols);
            *r = rows;
            *c = cols;
            *scroll_offset = 0;
        }
    }

    fn corkboard_shell_size(&self) -> (u16, u16) {
        let (tw, th) = self.term_size;
        let cols = tw.saturating_sub(8).max(2);
        let rows = th.saturating_sub(7).max(2);
        (rows, cols)
    }

    /// Swap two pages within a notebook's `note_ids` list.
    /// `a` and `b` are indices into `page_note_indices` (the displayed grid order).
    /// Uses note IDs rather than raw list positions so it works correctly even
    /// when some notes are absent from the display list.
    fn swap_notebook_pages(
        &mut self,
        nb_id: u64,
        page_note_indices: &[usize],
        a: usize,
        b: usize,
    ) {
        let Some(&ni_a) = page_note_indices.get(a) else { return };
        let Some(&ni_b) = page_note_indices.get(b) else { return };
        let id_a = self.notes[ni_a].data.id;
        let id_b = self.notes[ni_b].data.id;
        if let Some(nb) = self.notebooks.iter_mut().find(|nb| nb.id == nb_id) {
            let pos_a = nb.note_ids.iter().position(|&id| id == id_a);
            let pos_b = nb.note_ids.iter().position(|&id| id == id_b);
            if let (Some(pa), Some(pb)) = (pos_a, pos_b) {
                nb.note_ids.swap(pa, pb);
            }
        }
    }

    /// Remove a page from a notebook and return it to the free corkboard grid.
    /// `page_count` is the number of pages before removal, used to clamp the selection.
    fn remove_notebook_page(&mut self, nb_id: u64, note_idx: usize, page_count: usize) {
        let note_id = self.notes[note_idx].data.id;
        self.notes[note_idx].data.notebook_id = None;
        // Keep on_corkboard = true so the note reappears in the main grid.
        if let Some(nb) = self.notebooks.iter_mut().find(|nb| nb.id == nb_id) {
            nb.note_ids.retain(|&id| id != note_id);
        }
        if self.corkboard_nb_selected > 0 && self.corkboard_nb_selected >= page_count - 1 {
            self.corkboard_nb_selected -= 1;
        }
    }

    pub(crate) fn handle_trash_key(&mut self, key: KeyEvent) -> Result<()> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let count = self.trash.len();
        let cols = ((self.term_size.0 + CARD_GAP) / (CARD_W + CARD_GAP)).max(1) as usize;

        if count > 0 {
            self.corkboard_trash_selected = self.corkboard_trash_selected.min(count - 1);
        }
        let sel = self.corkboard_trash_selected;

        match key.code {
            KeyCode::Esc => { self.corkboard_trash_open = false; }

            KeyCode::Right | KeyCode::Char('l') if count > 0 => {
                self.corkboard_trash_selected = (sel + 1).min(count - 1);
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.corkboard_trash_selected = sel.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') if count > 0 => {
                self.corkboard_trash_selected = (sel + cols).min(count - 1);
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.corkboard_trash_selected = sel.saturating_sub(cols);
            }

            // Enter / 'r' — restore selected note to corkboard
            KeyCode::Enter | KeyCode::Char('r') if count > 0 => {
                self.restore_from_trash(sel)?;
                let new_count = self.trash.len();
                if new_count == 0 {
                    self.corkboard_trash_selected = 0;
                } else {
                    self.corkboard_trash_selected = sel.min(new_count - 1);
                }
            }

            // Ctrl+W — permanently delete selected note
            KeyCode::Char('w') if ctrl && count > 0 => {
                self.permanently_delete_trash(sel);
                let new_count = self.trash.len();
                if new_count == 0 {
                    self.corkboard_trash_selected = 0;
                } else {
                    self.corkboard_trash_selected = sel.min(new_count - 1);
                }
            }

            // Ctrl+X — empty entire trash
            KeyCode::Char('x') if ctrl => {
                self.empty_trash();
                self.corkboard_trash_selected = 0;
            }

            _ => {}
        }
        Ok(())
    }

    /// Move the note at `note_idx` back to the main view, centred.
    pub(crate) fn pickup_from_corkboard_idx(&mut self, note_idx: usize) {
        let (term_w, term_h) = self.term_size;
        {
            let note = &mut self.notes[note_idx];
            note.data.x = (term_w.saturating_sub(note.data.width)) / 2;
            note.data.y = (term_h.saturating_sub(note.data.height)) / 2;
            note.data.on_corkboard = false;
            note.data.notebook_id = None; // detach from notebook when picking up
            note.data.workspace_id = self.active_workspace; // adopt the current workspace
        }
        self.corkboard_open = false;
        let note_idx = self.bring_to_front(note_idx);
        self.focus = self.note_focus(note_idx);
    }
}
