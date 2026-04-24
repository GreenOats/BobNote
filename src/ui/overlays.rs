//! Settings popup, hint bar, splash screen, and workspace bar rendering.

use crate::{
    app::{App, Focus, NotebookPickerMode, NoteType, SettingsSection},
    colors::{BG_PALETTE, BORDER_PALETTE, contrast_color},
    note::NoteKind,
    ui::corkboard::{NOTEBOOK_CARD_BG, SPINE_BG, SPINE_FG},
};
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

// ---------------------------------------------------------------------------
// Settings popup
// ---------------------------------------------------------------------------

pub(super) fn render_settings_popup(frame: &mut Frame, app: &App, area: Rect) {
    let Focus::Settings(note_idx, ref section) = app.focus else { return; };

    let note = &app.notes[note_idx];
    let toggle_active = matches!(section, SettingsSection::BorderToggle);
    let border_active = matches!(section, SettingsSection::Border);
    let bg_active     = matches!(section, SettingsSection::Background);
    let wrap_active   = matches!(section, SettingsSection::TextWrap);

    let is_text_note = matches!(note.kind, NoteKind::Text(..));

    let inner_w = (BORDER_PALETTE.len() * 2 + 4) as u16;
    let popup_w = inner_w + 2;
    let popup_h = if is_text_note { 17u16 } else { 14u16 };
    let popup_area = super::centered_rect(popup_w, popup_h, area);

    frame.render_widget(Clear, popup_area);

    let popup_block = Block::default()
        .borders(Borders::ALL)
        .title(" Note Settings ")
        .border_style(Style::default().fg(Color::Yellow));

    let inner = popup_block.inner(popup_area);
    frame.render_widget(popup_block, popup_area);

    let active_label   = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let inactive_label = Style::default().fg(Color::DarkGray);
    let dim_label      = Style::default().fg(Color::DarkGray);

    // Border toggle checkbox
    let checkbox = if note.data.show_border { "[x]" } else { "[ ]" };
    let toggle_label_style = if toggle_active { active_label } else { inactive_label };

    // Border-color section: dimmed when border is hidden
    let border_label_style = if note.data.show_border {
        if border_active { active_label } else { inactive_label }
    } else {
        dim_label
    };

    let mut lines: Vec<Line> = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled(format!(" {} Show Border", checkbox), toggle_label_style),
        ]),
        Line::raw(""),
        Line::from(Span::styled("  Border Color", border_label_style)),
        border_color_name_line(BORDER_PALETTE, note.data.border_color_idx, note.data.show_border),
        border_swatch_line(BORDER_PALETTE, note.data.border_color_idx, note.data.show_border),
        Line::raw(""),
        Line::from(Span::styled(
            " Background",
            if bg_active { active_label } else { inactive_label },
        )),
        color_name_line(BG_PALETTE, note.data.bg_color_idx),
        swatch_line(BG_PALETTE, note.data.bg_color_idx),
        Line::raw(""),
    ];
    if is_text_note {
        let wrap_checkbox = if note.data.text_wrap { "[x]" } else { "[ ]" };
        lines.push(Line::from(Span::styled(
            format!(" {} Text Wrap", wrap_checkbox),
            if wrap_active { active_label } else { inactive_label },
        )));
        lines.push(Line::raw(""));
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(Span::styled(
        " Tab/↑↓: section   ←→: colour   Space: toggle   Esc: close",
        Style::default().fg(Color::DarkGray),
    )));

    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn color_name_line<'a>(palette: &'a [(Color, &'a str)], selected: usize) -> Line<'a> {
    let (color, name) = palette[selected];
    Line::from(vec![
        Span::raw(" ◄ "),
        Span::styled(name, Style::default().fg(color).add_modifier(Modifier::BOLD)),
        Span::raw(" ►"),
    ])
}

/// Like `color_name_line` but dimmed when `enabled` is false.
fn border_color_name_line<'a>(palette: &'a [(Color, &'a str)], selected: usize, enabled: bool) -> Line<'a> {
    if !enabled {
        let (_, name) = palette[selected];
        return Line::from(Span::styled(
            format!(" ◄ {} ►", name),
            Style::default().fg(Color::DarkGray),
        ));
    }
    color_name_line(palette, selected)
}

fn swatch_line<'a>(palette: &'a [(Color, &'a str)], selected: usize) -> Line<'a> {
    let mut spans = vec![Span::raw("  ")];
    for (i, (color, _)) in palette.iter().enumerate() {
        let bg = Style::default().bg(*color);
        if i == selected {
            spans.push(Span::styled("*", bg.fg(Color::Black).add_modifier(Modifier::BOLD)));
        } else {
            spans.push(Span::styled(" ", bg));
        }
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

/// Like `swatch_line` but all swatches shown as dark gray blocks when `enabled` is false.
fn border_swatch_line<'a>(palette: &'a [(Color, &'a str)], selected: usize, enabled: bool) -> Line<'a> {
    if !enabled {
        let mut spans = vec![Span::raw("  ")];
        for _ in palette.iter() {
            spans.push(Span::styled(" ", Style::default().bg(Color::DarkGray)));
            spans.push(Span::raw(" "));
        }
        return Line::from(spans);
    }
    swatch_line(palette, selected)
}

// ---------------------------------------------------------------------------
// Notebook picker overlay
// ---------------------------------------------------------------------------

pub(super) fn render_notebook_picker(frame: &mut Frame, app: &App, area: Rect) {
    let Some(ref picker) = app.notebook_picker else { return; };

    match &picker.mode {
        NotebookPickerMode::AssignToNotebook(_) => {
            // Show a list of notebooks to assign to.
            let popup_h = (app.notebooks.len() as u16 + 4).min(area.height.saturating_sub(4));
            let popup_w = 44u16.min(area.width.saturating_sub(4));
            let popup_area = super::centered_rect(popup_w, popup_h, area);
            frame.render_widget(Clear, popup_area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" Assign to Notebook ")
                .title_alignment(Alignment::Center)
                .border_style(
                    Style::default()
                        .fg(Color::Rgb(180, 120, 60))
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(Color::Rgb(30, 20, 10)));
            let inner = block.inner(popup_area);
            frame.render_widget(block, popup_area);

            if app.notebooks.is_empty() {
                frame.render_widget(
                    Paragraph::new("No notebooks yet.  Create one with 'n' on the corkboard.")
                        .style(Style::default().fg(Color::DarkGray)),
                    inner,
                );
                return;
            }

            let sel = picker.selected.min(app.notebooks.len().saturating_sub(1));
            for (i, nb) in app.notebooks.iter().enumerate() {
                if i >= inner.height as usize { break; }
                let label = format!(" 📒 {} ({} pages) ", nb.title, nb.note_ids.len());
                let style = if i == sel {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(255, 220, 100))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Rgb(220, 190, 140))
                };
                frame.render_widget(
                    Paragraph::new(label.as_str()).style(style),
                    Rect::new(inner.x, inner.y + i as u16, inner.width, 1),
                );
            }

            // Hint at bottom
            let hint = " ↑↓/jk: navigate  Enter: assign  Esc: cancel ";
            let hint_w = (hint.len() as u16).min(inner.width);
            frame.render_widget(
                Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
                Rect::new(
                    inner.x,
                    inner.y + inner.height.saturating_sub(1),
                    hint_w,
                    1,
                ),
            );
        }

        NotebookPickerMode::AddToNotebook(nb_id) => {
            let nb_id = *nb_id;
            // Show free corkboard notes that can be added.
            let free: Vec<(usize, &crate::note::Note)> = app
                .notes
                .iter()
                .enumerate()
                .filter(|(_, n)| n.data.on_corkboard && n.data.notebook_id != Some(nb_id))
                .collect();

            let popup_h = (free.len() as u16 + 4).min(area.height.saturating_sub(4)).max(5);
            let popup_w = 44u16.min(area.width.saturating_sub(4));
            let popup_area = super::centered_rect(popup_w, popup_h, area);
            frame.render_widget(Clear, popup_area);

            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .title(" Add Page from Corkboard ")
                .title_alignment(Alignment::Center)
                .border_style(
                    Style::default()
                        .fg(Color::Rgb(180, 120, 60))
                        .add_modifier(Modifier::BOLD),
                )
                .style(Style::default().bg(Color::Rgb(30, 20, 10)));
            let inner = block.inner(popup_area);
            frame.render_widget(block, popup_area);

            if free.is_empty() {
                frame.render_widget(
                    Paragraph::new("No free corkboard notes available.")
                        .style(Style::default().fg(Color::DarkGray)),
                    inner,
                );
                return;
            }

            let sel = picker.selected.min(free.len().saturating_sub(1));
            for (list_idx, (_, note)) in free.iter().enumerate() {
                if list_idx >= inner.height as usize { break; }
                let label = format!("   {} ", note.data.title);
                let style = if list_idx == sel {
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(255, 220, 100))
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Rgb(220, 190, 140))
                };
                frame.render_widget(
                    Paragraph::new(label.as_str()).style(style),
                    Rect::new(inner.x, inner.y + list_idx as u16, inner.width, 1),
                );
            }

            let hint = " ↑↓/jk: navigate  Enter: add  Esc: cancel ";
            let hint_w = (hint.len() as u16).min(inner.width);
            frame.render_widget(
                Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
                Rect::new(
                    inner.x,
                    inner.y + inner.height.saturating_sub(1),
                    hint_w,
                    1,
                ),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Logging setup popup
// ---------------------------------------------------------------------------

pub(super) fn render_logging_popup(frame: &mut Frame, app: &App, area: Rect) {
    let Focus::LoggingSetup(note_idx, ref input) = app.focus else { return; };

    let note_title = app.notes[note_idx].data.title.as_str();
    let title = format!(" Start logging: {} ", note_title);

    // Show the default path as a placeholder hint when the field is empty.
    let default_path = app.default_log_path(note_idx);
    let display_path = if input.is_empty() {
        default_path.as_str()
    } else {
        input.as_str()
    };
    // Truncate path for display if it is very long.
    let max_inner_w = area.width.saturating_sub(6) as usize;
    let display_truncated = if display_path.len() > max_inner_w && max_inner_w > 3 {
        format!("...{}", &display_path[display_path.len().saturating_sub(max_inner_w - 3)..])
    } else {
        display_path.to_string()
    };

    let cursor_line = if input.is_empty() {
        // Placeholder: dim hint showing default path
        format!(" {} ", display_truncated)
    } else {
        format!(" {}▌ ", display_truncated)
    };

    let popup_w = (cursor_line.len() as u16 + 4)
        .max(title.len() as u16 + 4)
        .max(50)
        .min(area.width.saturating_sub(4));
    let popup_h = 7u16;
    let popup_area = super::centered_rect(popup_w, popup_h, area);

    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title.as_str())
        .title_alignment(Alignment::Center)
        .border_style(
            Style::default()
                .fg(Color::Rgb(220, 80, 80))
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(Color::Rgb(30, 10, 10)));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let path_label_style = Style::default().fg(Color::DarkGray);
    let path_style = if input.is_empty() {
        Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC)
    } else {
        Style::default().fg(Color::Rgb(255, 180, 180))
    };
    let hint_style = Style::default().fg(Color::DarkGray);

    use ratatui::text::{Line, Span};
    let lines = vec![
        Line::raw(""),
        Line::from(Span::styled("  Log path:", path_label_style)),
        Line::from(Span::styled(cursor_line.as_str(), path_style)),
        Line::raw(""),
        Line::from(Span::styled(
            "  Leave blank to use the default path above.",
            hint_style,
        )),
        Line::from(Span::styled(
            "  Enter: start logging   Esc: cancel",
            hint_style,
        )),
    ];
    use ratatui::widgets::Paragraph;
    use ratatui::text::Text;
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

// ---------------------------------------------------------------------------
// Splash screen
// ---------------------------------------------------------------------------

const BOB_TXT: &str = include_str!("../../bob.txt");

/// Expand tab characters using 4-column tab stops.
fn expand_tabs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut col = 0usize;
    for ch in s.chars() {
        if ch == '\t' {
            let spaces = 4 - (col % 4);
            for _ in 0..spaces {
                out.push(' ');
            }
            col += spaces;
        } else {
            out.push(ch);
            col += 1;
        }
    }
    out
}

pub(super) fn render_splash(frame: &mut Frame, area: Rect) {
    let lines: Vec<String> = BOB_TXT.lines().map(expand_tabs).collect();
    let content_w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
    let content_h = lines.len() as u16;
    let popup_w = content_w + 2; // +2 for left/right border
    let popup_h = content_h + 2; // +2 for top/bottom border

    let popup_area = super::centered_rect(popup_w, popup_h, area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let text_lines: Vec<Line> = lines.iter().map(|l| Line::raw(l.as_str())).collect();
    frame.render_widget(Paragraph::new(Text::from(text_lines)), inner);
}

// ---------------------------------------------------------------------------
// Workspace tab bar
// ---------------------------------------------------------------------------

pub(super) fn render_workspace_bar(frame: &mut Frame, app: &App, area: Rect) {
    if app.workspace_count == 0 { return; }
    let count = app.workspace_count as u16;
    let tab_w = (area.width / count).max(1);

    for i in 0..app.workspace_count {
        let x = area.x + i as u16 * tab_w;
        // Last tab absorbs any leftover columns from integer division.
        let w = if i == app.workspace_count - 1 {
            area.width.saturating_sub(x)
        } else {
            tab_w
        };
        if w == 0 { continue; }

        // While renaming, show the live input in the active tab.
        let label = if i == app.active_workspace {
            if let Focus::RenamingWorkspace(ref input) = app.focus {
                format!(" {}▌ ", input)
            } else {
                let name = app.workspace_names.get(i as usize).map(|s| s.as_str()).unwrap_or("?");
                format!(" {} ", name)
            }
        } else {
            let name = app.workspace_names.get(i as usize).map(|s| s.as_str()).unwrap_or("?");
            format!(" {} ", name)
        };

        let style = if i == app.active_workspace {
            Style::default()
                .fg(NOTEBOOK_CARD_BG)
                .bg(SPINE_FG)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(SPINE_FG)
                .bg(SPINE_BG)
        };
        frame.render_widget(
            Paragraph::new(label.as_str()).style(style),
            Rect::new(x, area.y, w, 1),
        );
    }

    // Rename popup
    if let Focus::RenamingWorkspace(ref input) = app.focus {
        render_workspace_rename_popup(frame, app, area, input);
    }
}

fn render_workspace_rename_popup(frame: &mut Frame, app: &App, area: Rect, input: &str) {
    let ws_name = app.workspace_names
        .get(app.active_workspace as usize)
        .map(|s| s.as_str())
        .unwrap_or("?");
    let title = format!(" Rename workspace: {} ", ws_name);
    let prompt = format!(" {}▌ ", input);
    let popup_w = (prompt.len() as u16 + 4)
        .max(title.len() as u16 + 4)
        .max(40)
        .min(area.width.saturating_sub(6));
    let popup_h = 3u16;
    let popup_x = area.x + area.width.saturating_sub(popup_w) / 2;
    let popup_y = area.y + area.height.saturating_sub(popup_h) / 2;
    let popup_area = Rect::new(popup_x, popup_y, popup_w, popup_h);

    frame.render_widget(Clear, popup_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .title(title.as_str())
        .title_alignment(Alignment::Center)
        .border_style(
            Style::default()
                .fg(SPINE_FG)
                .add_modifier(Modifier::BOLD),
        )
        .style(Style::default().bg(SPINE_BG));
    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);
    frame.render_widget(
        Paragraph::new(prompt.as_str()).style(Style::default().fg(SPINE_FG)),
        inner,
    );
}

// ---------------------------------------------------------------------------
// Hint bar
// ---------------------------------------------------------------------------

pub(super) fn render_hint(frame: &mut Frame, app: &App, area: Rect) {
    let y = area.height.saturating_sub(1);

    frame.render_widget(
        //Paragraph::new(" ".re(area.width).as_str()),
Block::new().bg(SPINE_FG),
  Rect::new(0, y, area.width, 1
    ));

    // Read scroll state and active-app info from the background shell note.
    let (bg_scroll_offset, bg_active_app, bg_detected_bg) =
        if let Some(bg_idx) = app.background_note_idx() {
            if let crate::note::NoteKind::Shell { scroll_offset, active_app, detected_bg, .. } =
                &app.notes[bg_idx].kind
            {
                (*scroll_offset, active_app.clone(), *detected_bg)
            } else {
                (0, None, None)
            }
        } else {
            (0, None, None)
        };

    // Scroll indicator on the left when the background terminal is scrolled
    if bg_scroll_offset != 0 {
        let indicator = if bg_scroll_offset > 0 {
            format!(" ↑ SCROLL  -{} lines  PgDn↓ ", bg_scroll_offset)
        } else {
            format!(" ↓ -{} lines below  PgUp↑ ", bg_scroll_offset.abs())
        };
        let ind_w = (indicator.len() as u16).min(area.width);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                indicator.as_str(),
                Style::default()
                    .fg(NOTEBOOK_CARD_BG)
                    .bg(SPINE_FG)
                    .add_modifier(Modifier::BOLD),
            ))),
            Rect::new(area.x, y, ind_w, 1),
        );
    }

    // Active-app pill: shown when a TUI app is detected in the background terminal.
    if let (Some(app_name), Some(bg)) = (bg_active_app, bg_detected_bg) {
        let pill = format!("  {}  ", app_name);
        let pill_w = (pill.len() as u16).min(area.width);
        let pill_x = area.width.saturating_sub(pill_w);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                pill.as_str(),
                Style::default()
                    .fg(contrast_color(bg))
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ))),
            Rect::new(pill_x, y, pill_w, 1),
        );
    }

    // Show book-mode hints when focused on a note that is a current book page.
    let in_book_mode = !app.notebooks_open.is_empty() && {
        if let Focus::Note(i, _) = &app.focus {
            app.notes[*i].data.notebook_id
                .map_or(false, |nb_id| app.notebooks_open.contains_key(&nb_id))
        } else {
            false
        }
    };

    // Check if the focused shell note is actively logging — used for the REC indicator.
    let active_log_path: Option<String> = match app.focus {
        Focus::BackgroundShell(idx) | Focus::Note(idx, NoteType::Shell) => {
            if let Some(note) = app.notes.get(idx) {
                if let crate::note::NoteKind::Shell { ref log_path, .. } = note.kind {
                    log_path.as_ref()
                        .and_then(|p| p.file_name())
                        .and_then(|n| n.to_str())
                        .map(|s| s.to_string())
                } else { None }
            } else { None }
        }
        _ => None,
    };

    let hint = if in_book_mode && matches!(app.focus, Focus::Note(..)) {
        " Tab: next page  Shift+Tab: prev page  Alt+O: next notebook  Ctrl+R: remove page  Ctrl+P: pin to board  Ctrl+E: shell  Ctrl+K: back to notebook ".to_string()
    } else {
        match app.focus {
            Focus::BackgroundShell(_) =>
                " Alt+Q: quit | Alt+B: board | Alt+N: note | Alt+T: new terminal | Alt+G: focus terminal | Alt+V: paste | Ctrl+V: screenshot | Alt+←/→: workspace | Alt+I: log | PgUp/Dn: scroll | F1: hints ".to_string(),
            Focus::Note(_, NoteType::Text) =>
                " Ctrl+E: shell | Ctrl+W: close | Ctrl+P: pin | Alt+B: board | Ctrl+G: notebook | Tab: cycle | Ctrl+V: visual | Alt+C: copy | Alt+V: paste | Ctrl+T: rename | Ctrl+S: settings ".to_string(),
            Focus::Note(_, NoteType::Shell) =>
                " Ctrl+E: shell | Alt+Space: cycle | Ctrl+W: close | Ctrl+Y: snapshot | Ctrl+B: background | Ctrl+P: pin | Alt+I: log | Alt+V: paste | Alt+C: copy | Ctrl+S: settings | Alt+B: board ".to_string(),
            Focus::Note(_, NoteType::Photo) =>
                " Ctrl+W: close | Ctrl+C: copy | Ctrl+T: rename | Ctrl+P: pin | Ctrl+G: notebook | Ctrl+F: top | Alt+B: board ".to_string(),
            Focus::Note(_, NoteType::CheckList) =>
                " Ctrl+E: shell | Ctrl+W: close | Ctrl+X: toggle | Ctrl+T: rename | Ctrl+S: settings | Ctrl+P: pin | Alt+B: board | Ctrl+G: notebook | Tab: cycle ".to_string(),
            Focus::RenamingWorkspace(_) => " Enter: confirm | Esc: cancel ".to_string(),
            Focus::Renaming(_, _) => " Enter: confirm | Esc: cancel ".to_string(),
            Focus::NamingNotebook(_, _) => " Enter: confirm notebook name | Esc: cancel ".to_string(),
            Focus::Settings(_, _) => " Tab/↑↓: section  ←→: colour  Space: toggle border  Esc: close ".to_string(),
            Focus::Selecting { .. } => " [ SCREENSHOT MODE ]  Drag or hjkl/arrows to select  Enter/y: photo  Alt+C: copy text  Esc: exit ".to_string(),
            Focus::TextVisual { .. } => " [ VISUAL SELECT ]  hjkl/arrows move  w/b word  0/$ line  y/Alt+C: copy  Esc: cancel ".to_string(),
            Focus::LoggingSetup(_, _) => " Enter: start logging | Esc: cancel ".to_string(),
        }
    };

    // ● REC indicator: shown to the left of the hint when a shell is logging.
    let rec_right_offset = if let Some(ref filename) = active_log_path {
        let rec = format!(" ● REC: {} ", filename);
        let rec_w = (rec.len() as u16).min(area.width);
        let rec_x = area.width.saturating_sub(rec_w);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                rec.as_str(),
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(220, 60, 60))
                    .add_modifier(Modifier::BOLD),
            ))),
            Rect::new(rec_x, y, rec_w, 1),
        );
        rec_w
    } else {
        0
    };

    let avail_w = area.width.saturating_sub(rec_right_offset);
    let width = (hint.len() as u16).min(avail_w);
    let x = avail_w.saturating_sub(width);

    let hint_style = if matches!(app.focus, Focus::Selecting { .. } | Focus::TextVisual { .. }) {
        Style::default()
            .fg(Color::Black)
            .bg(Color::Rgb(0, 210, 180))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(NOTEBOOK_CARD_BG)
            .bg(SPINE_BG)
            .add_modifier(Modifier::BOLD)
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(hint.as_str(), hint_style))),
        Rect::new(x, y, width, 1),
    );
}
