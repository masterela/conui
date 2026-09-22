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

use conui_cell::{Padding, Rect, Style};

use crate::canvas::{Canvas, text_width};
use crate::layout::Constraint;
use crate::state::{Dropdown, Editor, Selection, Viewport};
use crate::theme::Role;
use crate::typography::{self, BarStyle, DIGIT_HEIGHT, line, mark};
use crate::view::{Stack, View};

/// Horizontal placement of text within its region.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
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
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            role: Role::Text,
            align: Align::Left,
            wrap: false,
            style: None,
        }
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    /// Shorthand for the three roles a label most often wants.
    pub fn muted(self) -> Self {
        self.role(Role::Muted)
    }

    pub fn accent(self) -> Self {
        self.role(Role::Accent)
    }

    pub fn dim(self) -> Self {
        self.role(Role::Dim)
    }

    /// Override the role with a full style, for emphasis a role cannot express.
    pub fn styled(mut self, style: Style) -> Self {
        self.style = Some(style);
        self
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn centered(self) -> Self {
        self.align(Align::Center)
    }

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

    fn constraint(&self) -> Constraint {
        if self.wrap {
            Constraint::Fill(1)
        } else {
            Constraint::Length(self.content.split('\n').count() as u16)
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
    pub fn new() -> Self {
        Self { title: None, role: Role::Dim, title_role: Role::Muted, heavy: false }
    }

    pub fn titled(title: impl Into<String>) -> Self {
        Self { title: Some(title.into()), ..Self::new() }
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    pub fn title_role(mut self, role: Role) -> Self {
        self.title_role = role;
        self
    }

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

    fn constraint(&self) -> Constraint {
        Constraint::Length(1)
    }
}

// ---- Gauge ------------------------------------------------------------------------------

/// What a gauge prints next to its bar.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Readout {
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

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    pub fn track(mut self, role: Role) -> Self {
        self.track = role;
        self
    }

    pub fn style(mut self, style: BarStyle) -> Self {
        self.style = style;
        self
    }

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

    fn constraint(&self) -> Constraint {
        Constraint::Length(1)
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
    pub fn new(label: impl Into<String>, value: i64) -> Self {
        Self { label: label.into(), value, digits: 3, role: Role::Accent, label_role: Role::Muted }
    }

    /// Minimum digit count; the number is zero-padded to this width.
    pub fn digits(mut self, digits: usize) -> Self {
        self.digits = digits;
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

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

    fn constraint(&self) -> Constraint {
        Constraint::Length(DIGIT_HEIGHT + 1)
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
    pub fn new(values: impl IntoIterator<Item = f32>) -> Self {
        Self { values: values.into_iter().collect(), max: None, role: Role::Info }
    }

    /// Fix the top of the scale. Without this the chart scales to its own maximum, which shows
    /// shape but hides level — a flat series at 10% looks identical to one at 90%.
    pub fn max(mut self, max: f32) -> Self {
        self.max = Some(max);
        self
    }

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

    fn constraint(&self) -> Constraint {
        Constraint::Length(1)
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
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            label_role: Role::Muted,
            value_role: Role::Text,
            value_column: None,
        }
    }

    pub fn label_role(mut self, role: Role) -> Self {
        self.label_role = role;
        self
    }

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

    fn constraint(&self) -> Constraint {
        Constraint::Length(1)
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
    Line,
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

    pub fn subtitle(mut self, subtitle: impl Into<String>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn border(mut self, border: Border) -> Self {
        self.border = border;
        self
    }

    pub fn title_role(mut self, role: Role) -> Self {
        self.title_role = role;
        self
    }

    pub fn border_role(mut self, role: Role) -> Self {
        self.border_role = role;
        self
    }

    pub fn child(mut self, view: impl View + 'a) -> Self {
        self.body = self.body.child(view);
        self
    }

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
    pub fn new() -> Self {
        Self { items: Vec::new(), key_role: Role::Muted, label_role: Role::Muted, spacing: 3 }
    }

    pub fn key(mut self, key: impl Into<String>, action: impl Into<String>) -> Self {
        self.items.push((key.into(), action.into()));
        self
    }

    /// Draw the key names in a stronger colour than their descriptions.
    pub fn emphasise_keys(mut self) -> Self {
        self.key_role = Role::Text;
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.label_role = role;
        self.key_role = role;
        self
    }

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

    fn constraint(&self) -> Constraint {
        Constraint::Length(1)
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
    marker: String,
    role: Role,
    selected_role: Role,
    highlight: bool,
    empty: Option<String>,
}

impl<'a> List<'a> {
    pub fn new<R: Into<ListRow>>(rows: impl IntoIterator<Item = R>) -> Self {
        Self {
            rows: rows.into_iter().map(Into::into).collect(),
            selection: None,
            marker: format!("{} ", mark::SELECTED),
            role: Role::Text,
            selected_role: Role::Accent,
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

    /// The cursor drawn against the selected row. Its width indents every row, selected or not,
    /// so moving the selection never shifts the text sideways.
    pub fn marker(mut self, marker: impl Into<String>) -> Self {
        self.marker = marker.into();
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

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

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
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
        marker + mark + u16::from(mark > 0)
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
            if let Some((text, role)) = &row.mark {
                canvas.put_truncated(i32::from(marker_width), y, text, mark_width, *role);
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

    fn constraint(&self) -> Constraint {
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

    pub fn prompt_role(mut self, role: Role) -> Self {
        self.prompt_role = role;
        self
    }

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

    fn constraint(&self) -> Constraint {
        Constraint::Length(1)
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

    fn constraint(&self) -> Constraint {
        Constraint::Length(1)
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
    pub fn gap(mut self, gap: u16) -> Self {
        self.gap = gap;
        self
    }

    /// Drop the underline row, making this one row tall.
    pub fn no_underline(mut self) -> Self {
        self.underline = false;
        self
    }

    pub fn role(mut self, role: Role) -> Self {
        self.role = role;
        self
    }

    pub fn selected_role(mut self, role: Role) -> Self {
        self.selected_role = role;
        self
    }

    pub fn len(&self) -> usize {
        self.labels.len()
    }

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

    fn constraint(&self) -> Constraint {
        Constraint::Length(if self.underline { 2 } else { 1 })
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

    pub fn focused(mut self, focused: bool) -> Self {
        self.focused = focused;
        self
    }

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
        // Record the whole field, not just the text, so the list lines up with the brackets.
        self.dropdown.set_field(Rect::new(
            canvas.screen_area().x,
            canvas.screen_area().y,
            width,
            1,
        ));

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

    fn constraint(&self) -> Constraint {
        Constraint::Length(1)
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

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

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

    fn constraint(&self) -> Constraint {
        Constraint::Length(u16::try_from(self.options.len()).unwrap_or(u16::MAX).saturating_add(2))
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
        assert_eq!(text.constraint(), Constraint::Length(1));
    }

    #[test]
    fn text_alignment_places_the_line_within_the_region() {
        assert_eq!(row(&Text::new("ab").centered(), 6), "  ab  ");
        assert_eq!(row(&Text::new("ab").right(), 6), "    ab");
    }

    #[test]
    fn embedded_newlines_become_rows_and_are_counted() {
        let text = Text::new("one\ntwo");
        assert_eq!(text.constraint(), Constraint::Length(2));
        assert_eq!(rows(&text, 4, 2), ["one ", "two "]);
    }

    #[test]
    fn unwrapped_text_is_clipped_rather_than_reflowed() {
        // Clipping keeps the layout stable; reflowing would push everything below it down.
        assert_eq!(row(&Text::new("a long sentence"), 6), "a long");
        assert_eq!(Text::new("a long sentence").constraint(), Constraint::Length(1));
    }

    #[test]
    fn wrapped_text_breaks_on_word_boundaries() {
        let text = Text::new("the quick brown fox").wrapped();
        assert_eq!(rows(&text, 10, 3), ["the quick ", "brown fox ", "          "]);
        assert_eq!(text.constraint(), Constraint::Fill(1));
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
        assert_eq!(gauge.constraint(), Constraint::Length(1));
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

    // ---- Stat ---------------------------------------------------------------------------

    #[test]
    fn a_stat_puts_its_label_above_block_digits() {
        let stat = Stat::new("SCORE", 42);
        assert_eq!(stat.constraint(), Constraint::Length(4));
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
        assert_eq!(hints.constraint(), Constraint::Length(1));
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
        assert_eq!(list.constraint(), Constraint::Fill(1));
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
        assert_eq!(input.constraint(), Constraint::Length(1));
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

    // ---- Tabs ---------------------------------------------------------------------------

    #[test]
    fn tabs_underline_the_selected_label_and_nothing_else() {
        let selection = Selection::at(1);
        let tabs = Tabs::new(["ONE", "TWO"]).selection(&selection).focused(true);
        assert_eq!(tabs.constraint(), Constraint::Length(2));
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
        assert_eq!(tabs.constraint(), Constraint::Length(1));
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

    #[test]
    fn a_menu_too_small_to_frame_draws_nothing_rather_than_half_a_border() {
        let selection = Selection::new();
        for (width, height) in [(0, 0), (1, 1), (1, 4), (4, 1)] {
            let drawn = rows(&Menu::new(&selection, ["red"]), width, height);
            assert!(drawn.iter().all(|row| row.trim().is_empty()), "got {drawn:?}");
        }
    }
}
