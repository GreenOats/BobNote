//! User configuration loaded from `~/.bobrc` (TOML).
//!
//! On first launch the file is created with commented defaults so the user
//! can edit it without consulting any external documentation.

use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

// ---------------------------------------------------------------------------
// KeyBind
// ---------------------------------------------------------------------------

/// A single key binding: modifier mask + key code.
#[derive(Clone, Copy, Debug)]
pub struct KeyBind {
    pub modifiers: KeyModifiers,
    pub code: KeyCode,
}

impl KeyBind {
    #[inline]
    pub fn matches(self, key: KeyEvent) -> bool {
        key.modifiers == self.modifiers && key.code == self.code
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// All user-configurable settings.  Loaded once at startup from `~/.bobrc`.
/// Implements `Copy` so callers can snapshot it with `let cfg = self.config;`
/// and avoid lifetime issues when subsequently mutating other `App` fields.
#[derive(Clone, Copy)]
pub struct Config {
    // ── Shell behaviour ──────────────────────────────────────────────────────
    /// Scrollback buffer depth for each shell note (lines).
    pub shell_scrollback: usize,

    // ── Global (any context) ─────────────────────────────────────────────────
    pub hint_bar: KeyBind,
    pub quit: KeyBind,
    pub corkboard: KeyBind,
    pub new_note: KeyBind,
    pub focus_note: KeyBind,
    pub new_checklist: KeyBind,
    pub new_terminal: KeyBind,
    pub focus_terminal: KeyBind,
    pub focus_book: KeyBind,
    pub rename_workspace: KeyBind,
    pub paste: KeyBind,

    // ── Note movement (note focused) ─────────────────────────────────────────
    pub move_left: KeyBind,
    pub move_right: KeyBind,
    pub move_up: KeyBind,
    pub move_down: KeyBind,

    // ── Note resize (note focused) ───────────────────────────────────────────
    pub resize_left: KeyBind,
    pub resize_right: KeyBind,
    pub resize_up: KeyBind,
    pub resize_down: KeyBind,

    // ── Note management (note focused) ───────────────────────────────────────
    pub return_to_shell: KeyBind,
    pub rename: KeyBind,
    pub note_settings: KeyBind,
    pub pin_to_corkboard: KeyBind,
    pub assign_notebook: KeyBind,
    pub toggle_front: KeyBind,
    pub close_note: KeyBind,
    pub copy: KeyBind,
    pub visual_select: KeyBind,
    pub snapshot_photo: KeyBind,
    pub toggle_item: KeyBind,

    // ── Corkboard notebook page reordering ───────────────────────────────────
    pub reorder_left: KeyBind,
    pub reorder_right: KeyBind,
    pub reorder_up: KeyBind,
    pub reorder_down: KeyBind,
}

impl Config {
    fn defaults() -> Self {
        let kb = |s: &str| parse_keybind(s).expect("invalid default keybind");
        Self {
            shell_scrollback: 1_000,
            hint_bar:         kb("f1"),
            quit:             kb("alt+q"),
            corkboard:        kb("alt+b"),
            new_note:         kb("alt+n"),
            focus_note:       kb("alt+f"),
            new_checklist:    kb("alt+l"),
            new_terminal:     kb("alt+t"),
            focus_terminal:   kb("alt+g"),
            focus_book:       kb("alt+o"),
            rename_workspace: kb("alt+r"),
            paste:            kb("alt+v"),

            move_left:        kb("alt+h"),
            move_right:       kb("alt+l"),
            move_up:          kb("alt+k"),
            move_down:        kb("alt+j"),

            resize_left:      kb("ctrl+alt+h"),
            resize_right:     kb("ctrl+alt+l"),
            resize_up:        kb("ctrl+alt+k"),
            resize_down:      kb("ctrl+alt+j"),

            return_to_shell:  kb("ctrl+e"),
            rename:           kb("ctrl+t"),
            note_settings:    kb("ctrl+s"),
            pin_to_corkboard: kb("ctrl+p"),
            assign_notebook:  kb("ctrl+g"),
            toggle_front:     kb("ctrl+f"),
            close_note:       kb("ctrl+w"),
            copy:             kb("ctrl+c"),
            visual_select:    kb("ctrl+v"),
            snapshot_photo:   kb("ctrl+y"),
            toggle_item:      kb("ctrl+x"),

            reorder_left:     kb("ctrl+left"),
            reorder_right:    kb("ctrl+right"),
            reorder_up:       kb("ctrl+up"),
            reorder_down:     kb("ctrl+down"),
        }
    }

    /// Load config from `~/.bobrc`, creating it with defaults if absent.
    /// Falls back to built-in defaults on any error (parse or I/O).
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(cfg) => cfg,
            Err(e) => {
                eprintln!("BobNote: config error — {e}. Using built-in defaults.");
                Self::defaults()
            }
        }
    }

    fn try_load() -> Result<Self> {
        let path = {
            let home = dirs::home_dir().context("cannot determine home directory")?;
            home.join(".bobrc")
        };

        if !path.exists() {
            std::fs::write(&path, DEFAULT_RC)
                .with_context(|| format!("creating {}", path.display()))?;
            return Ok(Self::defaults());
        }

        let src = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let raw: toml::Table = src.parse()
            .with_context(|| format!("parsing {}", path.display()))?;

        let mut cfg = Self::defaults();

        macro_rules! override_bind {
            ($field:ident, $key:literal) => {
                if let Some(toml::Value::String(s)) = raw.get($key) {
                    match parse_keybind(s) {
                        Ok(kb) => cfg.$field = kb,
                        Err(e) => eprintln!(
                            "BobNote: bad binding for `{}` ({e}), keeping default",
                            $key
                        ),
                    }
                }
            };
        }

        if let Some(toml::Value::Integer(n)) = raw.get("shell_scrollback") {
            if *n > 0 {
                cfg.shell_scrollback = *n as usize;
            } else {
                eprintln!("BobNote: shell_scrollback must be > 0, keeping default");
            }
        }

        override_bind!(hint_bar,         "hint_bar");
        override_bind!(quit,             "quit");
        override_bind!(corkboard,        "corkboard");
        override_bind!(new_note,         "new_note");
        override_bind!(focus_note,       "focus_note");
        override_bind!(new_checklist,    "new_checklist");
        override_bind!(new_terminal,     "new_terminal");
        override_bind!(focus_terminal,   "focus_terminal");
        override_bind!(focus_book,       "focus_book");
        override_bind!(rename_workspace, "rename_workspace");
        override_bind!(paste,            "paste");
        override_bind!(move_left,        "move_left");
        override_bind!(move_right,       "move_right");
        override_bind!(move_up,          "move_up");
        override_bind!(move_down,        "move_down");
        override_bind!(resize_left,      "resize_left");
        override_bind!(resize_right,     "resize_right");
        override_bind!(resize_up,        "resize_up");
        override_bind!(resize_down,      "resize_down");
        override_bind!(return_to_shell,  "return_to_shell");
        override_bind!(rename,           "rename");
        override_bind!(note_settings,    "note_settings");
        override_bind!(pin_to_corkboard, "pin_to_corkboard");
        override_bind!(assign_notebook,  "assign_notebook");
        override_bind!(toggle_front,     "toggle_front");
        override_bind!(close_note,       "close_note");
        override_bind!(copy,             "copy");
        override_bind!(visual_select,    "visual_select");
        override_bind!(snapshot_photo,   "snapshot_photo");
        override_bind!(toggle_item,      "toggle_item");
        override_bind!(reorder_left,     "reorder_left");
        override_bind!(reorder_right,    "reorder_right");
        override_bind!(reorder_up,       "reorder_up");
        override_bind!(reorder_down,     "reorder_down");

        Ok(cfg)
    }
}

// ---------------------------------------------------------------------------
// Key bind parser
// ---------------------------------------------------------------------------

fn parse_keybind(s: &str) -> Result<KeyBind> {
    let s = s.trim().to_lowercase();
    let parts: Vec<&str> = s.split('+').collect();
    if parts.is_empty() {
        anyhow::bail!("empty key binding");
    }

    let (mod_parts, key_slice) = parts.split_at(parts.len() - 1);
    let key_str = key_slice[0];

    let mut modifiers = KeyModifiers::NONE;
    for m in mod_parts {
        match *m {
            "alt"              => modifiers |= KeyModifiers::ALT,
            "ctrl" | "control" => modifiers |= KeyModifiers::CONTROL,
            "shift"            => modifiers |= KeyModifiers::SHIFT,
            other              => anyhow::bail!("unknown modifier: {other}"),
        }
    }

    let code = match key_str {
        "esc" | "escape"       => KeyCode::Esc,
        "enter" | "return"     => KeyCode::Enter,
        "tab"                  => KeyCode::Tab,
        "backtab"              => KeyCode::BackTab,
        "backspace"            => KeyCode::Backspace,
        "delete" | "del"       => KeyCode::Delete,
        "insert" | "ins"       => KeyCode::Insert,
        "home"                 => KeyCode::Home,
        "end"                  => KeyCode::End,
        "pageup"   | "pgup"    => KeyCode::PageUp,
        "pagedown" | "pgdn"    => KeyCode::PageDown,
        "up"                   => KeyCode::Up,
        "down"                 => KeyCode::Down,
        "left"                 => KeyCode::Left,
        "right"                => KeyCode::Right,
        "f1"  => KeyCode::F(1),  "f2"  => KeyCode::F(2),
        "f3"  => KeyCode::F(3),  "f4"  => KeyCode::F(4),
        "f5"  => KeyCode::F(5),  "f6"  => KeyCode::F(6),
        "f7"  => KeyCode::F(7),  "f8"  => KeyCode::F(8),
        "f9"  => KeyCode::F(9),  "f10" => KeyCode::F(10),
        "f11" => KeyCode::F(11), "f12" => KeyCode::F(12),
        s if s.chars().count() == 1 => KeyCode::Char(s.chars().next().unwrap()),
        other => anyhow::bail!("unknown key: {other}"),
    };

    Ok(KeyBind { modifiers, code })
}

// ---------------------------------------------------------------------------
// Default ~/.bobrc contents
// ---------------------------------------------------------------------------

const DEFAULT_RC: &str = r#"# BobNote configuration (~/.bobrc)
#
# Key bindings use the format: "modifier+key"
# Modifiers: alt, ctrl  (avoid shift — terminal compatibility varies)
# Keys: a-z, 0-9, f1-f12, esc, enter, tab, backspace,
#       up, down, left, right, pageup, pagedown, home, end, insert, delete
#
# Examples:
#   "alt+n"      → Alt+N
#   "ctrl+w"     → Ctrl+W
#   "ctrl+alt+h" → Ctrl+Alt+H
#   "f1"         → F1 (no modifier needed)

# ── Shell behaviour ────────────────────────────────────────────────────────

# Number of lines kept in the scrollback buffer for each shell note.
# Higher values let you scroll further back but use more memory.
shell_scrollback = 1000

# ── Global ─────────────────────────────────────────────────────────────────
# These work from any context (shell, note, corkboard).

hint_bar      = "f1"       # Toggle the hint bar
quit          = "alt+q"    # Quit BobNote
corkboard     = "alt+b"    # Open / close the corkboard
new_note      = "alt+n"    # Create a new text note
focus_note    = "alt+f"    # Focus the topmost visible text note
new_checklist = "alt+l"    # New checklist  (or focus topmost if one exists)
new_terminal   = "alt+t"    # New terminal note (always creates a new one)
focus_terminal = "alt+g"    # Focus the topmost visible terminal note
focus_book     = "alt+o"    # Focus the open book / cycle notebooks
rename_workspace = "alt+r"  # Rename the active workspace
paste         = "alt+v"    # Paste from system clipboard

# ── Note movement ──────────────────────────────────────────────────────────
# Active when any note has focus.  Alt+arrows are always available as aliases.

move_left  = "alt+h"   # Move note left
move_right = "alt+l"   # Move note right
move_up    = "alt+k"   # Move note up
move_down  = "alt+j"   # Move note down

# ── Note resize ────────────────────────────────────────────────────────────
# Active when any note has focus.  Ctrl+Alt+arrows are always available as aliases.

resize_left  = "ctrl+alt+h"   # Shrink note width
resize_right = "ctrl+alt+l"   # Grow note width
resize_up    = "ctrl+alt+k"   # Shrink note height
resize_down  = "ctrl+alt+j"   # Grow note height

# ── Note management ────────────────────────────────────────────────────────
# Active when a note has focus.

return_to_shell  = "ctrl+e"   # Return focus to the background shell
rename           = "ctrl+t"   # Rename the note
note_settings    = "ctrl+s"   # Open note settings (border, colour, wrap)
pin_to_corkboard = "ctrl+p"   # Pin note to corkboard
assign_notebook  = "ctrl+g"   # Assign note to a notebook
toggle_front     = "ctrl+f"   # Toggle always-on-top pin
close_note       = "ctrl+w"   # Close / delete the note
copy             = "ctrl+c"   # Copy selection to clipboard (text notes)
visual_select    = "ctrl+v"   # Enter visual-select mode
snapshot_photo   = "ctrl+y"   # Snapshot shell note as a photo note
toggle_item      = "ctrl+x"   # Toggle checklist item [ ]/[x]

# ── Corkboard ──────────────────────────────────────────────────────────────
# Page reordering inside a notebook sub-grid (replaces old Shift+arrows).

reorder_left  = "ctrl+left"   # Move selected page left
reorder_right = "ctrl+right"  # Move selected page right
reorder_up    = "ctrl+up"     # Move selected page up
reorder_down  = "ctrl+down"   # Move selected page down
"#;
