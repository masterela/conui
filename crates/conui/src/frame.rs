//! One frame of output, and the seam between the two API layers.
//!
//! A [`Frame`] is handed to your draw function once per frame. It owns the back buffer for that
//! frame and nothing else — no terminal, no state, no I/O — which is what makes a whole screen
//! testable: render into a [`Buffer`], read the rows back as strings, assert on them.
//!
//! Both layers are reachable from here and compose freely. [`Frame::render`] draws a view tree;
//! [`Frame::canvas`] hands you the cells. A view tree can contain a hand-painted region, and a
//! hand-painted region can call back into a view. Neither is privileged.

use std::time::Duration;

use conui_cell::{Buffer, Pos, Rect};

use crate::canvas::Canvas;
use crate::theme::Theme;
use crate::view::View;

/// The drawing surface for a single frame.
pub struct Frame<'a> {
    buffer: &'a mut Buffer,
    theme: Theme,
    cursor: Option<Pos>,
    count: u64,
    elapsed: Duration,
}

impl<'a> Frame<'a> {
    /// Wrap a buffer as a frame. The app runner does this for you; construct one directly to
    /// render a screen in a test.
    pub fn new(buffer: &'a mut Buffer, theme: Theme) -> Self {
        Self { buffer, theme, cursor: None, count: 0, elapsed: Duration::ZERO }
    }

    /// Record which frame this is and how long the app has been running, for animation.
    pub fn with_timing(mut self, count: u64, elapsed: Duration) -> Self {
        self.count = count;
        self.elapsed = elapsed;
        self
    }

    /// The whole drawable area, always at the origin.
    pub fn area(&self) -> Rect {
        self.buffer.area()
    }

    /// The drawable area as columns and rows.
    pub fn size(&self) -> (u16, u16) {
        (self.buffer.width(), self.buffer.height())
    }

    /// The palette this frame draws against.
    pub const fn theme(&self) -> &Theme {
        &self.theme
    }

    /// How many frames have been drawn before this one.
    pub const fn count(&self) -> u64 {
        self.count
    }

    /// Time since the app started. The right clock for animation: it does not assume a fixed
    /// frame rate, so an animation runs at the same speed whether the app is idle or busy.
    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Draw directly into the cells of `area`.
    ///
    /// The escape hatch, and not a second-class one — it is the same path every widget takes.
    ///
    /// ```
    /// # use conui::{Frame, Role, Theme};
    /// # use conui_cell::{Buffer, Rect};
    /// # let mut buffer = Buffer::new(12, 3);
    /// # let mut frame = Frame::new(&mut buffer, Theme::LAYA);
    /// frame.canvas(Rect::sized(12, 3), |c| {
    ///     c.number(0, 0, 42, 3, Role::Accent);
    /// });
    /// ```
    pub fn canvas(&mut self, area: Rect, body: impl FnOnce(&mut Canvas<'_>)) {
        let mut canvas = Canvas::new(self.buffer, area, self.theme);
        body(&mut canvas);
    }

    /// A canvas over `area`, when a closure is awkward.
    pub fn surface(&mut self, area: Rect) -> Canvas<'_> {
        Canvas::new(self.buffer, area, self.theme)
    }

    /// A canvas over the whole frame.
    pub fn full(&mut self) -> Canvas<'_> {
        Canvas::full(self.buffer, self.theme)
    }

    /// Render a view tree into `area`.
    pub fn render(&mut self, view: &dyn View, area: Rect) {
        let mut canvas = Canvas::new(self.buffer, area, self.theme);
        view.render(&mut canvas);
    }

    /// Render a view tree into the whole frame.
    pub fn render_full(&mut self, view: &dyn View) {
        let area = self.area();
        self.render(view, area);
    }

    /// Fill the frame with the theme background.
    pub fn clear(&mut self) {
        self.full().clear();
    }

    /// Show the terminal's own cursor at this cell after the frame is presented.
    ///
    /// Left hidden unless a frame asks for it, because a blinking block parked in a corner is
    /// the single most common tell that a terminal app was not finished. Set it when you have a
    /// text field, and only then — a screen reader and a user both follow it.
    pub fn set_cursor(&mut self, position: Pos) {
        self.cursor = Some(position);
    }

    /// Leave the cursor hidden for this frame, whatever an earlier call asked for.
    pub fn hide_cursor(&mut self) {
        self.cursor = None;
    }

    /// Where the cursor should end up, for the presenter.
    pub const fn cursor(&self) -> Option<Pos> {
        self.cursor
    }

    /// The buffer being drawn into, for code that wants cells without a canvas.
    pub fn buffer(&self) -> &Buffer {
        self.buffer
    }

    /// The buffer being drawn into, for code that writes cells directly.
    ///
    /// Bypasses clipping, which a [`Canvas`] would have done for you, so a bad coordinate here
    /// lands somewhere else on the screen rather than being dropped.
    ///
    /// [`Canvas`]: crate::canvas::Canvas
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Role;

    #[test]
    fn a_frame_draws_through_the_canvas_into_its_buffer() {
        let mut buffer = Buffer::new(8, 1);
        let mut frame = Frame::new(&mut buffer, Theme::LAYA);
        frame.canvas(Rect::sized(8, 1), |canvas| {
            canvas.put(1, 0, "ok", Role::Accent);
        });
        assert_eq!(buffer.row_text(0), " ok     ");
    }

    #[test]
    fn the_raw_buffer_writes_where_a_canvas_would_have_clipped() {
        // Both halves of what the doc promises. The write lands, and it lands at absolute
        // coordinates with no region to be relative to — which is exactly the hazard being
        // documented, so it is worth a test that would notice if clipping were quietly added.
        let mut buffer = Buffer::new(6, 2);
        let mut frame = Frame::new(&mut buffer, Theme::LAYA);
        frame.canvas(Rect::new(0, 0, 2, 1), |canvas| {
            canvas.text(0, 0, "ab");
        });
        frame.buffer_mut().set_str(3, 1, "xy", conui_cell::Style::EMPTY, 6);
        assert_eq!(buffer.row_text(0), "ab    ");
        assert_eq!(buffer.row_text(1), "   xy ");
    }

    #[test]
    fn a_region_canvas_is_offset_and_clipped() {
        let mut buffer = Buffer::new(8, 2);
        let mut frame = Frame::new(&mut buffer, Theme::LAYA);
        frame.canvas(Rect::new(4, 1, 2, 1), |canvas| {
            canvas.text(0, 0, "abcdef");
        });
        assert_eq!(buffer.row_text(0), "        ");
        assert_eq!(buffer.row_text(1), "    ab  ");
    }

    #[test]
    fn the_cursor_is_hidden_until_a_frame_asks_for_it() {
        let mut buffer = Buffer::new(4, 1);
        let mut frame = Frame::new(&mut buffer, Theme::LAYA);
        assert_eq!(frame.cursor(), None);
        frame.set_cursor(Pos { x: 2, y: 0 });
        assert_eq!(frame.cursor(), Some(Pos { x: 2, y: 0 }));
        frame.hide_cursor();
        assert_eq!(frame.cursor(), None);
    }
}
