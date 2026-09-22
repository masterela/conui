//! Integer rectangles in cell space. Every coordinate is a terminal cell, never a pixel.

/// A position in cell space, measured from the top-left of the screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Pos {
    /// Columns from the left edge.
    pub x: u16,
    /// Rows from the top edge.
    pub y: u16,
}

impl Pos {
    /// A position at column `x`, row `y`.
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }
}

/// Uniform or per-side inset applied inside a [`Rect`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Padding {
    /// Rows taken off the top.
    pub top: u16,
    /// Columns taken off the right.
    pub right: u16,
    /// Rows taken off the bottom.
    pub bottom: u16,
    /// Columns taken off the left.
    pub left: u16,
}

impl Padding {
    /// No inset on any side.
    pub const ZERO: Self = Self::all(0);

    /// The same inset on all four sides.
    pub const fn all(n: u16) -> Self {
        Self { top: n, right: n, bottom: n, left: n }
    }

    /// Horizontal then vertical, matching the CSS two-value shorthand.
    pub const fn xy(x: u16, y: u16) -> Self {
        Self { top: y, right: x, bottom: y, left: x }
    }

    /// `n` columns on the left and right, nothing above or below.
    pub const fn horizontal(n: u16) -> Self {
        Self::xy(n, 0)
    }

    /// `n` rows above and below, nothing either side.
    pub const fn vertical(n: u16) -> Self {
        Self::xy(0, n)
    }
}

/// A half-open rectangle of cells: `x .. x + width` by `y .. y + height`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    /// Leftmost column.
    pub x: u16,
    /// Topmost row.
    pub y: u16,
    /// Columns across.
    pub width: u16,
    /// Rows down.
    pub height: u16,
}

impl Rect {
    /// An empty rect at the origin. What a clip collapses to when nothing is visible.
    pub const ZERO: Self = Self { x: 0, y: 0, width: 0, height: 0 };

    /// A rect of `width` by `height` cells with its top-left corner at `x`, `y`.
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self { x, y, width, height }
    }

    /// A rectangle anchored at the origin, as used for a whole screen.
    pub const fn sized(width: u16, height: u16) -> Self {
        Self { x: 0, y: 0, width, height }
    }

    /// Cells covered. Widened to `u32` because a full screen of `u16` dimensions overflows one.
    pub const fn area(&self) -> u32 {
        self.width as u32 * self.height as u32
    }

    /// True when it covers no cells, which is what a rect with either dimension zero means.
    pub const fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// One past the last column. Saturating, so a rect can never wrap the u16 range.
    pub const fn right(&self) -> u16 {
        self.x.saturating_add(self.width)
    }

    /// One past the last row.
    pub const fn bottom(&self) -> u16 {
        self.y.saturating_add(self.height)
    }

    /// True when `pos` is one of this rect's cells. Half-open, so `right()` is outside it.
    pub const fn contains(&self, pos: Pos) -> bool {
        pos.x >= self.x && pos.x < self.right() && pos.y >= self.y && pos.y < self.bottom()
    }

    /// Shrink by `padding`, collapsing to zero rather than underflowing.
    ///
    /// When the padding exceeds the size, the result is an empty rect whose origin is still
    /// clamped inside `self`. A collapsed area that sits outside its own parent would put
    /// later `intersection` checks into a state no clipping could recover from.
    pub fn inset(self, padding: Padding) -> Self {
        let horizontal = padding.left.saturating_add(padding.right);
        let vertical = padding.top.saturating_add(padding.bottom);
        Self {
            x: self.x.saturating_add(padding.left).min(self.right()),
            y: self.y.saturating_add(padding.top).min(self.bottom()),
            width: self.width.saturating_sub(horizontal),
            height: self.height.saturating_sub(vertical),
        }
    }

    /// Shrink by one cell on every side, the common "inside this border" case.
    pub fn shrink(self, n: u16) -> Self {
        self.inset(Padding::all(n))
    }

    /// The overlapping region, or an empty rect when they do not touch.
    pub fn intersection(self, other: Self) -> Self {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= x || bottom <= y {
            return Self::ZERO;
        }
        Self { x, y, width: right - x, height: bottom - y }
    }

    /// The smallest rect covering both.
    pub fn union(self, other: Self) -> Self {
        if self.is_empty() {
            return other;
        }
        if other.is_empty() {
            return self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Self { x, y, width: right - x, height: bottom - y }
    }

    /// Center a `width` x `height` box inside `self`, clamped to fit.
    pub fn centered(self, width: u16, height: u16) -> Self {
        let width = width.min(self.width);
        let height = height.min(self.height);
        Self {
            x: self.x + (self.width - width) / 2,
            y: self.y + (self.height - height) / 2,
            width,
            height,
        }
    }

    /// Rows of the rect, top to bottom, each one cell tall.
    pub fn rows(self) -> impl Iterator<Item = Self> {
        (self.y..self.bottom()).map(move |y| Self::new(self.x, y, self.width, 1))
    }

    /// Columns of the rect, left to right, each one cell wide.
    pub fn columns(self) -> impl Iterator<Item = Self> {
        (self.x..self.right()).map(move |x| Self::new(x, self.y, 1, self.height))
    }

    /// Every cell position in the rect, in row-major order.
    pub fn positions(self) -> impl Iterator<Item = Pos> {
        (self.y..self.bottom())
            .flat_map(move |y| (self.x..self.right()).map(move |x| Pos::new(x, y)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inset_collapses_instead_of_underflowing() {
        let tiny = Rect::sized(3, 2);
        assert_eq!(tiny.inset(Padding::all(4)), Rect::new(3, 2, 0, 0));
    }

    #[test]
    fn intersection_of_disjoint_rects_is_empty() {
        let a = Rect::new(0, 0, 4, 4);
        let b = Rect::new(9, 9, 4, 4);
        assert!(a.intersection(b).is_empty());
        assert_eq!(a.intersection(Rect::new(2, 2, 10, 10)), Rect::new(2, 2, 2, 2));
    }

    #[test]
    fn right_and_bottom_saturate() {
        let huge = Rect::new(u16::MAX - 1, u16::MAX - 1, 8, 8);
        assert_eq!(huge.right(), u16::MAX);
        assert_eq!(huge.bottom(), u16::MAX);
    }

    #[test]
    fn centered_clamps_to_container() {
        assert_eq!(Rect::sized(10, 10).centered(4, 2), Rect::new(3, 4, 4, 2));
        assert_eq!(Rect::sized(4, 4).centered(80, 80), Rect::new(0, 0, 4, 4));
    }

    #[test]
    fn positions_walks_row_major() {
        let seen: Vec<_> = Rect::new(1, 1, 2, 2).positions().collect();
        assert_eq!(seen, vec![Pos::new(1, 1), Pos::new(2, 1), Pos::new(1, 2), Pos::new(2, 2)]);
    }
}
