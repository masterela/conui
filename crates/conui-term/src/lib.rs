//! The terminal layer: raw mode, capability detection, and frame presentation.
//!
//! [`Terminal`] is the only thing here that touches real file descriptors. It owns the
//! *front buffer* — conui's model of what is currently on screen — so that [`Terminal::present`]
//! can diff a freshly rendered frame against it and send only the difference.
//!
//! ```no_run
//! use conui_cell::{Buffer, Color, Style};
//! use conui_term::Terminal;
//!
//! let mut terminal = Terminal::new()?;
//! terminal.enter()?;
//!
//! let (width, height) = terminal.size();
//! let mut frame = Buffer::new(width, height);
//! frame.set_str(2, 1, "hello", Style::new().fg(Color::hex("#62f5b5")), width);
//! terminal.present(&frame, None)?;
//!
//! terminal.leave()?;
//! # Ok::<(), std::io::Error>(())
//! ```

// This crate is the one an app reaches past conui for when it needs a sequence we do not wrap,
// so every public item here should say what it emits.
#![warn(missing_docs)]

pub mod ansi;
mod caps;
mod platform;
mod writer;

pub use caps::Capabilities;
pub use writer::{MONOCHROME_EMPHASIS, Painter};

use std::io::{self, Stdout, Write};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use conui_cell::{Buffer, Cell, Color, Pos};

/// Terminal settings captured on entry, restored on exit.
pub use platform::SavedMode;

/// The terminal window in pixels, if the tty will say, before any [`Terminal`] exists.
///
/// Free-standing because the answer is usually wanted *early* — a program deciding how big an image
/// to rasterise needs it to size the buffer it will draw into, which happens before there is a
/// screen to draw it on. See [`Terminal::cell_pixels`] for the derived number.
pub fn window_pixels() -> Option<(u16, u16)> {
    platform::window_pixels().ok().flatten()
}

/// One cell in pixels, if the terminal will say, before any [`Terminal`] exists.
///
/// The same answer as [`Terminal::cell_pixels`], which this is the early-bird form of.
pub fn cell_pixels() -> Option<(u16, u16)> {
    divide_into_cells(window_pixels()?, platform::window_size().ok()?)
}

/// The window's pixels over its cells, clamped to a size a font could plausibly be.
///
/// Clamped because a terminal that reports nonsense is worse than one that reports nothing: `None`
/// has an obvious fallback and a 2x400 cell does not. The bounds are generous — 4x8 is smaller than
/// any readable font and 20x44 is larger than any retina cell — so the only values they reject are
/// ones no font has.
fn divide_into_cells(pixels: (u16, u16), cells: (u16, u16)) -> Option<(u16, u16)> {
    let ((pixel_width, pixel_height), (columns, rows)) = (pixels, cells);
    if columns == 0 || rows == 0 {
        return None;
    }
    Some(((pixel_width / columns).clamp(4, 20), (pixel_height / rows).clamp(8, 44)))
}

/// Settings saved by whichever [`Terminal`] most recently entered raw mode.
///
/// Global because a panic hook has no access to the app's state, and restoring the terminal
/// on the way out of a panic matters more than architectural purity: the alternative is a
/// user left at an echo-less prompt with an invisible cursor, unsure what happened.
static PANIC_RESTORE: Mutex<Option<SavedMode>> = Mutex::new(None);
static PANIC_HOOK: OnceLock<()> = OnceLock::new();

/// Undo raw mode and screen state, unconditionally and without allocating.
///
/// Runs from the panic hook, so it cannot assume any invariant holds and cannot report
/// failure. Each step is attempted independently.
fn emergency_restore() {
    let mut stdout = io::stdout();
    let _ = stdout.write_all(
        concat!(
            "\x1b[?2026l", // close any open synchronized frame
            "\x1b[0m",     // reset style
            "\x1b[?1006l\x1b[?1002l\x1b[?1000l",
            "\x1b[?1004l",
            "\x1b[?2004l",
            "\x1b[?7h",       // autowrap back on
            "\x1b]111\x1b\\", // the terminal's own background back, in case we claimed it
            "\x1b[?1049l",    // leave the alternate screen
            "\x1b[?25h",      // show the cursor
        )
        .as_bytes(),
    );
    let _ = stdout.flush();
    if let Ok(mut saved) = PANIC_RESTORE.lock() {
        if let Some(mode) = saved.take() {
            let _ = platform::restore_mode(&mode);
        }
    }
}

/// What a caller gets for taking over a terminal that is not there.
///
/// Letting the platform speak for itself here is the wrong call, and it took piping an example to
/// notice: asking `/dev/null` about its terminal settings fails with `ENODEV`, which reaches the
/// user as `Error: Os { code: 19, kind: Uncategorized, message: "Operation not supported by
/// device" }` and sends them looking for a bug in their own code. There is no bug. They redirected
/// stdin — a pipe, a CI log, an editor's output pane, `< /dev/null` — and a full-screen program
/// cannot run without a keyboard attached to a screen.
///
/// So the message names the requirement rather than the failed syscall, and the kind is
/// `Unsupported` rather than `Other`, which is the difference between a caller being able to match
/// on this and having to match on a string.
fn not_a_terminal() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "conui needs an interactive terminal, and stdin is not one (is it redirected?)",
    )
}

/// Chain a terminal restore ahead of the existing panic hook, once per process.
fn install_panic_hook() {
    PANIC_HOOK.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Restore first: the default hook prints a backtrace, and it should land on the
            // real screen where the user can read and scroll it, not in the alternate buffer
            // that is about to be discarded.
            emergency_restore();
            previous(info);
        }));
    });
}

/// A terminal conui can draw to.
pub struct Terminal {
    painter: Painter<Stdout>,
    saved_mode: Option<SavedMode>,
    /// What we believe is on screen, for diffing the next frame against.
    front: Buffer,
    /// The cell a cleared or resized screen is filled with, so a themed background survives
    /// a resize without every widget having to repaint it.
    blank: Cell,
    entered: bool,
}

impl Terminal {
    /// Open the terminal with capabilities detected from the environment.
    pub fn new() -> io::Result<Self> {
        let caps = Capabilities::detect(platform::is_output_tty());
        Self::with_capabilities(caps)
    }

    /// Open the terminal with explicit capabilities, bypassing detection.
    pub fn with_capabilities(caps: Capabilities) -> io::Result<Self> {
        let (width, height) = platform::window_size().unwrap_or((80, 24));
        Ok(Self {
            painter: Painter::new(io::stdout(), caps),
            saved_mode: None,
            front: Buffer::new(width, height),
            blank: Cell::BLANK,
            entered: false,
        })
    }

    /// What this terminal was detected to be capable of.
    pub fn capabilities(&self) -> Capabilities {
        self.painter.capabilities()
    }

    /// The cell used to fill cleared regions. Set this to the theme's background so that a
    /// resize reveals the app's own color rather than the terminal's default.
    pub fn set_blank_cell(&mut self, cell: Cell) {
        self.blank = cell;
        self.front.reset_to(cell);
        // And the same colour to the painter, so that a clear really does leave this cell behind
        // rather than the terminal's own background under a front buffer that claims otherwise.
        self.painter.set_ground(cell.style.bg.unwrap_or(Color::Reset));
    }

    /// The painter underneath, for emitting a sequence this type does not wrap.
    pub fn painter(&mut self) -> &mut Painter<Stdout> {
        &mut self.painter
    }

    /// Whether stdin is a terminal.
    ///
    /// [`Terminal::enter`] refuses when this is false, so an app does not have to check to stay
    /// correct. Worth checking anyway when there is something better to do than fail: a program
    /// with a plain-text mode can fall back to it, which is friendlier than an error the user can
    /// only fix by rerunning.
    ///
    /// This asks about *input*, because raw mode is a property of the input stream. Output being a
    /// pipe is a separate and much less fatal thing — it only costs colour, and
    /// [`Capabilities::plain`] takes even that back.
    pub fn is_interactive() -> bool {
        platform::is_input_tty()
    }

    /// Take over the terminal: raw mode, alternate screen, and a panic-safe restore.
    ///
    /// Fails with [`io::ErrorKind::Unsupported`] when there is no terminal to take over — see
    /// [`Terminal::is_interactive`], which answers the same question without the error.
    pub fn enter(&mut self) -> io::Result<()> {
        if self.entered {
            return Ok(());
        }
        if !Self::is_interactive() {
            return Err(not_a_terminal());
        }
        install_panic_hook();
        let saved = platform::enter_raw_mode()?;
        if let Ok(mut slot) = PANIC_RESTORE.lock() {
            *slot = Some(saved.clone());
        }
        self.saved_mode = Some(saved);

        if let Err(error) = self.painter.enter_screen() {
            // Entering the screen failed, so raw mode must not be left on.
            let _ = self.restore_mode();
            return Err(error);
        }
        self.entered = true;
        self.sync_size()?;
        // Nothing is on screen yet beyond the clear, so the front buffer is all blanks.
        self.front.reset_to(self.blank);
        Ok(())
    }

    /// Give the terminal back. Idempotent, and safe to call from a cleanup path.
    pub fn leave(&mut self) -> io::Result<()> {
        if !self.entered {
            return Ok(());
        }
        let screen = self.painter.leave_screen();
        let mode = self.restore_mode();
        self.entered = false;
        screen.and(mode)
    }

    fn restore_mode(&mut self) -> io::Result<()> {
        if let Ok(mut slot) = PANIC_RESTORE.lock() {
            *slot = None;
        }
        match self.saved_mode.take() {
            Some(mode) => platform::restore_mode(&mode),
            None => Ok(()),
        }
    }

    /// The current size in cells, as of the last [`Terminal::sync_size`] or [`Terminal::present`].
    pub fn size(&self) -> (u16, u16) {
        (self.front.width(), self.front.height())
    }

    /// The terminal window in pixels, if it will say. `None` on Windows and over most ssh.
    ///
    /// Read fresh rather than cached, because the window can be resized between calls and this is
    /// asked once per frame at most.
    pub fn window_pixels(&self) -> Option<(u16, u16)> {
        platform::window_pixels().ok().flatten()
    }

    /// One cell in pixels, if the terminal will say — the window's pixels over its cells.
    ///
    /// Worth asking rather than assuming 8x17. On a high-density display a cell is nearer 16x34, and
    /// anything committing pixels to a decision — an image handed over at half the real size, a
    /// bitmap that wanted square dots — pays for the wrong guess on every frame.
    ///
    /// Divided against *our* cell count rather than the tty's, so the answer agrees with the frame
    /// the caller is about to draw even in the moment between a resize and the next
    /// [`Terminal::sync_size`].
    pub fn cell_pixels(&self) -> Option<(u16, u16)> {
        divide_into_cells(self.window_pixels()?, self.size())
    }

    /// Re-read the terminal size, resizing the front buffer if it changed.
    ///
    /// Returns the new size when it changed. Polling beats installing a `SIGWINCH` handler: a
    /// library that claims a process-wide signal disposition breaks any application that also
    /// wants it, and an `ioctl` once per frame does not register in a profile.
    pub fn sync_size(&mut self) -> io::Result<Option<(u16, u16)>> {
        let (width, height) = platform::window_size()?;
        if (width, height) == self.size() {
            return Ok(None);
        }
        self.front.resize(width, height);
        self.front.reset_to(self.blank);
        // Every coordinate we knew is now meaningless.
        self.painter.invalidate();
        Ok(Some((width, height)))
    }

    /// Discard our model of the screen so the next [`Terminal::present`] repaints everything.
    ///
    /// Needed after anything else writes to the same terminal, or after a suspend and resume.
    pub fn force_repaint(&mut self) {
        self.front.reset_to(self.blank);
        self.painter.invalidate();
        // A stale front buffer full of blanks would make the differ skip genuinely blank
        // cells, so clear the real screen to match what we now claim is there.
        self.painter.clear_screen();
    }

    /// Send `frame` to the screen, emitting only what differs from the previous frame.
    ///
    /// `cursor` places a visible caret, for a text input; `None` hides it.
    pub fn present(&mut self, frame: &Buffer, cursor: Option<Pos>) -> io::Result<()> {
        let resized = frame.area() != self.front.area();
        self.painter.begin_frame();
        if resized {
            // The old contents cannot be diffed against a grid of a different shape, and
            // anything outside the new bounds would linger.
            self.painter.clear_screen();
            self.front.resize(frame.width(), frame.height());
            self.front.reset_to(self.blank);
        }
        let patches = frame.diff(&self.front);
        self.painter.draw(&patches, frame.width())?;
        self.painter.end_frame(cursor)?;
        self.front.clone_from(frame);
        Ok(())
    }

    /// Wait for input, up to `timeout`. `None` waits indefinitely.
    pub fn wait_readable(&self, timeout: Option<Duration>) -> io::Result<bool> {
        platform::wait_readable(timeout)
    }

    /// Read available input bytes. Call only after [`Terminal::wait_readable`] returns `true`.
    pub fn read_input(&self, buffer: &mut [u8]) -> io::Result<usize> {
        platform::read_input(buffer)
    }

    /// Turn mouse reporting on or off.
    pub fn set_mouse_capture(&mut self, enabled: bool) -> io::Result<()> {
        self.painter.set_mouse_capture(enabled)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        let _ = self.leave();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conui_cell::ColorDepth;

    #[test]
    fn a_terminal_can_be_constructed_without_a_tty() {
        // Tests run with stdout redirected; construction must still succeed so that
        // non-interactive rendering paths work in CI.
        let terminal = Terminal::with_capabilities(Capabilities::plain(ColorDepth::TrueColor));
        assert!(terminal.is_ok());
    }

    #[test]
    fn refusing_a_non_terminal_names_the_requirement_not_the_failed_syscall() {
        let error = not_a_terminal();
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        let message = error.to_string();
        assert!(message.contains("terminal"), "{message}");
        assert!(message.contains("stdin"), "{message}");
        // The whole point is that this is *not* the platform's own error, which arrives as a
        // debug-printed `Os { code: .. }` and names a device rather than a requirement.
        assert!(!message.contains("code:"), "{message}");
    }

    #[test]
    fn asking_whether_there_is_a_terminal_does_not_take_one_over() {
        // Deliberately not asserting *which* answer: a developer running `cargo test` in a
        // terminal has a tty on stdin and CI does not, and both are correct. What has to hold is
        // that asking is free — no raw mode, no panic hook, and the same answer twice.
        let first = Terminal::is_interactive();
        assert_eq!(first, Terminal::is_interactive());
    }

    #[test]
    fn detection_disables_color_when_output_is_not_a_tty() {
        // Piping a conui app into a file should produce text, not escape sequences.
        assert_eq!(Capabilities::detect(false).color_depth, ColorDepth::NoColor);
    }

    #[test]
    fn leaving_without_entering_is_a_no_op() {
        let mut terminal =
            Terminal::with_capabilities(Capabilities::plain(ColorDepth::NoColor)).unwrap();
        assert!(terminal.leave().is_ok());
        assert!(terminal.leave().is_ok());
    }

    #[test]
    fn a_cell_size_is_the_window_divided_by_its_cells() {
        assert_eq!(divide_into_cells((1216, 901), (152, 53)), Some((8, 17)));
        // A retina cell, which is the case that makes assuming 8x17 wrong by a factor of two.
        assert_eq!(divide_into_cells((2432, 1802), (152, 53)), Some((16, 34)));
    }

    #[test]
    fn a_nonsense_report_is_clamped_rather_than_believed() {
        // No screen is 40 pixels wide, and a caller that rasterises for a 0-pixel cell divides by
        // zero somewhere downstream.
        assert_eq!(divide_into_cells((40, 40), (152, 53)), Some((4, 8)));
        assert_eq!(divide_into_cells((9000, 9000), (152, 53)), Some((20, 44)));
        // And a screen with no cells has no cell size, rather than a panic.
        assert_eq!(divide_into_cells((1216, 901), (0, 53)), None);
        assert_eq!(divide_into_cells((1216, 901), (152, 0)), None);
    }

    #[test]
    fn the_initial_size_is_never_degenerate() {
        let terminal =
            Terminal::with_capabilities(Capabilities::plain(ColorDepth::NoColor)).unwrap();
        let (width, height) = terminal.size();
        assert!(width > 0 && height > 0, "got {width}x{height}");
    }
}
