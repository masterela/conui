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

/// Something that can draw itself into a region.
pub trait View {
    /// Draw into `canvas`, whose local origin is this view's top-left corner and whose extent
    /// is the region the parent allocated. Anything outside it is clipped, so a view cannot
    /// corrupt its siblings even if its arithmetic is wrong.
    fn render(&self, canvas: &mut Canvas<'_>);

    /// How much of the parent's axis this view wants.
    ///
    /// The default asks for an equal share of whatever is spare. Views with an intrinsic size —
    /// a one-line label, a three-row block number — override this so that stacking them does
    /// the obvious thing without the caller restating it. [`ViewExt`] overrides it per use.
    fn constraint(&self) -> Constraint {
        Constraint::Fill(1)
    }
}

impl<V: View + ?Sized> View for &V {
    fn render(&self, canvas: &mut Canvas<'_>) {
        (**self).render(canvas);
    }
    fn constraint(&self) -> Constraint {
        (**self).constraint()
    }
}

impl<V: View + ?Sized> View for Box<V> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        (**self).render(canvas);
    }
    fn constraint(&self) -> Constraint {
        (**self).constraint()
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
}

impl<V: View> ViewExt for V {}

/// A view with its parent-axis size overridden. See [`ViewExt::flex`].
pub struct Constrained<V> {
    view: V,
    constraint: Constraint,
}

impl<V: View> View for Constrained<V> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        self.view.render(canvas);
    }
    fn constraint(&self) -> Constraint {
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
    fn constraint(&self) -> Constraint {
        self.view.constraint()
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
}

impl<'a> Stack<'a> {
    pub fn new(direction: Direction) -> Self {
        Self { direction, children: Vec::new(), gap: 0, padding: Padding::ZERO }
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

    pub fn len(&self) -> usize {
        self.children.len()
    }

    pub fn is_empty(&self) -> bool {
        self.children.is_empty()
    }
}

impl View for Stack<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        if self.children.is_empty() {
            return;
        }
        let constraints: Vec<Constraint> =
            self.children.iter().map(|child| child.constraint()).collect();
        let layout = Layout::new(self.direction, constraints).gap(self.gap).margin(self.padding);
        for (child, area) in self.children.iter().zip(layout.split(canvas.area())) {
            // An empty region still gets rendered into: the canvas clips it away, and skipping
            // it would mean a widget could not rely on being called every frame.
            let mut region = canvas.sub(area);
            child.render(&mut region);
        }
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
    fn constraint(&self) -> Constraint {
        self.view.constraint()
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
        assert_eq!(boxed.constraint(), Constraint::Length(2));
        let view = Row::new().child(boxed).child(Fill::new('.', Role::Text));
        assert_eq!(rows(&view, 5, 1), ["zz..."]);
    }
}
