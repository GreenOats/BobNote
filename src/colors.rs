use ratatui::style::Color;

/// Regular/saturated colours for the border picker.
pub const BORDER_PALETTE: &[(Color, &str)] = &[
    (Color::Yellow,   "Yellow"),
    (Color::Green,    "Green"),
    (Color::Cyan,     "Cyan"),
    (Color::Blue,     "Blue"),
    (Color::Red,      "Red"),
    (Color::Magenta,  "Magenta"),
    (Color::White,    "White"),
    (Color::Gray,     "Gray"),
    (Color::DarkGray, "Dark Gray"),
    (Color::Black,    "Black"),
];

/// Light colours for the background picker.
/// Index 0 is always "None" (transparent / terminal default).
pub const BG_PALETTE: &[(Color, &str)] = &[
    (Color::Reset,        "None"),
    (Color::LightYellow,  "Lt Yellow"),
    (Color::LightGreen,   "Lt Green"),
    (Color::LightCyan,    "Lt Cyan"),
    (Color::LightBlue,    "Lt Blue"),
    (Color::LightRed,     "Lt Red"),
    (Color::LightMagenta, "Lt Magenta"),
    (Color::White,        "White"),
    (Color::Gray,         "Gray"),
];

/// Returns a readable foreground colour for the given background.
/// Dark terminal colours (Blue, Red, Magenta, DarkGray, Black) get White text;
/// everything else (light variants, Yellow, Cyan, Green, Gray) gets Black text;
/// transparent (Reset) returns Reset so the terminal's own foreground shows through.
pub fn contrast_color(bg: Color) -> Color {
    match bg {
        Color::Reset => Color::Reset,
        Color::Black | Color::DarkGray
        | Color::Blue | Color::Red | Color::Magenta => Color::White,
        _ => Color::Black,
    }
}
