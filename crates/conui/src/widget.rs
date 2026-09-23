//! The widget set.
//!
//! Deliberately small. Each of these exists because the reference design uses it more than
//! once, and most of them are a few dozen lines over the canvas — which is the point: if a
//! widget you need is missing, writing it is a `Paint` closure away, not a framework extension.
//!
//! None of them own state. A widget that needs a cursor, a selected index or an open/closed flag
//! borrows one of the plain structs in [`crate::state`] for the frame, and a widget that can be
//! focused is *told* so with `.focused(bool)` rather than asking a registry.

use std::ops::Range;

use conui_cell::{Padding, Pos, Rect, Style};

use crate::canvas::{Canvas, text_width};
use crate::layout::{Constraint, Direction, resolve};
use crate::state::{Checklist, Dropdown, Editor, Selection, Viewport};
use crate::theme::Role;
use crate::typography::{self, BarStyle, DIGIT_HEIGHT, line, mark};
use crate::view::{Stack, View};

/// Horizontal placement of text within its region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    /// Flush against the left edge.
    #[default]
    Left,
    /// Centred, with any odd column of slack going to the right.
    Center,
    /// Flush against the right edge, which is where a number belongs so that its digits line up
    /// with the number above it.
    Right,
}

/// The answer for a widget one row tall that takes whatever width it is given, which is most of
/// them: a gauge, a field, a text input. Saying `Fill(1)` across is not a shrug — it is the honest
/// answer for something that draws itself to the width it is handed.
const fn one_row(axis: Direction) -> Constraint {
    match axis {
        Direction::Vertical => Constraint::Length(1),
        Direction::Horizontal => Constraint::Fill(1),
    }
}

// ---- Text -------------------------------------------------------------------------------

/// A string, in a role's colour.
///
/// Honours embedded newlines, and will word-wrap on request. It does not wrap by default,
/// because in a fixed-size UI a line that silently becomes two lines pushes everything below it
/// off the screen — much worse than a line that gets clipped.
pub struct Text {
    content: String,
    role: Role,
    align: Align,
    wrap: bool,
    style: Option<Style>,
}

impl Text {
    /// A line of text in [`Role::Text`], left-aligned and unwrapped.
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            role: Role::Text,
            align: Align::Left,
            wrap: false,
            style: None,
        }
    }

    /// Draw in `role`'s colour, resolved against the theme at render time rather than now.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Shorthand for the three roles a label most often wants.
    pub fn muted(self) -> Self {
        self.role(Role::Muted)
    }

    /// [`Role::Accent`]: the one thing on this row worth looking at first.
    pub fn accent(self) -> Self {
        self.role(Role::Accent)
    }

    /// [`Role::Dim`]: present, readable, and not competing for attention.
    pub fn dim(self) -> Self {
        self.role(Role::Dim)
    }

    /// Override the role with a full style, for emphasis a role cannot express.
    pub fn styled(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    /// Place each line within the region width. Alignment only shows when the region is wider
    /// than the text, which for an unwrapped `Text` means when something else gave it the room.
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// [`Align::Center`].
    pub fn centered(self) -> Self {
        self.align(Align::Center)
    }

    /// [`Align::Right`], which is what a column of numbers wants.
    pub fn right(self) -> Self {
        self.align(Align::Right)
    }

    /// Wrap at the region width instead of clipping. Makes the view's height content-dependent,
    /// so it asks for all the space it is offered rather than a fixed number of rows.
    pub fn wrapped(mut self) -> Self {
        self.wrap = true;
        self
    }

    fn lines(&self, width: u16) -> Vec<String> {
        if self.wrap {
            self.content.split('\n').flat_map(|line| wrap(line, width)).collect()
        } else {
            self.content.split('\n').map(str::to_owned).collect()
        }
    }
}

impl View for Text {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let style = self.style.unwrap_or_else(|| canvas.role(self.role));
        for (row, line) in self.lines(canvas.width()).into_iter().enumerate() {
            let y = row as i32;
            let x = match self.align {
                Align::Left => 0,
                Align::Center => (i32::from(canvas.width()) - i32::from(text_width(&line))) / 2,
                Align::Right => i32::from(canvas.width()) - i32::from(text_width(&line)),
            };
            canvas.put_styled(x, y, &line, style);
        }
    }

    /// One row per line, and as wide as its widest line. Wrapping trades the first for the second:
    /// how many rows the text needs then depends on the width it is given, so it asks for a share
    /// and takes what it gets.
    fn constraint(&self, axis: Direction) -> Constraint {
        match (axis, self.wrap) {
            (_, true) => Constraint::Fill(1),
            (Direction::Vertical, false) => {
                Constraint::Length(self.content.split('\n').count() as u16)
            }
            (Direction::Horizontal, false) => {
                Constraint::Length(self.content.split('\n').map(text_width).max().unwrap_or(0))
            }
        }
    }
}

/// Greedy word wrap to `width` columns, hard-breaking words that do not fit on their own.
fn wrap(text: &str, width: u16) -> Vec<String> {
    if width == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut used = 0u16;

    for word in text.split_whitespace() {
        let word_width = text_width(word);
        // A word wider than the whole region has to be broken, or it would vanish entirely.
        if word_width > width {
            if used > 0 {
                lines.push(std::mem::take(&mut current));
            }
            let mut chunk = String::new();
            let mut chunk_width = 0u16;
            for character in word.chars() {
                let character_width = text_width(&character.to_string());
                if chunk_width + character_width > width {
                    lines.push(std::mem::take(&mut chunk));
                    chunk_width = 0;
                }
                chunk.push(character);
                chunk_width += character_width;
            }
            current = chunk;
            used = chunk_width;
            continue;
        }
        let needed = if used == 0 { word_width } else { used + 1 + word_width };
        if needed > width {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
            used = word_width;
        } else {
            if used > 0 {
                current.push(' ');
                used += 1;
            }
            current.push_str(word);
            used += word_width;
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

// ---- Rule -------------------------------------------------------------------------------

/// A horizontal divider, optionally with a label set into it.
pub struct Rule {
    title: Option<String>,
    role: Role,
    title_role: Role,
    heavy: bool,
}

impl Rule {
    /// A plain divider with no label.
    pub fn new() -> Self {
        Self { title: None, role: Role::Dim, title_role: Role::Muted, heavy: false }
    }

    /// A divider with `title` set into it, one cell in from the left. The cheapest way to head a
    /// section without spending the two rows and two columns a [`Panel`] costs.
    pub fn titled(title: impl Into<String>) -> Self {
        Self { title: Some(title.into()), ..Self::new() }
    }

    /// Colour of the rule itself. Defaults to [`Role::Dim`], because a divider that draws the eye
    /// is doing the opposite of its job.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Colour of the label, separately from the rule, so a heading can be legible while the line
    /// it sits in stays quiet.
    pub fn title_role(mut self, role: Role) -> Self {
        self.title_role = role;
        self
    }

    /// Draw with `━` instead of `─`, for the one divider on a screen that separates more than the
    /// others do.
    pub fn heavy(mut self) -> Self {
        self.heavy = true;
        self
    }
}

impl Default for Rule {
    fn default() -> Self {
        Self::new()
    }
}

impl View for Rule {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let glyph = if self.heavy { line::HEAVY_HORIZONTAL } else { line::HORIZONTAL };
        let width = canvas.width();
        canvas.run(0, 0, glyph, width, self.role);
        if let Some(title) = &self.title {
            // One cell of rule, then the label with breathing room either side. A label butted
            // straight against the rule reads as part of it.
            let label = format!(" {title} ");
            canvas.put(1, 0, &label, self.title_role);
        }
    }

    /// One row, and as wide as it is given: a divider that stopped short of the edge would read as
    /// an underline for whatever happened to be above it.
    fn constraint(&self, axis: Direction) -> Constraint {
        one_row(axis)
    }
}

// ---- Gauge ------------------------------------------------------------------------------

/// What a gauge prints next to its bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Readout {
    /// Nothing: the bar is the whole story, and the column the number would have cost goes back
    /// to the bar.
    None,
    /// `0.42`, matching the reference design's probability columns.
    #[default]
    Fraction,
    /// `42%`.
    Percent,
    /// Anything else — a rate, a byte count, a duration.
    Text(String),
}

impl Readout {
    fn render(&self, value: f32) -> Option<String> {
        match self {
            Self::None => None,
            Self::Fraction => Some(format!("{value:.2}")),
            Self::Percent => Some(format!("{:.0}%", value * 100.0)),
            Self::Text(text) => Some(text.clone()),
        }
    }
}

/// A labelled bar with a numeric readout: `RISK  ████░░░░  0.42`.
///
/// The bar takes whatever width is left after the label and readout, so a column of gauges
/// stays aligned however wide the panel is.
pub struct Gauge {
    value: f32,
    label: Option<String>,
    label_width: Option<u16>,
    label_role: Role,
    role: Role,
    track: Role,
    style: BarStyle,
    readout: Readout,
    bar_width: Option<u16>,
    /// Whether this gauge is one of a selectable set, and whether it is the selected one.
    selectable: bool,
    selected: bool,
}

impl Gauge {
    /// `value` is clamped to `0.0..=1.0` at draw time.
    pub fn new(value: f32) -> Self {
        Self {
            value,
            label: None,
            label_width: None,
            label_role: Role::Muted,
            role: Role::Accent,
            track: Role::Dim,
            style: BarStyle::default(),
            readout: Readout::default(),
            bar_width: None,
            selectable: false,
            selected: false,
        }
    }

    /// A value out of a total, for the common "length of N cells" case.
    pub fn of(value: f32, total: f32) -> Self {
        Self::new(if total > 0.0 { value / total } else { 0.0 })
    }

    /// Put a label to the left of the bar, in the columns the bar then does without.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Reserve a fixed number of columns for the label, so a column of gauges lines up even
    /// when the labels differ in length.
    pub fn label_width(mut self, width: u16) -> Self {
        self.label_width = Some(width);
        self
    }

    /// Colour the label differently from the default muted grey — for a selected row, whose
    /// label should carry the same colour as its bar.
    pub fn label_role(mut self, role: Role) -> Self {
        self.label_role = role;
        self
    }

    /// Colour of the filled run, and of the readout beside it.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Colour of the unfilled run. With [`BarStyle::Rule`] this is the *only* thing separating
    /// full from empty, so a track the same colour as the fill draws a bar with no level in it.
    pub fn track(mut self, role: Role) -> Self {
        self.track = role;
        self
    }

    /// Which glyphs the bar is drawn from. See [`BarStyle`] for what each one reads as.
    pub fn style(mut self, style: BarStyle) -> Self {
        self.style = style;
        self
    }

    /// What to print to the right of the bar, if anything.
    pub fn readout(mut self, readout: Readout) -> Self {
        self.readout = readout;
        self
    }

    /// Cap the bar at this many columns instead of letting it fill the region.
    pub fn bar_width(mut self, width: u16) -> Self {
        self.bar_width = Some(width);
        self
    }

    /// Mark this gauge as one of a selectable set, and say whether it is the current choice.
    ///
    /// Passing `false` still reserves the marker column, so selecting a different row does not
    /// shift every bar sideways — the movement would read as the values changing.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selectable = true;
        self.selected = selected;
        self
    }
}

impl View for Gauge {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let width = canvas.width();
        if width == 0 {
            return;
        }
        let gap = 1i32;
        let mut x = 0i32;

        if self.selectable {
            if self.selected {
                canvas.set(0, 0, mark::SELECTED, self.role);
            }
            x += 2;
        }

        if let Some(label) = &self.label {
            let reserved = self.label_width.unwrap_or_else(|| text_width(label));
            canvas.put_truncated(x, 0, label, reserved, self.label_role);
            x += i32::from(reserved) + gap;
        }

        let readout = self.readout.render(self.value.clamp(0.0, 1.0));
        let readout_width = readout.as_deref().map_or(0, text_width);
        let tail = if readout_width > 0 { i32::from(readout_width) + gap } else { 0 };

        let available = i32::from(width) - x - tail;
        if available > 0 {
            let length = match self.bar_width {
                Some(cap) => u16::try_from(available).unwrap_or(0).min(cap),
                None => u16::try_from(available).unwrap_or(0),
            };
            canvas.bar_with(x, 0, self.value, length, self.style, self.role, self.track);
        }
        if let Some(readout) = readout {
            // Right-aligned so the decimal points of stacked gauges line up.
            canvas.put_right(i32::from(width), 0, &readout, self.role);
        }
    }

    fn constraint(&self, axis: Direction) -> Constraint {
        one_row(axis)
    }
}

// ---- Progress ---------------------------------------------------------------------------

/// What a [`Progress`] bar prints beside itself.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Tally {
    /// `37/84 · 44%`. Both, because the percentage alone hides how much work there is and the
    /// count alone leaves the reader doing the division.
    #[default]
    CountAndPercent,
    /// `37/84`.
    Count,
    /// `44%`.
    Percent,
    /// Nothing: the bar is the whole story, and the columns go back to it.
    None,
    /// Anything the caller would rather say — a rate, a time remaining, a stage name.
    Text(String),
}

impl Tally {
    /// The text for `done` items of `total`, or of an unknown total.
    ///
    /// With no total there is no percentage to print, so the forms that would have shown one show
    /// the count on its own rather than a number nobody can compute.
    fn render(&self, done: usize, total: Option<usize>) -> Option<String> {
        match (self, total) {
            (Self::None, _) => None,
            (Self::Text(text), _) => Some(text.clone()),
            (Self::CountAndPercent, Some(total)) => {
                Some(format!("{done}/{total} {} {}%", mark::SEPARATOR, percent(done, total)))
            }
            (Self::Count, Some(total)) => Some(format!("{done}/{total}")),
            (Self::Percent, Some(total)) => Some(format!("{}%", percent(done, total))),
            (Self::CountAndPercent | Self::Count, None) => Some(done.to_string()),
            (Self::Percent, None) => None,
        }
    }
}

/// `done` out of `total` as a whole percentage, truncated and never above a hundred.
///
/// Truncated rather than rounded because a bar that says `100%` with work still to do is a bar
/// nobody believes the next time it says it. A total of nothing is complete by definition.
fn percent(done: usize, total: usize) -> usize {
    match total {
        0 => 100,
        total => (done.min(total) * 100) / total,
    }
}

/// How far through a job of known length you are: `SCAN ████████░░░░░░  37/84 · 44%`.
///
/// Distinct from [`Gauge`], which measures a level — a risk, a load, a share. This measures work,
/// and the difference is not cosmetic. It counts in whole items rather than a fraction, so the bar
/// and the numbers beside it can never disagree; it fills by truncation, so it reaches the end
/// exactly when the last item is done and not a moment before; and it accepts not knowing the
/// total, for the part of a job spent finding out how much of it there is.
///
/// Set a [`caption`](Self::caption) to name what is being worked on right now, which is the line
/// that turns a bar into something worth watching.
///
/// ```
/// use conui::widget::{Progress, Tally};
///
/// let scan = Progress::new(37, 84).label("SCAN").caption("chatSessions/3f2c…json");
/// assert_eq!(scan.fraction(), 37.0 / 84.0);
/// assert!(!scan.is_complete());
///
/// // Before the total is known, a marching bar and a bare count.
/// let finding = Progress::indeterminate(12).done(312).tally(Tally::Count);
/// assert!(finding.fraction() == 0.0);
/// ```
pub struct Progress {
    done: usize,
    /// `None` while the size of the job is still being discovered, which makes the bar march
    /// instead of fill.
    total: Option<usize>,
    /// Advances the marching block of an indeterminate bar. Ignored by a determinate one.
    tick: u64,
    label: Option<String>,
    label_width: Option<u16>,
    label_role: Role,
    caption: Option<String>,
    caption_role: Role,
    role: Role,
    track: Role,
    style: BarStyle,
    tally: Tally,
    bar_width: Option<u16>,
}

impl Progress {
    /// `done` items of `total` finished. A `done` past the total is clamped rather than overflowing
    /// the bar, since a miscounted job should not also draw wrongly.
    pub fn new(done: usize, total: usize) -> Self {
        Self {
            done,
            total: Some(total),
            tick: 0,
            label: None,
            label_width: None,
            label_role: Role::Muted,
            caption: None,
            caption_role: Role::Dim,
            role: Role::Accent,
            track: Role::Dim,
            style: BarStyle::Shaded,
            tally: Tally::default(),
            bar_width: None,
        }
    }

    /// A bar for a job whose length is not known yet: a block marches back and forth instead of
    /// filling, and the tally shows however many items have been dealt with so far.
    ///
    /// `tick` is what moves it, and it has to come from outside: a widget is a value rebuilt every
    /// frame and has no memory to animate from. A frame counter does, or
    /// `app.elapsed().as_millis() / 80` for a speed that does not depend on the frame rate.
    pub fn indeterminate(tick: u64) -> Self {
        Self { total: None, tick, ..Self::new(0, 0) }
    }

    /// How many items are done — the part of [`Progress::new`] that changes every frame, and the
    /// only number an [`indeterminate`](Self::indeterminate) bar has to show.
    pub fn done(mut self, done: usize) -> Self {
        self.done = done;
        self
    }

    /// Put a label to the left of the bar, in the columns the bar then does without.
    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// Reserve a fixed number of columns for the label, so that stacked bars line up even when
    /// their labels differ in length.
    pub fn label_width(mut self, width: u16) -> Self {
        self.label_width = Some(width);
        self
    }

    /// Colour the label differently from the default muted grey.
    pub fn label_role(mut self, role: Role) -> Self {
        self.label_role = role;
        self
    }

    /// A second row under the bar naming what is being worked on now, truncated to fit.
    ///
    /// This is what makes a long job legible: the bar says how far, the caption says where. It
    /// costs a row, which is why it is not the default.
    pub fn caption(mut self, caption: impl Into<String>) -> Self {
        self.caption = Some(caption.into());
        self
    }

    /// Colour of the caption row. Dim by default, so the eye goes to the bar first.
    pub fn caption_role(mut self, role: Role) -> Self {
        self.caption_role = role;
        self
    }

    /// Colour of the filled run, and of the tally beside it.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Colour of the unfilled run.
    pub fn track(mut self, role: Role) -> Self {
        self.track = role;
        self
    }

    /// Which glyphs the bar is drawn from. Solid blocks over a light shade by default: progress
    /// wants its full extent visible, so that how much is left reads at a glance.
    pub fn style(mut self, style: BarStyle) -> Self {
        self.style = style;
        self
    }

    /// What to print to the right of the bar, if anything.
    pub fn tally(mut self, tally: Tally) -> Self {
        self.tally = tally;
        self
    }

    /// Cap the bar at this many columns instead of letting it fill the region.
    pub fn bar_width(mut self, width: u16) -> Self {
        self.bar_width = Some(width);
        self
    }

    /// How far along, in `0.0..=1.0`. Zero for a job of unknown length, which has no fraction to
    /// report — ask [`Progress::is_indeterminate`] before believing it.
    pub fn fraction(&self) -> f32 {
        match self.total {
            Some(0) => 1.0,
            Some(total) => self.done.min(total) as f32 / total as f32,
            None => 0.0,
        }
    }

    /// Whether every item is done. False for a job whose length is not known yet: finishing is not
    /// something you can claim before you know what there was to do.
    pub fn is_complete(&self) -> bool {
        self.total.is_some_and(|total| self.done >= total)
    }

    /// Whether the length of the job is still unknown.
    pub fn is_indeterminate(&self) -> bool {
        self.total.is_none()
    }

    /// Columns the marching block of an indeterminate bar occupies: a fifth of the bar, and never
    /// so short that it reads as a cursor rather than as activity.
    fn block(length: u16) -> u16 {
        (length / 5).max(3).min(length)
    }

    /// Where that block starts, for a bar `length` wide at this tick.
    ///
    /// It bounces rather than wrapping. A block that reappears at the left the instant it leaves
    /// the right reads as two blocks at the seam; one that turns round reads as one thing moving.
    fn sweep(&self, length: u16) -> u16 {
        let travel = length.saturating_sub(Self::block(length));
        if travel == 0 {
            return 0;
        }
        let period = u64::from(travel) * 2;
        let position = self.tick % period;
        u16::try_from(position.min(period - position)).unwrap_or(0)
    }

    /// Draw the bar itself into `length` columns at `x`, track first.
    ///
    /// The fill is integer arithmetic over the counts rather than the rounded fraction the canvas
    /// would use, which is the whole reason this does not call `Canvas::bar_with`: rounding fills
    /// the last column early, and a bar that looks finished while the job is not is the one bug a
    /// progress bar must not have.
    fn bar(&self, canvas: &mut Canvas<'_>, x: i32, length: u16) {
        if length == 0 {
            return;
        }
        if let Some(character) = self.style.track() {
            canvas.run(x, 0, character, length, self.track);
        }
        let Some(total) = self.total else {
            let start = self.sweep(length);
            canvas.run(x + i32::from(start), 0, self.style.fill(), Self::block(length), self.role);
            return;
        };
        if total == 0 || self.done >= total {
            canvas.run(x, 0, self.style.fill(), length, self.role);
            return;
        }
        if self.style.is_smooth() {
            // Eighth-cell resolution, so a job of two hundred items still moves on every one of
            // them in a bar sixty columns wide.
            let eighths = self.done * usize::from(length) * 8 / total;
            let whole = u16::try_from(eighths / 8).unwrap_or(length).min(length);
            canvas.run(x, 0, self.style.fill(), whole, self.role);
            if eighths % 8 > 0 && whole < length {
                let partial = typography::HORIZONTAL_LEVELS[eighths % 8];
                canvas.set(x + i32::from(whole), 0, partial, self.role);
            }
            return;
        }
        let filled = u16::try_from(self.done * usize::from(length) / total).unwrap_or(length);
        canvas.run(x, 0, self.style.fill(), filled.min(length), self.role);
    }
}

impl View for Progress {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let width = canvas.width();
        if width == 0 || canvas.height() == 0 {
            return;
        }
        let gap = 1i32;
        let mut x = 0i32;

        if let Some(label) = &self.label {
            let reserved = self.label_width.unwrap_or_else(|| text_width(label));
            canvas.put_truncated(x, 0, label, reserved, self.label_role);
            x += i32::from(reserved) + gap;
        }

        let tally = self.tally.render(self.done, self.total);
        let tally_width = tally.as_deref().map_or(0, text_width);
        let tail = if tally_width > 0 { i32::from(tally_width) + gap } else { 0 };

        let available = i32::from(width) - x - tail;
        if available > 0 {
            let length = u16::try_from(available).unwrap_or(0);
            self.bar(canvas, x, self.bar_width.map_or(length, |cap| length.min(cap)));
        }
        if let Some(tally) = tally {
            // Right-aligned, so the numbers stay put as the bar grows past them.
            canvas.put_right(i32::from(width), 0, &tally, self.role);
        }

        // Under the label as well as the bar: the caption is prose, and prose indented to the
        // bar's column looks like it belongs to whichever item the bar happens to have reached.
        if let Some(caption) = &self.caption {
            if canvas.height() > 1 {
                canvas.put_truncated(0, 1, caption, width, self.caption_role);
            }
        }
    }

    /// Two rows with a caption, one without, and whatever width it is given.
    fn constraint(&self, axis: Direction) -> Constraint {
        match axis {
            Direction::Vertical => Constraint::Length(if self.caption.is_some() { 2 } else { 1 }),
            Direction::Horizontal => Constraint::Fill(1),
        }
    }
}

// ---- Stat -------------------------------------------------------------------------------

/// A label above a number in three-row block digits.
///
/// The reference design's score readouts. Zero-padded to a fixed digit count so that the
/// number keeps its footprint and neighbouring stats stay aligned as values grow.
pub struct Stat {
    label: String,
    value: i64,
    digits: usize,
    role: Role,
    label_role: Role,
}

impl Stat {
    /// A stat showing `value` under `label`, padded to three digits.
    pub fn new(label: impl Into<String>, value: i64) -> Self {
        Self { label: label.into(), value, digits: 3, role: Role::Accent, label_role: Role::Muted }
    }

    /// Minimum digit count; the number is zero-padded to this width.
    pub fn digits(mut self, digits: usize) -> Self {
        self.digits = digits;
        self
    }

    /// Colour of the digits.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Colour of the label above them.
    pub fn label_role(mut self, role: Role) -> Self {
        self.label_role = role;
        self
    }

    /// Columns this stat occupies, so a row of them can be sized without guessing.
    pub fn width(&self) -> u16 {
        let digits = typography::large_width(&format!("{:0width$}", 0, width = self.digits));
        digits.max(text_width(&self.label))
    }
}

impl View for Stat {
    fn render(&self, canvas: &mut Canvas<'_>) {
        canvas.put(0, 0, &self.label, self.label_role);
        canvas.number(0, 1, self.value, self.digits, self.role);
    }

    /// A label above block digits, and exactly as wide as the wider of the two. Both numbers were
    /// already here — [`Stat::width`] existed so callers could size a row of these by hand.
    fn constraint(&self, axis: Direction) -> Constraint {
        match axis {
            Direction::Vertical => Constraint::Length(DIGIT_HEIGHT + 1),
            Direction::Horizontal => Constraint::Length(self.width()),
        }
    }
}

// ---- Sparkline --------------------------------------------------------------------------

/// A one-row chart of a series, eight levels of height per column.
///
/// Shows the most recent values: when the series is longer than the region, the *start* is
/// dropped, because in a live dashboard the newest reading is the one that matters.
pub struct Sparkline {
    values: Vec<f32>,
    max: Option<f32>,
    role: Role,
}

impl Sparkline {
    /// A chart of `values`, oldest first, scaled to the largest value in the series.
    pub fn new(values: impl IntoIterator<Item = f32>) -> Self {
        Self { values: values.into_iter().collect(), max: None, role: Role::Info }
    }

    /// Fix the top of the scale. Without this the chart scales to its own maximum, which shows
    /// shape but hides level — a flat series at 10% looks identical to one at 90%.
    pub fn max(mut self, max: f32) -> Self {
        self.max = Some(max);
        self
    }

    /// Colour of the chart.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }
}

impl View for Sparkline {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let width = usize::from(canvas.width());
        if width == 0 || self.values.is_empty() {
            return;
        }
        let visible = &self.values[self.values.len().saturating_sub(width)..];
        let max =
            self.max.unwrap_or_else(|| visible.iter().copied().fold(f32::MIN_POSITIVE, f32::max));
        canvas.sparkline(0, 0, visible, max, self.role);
    }

    fn constraint(&self, axis: Direction) -> Constraint {
        one_row(axis)
    }
}

// ---- Field ------------------------------------------------------------------------------

/// A label and a value on one row: `INFERENCE          12.4 ms`.
///
/// The workhorse of a status panel. The value is right-aligned by default so a column of them
/// reads as a table without anyone having to count columns.
pub struct Field {
    label: String,
    value: String,
    label_role: Role,
    value_role: Role,
    value_column: Option<u16>,
}

impl Field {
    /// A row with `label` at the left and `value` pushed to the right edge, the label truncated
    /// first if the two together do not fit.
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            label_role: Role::Muted,
            value_role: Role::Text,
            value_column: None,
        }
    }

    /// Colour of the label.
    pub fn label_role(mut self, role: Role) -> Self {
        self.label_role = role;
        self
    }

    /// Colour of the value, which is the half worth colouring when a reading goes out of range.
    pub fn value_role(mut self, role: Role) -> Self {
        self.value_role = role;
        self
    }

    /// Start the value at a fixed column rather than right-aligning it.
    pub fn value_column(mut self, column: u16) -> Self {
        self.value_column = Some(column);
        self
    }
}

impl View for Field {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let width = canvas.width();
        let value_width = text_width(&self.value);
        match self.value_column {
            Some(column) => {
                canvas.put_truncated(0, 0, &self.label, column.saturating_sub(1), self.label_role);
                canvas.put(i32::from(column), 0, &self.value, self.value_role);
            }
            None => {
                let room = width.saturating_sub(value_width).saturating_sub(1);
                canvas.put_truncated(0, 0, &self.label, room, self.label_role);
                canvas.put_right(i32::from(width), 0, &self.value, self.value_role);
            }
        }
    }

    fn constraint(&self, axis: Direction) -> Constraint {
        one_row(axis)
    }
}

// ---- Panel ------------------------------------------------------------------------------

/// How a panel marks its edges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Border {
    /// No frame at all. The default, and what the reference design uses: a title in the accent
    /// colour and whitespace around a group reads as a panel without spending four rows and
    /// four columns on lines.
    #[default]
    None,
    /// A light box round the whole panel, with the title set into the top rule so it costs no
    /// rows of its own.
    Line,
    /// The same box with `╭╮╰╯` corners.
    Rounded,
}

/// A titled group of views, stacked vertically.
pub struct Panel<'a> {
    title: Option<String>,
    subtitle: Option<String>,
    border: Border,
    title_role: Role,
    subtitle_role: Role,
    border_role: Role,
    body: Stack<'a>,
    padding: Padding,
}

impl<'a> Panel<'a> {
    /// An unbordered panel headed by `title`, with children added by [`child`](Self::child).
    ///
    /// The title is the one part of a panel a short window cannot take away — children get
    /// shrunk, chrome does not — so it is the place to put whatever the reader must still be able
    /// to see when the pane is squeezed.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: Some(title.into()),
            subtitle: None,
            border: Border::None,
            title_role: Role::Accent,
            subtitle_role: Role::Muted,
            border_role: Role::Dim,
            body: Stack::new(crate::layout::Direction::Vertical),
            padding: Padding::ZERO,
        }
    }

    /// A panel with no title, for grouping without labelling.
    pub fn bare() -> Self {
        Self { title: None, ..Self::new("") }
    }

    /// A second header row under the title, for the qualifier that would make the title too long.
    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    /// Whether to draw a frame, and of what kind. Defaults to [`Border::None`].
    pub fn border(mut self, border: Border) -> Self {
        self.border = border;
        self
    }

    /// Colour of the title. Defaults to [`Role::Accent`], which is what makes an unbordered panel
    /// read as a panel.
    pub fn title_role(mut self, role: Role) -> Self {
        self.title_role = role;
        self
    }

    /// Colour of the frame, if there is one. A focused pane is usually this rather than anything
    /// louder.
    pub fn border_role(mut self, role: Role) -> Self {
        self.border_role = role;
        self
    }

    /// Append a view to the body, below whatever is already there.
    pub fn child(mut self, view: impl View + 'a) -> Self {
        self.body = self.body.child(view);
        self
    }

    /// Append several views of the same type, for a body built from a collection.
    pub fn children<V: View + 'a>(mut self, views: impl IntoIterator<Item = V>) -> Self {
        self.body = self.body.children(views);
        self
    }

    /// Blank rows between children.
    pub fn gap(mut self, gap: u16) -> Self {
        self.body = self.body.gap(gap);
        self
    }

    /// Space between the panel's edge (or frame) and its content.
    pub fn padding(mut self, padding: Padding) -> Self {
        self.padding = padding;
        self
    }

    /// Rows this panel spends on its own chrome before the first child.
    ///
    /// Public because sizing a panel's slot in a parent means knowing this, and counting it by
    /// hand at the call site goes stale the moment a subtitle is added.
    pub fn header_height(&self) -> u16 {
        let title = u16::from(self.title.is_some() && self.border == Border::None);
        let frame = u16::from(self.border != Border::None);
        title + u16::from(self.subtitle.is_some()) + frame + self.padding.top
    }
}

impl View for Panel<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let area = canvas.area();
        if area.is_empty() {
            return;
        }
        let framed = match self.border {
            Border::None => false,
            Border::Line => {
                canvas.border(area, self.border_role);
                true
            }
            Border::Rounded => {
                canvas.border_rounded(area, self.border_role);
                true
            }
        };

        let frame_padding = if framed { Padding::all(1) } else { Padding::ZERO };
        let mut framed_inner = canvas.inset(frame_padding);
        let mut inner = framed_inner.inset(self.padding);

        // A framed title lives in the top rule and costs no rows; an unframed one takes one.
        let mut header = 0u16;
        if let (false, Some(title)) = (framed, &self.title) {
            let width = inner.width();
            inner.put_truncated(0, 0, title, width, self.title_role);
            header += 1;
        }
        if let Some(subtitle) = &self.subtitle {
            let width = inner.width();
            inner.put_truncated(0, i32::from(header), subtitle, width, self.subtitle_role);
            header += 1;
        }

        let body_area = Rect::new(0, header, inner.width(), inner.height().saturating_sub(header));
        let mut body = inner.sub(body_area);
        self.body.render(&mut body);

        // The framed title goes on last so it overwrites the rule rather than the reverse.
        if framed {
            if let Some(title) = &self.title {
                let room = area.width.saturating_sub(4);
                if room > 0 {
                    canvas.put_truncated(2, 0, &format!(" {title} "), room, self.title_role);
                }
            }
        }
    }
}

// ---- Hints ------------------------------------------------------------------------------

/// A footer legend of keys and what they do.
///
/// Worth a widget rather than a format string because discoverability is the hardest problem a
/// terminal UI has: there are no menus and nothing to hover, so the key map has to be on screen.
pub struct Hints {
    items: Vec<(String, String)>,
    key_role: Role,
    label_role: Role,
    spacing: u16,
}

impl Hints {
    /// An empty legend. Add entries with [`key`](Self::key).
    pub fn new() -> Self {
        Self { items: Vec::new(), key_role: Role::Muted, label_role: Role::Muted, spacing: 3 }
    }

    /// Add `key` and what pressing it does, to the right of everything already added.
    ///
    /// Order is the order they appear, and it is worth choosing: when the window is too narrow
    /// for the whole legend the entries at the end are the ones that go.
    pub fn key(mut self, key: impl Into<String>, action: impl Into<String>) -> Self {
        self.items.push((key.into(), action.into()));
        self
    }

    /// Draw the key names in a stronger colour than their descriptions.
    pub fn emphasise_keys(mut self) -> Self {
        self.key_role = Role::Text;
        self
    }

    /// Draw keys and descriptions in one colour, undoing any
    /// [`emphasise_keys`](Self::emphasise_keys).
    pub fn role(mut self, role: Role) -> Self {
        self.label_role = role;
        self.key_role = role;
        self
    }

    /// Columns between one entry and the next. Three by default, which is enough to read the pairs
    /// as pairs without a separator glyph between them.
    pub fn spacing(mut self, spacing: u16) -> Self {
        self.spacing = spacing;
        self
    }

    /// Columns one item occupies: the key, a space, the description.
    fn item_width(key: &str, action: &str) -> u16 {
        text_width(key) + 1 + text_width(action)
    }

    /// Columns this legend wants, with no trailing spacing.
    pub fn width(&self) -> u16 {
        let items: u16 = self.items.iter().map(|(k, a)| Self::item_width(k, a)).sum();
        let gaps = self.items.len().saturating_sub(1) as u16 * self.spacing;
        items + gaps
    }
}

impl Default for Hints {
    fn default() -> Self {
        Self::new()
    }
}

impl View for Hints {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let mut x = 0i32;
        for (key, action) in &self.items {
            // Drop an item that does not fit whole rather than letting the canvas clip it. A
            // legend reading `ESC qui` looks like a bug in the program; one hint fewer just looks
            // like a narrow window.
            if x + i32::from(Self::item_width(key, action)) > i32::from(canvas.width()) {
                break;
            }
            x += i32::from(canvas.put(x, 0, key, self.key_role));
            x += 1;
            x += i32::from(canvas.put(x, 0, action, self.label_role));
            x += i32::from(self.spacing);
        }
    }

    /// One row, and as wide as its items. A legend that has been given less says as much of itself
    /// as fits, but it can state what it wants — [`Hints::width`] is the same number it draws to.
    fn constraint(&self, axis: Direction) -> Constraint {
        match axis {
            Direction::Vertical => Constraint::Length(1),
            Direction::Horizontal => Constraint::Length(self.width()),
        }
    }
}

// ---- List -------------------------------------------------------------------------------

/// One row of a [`List`].
///
/// A plain string converts into one, so the common case never mentions this type. Reach for the
/// builders when a row needs its own colour, or a short status column that lines up down the list.
pub struct ListRow {
    text: String,
    role: Option<Role>,
    mark: Option<(String, Role)>,
}

impl ListRow {
    /// A row showing `text`, in the list's own colour.
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into(), role: None, mark: None }
    }

    /// Colour this row differently from the rest — a done task, a failing test, a stale entry.
    pub fn role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }

    /// A short status column drawn before the text, in its own colour.
    ///
    /// Every row's text starts at the same column, set by the widest mark in the list, so a
    /// checkbox or a `WARN`/`INFO` level reads as a column rather than as ragged prose.
    pub fn mark(mut self, text: impl Into<String>, role: Role) -> Self {
        self.mark = Some((text.into(), role));
        self
    }
}

impl From<&str> for ListRow {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for ListRow {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

/// A vertical list of rows, one of which may be selected.
///
/// The selection and scroll position live in a [`Selection`] you own, so moving the cursor is
/// your keybinding calling your state — the list only draws. Scrolling needs nothing from you:
/// the window follows the selection, because the height it has to fit into is only known here.
pub struct List<'a> {
    rows: Vec<ListRow>,
    selection: Option<&'a Selection>,
    checklist: Option<&'a Checklist>,
    marker: String,
    role: Role,
    selected_role: Role,
    check_role: Role,
    highlight: bool,
    empty: Option<String>,
}

impl<'a> List<'a> {
    /// A list of `rows`, with no cursor until one is given a
    /// [`selection`](Self::selection). Strings convert into rows, so an iterator of `&str` works.
    pub fn new<R: Into<ListRow>>(rows: impl IntoIterator<Item = R>) -> Self {
        Self {
            rows: rows.into_iter().map(Into::into).collect(),
            selection: None,
            checklist: None,
            marker: format!("{} ", mark::SELECTED),
            role: Role::Text,
            selected_role: Role::Accent,
            check_role: Role::Accent,
            highlight: false,
            empty: None,
        }
    }

    /// Draw a cursor on the row this selection points at.
    ///
    /// Without one the list is a static column of text, which is the right thing for a log pane.
    pub fn selection(mut self, selection: &'a Selection) -> Self {
        self.selection = Some(selection);
        self
    }

    /// Draw a tick box in front of every row, from a [`Checklist`] that also supplies the cursor.
    ///
    /// This is the whole of a multi-select list: the boxes come from the same state as the cursor,
    /// so they cannot disagree about how many rows there are, and there is no second
    /// [`selection`](Self::selection) call to remember. Takes precedence over one if both are given.
    pub fn checklist(mut self, checklist: &'a Checklist) -> Self {
        self.checklist = Some(checklist);
        self.selection = Some(checklist.selection());
        self
    }

    /// The cursor drawn against the selected row. Its width indents every row, selected or not,
    /// so moving the selection never shifts the text sideways.
    pub fn marker(mut self, marker: impl Into<String>) -> Self {
        self.marker = marker.into();
        self
    }

    /// Colour of an ordinary row. A row's own [`ListRow::role`] wins over this.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Colour of the selected row and its marker. This one wins over a row's own role, because a
    /// cursor that disappears on some rows is worse than a row that loses its colour while it is
    /// under the cursor.
    pub fn selected_role(mut self, role: Role) -> Self {
        self.selected_role = role;
        self
    }

    /// Also lift the selected row onto the theme's surface colour.
    ///
    /// Worth it when the list is the only thing on screen; skip it when a marker and a colour are
    /// already enough, since a filled bar is a lot of ink.
    pub fn highlight(mut self) -> Self {
        self.highlight = true;
        self
    }

    /// What to say when there are no rows at all.
    ///
    /// An empty list with no message is indistinguishable from a broken one.
    pub fn empty(mut self, message: impl Into<String>) -> Self {
        self.empty = Some(message.into());
        self
    }

    /// How many rows the list holds — all of them, not just the visible ones. The number to pass
    /// to [`Selection::clamp`](crate::state::Selection::clamp) after the data changed underneath.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are no rows, in which case the [`empty`](Self::empty) message is what draws.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Colour of a ticked box. The empty ones are always dim: a column of boxes should read as the
    /// ticked ones and nothing else.
    pub fn check_role(mut self, role: Role) -> Self {
        self.check_role = role;
        self
    }

    /// Columns a tick box and its trailing space take, or none if this is not a checklist.
    fn check_width(&self) -> u16 {
        match self.checklist {
            Some(_) => 4,
            None => 0,
        }
    }

    /// The column the tick boxes start at, for resolving a click on one.
    ///
    /// `None` for a list without a [`checklist`](Self::checklist). Pair it with
    /// [`Selection::row_at`](crate::state::Selection::row_at): the row says which item, and whether
    /// the click was in this column says whether it meant "tick it" or "select it".
    pub fn check_column(&self) -> Option<Range<u16>> {
        self.checklist.map(|_| {
            let start = if self.selection.is_some() { text_width(&self.marker) } else { 0 };
            // The trailing space belongs to the text, not to the box: clicking the gap between a
            // box and its label is a click on the label.
            start..start + 3
        })
    }

    /// Width of the mark column: the widest mark in the list, so the column does not jitter as
    /// rows scroll through it.
    fn mark_width(&self) -> u16 {
        self.rows
            .iter()
            .filter_map(|row| row.mark.as_ref())
            .map(|(text, _)| text_width(text))
            .max()
            .unwrap_or(0)
    }

    /// The column a row's text starts at, past the cursor column and the mark.
    ///
    /// For resolving a click across a row: everything left of this is the mark, and a click on a
    /// task's tick usually means "tick it" rather than "select it". Depends on the rows, so ask the
    /// list you actually drew.
    pub fn text_column(&self) -> u16 {
        let marker = if self.selection.is_some() { text_width(&self.marker) } else { 0 };
        let mark = self.mark_width();
        marker + self.check_width() + mark + u16::from(mark > 0)
    }
}

impl View for List<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let (width, height) = (canvas.width(), canvas.height());
        if width == 0 || height == 0 {
            return;
        }
        if self.rows.is_empty() {
            if let Some(message) = &self.empty {
                canvas.put_truncated(0, 0, message, width, Role::Muted);
            }
            return;
        }

        let marker_width = if self.selection.is_some() { text_width(&self.marker) } else { 0 };
        let mark_width = self.mark_width();
        let text_x = self.text_column();
        let selected = self.selection.map(Selection::selected);
        let window = match self.selection {
            Some(selection) => selection.window(height, self.rows.len()),
            None => 0..usize::from(height).min(self.rows.len()),
        };

        for (row_offset, index) in window.enumerate() {
            let row = &self.rows[index];
            let y = row_offset as i32;
            let is_selected = selected == Some(index);

            if is_selected && marker_width > 0 {
                canvas.put(0, y, &self.marker, self.selected_role);
            }
            if let Some(checklist) = self.checklist {
                let ticked = checklist.is_checked(index);
                let box_role = if ticked { self.check_role } else { Role::Dim };
                let glyph = if ticked { mark::CHECK } else { ' ' };
                canvas.put(i32::from(marker_width), y, "[", Role::Dim);
                canvas.set(i32::from(marker_width) + 1, y, glyph, box_role);
                canvas.put(i32::from(marker_width) + 2, y, "]", Role::Dim);
            }
            if let Some((text, role)) = &row.mark {
                canvas.put_truncated(
                    i32::from(marker_width + self.check_width()),
                    y,
                    text,
                    mark_width,
                    *role,
                );
            }
            let role = if is_selected { self.selected_role } else { row.role.unwrap_or(self.role) };
            canvas.put_truncated(i32::from(text_x), y, &row.text, width - text_x.min(width), role);

            // Last, because every write above sets a background of its own: patching the row
            // afterwards is the only order in which the highlight survives.
            if is_selected && self.highlight {
                let surface = canvas.theme().surface;
                canvas.style_area(
                    Rect::new(0, row_offset as u16, width, 1),
                    Style::new().bg(surface),
                );
            }
        }
    }

    fn constraint(&self, axis: Direction) -> Constraint {
        let _ = axis;
        Constraint::Fill(1)
    }
}

// ---- Table ------------------------------------------------------------------------------

/// One column of a [`Table`]: what it is called, how wide it is, and which edge its cells sit
/// against.
///
/// A plain string converts into one, so a table of equal, left-aligned columns never names this
/// type. Reach for the builders when a column holds numbers, which want a fixed width and a right
/// edge so that a digit lines up with the digit above it.
///
/// Prefixed rather than called `Column` because [`Column`](crate::view::Column) is already the
/// vertical stack, and a table is very often inside one — two types of that name in a file would
/// have to be renamed at the import, which is a worse place to learn about the clash.
pub struct TableColumn {
    heading: String,
    constraint: Constraint,
    align: Align,
}

impl TableColumn {
    /// A column headed `heading`, taking an equal share of the width, text against the left.
    pub fn new(heading: impl Into<String>) -> Self {
        Self { heading: heading.into(), constraint: Constraint::Fill(1), align: Align::Left }
    }

    /// Exactly this many cells, whatever the table is given. What a column of numbers wants, since
    /// its width is decided by the widest figure it will ever hold rather than by the window.
    pub fn length(mut self, columns: u16) -> Self {
        self.constraint = Constraint::Length(columns);
        self
    }

    /// Take a share of whatever the fixed columns leave, weighted against the other flexible ones.
    /// The default, at weight 1.
    pub fn flex(mut self, weight: u16) -> Self {
        self.constraint = Constraint::Fill(weight);
        self
    }

    /// At least this many cells, and more if any is going spare.
    pub fn at_least(mut self, columns: u16) -> Self {
        self.constraint = Constraint::Min(columns);
        self
    }

    /// Which edge the cells sit against. The heading goes the same way, because a heading that
    /// does not sit over its own figures is worse than no heading.
    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// Against the right edge — for a column of numbers, and the reason this is worth a shorthand.
    pub fn right(self) -> Self {
        self.align(Align::Right)
    }
}

impl From<&str> for TableColumn {
    fn from(heading: &str) -> Self {
        Self::new(heading)
    }
}

impl From<String> for TableColumn {
    fn from(heading: String) -> Self {
        Self::new(heading)
    }
}

/// One row of a [`Table`]: a cell per column.
///
/// Cells past the last column are not drawn, and columns past the last cell are left blank — a row
/// that does not match the header is a bug in the caller, and a panic in a draw is a worse way to
/// report it than a gap on screen.
pub struct TableRow {
    cells: Vec<String>,
    role: Option<Role>,
}

impl TableRow {
    /// A row of `cells`, in the table's own colour.
    pub fn new<S: Into<String>>(cells: impl IntoIterator<Item = S>) -> Self {
        Self { cells: cells.into_iter().map(Into::into).collect(), role: None }
    }

    /// Colour this row differently from the rest — a failing check, a stale entry, a dead process.
    pub fn role(mut self, role: Role) -> Self {
        self.role = Some(role);
        self
    }
}

impl<S: Into<String>> From<Vec<S>> for TableRow {
    fn from(cells: Vec<S>) -> Self {
        Self::new(cells)
    }
}

/// What a click on a [`Table`] landed on.
///
/// The two answers are different questions — one sorts, one selects — and a table is the only thing
/// that can tell them apart, because it alone knows where its heading ends and how wide each column
/// came out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableHit {
    /// The heading of this column, by index. Which is how a table gets click-to-sort.
    Heading(usize),
    /// This row, by index into the rows given — not into the ones on screen.
    Row(usize),
}

/// A grid of cells under a heading, one row of which may be selected.
///
/// The difference between this and a [`List`] of pre-formatted strings is that the widths are stated
/// once and everything else follows from them. The heading sits over its own figures because it is
/// placed by the same numbers the cells are; a click resolves to a column because the widths are
/// still in hand when the question is asked; and a cell of Japanese does not shove the column to its
/// right, because the placement measures display width rather than counting characters — which
/// `format!("{:>7}")` cannot do.
///
/// Columns divide the width by [`resolve`], the same function a [`Row`](crate::view::Row) uses, so
/// `Length` columns are honoured first and `Fill` absorbs the rest. As with [`List`], the cursor and
/// scroll position live in a [`Selection`] you own; the window follows the cursor because only the
/// widget knows the height it has to fit.
///
/// ```
/// use conui::widget::{Table, TableColumn};
/// use conui::Selection;
///
/// let selection = Selection::new();
/// let table = Table::new([
///         TableColumn::new("PID").length(7).right(),
///         TableColumn::new("CPU%").length(6).right(),
///         TableColumn::new("COMMAND"),
///     ])
///     .rows([vec!["4821", "62.0", "cargo"], vec!["4832", "58.0", "rustc"]])
///     .selection(&selection)
///     .sorted_by(1, true);
/// ```
pub struct Table<'a> {
    columns: Vec<TableColumn>,
    rows: Vec<TableRow>,
    selection: Option<&'a Selection>,
    marker: String,
    gap: u16,
    role: Role,
    selected_role: Role,
    heading_role: Role,
    highlight: bool,
    empty: Option<String>,
    sorted_by: Option<(usize, bool)>,
}

impl<'a> Table<'a> {
    /// A table with these columns and no rows yet. Strings convert into columns, so an iterator of
    /// `&str` gives equal, left-aligned ones.
    pub fn new<C: Into<TableColumn>>(columns: impl IntoIterator<Item = C>) -> Self {
        Self {
            columns: columns.into_iter().map(Into::into).collect(),
            rows: Vec::new(),
            selection: None,
            marker: format!("{} ", mark::SELECTED),
            gap: 1,
            role: Role::Text,
            selected_role: Role::Accent,
            heading_role: Role::Muted,
            highlight: false,
            empty: None,
            sorted_by: None,
        }
    }

    /// The rows to draw, a `Vec` of cells each, or [`TableRow`]s where a row wants its own colour.
    pub fn rows<R: Into<TableRow>>(mut self, rows: impl IntoIterator<Item = R>) -> Self {
        self.rows = rows.into_iter().map(Into::into).collect();
        self
    }

    /// Draw a cursor on the row this selection points at, and scroll to keep it in view.
    pub fn selection(mut self, selection: &'a Selection) -> Self {
        self.selection = Some(selection);
        self
    }

    /// The cursor drawn against the selected row. Its width indents every row and the heading
    /// alike, so moving the selection never shifts a column sideways.
    pub fn marker(mut self, marker: impl Into<String>) -> Self {
        self.marker = marker.into();
        self
    }

    /// Cells between one column and the next. One by default, which is the least that still reads
    /// as a gap; zero is right for columns that are already separated by their contents.
    pub fn gap(mut self, cells: u16) -> Self {
        self.gap = cells;
        self
    }

    /// Colour of an ordinary cell. A row's own [`TableRow::role`] wins over this.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Colour of the selected row and its marker. Wins over a row's own role, for the reason
    /// [`List::selected_role`] gives: a cursor that vanishes on some rows is worse than a row that
    /// loses its colour while it is under the cursor.
    pub fn selected_role(mut self, role: Role) -> Self {
        self.selected_role = role;
        self
    }

    /// Colour of the heading row. Muted by default, because a heading is read once and the figures
    /// under it are read every frame.
    pub fn heading_role(mut self, role: Role) -> Self {
        self.heading_role = role;
        self
    }

    /// Also lift the selected row onto the theme's surface colour.
    pub fn highlight(mut self) -> Self {
        self.highlight = true;
        self
    }

    /// What to say when there are no rows. The heading still draws, because the columns are still
    /// true; an empty table with no message is indistinguishable from a broken one.
    pub fn empty(mut self, message: impl Into<String>) -> Self {
        self.empty = Some(message.into());
        self
    }

    /// Mark a column as the one the rows are sorted by, with an arrow saying which way.
    ///
    /// The table does no sorting — the order of the rows is whatever you passed. This only says so
    /// on screen, which is the half a widget can honestly do.
    pub fn sorted_by(mut self, column: usize, descending: bool) -> Self {
        self.sorted_by = Some((column, descending));
        self
    }

    /// How many rows the table holds — all of them, not just the visible ones. The number to pass
    /// to [`Selection::clamp`](crate::state::Selection::clamp) after the data changed underneath.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are no rows, in which case the [`empty`](Self::empty) message is what draws.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// Rows of heading. One, always: a table without a heading is a [`List`], and the column widths
    /// are the only reason to reach for this instead.
    const HEADING_HEIGHT: u16 = 1;

    /// Width of the cursor gutter, which is zero without a selection to draw one for.
    fn gutter(&self) -> u16 {
        if self.selection.is_some() { text_width(&self.marker) } else { 0 }
    }

    /// The width each column came out at, for a table `width` cells across.
    fn widths(&self, width: u16) -> Vec<u16> {
        let constraints: Vec<Constraint> = self.columns.iter().map(|c| c.constraint).collect();
        resolve(&constraints, width.saturating_sub(self.gutter()), self.gap)
    }

    /// A heading, with the sort arrow on it if this is the sorted column.
    fn heading(&self, index: usize) -> String {
        let column = &self.columns[index];
        match self.sorted_by {
            Some((sorted, descending)) if sorted == index => {
                let arrow = if descending { mark::ARROW_DOWN } else { mark::ARROW_UP };
                format!("{} {}", column.heading, arrow)
            }
            _ => column.heading.clone(),
        }
    }

    /// What a click at `pos` landed on, given the `area` the table was drawn in.
    ///
    /// Record the area with [`.hit(..)`](crate::view::ViewExt::hit) while composing the frame and
    /// ask this when the click arrives — the same shape as [`Selection::row_at`], and for the same
    /// reason: only the frame that was drawn knows where anything ended up.
    ///
    /// A click in the cursor gutter counts as the first column rather than as nothing. The gutter is
    /// two cells directly left of a heading, aiming at a heading is a coarse gesture, and a dead
    /// strip that silently does nothing is the worse of the two answers.
    pub fn hit_at(&self, area: Rect, pos: Pos) -> Option<TableHit> {
        if !area.contains(pos) || self.columns.is_empty() {
            return None;
        }
        if pos.y == area.y {
            let local = pos.x.saturating_sub(area.x + self.gutter());
            let mut edge = 0u16;
            for (index, width) in self.widths(area.width).iter().enumerate() {
                edge = edge.saturating_add(width.saturating_add(self.gap));
                if local < edge {
                    return Some(TableHit::Heading(index));
                }
            }
            // Past the last boundary is the last column, which is where the eye puts it: the right
            // edge of a table belongs to whatever column reaches it.
            return Some(TableHit::Heading(self.columns.len() - 1));
        }
        let selection = self.selection?;
        let body = Rect::new(
            area.x,
            area.y.saturating_add(Self::HEADING_HEIGHT),
            area.width,
            area.height.saturating_sub(Self::HEADING_HEIGHT),
        );
        selection.row_at(body, pos, self.rows.len()).map(TableHit::Row)
    }
}

/// Place `text` within a column `width` wide starting at `x`, against the edge `align` names.
///
/// Measured in display width rather than characters, which is the whole reason a table is a widget
/// and not a `format!`: `{:>9}` pads a two-cell ideograph as though it were one column wide, and
/// every column to its right ends up one cell out.
fn place(
    canvas: &mut Canvas<'_>,
    x: u16,
    y: i32,
    text: &str,
    width: u16,
    align: Align,
    role: Role,
) {
    let slack = width.saturating_sub(text_width(text));
    let indent = match align {
        Align::Left => 0,
        Align::Center => slack / 2,
        Align::Right => slack,
    };
    canvas.put_truncated(i32::from(x + indent), y, text, width - indent, role);
}

impl View for Table<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let (width, height) = (canvas.width(), canvas.height());
        if width == 0 || height == 0 || self.columns.is_empty() {
            return;
        }

        let gutter = self.gutter();
        let widths = self.widths(width);
        let mut offsets = Vec::with_capacity(widths.len());
        let mut x = gutter;
        for width in &widths {
            offsets.push(x);
            x = x.saturating_add(width.saturating_add(self.gap));
        }

        for index in 0..self.columns.len() {
            place(
                canvas,
                offsets[index],
                0,
                &self.heading(index),
                widths[index],
                self.columns[index].align,
                self.heading_role,
            );
        }

        let body = height.saturating_sub(Self::HEADING_HEIGHT);
        if body == 0 {
            return;
        }
        if self.rows.is_empty() {
            if let Some(message) = &self.empty {
                canvas.put_truncated(
                    i32::from(gutter),
                    1,
                    message,
                    width - gutter.min(width),
                    Role::Muted,
                );
            }
            return;
        }

        let selected = self.selection.map(Selection::selected);
        let window = match self.selection {
            Some(selection) => selection.window(body, self.rows.len()),
            None => 0..usize::from(body).min(self.rows.len()),
        };

        for (row_offset, index) in window.enumerate() {
            let row = &self.rows[index];
            let y = i32::from(Self::HEADING_HEIGHT) + row_offset as i32;
            let is_selected = selected == Some(index);

            if is_selected && gutter > 0 {
                canvas.put(0, y, &self.marker, self.selected_role);
            }
            let role = if is_selected { self.selected_role } else { row.role.unwrap_or(self.role) };
            for (column, cell) in row.cells.iter().enumerate().take(widths.len()) {
                place(
                    canvas,
                    offsets[column],
                    y,
                    cell,
                    widths[column],
                    self.columns[column].align,
                    role,
                );
            }

            // Last, for the reason `List` gives: every write above sets a background of its own.
            if is_selected && self.highlight {
                let surface = canvas.theme().surface;
                let row = Rect::new(0, Self::HEADING_HEIGHT + row_offset as u16, width, 1);
                canvas.style_area(row, Style::new().bg(surface));
            }
        }
    }

    fn constraint(&self, axis: Direction) -> Constraint {
        let _ = axis;
        Constraint::Fill(1)
    }
}

// ---- Scrollbar --------------------------------------------------------------------------

/// A one-column indicator of how much is out of sight, and where you are in it.
///
/// [`Scroll`](crate::view::Scroll) draws one on its own; this is separately public because a
/// [`List`] scrolls itself and may want one too — `Scrollbar::new(selection.offset(), rows.len())`
/// beside it, in a `Row` — and because the numbers are plain enough that nothing here needs to be
/// hidden.
///
/// Draws nothing at all when everything fits. A permanently full-length bar is noise that trains
/// the eye to stop seeing it, and then it fails to say the one thing it is for.
pub struct Scrollbar {
    offset: usize,
    content: usize,
    role: Role,
    track_role: Role,
}

impl Scrollbar {
    /// A bar for a window showing `content` rows in total, starting at row `offset`.
    ///
    /// The height of the window is not a parameter: it is whatever region the bar is given, and
    /// the thumb is sized from that at draw time.
    pub fn new(offset: usize, content: usize) -> Self {
        Self { offset, content, role: Role::Muted, track_role: Role::Dim }
    }

    /// A bar for a viewport, taking its numbers from the last draw.
    ///
    /// What a mouse handler wants: the bar the user is pointing at is the one that was *drawn*, and
    /// a [`Viewport`] remembers the offset and content height it was drawn with.
    ///
    /// ```
    /// use conui::widget::Scrollbar;
    /// use conui::Viewport;
    ///
    /// let viewport = Viewport::new();
    /// viewport.window(10, 40); // Drawn: ten rows of forty.
    /// viewport.bottom();
    /// // Three of the ten rows, pinned to the bottom because that is where the offset is.
    /// assert_eq!(Scrollbar::of(&viewport).thumb(10), Some(7..10));
    /// ```
    pub fn of(viewport: &Viewport) -> Self {
        Self::new(usize::from(viewport.offset()), usize::from(viewport.content()))
    }

    /// The thumb's colour.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// The track's colour, behind and either side of the thumb.
    pub fn track_role(mut self, role: Role) -> Self {
        self.track_role = role;
        self
    }

    /// The rows the thumb covers in a bar `height` rows tall. `None` when it all fits.
    ///
    /// Two properties matter more than proportionality: the thumb is never shorter than one row, so
    /// a long document does not lose it entirely, and it touches the top only at the top and the
    /// bottom only at the bottom — a bar that looks finished with two rows left to read is a lie.
    ///
    /// Public because a bar you can drag needs to know where its own thumb is, and answering that
    /// is the widget's business rather than the caller's: `thumb(height).contains(&row)` is how a
    /// press decides whether it grabbed the thumb or hit the track beside it.
    pub fn thumb(&self, height: u16) -> Option<Range<u16>> {
        self.thumb_at(self.offset, height)
    }

    /// Which content offset draws the thumb with its top at `top`. The inverse of [`Scrollbar::thumb`].
    ///
    /// What dragging the thumb means: the pointer names a row, and the content has to follow. The
    /// answer is found by *asking* [`Scrollbar::thumb`] — a binary search over offsets, since the
    /// thumb only ever moves down as the offset grows — rather than by inverting its arithmetic in a
    /// second place, which is how a thumb comes to jump out from under the pointer as you drag it.
    ///
    /// The lowest offset that reaches `top`, so the guarantee runs one way: the offset this returns
    /// draws the thumb exactly where it was asked for. Going the other way cannot be promised, and
    /// nor should it be — a track forty rows long has more offsets than places to put them.
    pub fn offset_at(&self, top: u16, height: u16) -> usize {
        let furthest = self.content.saturating_sub(usize::from(height));
        let (mut low, mut high) = (0usize, furthest);
        while low < high {
            let middle = low + (high - low) / 2;
            match self.thumb_at(middle, height) {
                Some(thumb) if thumb.start < top => low = middle + 1,
                // Nothing overflows, so there is nowhere to scroll to and no bar to have dragged.
                None => return 0,
                Some(_) => high = middle,
            }
        }
        low
    }

    /// [`Scrollbar::thumb`] for an offset other than this bar's own, which the search above needs.
    fn thumb_at(&self, offset: usize, height: u16) -> Option<Range<u16>> {
        if height == 0 || self.content <= usize::from(height) {
            return None;
        }
        let (rows, content) = (usize::from(height), self.content);
        let length = ((rows * rows + content / 2) / content).clamp(1, rows);
        let travel = rows - length;
        let furthest = content - rows;
        let offset = offset.min(furthest);
        // Interior offsets are mapped to interior positions rather than rounded to the nearest
        // row, which is what keeps both ends honest: any rounding at all would let a thumb touch
        // the bottom with a row still to read, and then the bar is worse than nothing.
        let top = match (offset, travel) {
            (0, _) => 0,
            _ if offset >= furthest => travel,
            // Two positions and more than two offsets: there is nowhere in between to put it, so
            // the end that must not lie is the one it stays away from.
            (_, 0 | 1) => 0,
            // `max(1)` for the one interior offset of `furthest == 2`, which has no span to
            // divide by and belongs at the first interior row regardless.
            _ => 1 + (offset - 1) * (travel - 2) / (furthest - 2).max(1),
        };
        Some(top as u16..(top + length) as u16)
    }
}

impl View for Scrollbar {
    /// One column, and as tall as it is given: the bar is the height of whatever it is beside.
    fn constraint(&self, axis: Direction) -> Constraint {
        match axis {
            Direction::Horizontal => Constraint::Length(1),
            Direction::Vertical => Constraint::Fill(1),
        }
    }

    fn render(&self, canvas: &mut Canvas<'_>) {
        let height = canvas.height();
        let Some(thumb) = self.thumb(height) else { return };
        for row in 0..i32::from(height) {
            canvas.put(0, row, &line::VERTICAL.to_string(), self.track_role);
        }
        for row in thumb {
            canvas.put(0, i32::from(row), "█", self.role);
        }
    }
}

// ---- Input ------------------------------------------------------------------------------

/// A single-line text field over an [`Editor`] you own.
///
/// Scrolls horizontally to keep the cursor in view, and draws the cursor itself as a reversed
/// cell rather than asking for the terminal's — so a field nested three layers deep in a layout
/// needs no cooperation from the code that placed it. If you would rather have the real cursor,
/// switch this one off with [`Input::no_cursor`] and call [`Frame::set_cursor`].
///
/// [`Frame::set_cursor`]: crate::Frame::set_cursor
pub struct Input<'a> {
    editor: &'a Editor,
    prompt: Option<String>,
    prompt_role: Role,
    role: Role,
    placeholder: Option<String>,
    placeholder_role: Role,
    cursor: bool,
}

impl<'a> Input<'a> {
    /// A field showing `editor`'s text and cursor.
    ///
    /// The editor is borrowed, not owned: it holds the text and the caret, it survives between
    /// frames, and your key handler is what moves it. This view only draws what it finds there.
    pub fn new(editor: &'a Editor) -> Self {
        Self {
            editor,
            prompt: None,
            prompt_role: Role::Accent,
            role: Role::Text,
            placeholder: None,
            placeholder_role: Role::Dim,
            cursor: true,
        }
    }

    /// A label or sigil before the field. One column of space is left after it.
    pub fn prompt(mut self, prompt: impl Into<String>) -> Self {
        self.prompt = Some(prompt.into());
        self
    }

    /// Colour of the prompt, separately from the text being typed.
    pub fn prompt_role(mut self, role: Role) -> Self {
        self.prompt_role = role;
        self
    }

    /// Colour of the text in the field. The placeholder has its own.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// What to show while the field is empty.
    pub fn placeholder(mut self, text: impl Into<String>) -> Self {
        self.placeholder = Some(text.into());
        self
    }

    /// Stop drawing the block cursor — for an unfocused field, or when you are placing the
    /// terminal's own cursor instead.
    pub fn no_cursor(mut self) -> Self {
        self.cursor = false;
        self
    }
}

impl View for Input<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let width = canvas.width();
        if width == 0 {
            return;
        }
        let mut x = 0u16;
        if let Some(prompt) = &self.prompt {
            x = canvas.put_truncated(0, 0, prompt, width, self.prompt_role) + 1;
        }
        let field = width.saturating_sub(x);
        if field == 0 {
            return;
        }

        // Scrolled only as far as needed to keep the cursor inside the field. The rule belongs to
        // the editor, not to this widget, so that a click can be resolved against the same one.
        let (shown, hidden) = self.editor.view_from(field);
        let cursor_column = self.editor.cursor_column();

        if self.editor.is_empty() {
            if let Some(placeholder) = &self.placeholder {
                canvas.put_truncated(i32::from(x), 0, placeholder, field, self.placeholder_role);
            }
        } else {
            // `put`, not `put_truncated`: an overlong value has scrolled out of view, and an
            // ellipsis would claim text was dropped when it is merely off to the left.
            canvas.put(i32::from(x), 0, shown, self.role);
        }

        if self.cursor {
            let column = x + cursor_column.saturating_sub(hidden);
            if column < width {
                canvas.style_area(Rect::new(column, 0, 1, 1), Style::new().reverse());
            }
        }
    }

    fn constraint(&self, axis: Direction) -> Constraint {
        one_row(axis)
    }
}

// ---- Button -----------------------------------------------------------------------------

/// A label you can activate, drawn as `‹ LABEL ›`.
///
/// It has no click handler and no callback. A button in an immediate-mode tree is a *picture* of
/// an action: you tell it whether it is focused, and when your key handler sees `Enter` on that
/// focus id, you run the action yourself. That sounds like less, and it is — there is no question
/// of what runs when, no borrow of your state trapped inside a closure, and the action is a plain
/// method you can also bind to a shortcut or call from a test.
///
/// ```
/// use conui::widget::Button;
///
/// let save = Button::new("Save").focused(true).accent();
/// assert_eq!(Button::width("Save"), 8); // "‹ Save ›"
/// # let _ = save;
/// ```
pub struct Button {
    label: String,
    focused: bool,
    enabled: bool,
    role: Role,
}

impl Button {
    /// An enabled, unfocused button reading `‹ label ›`.
    pub fn new(label: impl Into<String>) -> Self {
        Self { label: label.into(), focused: false, enabled: true, role: Role::Text }
    }

    /// Whether this button currently has focus — `focus.is(Id::Save)`, usually.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// The button's own colour: accent for the default action, danger for a destructive one.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// The action the screen is built around.
    pub fn accent(self) -> Self {
        self.role(Role::Accent)
    }

    /// An action that destroys something.
    pub fn danger(self) -> Self {
        self.role(Role::Danger)
    }

    /// Draw it as unavailable. Keep it out of the focus ring too — greying a control that still
    /// takes `Enter` is worse than not greying it at all.
    pub fn disabled(mut self) -> Self {
        self.enabled = false;
        self
    }

    /// Columns `label` needs, including the brackets and their spaces.
    ///
    /// Sizing a button's slot means knowing this, and `label.len() + 4` is wrong the moment the
    /// label is not ASCII.
    pub fn width(label: &str) -> u16 {
        text_width(label).saturating_add(4)
    }
}

impl View for Button {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let region = canvas.width();
        if region == 0 || canvas.height() == 0 {
            return;
        }
        let wanted = Self::width(&self.label);
        let width = wanted.min(region);
        // Centred in whatever it was given, so a row of buttons of different widths still reads
        // as a row rather than as ragged text.
        let x = i32::from((region - width) / 2);

        let (frame_role, label_role) = match (self.enabled, self.focused) {
            (false, _) => (Role::Dim, Role::Dim),
            (true, false) => (Role::Dim, self.role),
            (true, true) => (self.role, self.role),
        };

        canvas.set(x, 0, mark::BUTTON_LEFT, frame_role);
        let inner = width.saturating_sub(4);
        canvas.put_truncated(x + 2, 0, &self.label, inner, label_role);
        canvas.set(x + i32::from(width) - 1, 0, mark::BUTTON_RIGHT, frame_role);

        // Last, for the same reason the list highlight is last: every write above carries its own
        // background, so patching afterwards is the only order the lift survives.
        if self.focused && self.enabled {
            let surface = canvas.theme().surface;
            canvas
                .style_area(Rect::new((region - width) / 2, 0, width, 1), Style::new().bg(surface));
        }
    }

    /// One row, and the width of its label plus its brackets — a button is the one control here
    /// with an intrinsic width, which is why `Button::width` existed before this method could ask
    /// for it.
    fn constraint(&self, axis: Direction) -> Constraint {
        match axis {
            Direction::Vertical => Constraint::Length(1),
            Direction::Horizontal => Constraint::Length(Self::width(&self.label)),
        }
    }
}

// ---- Buttons ----------------------------------------------------------------------------

/// A row of [`Button`]s with one of them focused: `‹ Repair ›  ‹ Cancel ›`.
///
/// Which one is focused is a [`Selection`] you own, so moving along the row is
/// `selection.cycle_down(len)` for `→` and `cycle_up` for `←` — the same calls a list uses, because
/// it is the same question. Wrapping is usually right here: a choice of two or three is a short
/// menu, and running off the end of one and stopping feels broken.
///
/// The point of the widget is that a question can be answered two ways at once. The arrow keys and
/// `Enter` walk the row for somebody who is reading it, and [`Buttons::index_for`] turns a typed
/// letter straight into an answer for somebody who already knows what they want — the same
/// `y`/`n` that worked before the row existed. Both end in the same `usize`, so the code that acts
/// on the answer is written once.
///
/// ```
/// use conui::state::Selection;
/// use conui::widget::Buttons;
///
/// let choice = Selection::new();
/// let buttons = Buttons::new(["Yes", "No"]).selection(&choice);
/// assert_eq!(buttons.index_for('n'), Some(1)); // typed straight in
/// assert_eq!(choice.selected(), 0);            // or walked to with the arrows
/// assert_eq!(buttons.width(), 7 + 2 + 6);      // "‹ Yes ›", a gap, "‹ No ›"
/// ```
pub struct Buttons<'a> {
    labels: Vec<String>,
    selection: Option<&'a Selection>,
    focused: bool,
    gap: u16,
    role: Role,
    roles: Vec<Role>,
    align: Align,
}

impl<'a> Buttons<'a> {
    /// A row of buttons reading `labels`, none of them focused until a
    /// [`selection`](Self::selection) says which.
    pub fn new<S: Into<String>>(labels: impl IntoIterator<Item = S>) -> Self {
        Self {
            labels: labels.into_iter().map(Into::into).collect(),
            selection: None,
            focused: true,
            gap: 2,
            role: Role::Text,
            roles: Vec::new(),
            align: Align::Left,
        }
    }

    /// Which button the keyboard is on, as an index into the labels.
    pub fn selection(mut self, selection: &'a Selection) -> Self {
        self.selection = Some(selection);
        self
    }

    /// Whether the row is what the arrow keys are talking to. True by default, unlike every other
    /// focusable widget here: a row of buttons is put on screen to be answered, and the case where
    /// it is one control among several is the rarer one. Say `.focused(false)` for that case, and
    /// the row keeps showing which button is current without claiming the keystrokes.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Columns between one button and the next. Two by default; the gap belongs to neither
    /// neighbour, which is what lets [`index_at`](Self::index_at) return `None` for a click in it.
    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    /// Colour of every button that has not been given one of its own.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// A colour per button, index-matched to the labels: danger for the one that deletes something,
    /// accent for the one the dialog is built around. Buttons past the end of this list fall back to
    /// the shared [`role`](Self::role).
    pub fn roles(mut self, roles: impl IntoIterator<Item = Role>) -> Self {
        self.roles = roles.into_iter().collect();
        self
    }

    /// Centre the row in whatever width it is given, which is what a dialog wants.
    pub fn centered(mut self) -> Self {
        self.align = Align::Center;
        self
    }

    /// Push the row against the right edge, where a form's buttons usually sit.
    pub fn right(mut self) -> Self {
        self.align = Align::Right;
        self
    }

    /// How many buttons there are — the argument
    /// [`Selection::cycle_down`](crate::state::Selection::cycle_down) wants.
    pub fn len(&self) -> usize {
        self.labels.len()
    }

    /// Whether there are no buttons, in which case the row draws nothing.
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    /// Columns every button and gap needs, for sizing or centring the row.
    pub fn width(&self) -> u16 {
        let buttons: u16 =
            self.labels.iter().map(|label| Button::width(label)).fold(0, u16::saturating_add);
        let gaps = self.gap.saturating_mul(self.labels.len().saturating_sub(1) as u16);
        buttons.saturating_add(gaps)
    }

    /// Which button a letter answers: the first whose label starts with it, ignoring case.
    ///
    /// `Yes`/`No` gives you `y` and `n` without stating them anywhere, and `Enter` on the focused
    /// button gives the same index. Labels that share an initial are a UI problem rather than a
    /// programming one — the earlier button wins, and the later one is only reachable with the
    /// arrows.
    pub fn index_for(&self, key: char) -> Option<usize> {
        let key = key.to_lowercase().next()?;
        self.labels.iter().position(|label| {
            label.chars().next().is_some_and(|initial| {
                initial.to_lowercase().next().is_some_and(|initial| initial == key)
            })
        })
    }

    /// Which button was clicked, given the region the row drew into.
    ///
    /// Takes the region rather than a bare column because the row may be centred in it, and a
    /// centred row's buttons are nowhere near the coordinates its labels would suggest. `None` for a
    /// click in a gap, outside the row, or on a row that has no buttons: a click between two buttons
    /// should do nothing rather than guess which one was meant.
    pub fn index_at(&self, area: Rect, pos: Pos) -> Option<usize> {
        if !area.contains(pos) {
            return None;
        }
        let x = pos.x - area.x;
        self.spans(area.width)
            .into_iter()
            .find(|&(_, start, width)| x >= start && x < start.saturating_add(width))
            .map(|(index, _, _)| index)
    }

    /// Where each button sits within a region `region` wide: its index, its first column and its
    /// width.
    ///
    /// One helper for both drawing and hit testing, so a click can never land somewhere other than
    /// what it looks like it landed on. Buttons that do not fit are left out entirely rather than
    /// half-drawn, since half a bracket reads as a broken program.
    fn spans(&self, region: u16) -> Vec<(usize, u16, u16)> {
        let total = self.width();
        let slack = region.saturating_sub(total);
        let mut x = match self.align {
            Align::Left => 0,
            Align::Center => slack / 2,
            Align::Right => slack,
        };
        let mut spans = Vec::with_capacity(self.labels.len());
        for (index, label) in self.labels.iter().enumerate() {
            let width = Button::width(label);
            if x.saturating_add(width) > region {
                break;
            }
            spans.push((index, x, width));
            x = x.saturating_add(width).saturating_add(self.gap);
        }
        spans
    }
}

impl View for Buttons<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let width = canvas.width();
        if width == 0 || canvas.height() == 0 {
            return;
        }
        let selected = self.selection.map(Selection::selected);
        for (index, x, span) in self.spans(width) {
            let current = selected == Some(index);
            // An unfocused row quietens everything except the button `Enter` would hit if focus came
            // back, which keeps the answer-in-progress visible without claiming the keystrokes.
            let role = match (self.focused, current) {
                (true, _) | (false, true) => self.roles.get(index).copied().unwrap_or(self.role),
                (false, false) => Role::Dim,
            };
            let button = Button::new(self.labels[index].as_str())
                .role(role)
                .focused(self.focused && current);
            let mut cell = canvas.sub(Rect::new(x, 0, span, 1));
            button.render(&mut cell);
        }
    }

    /// One row, and the width of every button and gap.
    fn constraint(&self, axis: Direction) -> Constraint {
        match axis {
            Direction::Vertical => Constraint::Length(1),
            Direction::Horizontal => Constraint::Length(self.width()),
        }
    }
}

// ---- Tabs -------------------------------------------------------------------------------

/// A row of labels with the current one underlined.
///
/// Which tab is current is a [`Selection`] you own, so moving between tabs is
/// `selection.cycle_down(len)` — the same call a list uses, because it is the same question.
pub struct Tabs<'a> {
    labels: Vec<String>,
    selection: Option<&'a Selection>,
    focused: bool,
    gap: u16,
    underline: bool,
    role: Role,
    selected_role: Role,
}

impl<'a> Tabs<'a> {
    /// A bar of `labels`, with none of them marked current until a
    /// [`selection`](Self::selection) says which.
    pub fn new<S: Into<String>>(labels: impl IntoIterator<Item = S>) -> Self {
        Self {
            labels: labels.into_iter().map(Into::into).collect(),
            selection: None,
            focused: false,
            gap: 3,
            underline: true,
            role: Role::Muted,
            selected_role: Role::Accent,
        }
    }

    /// Which tab is current, as an index into the labels.
    ///
    /// The same [`Selection`] a list uses, because "which of these N" is the same question whether
    /// the N are stacked or in a row.
    pub fn selection(mut self, selection: &'a Selection) -> Self {
        self.selection = Some(selection);
        self
    }

    /// Whether the tab bar is the thing the arrow keys are talking to.
    ///
    /// An unfocused bar still shows which tab you are on — it just stops claiming to be where
    /// your keystrokes are going, which is the whole job of a focus ring.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Columns between labels.
    /// Columns between one label and the next. Three by default; the gap belongs to neither
    /// neighbour, which is what lets [`index_at`](Self::index_at) return `None` for a click in it.
    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    /// Drop the underline row, making this one row tall.
    pub fn no_underline(mut self) -> Self {
        self.underline = false;
        self
    }

    /// Colour of the tabs you are not on.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Colour of the current tab and its underline — but only while the bar is
    /// [`focused`](Self::focused). Unfocused, the current tab falls back to [`Role::Text`] so the
    /// bar says where you are without claiming the keystrokes.
    pub fn selected_role(mut self, role: Role) -> Self {
        self.selected_role = role;
        self
    }

    /// How many tabs there are — the argument
    /// [`Selection::cycle_down`](crate::state::Selection::cycle_down) wants.
    pub fn len(&self) -> usize {
        self.labels.len()
    }

    /// Whether there are no tabs, in which case the bar draws nothing.
    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    /// Columns every label and gap needs, for sizing or centring the bar.
    pub fn width(&self) -> u16 {
        let labels: u16 = self.labels.iter().map(|label| text_width(label)).sum();
        let gaps = self.gap.saturating_mul(self.labels.len().saturating_sub(1) as u16);
        labels.saturating_add(gaps)
    }

    /// Which tab is at column `x`, measured from the bar's own left edge.
    ///
    /// The gap between two labels belongs to neither, so a click that lands in it selects nothing
    /// rather than guessing. Build the bar the same way you render it — one helper both call sites
    /// use — or this and the pixels will disagree the first time a gap changes.
    pub fn index_at(&self, x: u16) -> Option<usize> {
        let mut start = 0u16;
        for (index, label) in self.labels.iter().enumerate() {
            let end = start.saturating_add(text_width(label));
            if x < end {
                return (x >= start).then_some(index);
            }
            start = end.saturating_add(self.gap);
        }
        None
    }
}

impl View for Tabs<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let width = canvas.width();
        if width == 0 || canvas.height() == 0 || self.labels.is_empty() {
            return;
        }
        let selected = self.selection.map(Selection::selected);

        let mut x = 0u16;
        for (index, label) in self.labels.iter().enumerate() {
            if x >= width {
                break;
            }
            let is_selected = selected == Some(index);
            let role = match (is_selected, self.focused) {
                // A selected tab on an unfocused bar keeps its place without claiming the keys.
                (true, false) => Role::Text,
                (true, true) => self.selected_role,
                (false, _) => self.role,
            };
            let drawn = canvas.put_truncated(i32::from(x), 0, label, width - x, role);
            if is_selected && self.underline && canvas.height() > 1 {
                let rule = if self.focused { self.selected_role } else { Role::Dim };
                canvas.run(i32::from(x), 1, line::HORIZONTAL, drawn, rule);
            }
            x = x.saturating_add(drawn).saturating_add(self.gap);
        }
    }

    /// Two rows with the underline, one without, and the width of every label and gap.
    fn constraint(&self, axis: Direction) -> Constraint {
        match axis {
            Direction::Vertical => Constraint::Length(if self.underline { 2 } else { 1 }),
            Direction::Horizontal => Constraint::Length(self.width()),
        }
    }
}

// ---- Select -----------------------------------------------------------------------------

/// The closed field of a dropdown: the current choice, bracketed, with a caret.
///
/// Rendering this also records where it landed, which is how [`Dropdown::popup_area`] knows where
/// to put the open list. Draw the field first, the list second — see [`Dropdown`] for the two-pass
/// shape and why it is two passes.
pub struct Select<'a> {
    dropdown: &'a Dropdown,
    options: Vec<String>,
    focused: bool,
    role: Role,
    empty: String,
}

impl<'a> Select<'a> {
    /// A closed field showing whichever of `options` the dropdown has selected.
    ///
    /// The same `options` have to be handed to the [`Menu`] that draws the open list, since the
    /// dropdown holds only an index into them.
    pub fn new<S: Into<String>>(
        dropdown: &'a Dropdown,
        options: impl IntoIterator<Item = S>,
    ) -> Self {
        Self {
            dropdown,
            options: options.into_iter().map(Into::into).collect(),
            focused: false,
            role: Role::Text,
            empty: String::from("—"),
        }
    }

    /// Whether this field is where the keystrokes are going. An open dropdown is drawn as focused
    /// whatever this says, because a list nobody is talking to has no business being open.
    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

    /// Colour of the chosen value while the list is closed.
    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// What to show when there is nothing to choose from.
    pub fn empty(mut self, text: impl Into<String>) -> Self {
        self.empty = text.into();
        self
    }

    /// Columns the widest choice needs, brackets and caret included — so a column of selects can
    /// be sized to its contents rather than guessed at.
    pub fn width(options: &[impl AsRef<str>]) -> u16 {
        let widest = options.iter().map(|option| text_width(option.as_ref())).max().unwrap_or(0);
        widest.saturating_add(6)
    }
}

impl View for Select<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let width = canvas.width();
        if width == 0 || canvas.height() == 0 {
            return;
        }
        // Record the whole field, not just the text, so the list lines up with the brackets — and
        // only the part of it that is on screen, because this rect is also what decides whether a
        // click landed on the field. A select scrolled out of its pane records nothing and cannot
        // be clicked; asking a one-row sub-canvas where it ended up is what makes that exact, since
        // a region above the fold has a negative origin and no `Rect` can hold one.
        let field = canvas.sub(Rect::new(0, 0, width, 1)).visible_area();
        self.dropdown.set_field(field);

        let open = self.dropdown.is_open();
        let frame_role = if self.focused || open { Role::Accent } else { Role::Dim };
        canvas.set(0, 0, '[', frame_role);
        canvas.set(i32::from(width) - 1, 0, ']', frame_role);

        let caret = if open { mark::CARET_UP } else { mark::CARET_DOWN };
        canvas.set(i32::from(width) - 3, 0, caret, frame_role);

        let value = self.options.get(self.dropdown.selected()).map_or(self.empty.as_str(), |s| s);
        let room = width.saturating_sub(6);
        let role = if open { Role::Accent } else { self.role };
        canvas.put_truncated(2, 0, value, room, role);

        if self.focused && !open {
            let surface = canvas.theme().surface;
            canvas.style_area(Rect::new(0, 0, width, 1), Style::new().bg(surface));
        }
    }

    /// One row, and wide enough for the widest choice it could have to show.
    fn constraint(&self, axis: Direction) -> Constraint {
        match axis {
            Direction::Vertical => Constraint::Length(1),
            Direction::Horizontal => Constraint::Length(Self::width(&self.options)),
        }
    }
}

// ---- Menu -------------------------------------------------------------------------------

/// A bordered list of choices, opaque, for drawing over the top of a frame.
///
/// This is what a dropdown's open list is, and it is a plain view: nothing about it knows it is an
/// overlay. It clears its region before drawing — the one thing an overlay must do that an
/// ordinary view must not — and it is on top because you drew it last.
pub struct Menu<'a> {
    selection: &'a Selection,
    options: Vec<String>,
    title: Option<String>,
    border_role: Role,
}

impl<'a> Menu<'a> {
    /// A bordered list of `options` with `selection`'s row highlighted.
    ///
    /// For a dropdown's open list this is [`Dropdown::selection`], which is also the choice: the
    /// dropdown moves the real value as you browse and puts it back on
    /// [`dismiss`](Dropdown::dismiss), so there is only ever one index to draw.
    ///
    /// [`Dropdown::selection`]: crate::state::Dropdown::selection
    /// [`Dropdown::dismiss`]: crate::state::Dropdown::dismiss
    pub fn new<S: Into<String>>(
        selection: &'a Selection,
        options: impl IntoIterator<Item = S>,
    ) -> Self {
        Self {
            selection,
            options: options.into_iter().map(Into::into).collect(),
            title: None,
            border_role: Role::Accent,
        }
    }

    /// A label set into the top border, costing no row of its own but widening the menu if it is
    /// longer than the choices under it.
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Colour of the frame, and of the `↑`/`↓` marks on it that say there are choices out of sight.
    pub fn border_role(mut self, role: Role) -> Self {
        self.border_role = role;
        self
    }
}

impl View for Menu<'_> {
    fn render(&self, canvas: &mut Canvas<'_>) {
        let area = canvas.area();
        if area.width < 2 || area.height < 2 {
            return;
        }
        // An overlay that lets the frame beneath show through is not an overlay.
        canvas.clear();
        canvas.border(area, self.border_role);
        if let Some(title) = &self.title {
            let room = area.width.saturating_sub(4);
            canvas.put_truncated(2, 0, title, room, Role::Muted);
        }

        let mut inner = canvas.inset(Padding::all(1));
        let list = List::new(self.options.iter().map(String::as_str))
            .selection(self.selection)
            .highlight();
        list.render(&mut inner);

        // More choices than rows: say so on the frame, or the list looks like the whole of it.
        let rows = area.height.saturating_sub(2);
        let window = self.selection.window(rows, self.options.len());
        let right = i32::from(area.width) - 1;
        if window.start > 0 {
            canvas.set(right, 0, mark::ARROW_UP, self.border_role);
        }
        if window.end < self.options.len() {
            canvas.set(right, i32::from(area.height) - 1, mark::ARROW_DOWN, self.border_role);
        }
    }

    /// Every option, plus the border it draws around them, on both axes.
    fn constraint(&self, axis: Direction) -> Constraint {
        let border = 2;
        match axis {
            Direction::Vertical => Constraint::Length(
                u16::try_from(self.options.len()).unwrap_or(u16::MAX).saturating_add(border),
            ),
            Direction::Horizontal => {
                let widest =
                    self.options.iter().map(|option| text_width(option)).max().unwrap_or(0);
                // A title sits in the top border between two cells of frame and a space either
                // side, so it can need more room than the list under it.
                let titled =
                    self.title.as_deref().map_or(0, |title| text_width(title).saturating_add(2));
                Constraint::Length(widest.max(titled).saturating_add(border))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Theme;
    use conui_cell::Buffer;

    fn rows(view: &dyn View, width: u16, height: u16) -> Vec<String> {
        let mut buffer = Buffer::new(width, height);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        view.render(&mut canvas);
        (0..height).map(|row| buffer.row_text(row)).collect()
    }

    fn row(view: &dyn View, width: u16) -> String {
        rows(view, width, 1).remove(0)
    }

    // ---- Text ---------------------------------------------------------------------------

    #[test]
    fn text_draws_at_the_origin_and_asks_for_one_row() {
        let text = Text::new("hello");
        assert_eq!(row(&text, 8), "hello   ");
        assert_eq!(text.constraint(Direction::Vertical), Constraint::Length(1));
    }

    #[test]
    fn text_alignment_places_the_line_within_the_region() {
        assert_eq!(row(&Text::new("ab").centered(), 6), "  ab  ");
        assert_eq!(row(&Text::new("ab").right(), 6), "    ab");
    }

    #[test]
    fn embedded_newlines_become_rows_and_are_counted() {
        let text = Text::new("one\ntwo");
        assert_eq!(text.constraint(Direction::Vertical), Constraint::Length(2));
        assert_eq!(rows(&text, 4, 2), ["one ", "two "]);
    }

    #[test]
    fn unwrapped_text_is_clipped_rather_than_reflowed() {
        // Clipping keeps the layout stable; reflowing would push everything below it down.
        assert_eq!(row(&Text::new("a long sentence"), 6), "a long");
        assert_eq!(
            Text::new("a long sentence").constraint(Direction::Vertical),
            Constraint::Length(1)
        );
    }

    #[test]
    fn wrapped_text_breaks_on_word_boundaries() {
        let text = Text::new("the quick brown fox").wrapped();
        assert_eq!(rows(&text, 10, 3), ["the quick ", "brown fox ", "          "]);
        assert_eq!(text.constraint(Direction::Vertical), Constraint::Fill(1));
    }

    #[test]
    fn wrapping_hard_breaks_a_word_too_long_to_fit() {
        assert_eq!(wrap("antidisestablishmentarianism", 10).len(), 3);
        assert_eq!(wrap("antidis", 10), ["antidis"]);
    }

    #[test]
    fn wrapping_a_zero_width_region_yields_nothing_rather_than_looping() {
        assert!(wrap("anything at all", 0).is_empty());
    }

    // ---- Rule ---------------------------------------------------------------------------

    #[test]
    fn a_rule_spans_its_region() {
        assert_eq!(row(&Rule::new(), 6), "──────");
        assert_eq!(row(&Rule::new().heavy(), 4), "━━━━");
    }

    #[test]
    fn a_titled_rule_sets_its_label_into_the_line() {
        assert_eq!(row(&Rule::titled("AB"), 10), "─ AB ─────");
    }

    // ---- Gauge --------------------------------------------------------------------------

    #[test]
    fn a_gauge_fills_the_space_between_its_label_and_its_readout() {
        let gauge = Gauge::new(0.5).label("RISK");
        assert_eq!(row(&gauge, 20), "RISK ━━━━━━━━━━ 0.50");
        assert_eq!(gauge.constraint(Direction::Vertical), Constraint::Length(1));
    }

    #[test]
    fn a_gauge_without_a_label_or_readout_is_all_bar() {
        let gauge = Gauge::new(1.0).readout(Readout::None).style(BarStyle::Blocks);
        assert_eq!(row(&gauge, 6), "██████");
    }

    #[test]
    fn a_percent_readout_reads_as_a_percentage() {
        assert_eq!(row(&Gauge::new(0.42).readout(Readout::Percent), 12), "━━━━━━━━ 42%");
    }

    #[test]
    fn a_custom_readout_can_carry_a_unit() {
        let gauge = Gauge::new(0.3).readout(Readout::Text("12.4 ms".into()));
        assert!(row(&gauge, 20).ends_with("12.4 ms"));
    }

    #[test]
    fn a_fixed_label_width_aligns_a_column_of_gauges() {
        let short = Gauge::new(0.5).label("UP").label_width(6);
        let long = Gauge::new(0.5).label("RIGHT").label_width(6);
        let (a, b) = (row(&short, 24), row(&long, 24));
        // The bars must start at the same column despite the labels differing in length.
        assert_eq!(a.find('━'), b.find('━'));
    }

    #[test]
    fn selecting_a_gauge_adds_a_marker_without_moving_its_bar() {
        let gauge = Gauge::new(0.5).label("UP").label_width(6).selected(true);
        let line = row(&gauge, 24);
        assert!(line.starts_with('›'), "got {line:?}");
    }

    #[test]
    fn a_gauge_in_a_region_too_small_for_its_parts_does_not_panic() {
        for width in 0..12u16 {
            let _ = row(&Gauge::new(0.5).label("LONG LABEL"), width);
        }
    }

    // ---- Progress -----------------------------------------------------------------------

    #[test]
    fn a_progress_bar_prints_the_count_and_the_percentage() {
        let progress = Progress::new(1, 4).label("SCAN");
        assert_eq!(row(&progress, 22), "SCAN █░░░░░░ 1/4 · 25%");
        assert_eq!(progress.constraint(Direction::Vertical), Constraint::Length(1));
    }

    #[test]
    fn a_bar_fills_by_truncation_so_it_is_only_full_when_the_job_is() {
        // Nine of ten items into a ten-column bar is nine columns, and the tenth arrives with the
        // tenth item. A rounded fill would have shown ten for both, which is the one lie a
        // progress bar must not tell.
        let nearly = Progress::new(9, 10).tally(Tally::None).style(BarStyle::Blocks);
        assert_eq!(row(&nearly, 10), "█████████ ");
        let done = Progress::new(10, 10).tally(Tally::None).style(BarStyle::Blocks);
        assert_eq!(row(&done, 10), "██████████");
        assert!(done.is_complete() && !nearly.is_complete());
    }

    #[test]
    fn a_percentage_never_reads_as_finished_before_the_last_item() {
        assert_eq!(percent(99, 100), 99);
        assert_eq!(percent(999, 1000), 99, "truncated, not rounded to 100");
        assert_eq!(percent(1000, 1000), 100);
        assert_eq!(percent(7, 0), 100, "no work is work done");
        assert_eq!(percent(11, 10), 100, "a miscount cannot exceed the whole");
    }

    #[test]
    fn a_job_of_nothing_is_a_full_bar_rather_than_a_division_by_zero() {
        let empty = Progress::new(0, 0).style(BarStyle::Blocks);
        assert_eq!(row(&empty, 17), "██████ 0/0 · 100%");
        assert_eq!(empty.fraction(), 1.0);
        assert!(empty.is_complete());
    }

    #[test]
    fn a_smooth_bar_moves_on_items_too_small_to_fill_a_column() {
        // A fifth of a column: the eighth-cell glyph is the only thing that can show it.
        let progress = Progress::new(1, 40).tally(Tally::None).style(BarStyle::Smooth);
        assert_eq!(row(&progress, 8), "▏       ");
    }

    #[test]
    fn an_indeterminate_bar_marches_and_counts_without_a_percentage() {
        let scanning = |tick| row(&Progress::indeterminate(tick).done(312), 20);
        assert!(scanning(0).ends_with(" 312"), "no percentage without a total: {:?}", scanning(0));
        // The block moves, and comes back: bouncing, so it never reads as two blocks at a seam.
        let positions: Vec<_> = (0..27).map(|tick| scanning(tick).find('█')).collect();
        assert!(positions.windows(2).all(|pair| pair[0] != pair[1]), "{positions:?}");
        assert_eq!(positions[0], positions[26], "a period of 26 for a bar of 16 columns");
        assert!(Progress::indeterminate(0).is_indeterminate());
        assert!(!Progress::indeterminate(0).done(9).is_complete());
    }

    #[test]
    fn a_caption_takes_a_second_row_under_the_whole_bar() {
        let progress = Progress::new(2, 4).label("FIX").caption("chatSessions/3f2c.json");
        assert_eq!(progress.constraint(Direction::Vertical), Constraint::Length(2));
        assert_eq!(rows(&progress, 22, 2), ["FIX ████░░░░ 2/4 · 50%", "chatSessions/3f2c.json"]);
    }

    #[test]
    fn a_tally_can_be_the_count_alone_or_a_string_of_your_own() {
        assert_eq!(Tally::Count.render(3, Some(9)).unwrap(), "3/9");
        assert_eq!(Tally::Percent.render(3, Some(9)).unwrap(), "33%");
        assert_eq!(Tally::Percent.render(3, None), None, "nothing to be a percentage of");
        assert_eq!(Tally::Count.render(3, None).unwrap(), "3");
        assert_eq!(Tally::Text("2m left".into()).render(3, Some(9)).unwrap(), "2m left");
        assert_eq!(Tally::None.render(3, Some(9)), None);
    }

    #[test]
    fn a_progress_bar_in_a_region_too_small_for_its_parts_does_not_panic() {
        for width in 0..14u16 {
            let _ = row(&Progress::new(3, 7).label("SCANNING"), width);
            let _ = row(&Progress::indeterminate(5).label("SCANNING"), width);
        }
    }

    // ---- Stat ---------------------------------------------------------------------------

    #[test]
    fn a_stat_puts_its_label_above_block_digits() {
        let stat = Stat::new("SCORE", 42);
        assert_eq!(stat.constraint(Direction::Vertical), Constraint::Length(4));
        let drawn = rows(&stat, 12, 4);
        assert_eq!(drawn[0], "SCORE       ");
        assert_eq!(drawn[1], "█▀█ █ █ ▀▀█ ");
    }

    #[test]
    fn a_stat_reports_a_width_that_covers_both_its_parts() {
        assert_eq!(Stat::new("BEST", 5).width(), 11, "three digits at four columns each");
        assert_eq!(Stat::new("A VERY LONG LABEL", 5).width(), 17);
    }

    // ---- Sparkline ----------------------------------------------------------------------

    #[test]
    fn a_sparkline_scales_to_its_fixed_maximum() {
        let spark = Sparkline::new([0.0, 0.5, 1.0]).max(1.0);
        assert_eq!(row(&spark, 3), " ▄█");
    }

    #[test]
    fn a_sparkline_longer_than_its_region_keeps_the_newest_values() {
        let spark = Sparkline::new([1.0, 1.0, 0.0, 0.0]).max(1.0);
        assert_eq!(row(&spark, 2), "  ", "the two most recent values are both zero");
    }

    #[test]
    fn an_empty_sparkline_draws_nothing() {
        assert_eq!(row(&Sparkline::new([]), 4), "    ");
    }

    // ---- Field --------------------------------------------------------------------------

    #[test]
    fn a_field_right_aligns_its_value() {
        assert_eq!(row(&Field::new("NET", "OFFLINE"), 14), "NET    OFFLINE");
    }

    #[test]
    fn a_field_can_pin_its_value_to_a_column() {
        assert_eq!(row(&Field::new("NET", "UP").value_column(8), 12), "NET     UP  ");
    }

    #[test]
    fn a_field_truncates_the_label_rather_than_the_value() {
        // The value is the reading; the label is a reminder of what it means.
        let line = row(&Field::new("A REALLY LONG LABEL", "42"), 12);
        assert!(line.ends_with("42"), "got {line:?}");
        assert!(line.contains('…'), "got {line:?}");
    }

    // ---- Panel --------------------------------------------------------------------------

    #[test]
    fn a_bordered_panel_sets_its_title_into_the_frame() {
        let panel = Panel::new("HEAD").border(Border::Line).child(Text::new("body"));
        let drawn = rows(&panel, 14, 4);
        assert_eq!(drawn[0], "┌─ HEAD ─────┐");
        assert_eq!(drawn[1], "│body        │");
        assert_eq!(drawn[3], "└────────────┘");
    }

    #[test]
    fn a_borderless_panel_spends_one_row_on_its_title() {
        let panel = Panel::new("HEAD").child(Text::new("body"));
        assert_eq!(rows(&panel, 6, 2), ["HEAD  ", "body  "]);
    }

    #[test]
    fn a_panel_subtitle_takes_the_row_under_the_title() {
        let panel = Panel::new("HEAD").subtitle("sub").child(Text::new("body"));
        assert_eq!(rows(&panel, 6, 3), ["HEAD  ", "sub   ", "body  "]);
    }

    #[test]
    fn panel_children_stack_with_the_requested_gap() {
        let panel = Panel::bare().gap(1).child(Text::new("a")).child(Text::new("b"));
        assert_eq!(rows(&panel, 2, 3), ["a ", "  ", "b "]);
    }

    #[test]
    fn the_header_height_is_the_row_the_first_child_actually_lands_on() {
        // Restating the formula would test nothing — it is three lines and a test that copies it
        // would agree with a wrong one. So the render answers instead: find the row the body is
        // on and require `header_height` to have predicted it. This is what a parent sizing a
        // panel's slot is relying on, and the number is only worth publishing if it is the truth.
        let cases: Vec<(&str, Panel<'_>)> = vec![
            ("bare", Panel::bare().child(Text::new("body"))),
            ("title", Panel::new("HEAD").child(Text::new("body"))),
            ("title and subtitle", Panel::new("HEAD").subtitle("sub").child(Text::new("body"))),
            ("framed", Panel::bare().border(Border::Line).child(Text::new("body"))),
            (
                "framed with a title",
                Panel::new("HEAD").border(Border::Line).child(Text::new("body")),
            ),
            ("padded", Panel::bare().padding(Padding::xy(0, 2)).child(Text::new("body"))),
            (
                "everything at once",
                Panel::new("HEAD")
                    .subtitle("sub")
                    .border(Border::Line)
                    .padding(Padding::xy(1, 1))
                    .child(Text::new("body")),
            ),
        ];
        for (what, panel) in cases {
            let claimed = panel.header_height();
            let drawn = rows(&panel, 12, 10);
            let found = drawn.iter().position(|row| row.contains("body"));
            assert_eq!(found, Some(usize::from(claimed)), "{what}: rows were {drawn:?}");
        }
    }

    #[test]
    fn a_panel_smaller_than_its_frame_does_not_panic() {
        for width in 0..6u16 {
            for height in 0..4u16 {
                let panel = Panel::new("T").border(Border::Line).child(Text::new("x"));
                let _ = rows(&panel, width, height);
            }
        }
    }

    // ---- Hints --------------------------------------------------------------------------

    #[test]
    fn hints_lay_out_keys_and_actions_on_one_row() {
        let hints = Hints::new().key("Q", "quit").key("R", "reset");
        assert_eq!(row(&hints, 22), "Q quit   R reset      ");
        assert_eq!(hints.constraint(Direction::Vertical), Constraint::Length(1));
    }

    #[test]
    fn hints_stop_at_the_region_edge_instead_of_wrapping() {
        let hints = Hints::new().key("SPACE", "pause").key("Q", "quit");
        assert_eq!(row(&hints, 11), "SPACE pause");
    }

    #[test]
    fn a_hint_that_does_not_fit_whole_is_dropped_rather_than_clipped() {
        let hints = Hints::new().key("TAB", "next").key("ESC", "quit");
        assert_eq!(hints.width(), 8 + 3 + 8);
        // One column short of the second hint: it goes entirely, not half of it.
        assert_eq!(row(&hints, 18), "TAB next          ");
        assert_eq!(row(&hints, 19), "TAB next   ESC quit");
    }

    // ---- List ---------------------------------------------------------------------------

    #[test]
    fn a_list_without_a_selection_draws_rows_flush_left() {
        let list = List::new(["milk", "eggs"]);
        assert_eq!(rows(&list, 6, 2), ["milk  ", "eggs  "]);
        assert_eq!(list.constraint(Direction::Vertical), Constraint::Fill(1));
    }

    #[test]
    fn a_selection_marks_its_row_and_indents_every_other_one() {
        let selection = Selection::at(1);
        let list = List::new(["milk", "eggs"]).selection(&selection);
        assert_eq!(rows(&list, 8, 2), ["  milk  ", "› eggs  "]);
    }

    #[test]
    fn moving_the_selection_does_not_shift_the_text_sideways() {
        let first = Selection::at(0);
        let second = Selection::at(1);
        let one = rows(&List::new(["milk", "eggs"]).selection(&first), 8, 2);
        let two = rows(&List::new(["milk", "eggs"]).selection(&second), 8, 2);
        // Columns, not byte offsets: the marker is a three-byte character one column wide.
        let column = |row: &str, needle: &str| {
            row.find(needle).map(|byte| text_width(&row[..byte])).expect("row should contain it")
        };
        assert_eq!(column(&one[0], "milk"), column(&two[0], "milk"));
        assert_eq!(column(&one[1], "eggs"), column(&two[1], "eggs"));
    }

    #[test]
    fn a_list_scrolls_to_keep_the_selection_in_view() {
        let selection = Selection::at(3);
        let list = List::new(["a", "b", "c", "d"]).selection(&selection);
        assert_eq!(rows(&list, 4, 2), ["  c ", "› d "]);
    }

    #[test]
    fn marks_form_a_column_as_wide_as_the_widest_of_them() {
        let list = List::new([
            ListRow::new("failed").mark("ERROR", Role::Danger),
            ListRow::new("fine").mark("OK", Role::Accent),
        ]);
        assert_eq!(rows(&list, 13, 2), ["ERROR failed ", "OK    fine   "]);
    }

    #[test]
    fn a_rows_own_role_survives_unless_it_is_selected() {
        // Row 0 carries its own role and is not selected; row 1 is selected and should not.
        let selection = Selection::at(1);
        let list =
            List::new([ListRow::new("done").role(Role::Dim), ListRow::new("todo").role(Role::Dim)])
                .selection(&selection)
                .selected_role(Role::Warn);
        let mut buffer = Buffer::new(8, 2);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        list.render(&mut canvas);
        assert_eq!(
            buffer.get(2, 0).unwrap().style.fg,
            Some(Theme::LAYA.dim),
            "unselected keeps its own"
        );
        assert_eq!(
            buffer.get(2, 1).unwrap().style.fg,
            Some(Theme::LAYA.warn),
            "selected overrides it"
        );
    }

    #[test]
    fn a_highlight_paints_the_whole_selected_row_including_past_the_text() {
        let selection = Selection::at(0);
        let list = List::new(["ab", "cd"]).selection(&selection).highlight();
        let mut buffer = Buffer::new(8, 2);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        list.render(&mut canvas);
        assert_eq!(buffer.get(2, 0).unwrap().style.bg, Some(Theme::LAYA.surface), "under the text");
        assert_eq!(
            buffer.get(7, 0).unwrap().style.bg,
            Some(Theme::LAYA.surface),
            "and past its end"
        );
        assert_ne!(
            buffer.get(7, 1).unwrap().style.bg,
            Some(Theme::LAYA.surface),
            "but not elsewhere"
        );
    }

    #[test]
    fn a_checklist_draws_a_box_per_row_and_the_cursor_from_the_same_state() {
        let mut picker = Checklist::new(3);
        picker.toggle();
        picker.down();
        let list = List::new(["Code", "Insiders", "Cursor"]).checklist(&picker);
        assert_eq!(
            rows(&list, 15, 3),
            ["  [\u{2713}] Code     ", "› [ ] Insiders ", "  [ ] Cursor   "]
        );
        assert_eq!(list.text_column(), 6, "the cursor, the box, and a space");
    }

    #[test]
    fn a_ticked_box_is_the_only_thing_in_a_column_of_boxes_with_colour() {
        let mut picker = Checklist::new(2);
        picker.toggle();
        let list = List::new(["on", "off"]).checklist(&picker).check_role(Role::Warn);
        let mut buffer = Buffer::new(10, 2);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        list.render(&mut canvas);
        assert_eq!(buffer.get(3, 0).unwrap().style.fg, Some(Theme::LAYA.warn), "the tick");
        assert_eq!(buffer.get(2, 0).unwrap().style.fg, Some(Theme::LAYA.dim), "not its bracket");
        assert_eq!(buffer.get(3, 1).unwrap().style.fg, Some(Theme::LAYA.dim), "nor an empty box");
    }

    #[test]
    fn a_click_on_a_box_is_distinguishable_from_a_click_on_its_row() {
        let picker = Checklist::new(2);
        let list = List::new(["one", "two"]).checklist(&picker);
        let boxes = list.check_column().expect("a checklist has one");
        assert_eq!(boxes, 2..5);
        assert!(boxes.contains(&3), "the tick itself");
        assert!(!boxes.contains(&5), "the space before the text belongs to the text");
        assert_eq!(List::new(["one"]).check_column(), None, "no boxes, no column");
    }

    #[test]
    fn a_checklist_row_can_still_carry_a_mark_of_its_own() {
        let picker = Checklist::all(1);
        let list = List::new([ListRow::new("broken").mark("WARN", Role::Warn)]).checklist(&picker);
        assert_eq!(row(&list, 20), "› [\u{2713}] WARN broken   ");
        assert_eq!(list.text_column(), 11);
    }

    #[test]
    fn an_empty_list_says_so() {
        let list = List::new(Vec::<String>::new()).empty("nothing here");
        assert_eq!(row(&list, 14), "nothing here  ");
    }

    #[test]
    fn a_row_too_long_for_the_region_is_truncated_with_a_marker() {
        let list = List::new(["a very long entry indeed"]);
        assert_eq!(row(&list, 8), "a very …");
    }

    #[test]
    fn a_list_in_a_region_too_small_to_draw_does_not_panic() {
        let selection = Selection::at(2);
        for width in 0..4u16 {
            for height in 0..3u16 {
                let list = List::new(["a", "b", "c"]).selection(&selection).highlight();
                let _ = rows(&list, width, height);
            }
        }
    }

    // ---- Table --------------------------------------------------------------------------

    #[test]
    fn a_table_puts_each_heading_over_its_own_column() {
        let table =
            Table::new([TableColumn::new("PID").length(4).right(), TableColumn::new("NAME")])
                .rows([vec!["42", "cargo"]]);
        assert_eq!(rows(&table, 12, 2), [" PID NAME   ", "  42 cargo  "]);
        assert_eq!(table.constraint(Direction::Vertical), Constraint::Fill(1));
        assert_eq!(table.constraint(Direction::Horizontal), Constraint::Fill(1));
    }

    #[test]
    fn a_right_aligned_column_lines_its_digits_up() {
        // The whole point of stating a width: 124.0 and 7.0 have their decimal points in the same
        // column, which is the difference between a table of figures and a list of strings.
        let table =
            Table::new([TableColumn::new("CPU%").length(6).right(), TableColumn::new("COMMAND")])
                .rows([vec!["124.0", "kernel_task"], vec!["7.0", "mds"]]);
        assert_eq!(rows(&table, 14, 3), ["  CPU% COMMAND", " 124.0 kernel…", "   7.0 mds    "]);
    }

    #[test]
    fn a_wide_character_does_not_shove_the_column_to_its_right() {
        // Why this is a widget and not a `format!`: padding is measured in cells, not characters.
        let table = Table::new([TableColumn::new("A").length(4), TableColumn::new("B")])
            .rows([vec!["日本", "x"], vec!["ab", "y"]]);
        let drawn = rows(&table, 9, 3);
        let column = |row: &str, needle: &str| {
            row.find(needle).map(|byte| text_width(&row[..byte])).expect("row should contain it")
        };
        assert_eq!(column(&drawn[1], "x"), column(&drawn[2], "y"));
        assert_eq!(text_width(&format!("{:<4}", "日本")), 6, "and this is what `format!` costs");
    }

    #[test]
    fn a_selection_marks_its_row_and_indents_the_heading_along_with_it() {
        let selection = Selection::at(1);
        let table = Table::new([TableColumn::new("N").length(3).right(), TableColumn::new("WHAT")])
            .rows([vec!["1", "one"], vec!["2", "two"]])
            .selection(&selection);
        assert_eq!(rows(&table, 12, 3), ["    N WHAT  ", "    1 one   ", "›   2 two   "]);
    }

    #[test]
    fn the_sorted_column_wears_an_arrow_saying_which_way() {
        let columns = || {
            [TableColumn::new("PID").length(4).right(), TableColumn::new("CPU%").length(6).right()]
        };
        assert_eq!(row(&Table::new(columns()).sorted_by(1, true), 11), " PID CPU% ↓");
        assert_eq!(row(&Table::new(columns()).sorted_by(1, false), 11), " PID CPU% ↑");
        // An arrow costs two cells, so a six-wide column of percentages still fits its heading.
        assert_eq!(row(&Table::new(columns()), 11), " PID   CPU%");
    }

    #[test]
    fn a_table_scrolls_its_body_and_leaves_the_heading_where_it_is() {
        let selection = Selection::at(3);
        let table = Table::new(["N"])
            .rows([vec!["a"], vec!["b"], vec!["c"], vec!["d"]])
            .selection(&selection);
        assert_eq!(rows(&table, 5, 3), ["  N  ", "  c  ", "› d  "]);
    }

    #[test]
    fn a_click_on_the_heading_names_a_column() {
        let selection = Selection::at(0);
        let table = Table::new([
            TableColumn::new("PID").length(4),
            TableColumn::new("CPU%").length(6),
            TableColumn::new("COMMAND"),
        ])
        .rows([vec!["1", "2", "three"]])
        .selection(&selection);
        // Two cells of gutter, then columns four, six and the ten that are left.
        let area = Rect::new(0, 0, 24, 4);
        let at = |x| table.hit_at(area, Pos::new(x, 0));
        assert_eq!(at(0), Some(TableHit::Heading(0)), "the cursor gutter, not nothing");
        assert_eq!(at(3), Some(TableHit::Heading(0)));
        assert_eq!(at(6), Some(TableHit::Heading(0)), "and the gap on its right");
        assert_eq!(at(7), Some(TableHit::Heading(1)));
        assert_eq!(at(14), Some(TableHit::Heading(2)));
        assert_eq!(at(23), Some(TableHit::Heading(2)), "out to the last cell");
        assert_eq!(table.hit_at(area, Pos::new(24, 0)), None, "but not past the area");
    }

    #[test]
    fn a_click_below_the_heading_names_a_row() {
        let selection = Selection::at(0);
        let table =
            Table::new(["A", "B"]).rows([vec!["1", "a"], vec!["2", "b"]]).selection(&selection);
        let area = Rect::new(2, 3, 10, 4);
        assert_eq!(table.hit_at(area, Pos::new(4, 3)), Some(TableHit::Heading(0)), "the heading");
        assert_eq!(table.hit_at(area, Pos::new(4, 4)), Some(TableHit::Row(0)));
        assert_eq!(table.hit_at(area, Pos::new(4, 5)), Some(TableHit::Row(1)));
        assert_eq!(table.hit_at(area, Pos::new(4, 6)), None, "past the last row");
        assert_eq!(table.hit_at(area, Pos::new(4, 9)), None, "outside the area");
    }

    #[test]
    fn a_click_lands_on_the_row_it_looks_like_even_when_the_table_is_scrolled() {
        // The index is into the rows given, not into the ones on screen, which is the only answer
        // the caller can do anything with.
        let selection = Selection::at(3);
        let table = Table::new(["N"])
            .rows([vec!["a"], vec!["b"], vec!["c"], vec!["d"]])
            .selection(&selection);
        let area = Rect::new(0, 0, 5, 3);
        let _ = rows(&table, 5, 3); // The window is only known once it has been drawn.
        assert_eq!(table.hit_at(area, Pos::new(2, 1)), Some(TableHit::Row(2)));
        assert_eq!(table.hit_at(area, Pos::new(2, 2)), Some(TableHit::Row(3)));
    }

    #[test]
    fn an_empty_table_keeps_its_columns_and_says_why_it_is_empty() {
        let table = Table::new([TableColumn::new("PID").length(4), TableColumn::new("NAME")])
            .rows(Vec::<Vec<String>>::new())
            .empty("no processes");
        assert!(table.is_empty());
        // The heading still draws: the columns are still true, and a table with neither a row nor
        // a reason is indistinguishable from one that failed to draw.
        assert_eq!(rows(&table, 14, 2), ["PID  NAME     ", "no processes  "]);
    }

    #[test]
    fn a_row_that_does_not_match_the_heading_leaves_a_gap_rather_than_panicking() {
        let table = Table::new([TableColumn::new("A").length(2), TableColumn::new("B").length(2)])
            .rows([vec!["1"], vec!["1", "2", "3"]]);
        assert_eq!(rows(&table, 5, 3), ["A  B ", "1    ", "1  2 "]);
    }

    #[test]
    fn a_table_rows_own_role_survives_unless_it_is_selected() {
        let selection = Selection::at(1);
        let table = Table::new(["WHAT"])
            .rows([
                TableRow::new(["done"]).role(Role::Dim),
                TableRow::new(["todo"]).role(Role::Dim),
            ])
            .selection(&selection)
            .selected_role(Role::Warn);
        let mut buffer = Buffer::new(8, 3);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        table.render(&mut canvas);
        assert_eq!(buffer.get(2, 1).unwrap().style.fg, Some(Theme::LAYA.dim), "keeps its own");
        assert_eq!(buffer.get(2, 2).unwrap().style.fg, Some(Theme::LAYA.warn), "except selected");
    }

    #[test]
    fn a_highlight_paints_the_selected_row_and_nothing_above_it() {
        let selection = Selection::at(0);
        let table =
            Table::new(["A"]).rows([vec!["x"], vec!["y"]]).selection(&selection).highlight();
        let mut buffer = Buffer::new(8, 3);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        table.render(&mut canvas);
        let bg = |x, y| buffer.get(x, y).unwrap().style.bg;
        assert_eq!(bg(7, 1), Some(Theme::LAYA.surface), "past the end of the selected row");
        assert_ne!(bg(7, 0), Some(Theme::LAYA.surface), "never the heading");
        assert_ne!(bg(7, 2), Some(Theme::LAYA.surface), "nor another row");
    }

    #[test]
    fn a_table_in_a_region_too_small_to_draw_does_not_panic() {
        let selection = Selection::at(2);
        for width in 0..6u16 {
            for height in 0..3u16 {
                let table =
                    Table::new([TableColumn::new("PID").length(4).right(), TableColumn::new("N")])
                        .rows([vec!["1", "a"], vec!["2", "b"], vec!["3", "c"]])
                        .selection(&selection)
                        .highlight();
                let _ = rows(&table, width, height);
                let _ = table.hit_at(Rect::new(0, 0, width, height), Pos::new(0, 0));
            }
        }
    }

    // ---- Scrollbar ----------------------------------------------------------------------

    /// The bar as one string, top to bottom, which is easier to read than a column of rows.
    fn bar(offset: usize, content: usize, height: u16) -> String {
        rows(&Scrollbar::new(offset, content), 1, height).concat()
    }

    #[test]
    fn a_scrollbar_thumb_is_proportional_and_starts_at_the_top() {
        assert_eq!(bar(0, 20, 10), "█████│││││");
    }

    #[test]
    fn a_scrollbar_touches_an_end_only_at_that_end() {
        // Worth more than proportionality: a bar that looks finished with a row still to read is
        // a lie, and one that looks untouched after scrolling makes people scroll again.
        assert_eq!(bar(10, 20, 10), "│││││█████");
        assert_eq!(bar(9, 20, 10), "││││█████│", "one row short of the end, and it shows");
        assert_eq!(bar(1, 20, 10), "│█████││││", "one row down from the top, and it shows");
    }

    #[test]
    fn every_scrollbar_position_is_honest_about_the_ends() {
        // Exhaustively, because the arithmetic has four cases and the interesting ones are the
        // boundaries between them — `furthest == 2` and `travel == 1` are each one offset wide.
        for content in 2..60usize {
            for height in 1..16u16 {
                let furthest = content.saturating_sub(usize::from(height));
                for offset in 0..=furthest {
                    let Some(thumb) = Scrollbar::new(offset, content).thumb(height) else {
                        continue;
                    };
                    let (top, length) = (thumb.start, thumb.end - thumb.start);
                    let travel = height - length;
                    let at = format!("content {content} in {height} rows at {offset}");
                    assert!(length >= 1, "the thumb must stay findable: {at}");
                    assert!(top + length <= height, "and inside the track: {at}");
                    // A thumb that has not moved must mean nothing has been scrolled, and a thumb
                    // at the bottom must mean there is nothing left to read. The converses are
                    // what a track too short to hold three positions cannot promise.
                    if offset == 0 {
                        assert_eq!(top, 0, "the top is the top: {at}");
                    }
                    if offset == furthest {
                        assert_eq!(top, travel, "the end is the end: {at}");
                    } else if travel > 0 {
                        assert!(top < travel, "claims the end early: {at}");
                    }
                    if offset > 0 && travel >= 2 {
                        assert!(top > 0, "hides a scroll that happened: {at}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_scrollbar_over_a_long_document_keeps_a_thumb_you_can_see() {
        let drawn = bar(0, 10_000, 8);
        assert_eq!(drawn.matches('█').count(), 1, "rounds to nothing, so it is given one row");
        assert_eq!(drawn, "█│││││││");
    }

    #[test]
    fn a_scrollbar_with_nothing_to_say_says_nothing() {
        // Not a full-length thumb: a bar that is always there trains the eye to stop seeing it.
        assert_eq!(bar(0, 4, 10), "          ");
        assert_eq!(bar(0, 10, 10), "          ", "exactly fitting is still fitting");
    }

    #[test]
    fn a_scrollbar_past_the_end_pins_to_the_bottom_rather_than_running_off_it() {
        assert_eq!(bar(9_999, 20, 10), "│││││█████");
    }

    #[test]
    fn dragging_the_thumb_to_where_it_already_is_scrolls_nothing() {
        // The property a drag lives or dies by: the thumb must not shift under the pointer. The
        // offset may change — several offsets can share a row — but the drawn bar may not.
        for content in 2..60usize {
            for height in 1..16u16 {
                for offset in 0..=content {
                    let bar = Scrollbar::new(offset, content);
                    let Some(thumb) = bar.thumb(height) else { continue };
                    let landed = Scrollbar::new(bar.offset_at(thumb.start, height), content);
                    assert_eq!(
                        landed.thumb(height),
                        Some(thumb),
                        "content {content} in {height} rows at {offset}"
                    );
                }
            }
        }
    }

    #[test]
    fn dragging_the_thumb_to_an_end_reaches_that_end_exactly() {
        let bar = Scrollbar::new(0, 40);
        assert_eq!(bar.offset_at(0, 10), 0);
        assert_eq!(bar.offset_at(7, 10), 30, "the last row of travel is the last row of content");
        assert_eq!(bar.offset_at(9, 10), 30, "and dragging past it stays there");
    }

    #[test]
    fn dragging_a_bar_with_nothing_to_scroll_goes_nowhere() {
        assert_eq!(Scrollbar::new(0, 4).offset_at(3, 10), 0);
    }

    #[test]
    fn a_dragged_thumb_covers_the_whole_track_over_a_long_document() {
        // One row of thumb and ten of track: every row of the bar must be reachable by dragging to
        // it, and the ends must be the ends, or a long file has parts you can only reach by key.
        let bar = Scrollbar::new(0, 10_000);
        let reached: Vec<u16> = (0..10)
            .map(|row| Scrollbar::new(bar.offset_at(row, 10), 10_000).thumb(10).unwrap().start)
            .collect();
        assert_eq!(reached, (0..10).collect::<Vec<_>>());
        assert_eq!(bar.offset_at(9, 10), 9_990, "the bottom row is the end of the document");
    }

    // ---- Input --------------------------------------------------------------------------

    #[test]
    fn an_input_draws_its_prompt_then_the_value() {
        let editor = Editor::with("milk");
        let input = Input::new(&editor).prompt(">");
        assert_eq!(row(&input, 10), "> milk    ");
        assert_eq!(input.constraint(Direction::Vertical), Constraint::Length(1));
    }

    #[test]
    fn an_empty_input_shows_its_placeholder() {
        let editor = Editor::new();
        let input = Input::new(&editor).placeholder("what next?");
        assert_eq!(row(&input, 12), "what next?  ");
    }

    #[test]
    fn a_value_hides_the_placeholder() {
        let editor = Editor::with("x");
        let input = Input::new(&editor).placeholder("what next?");
        assert_eq!(row(&input, 12), "x           ");
    }

    #[test]
    fn the_cursor_is_a_reversed_cell_at_the_insertion_point() {
        let mut editor = Editor::with("ab");
        editor.left();
        let mut buffer = Buffer::new(6, 1);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        Input::new(&editor).render(&mut canvas);
        assert!(buffer.get(1, 0).unwrap().style.attrs.contains(conui_cell::Attrs::REVERSE));
        assert!(!buffer.get(0, 0).unwrap().style.attrs.contains(conui_cell::Attrs::REVERSE));
    }

    #[test]
    fn the_cursor_can_be_turned_off_for_an_unfocused_field() {
        let editor = Editor::with("ab");
        let mut buffer = Buffer::new(6, 1);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        Input::new(&editor).no_cursor().render(&mut canvas);
        for column in 0..6 {
            assert!(
                !buffer.get(column, 0).unwrap().style.attrs.contains(conui_cell::Attrs::REVERSE)
            );
        }
    }

    #[test]
    fn a_value_wider_than_the_field_scrolls_to_keep_the_cursor_visible() {
        let editor = Editor::with("abcdefgh");
        // Cursor at the end, five columns: the tail is what matters, not the head.
        assert_eq!(row(&Input::new(&editor), 5), "efgh ");
    }

    #[test]
    fn scrolling_back_to_the_start_shows_the_head_again() {
        let mut editor = Editor::with("abcdefgh");
        editor.home();
        assert_eq!(row(&Input::new(&editor), 5), "abcde");
    }

    #[test]
    fn a_prompt_narrows_the_field_it_scrolls_within() {
        let editor = Editor::with("abcdefgh");
        assert_eq!(row(&Input::new(&editor).prompt("»"), 5), "» gh ");
    }

    #[test]
    fn an_input_with_no_room_left_for_the_field_does_not_panic() {
        let editor = Editor::with("abc");
        for width in 0..5u16 {
            let _ = row(&Input::new(&editor).prompt("long prompt"), width);
        }
    }

    // ---- Button -------------------------------------------------------------------------

    #[test]
    fn a_button_is_its_label_between_brackets() {
        assert_eq!(row(&Button::new("Save"), 8), "\u{2039} Save \u{203a}");
        assert_eq!(Button::width("Save"), 8);
    }

    #[test]
    fn a_button_centres_itself_in_a_wider_slot() {
        assert_eq!(row(&Button::new("Go"), 10), "  \u{2039} Go \u{203a}  ");
    }

    #[test]
    fn a_focused_button_is_lifted_onto_the_surface_and_an_unfocused_one_is_not() {
        let mut buffer = Buffer::new(8, 1);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        Button::new("Save").focused(true).render(&mut canvas);
        assert_eq!(buffer.get(0, 0).expect("cell").style.bg, Some(Theme::LAYA.surface));

        let mut buffer = Buffer::new(8, 1);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        Button::new("Save").render(&mut canvas);
        assert_ne!(buffer.get(0, 0).expect("cell").style.bg, Some(Theme::LAYA.surface));
    }

    #[test]
    fn a_disabled_button_never_looks_focused_even_if_it_is_told_it_is() {
        let mut buffer = Buffer::new(8, 1);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        Button::new("Save").focused(true).disabled().render(&mut canvas);
        assert_ne!(buffer.get(0, 0).expect("cell").style.bg, Some(Theme::LAYA.surface));
        assert_eq!(buffer.get(2, 0).expect("cell").style.fg, Some(Theme::LAYA.dim));
    }

    #[test]
    fn a_button_wider_than_its_slot_truncates_instead_of_overflowing() {
        let drawn = row(&Button::new("Save everything"), 9);
        assert_eq!(text_width(&drawn), 9);
        assert!(drawn.starts_with('\u{2039}') && drawn.ends_with('\u{203a}'), "got {drawn:?}");
    }

    #[test]
    fn a_button_measures_its_label_in_columns_not_bytes() {
        assert_eq!(Button::width("\u{754c}\u{754c}"), 8);
    }

    // ---- Buttons ------------------------------------------------------------------------

    #[test]
    fn a_row_of_buttons_lifts_the_one_the_selection_is_on() {
        let choice = Selection::at(1);
        let buttons = Buttons::new(["Yes", "No"]).selection(&choice);
        assert_eq!(row(&buttons, 15), "\u{2039} Yes \u{203a}  \u{2039} No \u{203a}");
        assert_eq!(buttons.constraint(Direction::Horizontal), Constraint::Length(15));
        assert_eq!(buttons.constraint(Direction::Vertical), Constraint::Length(1));

        let mut buffer = Buffer::new(15, 1);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        buttons.render(&mut canvas);
        let lift = Some(Theme::LAYA.surface);
        assert_ne!(buffer.get(0, 0).expect("cell").style.bg, lift, "‹ Yes › is not focused");
        assert_eq!(buffer.get(9, 0).expect("cell").style.bg, Some(Theme::LAYA.surface));
    }

    #[test]
    fn a_letter_answers_the_same_question_the_arrows_do() {
        let buttons = Buttons::new(["Yes", "No", "Cancel"]);
        assert_eq!(buttons.index_for('y'), Some(0));
        assert_eq!(buttons.index_for('N'), Some(1), "case is not the user's problem");
        assert_eq!(buttons.index_for('c'), Some(2));
        assert_eq!(buttons.index_for('q'), None);
        // And the arrows reach the same indices, wrapping round a short row.
        let mut choice = Selection::new();
        choice.cycle_up(buttons.len());
        assert_eq!(choice.selected(), 2);
        choice.cycle_down(buttons.len());
        assert_eq!(choice.selected(), 0);
    }

    #[test]
    fn a_centred_row_is_hit_tested_where_it_was_actually_drawn() {
        let buttons = Buttons::new(["Yes", "No"]).centered();
        let area = Rect::new(0, 4, 21, 1);
        assert_eq!(row(&buttons, 21), "   \u{2039} Yes \u{203a}  \u{2039} No \u{203a}   ");
        assert_eq!(buttons.index_at(area, Pos::new(3, 4)), Some(0), "the left bracket of ‹ Yes ›");
        assert_eq!(buttons.index_at(area, Pos::new(9, 4)), Some(0), "its right bracket");
        assert_eq!(buttons.index_at(area, Pos::new(10, 4)), None, "the gap belongs to neither");
        assert_eq!(buttons.index_at(area, Pos::new(12, 4)), Some(1));
        assert_eq!(buttons.index_at(area, Pos::new(18, 4)), None, "past the row");
        assert_eq!(buttons.index_at(area, Pos::new(12, 5)), None, "another row entirely");
    }

    #[test]
    fn an_unfocused_row_still_says_which_button_enter_would_hit() {
        let choice = Selection::at(1);
        let buttons = Buttons::new(["Yes", "No"]).selection(&choice).focused(false);
        let mut buffer = Buffer::new(15, 1);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        buttons.render(&mut canvas);
        let lift = Some(Theme::LAYA.surface);
        assert_ne!(buffer.get(9, 0).expect("cell").style.bg, lift, "no lift while unfocused");
        assert_eq!(
            buffer.get(11, 0).expect("cell").style.fg,
            Some(Theme::LAYA.text),
            "but current"
        );
        assert_eq!(buffer.get(2, 0).expect("cell").style.fg, Some(Theme::LAYA.dim), "and this dim");
    }

    #[test]
    fn a_button_that_does_not_fit_is_left_out_rather_than_half_drawn() {
        let buttons = Buttons::new(["Yes", "No"]);
        let drawn = row(&buttons, 12);
        assert_eq!(drawn, "\u{2039} Yes \u{203a}     ", "‹ No › needs six columns and has five");
        assert_eq!(buttons.index_at(Rect::new(0, 0, 12, 1), Pos::new(10, 0)), None);
    }

    #[test]
    fn a_row_of_buttons_in_a_region_too_small_for_any_of_them_does_not_panic() {
        for width in 0..8u16 {
            let _ = row(&Buttons::new(["Repair", "Cancel"]).centered(), width);
        }
        assert_eq!(Buttons::new(Vec::<String>::new()).width(), 0);
        assert!(Buttons::new(Vec::<String>::new()).is_empty());
    }

    // ---- Tabs ---------------------------------------------------------------------------

    #[test]
    fn tabs_underline_the_selected_label_and_nothing_else() {
        let selection = Selection::at(1);
        let tabs = Tabs::new(["ONE", "TWO"]).selection(&selection).focused(true);
        assert_eq!(tabs.constraint(Direction::Vertical), Constraint::Length(2));
        let drawn = rows(&tabs, 12, 2);
        assert_eq!(drawn[0], "ONE   TWO   ");
        assert_eq!(drawn[1], "      \u{2500}\u{2500}\u{2500}   ");
    }

    #[test]
    fn an_unfocused_tab_bar_still_shows_where_you_are() {
        let selection = Selection::at(0);
        let tabs = Tabs::new(["ONE", "TWO"]).selection(&selection);
        let drawn = rows(&tabs, 12, 2);
        assert_eq!(drawn[1], "\u{2500}\u{2500}\u{2500}         ", "the underline stays");

        let mut buffer = Buffer::new(12, 2);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        tabs.render(&mut canvas);
        let rule = buffer.get(0, 1).expect("cell").style.fg;
        assert_eq!(rule, Some(Theme::LAYA.dim), "but it stops claiming the keys");
    }

    #[test]
    fn tabs_that_do_not_fit_are_clipped_at_the_edge_rather_than_wrapping() {
        let selection = Selection::new();
        let tabs = Tabs::new(["ALPHA", "BETA", "GAMMA"]).selection(&selection);
        let drawn = row(&tabs, 10);
        assert_eq!(text_width(&drawn), 10);
    }

    #[test]
    fn tabs_report_the_width_they_want() {
        let tabs = Tabs::new(["ONE", "TWO"]).gap(3);
        assert_eq!(tabs.width(), 9);
        assert_eq!(Tabs::new(["ONE"]).gap(3).width(), 3, "one tab has no gap after it");
    }

    #[test]
    fn a_click_resolves_to_the_label_under_it_and_a_gap_resolves_to_nothing() {
        // ONE at 0..3, three columns of gap, TWO at 6..9.
        let tabs = Tabs::new(["ONE", "TWO"]).gap(3);
        assert_eq!(tabs.index_at(0), Some(0));
        assert_eq!(tabs.index_at(2), Some(0));
        assert_eq!(tabs.index_at(3), None, "the gap belongs to neither label");
        assert_eq!(tabs.index_at(5), None);
        assert_eq!(tabs.index_at(6), Some(1));
        assert_eq!(tabs.index_at(8), Some(1));
        assert_eq!(tabs.index_at(9), None, "past the end of the bar");
    }

    #[test]
    fn a_one_row_tab_bar_drops_the_underline() {
        let selection = Selection::new();
        let tabs = Tabs::new(["ONE"]).selection(&selection).no_underline();
        assert_eq!(tabs.constraint(Direction::Vertical), Constraint::Length(1));
    }

    // ---- Select -------------------------------------------------------------------------

    #[test]
    fn a_closed_select_shows_the_current_choice_and_a_caret() {
        let dropdown = Dropdown::at(1);
        let select = Select::new(&dropdown, ["red", "green", "blue"]);
        assert_eq!(row(&select, 14), "[ green    \u{25be} ]");
    }

    #[test]
    fn an_open_select_turns_its_caret_over() {
        let mut dropdown = Dropdown::new();
        dropdown.open();
        let drawn = row(&Select::new(&dropdown, ["red"]), 12);
        assert!(drawn.contains('\u{25b4}'), "got {drawn:?}");
    }

    #[test]
    fn a_select_records_where_it_drew_so_the_list_can_find_it() {
        let dropdown = Dropdown::new();
        let mut buffer = Buffer::new(30, 4);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        let mut slot = canvas.sub(Rect::new(6, 2, 12, 1));
        Select::new(&dropdown, ["a"]).render(&mut slot);
        assert_eq!(dropdown.field(), Rect::new(6, 2, 12, 1));
    }

    #[test]
    fn a_select_scrolled_out_of_its_pane_records_no_field_to_click() {
        let dropdown = Dropdown::new();
        let mut buffer = Buffer::new(30, 6);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        // A pane three rows down the screen, holding content scrolled by four: the select sits on
        // the first content row, which is now above the pane.
        let mut pane = canvas.sub(Rect::new(0, 3, 30, 3));
        let mut content = pane.shifted(0, -4, 30, 9);
        let mut slot = content.sub(Rect::new(0, 0, 12, 1));
        Select::new(&dropdown, ["a"]).render(&mut slot);
        assert!(dropdown.field().is_empty(), "got {:?}", dropdown.field());
        // The rect a clamp would have produced is row 0 of the screen, three rows above the pane,
        // where a click would open a list for a field nobody can see.
        assert!(!dropdown.field().contains(conui_cell::Pos::new(2, 0)));

        // Scrolled to where it does show, it is the field it draws.
        let mut visible = pane.shifted(0, 0, 30, 9);
        let mut slot = visible.sub(Rect::new(0, 0, 12, 1));
        Select::new(&dropdown, ["a"]).render(&mut slot);
        assert_eq!(dropdown.field(), Rect::new(0, 3, 12, 1));
    }

    #[test]
    fn a_select_over_no_choices_says_so_rather_than_drawing_an_empty_field() {
        let dropdown = Dropdown::new();
        let select = Select::new(&dropdown, Vec::<String>::new()).empty("none");
        assert_eq!(row(&select, 12), "[ none   \u{25be} ]");
    }

    #[test]
    fn a_select_sizes_itself_to_its_widest_choice() {
        assert_eq!(Select::width(&["red", "magenta"]), 13);
    }

    #[test]
    fn a_select_squeezed_to_nothing_does_not_panic() {
        let dropdown = Dropdown::new();
        for width in 0..8u16 {
            let _ = row(&Select::new(&dropdown, ["something long"]), width);
        }
    }

    // ---- Menu ---------------------------------------------------------------------------

    #[test]
    fn a_menu_frames_its_choices_and_marks_the_current_one() {
        let selection = Selection::at(1);
        let menu = Menu::new(&selection, ["red", "green"]);
        let drawn = rows(&menu, 11, 4);
        assert_eq!(
            drawn[0],
            "\u{250c}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2510}"
        );
        assert_eq!(drawn[1], "\u{2502}  red    \u{2502}");
        assert_eq!(drawn[2], "\u{2502}\u{203a} green  \u{2502}");
    }

    #[test]
    fn a_menu_paints_over_what_was_underneath_it() {
        let mut buffer = Buffer::new(11, 4);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        canvas.run(0, 1, '#', 11, Role::Text);
        let selection = Selection::new();
        Menu::new(&selection, ["red"]).render(&mut canvas);
        assert!(!buffer.row_text(1).contains('#'), "the frame beneath showed through");
    }

    #[test]
    fn a_menu_with_more_choices_than_rows_says_which_way_the_rest_are() {
        let selection = Selection::at(5);
        let options = ["a", "b", "c", "d", "e", "f"];
        let drawn = rows(&Menu::new(&selection, options), 8, 4);
        assert!(drawn[0].contains(mark::ARROW_UP), "no hint that choices are above: {drawn:?}");

        let selection = Selection::at(0);
        let drawn = rows(&Menu::new(&selection, options), 8, 4);
        assert!(drawn[3].contains(mark::ARROW_DOWN), "no hint that choices are below: {drawn:?}");
    }

    /// Drawing the menu is what makes `PageDown` mean six rows here rather than the `rows(8)` cap
    /// or the twenty choices: the selection learns the height from whoever windowed it, and for an
    /// open dropdown that is the menu, borders already taken off.
    #[test]
    fn paging_an_open_dropdown_moves_by_the_menu_that_was_drawn() {
        let options: Vec<String> = (0..20).map(|n| format!("choice {n:02}")).collect();
        let mut dropdown = Dropdown::new().rows(8);
        dropdown.open();
        let menu = Menu::new(dropdown.selection(), options.iter().map(String::as_str));
        let _ = rows(&menu, 14, 8);
        assert_eq!(dropdown.selection().height(), 6, "eight rows less two of border");

        let key = conui_input::KeyEvent::plain(conui_input::KeyCode::PageDown);
        assert!(dropdown.handle(&key, options.len()));
        assert_eq!(dropdown.selected(), 5, "a page of six, less the row carried over");
    }

    #[test]
    fn a_menu_too_small_to_frame_draws_nothing_rather_than_half_a_border() {
        let selection = Selection::new();
        for (width, height) in [(0, 0), (1, 1), (1, 4), (4, 1)] {
            let drawn = rows(&Menu::new(&selection, ["red"]), width, height);
            assert!(drawn.iter().all(|row| row.trim().is_empty()), "got {drawn:?}");
        }
    }

    // ---- Per-axis constraints -----------------------------------------------------------

    #[test]
    fn a_widget_one_row_tall_takes_whatever_width_it_is_given() {
        // These draw themselves to the width of their region, so `Fill` is the honest answer
        // across — and answering `Length(1)` to a `Row` would make each a single column wide.
        let editor = Editor::new();
        let views: [&dyn View; 3] = [&Rule::new(), &Gauge::new(0.5), &Input::new(&editor)];
        for view in views {
            assert_eq!(view.constraint(Direction::Vertical), Constraint::Length(1));
            assert_eq!(view.constraint(Direction::Horizontal), Constraint::Fill(1));
        }
    }

    #[test]
    fn a_scrollbar_is_one_column_wide_and_as_tall_as_it_is_given() {
        let bar = Scrollbar::new(0, 40);
        assert_eq!(bar.constraint(Direction::Horizontal), Constraint::Length(1));
        assert_eq!(bar.constraint(Direction::Vertical), Constraint::Fill(1));
    }

    #[test]
    fn widgets_that_know_their_own_width_report_the_one_they_draw() {
        // Each of these already had a `width()` for callers to size a slot with by hand; asked
        // per axis, they can answer with it themselves.
        let stat = Stat::new("BEST", 5);
        assert_eq!(stat.constraint(Direction::Horizontal), Constraint::Length(stat.width()));
        let tabs = Tabs::new(["one", "two"]);
        assert_eq!(tabs.constraint(Direction::Horizontal), Constraint::Length(tabs.width()));
        assert_eq!(
            Button::new("Apply").constraint(Direction::Horizontal),
            Constraint::Length(Button::width("Apply"))
        );
    }

    #[test]
    fn a_row_of_widgets_lays_itself_out_with_no_widths_written_down() {
        // What a caller used to write as `.length(Button::width("Save"))` beside every button.
        let view = crate::view::Row::new()
            .child(Button::new("Save"))
            .child(Rule::new())
            .child(Button::new("Quit"));
        assert_eq!(
            row(&view, 20),
            "\u{2039} Save \u{203a}\u{2500}\u{2500}\u{2500}\u{2500}\u{2039} Quit \u{203a}"
        );
    }
}
