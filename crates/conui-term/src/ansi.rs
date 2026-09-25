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
/// Switch back to the main screen buffer, putting the user's shell back as it was.
pub const LEAVE_ALT_SCREEN: &str = "\x1b[?1049l";

/// Hide the hardware cursor. A UI draws its own caret where it wants one.
pub const HIDE_CURSOR: &str = "\x1b[?25l";
/// Show the hardware cursor again.
pub const SHOW_CURSOR: &str = "\x1b[?25h";

/// Disable autowrap (DECAWM).
///
/// With wrap on, writing to the last column leaves the terminal in a "pending wrap" state,
/// and the next glyph scrolls the whole screen up by a line. For a fixed grid that is never
/// what we want: the grid defines where things go, so overflow should be discarded.
pub const DISABLE_AUTOWRAP: &str = "\x1b[?7l";
/// Re-enable autowrap (DECAWM), which is how a terminal is normally left.
pub const ENABLE_AUTOWRAP: &str = "\x1b[?7h";

/// Clear the entire screen, ignoring scrollback.
pub const CLEAR_SCREEN: &str = "\x1b[2J";
/// Clear from the cursor to the end of the line.
pub const CLEAR_TO_LINE_END: &str = "\x1b[K";

/// Move the cursor home, `1;1`.
pub const CURSOR_HOME: &str = "\x1b[H";

/// Reset every graphic rendition to the terminal default.
pub const RESET_STYLE: &str = "\x1b[0m";

// ---- The terminal's own background ------------------------------------------------------

/// Put the terminal's default background back the way the user configured it (OSC 111).
///
/// Sent on the way out, and unconditionally from the panic path: a terminal left holding an app's
/// background is a terminal the user has to restart.
pub const RESET_BACKGROUND: &str = "\x1b]111\x1b\\";

/// Tell the terminal its own default background is this colour (OSC 11).
///
/// The one colour a grid cannot reach. A terminal draws a few pixels of padding around its cells and
/// fills them with its *own* background, not with any cell — so an app whose theme is lighter than
/// the terminal it is running in gets a dark frame around the whole screen, which no amount of
/// clearing to the theme's ground will touch. This is the only sequence that closes that gap.
///
/// Allocates, unlike everything else here, because the colour is not known until run time. It is
/// emitted twice a run at most.
pub fn set_background(red: u8, green: u8, blue: u8) -> String {
    format!("\x1b]11;rgb:{red:02x}/{green:02x}/{blue:02x}\x1b\\")
}

// ---- Synchronized output ---------------------------------------------------------------

/// Begin an atomic frame (DECSET 2026).
///
/// The terminal buffers everything until the matching end and then presents it in one go, so
/// a frame is never shown half-drawn. This is what removes tearing on a fast repaint.
pub const BEGIN_SYNC: &str = "\x1b[?2026h";
/// End an atomic frame, presenting everything written since [`BEGIN_SYNC`].
pub const END_SYNC: &str = "\x1b[?2026l";

// ---- Input protocols -------------------------------------------------------------------

/// Report mouse press, release and motion-while-dragging, in SGR encoding.
///
/// 1002 is drag-only motion rather than 1003's every-pixel reporting: 1003 floods the input
/// stream with events an app almost never uses. 1006 selects SGR encoding, which lifts the
/// 223-column ceiling of the original X10 scheme.
pub const ENABLE_MOUSE: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
/// Stop mouse reporting, unwinding the three modes in the order they were set.
pub const DISABLE_MOUSE: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";

/// Bracket pasted text with `ESC [ 200 ~` and `ESC [ 201 ~`.
pub const ENABLE_BRACKETED_PASTE: &str = "\x1b[?2004h";
/// Stop bracketing pasted text.
pub const DISABLE_BRACKETED_PASTE: &str = "\x1b[?2004l";

/// Report window focus as `ESC [ I` and `ESC [ O`, so an app can dim when unfocused.
pub const ENABLE_FOCUS_EVENTS: &str = "\x1b[?1004h";
/// Stop reporting window focus.
pub const DISABLE_FOCUS_EVENTS: &str = "\x1b[?1004l";

/// SGR attribute codes, matching [`conui_cell::Attrs`] bit order.
pub mod sgr {
    /// `SGR 1`: bold.
    pub const BOLD: u16 = 1;
    /// `SGR 2`: faint.
    pub const DIM: u16 = 2;
    /// `SGR 3`: italic.
    pub const ITALIC: u16 = 3;
    /// `SGR 4`: underline.
    pub const UNDERLINE: u16 = 4;
    /// `SGR 5`: blink.
    pub const BLINK: u16 = 5;
    /// `SGR 7`: reverse video. Note the gap: 6 is rapid blink, which nothing honours.
    pub const REVERSE: u16 = 7;
    /// `SGR 8`: concealed.
    pub const HIDDEN: u16 = 8;
    /// `SGR 9`: struck through.
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
