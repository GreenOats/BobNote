//! Rendering for the floating note overlay.

use crate::{
    app::{App, DragMode, Focus, OcclusionDim},
    colors::{BG_PALETTE, BORDER_PALETTE, contrast_color},
    effects,
    note::NoteKind,
    terminal::{OwnScrollbackView, PhotoView, PtyView},
};

// ---------------------------------------------------------------------------
// Helpers for visual-selection coordinate mapping
// ---------------------------------------------------------------------------

/// Map a buffer (row, col) position to content-relative (visual_row, visual_col),
/// accounting for text wrapping. Returns `None` when the position is scrolled above
/// the visible area (`buf_row < scroll_top`).
fn buf_to_visual(
    buf_row: usize, buf_col: usize,
    lines: &[String], scroll_top: usize,
    inner_w: u16, text_wrap: bool,
) -> Option<(u16, u16)> {
    if buf_row < scroll_top { return None; }
    if !text_wrap || inner_w == 0 {
        return Some(((buf_row - scroll_top) as u16, buf_col as u16));
    }
    let iw = inner_w as usize;
    let mut vrow = 0u16;
    for (br, line) in lines.iter().enumerate().skip(scroll_top) {
        let cc = line.chars().count();
        if br == buf_row {
            let vr_within = (buf_col / iw) as u16;
            let vc = (buf_col % iw) as u16;
            return Some((vrow + vr_within, vc));
        }
        let line_vrows = if cc == 0 { 1 } else { ((cc + iw - 1) / iw) as u16 };
        vrow += line_vrows;
    }
    None
}
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

/// Render a note without a border box — just a coloured title bar on the first
/// row and the content below. Used for text notes when `show_border` is false.
fn render_borderless_note(
    frame: &mut Frame,
    note_area: Rect,
    title: &str,
    title_style: Style,
    bg_color: Color,
    content: impl FnOnce(&mut Frame, Rect),
) {
    // Fill background
    frame.render_widget(
        Block::default().style(Style::default().bg(bg_color)),
        note_area,
    );

    // Title on first row — full width, no side padding
    let title_area = Rect::new(note_area.x, note_area.y, note_area.width, 1);
    frame.render_widget(
        Paragraph::new(title).style(title_style),
        title_area,
    );

    // Content: 1-cell inset on left, right, and bottom (mirrors what a border gives)
    let content_area = Rect::new(
        note_area.x + 1,
        note_area.y + 1,
        note_area.width.saturating_sub(2),
        note_area.height.saturating_sub(2),
    );
    content(frame, content_area);
}

/// Render a single-cell scroll position dot on the right edge of a note.
/// Only drawn when `total_lines > visible_height` (content overflows).
/// The dot slides from the top to the bottom of the track as `scroll_top`
/// increases, giving an instant sense of scroll depth without a full scrollbar.
fn render_scroll_dot(
    frame: &mut Frame,
    note_area: Rect,
    scroll_top: usize,
    total_lines: usize,
    visible_height: usize,
    color: Color,
) {
    if total_lines <= visible_height || visible_height == 0 { return; }
    // Track: the rows between the top and bottom edge cells (title bar / border).
    let track_len = note_area.height.saturating_sub(2) as usize;
    if track_len == 0 { return; }
    let max_scroll = total_lines.saturating_sub(visible_height);
    let frac = scroll_top.min(max_scroll) as f32 / max_scroll as f32;
    let dot_offset = (frac * track_len.saturating_sub(1) as f32).round() as u16;
    frame.render_widget(
        Paragraph::new("●").style(Style::default().fg(color)),
        Rect::new(note_area.x + note_area.width - 1, note_area.y + 1 + dot_offset, 1, 1),
    );
}

/// Render all floating notes (those not on the corkboard).
pub(super) fn render_notes(frame: &mut Frame, app: &mut App, area: Rect) {
    // Precompute book state before the mutable render loop.

    // all_book_ids  — every open book page (exempts from on_corkboard filter).
    // persistent_ids — only pages of persistent notebooks (exempts from workspace filter).
    let (all_book_ids, persistent_ids): (
        std::collections::HashSet<u64>,
        std::collections::HashSet<u64>,
    ) = {
        let mut all = std::collections::HashSet::new();
        let mut pers = std::collections::HashSet::new();
        for (&nb_id, &page_idx) in &app.notebooks_open {
            if let Some(nb) = app.notebooks.iter().find(|nb| nb.id == nb_id) {
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

    // Map: note_id → (notebook_title, page_num, total_pages, persistent) for title rendering.
    let book_contexts: std::collections::HashMap<u64, (String, usize, usize, bool)> =
        app.notebooks_open.iter()
            .filter_map(|(&nb_id, &page_idx)| {
                let nb = app.notebooks.iter().find(|nb| nb.id == nb_id)?;
                let note_id = *nb.note_ids.get(page_idx)?;
                Some((note_id, (nb.title.clone(), page_idx + 1, nb.note_ids.len(), nb.persistent)))
            })
            .collect();

    // Pre-compute clamped rects for overlap detection (avoids borrow issues inside
    // the mutable render loop).
    let clamped_rects: Vec<Option<Rect>> = app.notes.iter()
        .map(|n| {
            // Background notes are rendered as the workspace background, not as floating notes.
            if n.data.is_background { return None; }
            let is_book_page = all_book_ids.contains(&n.data.id);
            let is_workspace_exempt = persistent_ids.contains(&n.data.id);
            // Notes on other workspaces are hidden, except persistent book pages which float globally.
            if n.data.workspace_id != app.active_workspace && !is_workspace_exempt { return None; }
            // On-corkboard notes are invisible unless they're a current book page.
            if n.data.on_corkboard && !is_book_page { return None; }
            Some(super::clamp_rect(
                Rect::new(n.data.x, n.data.y, n.data.width, n.data.height),
                area,
            ))
        })
        .collect();

    let has_overlap: Vec<bool> = clamped_rects.iter().enumerate()
        .map(|(i, ri)| {
            let Some(ri) = ri else { return false; };
            clamped_rects.iter().enumerate().any(|(j, rj)| {
                j != i && rj.map_or(false, |rj| effects::rects_overlap(*ri, rj))
            })
        })
        .collect();

    // A note is occluded when any higher-indexed (rendered-on-top) note overlaps it.
    let is_occluded: Vec<bool> = clamped_rects.iter().enumerate()
        .map(|(i, ri)| {
            let Some(ri) = ri else { return false; };
            clamped_rects[i + 1..].iter().any(|rj| {
                rj.map_or(false, |rj| effects::rects_overlap(*ri, rj))
            })
        })
        .collect();

    for (i, note) in app.notes.iter_mut().enumerate() {
        let is_current_book_page = all_book_ids.contains(&note.data.id);
        let is_workspace_exempt = persistent_ids.contains(&note.data.id);
        // Background notes are rendered by ui.rs as the workspace background.
        if note.data.is_background { continue; }
        // Skip notes on other workspaces. Persistent book pages are an exception — they float globally.
        if note.data.workspace_id != app.active_workspace && !is_workspace_exempt { continue; }
        if note.data.on_corkboard && !is_current_book_page { continue; }

        let is_focused = matches!(
            app.focus,
            Focus::Note(idx, _) | Focus::Renaming(idx, _) | Focus::Settings(idx, _) if idx == i
        ) || matches!(app.focus, Focus::TextVisual { note_idx, .. } if note_idx == i);
        let is_renaming = matches!(app.focus, Focus::Renaming(idx, _) if idx == i);

        let note_area = super::clamp_rect(
            Rect::new(note.data.x, note.data.y, note.data.width, note.data.height),
            area,
        );

        // Drop-shadow when this note overlaps at least one other note (if enabled)
        if app.show_shadows && has_overlap[i] {
            effects::draw_drop_shadow(frame, note_area, area, effects::NOTE_SHADOW);
        }

        frame.render_widget(Clear, note_area);

        // Book spine: drawn one cell to the left of the note when it's the active
        // book page and there's room on screen.
        if is_current_book_page && note_area.x > 0 {
            let spine_x = note_area.x - 1;
            for row in 0..note_area.height {
                frame.render_widget(
                    Paragraph::new("▐").style(
                        Style::default()
                            .fg(Color::Rgb(180, 120, 60))
                            .bg(Color::Rgb(80, 50, 20)),
                    ),
                    Rect::new(spine_x, note_area.y + row, 1, 1),
                );
            }
        }

        // Border colour: use the note's own detected app colour if available,
        // otherwise the user's chosen palette colour. The background terminal's
        // detected_bg intentionally does NOT affect note borders so colour-coded
        // notes remain distinguishable.
        let border_color = if let NoteKind::Shell { detected_bg: Some(bg), .. } = &note.kind {
            *bg
        } else {
            BORDER_PALETTE[note.data.border_color_idx].0
        };
        let border_style = if is_focused {
            Style::default().fg(border_color).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(border_color)
        };

        let pin_prefix = if note.data.pinned { "▲ " } else { "" };
        let log_prefix = if let NoteKind::Shell { ref log_path, .. } = note.kind {
            if log_path.is_some() { "● " } else { "" }
        } else { "" };
        let title = if let Focus::Renaming(idx, ref input) = app.focus {
            if idx == i {
                format!(" {}▌ ", input)
            } else {
                format!(" {}{}{} ", log_prefix, pin_prefix, note.data.title)
            }
        } else if let Some((ref nb_title, page_num, total_pages, persistent)) = book_contexts.get(&note.data.id) {
            let persist_marker = if *persistent { " ∞" } else { "" };
            format!(" {}📒 {}{} [{}/{}] : {} ", log_prefix, nb_title, persist_marker, page_num, total_pages, note.data.title)
        } else {
            format!(" {}{}{} ", log_prefix, pin_prefix, note.data.title)
        };

        let bg_color = {
            // Shell notes with an active app use the detected background colour
            // to fill the entire note, so empty/default cells match the app's theme.
            let raw = if let NoteKind::Shell { detected_bg: Some(bg), .. } = &note.kind {
                *bg
            } else {
                BG_PALETTE[note.data.bg_color_idx].0
            };
            if app.occlusion_dim != OcclusionDim::Off && is_occluded[i] {
                effects::darken_bg(raw)
            } else {
                raw
            }
        };
        // Text colour for text notes: forced black in BlackText mode, otherwise auto-contrast.
        let text_color = if app.occlusion_dim == OcclusionDim::BlackText && is_occluded[i] {
            Color::Black
        } else {
            contrast_color(bg_color)
        };
        let is_shell = matches!(note.kind, NoteKind::Shell { .. });
        let use_border = note.data.show_border || is_shell;

        if use_border {
            // When a shell note is in "flat" mode (detected_bg active), the border
            // colour matches the background so the frame is invisible.  Give the
            // title an explicit contrasting colour so it stays readable.
            let title_span = if matches!(&note.kind, NoteKind::Shell { detected_bg: Some(_), .. }) {
                let title_style = if is_focused {
                    Style::default().fg(contrast_color(bg_color)).bg(bg_color).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(contrast_color(bg_color)).bg(bg_color)
                };
                Span::styled(title, title_style)
            } else {
                Span::raw(title)
            };

            let block = Block::default()
                .borders(Borders::ALL)
                .title(title_span)
                .border_style(border_style)
                .style(Style::default().bg(bg_color));

            let inner = block.inner(note_area);
            frame.render_widget(block, note_area);

            match &mut note.kind {
                NoteKind::Photo => {
                    // Fill background first so empty cells match the note bg.
                    frame.render_widget(
                        Block::default().style(Style::default().bg(bg_color)),
                        inner,
                    );
                    frame.render_widget(
                        PhotoView { rows: &note.data.photo_rows, top_idx: 0 },
                        inner,
                    );
                }
                NoteKind::CheckList(textarea, scroll_top) => {
                    frame.render_widget(
                        Block::default().style(Style::default().bg(bg_color)),
                        inner,
                    );
                    let cursor = textarea.cursor();
                    let cursor_row = if is_focused && !is_renaming { Some(cursor.0) } else { None };
                    let cursor_col = cursor.1;
                    let visible_height = inner.height as usize;

                    // Mirror tui_textarea's lazy-scroll: advance scroll_top just
                    // enough to keep the cursor inside the visible window.
                    if let Some(cr) = cursor_row {
                        if cr < *scroll_top {
                            *scroll_top = cr;
                        } else if visible_height > 0 && cr >= *scroll_top + visible_height {
                            *scroll_top = cr + 1 - visible_height;
                        }
                    }

                    let st = *scroll_top;
                    let total_lines = textarea.lines().len();
                    let items: Vec<Line> = textarea.lines().iter().enumerate()
                        .skip(st)
                        .map(|(i, l)| {
                            let has_prefix = l.starts_with("[ ] ") || l.starts_with("[x] ");
                            let display = if has_prefix {
                                l.clone()
                            } else if l.is_empty() {
                                String::new()
                            } else {
                                format!("[ ] {l}")
                            };
                            if cursor_row == Some(i) {
                                let inv = Style::default().fg(bg_color).bg(text_color);
                                if display.is_empty() {
                                    // Cursor on empty checklist line: ZWNJ anchor prevents
                                    // the whitespace-only Line from becoming 2 visual rows.
                                    Line::from(vec![
                                        Span::styled("\u{200C}", inv),
                                        Span::styled(" ", Style::default().fg(text_color).bg(bg_color)),
                                    ])
                                } else {
                                    // Offset cursor column by 4 when "[ ] " was prepended.
                                    let prefix_added = !has_prefix && !l.is_empty();
                                    let display_col = if prefix_added { cursor_col + 4 } else { cursor_col };
                                    let byte_idx = display.char_indices()
                                        .nth(display_col)
                                        .map(|(b, _)| b)
                                        .unwrap_or(display.len());
                                    let before = &display[..byte_idx];
                                    let rest   = &display[byte_idx..];
                                    let cur_ch = rest.chars().next();
                                    let cur_str = cur_ch.map_or(" ".to_string(), |c| c.to_string());
                                    let after   = cur_ch.map_or("", |c| &rest[c.len_utf8()..]);
                                    // The whole row uses the inverted (line-highlight) style;
                                    // the cursor character is "double-reversed" back to normal
                                    // so it stands out as a block within the highlight.
                                    Line::from(vec![
                                        Span::styled(before.to_string(), inv),
                                        Span::styled(cur_str, Style::default().fg(text_color).bg(bg_color)),
                                        Span::styled(after.to_string(), inv),
                                    ])
                                }
                            } else {
                                // Empty lines: "" → 1 row (WordWrapper empty-line fallback).
                                // Do NOT use " " — whitespace-only Lines produce 2 rows.
                                Line::styled(display, Style::default().fg(text_color))
                            }
                        })
                        .collect();
                    frame.render_widget(
                        Paragraph::new(items)
                            .style(Style::default().fg(text_color).bg(bg_color))
                            .wrap(Wrap { trim: false }),
                        inner,
                    );
                    render_scroll_dot(frame, note_area, st, total_lines, visible_height, border_color);
                }
                NoteKind::Text(textarea, scroll_top) => {
                    if note.data.text_wrap {
                        // ── Wrap mode: render as Paragraph so lines fold at note width.
                        frame.render_widget(
                            Block::default().style(Style::default().bg(bg_color)),
                            inner,
                        );
                        let cursor = textarea.cursor(); // (row, col) in char units
                        let cursor_row_opt = if is_focused && !is_renaming { Some(cursor.0) } else { None };
                        let visible_height = inner.height as usize;
                        let total_lines = textarea.lines().len();
                        // No cursor-follow here — both directions are handled in the keyboard
                        // input handler so mouse scroll can move the viewport freely past the
                        // cursor in either direction without rubber-banding back each frame.
                        // Allow scroll_top to reach total_lines (blank zone below last line).
                        *scroll_top = (*scroll_top).min(total_lines);
                        let st = *scroll_top;
                        let items: Vec<Line> = textarea.lines().iter().enumerate()
                            .skip(st)
                            .map(|(i, l)| {
                                if cursor_row_opt == Some(i) {
                                    if l.is_empty() {
                                        // Cursor on an empty line: a whitespace-only Line produces
                                        // 2 visual rows with WordWrapper trim:false (phantom empty
                                        // row + space row). A U+200C zero-width non-joiner acts as
                                        // a non-whitespace anchor, keeping the line to 1 row while
                                        // the space remains the visible reversed cursor block.
                                        Line::from(vec![
                                            Span::styled("\u{200C}", Style::default().fg(text_color)),
                                            Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED)),
                                        ])
                                    } else {
                                        // Inline block cursor at cursor_col
                                        let char_idx = cursor.1;
                                        let byte_idx = l.char_indices()
                                            .nth(char_idx)
                                            .map(|(b, _)| b)
                                            .unwrap_or(l.len());
                                        let before = &l[..byte_idx];
                                        let rest   = &l[byte_idx..];
                                        let cur_ch = rest.chars().next();
                                        let cur_str = cur_ch.map_or(" ".to_string(), |c| c.to_string());
                                        let after   = cur_ch.map_or("", |c| &rest[c.len_utf8()..]);
                                        Line::from(vec![
                                            Span::styled(before.to_string(), Style::default().fg(text_color)),
                                            Span::styled(cur_str, Style::default().add_modifier(Modifier::REVERSED)),
                                            Span::styled(after.to_string(), Style::default().fg(text_color)),
                                        ])
                                    }
                                } else {
                                    // Empty lines: use an empty Line (0 graphemes → 1 row via
                                    // WordWrapper's empty-line fallback). Do NOT use " " — a
                                    // whitespace-only Line with trim:false produces 2 rows.
                                    Line::styled(l.as_str(), Style::default().fg(text_color))
                                }
                            })
                            .collect();
                        frame.render_widget(
                            Paragraph::new(items)
                                .style(Style::default().fg(text_color).bg(bg_color))
                                .wrap(Wrap { trim: false }),
                            inner,
                        );
                        render_scroll_dot(frame, note_area, st, total_lines, visible_height, border_color);
                    } else {
                        // ── No-wrap mode: use tui_textarea directly (handles scroll/cursor).
                        if is_focused && !is_renaming {
                            textarea.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
                        } else {
                            textarea.set_cursor_style(Style::default());
                        }
                        textarea.set_style(Style::default().fg(text_color).bg(bg_color));
                        frame.render_widget(&*textarea, inner);
                    }
                }
                NoteKind::Shell { parser, scroll_offset, own_scrollback, .. } => {
                    // Split render: own_scrollback fills the top rows that vt100
                    // can no longer reach; PtyView fills the remainder below.
                    let vt100_depth = parser.screen().scrollback() as i64;
                    let own_sb_rows_needed = (*scroll_offset - vt100_depth).max(0);

                    if own_sb_rows_needed > 0 && !own_scrollback.is_empty() {
                        let own_sb_rows = own_sb_rows_needed
                            .min(own_scrollback.len() as i64)
                            .min(inner.height as i64) as u16;
                        let top_idx = own_scrollback
                            .len()
                            .saturating_sub(*scroll_offset as usize);

                        let own_sb_area = Rect::new(inner.x, inner.y, inner.width, own_sb_rows);
                        frame.render_widget(
                            OwnScrollbackView { rows: own_scrollback, top_idx },
                            own_sb_area,
                        );

                        let pty_rows = inner.height.saturating_sub(own_sb_rows);
                        if pty_rows > 0 {
                            let pty_area = Rect::new(
                                inner.x,
                                inner.y + own_sb_rows,
                                inner.width,
                                pty_rows,
                            );
                            frame.render_widget(PtyView(parser.screen(), 0), pty_area);
                        }
                    } else {
                        let row_offset = (*scroll_offset).min(0).unsigned_abs() as usize;
                        frame.render_widget(PtyView(parser.screen(), row_offset), inner);
                    }

                    // Selection overlay: live drag or persistent selection for this note.
                    // Coordinates are stored screen-absolute; convert to inner-area-relative.
                    let inner_x = note.data.x + 1;
                    let inner_y = note.data.y + 1;
                    let sel: Option<(u16, u16, u16, u16)> = match &app.drag {
                        Some(DragMode::ShellSelecting {
                            note_idx, start_col, start_row, cur_col, cur_row,
                        }) if *note_idx == i => {
                            // Normalise to text order for live feedback.
                            if start_row < cur_row || (start_row == cur_row && start_col <= cur_col) {
                                Some((*start_col, *start_row, *cur_col, *cur_row))
                            } else {
                                Some((*cur_col, *cur_row, *start_col, *start_row))
                            }
                        }
                        _ => app.shell_note_selection
                            .filter(|(nidx, ..)| *nidx == i)
                            .map(|(_, sc, sr, ec, er)| (sc, sr, ec, er)),
                    };
                    if let Some((sc, sr, ec, er)) = sel {
                        super::render_stream_sel(
                            frame,
                            inner,
                            sc.saturating_sub(inner_x),
                            sr.saturating_sub(inner_y),
                            ec.saturating_sub(inner_x),
                            er.saturating_sub(inner_y),
                        );
                    }
                }
            }
        } else {
            // Borderless text note: coloured background, contrast title, padded content
            if let NoteKind::Text(textarea, scroll_top) = &mut note.kind {
                let title_style = if is_focused {
                    Style::default().fg(text_color).bg(bg_color).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(text_color).bg(bg_color)
                };

                if note.data.text_wrap {
                    // Content area height for borderless: height - 2 (title bar + bottom inset).
                    let visible_height = note_area.height.saturating_sub(2) as usize;
                    let cursor = textarea.cursor();
                    let cursor_row_opt = if is_focused && !is_renaming { Some(cursor.0) } else { None };
                    let total_lines = textarea.lines().len();
                    // No cursor-follow here — handled in the keyboard input handler.
                    *scroll_top = (*scroll_top).min(total_lines);
                    let st = *scroll_top;
                    let items: Vec<Line> = textarea.lines().iter().enumerate()
                        .skip(st)
                        .map(|(i, l)| {
                            if cursor_row_opt == Some(i) {
                                if l.is_empty() {
                                    // ZWNJ anchor: keeps whitespace-only Lines to 1 row.
                                    Line::from(vec![
                                        Span::styled("\u{200C}", Style::default().fg(text_color)),
                                        Span::styled(" ", Style::default().add_modifier(Modifier::REVERSED)),
                                    ])
                                } else {
                                    let char_idx = cursor.1;
                                    let byte_idx = l.char_indices()
                                        .nth(char_idx)
                                        .map(|(b, _)| b)
                                        .unwrap_or(l.len());
                                    let before = &l[..byte_idx];
                                    let rest   = &l[byte_idx..];
                                    let cur_ch = rest.chars().next();
                                    let cur_str = cur_ch.map_or(" ".to_string(), |c| c.to_string());
                                    let after   = cur_ch.map_or("", |c| &rest[c.len_utf8()..]);
                                    Line::from(vec![
                                        Span::styled(before.to_string(), Style::default().fg(text_color)),
                                        Span::styled(cur_str, Style::default().add_modifier(Modifier::REVERSED)),
                                        Span::styled(after.to_string(), Style::default().fg(text_color)),
                                    ])
                                }
                            } else {
                                // Empty lines: use "" not " " — whitespace-only produces 2 rows.
                                Line::styled(l.as_str(), Style::default().fg(text_color))
                            }
                        })
                        .collect();
                    render_borderless_note(
                        frame,
                        note_area,
                        &title,
                        title_style,
                        bg_color,
                        |f, content_area| {
                            f.render_widget(
                                Paragraph::new(items)
                                    .style(Style::default().fg(text_color).bg(bg_color))
                                    .wrap(Wrap { trim: false }),
                                content_area,
                            );
                        },
                    );
                    render_scroll_dot(frame, note_area, st, total_lines, visible_height, border_color);
                } else {
                    if is_focused && !is_renaming {
                        textarea.set_cursor_style(Style::default().add_modifier(Modifier::REVERSED));
                    } else {
                        textarea.set_cursor_style(Style::default());
                    }
                    textarea.set_style(Style::default().fg(text_color).bg(bg_color));
                    let ta_widget = &*textarea;
                    render_borderless_note(
                        frame,
                        note_area,
                        &title,
                        title_style,
                        bg_color,
                        |f, content_area| {
                            f.render_widget(ta_widget, content_area);
                        },
                    );
                }
            } else if let NoteKind::CheckList(textarea, scroll_top) = &mut note.kind {
                let title_style = if is_focused {
                    Style::default().fg(text_color).bg(bg_color).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(text_color).bg(bg_color)
                };
                let cursor = textarea.cursor();
                let cursor_row = if is_focused && !is_renaming { Some(cursor.0) } else { None };
                let cursor_col = cursor.1;
                // Content area height for borderless: height - 2 (title bar + bottom inset).
                let visible_height = note_area.height.saturating_sub(2) as usize;

                // Mirror tui_textarea's lazy-scroll logic.
                if let Some(cr) = cursor_row {
                    if cr < *scroll_top {
                        *scroll_top = cr;
                    } else if visible_height > 0 && cr >= *scroll_top + visible_height {
                        *scroll_top = cr + 1 - visible_height;
                    }
                }

                let st = *scroll_top;
                let total_lines = textarea.lines().len();
                let items: Vec<Line> = textarea.lines().iter().enumerate()
                    .skip(st)
                    .map(|(i, l)| {
                        let has_prefix = l.starts_with("[ ] ") || l.starts_with("[x] ");
                        let display = if has_prefix {
                            l.clone()
                        } else if l.is_empty() {
                            String::new()
                        } else {
                            format!("[ ] {l}")
                        };
                        if cursor_row == Some(i) {
                            let inv = Style::default().fg(bg_color).bg(text_color);
                            if display.is_empty() {
                                // ZWNJ anchor: keeps whitespace-only Lines to 1 row.
                                Line::from(vec![
                                    Span::styled("\u{200C}", inv),
                                    Span::styled(" ", Style::default().fg(text_color).bg(bg_color)),
                                ])
                            } else {
                                // Offset cursor column by 4 when "[ ] " was prepended.
                                let prefix_added = !has_prefix && !l.is_empty();
                                let display_col = if prefix_added { cursor_col + 4 } else { cursor_col };
                                let byte_idx = display.char_indices()
                                    .nth(display_col)
                                    .map(|(b, _)| b)
                                    .unwrap_or(display.len());
                                let before = &display[..byte_idx];
                                let rest   = &display[byte_idx..];
                                let cur_ch = rest.chars().next();
                                let cur_str = cur_ch.map_or(" ".to_string(), |c| c.to_string());
                                let after   = cur_ch.map_or("", |c| &rest[c.len_utf8()..]);
                                // Inverted line-highlight with a double-reversed cursor block.
                                Line::from(vec![
                                    Span::styled(before.to_string(), inv),
                                    Span::styled(cur_str, Style::default().fg(text_color).bg(bg_color)),
                                    Span::styled(after.to_string(), inv),
                                ])
                            }
                        } else {
                            // Empty lines: "" → 1 row. Do NOT use " " — whitespace-only produces 2.
                            Line::styled(display, Style::default().fg(text_color))
                        }
                    })
                    .collect();
                render_borderless_note(
                    frame,
                    note_area,
                    &title,
                    title_style,
                    bg_color,
                    |f, content_area| {
                        f.render_widget(
                            Paragraph::new(items)
                                .style(Style::default().fg(text_color).bg(bg_color))
                                .wrap(Wrap { trim: false }),
                            content_area,
                        );
                    },
                );
                render_scroll_dot(frame, note_area, st, total_lines, visible_height, border_color);
            }
        }

        // ── Text-note selection overlay ──────────────────────────────────────────
        // Rendered after content so the REVERSED modifier applies on top.
        // Content inner area is (note.x+1, note.y+1, width-2, height-2) for both
        // bordered and borderless layouts.
        if matches!(note.kind, NoteKind::Text(..)) {
            let content_inner = Rect::new(
                note_area.x + 1,
                note_area.y + 1,
                note_area.width.saturating_sub(2),
                note_area.height.saturating_sub(2),
            );

            // Determine selection coordinates in content-relative space.
            let sel: Option<(u16, u16, u16, u16)> = 'sel: {
                // 1. Live mouse drag
                if let Some(DragMode::TextSelecting {
                    note_idx, start_col, start_row, cur_col, cur_row,
                }) = &app.drag {
                    if *note_idx == i {
                        let (sc, sr, ec, er) = if start_row < cur_row
                            || (start_row == cur_row && start_col <= cur_col)
                        { (*start_col, *start_row, *cur_col, *cur_row) }
                        else { (*cur_col, *cur_row, *start_col, *start_row) };
                        break 'sel Some((sc, sr, ec, er));
                    }
                }
                // 2. Persistent mouse selection
                if let Some((nidx, sc, sr, ec, er)) = app.text_note_selection {
                    if nidx == i { break 'sel Some((sc, sr, ec, er)); }
                }
                // 3. Keyboard visual mode: anchor ↔ textarea cursor
                if let Focus::TextVisual { note_idx, anchor_row, anchor_col } = &app.focus {
                    if *note_idx == i {
                        if let NoteKind::Text(ta, scroll_top) = &note.kind {
                            let (cur_row, cur_col) = ta.cursor();
                            let iw = content_inner.width;
                            let tw = note.data.text_wrap;
                            let lines = ta.lines();
                            let st = *scroll_top;
                            let av = buf_to_visual(*anchor_row, *anchor_col, lines, st, iw, tw);
                            let cv = buf_to_visual(cur_row, cur_col, lines, st, iw, tw);
                            if let (Some((ar, ac)), Some((cr, cc))) = (av, cv) {
                                let (sc, sr, ec, er) = if ar < cr || (ar == cr && ac <= cc) {
                                    (ac, ar, cc, cr)
                                } else {
                                    (cc, cr, ac, ar)
                                };
                                break 'sel Some((sc, sr, ec, er));
                            }
                        }
                    }
                }
                None
            };

            if let Some((sc, sr, ec, er)) = sel {
                super::render_stream_sel(frame, content_inner, sc, sr, ec, er);
            }
        }
    }
}
