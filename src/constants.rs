//! Shared constants for note geometry, scrollback sizes, and scroll behaviour.
//!
//! Centralising these here removes the circular dependency where `note.rs`
//! had to reach into `app.rs` just to get `SHELL_SCROLLBACK`.

// ---------------------------------------------------------------------------
// Note / layout geometry
// ---------------------------------------------------------------------------

/// Minimum note width in terminal columns.
pub const MIN_NOTE_W: u16 = 12;
/// Minimum note height in terminal rows.
pub const MIN_NOTE_H: u16 = 4;

/// Corkboard card width — also used as the spawn width for new text notes.
pub const CARD_W: u16 = 26;
/// Corkboard card height — also used as the spawn height for new text notes.
pub const CARD_H: u16 = 9;
/// Gap between cards in the corkboard grid.
pub const CARD_GAP: u16 = 2;

// ---------------------------------------------------------------------------
// Scrollback buffer sizes
// ---------------------------------------------------------------------------

/// vt100 scrollback capacity for the main background terminal (lines).
#[allow(dead_code)]
pub const MAIN_SCROLLBACK: usize = 10_000;

// ---------------------------------------------------------------------------
// Scroll behaviour
// ---------------------------------------------------------------------------

/// Number of prompt lines to keep visible at the bottom when scrolling a shell
/// past the live view.  Increase if your prompt is taller than 2 lines.
pub const PROMPT_LINES: i64 = 2;

/// Number of rows reserved at the top *and* bottom of the terminal for the
/// workspace tab bar and hint bar.  Background shells (both the main App.pty
/// and per-workspace background notes) are sized and rendered within the
/// remaining area so the bars are never obscured.
pub const BG_SHELL_INSET: u16 = 1;
