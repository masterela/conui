//! Turning changed cells into the shortest correct byte stream.
//!
//! [`Painter`] is where a frame becomes bytes. It is generic over any [`Write`], which is the
//! point: the entire output layer is tested against a `Vec<u8>` with exact byte assertions,
//! no TTY involved. Three things keep the stream small:
//!
//! - **Style deltas.** The painter models the terminal's current SGR state and emits only
//!   what differs, so a run of same-colored cells costs one escape for the whole run.
//! - **Cursor deltas.** Consecutive cells need no positioning at all; a short forward jump
//!   uses `CUF` instead of a full absolute move.
//! - **One write per frame.** Everything is staged in a reusable buffer and handed to the
//!   OS in a single call, wrapped in synchronized-output markers so it presents atomically.

use std::io::{self, Write};

use conui_cell::{Attrs, Color, ColorDepth, Patch, Pos, ResolvedStyle};

use crate::ansi::{self, attr_code};
use crate::caps::Capabilities;

/// Longest forward jump worth expressing as `CUF` rather than an absolute move.
///
/// `ESC [ n C` is 4-5 bytes; `ESC [ row ; col H` runs to 9. Past a handful of columns the
/// difference stops mattering, and absolute moves are self-correcting if our model of the
/// cursor is ever wrong.
const MAX_RELATIVE_JUMP: u16 = 4;

/// Writes frames to a terminal, tracking its state to avoid redundant escapes.
pub struct Painter<W: Write> {
    out: W,
    caps: Capabilities,
    /// Staging buffer for the frame in progress, reused across frames.
    staged: Vec<u8>,
    /// The SGR state we believe the terminal is in. `None` means unknown, which forces a
    /// reset before the next styled write.
    style: Option<ResolvedStyle>,
    /// Where we believe the cursor is. `None` forces an absolute move.
    cursor: Option<Pos>,
    /// The background an erase should leave behind. See [`Painter::set_ground`].
    ground: Color,
    /// Whether we are between `begin_frame` and `end_frame`.
    in_frame: bool,
    /// Whether the alternate screen and input protocols are currently active.
    screen_entered: bool,
    /// Whether we have told the terminal its own default background, and so owe it a reset.
    owns_background: bool,
}

impl<W: Write> Painter<W> {
    /// A painter writing to `out`, emitting only what `caps` says the terminal understands.
    pub fn new(out: W, caps: Capabilities) -> Self {
        Self {
            out,
            caps,
            staged: Vec::with_capacity(8 * 1024),
            style: None,
            cursor: None,
            ground: Color::Reset,
            in_frame: false,
            screen_entered: false,
            owns_background: false,
        }
    }

    /// What this painter believes the terminal can do.
    pub fn capabilities(&self) -> Capabilities {
        self.caps
    }

    /// Replace the capabilities, forgetting any style the terminal was assumed to be in.
    pub fn set_capabilities(&mut self, caps: Capabilities) {
        self.caps = caps;
        self.invalidate();
    }

    /// The sink being written to. For a test that wants to read back the bytes emitted.
    pub fn get_ref(&self) -> &W {
        &self.out
    }

    /// Recover the underlying writer, restoring the screen first if it was taken over.
    pub fn into_inner(mut self) -> W {
        if self.screen_entered {
            let _ = self.leave_screen();
        }
        // `Painter` has a `Drop` impl as a safety net, which makes moving a field out of it
        // impossible by ordinary means. Suppress the drop and take the writer by hand.
        let mut this = std::mem::ManuallyDrop::new(self);
        // SAFETY: `this` is never dropped, so `out` is read exactly once and never again.
        // `staged` is the only other field that owns anything, and it is dropped explicitly;
        // every remaining field is `Copy`.
        unsafe {
            std::ptr::drop_in_place(&raw mut this.staged);
            std::ptr::read(&raw const this.out)
        }
    }

    /// The colour an erase leaves behind, which should be the theme's background.
    ///
    /// `ED` paints with whatever background is current, so a clear emitted after a plain reset
    /// fills the screen with the *terminal's* default colour. The caller meanwhile takes the
    /// clear as licence to believe the screen now holds its own blank cell, and the differ then
    /// skips every cell that stays blank — so on a theme whose background is nothing like the
    /// terminal's, the gaps between the writing keep the terminal's colour for the whole run.
    /// Telling the painter the ground closes that gap: the erase paints what the caller claims.
    ///
    /// It also tells the terminal, which owns the padding around the grid — see
    /// [`ansi::set_background`]. Staged rather than flushed, because a colour change is worth exactly
    /// one frame's latency and this is called from a theme switch, which is redrawing anyway.
    pub fn set_ground(&mut self, ground: Color) {
        if self.ground == ground {
            return;
        }
        self.ground = ground;
        if self.screen_entered {
            self.own_background();
        }
    }

    /// Claim the terminal's default background, if the ground is a colour a terminal can be told.
    ///
    /// Only true colour. An indexed ground could be sent as `rgb:` too, but only by this crate
    /// deciding what index 4 looks like in the user's own palette, and getting that wrong paints the
    /// padding a colour that appears nowhere else on screen — worse than the frame it set out to fix.
    fn own_background(&mut self) {
        if self.caps.color_depth != ColorDepth::TrueColor {
            return;
        }
        if let Color::Rgb(red, green, blue) = self.ground {
            let sequence = ansi::set_background(red, green, blue);
            self.push(&sequence);
            self.owns_background = true;
        }
    }

    /// Forget everything we believe about the terminal.
    ///
    /// Call after anything that can change terminal state behind our back: a resize, a
    /// suspend/resume, or another process writing to the same tty. The next frame then
    /// re-establishes style and cursor from scratch instead of trusting a stale model.
    pub fn invalidate(&mut self) {
        self.style = None;
        self.cursor = None;
    }

    // ---- Screen lifecycle ---------------------------------------------------------------

    /// Take over the screen: alternate buffer, no cursor, no autowrap, input protocols on.
    pub fn enter_screen(&mut self) -> io::Result<()> {
        if self.screen_entered {
            return Ok(());
        }
        self.push(ansi::ENTER_ALT_SCREEN);
        self.push(ansi::HIDE_CURSOR);
        self.push(ansi::DISABLE_AUTOWRAP);
        if self.caps.bracketed_paste {
            self.push(ansi::ENABLE_BRACKETED_PASTE);
        }
        if self.caps.focus_events {
            self.push(ansi::ENABLE_FOCUS_EVENTS);
        }
        // Before the clear, so the terminal has the colour by the time it paints anything — and on
        // the alternate screen, so the shell underneath is never repainted on the way past.
        self.own_background();
        self.clear_screen();
        self.screen_entered = true;
        self.flush()
    }

    /// Hand the screen back exactly as we found it.
    ///
    /// Emitted in reverse order of `enter_screen`, and deliberately tolerant: this runs from
    /// cleanup paths, including a panic, where giving up halfway would leave the user with an
    /// invisible cursor and a terminal in raw mode.
    pub fn leave_screen(&mut self) -> io::Result<()> {
        if !self.screen_entered {
            return Ok(());
        }
        if self.in_frame {
            self.push(ansi::END_SYNC);
            self.in_frame = false;
        }
        self.push(ansi::RESET_STYLE);
        if self.caps.mouse {
            self.push(ansi::DISABLE_MOUSE);
        }
        if self.caps.focus_events {
            self.push(ansi::DISABLE_FOCUS_EVENTS);
        }
        if self.caps.bracketed_paste {
            self.push(ansi::DISABLE_BRACKETED_PASTE);
        }
        self.push(ansi::ENABLE_AUTOWRAP);
        if self.owns_background {
            self.push(ansi::RESET_BACKGROUND);
            self.owns_background = false;
        }
        self.push(ansi::LEAVE_ALT_SCREEN);
        self.push(ansi::SHOW_CURSOR);
        self.screen_entered = false;
        self.invalidate();
        self.flush()
    }

    /// Turn mouse reporting on or off, and record it in the capabilities.
    pub fn set_mouse_capture(&mut self, enabled: bool) -> io::Result<()> {
        self.push(if enabled { ansi::ENABLE_MOUSE } else { ansi::DISABLE_MOUSE });
        self.caps.mouse = enabled;
        self.flush()
    }

    // ---- Frames -------------------------------------------------------------------------

    /// Open an atomic frame.
    pub fn begin_frame(&mut self) {
        if self.caps.synchronized_output && !self.in_frame {
            self.push(ansi::BEGIN_SYNC);
        }
        self.in_frame = true;
    }

    /// Close the frame and send it, optionally leaving a visible cursor behind.
    ///
    /// `cursor` is where a text caret should sit: passing `Some` shows the real terminal
    /// cursor there, which is what makes a text input feel native and keeps screen readers
    /// and IME candidate windows anchored correctly.
    pub fn end_frame(&mut self, cursor: Option<Pos>) -> io::Result<()> {
        match cursor {
            Some(pos) => {
                self.move_to(pos.x, pos.y);
                self.push(ansi::SHOW_CURSOR);
            }
            None => self.push(ansi::HIDE_CURSOR),
        }
        if self.caps.synchronized_output && self.in_frame {
            self.push(ansi::END_SYNC);
        }
        self.in_frame = false;
        self.flush()
    }

    /// Blank the whole screen to the ground colour and park the cursor at the origin.
    ///
    /// Needed when the previous contents cannot be trusted — on taking the screen over, and after
    /// a resize, where every coordinate has moved and stale cells would otherwise survive outside
    /// the new bounds. The erase is to [`Painter::set_ground`], not to the terminal's default, so
    /// that a caller may treat the cleared screen as holding its own blank cell.
    pub fn clear_screen(&mut self) {
        self.push(ansi::RESET_STYLE);
        self.style = Some(ResolvedStyle::default());
        self.apply_style(ResolvedStyle { bg: self.ground, ..ResolvedStyle::default() });
        self.push(ansi::CLEAR_SCREEN);
        self.push(ansi::CURSOR_HOME);
        self.cursor = Some(Pos::new(0, 0));
    }

    /// Emit the given changed cells. `screen_width` is the terminal width, needed to detect
    /// writes that land on the final column.
    pub fn draw(&mut self, patches: &[Patch], screen_width: u16) -> io::Result<()> {
        for patch in patches {
            // A continuation carries no glyph of its own; the wide cell to its left already
            // covered the column. Emitting it would overwrite that glyph with a blank.
            if patch.cell.is_continuation() {
                continue;
            }
            self.move_to(patch.x, patch.y);
            self.apply_style(patch.cell.style.resolve(ResolvedStyle::default()));
            self.push(patch.cell.symbol.as_str());

            let advance = patch.cell.width().max(1);
            let next = patch.x.saturating_add(advance);
            // At the right edge the cursor's resting place is implementation-defined, so
            // stop believing we know it and let the next write position itself absolutely.
            self.cursor = if next >= screen_width { None } else { Some(Pos::new(next, patch.y)) };
        }
        Ok(())
    }

    /// Send everything staged so far as a single write.
    pub fn flush(&mut self) -> io::Result<()> {
        if self.staged.is_empty() {
            return self.out.flush();
        }
        self.out.write_all(&self.staged)?;
        self.staged.clear();
        self.out.flush()
    }

    // ---- Escape emission ----------------------------------------------------------------

    fn push(&mut self, text: &str) {
        self.staged.extend_from_slice(text.as_bytes());
    }

    /// Append a decimal integer without going through `format!`.
    ///
    /// Hand-rolled because this runs a few times per changed cell, and the formatting
    /// machinery's cost shows up in a profile of a full-screen animated repaint.
    fn push_number(&mut self, mut value: u16) {
        if value == 0 {
            self.staged.push(b'0');
            return;
        }
        let mut digits = [0u8; 5];
        let mut len = 0;
        while value > 0 {
            digits[len] = b'0' + (value % 10) as u8;
            value /= 10;
            len += 1;
        }
        for index in (0..len).rev() {
            self.staged.push(digits[index]);
        }
    }

    fn move_to(&mut self, x: u16, y: u16) {
        match self.cursor {
            Some(pos) if pos.x == x && pos.y == y => return,
            // Same row, a short hop forward: a relative jump is shorter than an absolute one.
            Some(pos) if pos.y == y && x > pos.x && x - pos.x <= MAX_RELATIVE_JUMP => {
                let delta = x - pos.x;
                self.push(ansi::CSI);
                self.push_number(delta);
                self.push("C");
            }
            _ => {
                // CUP is 1-based in both axes.
                self.push(ansi::CSI);
                self.push_number(y.saturating_add(1));
                self.push(";");
                self.push_number(x.saturating_add(1));
                self.push("H");
            }
        }
        self.cursor = Some(Pos::new(x, y));
    }

    /// Bring the terminal's rendition to `target`, emitting only the difference.
    fn apply_style(&mut self, target: ResolvedStyle) {
        let depth = self.caps.color_depth;
        let target = ResolvedStyle {
            fg: target.fg.degrade(depth),
            bg: target.bg.degrade(depth),
            attrs: target.attrs,
        };
        if self.style == Some(target) {
            return;
        }

        // Turning an attribute *off* individually is unreliable: SGR 22 clears bold and dim
        // together, and 21 means double-underline on some terminals and bold-off on others.
        // A full reset followed by a rebuild is the only portable route, so take it whenever
        // the new style drops an attribute the old one had.
        let current = match self.style {
            Some(current) if current.attrs.difference(target.attrs).is_empty() => current,
            _ => {
                self.push(ansi::RESET_STYLE);
                ResolvedStyle::default()
            }
        };

        let mut params: Vec<u16> = Vec::with_capacity(12);
        if current.fg != target.fg {
            push_color_params(&mut params, target.fg, Ground::Foreground);
        }
        if current.bg != target.bg {
            push_color_params(&mut params, target.bg, Ground::Background);
        }
        for attr in target.attrs.difference(current.attrs).iter() {
            if let Some(code) = attr_code(attr) {
                params.push(code);
            }
        }

        if !params.is_empty() {
            self.push(ansi::CSI);
            for (index, param) in params.iter().enumerate() {
                if index > 0 {
                    self.push(";");
                }
                self.push_number(*param);
            }
            self.push("m");
        }
        self.style = Some(target);
    }
}

impl<W: Write> Drop for Painter<W> {
    /// Restore the terminal even if the app forgot to, or unwound past its cleanup.
    ///
    /// A TUI that exits without undoing raw mode and the alternate screen leaves the user at
    /// an unusable prompt, so this is worth doing on a best-effort basis and ignoring errors:
    /// there is nothing useful to do about a failure while unwinding.
    fn drop(&mut self) {
        if self.screen_entered {
            let _ = self.leave_screen();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Ground {
    Foreground,
    Background,
}

fn push_color_params(params: &mut Vec<u16>, color: Color, ground: Ground) {
    let foreground = ground == Ground::Foreground;
    match color {
        Color::Reset => params.push(if foreground { 39 } else { 49 }),
        Color::Ansi(index) => {
            // Mask rather than trust: an out-of-range index would otherwise emit a parameter
            // that shifts the meaning of everything after it in the same sequence.
            let index = index & 0x0f;
            let base = match (foreground, index < 8) {
                (true, true) => 30,
                (true, false) => 90,
                (false, true) => 40,
                (false, false) => 100,
            };
            params.push(base + (index % 8) as u16);
        }
        Color::Indexed(index) => {
            params.push(if foreground { 38 } else { 48 });
            params.push(5);
            params.push(index as u16);
        }
        Color::Rgb(r, g, b) => {
            params.push(if foreground { 38 } else { 48 });
            params.push(2);
            params.push(r as u16);
            params.push(g as u16);
            params.push(b as u16);
        }
    }
}

/// Attributes that a [`conui_cell::ColorDepth::NoColor`] terminal can still express, used by
/// callers that want to keep a visual hierarchy without color.
pub const MONOCHROME_EMPHASIS: Attrs = Attrs::BOLD;

#[cfg(test)]
mod tests {
    use super::*;
    use conui_cell::{Buffer, Cell, ColorDepth, Style};

    fn painter(depth: ColorDepth) -> Painter<Vec<u8>> {
        Painter::new(Vec::new(), Capabilities::plain(depth))
    }

    /// Draw `next` over `previous` and return the exact bytes sent.
    fn bytes_for(previous: &Buffer, next: &Buffer, depth: ColorDepth) -> String {
        let mut painter = painter(depth);
        // Start from a known state so the assertions describe the diff, not the handshake.
        painter.style = Some(ResolvedStyle::default());
        painter.cursor = Some(Pos::new(0, 0));
        painter.draw(&next.diff(previous), next.width()).unwrap();
        painter.flush().unwrap();
        String::from_utf8(painter.into_inner()).unwrap()
    }

    #[test]
    fn replacing_the_capabilities_changes_what_the_next_write_emits() {
        // The escape hatch for a terminal whose abilities are learned after the painter exists —
        // a response to a query, or a `NO_COLOR` read late. Asserting on the bytes rather than on
        // the field is the point: a depth that is stored but not consulted would pass a field
        // check and still paint the wrong screen.
        let mut painter = painter(ColorDepth::NoColor);
        let mut buffer = Buffer::new(1, 1);
        buffer.set_symbol(0, 0, "a", Style::new().fg(Color::hex("#62f5b5")));

        painter.draw(&buffer.full_repaint(), 1).unwrap();
        painter.flush().unwrap();
        // A position, a reset, the glyph — and not the colour that was asked for.
        let plain = String::from_utf8_lossy(painter.get_ref()).to_string();
        assert!(!plain.contains("38;2;"), "NoColor must not emit a colour: {plain:?}");

        painter.set_capabilities(Capabilities::plain(ColorDepth::TrueColor));
        assert_eq!(painter.capabilities().color_depth, ColorDepth::TrueColor);
        painter.draw(&buffer.full_repaint(), 1).unwrap();
        painter.flush().unwrap();
        let output = String::from_utf8(painter.into_inner()).unwrap();
        assert!(
            output.contains("38;2;98;245;181"),
            "the new depth must reach the wire: {output:?}"
        );
    }

    #[test]
    fn replacing_the_capabilities_forgets_the_style_the_terminal_was_assumed_to_be_in() {
        // The reason `set_capabilities` cannot be a plain field assignment: the cached style was
        // recorded in the old depth's terms, so anything still trusting it would skip a sequence
        // the terminal never received.
        let mut painter = painter(ColorDepth::NoColor);
        painter.style = Some(ResolvedStyle::default());
        painter.cursor = Some(Pos::new(4, 2));
        painter.set_capabilities(Capabilities::plain(ColorDepth::TrueColor));
        assert!(painter.style.is_none());
        assert!(painter.cursor.is_none());
    }

    #[test]
    fn the_sink_can_be_read_without_being_taken() {
        // `into_inner` consumes the painter, which is no use halfway through a test that wants to
        // keep drawing. This is the borrow that lets one.
        let mut painter = painter(ColorDepth::NoColor);
        let mut buffer = Buffer::new(2, 1);
        buffer.set_str(0, 0, "hi", Style::EMPTY, 2);
        painter.draw(&buffer.full_repaint(), 2).unwrap();
        painter.flush().unwrap();

        assert!(String::from_utf8_lossy(painter.get_ref()).contains("hi"));
        // Still usable afterwards, which is the whole difference from `into_inner`.
        painter.flush().unwrap();
        assert!(String::from_utf8_lossy(painter.get_ref()).contains("hi"));
    }

    #[test]
    fn a_single_changed_cell_costs_one_move_and_one_glyph() {
        let previous = Buffer::new(10, 2);
        let mut next = previous.clone();
        next.set_symbol(3, 1, "x", Style::EMPTY);
        // Row 2, column 4 in 1-based CUP coordinates.
        assert_eq!(bytes_for(&previous, &next, ColorDepth::TrueColor), "\x1b[2;4Hx");
    }

    #[test]
    fn consecutive_cells_need_no_repositioning() {
        let previous = Buffer::new(10, 1);
        let mut next = previous.clone();
        next.set_str(0, 0, "abc", Style::EMPTY, 10);
        assert_eq!(bytes_for(&previous, &next, ColorDepth::TrueColor), "abc");
    }

    #[test]
    fn a_short_gap_uses_a_relative_jump() {
        let previous = Buffer::new(20, 1);
        let mut next = previous.clone();
        next.set_symbol(0, 0, "a", Style::EMPTY);
        next.set_symbol(3, 0, "b", Style::EMPTY);
        // Two columns skipped: forward by 2 rather than an absolute move.
        assert_eq!(bytes_for(&previous, &next, ColorDepth::TrueColor), "a\x1b[2Cb");
    }

    #[test]
    fn a_long_gap_uses_an_absolute_move() {
        let previous = Buffer::new(40, 1);
        let mut next = previous.clone();
        next.set_symbol(0, 0, "a", Style::EMPTY);
        next.set_symbol(30, 0, "b", Style::EMPTY);
        assert_eq!(bytes_for(&previous, &next, ColorDepth::TrueColor), "a\x1b[1;31Hb");
    }

    #[test]
    fn one_escape_covers_a_whole_run_of_same_colored_cells() {
        let previous = Buffer::new(10, 1);
        let mut next = previous.clone();
        let mint = Style::new().fg(Color::hex("#62f5b5"));
        next.set_str(0, 0, "abcd", mint, 10);
        assert_eq!(
            bytes_for(&previous, &next, ColorDepth::TrueColor),
            "\x1b[38;2;98;245;181mabcd",
            "color should be set once, not per cell"
        );
    }

    #[test]
    fn truecolor_foreground_and_background_ride_in_one_sequence() {
        let previous = Buffer::new(4, 1);
        let mut next = previous.clone();
        let style = Style::new().fg(Color::hex("#ffffff")).bg(Color::hex("#000000"));
        next.set_symbol(0, 0, "z", style);
        assert_eq!(
            bytes_for(&previous, &next, ColorDepth::TrueColor),
            "\x1b[38;2;255;255;255;48;2;0;0;0mz"
        );
    }

    #[test]
    fn only_the_changed_half_of_a_style_is_re_emitted() {
        let previous = Buffer::new(8, 1);
        let mut next = previous.clone();
        let base = Style::new().fg(Color::hex("#ffffff")).bg(Color::hex("#000000"));
        next.set_symbol(0, 0, "a", base);
        next.set_symbol(1, 0, "b", base.fg(Color::hex("#ff0000")));
        let output = bytes_for(&previous, &next, ColorDepth::TrueColor);
        assert_eq!(
            output, "\x1b[38;2;255;255;255;48;2;0;0;0ma\x1b[38;2;255;0;0mb",
            "the unchanged background must not be repeated"
        );
    }

    #[test]
    fn adding_an_attribute_does_not_reset() {
        let previous = Buffer::new(8, 1);
        let mut next = previous.clone();
        next.set_symbol(0, 0, "a", Style::new().bold());
        next.set_symbol(1, 0, "b", Style::new().bold().italic());
        assert_eq!(bytes_for(&previous, &next, ColorDepth::TrueColor), "\x1b[1ma\x1b[3mb");
    }

    #[test]
    fn dropping_an_attribute_forces_a_reset_and_rebuild() {
        let previous = Buffer::new(8, 1);
        let mut next = previous.clone();
        let mint = Color::hex("#62f5b5");
        next.set_symbol(0, 0, "a", Style::new().fg(mint).bold());
        next.set_symbol(1, 0, "b", Style::new().fg(mint));
        let output = bytes_for(&previous, &next, ColorDepth::TrueColor);
        // Bold cannot be turned off portably, so the painter resets and restates the color.
        assert_eq!(output, "\x1b[38;2;98;245;181;1ma\x1b[0m\x1b[38;2;98;245;181mb");
    }

    #[test]
    fn indexed_depth_degrades_rgb_at_write_time() {
        let previous = Buffer::new(4, 1);
        let mut next = previous.clone();
        next.set_symbol(0, 0, "q", Style::new().fg(Color::hex("#ff0000")));
        assert_eq!(bytes_for(&previous, &next, ColorDepth::Indexed256), "\x1b[38;5;196mq");
    }

    #[test]
    fn ansi16_depth_degrades_rgb_to_a_basic_slot() {
        let previous = Buffer::new(4, 1);
        let mut next = previous.clone();
        next.set_symbol(0, 0, "q", Style::new().fg(Color::hex("#62f5b5")));
        // Bright green is slot 10, emitted as 90 + (10 - 8).
        assert_eq!(bytes_for(&previous, &next, ColorDepth::Ansi16), "\x1b[92mq");
    }

    #[test]
    fn nocolor_depth_emits_attributes_but_no_color() {
        let previous = Buffer::new(4, 1);
        let mut next = previous.clone();
        next.set_symbol(0, 0, "q", Style::new().fg(Color::hex("#62f5b5")).bold());
        // The design's emphasis survives; the color does not.
        assert_eq!(bytes_for(&previous, &next, ColorDepth::NoColor), "\x1b[1mq");
    }

    #[test]
    fn an_unchanged_frame_emits_nothing_at_all() {
        let buffer = Buffer::new(40, 10);
        assert_eq!(bytes_for(&buffer, &buffer.clone(), ColorDepth::TrueColor), "");
    }

    #[test]
    fn a_wide_glyph_advances_the_cursor_by_two() {
        let previous = Buffer::new(10, 1);
        let mut next = previous.clone();
        next.set_symbol(0, 0, "界", Style::EMPTY);
        next.set_symbol(2, 0, "x", Style::EMPTY);
        // Nothing between them: the wide glyph already left the cursor at column 2.
        assert_eq!(bytes_for(&previous, &next, ColorDepth::TrueColor), "界x");
    }

    #[test]
    fn a_write_at_the_last_column_stops_tracking_the_cursor() {
        // Where the cursor rests after a glyph in the final column is implementation-defined:
        // with autowrap off it stays put, with autowrap on it enters a deferred-wrap state.
        // Rather than guess, the painter admits it does not know, so the next write positions
        // itself absolutely. This is internal state with no byte-level tell, so assert on it.
        let mut painter = painter(ColorDepth::TrueColor);
        painter.style = Some(ResolvedStyle::default());
        painter.cursor = Some(Pos::new(0, 0));

        let mut buffer = Buffer::new(4, 1);
        buffer.set_symbol(2, 0, "a", Style::EMPTY);
        painter.draw(&buffer.diff(&Buffer::new(4, 1)), 4).unwrap();
        assert_eq!(painter.cursor, Some(Pos::new(3, 0)), "mid-row stays tracked");

        let mut buffer = Buffer::new(4, 1);
        buffer.set_symbol(3, 0, "a", Style::EMPTY);
        painter.draw(&buffer.diff(&Buffer::new(4, 1)), 4).unwrap();
        assert_eq!(painter.cursor, None, "the final column must invalidate tracking");
    }

    #[test]
    fn a_wide_glyph_ending_at_the_edge_also_stops_tracking() {
        let mut painter = painter(ColorDepth::TrueColor);
        painter.cursor = Some(Pos::new(0, 0));
        let mut buffer = Buffer::new(4, 1);
        buffer.set_symbol(2, 0, "界", Style::EMPTY); // occupies columns 2 and 3
        painter.draw(&buffer.diff(&Buffer::new(4, 1)), 4).unwrap();
        assert_eq!(painter.cursor, None);
    }

    #[test]
    fn a_short_hop_from_the_origin_is_relative() {
        let previous = Buffer::new(8, 1);
        let mut next = previous.clone();
        next.set_symbol(3, 0, "a", Style::EMPTY);
        // Three columns from a known origin: forward is shorter than an absolute move.
        assert_eq!(bytes_for(&previous, &next, ColorDepth::TrueColor), "\x1b[3Ca");
    }

    #[test]
    fn invalidate_forces_a_style_reset_on_the_next_write() {
        let mut painter = painter(ColorDepth::TrueColor);
        let mut buffer = Buffer::new(4, 1);
        buffer.set_symbol(0, 0, "a", Style::EMPTY);
        painter.invalidate();
        painter.draw(&buffer.full_repaint(), 4).unwrap();
        painter.flush().unwrap();
        let output = String::from_utf8(painter.into_inner()).unwrap();
        assert!(output.starts_with("\x1b[1;1H"), "position must be re-established: {output:?}");
        assert!(output.contains("\x1b[0m"), "style must be reset: {output:?}");
    }

    #[test]
    fn a_frame_is_wrapped_in_synchronized_output_markers() {
        let mut painter = Painter::new(Vec::new(), Capabilities::default());
        painter.begin_frame();
        painter.end_frame(None).unwrap();
        let output = String::from_utf8(painter.into_inner()).unwrap();
        assert!(output.starts_with(ansi::BEGIN_SYNC));
        assert!(output.ends_with(ansi::END_SYNC));
    }

    #[test]
    fn end_frame_can_leave_a_visible_caret() {
        let mut painter = painter(ColorDepth::TrueColor);
        painter.cursor = Some(Pos::new(0, 0));
        painter.begin_frame();
        painter.end_frame(Some(Pos::new(6, 2))).unwrap();
        let output = String::from_utf8(painter.into_inner()).unwrap();
        assert_eq!(output, "\x1b[3;7H\x1b[?25h");
    }

    #[test]
    fn leaving_the_screen_undoes_entering_it() {
        let mut painter = Painter::new(Vec::new(), Capabilities::default());
        painter.enter_screen().unwrap();
        painter.out.clear();
        painter.leave_screen().unwrap();
        let output = String::from_utf8(painter.into_inner()).unwrap();
        for expected in [
            ansi::RESET_STYLE,
            ansi::DISABLE_BRACKETED_PASTE,
            ansi::ENABLE_AUTOWRAP,
            ansi::LEAVE_ALT_SCREEN,
            ansi::SHOW_CURSOR,
        ] {
            assert!(output.contains(expected), "restore is missing {expected:?}");
        }
    }

    #[test]
    fn entering_the_screen_disables_autowrap() {
        // Without this, a glyph in the last column scrolls the whole frame.
        let mut painter = Painter::new(Vec::new(), Capabilities::default());
        painter.enter_screen().unwrap();
        let output = String::from_utf8(painter.into_inner()).unwrap();
        assert!(output.contains(ansi::ENTER_ALT_SCREEN));
        assert!(output.contains(ansi::DISABLE_AUTOWRAP));
        assert!(output.contains(ansi::HIDE_CURSOR));
    }

    /// The bug this guards against is invisible on a dark theme in a dark terminal and glaring on
    /// a light one: an erase paints with the current background, and the caller is entitled to
    /// treat the cleared screen as holding its blank cell. If the two disagree, every cell that
    /// never gets written — which is most of a sparse screen — keeps the terminal's own colour.
    #[test]
    fn a_clear_paints_the_ground_rather_than_the_terminals_own_background() {
        let mut painter = Painter::new(Vec::new(), Capabilities::default());
        painter.set_ground(Color::Rgb(238, 241, 236));
        painter.enter_screen().unwrap();
        let output = String::from_utf8(painter.into_inner()).unwrap();
        let set = "\x1b[48;2;238;241;236m";
        let at = output.find(set).expect("the ground colour is never set");
        let cleared = output.find(ansi::CLEAR_SCREEN).expect("the screen is never cleared");
        assert!(
            at < cleared,
            "the ground has to be current before the erase, or it paints nothing"
        );

        // And a painter told nothing still says nothing, so a theme that inherits stays polite.
        let mut plain = Painter::new(Vec::new(), Capabilities::default());
        plain.enter_screen().unwrap();
        let output = String::from_utf8(plain.into_inner()).unwrap();
        assert!(!output.contains("\x1b[48"), "an inherited ground must not be painted: {output:?}");
    }

    /// The padding is the part of the screen no cell can reach, and on a light theme in a dark
    /// terminal it is a dark frame around the whole app. Only the terminal can paint it, and only if
    /// it is told — and it has to be untold on the way out, or the user's shell keeps our colour.
    #[test]
    fn the_terminal_is_told_the_ground_and_told_to_forget_it() {
        let mut painter = Painter::new(Vec::new(), Capabilities::default());
        painter.set_ground(Color::Rgb(238, 241, 236));
        painter.enter_screen().unwrap();
        let entered = String::from_utf8_lossy(painter.get_ref()).to_string();
        let set = "\x1b]11;rgb:ee/f1/ec\x1b\\";
        assert!(entered.contains(set), "the terminal is never told the ground: {entered:?}");
        assert!(
            entered.find(ansi::ENTER_ALT_SCREEN) < entered.find(set),
            "the shell's own screen must not be repainted on the way past"
        );

        // A theme switch while running reaches the padding too, or half the screen changes colour.
        painter.set_ground(Color::Rgb(9, 15, 19));
        assert!(
            String::from_utf8_lossy(&painter.staged).contains("\x1b]11;rgb:09/0f/13\x1b\\"),
            "a new ground has to reach the terminal as well as the cells"
        );

        painter.leave_screen().unwrap();
        let output = String::from_utf8(painter.into_inner()).unwrap();
        assert!(output.contains(ansi::RESET_BACKGROUND), "the background is never given back");

        // And a painter with nothing to say says nothing: an inherited ground leaves the terminal's
        // own background alone, so there is nothing to reset either.
        let mut plain = Painter::new(Vec::new(), Capabilities::default());
        plain.enter_screen().unwrap();
        plain.leave_screen().unwrap();
        let output = String::from_utf8(plain.into_inner()).unwrap();
        assert!(!output.contains("\x1b]11"), "an inherited ground must not be claimed: {output:?}");
        assert!(!output.contains(ansi::RESET_BACKGROUND), "nor given back: {output:?}");

        // Nor does a terminal that cannot be trusted with a colour it was never given in RGB.
        let mut shallow = Painter::new(Vec::new(), Capabilities::plain(ColorDepth::Indexed256));
        shallow.set_ground(Color::Rgb(238, 241, 236));
        shallow.enter_screen().unwrap();
        let output = String::from_utf8(shallow.into_inner()).unwrap();
        assert!(
            !output.contains("\x1b]11"),
            "256 colours is not a background to claim: {output:?}"
        );
    }

    #[test]
    fn push_number_formats_the_whole_u16_range_correctly() {
        for value in [0u16, 1, 9, 10, 99, 100, 255, 1000, 9999, 65535] {
            let mut painter = painter(ColorDepth::TrueColor);
            painter.push_number(value);
            assert_eq!(String::from_utf8(painter.staged.clone()).unwrap(), value.to_string());
        }
    }

    #[test]
    fn a_full_repaint_of_a_uniform_screen_stays_compact() {
        // The whole point of run coalescing: a themed background should not cost an escape
        // sequence per cell.
        let style = Style::new().fg(Color::hex("#e3f3ef")).bg(Color::hex("#090f13"));
        let buffer = Buffer::filled(80, 24, Cell::new(' ', style));
        let mut painter = painter(ColorDepth::TrueColor);
        painter.style = Some(ResolvedStyle::default());
        painter.cursor = Some(Pos::new(0, 0));
        painter.draw(&buffer.full_repaint(), 80).unwrap();
        painter.flush().unwrap();
        let bytes = painter.into_inner().len();
        let cells = 80 * 24;
        assert!(bytes < cells + 24 * 12 + 64, "full repaint took {bytes} bytes for {cells} cells");
    }
}
