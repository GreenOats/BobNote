use std::collections::{VecDeque, hash_map::DefaultHasher};
use std::hash::{Hash, Hasher};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::Widget,
};
use crate::note::{PhotoCell, PhotoRow, SerColor};

// ---------------------------------------------------------------------------
// Shared cell type
// ---------------------------------------------------------------------------

/// One captured terminal cell: (symbol, fg, bg, modifiers).
/// Using String for the symbol handles multi-byte / wide characters correctly.
pub type CapturedRow = Vec<(String, Color, Color, Modifier)>;

// ---------------------------------------------------------------------------
// PtyView — live vt100 screen renderer
// ---------------------------------------------------------------------------

/// Wraps a vt100 screen so it can be used as a ratatui Widget.
///
/// `row_offset` shifts which screen row maps to the top of the rendered area.
/// - `0` → normal live view.
/// - `N > 0` → the visible window starts at screen row N (downward scroll).
pub struct PtyView<'a>(pub &'a vt100::Screen, pub usize);

impl Widget for PtyView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for row in 0..area.height {
            let src_row = row as usize + self.1;
            for col in 0..area.width {
                let Some(cell) = self.0.cell(src_row as u16, col) else {
                    continue;
                };

                let contents = cell.contents();
                if contents.is_empty() {
                    continue;
                }

                let style = Style::default()
                    .fg(map_color(cell.fgcolor()))
                    .bg(map_color(cell.bgcolor()))
                    .add_modifier(attrs(cell));

                let x = area.x + col;
                let y = area.y + row;

                if let Some(buf_cell) = buf.cell_mut((x, y)) {
                    buf_cell.set_symbol(&contents);
                    buf_cell.set_style(style);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// OwnScrollbackView — renders from our captured ring buffer
// ---------------------------------------------------------------------------

/// Renders rows from an own-scrollback ring buffer instead of a live vt100 screen.
/// Used when `scroll_offset` exceeds what vt100 can show without panicking.
pub struct OwnScrollbackView<'a> {
    pub rows: &'a VecDeque<CapturedRow>,
    /// Index into `rows` that maps to the top of the rendered area.
    pub top_idx: usize,
}

impl Widget for OwnScrollbackView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for row in 0..area.height as usize {
            let src_idx = self.top_idx + row;
            if src_idx >= self.rows.len() {
                break;
            }
            let src_row = &self.rows[src_idx];
            for col in 0..(area.width as usize).min(src_row.len()) {
                let (sym, fg, bg, modifier) = &src_row[col];
                if sym.is_empty() {
                    continue;
                }
                let style = Style::default().fg(*fg).bg(*bg).add_modifier(*modifier);
                if let Some(buf_cell) = buf.cell_mut((area.x + col as u16, area.y + row as u16)) {
                    buf_cell.set_symbol(sym);
                    buf_cell.set_style(style);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scrollback capture helpers
// ---------------------------------------------------------------------------

/// Content fingerprint for one vt100 screen row.
/// Hashes character content + foreground colour so two rows with identical text
/// but different colours compare unequal.
fn row_fp(screen: &vt100::Screen, row: u16) -> u64 {
    let mut h = DefaultHasher::new();
    let cols = screen.size().1;
    for c in 0..cols {
        if let Some(cell) = screen.cell(row, c) {
            cell.contents().hash(&mut h);
            match cell.fgcolor() {
                vt100::Color::Default => 0u8.hash(&mut h),
                vt100::Color::Idx(i) => { 1u8.hash(&mut h); i.hash(&mut h); }
                vt100::Color::Rgb(r, g, b) => {
                    2u8.hash(&mut h);
                    r.hash(&mut h);
                    g.hash(&mut h);
                    b.hash(&mut h);
                }
            }
        }
    }
    h.finish()
}

/// Read one vt100 screen row into a `CapturedRow`.
fn capture_row(screen: &vt100::Screen, row: u16) -> CapturedRow {
    let cols = screen.size().1;
    (0..cols)
        .map(|c| {
            if let Some(cell) = screen.cell(row, c) {
                (
                    cell.contents(),
                    map_color(cell.fgcolor()),
                    map_color(cell.bgcolor()),
                    attrs(cell),
                )
            } else {
                (String::new(), Color::Reset, Color::Reset, Modifier::empty())
            }
        })
        .collect()
}

/// Capture any rows that have newly entered vt100's scrollback buffer into `own_sb`.
///
/// `rows` is the parser's visible row count — the maximum safe `set_scrollback` value.
/// `capacity` is the ring-buffer cap (older rows are evicted when exceeded).
///
/// Returns the vt100 accessible scrollback depth after the call, which the run-loop
/// uses to compute a dynamic scroll cap.
///
/// # How it works
/// vt100's `visible_rows()` does `take(rows_len - scrollback_offset)` as a plain
/// usize subtraction, so `scrollback_offset` must never exceed `rows_len`.  We call
/// `set_scrollback(rows)` (the maximum safe value) and read back `screen().scrollback()`
/// which is clamped to the actual buffer length.  The accessible window therefore
/// covers at most `rows` lines of scrollback.
///
/// To detect which lines are *new* since the last frame we store a fingerprint
/// (`prev_fps`) of the previously accessible scrollback rows.  We locate the last
/// known row in the current window via `rposition` and append everything after it.
/// If the entire window has rotated (a burst of more than `rows` new lines in one
/// frame), we fall back to capturing the whole window — we may miss some intermediate
/// lines in that extreme case, but the buffer stays consistent.
/// Returns `(depth, new_count)`:
/// - `depth`     — vt100's accessible scrollback depth after the call.
/// - `new_count` — number of newly discovered scrollback rows (= vt100 scroll
///                 events since the previous call).  The caller can use this to
///                 advance a negative scroll_offset so that output fills blank
///                 space below the prompt rather than appearing to jump.
pub fn capture_scrollback_rows(
    parser: &mut vt100::Parser,
    own_sb: &mut VecDeque<CapturedRow>,
    prev_fps: &mut Vec<u64>,
    rows: u16,
    capacity: usize,
) -> (usize, usize) {
    let max_safe = rows as usize;
    parser.set_scrollback(max_safe);
    let depth = parser.screen().scrollback(); // = min(actual_scrollback_len, rows)

    if depth == 0 {
        return (0, 0);
    }

    // Fingerprint all currently accessible scrollback rows (0 .. depth).
    let current_fps: Vec<u64> = (0..depth as u16)
        .map(|r| row_fp(parser.screen(), r))
        .collect();

    // Find where new rows begin: locate the newest previously-captured row
    // (prev_fps.last()) in the current window via rposition (right-to-left search
    // handles the common case where it sits near the end).
    let new_start = prev_fps
        .last()
        .and_then(|&last_fp| current_fps.iter().rposition(|&fp| fp == last_fp))
        .map(|pos| pos + 1)
        .unwrap_or(0);

    // Capture and append newly discovered rows.
    for r in new_start..depth {
        let row = capture_row(parser.screen(), r as u16);
        own_sb.push_back(row);
        if own_sb.len() > capacity {
            own_sb.pop_front();
        }
    }

    *prev_fps = current_fps;
    (depth, depth - new_start)
}

/// Snapshot every visible row of the vt100 screen into `own_sb` before a
/// resize operation.
///
/// `parser.set_size()` discards rows that no longer fit (Y-shrink) and clips
/// cell content to the new column count (X-shrink).  Calling this first
/// preserves the pre-resize state in the ring buffer so the user can still
/// scroll back to it.
///
/// Rows that are completely blank are skipped — they carry no content and
/// would needlessly consume ring-buffer capacity.  Internal blank rows
/// (between non-empty rows) are kept so vertical spacing is preserved.
///
/// `prev_fps` is cleared so that the next `capture_scrollback_rows` call
/// starts with a clean fingerprint baseline and doesn't confuse the
/// now-stale vt100 scrollback state with content we just saved ourselves.
pub fn capture_screen_before_resize(
    parser: &vt100::Parser,
    own_sb: &mut VecDeque<CapturedRow>,
    prev_fps: &mut Vec<u64>,
    capacity: usize,
) {
    let screen = parser.screen();
    let (rows, _) = screen.size();

    // Capture all rows; trim leading and trailing blank rows while keeping
    // any blank rows sandwiched between content rows.
    let all: Vec<CapturedRow> = (0..rows).map(|r| capture_row(screen, r)).collect();
    let is_blank = |row: &CapturedRow| row.iter().all(|(s, ..)| s.is_empty());

    let first = all.iter().position(|r| !is_blank(r));
    let last  = all.iter().rposition(|r| !is_blank(r));

    if let (Some(f), Some(l)) = (first, last) {
        for row in all.into_iter().skip(f).take(l - f + 1) {
            own_sb.push_back(row);
            if own_sb.len() > capacity {
                own_sb.pop_front();
            }
        }
    }

    // Invalidate fingerprints — the vt100 grid is about to change, so the
    // old baseline is meaningless for the next capture_scrollback_rows call.
    prev_fps.clear();
}

// ---------------------------------------------------------------------------
// Photo capture
// ---------------------------------------------------------------------------

/// Capture a rectangular region from a shell's **fully-rendered** view.
///
/// Unlike `capture_region`, which reads only from the vt100 parser, this
/// function also consults `own_scrollback` for rows that have scrolled past
/// what vt100 can reach (when `scroll_offset > vt100_depth`).
///
/// `row1`/`row2` are **shell-area-relative** (0-based from the shell area top,
/// i.e., with `BG_SHELL_INSET` already subtracted from screen-absolute rows).
/// `col1`/`col2` are ordinary column indices.
///
/// The parser must have `set_scrollback` already applied by the run loop so
/// that `parser.screen().scrollback()` reflects the current scroll depth.
pub fn capture_rendered_region(
    parser: &vt100::Parser,
    scroll_offset: i64,
    own_scrollback: &VecDeque<CapturedRow>,
    row1: u16, col1: u16,
    row2: u16, col2: u16,
) -> Vec<PhotoRow> {
    let vt100_depth = parser.screen().scrollback() as i64;
    // How many visual rows at the top come from own_scrollback instead of vt100.
    let own_sb_rows_needed = (scroll_offset - vt100_depth).max(0) as usize;
    // Index into own_scrollback for the first visible row.
    let top_idx = own_scrollback.len()
        .saturating_sub(scroll_offset.max(0) as usize);
    // When scroll_offset is negative the parser renders from this row downward
    // (Alacritty-style blank rows above the prompt).
    let parser_row_base = scroll_offset.min(0).unsigned_abs() as u16;

    let r1 = row1.min(row2);
    let r2 = row1.max(row2);
    let c1 = col1.min(col2);
    let c2 = col1.max(col2);

    (r1..=r2).map(|r| {
        (c1..=c2).map(|c| {
            if (r as usize) < own_sb_rows_needed {
                // Row is served by own_scrollback in the renderer.
                let sb_idx = top_idx + r as usize;
                own_scrollback
                    .get(sb_idx)
                    .and_then(|row_data| row_data.get(c as usize))
                    .map(|(sym, fg, bg, modifier)| PhotoCell {
                        sym: sym.clone(),
                        fg: SerColor::from(*fg),
                        bg: SerColor::from(*bg),
                        bold:      modifier.contains(Modifier::BOLD),
                        italic:    modifier.contains(Modifier::ITALIC),
                        underline: modifier.contains(Modifier::UNDERLINED),
                        reversed:  modifier.contains(Modifier::REVERSED),
                    })
                    .unwrap_or_default()
            } else {
                // Row is served by PtyView in the renderer.
                let parser_row = if own_sb_rows_needed > 0 {
                    // In own_sb territory: PtyView starts at parser row 0 below the
                    // own_sb block, so subtract the own_sb offset.
                    (r as usize - own_sb_rows_needed) as u16
                } else {
                    // Not in own_sb territory; apply negative-scroll base offset.
                    r + parser_row_base
                };
                parser.screen().cell(parser_row, c)
                    .map_or_else(PhotoCell::default, |cell| PhotoCell {
                        sym:       cell.contents(),
                        fg:        SerColor::from(map_color(cell.fgcolor())),
                        bg:        SerColor::from(map_color(cell.bgcolor())),
                        bold:      cell.bold(),
                        italic:    cell.italic(),
                        underline: cell.underline(),
                        reversed:  cell.inverse(),
                    })
            }
        }).collect()
    }).collect()
}

/// Capture a rectangular region of the vt100 screen into a `Vec<PhotoRow>`.
/// Coordinates are inclusive and automatically normalised (min/max).
pub fn capture_region(
    screen: &vt100::Screen,
    col1: u16, row1: u16,
    col2: u16, row2: u16,
) -> Vec<PhotoRow> {
    let r1 = row1.min(row2);
    let r2 = row1.max(row2);
    let c1 = col1.min(col2);
    let c2 = col1.max(col2);
    (r1..=r2)
        .map(|row| {
            (c1..=c2)
                .map(|col| {
                    screen.cell(row, col).map_or_else(PhotoCell::default, |cell| PhotoCell {
                        sym: cell.contents(),
                        fg: SerColor::from(map_color(cell.fgcolor())),
                        bg: SerColor::from(map_color(cell.bgcolor())),
                        bold: cell.bold(),
                        italic: cell.italic(),
                        underline: cell.underline(),
                        reversed: cell.inverse(),
                    })
                })
                .collect()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// PhotoView — renders a captured photo into the ratatui buffer
// ---------------------------------------------------------------------------

pub struct PhotoView<'a> {
    pub rows: &'a [PhotoRow],
    pub top_idx: usize,
}

impl Widget for PhotoView<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for (row_idx, src_row) in self.rows
            .iter()
            .skip(self.top_idx)
            .take(area.height as usize)
            .enumerate()
        {
            for (col_idx, cell) in src_row.iter().take(area.width as usize).enumerate() {
                if cell.sym.is_empty() {
                    continue;
                }
                let mut style = Style::default()
                    .fg(Color::from(cell.fg.clone()))
                    .bg(Color::from(cell.bg.clone()));
                if cell.bold      { style = style.add_modifier(Modifier::BOLD); }
                if cell.italic    { style = style.add_modifier(Modifier::ITALIC); }
                if cell.underline { style = style.add_modifier(Modifier::UNDERLINED); }
                if cell.reversed  { style = style.add_modifier(Modifier::REVERSED); }
                if let Some(buf_cell) = buf.cell_mut((area.x + col_idx as u16, area.y + row_idx as u16)) {
                    buf_cell.set_symbol(&cell.sym);
                    buf_cell.set_style(style);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// SelectionOverlay — highlights a rectangular region without erasing content
// ---------------------------------------------------------------------------

/// Applies `Modifier::REVERSED` to every cell in `area`, turning the existing
/// terminal content into a visible selection highlight.
pub struct SelectionOverlay;

impl Widget for SelectionOverlay {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for row in 0..area.height {
            for col in 0..area.width {
                if let Some(cell) = buf.cell_mut((area.x + col, area.y + row)) {
                    let style = cell.style().add_modifier(Modifier::REVERSED);
                    cell.set_style(style);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Colour / attribute helpers (shared by PtyView and capture_row)
// ---------------------------------------------------------------------------

pub fn map_color(c: vt100::Color) -> Color {
    match c {
        vt100::Color::Default => Color::Reset,
        vt100::Color::Idx(i) => Color::Indexed(i),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

fn attrs(cell: &vt100::Cell) -> Modifier {
    let mut m = Modifier::empty();
    if cell.bold() {
        m |= Modifier::BOLD;
    }
    if cell.italic() {
        m |= Modifier::ITALIC;
    }
    if cell.underline() {
        m |= Modifier::UNDERLINED;
    }
    if cell.inverse() {
        m |= Modifier::REVERSED;
    }
    m
}
