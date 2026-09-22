//! The screen as a flat grid of cells, plus the frame differ.
//!
//! Rendering never talks to the terminal. It writes into a [`Buffer`], and once the frame is
//! complete the buffer is compared against the one currently on screen. Only the cells that
//! actually changed are sent. That is what keeps a 60 fps repaint cheap and, more
//! importantly, flicker-free: no clear-then-redraw, so there is never a blank intermediate
//! state for the eye to catch.

use unicode_segmentation::UnicodeSegmentation;

use crate::{Cell, Rect, Style, Symbol};

/// One changed cell, as produced by [`Buffer::diff`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Patch {
    pub x: u16,
    pub y: u16,
    pub cell: Cell,
}

/// A rectangular grid of cells.
///
/// Always anchored at the origin, because it models a whole screen. Sub-regions are
/// addressed with a [`Rect`], not by allocating smaller buffers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Buffer {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl Buffer {
    /// A blank buffer of the given size.
    pub fn new(width: u16, height: u16) -> Self {
        Self { width, height, cells: vec![Cell::BLANK; width as usize * height as usize] }
    }

    /// A buffer where every cell starts as `cell`. Used to paint the app background once,
    /// so a theme's background color is a property of the buffer rather than of every widget.
    pub fn filled(width: u16, height: u16, cell: Cell) -> Self {
        Self { width, height, cells: vec![cell; width as usize * height as usize] }
    }

    pub const fn width(&self) -> u16 {
        self.width
    }

    pub const fn height(&self) -> u16 {
        self.height
    }

    /// The whole buffer as a rect, which is the root area every layout starts from.
    pub const fn area(&self) -> Rect {
        Rect::sized(self.width, self.height)
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    const fn index_of(&self, x: u16, y: u16) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        Some(y as usize * self.width as usize + x as usize)
    }

    pub fn get(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index_of(x, y).map(|index| &self.cells[index])
    }

    pub fn get_mut(&mut self, x: u16, y: u16) -> Option<&mut Cell> {
        self.index_of(x, y).map(|index| &mut self.cells[index])
    }

    /// Resize in place, discarding contents.
    ///
    /// The old contents are deliberately dropped rather than reflowed: after a resize the
    /// next frame is a full repaint anyway, and preserving a partial grid would only risk
    /// leaving stale cells that the differ then considers up to date.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.cells.clear();
        self.cells.resize(width as usize * height as usize, Cell::BLANK);
    }

    /// Reset every cell to `cell`.
    pub fn reset_to(&mut self, cell: Cell) {
        self.cells.fill(cell);
    }

    /// Reset every cell to blank.
    pub fn clear(&mut self) {
        self.reset_to(Cell::BLANK);
    }

    /// Fill `area` with `cell`, clipped to the buffer.
    pub fn fill(&mut self, area: Rect, cell: Cell) {
        let area = area.intersection(self.area());
        for y in area.y..area.bottom() {
            // Clear any wide grapheme straddling either vertical edge before overwriting,
            // so its orphaned half does not survive inside the filled region.
            self.split_grapheme(area.x, y);
            self.split_grapheme(area.right(), y);
            let Some(start) = self.index_of(area.x, y) else { continue };
            let end = start + area.width as usize;
            self.cells[start..end].fill(cell);
        }
    }

    /// Apply `style` to every cell in `area` without touching its symbols.
    pub fn style_area(&mut self, area: Rect, style: Style) {
        let area = area.intersection(self.area());
        for position in area.positions() {
            if let Some(cell) = self.get_mut(position.x, position.y) {
                cell.style = cell.style.patch(style);
            }
        }
    }

    /// If a double-width grapheme occupies column `x`, blank both of its halves.
    ///
    /// Writing into one half of a wide grapheme would otherwise leave the other half behind
    /// as a fragment, and the terminal would render a stray glyph the layout never asked for.
    fn split_grapheme(&mut self, x: u16, y: u16) {
        let Some(cell) = self.get(x, y).copied() else { return };
        if cell.is_continuation() {
            if let Some(lead) = x.checked_sub(1).and_then(|left| self.get_mut(left, y)) {
                *lead = Cell::BLANK;
            }
            if let Some(tail) = self.get_mut(x, y) {
                *tail = Cell::BLANK;
            }
        } else if cell.width() == 2 {
            if let Some(lead) = self.get_mut(x, y) {
                *lead = Cell::BLANK;
            }
            if let Some(tail) = self.get_mut(x + 1, y) {
                *tail = Cell::BLANK;
            }
        }
    }

    /// Write one grapheme at `(x, y)` and return the columns it consumed.
    ///
    /// Returns 0 when the position is off-buffer. A double-width grapheme that would hang off
    /// the right edge is replaced by a space: terminals disagree on whether to wrap, truncate
    /// or overflow such a glyph, and a space is the one outcome that keeps the grid aligned.
    pub fn set_symbol(&mut self, x: u16, y: u16, symbol: impl Into<Symbol>, style: Style) -> u16 {
        let symbol = symbol.into();
        if symbol.is_empty() || x >= self.width || y >= self.height {
            return 0;
        }
        let (symbol, width) = match symbol.width() {
            2 if x + 2 > self.width => (Symbol::SPACE, 1),
            width => (symbol, width),
        };
        for column in x..x + width {
            self.split_grapheme(column, y);
        }
        if let Some(cell) = self.get_mut(x, y) {
            *cell = Cell::new(symbol, style);
        }
        if width == 2 {
            if let Some(tail) = self.get_mut(x + 1, y) {
                *tail = Cell::CONTINUATION;
            }
        }
        width
    }

    /// Write `text` starting at `(x, y)`, stopping at `max_width` columns or the buffer edge.
    ///
    /// Splits on grapheme clusters, so multi-codepoint characters stay intact, and never
    /// wraps: a line of UI is a line, and overflow is the caller's decision to make.
    /// Returns the number of columns written.
    pub fn set_str(&mut self, x: u16, y: u16, text: &str, style: Style, max_width: u16) -> u16 {
        if y >= self.height {
            return 0;
        }
        let limit = max_width.min(self.width.saturating_sub(x));
        let mut written = 0u16;
        for grapheme in text.graphemes(true) {
            // Newlines and other controls would desynchronise the grid; a caller that wants
            // multiple lines asks for multiple lines.
            if grapheme == "\n" || grapheme == "\r" {
                break;
            }
            let symbol = Symbol::new(grapheme);
            let width = symbol.width();
            if written + width > limit {
                break;
            }
            written += self.set_symbol(x + written, y, symbol, style);
        }
        written
    }

    /// The cells that differ from `previous`, in the order the writer should emit them.
    ///
    /// Continuation cells are skipped: the wide grapheme to their left already advanced the
    /// terminal's cursor across that column. A change confined to a continuation cell is not
    /// possible, because the lead cell that owns it must have changed too.
    pub fn diff(&self, previous: &Self) -> Vec<Patch> {
        // A size change invalidates every coordinate, so nothing can be reused.
        if previous.width != self.width || previous.height != self.height {
            return self.full_repaint();
        }
        let mut patches = Vec::new();
        for (index, (cell, old)) in self.cells.iter().zip(previous.cells.iter()).enumerate() {
            if cell == old || cell.is_continuation() {
                continue;
            }
            let (x, y) = self.coords(index);
            patches.push(Patch { x, y, cell: *cell });
        }
        patches
    }

    /// Every drawable cell, as if the screen held nothing. Used for the first frame and
    /// after a resize or a suspend/resume, where the on-screen state is unknown.
    pub fn full_repaint(&self) -> Vec<Patch> {
        self.cells
            .iter()
            .enumerate()
            .filter(|(_, cell)| !cell.is_continuation())
            .map(|(index, cell)| {
                let (x, y) = self.coords(index);
                Patch { x, y, cell: *cell }
            })
            .collect()
    }

    const fn coords(&self, index: usize) -> (u16, u16) {
        ((index % self.width as usize) as u16, (index / self.width as usize) as u16)
    }

    /// Render a row as plain text, for tests and snapshot assertions.
    pub fn row_text(&self, y: u16) -> String {
        let mut text = String::new();
        for x in 0..self.width {
            match self.get(x, y) {
                Some(cell) if !cell.is_continuation() => text.push_str(cell.symbol.as_str()),
                _ => {}
            }
        }
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;

    fn styled(color: &str) -> Style {
        Style::new().fg(Color::hex(color))
    }

    #[test]
    fn set_str_writes_and_reports_its_width() {
        let mut buffer = Buffer::new(20, 1);
        let written = buffer.set_str(2, 0, "SCORE", Style::EMPTY, 20);
        assert_eq!(written, 5);
        assert_eq!(buffer.row_text(0).trim_end(), "  SCORE");
    }

    #[test]
    fn set_str_honours_max_width_without_splitting_a_grapheme() {
        let mut buffer = Buffer::new(20, 1);
        // Limit of 3 cannot fit a second double-width glyph, so it stops at one.
        let written = buffer.set_str(0, 0, "界界界", Style::EMPTY, 3);
        assert_eq!(written, 2);
        assert_eq!(buffer.row_text(0).trim_end(), "界");
    }

    #[test]
    fn set_str_clips_at_the_right_edge() {
        let mut buffer = Buffer::new(4, 1);
        assert_eq!(buffer.set_str(2, 0, "abcdef", Style::EMPTY, 99), 2);
        assert_eq!(buffer.row_text(0), "  ab");
    }

    #[test]
    fn wide_grapheme_claims_a_continuation_cell() {
        let mut buffer = Buffer::new(6, 1);
        assert_eq!(buffer.set_symbol(1, 0, "🦀", Style::EMPTY), 2);
        assert_eq!(buffer.get(1, 0).unwrap().symbol.as_str(), "🦀");
        assert!(buffer.get(2, 0).unwrap().is_continuation());
    }

    #[test]
    fn wide_grapheme_at_the_last_column_becomes_a_space() {
        let mut buffer = Buffer::new(4, 1);
        assert_eq!(buffer.set_symbol(3, 0, "🦀", Style::EMPTY), 1);
        assert_eq!(buffer.get(3, 0).unwrap().symbol.as_str(), " ");
    }

    #[test]
    fn overwriting_the_lead_of_a_wide_grapheme_clears_its_tail() {
        let mut buffer = Buffer::new(6, 1);
        buffer.set_symbol(1, 0, "🦀", Style::EMPTY);
        buffer.set_symbol(1, 0, "x", Style::EMPTY);
        assert_eq!(buffer.get(1, 0).unwrap().symbol.as_str(), "x");
        assert_eq!(buffer.get(2, 0).unwrap().symbol.as_str(), " ", "orphaned tail must be blanked");
    }

    #[test]
    fn overwriting_the_tail_of_a_wide_grapheme_clears_its_lead() {
        let mut buffer = Buffer::new(6, 1);
        buffer.set_symbol(1, 0, "🦀", Style::EMPTY);
        buffer.set_symbol(2, 0, "x", Style::EMPTY);
        assert_eq!(buffer.get(1, 0).unwrap().symbol.as_str(), " ", "orphaned lead must be blanked");
        assert_eq!(buffer.get(2, 0).unwrap().symbol.as_str(), "x");
    }

    #[test]
    fn a_wide_write_landing_on_another_wide_glyph_cleans_up_both() {
        let mut buffer = Buffer::new(8, 1);
        buffer.set_symbol(2, 0, "界", Style::EMPTY);
        // Overlaps only the tail of the existing glyph at column 2..4.
        buffer.set_symbol(3, 0, "🦀", Style::EMPTY);
        assert_eq!(buffer.get(2, 0).unwrap().symbol.as_str(), " ");
        assert_eq!(buffer.get(3, 0).unwrap().symbol.as_str(), "🦀");
        assert!(buffer.get(4, 0).unwrap().is_continuation());
    }

    #[test]
    fn fill_blanks_wide_glyphs_straddling_its_edges() {
        let mut buffer = Buffer::new(10, 1);
        buffer.set_symbol(1, 0, "界", Style::EMPTY); // spans 1..3
        buffer.set_symbol(5, 0, "界", Style::EMPTY); // spans 5..7
        buffer.fill(Rect::new(2, 0, 4, 1), Cell::new('.', Style::EMPTY));
        assert_eq!(buffer.get(1, 0).unwrap().symbol.as_str(), " ", "left straddle cleared");
        assert_eq!(buffer.get(6, 0).unwrap().symbol.as_str(), " ", "right straddle cleared");
        assert_eq!(buffer.row_text(0), "  ....    ");
    }

    #[test]
    fn writes_outside_the_buffer_are_dropped() {
        let mut buffer = Buffer::new(4, 2);
        assert_eq!(buffer.set_symbol(9, 0, "x", Style::EMPTY), 0);
        assert_eq!(buffer.set_symbol(0, 9, "x", Style::EMPTY), 0);
        assert_eq!(buffer.set_str(0, 5, "nope", Style::EMPTY, 10), 0);
        assert_eq!(buffer.row_text(0), "    ");
    }

    #[test]
    fn diff_reports_only_changed_cells() {
        let previous = Buffer::new(8, 2);
        let mut next = previous.clone();
        next.set_symbol(3, 1, "x", styled("#62f5b5"));
        let patches = next.diff(&previous);
        assert_eq!(patches.len(), 1);
        assert_eq!((patches[0].x, patches[0].y), (3, 1));
    }

    #[test]
    fn diff_catches_a_style_only_change() {
        let mut previous = Buffer::new(4, 1);
        previous.set_str(0, 0, "abcd", styled("#ffffff"), 4);
        let mut next = previous.clone();
        next.set_str(0, 0, "abcd", styled("#62f5b5"), 4);
        assert_eq!(next.diff(&previous).len(), 4, "recoloring is a change even if text is equal");
    }

    #[test]
    fn diff_of_identical_buffers_is_empty() {
        let mut buffer = Buffer::new(12, 3);
        buffer.set_str(0, 0, "LAYA", styled("#68868c"), 12);
        assert!(buffer.diff(&buffer.clone()).is_empty());
    }

    #[test]
    fn diff_skips_continuation_cells() {
        let previous = Buffer::new(6, 1);
        let mut next = previous.clone();
        next.set_symbol(0, 0, "🦀", Style::EMPTY);
        let patches = next.diff(&previous);
        // Only the lead cell is emitted; printing it moves the cursor across both columns.
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].x, 0);
    }

    #[test]
    fn a_size_change_forces_a_full_repaint() {
        let previous = Buffer::new(4, 1);
        let next = Buffer::new(6, 1);
        assert_eq!(next.diff(&previous).len(), next.area().area() as usize);
    }

    #[test]
    fn resize_drops_contents_and_reallocates() {
        let mut buffer = Buffer::new(4, 1);
        buffer.set_str(0, 0, "abcd", Style::EMPTY, 4);
        buffer.resize(2, 2);
        assert_eq!(buffer.area(), Rect::sized(2, 2));
        assert_eq!(buffer.row_text(0), "  ");
    }

    #[test]
    fn style_area_patches_without_disturbing_symbols() {
        let mut buffer = Buffer::new(6, 1);
        buffer.set_str(0, 0, "abcdef", Style::EMPTY, 6);
        buffer.style_area(Rect::new(1, 0, 2, 1), styled("#ff7c8c"));
        assert_eq!(buffer.row_text(0), "abcdef");
        assert_eq!(buffer.get(1, 0).unwrap().style.fg, Some(Color::hex("#ff7c8c")));
        assert_eq!(buffer.get(3, 0).unwrap().style.fg, None);
    }

    #[test]
    fn set_str_stops_at_a_newline() {
        let mut buffer = Buffer::new(12, 1);
        assert_eq!(buffer.set_str(0, 0, "one\ntwo", Style::EMPTY, 12), 3);
        assert_eq!(buffer.row_text(0).trim_end(), "one");
    }
}
