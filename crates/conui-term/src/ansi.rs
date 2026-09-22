//! The escape sequences conui emits, in one place.
//!
//! Named constants rather than inline string literals: an escape sequence is unreadable at
//! the point of use, and a typo in one produces a visual glitch that is miserable to trace
//! back to its source.

/// Control Sequence Introducer.
pub const CSI: &str = "\x1b[";

// ---- Screen and cursor lifecycle -------------------------------------------------------

/// Switch to the alternate screen buffer. The user's scrollback is left untouched, and
/// restoring it on exit is what makes a full-screen app feel like it was never there.
pub const ENTER_ALT_SCREEN: &str = "\x1b[?1049h";
pub const LEAVE_ALT_SCREEN: &str = "\x1b[?1049l";

pub const HIDE_CURSOR: &str = "\x1b[?25l";
pub const SHOW_CURSOR: &str = "\x1b[?25h";

/// Disable autowrap (DECAWM).
///
/// With wrap on, writing to the last column leaves the terminal in a "pending wrap" state,
/// and the next glyph scrolls the whole screen up by a line. For a fixed grid that is never
/// what we want: the grid defines where things go, so overflow should be discarded.
pub const DISABLE_AUTOWRAP: &str = "\x1b[?7l";
pub const ENABLE_AUTOWRAP: &str = "\x1b[?7h";

/// Clear the entire screen, ignoring scrollback.
pub const CLEAR_SCREEN: &str = "\x1b[2J";
/// Clear from the cursor to the end of the line.
pub const CLEAR_TO_LINE_END: &str = "\x1b[K";

/// Move the cursor home, `1;1`.
pub const CURSOR_HOME: &str = "\x1b[H";

/// Reset every graphic rendition to the terminal default.
pub const RESET_STYLE: &str = "\x1b[0m";

// ---- Synchronized output ---------------------------------------------------------------

/// Begin an atomic frame (DECSET 2026).
///
/// The terminal buffers everything until the matching end and then presents it in one go, so
/// a frame is never shown half-drawn. This is what removes tearing on a fast repaint.
pub const BEGIN_SYNC: &str = "\x1b[?2026h";
pub const END_SYNC: &str = "\x1b[?2026l";

// ---- Input protocols -------------------------------------------------------------------

/// Report mouse press, release and motion-while-dragging, in SGR encoding.
///
/// 1002 is drag-only motion rather than 1003's every-pixel reporting: 1003 floods the input
/// stream with events an app almost never uses. 1006 selects SGR encoding, which lifts the
/// 223-column ceiling of the original X10 scheme.
pub const ENABLE_MOUSE: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
pub const DISABLE_MOUSE: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";

/// Bracket pasted text with `ESC [ 200 ~` and `ESC [ 201 ~`.
pub const ENABLE_BRACKETED_PASTE: &str = "\x1b[?2004h";
pub const DISABLE_BRACKETED_PASTE: &str = "\x1b[?2004l";

/// Report window focus as `ESC [ I` and `ESC [ O`, so an app can dim when unfocused.
pub const ENABLE_FOCUS_EVENTS: &str = "\x1b[?1004h";
pub const DISABLE_FOCUS_EVENTS: &str = "\x1b[?1004l";

/// SGR attribute codes, matching [`conui_cell::Attrs`] bit order.
pub mod sgr {
    pub const BOLD: u16 = 1;
    pub const DIM: u16 = 2;
    pub const ITALIC: u16 = 3;
    pub const UNDERLINE: u16 = 4;
    pub const BLINK: u16 = 5;
    pub const REVERSE: u16 = 7;
    pub const HIDDEN: u16 = 8;
    pub const STRIKETHROUGH: u16 = 9;
}

/// Map a single [`conui_cell::Attrs`] flag to its SGR code.
pub fn attr_code(attr: conui_cell::Attrs) -> Option<u16> {
    use conui_cell::Attrs;
    Some(match attr {
        Attrs::BOLD => sgr::BOLD,
        Attrs::DIM => sgr::DIM,
        Attrs::ITALIC => sgr::ITALIC,
        Attrs::UNDERLINE => sgr::UNDERLINE,
        Attrs::BLINK => sgr::BLINK,
        Attrs::REVERSE => sgr::REVERSE,
        Attrs::HIDDEN => sgr::HIDDEN,
        Attrs::STRIKETHROUGH => sgr::STRIKETHROUGH,
        _ => return None,
    })
}
