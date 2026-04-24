mod corkboard;
mod notes;
mod overlays;

use crate::{
    app::{App, DragMode, Focus, NoteType},
    constants::BG_SHELL_INSET,
    note::NoteKind,
    terminal::{OwnScrollbackView, PtyView, SelectionOverlay},
};
use ratatui::style::Color;
use ratatui::{
    Frame,
    layout::Rect,
    layout::Position,
    style::Style,
    widgets::Block,
};

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // --- Corkboard overlay: takes over the full screen ---
    if app.corkboard_open {
        corkboard::render_corkboard(frame, app, area);
        // Notebook picker can be triggered from the corkboard — render it on top.
        if app.notebook_picker.is_some() {
            overlays::render_notebook_picker(frame, app, area);
        }
        return;
    }

    // --- Background: fill with detected app colour so empty cells match the theme ---
    // Use the background note's detected colour when one is active.
    let bg_fill_color: Option<Color> = if let Some(bg_idx) = app.background_note_idx() {
        if let NoteKind::Shell { detected_bg, .. } = &app.notes[bg_idx].kind {
            *detected_bg
        } else { app.detected_bg }
    } else { app.detected_bg };

    if let Some(bg) = bg_fill_color {
        frame.render_widget(
            Block::default().style(Style::default().bg(bg)),
            area,
        );
    }

    // --- Background: live shell session or active workspace's background note ---
    // BG_SHELL_INSET rows are reserved at the top (workspace tab bar) and bottom
    // (hint bar).  Both background shell types render into this smaller area so
    // the bars are never painted over by the PTY output.
    let shell_area = Rect::new(
        area.x,
        area.y + BG_SHELL_INSET,
        area.width,
        area.height.saturating_sub(2 * BG_SHELL_INSET),
    );

    // When the active workspace has a background shell note, render it instead of App.pty.
    if let Some(bg_idx) = app.background_note_idx() {
        if let NoteKind::Shell { parser, scroll_offset, own_scrollback, .. } =
            &app.notes[bg_idx].kind
        {
            let vt100_depth = parser.screen().scrollback() as i64;
            let own_sb_rows_needed = (*scroll_offset - vt100_depth).max(0);
            if own_sb_rows_needed > 0 && !own_scrollback.is_empty() {
                let own_sb_rows = own_sb_rows_needed
                    .min(own_scrollback.len() as i64)
                    .min(shell_area.height as i64) as u16;
                let top_idx = own_scrollback.len().saturating_sub(*scroll_offset as usize);
                let own_sb_area = Rect::new(shell_area.x, shell_area.y, shell_area.width, own_sb_rows);
                frame.render_widget(
                    OwnScrollbackView { rows: own_scrollback, top_idx },
                    own_sb_area,
                );
                let pty_rows = shell_area.height.saturating_sub(own_sb_rows);
                if pty_rows > 0 {
                    let pty_area = Rect::new(shell_area.x, shell_area.y + own_sb_rows, shell_area.width, pty_rows);
                    frame.render_widget(PtyView(parser.screen(), 0), pty_area);
                }
            } else {
                let row_offset = (*scroll_offset).min(0).unsigned_abs() as usize;
                frame.render_widget(PtyView(parser.screen(), row_offset), shell_area);
            }
        }
    } else {
        // Mirrors the shell-note split-render logic:
        //   scroll_offset > 0  →  history (own_scrollback rows on top, PtyView below)
        //   scroll_offset = 0  →  live view
        //   scroll_offset < 0  →  blank rows below prompt (Alacritty-style)
        let vt100_depth = app.parser.screen().scrollback() as i64;
        let own_sb_rows_needed = (app.scroll_offset - vt100_depth).max(0);

        if own_sb_rows_needed > 0 && !app.own_scrollback.is_empty() {
            let own_sb_rows = own_sb_rows_needed
                .min(app.own_scrollback.len() as i64)
                .min(shell_area.height as i64) as u16;
            let top_idx = app.own_scrollback
                .len()
                .saturating_sub(app.scroll_offset as usize);
            let own_sb_area = Rect::new(shell_area.x, shell_area.y, shell_area.width, own_sb_rows);
            frame.render_widget(
                OwnScrollbackView { rows: &app.own_scrollback, top_idx },
                own_sb_area,
            );
            let pty_rows = shell_area.height.saturating_sub(own_sb_rows);
            if pty_rows > 0 {
                let pty_area = Rect::new(shell_area.x, shell_area.y + own_sb_rows, shell_area.width, pty_rows);
                frame.render_widget(PtyView(app.parser.screen(), 0), pty_area);
            }
        } else {
            let row_offset = app.scroll_offset.min(0).unsigned_abs() as usize;
            frame.render_widget(PtyView(app.parser.screen(), row_offset), shell_area);
        }
    }

    // --- Selection overlay ---
    {
        // Screenshot mode drag and keyboard visual-block: rectangular highlight.
        let rect_sel = match (&app.drag, &app.focus) {
            (Some(DragMode::Selecting { start_col, start_row, cur_col, cur_row }),
             Focus::Selecting { .. }) =>
                Some((*start_col, *start_row, *cur_col, *cur_row)),
            (_, Focus::Selecting { anchor_col, anchor_row, cursor_col, cursor_row }) =>
                Some((*anchor_col, *anchor_row, *cursor_col, *cursor_row)),
            _ => None,
        };
        if let Some((c1, r1, c2, r2)) = rect_sel {
            let x = area.x + c1.min(c2);
            let y = area.y + r1.min(r2);
            let w = c1.abs_diff(c2) + 1;
            let h = r1.abs_diff(r2) + 1;
            frame.render_widget(SelectionOverlay, Rect::new(x, y, w, h).intersection(area));
        }

        // Normal text-selection drag (in progress) or persistent selection:
        // stream highlight — partial first line, full middle lines, partial last line.
        let stream_sel: Option<(u16, u16, u16, u16)> = match &app.drag {
            Some(DragMode::Selecting { start_col, start_row, cur_col, cur_row })
                if !matches!(app.focus, Focus::Selecting { .. }) =>
            {
                // Normalise into text order on the fly for live feedback.
                if start_row < cur_row || (start_row == cur_row && start_col <= cur_col) {
                    Some((*start_col, *start_row, *cur_col, *cur_row))
                } else {
                    Some((*cur_col, *cur_row, *start_col, *start_row))
                }
            }
            _ => app.text_selection,
        };
        if let Some((sc, sr, ec, er)) = stream_sel {
            render_stream_sel(frame, area, sc, sr, ec, er);
        }
    }

    // --- Floating notes (skip those on the corkboard) ---
    notes::render_notes(frame, app, area);

    // --- Workspace tab bar (always visible, drawn after notes on row 0) ---
    overlays::render_workspace_bar(frame, app, area);

    // --- Cursor: follow the active input ---
    let mut cursor_set = false;

    // BackgroundShell cursor: position it on the bg note's virtual cursor.
    if let Focus::BackgroundShell(bg_idx) = app.focus {
        cursor_set = true;
        if let Some(note) = app.notes.get(bg_idx) {
            if let NoteKind::Shell { parser, scroll_offset, .. } = &note.kind {
                if *scroll_offset <= 0 {
                    let row_offset = (*scroll_offset).min(0).unsigned_abs() as usize;
                    let (crow, ccol) = parser.screen().cursor_position();
                    if crow as usize >= row_offset {
                        let visible_row = (crow as usize - row_offset) as u16;
                        frame.set_cursor_position((shell_area.x + ccol, shell_area.y + visible_row));
                    }
                }
            }
        }
    }

    if let Focus::Note(idx, NoteType::Shell) = app.focus {
        if let Some(note) = app.notes.get(idx) {
            if let NoteKind::Shell { parser, scroll_offset, .. } = &note.kind {
                cursor_set = true; // always suppress background cursor for shell notes
                // Only show cursor when at or below the live view (not scrolled into history).
                if *scroll_offset <= 0 {
                    let row_offset = (*scroll_offset).min(0).unsigned_abs() as usize;
                    let (crow, ccol) = parser.screen().cursor_position();
                    if crow as usize >= row_offset {
                        let visible_row = (crow as usize - row_offset) as u16;
                        let note_area = clamp_rect(
                            Rect::new(note.data.x, note.data.y, note.data.width, note.data.height),
                            area,
                        );
                        frame.set_cursor_position((note_area.x + 1 + ccol, note_area.y + 1 + visible_row));
                    }
                }
            }
        }
    }
    // Checklist cursor: place the terminal cursor at the exact cell so the user
    // can see their position within the row highlight.
    if let Focus::Note(idx, NoteType::CheckList) = app.focus {
        cursor_set = true; // suppress background terminal cursor
        if let Some(note) = app.notes.get(idx) {
            if let NoteKind::CheckList(ta, scroll_top) = &note.kind {
                let (crow, ccol) = ta.cursor();
                // The renderer prepends "[ ] " (4 chars) to lines that don't already
                // carry a checkbox prefix, so we must offset the cursor column by 4
                // in that case to land on the right display cell.
                let raw_line = ta.lines().get(crow).map(|l| l.as_str()).unwrap_or("");
                let prefix_offset: u16 =
                    if !raw_line.starts_with("[ ] ") && !raw_line.starts_with("[x] ") && !raw_line.is_empty() {
                        4
                    } else {
                        0
                    };
                // Content starts 1 cell inside the note boundary for both bordered
                // and borderless layouts.
                let note_area = clamp_rect(
                    Rect::new(note.data.x, note.data.y, note.data.width, note.data.height),
                    area,
                );
                /*let cx = note_area.x + 1 + ccol as u16 + prefix_offset;
                // Subtract scroll_top so the terminal cursor tracks the visible row.
                let visible_row = crow.saturating_sub(*scroll_top) as u16;
                let cy = note_area.y + 1 + visible_row;
                if cx < note_area.x + note_area.width.saturating_sub(1)
                    && cy < note_area.y + note_area.height.saturating_sub(1)
                {

                    if let Some(cell) = frame.buffer_mut().cell_mut(Position::new(cx, cy)) {
                        if cell.symbol() != " " || cx < note_area.width + note.data.x - 10 {
                            frame.set_cursor_position((cx, cy)); 
                        } else {
                            //let diff = cx - note_area.width - note.data.x;
                            frame.set_cursorsd_position((cx - note_area.width, cy + 1));
                        }
                    }
                }*/
            }
        }
    }

    // Suppress terminal cursor for text notes in wrap mode (inline block cursor is rendered instead).
    if let Focus::Note(idx, NoteType::Text) = app.focus {
        if let Some(note) = app.notes.get(idx) {
            if note.data.text_wrap {
                cursor_set = true;
            }
        }
    }
    // Suppress terminal cursor in visual mode (selection overlay is shown instead).
    if matches!(app.focus, Focus::TextVisual { .. }) {
        cursor_set = true;
    }

    // Suppress terminal cursor while a selection is being made.
    if matches!(app.focus, Focus::Selecting { .. })
        || matches!(app.drag, Some(DragMode::Selecting { .. }))
        || matches!(app.drag, Some(DragMode::ShellSelecting { .. }))
    {
        cursor_set = true;
    }

    if !cursor_set && app.scroll_offset <= 0 {
        let row_offset = app.scroll_offset.min(0).unsigned_abs() as usize;
        let (crow, ccol) = app.parser.screen().cursor_position();
        if crow as usize >= row_offset {
            let visible_row = (crow as usize - row_offset) as u16;
            frame.set_cursor_position((shell_area.x + ccol, shell_area.y + visible_row));
        }
    }

    // --- Settings popup ---
    if matches!(app.focus, Focus::Settings(_, _)) {
        overlays::render_settings_popup(frame, app, area);
    }

    // --- Notebook picker overlay ---
    if app.notebook_picker.is_some() {
        overlays::render_notebook_picker(frame, app, area);
    }

    // --- Status bar hint ---
    if app.show_hints {
        overlays::render_hint(frame, app, area);
    }

    // --- Splash screen (drawn last so it sits on top of everything) ---
    if app.splash {
        overlays::render_splash(frame, area);
    }
}

// ---------------------------------------------------------------------------
// Shared geometry helpers (accessible to child modules via `super::`)
// ---------------------------------------------------------------------------

/// Clamp a requested rect so it never extends beyond the terminal boundary.
fn clamp_rect(r: Rect, bounds: Rect) -> Rect {
    let x = r.x.min(bounds.width.saturating_sub(r.width));
    let y = r.y.min(bounds.height.saturating_sub(r.height));
    let w = r.width.min(bounds.width);
    let h = r.height.min(bounds.height);
    Rect::new(x, y, w, h)
}

/// Render a stream (text-flow) selection overlay.
// pub(super) so ui/notes.rs can call it for shell-note selections.
/// `(sc, sr)` is the start and `(ec, er)` the end, already in text order.
/// Draws up to three rectangles: partial first line, full middle lines,
/// partial last line — matching how terminal emulators highlight selections.
pub(super) fn render_stream_sel(frame: &mut Frame, area: Rect, sc: u16, sr: u16, ec: u16, er: u16) {
    if sr == er {
        // Single row: highlight only the selected columns.
        let x = area.x + sc;
        let w = ec.saturating_sub(sc) + 1;
        frame.render_widget(SelectionOverlay, Rect::new(x, area.y + sr, w, 1).intersection(area));
    } else {
        // First line: from sc to the right edge.
        let w_first = area.width.saturating_sub(sc);
        frame.render_widget(
            SelectionOverlay,
            Rect::new(area.x + sc, area.y + sr, w_first, 1).intersection(area),
        );
        // Middle lines (if any): full width.
        if er > sr + 1 {
            frame.render_widget(
                SelectionOverlay,
                Rect::new(area.x, area.y + sr + 1, area.width, er - sr - 1).intersection(area),
            );
        }
        // Last line: from the left edge to ec.
        frame.render_widget(
            SelectionOverlay,
            Rect::new(area.x, area.y + er, ec + 1, 1).intersection(area),
        );
    }
}

/// Centre a rect of the given size within `area`.
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}
