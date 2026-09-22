//! The declarative layer: composable views and the containers that arrange them.
//!
//! A [`View`] knows how to draw itself into whatever region it is given, and how much of its
//! parent's axis it would like. That is the entire contract — two methods, no lifecycle, no
//! internal state, no diffing. Views are values you build and throw away every frame, which is
//! why there is nothing to keep in sync and nothing to invalidate.
//!
//! ```
//! use conui::view::{Row, View, ViewExt};
//! use conui::widget::{Gauge, Text};
//! use conui::{Frame, Role, Theme};
//! use conui_cell::{Buffer, Rect};
//!
//! let screen = Row::new()
//!     .gap(1)
//!     .child(Text::new("LOAD").role(Role::Muted).length(6))
//!     .child(Gauge::new(0.75).flex(1));
//!
//! let mut buffer = Buffer::new(24, 1);
//! Frame::new(&mut buffer, Theme::LAYA).render_full(&screen);
//! assert!(buffer.row_text(0).starts_with("LOAD"));
//! ```

use conui_cell::{Padding, Rect};

use crate::canvas::Canvas;
use crate::layout::{Constraint, Direction, Layout};
use crate::state::{Hits, Viewport};
use crate::widget::Scrollbar;

/// Something that can draw itself into a region.
pub trait View {
    /// Draw into `canvas`, whose local origin is this view's top-left corner and whose extent
    /// is the region the parent allocated. Anything outside it is clipped, so a view cannot
    /// corrupt its siblings even if its arithmetic is wrong.
    fn render(&self, canvas: &mut Canvas<'_>);

    /// How much of `axis` this view wants, `axis` being the one its parent is dividing.
    ///
    /// Asked per axis because a view's two intrinsic sizes are different questions: a label is one
    /// row tall and as wide as its text, and answering with one number means being wrong in a
    /// `Row` or wrong in a `Column`. A view with no intrinsic size along an axis says
    /// `Fill(1)` — an equal share of whatever is spare — which is also the default for both.
    /// [`ViewExt`] overrides the answer per use, for whichever axis the parent asks about.
    fn constraint(&self, axis: Direction) -> Constraint {
        let _ = axis;
        Constraint::Fill(1)
    }
}

impl<V: View + ?Sized> View for &V {
    fn render(&self, canvas: &mut Canvas<'_>) {
        (**self).render(canvas);
    }
    fn constraint(&self, axis: Direction) -> Constraint {
        (**self).constraint(axis)
    }
}

impl<V: View + ?Sized> View for Box<V> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        (**self).render(canvas);
    }
    fn constraint(&self, axis: Direction) -> Constraint {
        (**self).constraint(axis)
    }
}

/// Builder methods available on every view.
///
/// These return wrappers rather than mutating, so a view's own type never has to carry a
/// constraint field or a padding field that most of its users leave alone.
pub trait ViewExt: View + Sized {
    /// Take a share of the leftover space, weighted. `flex(2)` beside `flex(1)` takes twice as
    /// much.
    fn flex(self, weight: u16) -> Constrained<Self> {
        self.constrain(Constraint::Fill(weight))
    }

    /// Occupy exactly this many cells along the parent's axis.
    fn length(self, cells: u16) -> Constrained<Self> {
        self.constrain(Constraint::Length(cells))
    }

    /// Occupy this share of the parent's axis, 0-100.
    fn percent(self, percent: u16) -> Constrained<Self> {
        self.constrain(Constraint::Percentage(percent))
    }

    /// Occupy this fraction of the parent's axis.
    fn ratio(self, numerator: u32, denominator: u32) -> Constrained<Self> {
        self.constrain(Constraint::Ratio(numerator, denominator))
    }

    /// Occupy at least this many cells, and more if any is spare.
    fn at_least(self, cells: u16) -> Constrained<Self> {
        self.constrain(Constraint::Min(cells))
    }

    /// Occupy no more than this many cells.
    fn at_most(self, cells: u16) -> Constrained<Self> {
        self.constrain(Constraint::Max(cells))
    }

    fn constrain(self, constraint: Constraint) -> Constrained<Self> {
        Constrained { view: self, constraint }
    }

    /// Inset this view's own drawing region, without affecting what the parent allocated.
    fn padded(self, padding: Padding) -> Padded<Self> {
        Padded { view: self, padding }
    }

    /// Note where this view lands, under `id`, so a click can be resolved back to it.
    ///
    /// The widget learns nothing: hit-testing is a decorator, which is why `Button` has no
    /// `on_click` and no id field. See [`Hits`] for what to do with the result.
    fn hit<T: Copy>(self, hits: &Hits<T>, id: T) -> Hit<'_, T, Self> {
        Hit { view: self, hits, id }
    }
}

impl<V: View> ViewExt for V {}

/// A view that records its region before drawing. See [`ViewExt::hit`].
pub struct Hit<'a, T, V> {
    view: V,
    hits: &'a Hits<T>,
    id: T,
}

impl<T: Copy, V: View> View for Hit<'_, T, V> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        self.hits.record(self.id, canvas.screen_area());
        self.view.render(canvas);
    }
    fn constraint(&self, axis: Direction) -> Constraint {
        self.view.constraint(axis)
    }
}

/// A view with its parent-axis size overridden. See [`ViewExt::flex`].
pub struct Constrained<V> {
    view: V,
    constraint: Constraint,
}

impl<V: View> View for Constrained<V> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        self.view.render(canvas);
    }

    /// The same answer whichever axis is asked about: `.length(6)` in a `Row` means six columns and
    /// in a `Column` means six rows. A constraint written at the point of use is about the slot the
    /// parent is filling, so it speaks for whichever axis that parent divides.
    fn constraint(&self, _axis: Direction) -> Constraint {
        self.constraint
    }
}

/// A view drawn inside an inset of its region. See [`ViewExt::padded`].
pub struct Padded<V> {
    view: V,
    padding: Padding,
}

impl<V: View> View for Padded<V> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let mut inner = canvas.inset(self.padding);
        self.view.render(&mut inner);
    }
    fn constraint(&self, axis: Direction) -> Constraint {
        self.view.constraint(axis)
    }
}

/// Views arranged along one axis.
///
/// [`Row`] and [`Column`] are the names you will normally use; this is what they are.
pub struct Stack<'a> {
    direction: Direction,
    children: Vec<Box<dyn View + 'a>>,
    gap: u16,
    padding: Padding,
    fit: bool,
}

impl<'a> Stack<'a> {
    pub fn new(direction: Direction) -> Self {
        Self { direction, children: Vec::new(), gap: 0, padding: Padding::ZERO, fit: false }
    }

    pub fn child(mut self, view: impl View + 'a) -> Self {
        self.children.push(Box::new(view));
        self
    }

    /// Add several children at once, for building a stack from data.
    pub fn children<V: View + 'a>(mut self, views: impl IntoIterator<Item = V>) -> Self {
        for view in views {
            self.children.push(Box::new(view));
        }
        self
    }

    /// Blank cells between neighbouring children.
    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    /// Space inside the stack's own region, before its children are laid out.
    pub fn padding(mut self, padding: Padding) -> Self {
        self.padding = padding;
        self
    }

    /// Ask for exactly the space the children need, instead of a share of the parent.
    ///
    /// A stack normally asks for `Fill(1)`, because it has no idea what it is inside: a column of
    /// three rows sitting beside a panel should take its half of the screen, not three rows of it.
    /// This says the opposite — the size is the content's — which is what makes a stack scrollable,
    /// since [`Scroll`] has nothing to scroll unless its child has an intrinsic size.
    ///
    /// Only meaningful if every child has one: if any asks for `Fill` or a percentage, its size is
    /// a share of something this stack does not know, so the stack goes back to asking for `Fill`.
    pub fn fit(mut self) -> Self {
        self.fit = true;
        self
    }

    pub fn len(&self) -> usize {
        self.children.len()
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }

    /// The padding along this stack's own axis, which is the part that adds to its size.
    fn axis_padding(&self) -> u16 {
        match self.direction {
            Direction::Vertical => self.padding.top.saturating_add(self.padding.bottom),
            Direction::Horizontal => self.padding.left.saturating_add(self.padding.right),
        }
    }

    /// The padding across it, which adds to the size a parent stacking the other way sees.
    fn cross_padding(&self) -> u16 {
        match self.direction {
            Direction::Vertical => self.padding.left.saturating_add(self.padding.right),
            Direction::Horizontal => self.padding.top.saturating_add(self.padding.bottom),
        }
    }
}

impl View for Stack<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        if self.children.is_empty() {
            return;
        }
        let constraints: Vec<Constraint> =
            self.children.iter().map(|child| child.constraint(self.direction)).collect();
        let layout = Layout::new(self.direction, constraints).gap(self.gap).margin(self.padding);
        for (child, area) in self.children.iter().zip(layout.split(canvas.area())) {
            // An empty region still gets rendered into: the canvas clips it away, and skipping
            // it would mean a widget could not rely on being called every frame.
            let mut region = canvas.sub(area);
            child.render(&mut region);
        }
    }

    /// Along its own axis a fitted stack adds its children up; across it, they overlap, so the
    /// widest child is the answer. Either way one unmeasurable child makes the whole stack
    /// unmeasurable: a share of the parent is not a size.
    fn constraint(&self, axis: Direction) -> Constraint {
        if !self.fit {
            return Constraint::Fill(1);
        }
        let mut total: u16 = 0;
        for child in &self.children {
            match child.constraint(axis) {
                Constraint::Length(cells) if axis == self.direction => {
                    total = total.saturating_add(cells);
                }
                Constraint::Length(cells) => total = total.max(cells),
                // A child whose size is a share of the parent cannot be added up here.
                _ => return Constraint::Fill(1),
            }
        }
        if axis != self.direction {
            return Constraint::Length(total.saturating_add(self.cross_padding()));
        }
        let gaps = self.children.len().saturating_sub(1) as u16 * self.gap;
        Constraint::Length(total.saturating_add(gaps).saturating_add(self.axis_padding()))
    }
}

/// Views side by side.
pub struct Row<'a>(Stack<'a>);

impl<'a> Row<'a> {
    pub fn new() -> Self {
        Self(Stack::new(Direction::Horizontal))
    }

    pub fn child(mut self, view: impl View + 'a) -> Self {
        self.0 = self.0.child(view);
        self
    }

    pub fn children<V: View + 'a>(mut self, views: impl IntoIterator<Item = V>) -> Self {
        self.0 = self.0.children(views);
        self
    }

    pub fn gap(mut self, gap: u16) -> Self {
        self.0 = self.0.gap(gap);
        self
    }

    pub fn padding(mut self, padding: Padding) -> Self {
        self.0 = self.0.padding(padding);
        self
    }

    /// Ask for exactly the space the children need. See [`Stack::fit`].
    pub fn fit(mut self) -> Self {
        self.0 = self.0.fit();
        self
    }
}

impl Default for Row<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl View for Row<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        self.0.render(canvas);
    }
    fn constraint(&self, axis: Direction) -> Constraint {
        self.0.constraint(axis)
    }
}

/// Views stacked top to bottom.
pub struct Column<'a>(Stack<'a>);

impl<'a> Column<'a> {
    pub fn new() -> Self {
        Self(Stack::new(Direction::Vertical))
    }

    pub fn child(mut self, view: impl View + 'a) -> Self {
        self.0 = self.0.child(view);
        self
    }

    pub fn children<V: View + 'a>(mut self, views: impl IntoIterator<Item = V>) -> Self {
        self.0 = self.0.children(views);
        self
    }

    pub fn gap(mut self, gap: u16) -> Self {
        self.0 = self.0.gap(gap);
        self
    }

    pub fn padding(mut self, padding: Padding) -> Self {
        self.0 = self.0.padding(padding);
        self
    }

    /// Ask for exactly the space the children need. See [`Stack::fit`].
    pub fn fit(mut self) -> Self {
        self.0 = self.0.fit();
        self
    }
}

impl Default for Column<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl View for Column<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        self.0.render(canvas);
    }
    fn constraint(&self, axis: Direction) -> Constraint {
        self.0.constraint(axis)
    }
}

/// A window onto a child taller than itself.
///
/// The general case of scrolling, for content with no selection to follow: a pane of prose, a log,
/// a form longer than its panel. A [`List`](crate::widget::List) does its own scrolling because it
/// has a cursor; this is what you use when there is nothing to put a cursor on.
///
/// The child is drawn *whole*, at its own natural height, starting above the visible region — see
/// [`Canvas::shifted`] — so it does not know it is being scrolled and needs no cooperation. It does
/// need an intrinsic height, because a view that asks for `Fill` is by definition happy with
/// whatever it is given and so has nothing to scroll. Either `.fit()` a stack, or put a
/// [`ViewExt::length`] on the child and say how tall it is.
///
/// ```
/// use conui::view::{Column, Scroll, ViewExt};
/// use conui::widget::Text;
/// use conui::{Frame, Theme, Viewport};
/// use conui_cell::Buffer;
///
/// let viewport = Viewport::new();
/// let content = Column::new().children((0..8).map(|n| Text::new(format!("row {n}")).length(1)));
/// // Four rows of a column of eight, scrolled down by two.
/// viewport.scroll(2);
/// let mut buffer = Buffer::new(8, 4);
/// Frame::new(&mut buffer, Theme::LAYA).render_full(&Scroll::new(&viewport, content.fit()));
/// assert!(buffer.row_text(0).starts_with("row 2"));
/// assert_eq!(viewport.overflow(), 4, "four rows out of sight");
/// // The last column is the scrollbar, and its thumb has moved off the top row.
/// assert_eq!(buffer.row_text(0).chars().last(), Some('│'));
/// assert_eq!(buffer.row_text(1).chars().last(), Some('█'));
/// ```
pub struct Scroll<'a, V> {
    view: V,
    viewport: &'a Viewport,
    bar: bool,
}

impl<'a, V: View> Scroll<'a, V> {
    pub fn new(viewport: &'a Viewport, view: V) -> Self {
        Self { view, viewport, bar: true }
    }

    /// Draw without the scrollbar, giving the column back to the content.
    ///
    /// The bar is on by default because scrolled content with no indication of position is the most
    /// common way a terminal UI loses someone: there is no window chrome to tell them there is more.
    /// Turn it off when something else already says so — a footer reading `12/40`, say.
    pub fn bare(mut self) -> Self {
        self.bar = false;
        self
    }
}

impl<V: View> View for Scroll<'_, V> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let (width, height) = (canvas.width(), canvas.height());
        // A view that asks for a share of its parent has no height of its own to scroll past.
        // The vertical axis by name: this scrolls rows, so the child's *height* is the question,
        // whatever axis the stack above happened to be dividing.
        let content = match self.view.constraint(Direction::Vertical) {
            Constraint::Length(rows) => rows.max(height),
            _ => height,
        };
        let offset = self.viewport.window(height, content);
        // The bar costs a column, so reserve it before the content is laid out rather than drawing
        // over the text afterwards: a scrollbar that eats the last character of every long line is
        // worse than no scrollbar.
        let show_bar = self.bar && content > height && width > 1;
        let body = width - u16::from(show_bar);

        {
            let mut region = canvas.sub(Rect::sized(body, height));
            let mut whole = region.shifted(0, -i32::from(offset), body, content);
            self.view.render(&mut whole);
        }
        if show_bar {
            let mut track = canvas.sub(Rect::new(width - 1, 0, 1, height));
            Scrollbar::new(usize::from(offset), usize::from(content)).render(&mut track);
        }
    }
}

/// Empty space.
///
/// Reads better than an invisible padding value when the gap is structural: a `Spacer::flex(1)`
/// between two children says "push these apart" at the place where it happens.
pub struct Spacer;

impl Spacer {
    pub const fn new() -> Self {
        Self
    }
}

impl Default for Spacer {
    fn default() -> Self {
        Self
    }
}

impl View for Spacer {
    fn render(&self, _canvas: &mut Canvas<'_>) {}
}

/// A hand-painted view, wrapping a closure.
///
/// The bridge between the layers: drop one of these into a view tree wherever the declarative
/// widgets run out, and it gets a laid-out, clipped region like any other child.
///
/// ```
/// use conui::view::{Column, Paint, ViewExt};
/// use conui::widget::Text;
/// # use conui::{Frame, Theme};
/// # use conui_cell::Buffer;
///
/// let screen = Column::new()
///     .child(Text::new("BOARD").length(1))
///     .child(Paint::new(|c| c.run(0, 0, '█', c.width(), conui::Role::Accent)).flex(1));
/// # let mut buffer = Buffer::new(6, 2);
/// # Frame::new(&mut buffer, Theme::LAYA).render_full(&screen);
/// # assert_eq!(buffer.row_text(1), "██████");
/// ```
pub struct Paint<F> {
    body: F,
}

impl<F: Fn(&mut Canvas<'_>)> Paint<F> {
    pub const fn new(body: F) -> Self {
        Self { body }
    }
}

impl<F: Fn(&mut Canvas<'_>)> View for Paint<F> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        (self.body)(canvas);
    }
}

/// Draw a view only when a condition holds.
///
/// Cleaner than building a tree conditionally, because the surrounding layout keeps the same
/// shape: a hidden child still occupies its slot, so nothing jumps when it appears.
pub struct When<V> {
    condition: bool,
    view: V,
}

impl<V: View> When<V> {
    pub const fn new(condition: bool, view: V) -> Self {
        Self { condition, view }
    }
}

impl<V: View> View for When<V> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        if self.condition {
            self.view.render(canvas);
        }
    }
    fn constraint(&self, axis: Direction) -> Constraint {
        self.view.constraint(axis)
    }
}

/// A view that fills its region with one character. Mostly useful for seeing a layout.
pub struct Fill {
    character: char,
    role: crate::Role,
}

impl Fill {
    pub const fn new(character: char, role: crate::Role) -> Self {
        Self { character, role }
    }
}

impl View for Fill {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let area = canvas.area();
        canvas.fill(Rect::sized(area.width, area.height), self.character, self.role);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Role;
    use crate::state::Viewport;
    use crate::widget::Text;
    use conui_cell::Buffer;

    /// Render a view into a fresh buffer and return its rows.
    fn rows(view: &dyn View, width: u16, height: u16) -> Vec<String> {
        let mut buffer = Buffer::new(width, height);
        let mut canvas = Canvas::full(&mut buffer, crate::Theme::LAYA);
        view.render(&mut canvas);
        (0..height).map(|row| buffer.row_text(row)).collect()
    }

    #[test]
    fn a_row_divides_its_width_among_children() {
        let view = Row::new().child(Fill::new('a', Role::Text)).child(Fill::new('b', Role::Text));
        assert_eq!(rows(&view, 6, 1), ["aaabbb"]);
    }

    #[test]
    fn flex_weights_split_the_row_proportionally() {
        let view = Row::new()
            .child(Fill::new('a', Role::Text).flex(2))
            .child(Fill::new('b', Role::Text).flex(1));
        assert_eq!(rows(&view, 6, 1), ["aaaabb"]);
    }

    #[test]
    fn a_fixed_length_child_keeps_its_width_as_the_row_grows() {
        let view = Row::new()
            .child(Fill::new('a', Role::Text).length(2))
            .child(Fill::new('b', Role::Text).flex(1));
        assert_eq!(rows(&view, 6, 1), ["aabbbb"]);
        assert_eq!(rows(&view, 10, 1), ["aabbbbbbbb"]);
    }

    #[test]
    fn a_column_stacks_downwards() {
        let view =
            Column::new().child(Fill::new('a', Role::Text)).child(Fill::new('b', Role::Text));
        assert_eq!(rows(&view, 2, 4), ["aa", "aa", "bb", "bb"]);
    }

    #[test]
    fn a_gap_separates_children_without_being_drawn_into() {
        let view =
            Row::new().gap(2).child(Fill::new('a', Role::Text)).child(Fill::new('b', Role::Text));
        assert_eq!(rows(&view, 6, 1), ["aa  bb"]);
    }

    #[test]
    fn padding_insets_the_whole_stack() {
        let view = Column::new().padding(Padding::all(1)).child(Fill::new('x', Role::Text));
        assert_eq!(rows(&view, 4, 3), ["    ", " xx ", "    "]);
    }

    #[test]
    fn a_padded_child_insets_only_itself() {
        let view = Row::new()
            .child(Fill::new('a', Role::Text).padded(Padding::horizontal(1)))
            .child(Fill::new('b', Role::Text));
        assert_eq!(rows(&view, 8, 1), [" aa bbbb"]);
    }

    #[test]
    fn a_spacer_pushes_its_siblings_apart() {
        let view = Row::new()
            .child(Fill::new('a', Role::Text).length(1))
            .child(Spacer::new().flex(1))
            .child(Fill::new('b', Role::Text).length(1));
        assert_eq!(rows(&view, 5, 1), ["a   b"]);
    }

    #[test]
    fn a_child_cannot_draw_outside_the_region_it_was_given() {
        // The containment guarantee that makes it safe to compose views written by other people.
        let view = Row::new()
            .child(Paint::new(|canvas| {
                canvas.text(-10, 0, "XXXXXXXXXXXXXXXXXXXX");
                canvas.text(0, -4, "up");
            }))
            .child(Fill::new('.', Role::Text));
        assert_eq!(rows(&view, 6, 1), ["XXX..."]);
    }

    #[test]
    fn a_hidden_child_keeps_its_slot_so_the_layout_does_not_jump() {
        let shown = Row::new()
            .child(When::new(true, Fill::new('a', Role::Text)))
            .child(Fill::new('b', Role::Text));
        let hidden = Row::new()
            .child(When::new(false, Fill::new('a', Role::Text)))
            .child(Fill::new('b', Role::Text));
        assert_eq!(rows(&shown, 4, 1), ["aabb"]);
        assert_eq!(rows(&hidden, 4, 1), ["  bb"]);
    }

    #[test]
    fn nesting_rows_in_columns_composes_without_surprises() {
        let view = Column::new().child(Fill::new('t', Role::Text).length(1)).child(
            Row::new().child(Fill::new('l', Role::Text)).child(Fill::new('r', Role::Text)).flex(1),
        );
        assert_eq!(rows(&view, 4, 3), ["tttt", "llrr", "llrr"]);
    }

    #[test]
    fn a_stack_with_no_children_draws_nothing() {
        assert_eq!(rows(&Row::new(), 4, 1), ["    "]);
    }

    #[test]
    fn a_stack_in_a_region_too_small_for_it_does_not_panic() {
        let view = Row::new()
            .gap(3)
            .child(Fill::new('a', Role::Text).length(10))
            .child(Fill::new('b', Role::Text).length(10));
        // Nothing to assert about the result beyond "it produced one"; the point is survival.
        for width in 0..12u16 {
            let _ = rows(&view, width, 1);
        }
    }

    #[test]
    fn views_built_from_data_render_in_order() {
        let labels = ['a', 'b', 'c'];
        let view =
            Row::new().children(labels.into_iter().map(|label| Fill::new(label, Role::Text)));
        assert_eq!(rows(&view, 6, 1), ["aabbcc"]);
    }

    #[test]
    fn a_boxed_view_behaves_like_the_view_it_wraps() {
        let boxed: Box<dyn View> = Box::new(Fill::new('z', Role::Text).length(2));
        assert_eq!(boxed.constraint(Direction::Horizontal), Constraint::Length(2));
        let view = Row::new().child(boxed).child(Fill::new('.', Role::Text));
        assert_eq!(rows(&view, 5, 1), ["zz..."]);
    }

    #[test]
    fn a_fitted_stack_asks_for_the_space_its_children_need() {
        let column = Column::new()
            .gap(1)
            .padding(Padding::vertical(2))
            .child(Fill::new('a', Role::Text).length(3))
            .child(Fill::new('b', Role::Text).length(4))
            .fit();
        // Three and four rows, one row of gap, two rows of padding at each end.
        assert_eq!(column.constraint(Direction::Vertical), Constraint::Length(12));
    }

    #[test]
    fn a_stack_asks_for_a_share_of_its_parent_unless_told_to_fit() {
        let children = || Column::new().child(Fill::new('a', Role::Text).length(3));
        assert_eq!(children().constraint(Direction::Vertical), Constraint::Fill(1));
        assert_eq!(children().fit().constraint(Direction::Vertical), Constraint::Length(3));
    }

    #[test]
    fn a_fitted_stack_holding_an_elastic_child_cannot_add_itself_up() {
        // `Fill` means "a share of my parent", and this stack does not know what its parent is.
        let column = Column::new()
            .child(Fill::new('a', Role::Text).length(3))
            .child(Fill::new('b', Role::Text))
            .fit();
        assert_eq!(column.constraint(Direction::Vertical), Constraint::Fill(1));
    }

    #[test]
    fn a_fitted_row_measures_across_not_down() {
        let row = Row::new()
            .gap(2)
            .padding(Padding::horizontal(1))
            .child(Fill::new('a', Role::Text).length(4))
            .child(Fill::new('b', Role::Text).length(4))
            .fit();
        assert_eq!(row.constraint(Direction::Horizontal), Constraint::Length(12));
    }

    // ---- Per-axis constraints -----------------------------------------------------------

    #[test]
    fn the_same_view_answers_a_row_and_a_column_differently() {
        // A label is one row tall and five columns wide, and neither number is an answer to
        // the other question. This is the whole reason `constraint` takes an axis.
        let label = Text::new("hello");
        assert_eq!(label.constraint(Direction::Vertical), Constraint::Length(1));
        assert_eq!(label.constraint(Direction::Horizontal), Constraint::Length(5));
    }

    #[test]
    fn a_row_gives_a_label_the_width_of_its_text_unasked() {
        // No `.length(5)` anywhere: the `Row` asked, and the label knew.
        let view = Row::new()
            .child(Text::new("hello"))
            .child(Fill::new('.', Role::Text))
            .child(Text::new("bye"));
        assert_eq!(rows(&view, 12, 1), ["hello....bye"]);
    }

    #[test]
    fn a_column_gives_the_same_label_one_row_and_the_full_width() {
        let view = Column::new().child(Text::new("hello")).child(Fill::new('.', Role::Text));
        assert_eq!(rows(&view, 5, 3), ["hello", ".....", "....."]);
    }

    #[test]
    fn an_override_answers_whichever_axis_its_parent_asks_about() {
        // `.length(2)` is two columns in a `Row` and two rows in a `Column`; the view it wraps
        // is not consulted on either axis.
        let fixed = Text::new("hello").length(2);
        assert_eq!(fixed.constraint(Direction::Horizontal), Constraint::Length(2));
        assert_eq!(fixed.constraint(Direction::Vertical), Constraint::Length(2));
    }

    #[test]
    fn a_fitted_stack_is_as_wide_as_its_widest_child_is_tall_as_its_tallest() {
        // Along its own direction a stack adds its children up; across it, they overlap, so the
        // widest one speaks for all of them.
        let column = Column::new()
            .padding(Padding::horizontal(1))
            .child(Text::new("hello"))
            .child(Text::new("hi"))
            .fit();
        assert_eq!(column.constraint(Direction::Vertical), Constraint::Length(2));
        assert_eq!(column.constraint(Direction::Horizontal), Constraint::Length(7));
    }

    #[test]
    fn a_fitted_stack_with_one_unmeasurable_child_across_cannot_measure_across() {
        // The same rule as along the axis: `Fill` is a share of a parent this stack cannot see.
        let column =
            Column::new().child(Text::new("hello")).child(Fill::new('.', Role::Text)).fit();
        assert_eq!(column.constraint(Direction::Horizontal), Constraint::Fill(1));
    }

    #[test]
    fn a_row_of_labels_nested_in_a_column_sizes_itself_on_both_axes() {
        let row = Row::new().gap(1).child(Text::new("hello")).child(Text::new("bye")).fit();
        let view = Column::new().child(row).child(Fill::new('.', Role::Text));
        // One row for the labels, nine columns of it used, the rest to the filler.
        assert_eq!(rows(&view, 11, 2), ["hello bye  ", "..........."]);
    }

    // ---- Scroll -------------------------------------------------------------------------

    /// Eight rows of numbered content that knows its own height.
    fn document<'a>() -> Column<'a> {
        Column::new()
            .children((0..8).map(|n| Fill::new(char::from(b'0' + n), Role::Text).length(1)))
    }

    #[test]
    fn a_scroll_shows_a_window_of_taller_content() {
        let viewport = Viewport::new();
        let view = Scroll::new(&viewport, document().fit()).bare();
        assert_eq!(rows(&view, 2, 3), ["00", "11", "22"]);
        viewport.scroll(4);
        assert_eq!(rows(&view, 2, 3), ["44", "55", "66"]);
        assert_eq!(viewport.content(), 8);
        assert_eq!(viewport.height(), 3);
    }

    #[test]
    fn a_scroll_cannot_show_less_than_a_windowful_of_content_that_exists() {
        let viewport = Viewport::at(99);
        let view = Scroll::new(&viewport, document().fit()).bare();
        // Scrolled far past the end, it shows the last three rows rather than nothing.
        assert_eq!(rows(&view, 2, 3), ["55", "66", "77"]);
    }

    #[test]
    fn content_that_fits_is_not_scrolled_and_gets_no_bar() {
        let viewport = Viewport::new();
        let view = Scroll::new(&viewport, document().fit());
        let drawn = rows(&view, 3, 8);
        assert_eq!(drawn[0], "000", "the bar column went back to the content");
        assert!(!viewport.is_scrollable());
    }

    #[test]
    fn a_scroll_over_elastic_content_has_nothing_to_scroll() {
        // A view asking for `Fill` is by definition happy with the region it is given.
        let viewport = Viewport::new();
        let view = Scroll::new(&viewport, Fill::new('x', Role::Text));
        assert_eq!(rows(&view, 2, 2), ["xx", "xx"]);
        viewport.scroll(4);
        assert_eq!(rows(&view, 2, 2), ["xx", "xx"], "and stays put");
    }

    #[test]
    fn a_scroll_reserves_the_bar_column_before_the_content_is_laid_out() {
        let viewport = Viewport::new();
        let content = Column::new().children((0..6).map(|_| Text::new("ab").centered().length(1)));
        let view = Scroll::new(&viewport, content.fit());
        let drawn = rows(&view, 6, 3);
        // Centred within five columns, not six: the bar's column is gone before layout sees it,
        // rather than being painted over a line that had already used the space.
        assert_eq!(drawn[0], " ab  █");
        assert_eq!(drawn[2], " ab  │", "track below the thumb");
    }

    #[test]
    fn scrolled_content_still_cannot_draw_outside_the_viewport() {
        let viewport = Viewport::at(2);
        let content = Column::new().children((0..9).map(|_| Fill::new('c', Role::Text).length(1)));
        let view = Column::new()
            .child(Fill::new('t', Role::Text).length(1))
            .child(Scroll::new(&viewport, content.fit()).bare().length(2))
            .child(Fill::new('b', Role::Text).length(1));
        assert_eq!(rows(&view, 3, 4), ["ttt", "ccc", "ccc", "bbb"]);
    }

    #[test]
    fn hit_records_where_layout_actually_put_the_child() {
        let hits = Hits::new();
        let view = Column::new().padding(Padding::all(1)).child(
            Row::new()
                .gap(2)
                .child(Fill::new('a', Role::Text).length(3))
                .child(Fill::new('b', Role::Text).hit(&hits, 'b').length(4)),
        );
        assert_eq!(rows(&view, 12, 3), ["            ", " aaa  bbbb  ", "            "]);
        // One column of padding, three of the first child, two of gap.
        assert_eq!(hits.area_of('b'), Some(Rect::new(6, 1, 4, 1)));
        assert_eq!(hits.at(conui_cell::Pos::new(7, 1)), Some('b'));
        assert_eq!(hits.at(conui_cell::Pos::new(5, 1)), None, "the gap is nobody's");
    }

    #[test]
    fn hit_does_not_change_what_the_child_asks_for_or_draws() {
        let hits = Hits::new();
        let plain = Fill::new('a', Role::Text).length(2);
        let decorated = Fill::new('a', Role::Text).length(2).hit(&hits, ());
        assert_eq!(
            decorated.constraint(Direction::Vertical),
            plain.constraint(Direction::Vertical)
        );
        assert_eq!(rows(&decorated, 4, 1), rows(&plain, 4, 1));
    }
}
