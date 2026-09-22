//! The runner: terminal lifecycle, the event loop's waiting half, and frame presentation.
//!
//! [`App`] owns the three things every full-screen terminal program needs and most get subtly
//! wrong: raw mode restored on every exit path including a panic, input parsed incrementally
//! with the lone-`ESC` ambiguity resolved by a timeout, and a size that is re-read every frame
//! rather than trusted from startup.
//!
//! The loop stays yours. There is no `run(closure)` that owns your state, because the moment
//! your app needs to await something, own a channel, or step a simulation at its own rate, that
//! shape becomes a fight. What `App` offers instead is a `poll` that blocks correctly and a
//! `draw` that presents atomically:
//!
//! ```no_run
//! use conui::{App, Event, KeyCode};
//!
//! # fn main() -> std::io::Result<()> {
//! let mut app = App::new()?;
//! while app.is_running() {
//!     for event in app.poll()? {
//!         match event {
//!             Event::Key(key) if key.code == KeyCode::Char('q') => app.quit(),
//!             _ => {}
//!         }
//!     }
//!     app.draw(|frame| frame.clear())?;
//! }
//! # Ok(())
//! # }
//! ```

use std::io;
use std::time::{Duration, Instant};

use conui_cell::{Buffer, Cell, Symbol};
use conui_input::{ESCAPE_TIMEOUT, Event, KeyCode, KeyEvent, Modifiers, Parser};
use conui_term::{Capabilities, Terminal};

use crate::canvas::text_width;
use crate::frame::Frame;
use crate::theme::{Role, Theme};

/// How large an input read may be in one go. Larger than any plausible keystroke burst, so a
/// held-down key or a big paste arrives in a handful of reads rather than hundreds.
const READ_CHUNK: usize = 4096;

/// Everything about an app that is decided before the first frame.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub theme: Theme,
    /// How long [`App::poll`] will wait for input before returning so you can redraw.
    ///
    /// `None` blocks until something happens, which is right for an editor and wrong for
    /// anything that animates: with no tick there is no frame to animate on.
    pub tick_rate: Option<Duration>,
    /// Below this size the app's own view is replaced by a resize prompt.
    pub min_size: (u16, u16),
    pub mouse: bool,
    /// Whether `Ctrl+C` stops the loop.
    ///
    /// On by default. Raw mode means the kernel no longer turns `Ctrl+C` into a signal, so an
    /// app that does not handle the key itself cannot be interrupted at all — the user's only
    /// remaining move is to kill the terminal window.
    pub quit_on_ctrl_c: bool,
}

impl Config {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn theme(mut self, theme: Theme) -> Self {
        self.theme = theme;
        self
    }

    /// Redraw at roughly this rate even when nothing arrives.
    pub fn tick_rate(mut self, rate: Duration) -> Self {
        self.tick_rate = Some(rate);
        self
    }

    /// Redraw at this many frames per second.
    pub fn fps(self, fps: u32) -> Self {
        let rate = if fps == 0 { Duration::ZERO } else { Duration::from_secs(1) / fps };
        self.tick_rate(rate)
    }

    /// Wait indefinitely for input, drawing only in response to events.
    pub fn no_tick(mut self) -> Self {
        self.tick_rate = None;
        self
    }

    pub fn min_size(mut self, width: u16, height: u16) -> Self {
        self.min_size = (width, height);
        self
    }

    pub fn mouse(mut self, enabled: bool) -> Self {
        self.mouse = enabled;
        self
    }

    /// Let the app see `Ctrl+C` without the runner stopping the loop.
    ///
    /// Only worth doing if you handle it yourself — a "really quit?" prompt, or a save first.
    pub fn keep_ctrl_c(mut self) -> Self {
        self.quit_on_ctrl_c = false;
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::LAYA,
            // Thirty frames a second: past the point where motion reads as smooth, and far
            // below the point where a redraw costs anything measurable.
            tick_rate: Some(Duration::from_millis(33)),
            min_size: (40, 10),
            mouse: false,
            quit_on_ctrl_c: true,
        }
    }
}

/// A running terminal application.
pub struct App {
    terminal: Terminal,
    parser: Parser,
    /// The frame being built, reused so a steady-state redraw allocates nothing.
    back: Buffer,
    config: Config,
    running: bool,
    start: Instant,
    frames: u64,
    next_tick: Instant,
    read_buffer: Box<[u8; READ_CHUNK]>,
}

impl App {
    /// Take over the terminal with the default configuration.
    pub fn new() -> io::Result<Self> {
        Self::with(Config::default())
    }

    /// Take over the terminal with an explicit configuration.
    pub fn with(config: Config) -> io::Result<Self> {
        Self::build(Terminal::new()?, config)
    }

    /// Take over the terminal with capabilities you choose, bypassing detection.
    ///
    /// The escape hatch for a terminal that lies about itself in either direction — and the way
    /// to force colour when output is a pipe, which detection otherwise disables.
    pub fn with_capabilities(caps: Capabilities, config: Config) -> io::Result<Self> {
        Self::build(Terminal::with_capabilities(caps)?, config)
    }

    fn build(terminal: Terminal, config: Config) -> io::Result<Self> {
        let mut app = Self {
            terminal,
            parser: Parser::new(),
            back: Buffer::new(1, 1),
            config,
            running: true,
            start: Instant::now(),
            frames: 0,
            next_tick: Instant::now(),
            read_buffer: Box::new([0; READ_CHUNK]),
        };
        app.terminal.set_blank_cell(blank_cell(config.theme));
        app.terminal.enter()?;
        if config.mouse {
            app.terminal.set_mouse_capture(true)?;
        }
        let (width, height) = app.terminal.size();
        app.back.resize(width, height);
        // Reset the clock after entering: opening the screen involves a flush and an ioctl, and
        // charging that to the app's first animation frame would make it stutter.
        app.start = Instant::now();
        app.next_tick = app.start;
        Ok(app)
    }

    // ---- State --------------------------------------------------------------------------

    /// Whether the loop should keep going.
    pub const fn is_running(&self) -> bool {
        self.running
    }

    /// Ask the loop to stop. Takes effect when your loop next checks [`App::is_running`].
    pub fn quit(&mut self) {
        self.running = false;
    }

    pub const fn config(&self) -> &Config {
        &self.config
    }

    /// Change palette while running.
    pub fn set_theme(&mut self, theme: Theme) {
        self.config.theme = theme;
        self.terminal.set_blank_cell(blank_cell(theme));
        // The ground colour is on every cell, so the whole screen has to be re-sent.
        self.terminal.force_repaint();
    }

    pub fn set_tick_rate(&mut self, rate: Option<Duration>) {
        self.config.tick_rate = rate;
    }

    pub fn set_mouse(&mut self, enabled: bool) -> io::Result<()> {
        self.config.mouse = enabled;
        self.terminal.set_mouse_capture(enabled)
    }

    pub fn size(&self) -> (u16, u16) {
        self.terminal.size()
    }

    /// Whether the terminal is below [`Config::min_size`], and so showing the resize prompt.
    pub fn is_too_small(&self) -> bool {
        let (width, height) = self.terminal.size();
        width < self.config.min_size.0 || height < self.config.min_size.1
    }

    /// Time since the first frame.
    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    /// Frames presented so far.
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    pub fn terminal(&mut self) -> &mut Terminal {
        &mut self.terminal
    }

    // ---- Loop ---------------------------------------------------------------------------

    /// Wait for input up to the tick deadline and return everything that arrived.
    ///
    /// Returns an empty vector on a plain tick, which is normal and not an error: a frame with
    /// no input is how animation happens.
    pub fn poll(&mut self) -> io::Result<Vec<Event>> {
        let mut events = Vec::new();

        // Size first, so a view laid out this frame sees the real dimensions. Polling rather
        // than catching `SIGWINCH` keeps conui out of the process's signal dispositions.
        if let Some((width, height)) = self.terminal.sync_size()? {
            self.back.resize(width, height);
            events.push(Event::Resize { width, height });
        }

        let wait = self.wait_budget();
        if self.terminal.wait_readable(wait)? {
            let count = self.terminal.read_input(&mut self.read_buffer[..])?;
            if count == 0 {
                // End of input: the other side of the tty is gone. Continuing would spin.
                self.running = false;
            } else {
                self.parser.feed(&self.read_buffer[..count]);
            }
        } else if self.parser.has_pending() {
            // Nothing more is coming, so a held-back `ESC` is a real Escape key and a paste
            // that stopped mid-stream is delivered as far as it got.
            self.parser.flush_timeout();
        }

        events.extend(self.parser.drain());
        self.advance_tick();

        if self.config.quit_on_ctrl_c && events.iter().any(is_interrupt) {
            self.running = false;
        }
        Ok(events)
    }

    /// Wait for the next event, blocking for as long as it takes.
    ///
    /// For apps with nothing to animate. Still honours the escape timeout internally, so a
    /// lone `Escape` keypress is not swallowed until the user types something else.
    pub fn next_event(&mut self) -> io::Result<Option<Event>> {
        loop {
            if !self.running {
                return Ok(None);
            }
            let events = self.poll()?;
            if let Some(event) = events.into_iter().next() {
                return Ok(Some(event));
            }
            if self.config.tick_rate.is_some() {
                // A tick expired with nothing to report; the caller asked for one event, not a
                // frame, so keep waiting rather than returning a meaningless `None`.
                continue;
            }
        }
    }

    /// How long to block for input: until the next tick, or until the escape timeout if the
    /// parser is sitting on an ambiguous byte, whichever is sooner.
    fn wait_budget(&self) -> Option<Duration> {
        let tick =
            self.config.tick_rate.map(|_| self.next_tick.saturating_duration_since(Instant::now()));
        match (tick, self.parser.has_pending()) {
            (Some(tick), true) => Some(tick.min(ESCAPE_TIMEOUT)),
            (Some(tick), false) => Some(tick),
            (None, true) => Some(ESCAPE_TIMEOUT),
            (None, false) => None,
        }
    }

    /// Move the tick deadline forward, skipping any deadlines already missed.
    fn advance_tick(&mut self) {
        let Some(rate) = self.config.tick_rate else { return };
        let now = Instant::now();
        if self.next_tick > now {
            return;
        }
        if rate.is_zero() {
            self.next_tick = now;
            return;
        }
        // Catch up in whole ticks rather than from `now`, so a frame that overran does not
        // shift the phase of every frame after it. Bounded so that returning from a suspend
        // does not replay hours of missed ticks.
        let behind = now.saturating_duration_since(self.next_tick);
        let skipped = (behind.as_nanos() / rate.as_nanos()).min(1024) as u32 + 1;
        self.next_tick += rate * skipped;
    }

    /// Render and present one frame.
    ///
    /// The buffer starts blank, so `body` describes the whole screen rather than patching the
    /// last one; only the cells that actually changed are written to the terminal.
    pub fn draw(&mut self, body: impl FnOnce(&mut Frame<'_>)) -> io::Result<()> {
        let (width, height) = self.terminal.size();
        if (self.back.width(), self.back.height()) != (width, height) {
            self.back.resize(width, height);
        }
        self.back.reset_to(blank_cell(self.config.theme));

        let theme = self.config.theme;
        let elapsed = self.start.elapsed();
        let min_size = self.config.min_size;
        let too_small = self.is_too_small();

        let mut frame = Frame::new(&mut self.back, theme).with_timing(self.frames, elapsed);
        if too_small {
            // Drawing the app's view into a space it cannot fit produces a scrambled screen and
            // a bug report. Saying what is wrong, and what would fix it, produces neither.
            draw_resize_prompt(&mut frame, min_size);
        } else {
            body(&mut frame);
        }
        let cursor = frame.cursor();

        self.frames += 1;
        self.terminal.present(&self.back, cursor)
    }

    /// Repaint every cell on the next [`App::draw`], discarding what we believe is on screen.
    ///
    /// Needed after something else has written to the same terminal — a subprocess, a logger
    /// that escaped to stderr, a shell job resumed underneath us.
    pub fn force_repaint(&mut self) {
        self.terminal.force_repaint();
    }

    /// Give the terminal back early. Idempotent; [`Drop`] does it anyway.
    pub fn leave(&mut self) -> io::Result<()> {
        self.running = false;
        self.terminal.leave()
    }
}

/// The cell an untouched part of the screen is filled with.
fn blank_cell(theme: Theme) -> Cell {
    Cell::new(Symbol::SPACE, theme.ground())
}

/// Whether an event is `Ctrl+C`.
fn is_interrupt(event: &Event) -> bool {
    matches!(
        event,
        Event::Key(KeyEvent { code: KeyCode::Char('c'), modifiers, .. })
            if modifiers.contains(Modifiers::CTRL)
    )
}

/// The screen shown when the terminal is smaller than the app's minimum.
fn draw_resize_prompt(frame: &mut Frame<'_>, (width, height): (u16, u16)) {
    let (screen_width, screen_height) = frame.size();
    let mut canvas = frame.full();
    canvas.clear();

    let message = format!("Resize to at least {width} × {height}");
    let current = format!("now {screen_width} × {screen_height}");
    let middle = i32::from(screen_height / 2);

    // Centred, and shortened rather than clipped: at eight columns wide there is no room for a
    // sentence, but there is always room for the numbers.
    if text_width(&message) <= screen_width {
        canvas.put_centered(middle, &message, Role::Text);
        if middle + 1 < i32::from(screen_height) {
            canvas.put_centered(middle + 1, &current, Role::Muted);
        }
    } else {
        canvas.put_centered(middle, &format!("{width}×{height}"), Role::Text);
    }
}

impl Drop for App {
    fn drop(&mut self) {
        // `Terminal` restores itself on drop too; doing it here as well means the screen is
        // handed back before any of the app's own fields run their destructors.
        let _ = self.terminal.leave();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conui_cell::ColorDepth;

    /// An app over a non-tty stdout. Entering raw mode fails when tests are run with stdin
    /// redirected, so these only cover what does not need a terminal.
    fn config() -> Config {
        Config::new().min_size(20, 5)
    }

    #[test]
    fn fps_becomes_a_tick_rate() {
        assert_eq!(Config::new().fps(30).tick_rate, Some(Duration::from_secs(1) / 30));
        assert_eq!(Config::new().fps(1).tick_rate, Some(Duration::from_secs(1)));
    }

    #[test]
    fn a_zero_fps_request_does_not_divide_by_zero() {
        assert_eq!(Config::new().fps(0).tick_rate, Some(Duration::ZERO));
    }

    #[test]
    fn no_tick_means_block_forever() {
        assert_eq!(Config::new().no_tick().tick_rate, None);
    }

    #[test]
    fn ctrl_c_is_recognised_and_plain_c_is_not() {
        let interrupt = Event::Key(KeyEvent::new(KeyCode::Char('c'), Modifiers::CTRL));
        let letter = Event::Key(KeyEvent::plain(KeyCode::Char('c')));
        assert!(is_interrupt(&interrupt));
        assert!(!is_interrupt(&letter));
    }

    #[test]
    fn the_blank_cell_carries_the_theme_ground() {
        let cell = blank_cell(Theme::LAYA);
        assert_eq!(cell.symbol, Symbol::SPACE);
        assert_eq!(cell.style.bg, Some(Theme::LAYA.background));
    }

    #[test]
    fn the_resize_prompt_says_what_is_needed_and_what_there_is() {
        let mut buffer = Buffer::new(40, 6);
        let mut frame = Frame::new(&mut buffer, Theme::LAYA);
        draw_resize_prompt(&mut frame, (80, 24));
        let rendered: String = (0..6).map(|row| buffer.row_text(row)).collect();
        assert!(rendered.contains("Resize to at least 80 × 24"), "got {rendered:?}");
        assert!(rendered.contains("now 40 × 6"), "got {rendered:?}");
    }

    #[test]
    fn the_resize_prompt_degrades_to_numbers_in_a_tiny_window() {
        let mut buffer = Buffer::new(8, 2);
        let mut frame = Frame::new(&mut buffer, Theme::LAYA);
        draw_resize_prompt(&mut frame, (80, 24));
        let rendered: String = (0..2).map(|row| buffer.row_text(row)).collect();
        assert!(rendered.contains("80×24"), "got {rendered:?}");
    }

    #[test]
    fn a_config_is_cheap_to_copy_so_it_can_be_read_mid_frame() {
        let first = config();
        let second = first;
        assert_eq!(first.min_size, second.min_size);
    }

    #[test]
    fn capabilities_can_be_forced_for_a_non_tty() {
        // Not a full app: constructing one would try to enter raw mode. This only asserts the
        // capability plumbing compiles and that detection is bypassable.
        let caps = Capabilities::plain(ColorDepth::TrueColor);
        assert_eq!(caps.color_depth, ColorDepth::TrueColor);
    }
}
