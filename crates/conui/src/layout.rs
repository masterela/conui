//! Splitting a rectangle into rectangles.
//!
//! This is deliberately not a constraint solver. A solver gives you a system that is hard to
//! predict from reading the code and that can fail to converge at 40 columns; a terminal has
//! integer cells and a handful of sensible ways to divide them, so an explicit resolution
//! order is both simpler and easier to reason about.
//!
//! The order is: rigid sizes are honoured, [`Constraint::Fill`] absorbs what is left over in
//! proportion to its weight, and when there is not enough room the most elastic constraints
//! give way first. Every split is exact — the resulting rectangles tile the area with no gap
//! and no overlap beyond the gap you asked for.
//!
//! ```
//! use conui::layout::{Constraint, Layout};
//! use conui_cell::Rect;
//!
//! let [sidebar, body] = Layout::horizontal([Constraint::Length(20), Constraint::Fill(1)])
//!     .split_array(Rect::sized(80, 24));
//! assert_eq!(sidebar.width, 20);
//! assert_eq!(body.width, 60);
//! ```

use conui_cell::{Padding, Rect};

/// Which axis a layout divides.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    /// Side by side; splits width.
    #[default]
    Horizontal,
    /// Stacked; splits height.
    Vertical,
}

/// How much of the axis one child asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Constraint {
    /// Exactly this many cells. The last thing to be sacrificed when space runs short, which
    /// is what you want for a fixed-width label or a one-cell border.
    Length(u16),
    /// This share of the axis, 0-100.
    Percentage(u16),
    /// This fraction of the axis. Exact where a percentage would round badly: `Ratio(1, 3)`
    /// of 100 columns is 33, and three of them tile 100 without losing a column.
    Ratio(u32, u32),
    /// At least this many cells, and more if any is going spare.
    Min(u16),
    /// At most this many cells, giving space back before anything else does.
    Max(u16),
    /// Take a share of whatever is left, in proportion to this weight. `Fill(2)` next to
    /// `Fill(1)` gets twice as much.
    Fill(u16),
}

impl Constraint {
    /// The size this constraint starts from, before surplus or deficit is applied.
    ///
    /// Proportional constraints are resolved as a group instead — see [`proportional_share`] —
    /// so they contribute nothing here.
    fn base(self, available: u16) -> u16 {
        match self {
            Self::Length(length) | Self::Min(length) => length,
            Self::Max(length) => length.min(available),
            Self::Percentage(_) | Self::Ratio(_, _) | Self::Fill(_) => 0,
        }
    }

    /// How readily this constraint gives up space. Lower yields first.
    fn rigidity(self) -> u8 {
        match self {
            Self::Fill(_) => 0,
            Self::Max(_) => 1,
            Self::Min(_) => 2,
            Self::Percentage(_) | Self::Ratio(_, _) => 3,
            Self::Length(_) => 4,
        }
    }

    /// The weight used when handing out surplus, or 0 if this constraint does not want any.
    fn growth_weight(self) -> u32 {
        match self {
            Self::Fill(weight) => u32::from(weight.max(1)),
            _ => 0,
        }
    }
}

/// A division of one rectangle along one axis.
#[derive(Clone, Debug, Default)]
pub struct Layout {
    direction: Direction,
    constraints: Vec<Constraint>,
    gap: u16,
    margin: Padding,
}

impl Layout {
    /// A split of `constraints` along `direction`, with no gap and no margin.
    pub fn new(direction: Direction, constraints: impl IntoIterator<Item = Constraint>) -> Self {
        Self {
            direction,
            constraints: constraints.into_iter().collect(),
            gap: 0,
            margin: Padding::ZERO,
        }
    }

    /// A left-to-right split, one constraint per column band.
    pub fn horizontal(constraints: impl IntoIterator<Item = Constraint>) -> Self {
        Self::new(Direction::Horizontal, constraints)
    }

    /// A top-to-bottom split, one constraint per row band.
    pub fn vertical(constraints: impl IntoIterator<Item = Constraint>) -> Self {
        Self::new(Direction::Vertical, constraints)
    }

    /// Blank cells between neighbouring children. Comes out of the space available to them,
    /// so the result still tiles the area exactly.
    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    /// Space removed from the edges of the area before splitting.
    pub fn margin(mut self, margin: Padding) -> Self {
        self.margin = margin;
        self
    }

    /// The axis this splits along.
    pub fn direction(&self) -> Direction {
        self.direction
    }

    /// What each child asked for, in order.
    pub fn constraints(&self) -> &[Constraint] {
        &self.constraints
    }

    /// Divide `area`, returning one rectangle per constraint, in order.
    pub fn split(&self, area: Rect) -> Vec<Rect> {
        let inner = area.inset(self.margin);
        let sizes = resolve(&self.constraints, self.axis_length(inner), self.gap);
        self.place(inner, &sizes)
    }

    /// [`Layout::split`] into a fixed-size array, so children can be destructured by name.
    ///
    /// Panics if `N` does not match the number of constraints, which is a programming error
    /// rather than a runtime condition.
    pub fn split_array<const N: usize>(&self, area: Rect) -> [Rect; N] {
        assert_eq!(
            self.constraints.len(),
            N,
            "split_array asked for {N} rectangles from {} constraints",
            self.constraints.len()
        );
        let split = self.split(area);
        std::array::from_fn(|index| split[index])
    }

    fn axis_length(&self, area: Rect) -> u16 {
        match self.direction {
            Direction::Horizontal => area.width,
            Direction::Vertical => area.height,
        }
    }

    fn place(&self, area: Rect, sizes: &[u16]) -> Vec<Rect> {
        let axis = self.axis_length(area);
        let mut offset = 0u16;
        let mut rects = Vec::with_capacity(sizes.len());
        for (index, &size) in sizes.iter().enumerate() {
            // Clamp rather than trust the arithmetic. When the area is too small to hold even
            // the gaps, the running offset can walk past the end; a child placed outside its
            // parent would then be handed to a widget as a valid area to draw in.
            let start = offset.min(axis);
            let size = size.min(axis - start);
            let rect = match self.direction {
                Direction::Horizontal => Rect::new(area.x + start, area.y, size, area.height),
                Direction::Vertical => Rect::new(area.x, area.y + start, area.width, size),
            };
            rects.push(rect);
            offset = start.saturating_add(size);
            if index + 1 < sizes.len() {
                offset = offset.saturating_add(self.gap);
            }
        }
        rects
    }
}

/// Divide an axis of `total` cells among `constraints`, with `gap` cells between neighbours.
///
/// Exposed because it is occasionally useful on its own — laying out columns of a table
/// inside a canvas, say, where there are no rectangles involved.
pub fn resolve(constraints: &[Constraint], total: u16, gap: u16) -> Vec<u16> {
    if constraints.is_empty() {
        return Vec::new();
    }
    let gaps = gap.saturating_mul(constraints.len().saturating_sub(1) as u16);
    let available = total.saturating_sub(gaps);

    let mut sizes: Vec<u16> =
        constraints.iter().map(|constraint| constraint.base(available)).collect();

    // Percentages and ratios are shares of the same axis, so resolve them against it together.
    // Rounding each one alone loses a cell per child — three `Ratio(1, 3)` children would take
    // 33 columns each and leave the hundredth column stranded — whereas apportioning their
    // combined claim puts every cell somewhere. A group that claims less than the whole axis
    // still only gets its share: `Percentage(30)` alone is 30 cells, not everything.
    let shares: Vec<u32> =
        constraints.iter().map(|constraint| proportional_share(*constraint)).collect();
    let share_sum: u64 = shares.iter().map(|share| u64::from(*share)).sum();
    if share_sum > 0 {
        let claim = ((u64::from(available) * share_sum + PRECISION / 2) / PRECISION)
            .min(u64::from(available)) as u16;
        for (index, share) in apportion(claim, &shares).into_iter().enumerate() {
            if shares[index] > 0 {
                sizes[index] = share;
            }
        }
    }

    let requested: u32 = sizes.iter().map(|size| u32::from(*size)).sum();
    let available32 = u32::from(available);

    if requested < available32 {
        grow(constraints, &mut sizes, (available32 - requested) as u16);
    } else if requested > available32 {
        shrink(constraints, &mut sizes, (requested - available32).min(u32::from(u16::MAX)) as u16);
    }
    sizes
}

/// Hand out `surplus` cells.
fn grow(constraints: &[Constraint], sizes: &mut [u16], surplus: u16) {
    let weights: Vec<u32> = constraints.iter().map(|c| c.growth_weight()).collect();
    if weights.iter().any(|weight| *weight > 0) {
        for (size, share) in sizes.iter_mut().zip(apportion(surplus, &weights)) {
            *size = size.saturating_add(share);
        }
        return;
    }
    // Nothing asked to grow. `Min` is the next best claimant: it said "at least", which
    // implies it will take more. Split the surplus evenly rather than by current size, so a
    // long label does not soak up all the slack.
    let min_weights: Vec<u32> =
        constraints.iter().map(|c| u32::from(matches!(c, Constraint::Min(_)))).collect();
    if min_weights.iter().any(|weight| *weight > 0) {
        for (size, share) in sizes.iter_mut().zip(apportion(surplus, &min_weights)) {
            *size = size.saturating_add(share);
        }
    }
    // Otherwise every constraint got exactly what it asked for and the remainder is left
    // unused at the end of the area. Silently stretching a `Length` would be worse: the
    // caller said how wide it wanted the thing to be.
}

/// Reclaim `deficit` cells, taking from the most elastic constraints first.
fn shrink(constraints: &[Constraint], sizes: &mut [u16], mut deficit: u16) {
    for rigidity in 0..=4u8 {
        if deficit == 0 {
            return;
        }
        let group: Vec<usize> = (0..sizes.len())
            .filter(|&index| constraints[index].rigidity() == rigidity && sizes[index] > 0)
            .collect();
        if group.is_empty() {
            continue;
        }
        let pool: u32 = group.iter().map(|&index| u32::from(sizes[index])).sum();
        if pool <= u32::from(deficit) {
            // This whole class collapses and we still need more.
            for &index in &group {
                sizes[index] = 0;
            }
            deficit -= pool as u16;
            continue;
        }
        // The class can absorb the rest. Cut in proportion to current size, so the widest
        // child loses the most and the layout stays visually balanced.
        let weights: Vec<u32> = group.iter().map(|&index| u32::from(sizes[index])).collect();
        for (&index, cut) in group.iter().zip(apportion(deficit, &weights)) {
            sizes[index] = sizes[index].saturating_sub(cut);
        }
        return;
    }
}

/// Split `total` into parts proportional to `weights`, summing to exactly `total`.
///
/// Uses the largest-remainder method: every part gets its floor, then the leftover cells go
/// to whichever parts were rounded down hardest. Without this, ten `Fill(1)` children of a
/// 104-column area would each round to 10 and leave four columns stranded.
fn apportion(total: u16, weights: &[u32]) -> Vec<u16> {
    let sum: u64 = weights.iter().map(|weight| u64::from(*weight)).sum();
    if sum == 0 {
        return vec![0; weights.len()];
    }
    let total64 = u64::from(total);
    let mut shares = Vec::with_capacity(weights.len());
    let mut remainders: Vec<(u64, usize)> = Vec::with_capacity(weights.len());
    let mut assigned = 0u64;

    for (index, &weight) in weights.iter().enumerate() {
        let exact = total64 * u64::from(weight);
        let share = exact / sum;
        shares.push(share as u16);
        assigned += share;
        remainders.push((exact % sum, index));
    }

    // Largest remainder first; earlier index wins a tie so the result is deterministic.
    remainders.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut leftover = total64 - assigned;
    for (remainder, index) in remainders {
        if leftover == 0 {
            break;
        }
        if remainder == 0 && weights[index] == 0 {
            continue;
        }
        shares[index] += 1;
        leftover -= 1;
    }
    shares
}

/// Denominator for proportional shares: a share is expressed in millionths of the axis.
///
/// Fixed-point rather than floating-point so that a split is bit-for-bit reproducible, which
/// matters when the expected output of a UI test is a string of characters.
const PRECISION: u64 = 1_000_000;

/// How much of the axis a proportional constraint claims, in millionths. Zero for the rest.
fn proportional_share(constraint: Constraint) -> u32 {
    let share = match constraint {
        Constraint::Percentage(percent) => u64::from(percent.min(100)) * PRECISION / 100,
        Constraint::Ratio(_, 0) => 0,
        Constraint::Ratio(numerator, denominator) => {
            u64::from(numerator) * PRECISION / u64::from(denominator)
        }
        _ => 0,
    };
    share.min(PRECISION) as u32
}

/// Centre a fixed-size box inside `area`, clamped so it never spills out.
///
/// The usual way to place a modal or a "resize your terminal" message.
pub fn centered(area: Rect, width: u16, height: u16) -> Rect {
    area.centered(width.min(area.width), height.min(area.height))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every split must stay inside its area, in order, without overlapping.
    fn assert_tiles(area: Rect, layout: &Layout) {
        let split = layout.split(area);
        let inner = area.inset(layout.margin);
        let axis = match layout.direction {
            Direction::Horizontal => inner.width,
            Direction::Vertical => inner.height,
        };
        let used: u32 = split
            .iter()
            .map(|rect| match layout.direction {
                Direction::Horizontal => u32::from(rect.width),
                Direction::Vertical => u32::from(rect.height),
            })
            .sum();
        let gaps = u32::from(layout.gap) * (split.len().saturating_sub(1)) as u32;
        assert!(used <= u32::from(axis), "children used {used} of {axis} for {split:?}");
        // Gaps only have to fit once there is room for them at all. Below that the area cannot
        // hold the separators themselves, and collapsing children is the only sane outcome.
        if gaps <= u32::from(axis) {
            assert!(
                used + gaps <= u32::from(axis),
                "layout overflowed: {used} + {gaps} gap > {axis} for {split:?}"
            );
        }
        // Children must not overlap, and must stay inside the area.
        for pair in split.windows(2) {
            match layout.direction {
                Direction::Horizontal => assert!(pair[0].right() <= pair[1].x),
                Direction::Vertical => assert!(pair[0].bottom() <= pair[1].y),
            }
        }
        for rect in &split {
            assert_eq!(inner.union(*rect), inner, "{rect:?} escaped {inner:?}");
        }
    }

    #[test]
    fn fill_absorbs_what_a_length_leaves() {
        let [left, right] = Layout::horizontal([Constraint::Length(20), Constraint::Fill(1)])
            .split_array(Rect::sized(80, 24));
        assert_eq!((left.x, left.width), (0, 20));
        assert_eq!((right.x, right.width), (20, 60));
    }

    #[test]
    fn fill_weights_divide_the_remainder_proportionally() {
        let sizes = resolve(&[Constraint::Fill(2), Constraint::Fill(1)], 30, 0);
        assert_eq!(sizes, vec![20, 10]);
    }

    #[test]
    fn fill_with_an_indivisible_remainder_still_uses_every_cell() {
        // Three equal children of 100 columns cannot each be 33.33; the layout must not
        // silently drop the leftover column.
        let sizes = resolve(&[Constraint::Fill(1); 3], 100, 0);
        assert_eq!(sizes.iter().sum::<u16>(), 100);
        assert_eq!(sizes, vec![34, 33, 33]);
    }

    #[test]
    fn ten_equal_children_tile_an_awkward_width_exactly() {
        let sizes = resolve(&[Constraint::Fill(1); 10], 104, 0);
        assert_eq!(sizes.iter().sum::<u16>(), 104);
        assert!(sizes.iter().all(|size| (10..=11).contains(size)));
    }

    #[test]
    fn percentages_are_rounded_not_truncated() {
        let sizes = resolve(&[Constraint::Percentage(33), Constraint::Percentage(67)], 100, 0);
        assert_eq!(sizes, vec![33, 67]);
        // 50% of an odd width rounds up rather than losing the cell.
        assert_eq!(resolve(&[Constraint::Percentage(50)], 9, 0), vec![5]);
    }

    #[test]
    fn ratios_divide_exactly_where_percentages_would_not() {
        let sizes = resolve(&[Constraint::Ratio(1, 3); 3], 100, 0);
        assert_eq!(sizes.iter().sum::<u16>(), 100, "got {sizes:?}");
    }

    #[test]
    fn gaps_come_out_of_the_children_not_the_area() {
        let layout = Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).gap(2);
        let [left, right] = layout.split_array(Rect::sized(20, 3));
        assert_eq!(left.width, 9);
        assert_eq!(right.width, 9);
        assert_eq!(right.x, 11, "the gap must sit between the children");
        assert_tiles(Rect::sized(20, 3), &layout);
    }

    #[test]
    fn a_margin_insets_before_splitting() {
        let layout = Layout::vertical([Constraint::Fill(1)]).margin(Padding::all(1));
        let [only] = layout.split_array(Rect::sized(10, 10));
        assert_eq!(only, Rect::new(1, 1, 8, 8));
    }

    #[test]
    fn min_grows_when_nothing_else_wants_the_space() {
        let sizes = resolve(&[Constraint::Length(10), Constraint::Min(5)], 40, 0);
        assert_eq!(sizes, vec![10, 30]);
    }

    #[test]
    fn min_yields_to_fill_which_is_the_explicit_claimant() {
        let sizes = resolve(&[Constraint::Min(5), Constraint::Fill(1)], 40, 0);
        assert_eq!(sizes, vec![5, 35]);
    }

    #[test]
    fn max_never_exceeds_its_ceiling_even_with_room_to_spare() {
        let sizes = resolve(&[Constraint::Max(10), Constraint::Fill(1)], 100, 0);
        assert_eq!(sizes, vec![10, 90]);
    }

    #[test]
    fn surplus_with_no_claimant_is_left_unused_rather_than_stretching_a_length() {
        // `Length(10)` means ten cells. Growing it to fill the area would break every caller
        // who asked for a fixed-width gutter.
        let sizes = resolve(&[Constraint::Length(10), Constraint::Length(10)], 100, 0);
        assert_eq!(sizes, vec![10, 10]);
    }

    #[test]
    fn a_length_survives_when_a_fill_must_collapse() {
        let sizes = resolve(&[Constraint::Length(20), Constraint::Fill(1)], 15, 0);
        assert_eq!(sizes, vec![15, 0], "the fixed child keeps what it can, the fill gives up");
    }

    #[test]
    fn shrinking_takes_from_max_before_min_and_from_min_before_length() {
        let sizes =
            resolve(&[Constraint::Length(10), Constraint::Min(10), Constraint::Max(10)], 25, 0);
        assert_eq!(sizes, vec![10, 10, 5], "only the Max should have given ground");
    }

    #[test]
    fn an_impossible_layout_degrades_instead_of_overflowing() {
        let layout = Layout::horizontal([Constraint::Length(50); 4]).gap(3);
        assert_tiles(Rect::sized(40, 5), &layout);
        let split = layout.split(Rect::sized(40, 5));
        assert_eq!(split.len(), 4, "a child is never dropped, only shrunk to zero");
    }

    #[test]
    fn a_zero_sized_area_yields_zero_sized_children_not_a_panic() {
        let split =
            Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).split(Rect::ZERO);
        assert_eq!(split.len(), 2);
        assert!(split.iter().all(|rect| rect.is_empty()));
    }

    #[test]
    fn no_constraints_yields_no_rectangles() {
        assert!(Layout::horizontal([]).split(Rect::sized(80, 24)).is_empty());
    }

    #[test]
    fn a_ratio_with_a_zero_denominator_is_zero_rather_than_a_panic() {
        assert_eq!(resolve(&[Constraint::Ratio(1, 0), Constraint::Fill(1)], 10, 0), vec![0, 10]);
    }

    #[test]
    fn vertical_stacking_advances_down_the_area() {
        let layout =
            Layout::vertical([Constraint::Length(1), Constraint::Fill(1), Constraint::Length(1)]);
        let [header, body, footer] = layout.split_array(Rect::new(2, 3, 20, 10));
        assert_eq!(header, Rect::new(2, 3, 20, 1));
        assert_eq!(body, Rect::new(2, 4, 20, 8));
        assert_eq!(footer, Rect::new(2, 12, 20, 1));
        assert_tiles(Rect::new(2, 3, 20, 10), &layout);
    }

    #[test]
    fn centering_clamps_rather_than_spilling() {
        assert_eq!(centered(Rect::sized(10, 4), 6, 2), Rect::new(2, 1, 6, 2));
        // A box larger than the area is cropped to the area instead of landing off-screen.
        let clamped = centered(Rect::sized(10, 4), 40, 40);
        assert_eq!(clamped, Rect::sized(10, 4));
    }

    #[test]
    fn splits_tile_exactly_across_a_range_of_widths() {
        // The property that actually matters: whatever the terminal size, children never
        // overlap, never escape the area, and never leave a seam the background shows through.
        let layouts = [
            Layout::horizontal([Constraint::Length(20), Constraint::Fill(1)]),
            Layout::horizontal([Constraint::Fill(2), Constraint::Fill(1), Constraint::Fill(1)]),
            Layout::horizontal([Constraint::Percentage(30), Constraint::Percentage(70)]).gap(1),
            Layout::horizontal([Constraint::Min(10), Constraint::Max(30), Constraint::Fill(1)])
                .gap(2),
        ];
        for width in 0..=200u16 {
            for layout in &layouts {
                assert_tiles(Rect::sized(width, 10), layout);
            }
        }
    }
}
