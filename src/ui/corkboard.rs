//! Corkboard overlay rendering.

use crate::{
    app::{App, CorkItem, Focus},
    colors::{BG_PALETTE, BORDER_PALETTE, contrast_color},
    constants::{CARD_GAP, CARD_H, CARD_W},
    effects,
    note::NoteKind,
    terminal::{OwnScrollbackView, PhotoView, PtyView},
    trash,
};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style, palette::material::BROWN},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

// Spine/book colours used on notebook cards and the book-mode spine strip.
pub const SPINE_FG: Color = Color::Rgb(180, 120, 60);
pub const SPINE_BG: Color = Color::Rgb(80, 50, 20);
pub const NOTEBOOK_CARD_BG: Color = Color::Rgb(240, 225, 190);

pub(super) fn render_corkboard(frame: &mut Frame, app: &App, area: Rect) {
    // Popup bounds: 3-col margin left/right, 2-row margin top/bottom.
    let popup_area = Rect::new(
        area.x + 3,
        area.y + 2,
        area.width.saturating_sub(6),
        area.height.saturating_sub(4),
    );
    frame.render_widget(Clear, popup_area);

    // ── Expanded shell view ─────────────────────────────────────────────────
    if let Some(idx) = app.corkboard_expanded {
        render_expanded_shell(frame, app, popup_area, idx);
        return;
    }

    // ── Notebook page sub-grid ──────────────────────────────────────────────
    if let Some(nb_id) = app.corkboard_notebook {
        render_notebook_subgrid(frame, app, popup_area, nb_id);
        // NamingNotebook prompt on top (shouldn't overlap, but defensive)
        if let Focus::NamingNotebook(_, ref input) = app.focus {
            render_naming_prompt(frame, area, input);
        }
        // Renaming prompt on top
        if let Focus::Renaming(_, ref input) = app.focus {
            render_renaming_prompt(frame, area, input);
        }
        return;
    }

    // ── Trash sub-grid ──────────────────────────────────────────────────────
    if app.corkboard_trash_open {
        render_trash_subgrid(frame, app, popup_area);
        return;
    }

    // ── Main mixed grid ─────────────────────────────────────────────────────
    let items = app.corkboard_items();
    let item_count = items.len();

    let count_label = match item_count {
        0 => "empty".to_string(),
        1 => "1 item".to_string(),
        n => format!("{n} items"),
    };
    let is_naming = matches!(app.focus, Focus::NamingNotebook(..));
    let title = if is_naming {
        " ❖  Corkboard — new notebook  ❖ ".to_string()
    } else {
        format!(" ❖  Corkboard — {count_label}  ❖ ")
    };

    let popup_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .title(title)
        .title_alignment(Alignment::Center)
        .border_style(
            Style::default()
                .fg(Color::Rgb(205, 133, 63))
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(Color::Rgb(101, 67, 33)));

    let inner = popup_block.inner(popup_area);
    frame.render_widget(popup_block, popup_area);
    draw_cork_texture(frame, inner);

    // Hint bar
    let hint = " n: new notebook  a: add to notebook  Enter: open  Ctrl+W: trash  Esc: close ";
    render_hint_bar(frame, inner, hint);

    let content_h = inner.height.saturating_sub(1);
    let content = Rect::new(inner.x, inner.y, inner.width, content_h);

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(
                "No notes or notebooks yet.  Ctrl+P on a note to pin it here.  n: new notebook",
            )
            .alignment(Alignment::Center)
            .style(Style::default().fg(Color::Rgb(200, 155, 80))),
            Rect::new(content.x, content.y + content.height / 2, content.width, 1),
        );
        // NamingNotebook prompt
        if let Focus::NamingNotebook(_, ref input) = app.focus {
            render_naming_prompt(frame, area, input);
        }
        return;
    }

    let cols = ((content.width + CARD_GAP) / (CARD_W + CARD_GAP)).max(1) as usize;
    let sel = app.corkboard_selected.min(item_count.saturating_sub(1));

    let visible_rows = ((content.height + CARD_GAP) / (CARD_H + CARD_GAP)).max(1) as usize;
    let sel_row = sel / cols;
    let total_rows = (item_count + cols - 1) / cols;
    let scroll = sel_row.saturating_sub(visible_rows - 1);

    // Scroll dot indicator
    if total_rows > visible_rows {
        let dots: String = (0..total_rows)
            .map(|r| if r == sel_row { "●" } else { "○" })
            .collect::<Vec<_>>()
            .join(" ");
        let dot_y = inner.y + inner.height.saturating_sub(2);
        frame.render_widget(
            Paragraph::new(dots.as_str())
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(Color::Rgb(200, 160, 90))
                        .bg(Color::Rgb(101, 67, 33)),
                ),
            Rect::new(inner.x, dot_y, inner.width, 1),
        );
    }

    // Centre the grid horizontally.
    let grid_w = cols as u16 * CARD_W + cols.saturating_sub(1) as u16 * CARD_GAP;
    let grid_x = content.x + content.width.saturating_sub(grid_w) / 2;

    for (card_idx, item) in items.iter().enumerate() {
        let row = card_idx / cols;
        let col = card_idx % cols;
        if row < scroll { continue; }
        let display_row = (row - scroll) as u16;
        let x = grid_x + col as u16 * (CARD_W + CARD_GAP);
        let y = content.y + display_row * (CARD_H + CARD_GAP);
        if y + CARD_H > content.y + content.height { break; }

        let is_selected = card_idx == sel;
        let card_area = Rect::new(x, y, CARD_W, CARD_H);

        match item {
            CorkItem::Note(note_idx) => {
                render_note_card(frame, app, card_area, *note_idx, is_selected, content);
            }
            CorkItem::Notebook(nb_id) => {
                render_notebook_card(frame, app, card_area, *nb_id, is_selected, content, y);
            }
            CorkItem::Trash => {
                render_trash_card(frame, app, card_area, is_selected, content);
            }
        }
    }

    // NamingNotebook prompt rendered on top of everything.
    if let Focus::NamingNotebook(_, ref input) = app.focus {
        render_naming_prompt(frame, area, input);
    }
    // Renaming prompt rendered on top of everything.
    if let Focus::Renaming(_, ref input) = app.focus {
        render_renaming_prompt(frame, area, input);
    }
}

// ---------------------------------------------------------------------------
// Expanded shell (unchanged from original)
// ---------------------------------------------------------------------------

fn render_expanded_shell(frame: &mut Frame, app: &App, popup_area: Rect, idx: usize) {
    let note = &app.notes[idx];
    let title = format!(" ❖  {} — live terminal  ❖ ", note.data.title);

    let popup_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .title(title)
        .title_alignment(Alignment::Center)
        .border_style(
            Style::default()
                .fg(Color::Rgb(205, 133, 63))
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(Color::Black));

    let inner = popup_block.inner(popup_area);
    frame.render_widget(popup_block, popup_area);

    let hint = " PgUp/PgDn: scroll   Ctrl+E: pick up   Esc: back to board ";
    let hint_w = (hint.len() as u16).min(inner.width);
    let hint_area = Rect::new(
        inner.x + inner.width.saturating_sub(hint_w),
        inner.y + inner.height.saturating_sub(1),
        hint_w,
        1,
    );
    frame.render_widget(
        Paragraph::new(hint).style(
            Style::default()
                .fg(Color::Rgb(240, 210, 160))
                .bg(Color::Rgb(60, 38, 16))
                .add_modifier(Modifier::BOLD),
        ),
        hint_area,
    );

    let shell_area = Rect::new(inner.x, inner.y, inner.width, inner.height.saturating_sub(1));
    if let NoteKind::Shell { parser, scroll_offset, own_scrollback, .. } = &note.kind {
        let vt100_depth = parser.screen().scrollback() as i64;
        let own_sb_rows_needed = (*scroll_offset - vt100_depth).max(0);

        if own_sb_rows_needed > 0 && !own_scrollback.is_empty() {
            let own_sb_rows = own_sb_rows_needed
                .min(own_scrollback.len() as i64)
                .min(shell_area.height as i64) as u16;
            let top_idx = own_scrollback.len().saturating_sub(*scroll_offset as usize);
            let own_sb_area = Rect::new(shell_area.x, shell_area.y, shell_area.width, own_sb_rows);
            frame.render_widget(OwnScrollbackView { rows: own_scrollback, top_idx }, own_sb_area);
            let pty_rows = shell_area.height.saturating_sub(own_sb_rows);
            if pty_rows > 0 {
                let pty_area =
                    Rect::new(shell_area.x, shell_area.y + own_sb_rows, shell_area.width, pty_rows);
                frame.render_widget(PtyView(parser.screen(), 0), pty_area);
            }
        } else {
            let row_offset = (*scroll_offset).min(0).unsigned_abs() as usize;
            frame.render_widget(PtyView(parser.screen(), row_offset), shell_area);
        }
    }
}

// ---------------------------------------------------------------------------
// Notebook page sub-grid
// ---------------------------------------------------------------------------

fn render_notebook_subgrid(frame: &mut Frame, app: &App, popup_area: Rect, nb_id: u64) {
    let nb = match app.notebooks.iter().find(|nb| nb.id == nb_id) {
        Some(nb) => nb,
        None => return,
    };

    // Collect ordered page note indices.
    let page_indices: Vec<usize> = nb
        .note_ids
        .iter()
        .filter_map(|&nid| app.notes.iter().position(|n| n.data.id == nid))
        .collect();

    let title = format!(
        " 📒  {} — {} ",
        nb.title,
        match page_indices.len() {
            0 => "no pages".to_string(),
            1 => "1 page".to_string(),
            n => format!("{n} pages"),
        }
    );

    let popup_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .title(title)
        .title_alignment(Alignment::Center)
        .border_style(
            Style::default()
                .fg(SPINE_FG)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(Color::Rgb(101, 67, 33)));

    let inner = popup_block.inner(popup_area);
    frame.render_widget(popup_block, popup_area);
    draw_cork_texture(frame, inner);

    let hint =
        " arrows: navigate  Shift+arrows: reorder  Enter: open  a: add  r: remove  Esc: back ";
    render_hint_bar(frame, inner, hint);

    let content_h = inner.height.saturating_sub(1);
    let content = Rect::new(inner.x, inner.y, inner.width, content_h);

    if page_indices.is_empty() {
        frame.render_widget(
            Paragraph::new("No pages yet.  Press 'a' to add a note from the corkboard.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Rgb(200, 155, 80))),
            Rect::new(content.x, content.y + content.height / 2, content.width, 1),
        );
        return;
    }

    let cols = ((content.width + CARD_GAP) / (CARD_W + CARD_GAP)).max(1) as usize;
    let page_count = page_indices.len();
    let sel = app.corkboard_nb_selected.min(page_count.saturating_sub(1));

    let visible_rows = ((content.height + CARD_GAP) / (CARD_H + CARD_GAP)).max(1) as usize;
    let sel_row = sel / cols;
    let total_rows = (page_count + cols - 1) / cols;
    let scroll = sel_row.saturating_sub(visible_rows - 1);

    if total_rows > visible_rows {
        let dots: String = (0..total_rows)
            .map(|r| if r == sel_row { "●" } else { "○" })
            .collect::<Vec<_>>()
            .join(" ");
        frame.render_widget(
            Paragraph::new(dots.as_str())
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(Color::Rgb(200, 160, 90))
                        .bg(Color::Rgb(101, 67, 33)),
                ),
            Rect::new(inner.x, inner.y + inner.height.saturating_sub(2), inner.width, 1),
        );
    }

    let grid_w = cols as u16 * CARD_W + cols.saturating_sub(1) as u16 * CARD_GAP;
    let grid_x = content.x + content.width.saturating_sub(grid_w) / 2;

    for (card_idx, &note_idx) in page_indices.iter().enumerate() {
        let row = card_idx / cols;
        let col = card_idx % cols;
        if row < scroll { continue; }
        let display_row = (row - scroll) as u16;
        let x = grid_x + col as u16 * (CARD_W + CARD_GAP);
        let y = content.y + display_row * (CARD_H + CARD_GAP);
        if y + CARD_H > content.y + content.height { break; }

        let is_selected = card_idx == sel;
        let card_area = Rect::new(x, y, CARD_W, CARD_H);
        render_note_card(frame, app, card_area, note_idx, is_selected, content);

        // Tiny page-number badge in the top-left corner of the card.
        let badge = format!(" p.{} ", card_idx + 1);
        frame.render_widget(
            Paragraph::new(badge.as_str()).style(
                Style::default()
                    .fg(Color::Black)
                    .bg(SPINE_FG)
                    .add_modifier(Modifier::BOLD),
            ),
            Rect::new(x + 1, y, (badge.len() as u16).min(CARD_W - 2), 1),
        );
    }
}

// ---------------------------------------------------------------------------
// Individual card renderers
// ---------------------------------------------------------------------------

fn render_note_card(
    frame: &mut Frame,
    app: &App,
    card_area: Rect,
    note_idx: usize,
    is_selected: bool,
    content: Rect,
) {
    let note = &app.notes[note_idx];

    let card_bg = if note.data.bg_color_idx == 0 {
        Color::Rgb(245, 238, 210)
    } else {
        BG_PALETTE[note.data.bg_color_idx].0
    };

    if is_selected {
        effects::draw_drop_shadow(frame, card_area, content, effects::CORK_SHADOW);
    }
    frame.render_widget(Clear, card_area);

    let (border_type, border_color) = if is_selected {
        (BorderType::Thick, Color::Rgb(255, 220, 100))
    } else {
        (BorderType::Rounded, BORDER_PALETTE[note.data.border_color_idx].0)
    };
    let border_style = if is_selected {
        Style::default().fg(border_color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(border_color)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .title(format!(" {} ", note.data.title))
        .border_style(border_style)
        .style(Style::default().bg(card_bg));

    let card_inner = block.inner(card_area);
    frame.render_widget(block, card_area);

    match &note.kind {
        NoteKind::Text(textarea, _) | NoteKind::CheckList(textarea, _) => {
            let text_color = contrast_color(card_bg);
            let lines: Vec<Line> = textarea
                .lines()
                .iter()
                .map(|l| {
                    Line::from(Span::styled(l.as_str(), Style::default().fg(text_color)))
                })
                .collect();
            frame.render_widget(
                Paragraph::new(Text::from(lines)).style(Style::default().bg(card_bg)),
                card_inner,
            );
        }
        NoteKind::Shell { parser, .. } => {
            frame.render_widget(PtyView(parser.screen(), 0), card_inner);
        }
        NoteKind::Photo => {
            frame.render_widget(
                PhotoView { rows: &note.data.photo_rows, top_idx: 0 },
                card_inner,
            );
        }
    }
}

/// Render a notebook folder card with a "closed book / stacked pages" visual.
fn render_notebook_card(
    frame: &mut Frame,
    app: &App,
    card_area: Rect,
    nb_id: u64,
    is_selected: bool,
    content: Rect,
    card_y: u16,
) {
    let nb = match app.notebooks.iter().find(|nb| nb.id == nb_id) {
        Some(nb) => nb,
        None => return,
    };
    let page_count = nb.note_ids.len();

    if is_selected {
        effects::draw_drop_shadow(frame, card_area, content, effects::CORK_SHADOW);
    }
    frame.render_widget(Clear, card_area);

    // "Page block" effect: a single off-white strip just below the cover bottom,
    // inset 1 cell on each side so it peeks out like the pages of a closed book.
    let page_block_y = card_y + CARD_H;
    let page_block_w = CARD_W.saturating_sub(2);
    if page_block_w > 0 && page_block_y < content.y + content.height {
        frame.render_widget(
            Paragraph::new("▂".repeat(page_block_w as usize).as_str())
                .style(Style::default().fg(Color::Rgb(180, 120, 60)).bg(Color::Rgb(245, 242, 232))),
            Rect::new(card_area.x + 1, page_block_y, page_block_w, 1),
        );
        frame.render_widget(
            Paragraph::new("▓")
                .style(Style::default().fg(Color::Rgb(240, 225, 190)).bg(Color::Rgb(180, 120, 60))),
            Rect::new(card_area.x, page_block_y, 1, 1)
        );
        frame.render_widget(
            Paragraph::new(" ")
                .style(Style::default().bg(Color::Rgb(180, 120, 60))), 
            Rect::new(card_area.x + page_block_w + 1, page_block_y, 1, 1)    
        );
    }

    // Book-cover card with double border.
    let (border_type, border_color) = if is_selected {
        (BorderType::Double, Color::Rgb(160, 100, 50))
    } else {
        (BorderType::Double, SPINE_FG)
    };
    let border_style = if is_selected {
        Style::default().fg(border_color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(border_color)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .title(format!(" 📒 {} ", nb.title))
        .border_style(border_style)
        .style(Style::default().bg(NOTEBOOK_CARD_BG));

    let card_inner = block.inner(card_area);
    frame.render_widget(block, card_area);

    // Spine strip on the left edge of the card inner area.
    if card_inner.width > 2 {
        for row in 0..card_inner.height {
            frame.render_widget(
                Paragraph::new("▐").style(Style::default().fg(SPINE_FG).bg(SPINE_BG)),
                Rect::new(card_inner.x, card_inner.y + row, 1, 1),
            );
        }
    }

    // Page count centred in the card.
    let page_label = match page_count {
        0 => "  (empty)  ".to_string(),
        1 => "  1 page  ".to_string(),
        n => format!("  {n} pages  "),
    };
    let label_w = (page_label.len() as u16).min(card_inner.width);
    let label_x = card_inner.x + card_inner.width.saturating_sub(label_w) / 2;
    let label_y = card_inner.y + card_inner.height / 2;
    
    let pageblock = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Thick)
        .style(Style::default().bg(Color::Rgb(245, 242, 232)).fg(Color::Rgb(255, 220, 100)));
    
    frame.render_widget(pageblock, Rect::new(label_x - 1, label_y - 1, label_w + 2, 3));

    frame.render_widget(
        Paragraph::new(page_label.as_str()).style(
            Style::default()
                .fg(Color::Rgb(80, 50, 20))
                .bg(Color::Rgb(245, 242, 232))
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(label_x, label_y, label_w, 1),
    );

    // "Enter to open" hint at the bottom of selected card.
    if is_selected {
        let open_hint = " Open ▶ ";
        let hint_w = (open_hint.len() as u16).min(card_inner.width);
        let hint_x = card_inner.x + card_inner.width.saturating_sub(hint_w);
        let hint_y = card_inner.y + card_inner.height.saturating_sub(1);
        frame.render_widget(
            Paragraph::new(open_hint).style(
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(255, 220, 100))
                    .add_modifier(Modifier::BOLD),
            ),
            Rect::new(hint_x, hint_y, hint_w, 1),
        );
    }
}

// ---------------------------------------------------------------------------
// Trash card and trash sub-grid
// ---------------------------------------------------------------------------

fn render_trash_card(
    frame: &mut Frame,
    app: &App,
    card_area: Rect,
    is_selected: bool,
    content: Rect,
) {
    if is_selected {
        effects::draw_drop_shadow(frame, card_area, content, effects::CORK_SHADOW);
    }
    frame.render_widget(Clear, card_area);

    let (border_type, border_color) = if is_selected {
        (BorderType::Thick, Color::Rgb(200, 80, 80))
    } else {
        (BorderType::Rounded, Color::Rgb(120, 60, 60))
    };
    let border_style = if is_selected {
        Style::default().fg(border_color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(border_color)
    };

    let count = app.trash.len();
    let title = format!(" 🗑  Trash ({}) ", count);
    let card_bg = Color::Rgb(50, 40, 40);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(border_type)
        .title(title)
        .border_style(border_style)
        .style(Style::default().bg(card_bg));

    let card_inner = block.inner(card_area);
    frame.render_widget(block, card_area);

    let (label, label_color) = if count == 0 {
        ("  empty  ", Color::Rgb(100, 80, 80))
    } else {
        ("  Open ▶  ", Color::Rgb(200, 80, 80))
    };
    let label_w = (label.len() as u16).min(card_inner.width);
    let label_x = card_inner.x + card_inner.width.saturating_sub(label_w) / 2;
    let label_y = card_inner.y + card_inner.height / 2;
    frame.render_widget(
        Paragraph::new(label).style(
            Style::default()
                .fg(label_color)
                .bg(card_bg)
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(label_x, label_y, label_w, 1),
    );
}

fn render_trash_subgrid(frame: &mut Frame, app: &App, popup_area: Rect) {
    let count = app.trash.len();
    let title = format!(
        " 🗑  Trash — {} ",
        match count {
            0 => "empty".to_string(),
            1 => "1 item".to_string(),
            n => format!("{n} items"),
        }
    );

    let popup_block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Double)
        .title(title)
        .title_alignment(Alignment::Center)
        .border_style(
            Style::default()
                .fg(Color::Rgb(180, 60, 60))
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(Color::Rgb(35, 25, 25)));

    let inner = popup_block.inner(popup_area);
    frame.render_widget(popup_block, popup_area);

    // Cork texture with darker tint
    for row in 0..inner.height {
        let mut spans: Vec<Span> = Vec::with_capacity(inner.width as usize);
        for col in 0..inner.width {
            let ch = if (col + row) % 4 == 0 { "·" } else { " " };
            spans.push(Span::styled(
                ch,
                Style::default().fg(Color::Rgb(80, 50, 50)).bg(Color::Rgb(35, 25, 25)),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(inner.x, inner.y + row, inner.width, 1),
        );
    }

    let hint = " r/Enter: restore  Ctrl+W: delete forever  Ctrl+X: empty trash  Esc: back ";
    render_hint_bar(frame, inner, hint);

    let content_h = inner.height.saturating_sub(1);
    let content = Rect::new(inner.x, inner.y, inner.width, content_h);

    if count == 0 {
        frame.render_widget(
            Paragraph::new("Trash is empty.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(Color::Rgb(150, 80, 80))),
            Rect::new(content.x, content.y + content.height / 2, content.width, 1),
        );
        return;
    }

    let cols = ((content.width + CARD_GAP) / (CARD_W + CARD_GAP)).max(1) as usize;
    let sel = app.corkboard_trash_selected.min(count - 1);
    let visible_rows = ((content.height + CARD_GAP) / (CARD_H + CARD_GAP)).max(1) as usize;
    let sel_row = sel / cols;
    let total_rows = (count + cols - 1) / cols;
    let scroll = sel_row.saturating_sub(visible_rows - 1);

    if total_rows > visible_rows {
        let dots: String = (0..total_rows)
            .map(|r| if r == sel_row { "●" } else { "○" })
            .collect::<Vec<_>>()
            .join(" ");
        frame.render_widget(
            Paragraph::new(dots.as_str())
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(Color::Rgb(180, 90, 90))
                        .bg(Color::Rgb(35, 25, 25)),
                ),
            Rect::new(inner.x, inner.y + inner.height.saturating_sub(2), inner.width, 1),
        );
    }

    let grid_w = cols as u16 * CARD_W + cols.saturating_sub(1) as u16 * CARD_GAP;
    let grid_x = content.x + content.width.saturating_sub(grid_w) / 2;

    for (card_idx, trashed) in app.trash.iter().enumerate() {
        let row = card_idx / cols;
        let col = card_idx % cols;
        if row < scroll { continue; }
        let display_row = (row - scroll) as u16;
        let x = grid_x + col as u16 * (CARD_W + CARD_GAP);
        let y = content.y + display_row * (CARD_H + CARD_GAP);
        if y + CARD_H > content.y + content.height { break; }

        let is_selected = card_idx == sel;
        let card_area = Rect::new(x, y, CARD_W, CARD_H);

        // Card shadow
        if is_selected {
            effects::draw_drop_shadow(frame, card_area, content, effects::CORK_SHADOW);
        }
        frame.render_widget(Clear, card_area);

        let card_bg = Color::Rgb(60, 45, 45);
        let (border_type, border_color) = if is_selected {
            (BorderType::Thick, Color::Rgb(200, 80, 80))
        } else {
            (BorderType::Rounded, Color::Rgb(120, 70, 70))
        };

        let age = trash::format_age(trashed.deleted_at);
        let type_icon = if trashed.data.is_shell { "🐚" }
            else if trashed.data.is_photo { "📷" }
            else if trashed.data.is_checklist { "☑" }
            else { "📝" };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(border_type)
            .title(format!(" {} {} ", type_icon, trashed.data.title))
            .border_style(Style::default().fg(border_color))
            .style(Style::default().bg(card_bg));

        let card_inner = block.inner(card_area);
        frame.render_widget(block, card_area);

        // Content preview (text lines)
        let text_color = Color::Rgb(180, 140, 140);
        let lines: Vec<Line> = trashed.data.content.iter()
            .take(card_inner.height.saturating_sub(1) as usize)
            .map(|l| Line::from(Span::styled(l.as_str(), Style::default().fg(text_color))))
            .collect();
        if !lines.is_empty() {
            frame.render_widget(
                Paragraph::new(lines).style(Style::default().bg(card_bg)),
                card_inner,
            );
        }

        // Age badge in bottom-right
        let age_text = format!(" {} ago ", age);
        let age_w = (age_text.len() as u16).min(card_inner.width);
        let age_x = card_inner.x + card_inner.width.saturating_sub(age_w);
        let age_y = card_inner.y + card_inner.height.saturating_sub(1);
        frame.render_widget(
            Paragraph::new(age_text.as_str()).style(
                Style::default().fg(Color::Rgb(160, 100, 100)).bg(card_bg),
            ),
            Rect::new(age_x, age_y, age_w, 1),
        );

        // "Restore" hint on selected card
        if is_selected {
            let hint = " Restore ↩ ";
            let hw = (hint.len() as u16).min(card_inner.width);
            frame.render_widget(
                Paragraph::new(hint).style(
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(200, 80, 80))
                        .add_modifier(Modifier::BOLD),
                ),
                Rect::new(card_inner.x, card_inner.y, hw, 1),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// NamingNotebook prompt overlay
// ---------------------------------------------------------------------------

fn render_naming_prompt(frame: &mut Frame, area: Rect, input: &str) {
    let prompt = format!(" New notebook name: {}▌ ", input);
    let popup_w = (prompt.len() as u16 + 4).max(40).min(area.width.saturating_sub(6));
    let popup_h = 3u16;
    let popup_x = area.x + area.width.saturating_sub(popup_w) / 2;
    let popup_y = area.y + area.height.saturating_sub(popup_h) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_w, popup_h);

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Name your notebook ")
        .title_alignment(Alignment::Center)
        .border_style(Style::default().fg(SPINE_FG).add_modifier(Modifier::BOLD))
        .style(Style::default().bg(NOTEBOOK_CARD_BG));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);
    frame.render_widget(
        Paragraph::new(prompt.as_str()).style(Style::default().fg(Color::Rgb(60, 30, 10))),
        inner,
    );
}

// ---------------------------------------------------------------------------
// Rename note prompt overlay
// ---------------------------------------------------------------------------

fn render_renaming_prompt(frame: &mut Frame, area: Rect, input: &str) {
    let prompt = format!(" Rename: {}▌ ", input);
    let popup_w = (prompt.len() as u16 + 4).max(40).min(area.width.saturating_sub(6));
    let popup_h = 3u16;
    let popup_x = area.x + area.width.saturating_sub(popup_w) / 2;
    let popup_y = area.y + area.height.saturating_sub(popup_h) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_w, popup_h);

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(" Rename note ")
        .title_alignment(Alignment::Center)
        .border_style(
            Style::default()
                .fg(Color::Rgb(255, 220, 100))
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(Color::Rgb(40, 35, 20)));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);
    frame.render_widget(
        Paragraph::new(prompt.as_str())
            .style(Style::default().fg(Color::Rgb(255, 240, 150))),
        inner,
    );
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn draw_cork_texture(frame: &mut Frame, inner: Rect) {
    for row in 0..inner.height {
        let mut spans: Vec<Span> = Vec::with_capacity(inner.width as usize);
        for col in 0..inner.width {
            let ch = if (col + row) % 4 == 0 { "·" } else { " " };
            spans.push(Span::styled(
                ch,
                Style::default()
                    .fg(Color::Rgb(140, 90, 40))
                    .bg(Color::Rgb(101, 67, 33)),
            ));
        }
        frame.render_widget(
            Paragraph::new(Line::from(spans)),
            Rect::new(inner.x, inner.y + row, inner.width, 1),
        );
    }
}

fn render_hint_bar(frame: &mut Frame, inner: Rect, hint: &str) {
    let hint_w = (hint.len() as u16).min(inner.width);
    frame.render_widget(
        Paragraph::new(hint).style(
            Style::default()
                .fg(Color::Rgb(240, 210, 160))
                .bg(Color::Rgb(60, 38, 16))
                .add_modifier(Modifier::BOLD),
        ),
        Rect::new(
            inner.x + inner.width.saturating_sub(hint_w),
            inner.y + inner.height.saturating_sub(1),
            hint_w,
            1,
        ),
    );
}
