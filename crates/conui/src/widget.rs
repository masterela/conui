//! The widget set.
//!
//! Deliberately small. Each of these exists because the reference design uses it more than
//! once, and every one of them is under fifty lines over the canvas — which is the point: if a
//! widget you need is missing, writing it is a `Paint` closure away, not a framework extension.

use conui_cell::{Padding, Rect, Style};
use unicode_segmentation::UnicodeSegmentation;

use crate::canvas::{Canvas, text_width};
use crate::layout::Constraint;
use crate::state::{Editor, Selection};
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
            if x >= i32::from(canvas.width()) {
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
        let text_x = marker_width + mark_width + u16::from(mark_width > 0);
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

        let value = self.editor.value();
        let cursor_column = self.editor.cursor_column();
        // Scroll only as far as needed to keep the cursor inside the field, leaving it room to
        // sit one past the last character — where it is when you are typing at the end.
        let scroll = cursor_column.saturating_sub(field - 1);
        let mut skipped = 0u16;
        let mut start = 0usize;
        for grapheme in value.graphemes(true) {
            if skipped >= scroll {
                break;
            }
            skipped += text_width(grapheme);
            start += grapheme.len();
        }

        if value.is_empty() {
            if let Some(placeholder) = &self.placeholder {
                canvas.put_truncated(i32::from(x), 0, placeholder, field, self.placeholder_role);
            }
        } else {
            // `put`, not `put_truncated`: an overlong value has scrolled out of view, and an
            // ellipsis would claim text was dropped when it is merely off to the left.
            canvas.put(i32::from(x), 0, &value[start..], self.role);
        }

        if self.cursor {
            let column = x + cursor_column.saturating_sub(skipped);
            if column < width {
                canvas.style_area(Rect::new(column, 0, 1, 1), Style::new().reverse());
            }
        }
    }

    fn constraint(&self) -> Constraint {
        Constraint::Length(1)
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
}
