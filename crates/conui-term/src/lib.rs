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

pub mod ansi;
mod caps;
mod platform;
mod writer;

pub use caps::Capabilities;
pub use writer::{MONOCHROME_EMPHASIS, Painter};

use std::io::{self, Stdout, Write};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use conui_cell::{Buffer, Cell, Pos};

/// Terminal settings captured on entry, restored on exit.
pub use platform::SavedMode;

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
            "\x1b[?7h",    // autowrap back on
            "\x1b[?1049l", // leave the alternate screen
            "\x1b[?25h",   // show the cursor
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

    pub fn capabilities(&self) -> Capabilities {
        self.painter.capabilities()
    }

    /// The cell used to fill cleared regions. Set this to the theme's background so that a
    /// resize reveals the app's own color rather than the terminal's default.
    pub fn set_blank_cell(&mut self, cell: Cell) {
        self.blank = cell;
        self.front.reset_to(cell);
    }

    pub fn painter(&mut self) -> &mut Painter<Stdout> {
        &mut self.painter
    }

    /// Whether stdin is a terminal. An app should refuse interactive mode when this is false.
    pub fn is_interactive() -> bool {
        platform::is_input_tty()
    }

    /// Take over the terminal: raw mode, alternate screen, and a panic-safe restore.
    pub fn enter(&mut self) -> io::Result<()> {
        if self.entered {
            return Ok(());
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
    fn the_initial_size_is_never_degenerate() {
        let terminal =
            Terminal::with_capabilities(Capabilities::plain(ColorDepth::NoColor)).unwrap();
        let (width, height) = terminal.size();
        assert!(width > 0 && height > 0, "got {width}x{height}");
    }
}
