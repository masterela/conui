//! Rendering a screen with no terminal attached, and reading it back as text.
//!
//! A [`Frame`] already needs nothing but a [`Buffer`], which is what makes a whole conui screen
//! checkable without a tty. The eight lines that do it — make a buffer, wrap it, draw, read the rows
//! back trimmed — were nevertheless written out by hand in every app that wanted them, twice each:
//! once in the tests and once in the `--dump` flag that prints a frame to stdout. They came out
//! subtly different every time, and the differences were all in the same place: whether rows are
//! trimmed, whether the last one carries a newline, and what a failing assertion prints.
//!
//! So this is not a test utility, and is not behind `cfg(test)`. A `--dump` path ships.
//!
//! ```
//! use conui::headless::Screen;
//! use conui::widget::{Panel, Text};
//!
//! let mut screen = Screen::new(24, 4);
//! screen.view(&Panel::new("greeting").child(Text::new("Hi")));
//! screen.assert_shows("greeting");
//! print!("{screen}");
//! ```

use std::fmt;

use conui_cell::{Buffer, Pos};

use crate::frame::Frame;
use crate::theme::Theme;
use crate::view::View;

/// A frame rendered with no terminal, readable as text.
///
/// Holds the buffer between draws, so a caller can render, assert, render again at a different size,
/// and assert on that — which is how a layout's degradation gets checked at all.
pub struct Screen {
    buffer: Buffer,
    theme: Theme,
    cursor: Option<Pos>,
}

impl Screen {
    /// A screen of `width` by `height` cells, against [`Theme::LAYA`].
    ///
    /// The default is a real theme rather than a blank one because a screen drawn against no palette
    /// is a screen nobody ships, and the colours do not reach the text a caller asserts on anyway.
    pub fn new(width: u16, height: u16) -> Self {
        Self::themed(width, height, Theme::LAYA)
    }

    /// The same, against a palette of your choosing.
    ///
    /// Worth doing when the thing under test is the palette: a light skin that leaves one role
    /// unreadable is a bug that renders perfectly.
    pub fn themed(width: u16, height: u16, theme: Theme) -> Self {
        Self { buffer: Buffer::new(width, height), theme, cursor: None }
    }

    /// Draw into the screen the way an app's draw function would.
    ///
    /// The buffer is cleared to the theme's ground first, so two draws in a row do not leave the
    /// first one's text showing through the second's gaps — which on a real terminal they would not,
    /// because a frame is diffed against the front buffer rather than painted over it.
    pub fn draw(&mut self, body: impl FnOnce(&mut Frame<'_>)) -> &mut Self {
        let mut frame = Frame::new(&mut self.buffer, self.theme);
        frame.clear();
        body(&mut frame);
        self.cursor = frame.cursor();
        self
    }

    /// Draw one view across the whole screen. The common case, and the shortest way to spell it.
    pub fn view(&mut self, view: &dyn View) -> &mut Self {
        self.draw(|frame| frame.render_full(view))
    }

    /// The screen as one string, rows joined by newlines, each trailing blank trimmed.
    ///
    /// Trimmed because trailing spaces are the one part of a frame that is never deliberate, and an
    /// assertion that fails on them wastes the reader's time. There is no trailing newline; see
    /// [`Screen::to_string`] via [`fmt::Display`] for the form a `--dump` prints.
    pub fn text(&self) -> String {
        self.rows().join("\n")
    }

    /// Every row, top to bottom, each trailing blank trimmed.
    pub fn rows(&self) -> Vec<String> {
        (0..self.buffer.height()).map(|row| self.row(row)).collect()
    }

    /// One row by index, its trailing blanks trimmed. Empty past the bottom of the screen.
    pub fn row(&self, y: u16) -> String {
        if y >= self.buffer.height() {
            return String::new();
        }
        self.buffer.row_text(y).trim_end().to_string()
    }

    /// Whether `needle` appears anywhere on one row.
    ///
    /// One row, not the joined text, so a needle cannot match across a line break and quietly pass.
    pub fn contains(&self, needle: &str) -> bool {
        (0..self.buffer.height()).any(|row| self.buffer.row_text(row).contains(needle))
    }

    /// The index of the first row containing `needle`, if any.
    pub fn find(&self, needle: &str) -> Option<u16> {
        (0..self.buffer.height()).find(|&row| self.buffer.row_text(row).contains(needle))
    }

    /// Assert that `needle` is on screen, and print the whole screen when it is not.
    ///
    /// The reason to use this rather than `assert!(screen.contains(..))`: a layout assertion that
    /// fails tells you nothing without the layout, and reaching for the screen after the fact means
    /// rerunning with an `eprintln!` in it. The panic message carries the frame.
    ///
    /// # Panics
    ///
    /// When `needle` appears on no row.
    #[track_caller]
    pub fn assert_shows(&self, needle: &str) -> &Self {
        assert!(self.contains(needle), "{needle:?} is not on this screen:\n{self}");
        self
    }

    /// Assert that `needle` is nowhere on screen, and print the screen when it is.
    ///
    /// # Panics
    ///
    /// When `needle` appears on any row.
    #[track_caller]
    pub fn assert_hides(&self, needle: &str) -> &Self {
        match self.find(needle) {
            None => self,
            Some(row) => {
                panic!("{needle:?} should not be on this screen, but row {row} has it:\n{self}")
            }
        }
    }

    /// Where the caret was left, if the last draw placed one.
    pub fn cursor(&self) -> Option<Pos> {
        self.cursor
    }

    /// The cells themselves, for asserting on a colour or an attribute rather than on the text.
    pub fn buffer(&self) -> &Buffer {
        &self.buffer
    }

    /// The size in cells.
    pub fn size(&self) -> (u16, u16) {
        (self.buffer.width(), self.buffer.height())
    }

    /// Resize and clear, for checking the same view at a width that forces it to degrade.
    pub fn resize(&mut self, width: u16, height: u16) -> &mut Self {
        self.buffer.resize(width, height);
        self.buffer.clear();
        self
    }
}

/// The form a `--dump` flag prints: every row trimmed, each ending in a newline.
///
/// Including the last, so the shell prompt that follows starts at a column of its own.
impl fmt::Display for Screen {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for row in 0..self.buffer.height() {
            writeln!(f, "{}", self.row(row))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::Role;
    use crate::widget::{Panel, Text};

    #[test]
    fn a_view_renders_and_reads_back_as_text() {
        let mut screen = Screen::new(30, 5);
        screen.view(&Panel::new("head").child(Text::new("body")));
        screen.assert_shows("head").assert_shows("body").assert_hides("elsewhere");
    }

    #[test]
    fn rows_are_trimmed_but_not_dropped() {
        let mut screen = Screen::new(20, 4);
        screen.draw(|frame| {
            frame.full().put(0, 2, "third", Role::Text);
        });
        let rows = screen.rows();
        assert_eq!(rows.len(), 4, "every row is present, blank or not");
        assert_eq!(rows[2], "third", "with no trailing blanks on it");
        assert_eq!(rows[0], "");
        assert_eq!(screen.find("third"), Some(2));
    }

    #[test]
    fn the_display_form_ends_every_row_with_a_newline() {
        let mut screen = Screen::new(8, 2);
        screen.draw(|frame| {
            frame.full().put(0, 0, "a", Role::Text);
        });
        assert_eq!(screen.to_string(), "a\n\n");
    }

    #[test]
    fn a_second_draw_does_not_show_the_first_one_through() {
        let mut screen = Screen::new(20, 2);
        screen.draw(|frame| {
            frame.full().put(0, 0, "before", Role::Text);
        });
        screen.draw(|frame| {
            frame.full().put(0, 1, "after", Role::Text);
        });
        assert!(!screen.contains("before"), "{screen}");
        assert!(screen.contains("after"), "{screen}");
    }

    #[test]
    fn a_resize_lets_the_same_view_be_checked_narrow() {
        let mut screen = Screen::new(40, 6);
        let panel = || Panel::new("head").child(Text::new("a body wide enough to wrap"));
        screen.view(&panel());
        let wide = screen.text();
        screen.resize(18, 6).view(&panel());
        assert_ne!(screen.text(), wide, "the narrow render is a different shape");
        assert_eq!(screen.size(), (18, 6));
    }

    #[test]
    fn a_row_past_the_bottom_is_empty_rather_than_a_panic() {
        let screen = Screen::new(4, 2);
        assert_eq!(screen.row(9), "");
    }

    #[test]
    #[should_panic(expected = "is not on this screen")]
    fn a_missing_needle_panics_with_the_screen_in_the_message() {
        Screen::new(10, 2).assert_shows("nope");
    }
}
