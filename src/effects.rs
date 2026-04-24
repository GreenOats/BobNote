use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
};

/// Shadow colour used in the main note view.
pub const NOTE_SHADOW: Color = Color::Rgb(20, 20, 20);

/// Maps each `BG_PALETTE` light colour to its saturated counterpart for the
/// occlusion-dim effect. Transparent (`Reset`) and any unrecognised colour are
/// returned unchanged.
pub fn darken_bg(color: Color) -> Color {
    match color {
        Color::LightYellow  => Color::Yellow,
        Color::LightGreen   => Color::Green,
        Color::LightCyan    => Color::Cyan,
        Color::LightBlue    => Color::Blue,
        Color::LightRed     => Color::Red,
        Color::LightMagenta => Color::Magenta,
        Color::White        => Color::Gray,
        Color::Gray         => Color::DarkGray,
        other               => other,
    }
}

/// Shadow colour used inside the corkboard (warm dark-cork tone).
pub const CORK_SHADOW: Color = Color::Rgb(30, 18, 6);

/// Returns true if two rects share at least one cell.
pub fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x < b.x + b.width
        && a.x + a.width > b.x
        && a.y < b.y + b.height
        && a.y + a.height > b.y
}

/// Draw a 1-cell drop-shadow to the right and below `area`, clipped to `bounds`.
///
/// Layout (● = shadow cell):
/// ```
///  ┌──────┐
///  │ area │●   <- right strip (rows y+1 .. y+h-1)
///  │      │●
///  └──────┘●
///   ●●●●●●●   <- bottom strip (cols x+1 .. x+w, includes corner)
/// ```
pub fn draw_drop_shadow(frame: &mut Frame, area: Rect, bounds: Rect, color: Color) {
    let style = Style::default().bg(color);
    let buf = frame.buffer_mut();

    // Right strip: single-cell column, skips the top-right corner row.
    // The bottom-right corner is covered by the bottom strip instead.
    let rx = area.x + area.width;
    if rx < bounds.x + bounds.width {
        for row in 1..area.height {
            let ry = area.y + row;
            if ry >= bounds.y + bounds.height { break; }
            if let Some(cell) = buf.cell_mut((rx, ry)) {
                cell.set_symbol(" ");
                cell.set_style(style);
            }
        }
    }

    // Bottom strip: full width starting one cell in, includes corner.
    let by = area.y + area.height;
    if by < bounds.y + bounds.height {
        let bx = area.x + 1;
        let max_w = (bounds.x + bounds.width).saturating_sub(bx);
        let bw = area.width.min(max_w);
        for col in 0..bw {
            if let Some(cell) = buf.cell_mut((bx + col, by)) {
                cell.set_symbol(" ");
                cell.set_style(style);
            }
        }
    }
}
