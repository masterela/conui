//! The immediate-mode drawing surface.
//!
//! This is the layer everything else is built on, and it stays public on purpose. Declarative
//! widgets are a convenience for the ninety percent of a screen that is rows and panels; the
//! other ten percent is a game board, a waveform, a custom gauge — something nobody's widget
//! set anticipated. For that, you want to say "put these characters in these cells in this
//! colour" without inventing a type, and you want it to be exactly as fast as the widgets are.
//!
//! A [`Canvas`] is a view into a region of a [`Buffer`]. Coordinates are local to that region
//! and signed, so arithmetic that runs off the edge clips instead of panicking or wrapping —
//! `canvas.text(width - label.len() as i32, 0, label)` is safe even when the label is longer
//! than the region.
//!
//! ```
//! use conui::{Canvas, Role, Theme};
//! use conui_cell::{Buffer, Rect};
//!
//! let mut buffer = Buffer::new(20, 3);
//! let mut canvas = Canvas::new(&mut buffer, Rect::sized(20, 3), Theme::LAYA);
//! canvas.text(0, 0, "SCORE");
//! canvas.bar(0, 1, 0.5, 10, Role::Accent);
//! assert!(buffer.row_text(0).starts_with("SCORE"));
//! ```

use conui_cell::{Buffer, Cell, Color, Padding, Rect, Style, Symbol};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::{Role, Theme};
use crate::typography::{self, BarStyle, DIGIT_ADVANCE, DIGIT_HEIGHT, line};

/// A clipped, origin-shifted view into a [`Buffer`].
pub struct Canvas<'a> {
    buffer: &'a mut Buffer,
    /// Absolute buffer position of this canvas's local `(0, 0)`.
    origin_x: i32,
    origin_y: i32,
    /// The region's logical size, which may extend past the clip.
    width: u16,
    height: u16,
    /// Absolute region that writes are confined to.
    clip: Rect,
    theme: Theme,
}

impl<'a> Canvas<'a> {
    /// A canvas covering `area` of `buffer`.
    pub fn new(buffer: &'a mut Buffer, area: Rect, theme: Theme) -> Self {
        let clip = area.intersection(buffer.area());
        Self {
            buffer,
            origin_x: i32::from(area.x),
            origin_y: i32::from(area.y),
            width: area.width,
            height: area.height,
            clip,
            theme,
        }
    }

    /// The whole buffer, as a canvas.
    pub fn full(buffer: &'a mut Buffer, theme: Theme) -> Self {
        let area = buffer.area();
        Self::new(buffer, area, theme)
    }

    /// This canvas's own extent, in local coordinates, so its origin is always `(0, 0)`.
    pub const fn area(&self) -> Rect {
        Rect::sized(self.width, self.height)
    }

    /// This canvas's region in *buffer* coordinates.
    ///
    /// Almost nothing needs this — a widget that reads absolute coordinates has usually
    /// misunderstood that its own origin is `(0, 0)`. Two things genuinely do: something that
    /// wants to draw an overlay attached to where it landed (a dropdown's list, a tooltip), and
    /// something that wants to know whether a mouse position fell inside it. Both have to speak
    /// the buffer's coordinates, because both are resolved outside the view tree.
    pub fn screen_area(&self) -> Rect {
        let x = self.origin_x.clamp(0, u16::MAX as i32) as u16;
        let y = self.origin_y.clamp(0, u16::MAX as i32) as u16;
        Rect::new(x, y, self.width, self.height)
    }

    /// The part of this canvas's region that is actually on screen, in *buffer* coordinates.
    ///
    /// [`Canvas::screen_area`] is where the region claims to be; this is where it can be seen.
    /// The two differ whenever something above has clipped it — half a control in a window too
    /// narrow for it, or a row of a scrolled pane sitting above the fold, where the region's origin
    /// is negative and a `Rect` cannot say so. Empty when none of it was drawn.
    ///
    /// This is the one a click should be resolved against. A region that was clipped away was not
    /// on screen, and a pointer cannot have landed on it.
    pub fn visible_area(&self) -> Rect {
        // Intersected in signed space for the same reason `shifted` builds its clip that way: the
        // region's top-left can be negative, and clamping it to zero first would slide the region
        // down the screen into rows that belong to whatever is drawn there instead.
        let left = self.origin_x.max(i32::from(self.clip.x));
        let top = self.origin_y.max(i32::from(self.clip.y));
        let right = (self.origin_x + i32::from(self.width)).min(i32::from(self.clip.right()));
        let bottom = (self.origin_y + i32::from(self.height)).min(i32::from(self.clip.bottom()));
        if right <= left || bottom <= top {
            return Rect::ZERO;
        }
        Rect::new(left as u16, top as u16, (right - left) as u16, (bottom - top) as u16)
    }

    /// Columns this canvas spans, clipped or not.
    pub const fn width(&self) -> u16 {
        self.width
    }

    /// Rows this canvas spans, clipped or not.
    pub const fn height(&self) -> u16 {
        self.height
    }

    /// The palette in force, for drawing something a [`Role`] cannot express.
    pub const fn theme(&self) -> &Theme {
        &self.theme
    }

    /// The style for a role, over the theme's background.
    pub const fn role(&self, role: Role) -> Style {
        self.theme.style(role)
    }

    /// A nested canvas over `area`, which is interpreted in this canvas's coordinates.
    ///
    /// The child clips against the parent, so a sub-canvas can never draw outside the region
    /// it was given even if its own coordinates say otherwise. That is what makes it safe to
    /// hand a `Canvas` to code you did not write.
    pub fn sub(&mut self, area: Rect) -> Canvas<'_> {
        let origin_x = self.origin_x + i32::from(area.x);
        let origin_y = self.origin_y + i32::from(area.y);
        let requested = Rect::new(
            origin_x.clamp(0, i32::from(u16::MAX)) as u16,
            origin_y.clamp(0, i32::from(u16::MAX)) as u16,
            area.width,
            area.height,
        );
        Canvas {
            buffer: self.buffer,
            origin_x,
            origin_y,
            width: area.width,
            height: area.height,
            clip: requested.intersection(self.clip),
            theme: self.theme,
        }
    }

    /// A nested canvas inset from this one's edges.
    pub fn inset(&mut self, padding: Padding) -> Canvas<'_> {
        let area = self.area().inset(padding);
        self.sub(area)
    }

    /// A nested canvas at a signed offset, whose region may be larger than this one's.
    ///
    /// This is what a scroll viewport is, and it is why the canvas keeps its origin signed and its
    /// clip separate from its extent. The child is handed its *whole* height at an origin above the
    /// visible region — `shifted(0, -3, width, 40)` starts it three rows up — and the clip throws
    /// away what is out of sight. Nothing is re-laid-out and the child cannot tell: it draws row 0
    /// at its own row 0, exactly as it would if it were all on screen.
    ///
    /// Containment still holds. The clip is intersected with this canvas's, so a region that runs
    /// past the edge is cut off there rather than scribbling on a sibling.
    pub fn shifted(&mut self, x: i32, y: i32, width: u16, height: u16) -> Canvas<'_> {
        let origin_x = self.origin_x + x;
        let origin_y = self.origin_y + y;
        // Intersected in signed space, because the requested region's top-left can be negative and
        // a `Rect` cannot say so. Clamping first would move the region instead of cropping it.
        let left = origin_x.max(i32::from(self.clip.x));
        let top = origin_y.max(i32::from(self.clip.y));
        let right = (origin_x + i32::from(width)).min(i32::from(self.clip.right()));
        let bottom = (origin_y + i32::from(height)).min(i32::from(self.clip.bottom()));
        let clip = if right > left && bottom > top {
            Rect::new(left as u16, top as u16, (right - left) as u16, (bottom - top) as u16)
        } else {
            Rect::ZERO
        };
        Canvas { buffer: self.buffer, origin_x, origin_y, width, height, clip, theme: self.theme }
    }

    // ---- Text ---------------------------------------------------------------------------

    /// Write `text` in the body colour.
    pub fn text(&mut self, x: i32, y: i32, text: &str) -> u16 {
        self.put(x, y, text, Role::Text)
    }

    /// Write `text` in a role's colour.
    pub fn put(&mut self, x: i32, y: i32, text: &str, role: Role) -> u16 {
        let style = self.role(role);
        self.put_styled(x, y, text, style)
    }

    /// Write `text` with an explicit style, for the cases a role cannot express.
    pub fn put_styled(&mut self, x: i32, y: i32, text: &str, style: Style) -> u16 {
        let (abs_x, abs_y) = (self.origin_x + x, self.origin_y + y);
        self.write_str(abs_x, abs_y, text, style)
    }

    /// Write `text` so that it *ends* at local column `x`.
    ///
    /// Right-aligning by hand means measuring display width, which is not string length as
    /// soon as anything is non-ASCII. Doing it here means call sites cannot get it wrong.
    pub fn put_right(&mut self, x: i32, y: i32, text: &str, role: Role) -> u16 {
        let start = x - i32::from(text_width(text));
        self.put(start, y, text, role)
    }

    /// Write `text` centred within `[0, width)`.
    pub fn put_centered(&mut self, y: i32, text: &str, role: Role) -> u16 {
        let start = (i32::from(self.width) - i32::from(text_width(text))) / 2;
        self.put(start, y, text, role)
    }

    /// Write `text`, replacing its tail with `…` if it will not fit in `max_width`.
    ///
    /// Truncating mid-word without a marker makes a reading look complete when it is not,
    /// which is the kind of lie a dashboard must never tell.
    pub fn put_truncated(&mut self, x: i32, y: i32, text: &str, max_width: u16, role: Role) -> u16 {
        if text_width(text) <= max_width {
            return self.put(x, y, text, role);
        }
        if max_width == 0 {
            return 0;
        }
        if max_width == 1 {
            return self.put(x, y, &typography::mark::ELLIPSIS.to_string(), role);
        }
        let mut clipped = String::new();
        let mut used = 0u16;
        for grapheme in text.graphemes(true) {
            let width = Symbol::new(grapheme).width();
            if used + width > max_width - 1 {
                break;
            }
            used += width;
            clipped.push_str(grapheme);
        }
        clipped.push(typography::mark::ELLIPSIS);
        self.put(x, y, &clipped, role)
    }

    /// Write one character.
    pub fn set(&mut self, x: i32, y: i32, character: char, role: Role) {
        let style = self.role(role);
        self.write_str(self.origin_x + x, self.origin_y + y, &character.to_string(), style);
    }

    // ---- Fills and structure ------------------------------------------------------------

    /// Fill `area` with one character.
    pub fn fill(&mut self, area: Rect, character: char, role: Role) {
        let style = self.role(role);
        let symbol = Symbol::new(&character.to_string());
        let cell = Cell::new(symbol, style);
        let absolute = self.absolute(area);
        self.buffer.fill(absolute, cell);
    }

    /// Paint the whole canvas with the theme background, leaving the cells blank.
    pub fn clear(&mut self) {
        let style = self.theme.ground();
        self.buffer.fill(self.clip, Cell::new(Symbol::SPACE, style));
    }

    /// Recolour `area` without changing what it contains.
    ///
    /// Useful for selection highlights and for dimming a panel that has lost focus, neither of
    /// which should have to know how to redraw the content underneath.
    pub fn style_area(&mut self, area: Rect, style: Style) {
        let absolute = self.absolute(area);
        self.buffer.style_area(absolute, style);
    }

    /// Repeat `pattern` across every row of `area`.
    ///
    /// The board mesh in the reference design is `"· "` tiled over the play field: it gives
    /// the eye a grid to measure against while staying dark enough to read as empty.
    pub fn mesh(&mut self, area: Rect, pattern: &str, role: Role) {
        if pattern.is_empty() || area.is_empty() {
            return;
        }
        let style = self.role(role);
        let unit = text_width(pattern).max(1);
        let repeats = area.width.div_ceil(unit);
        let tiled: String = pattern.repeat(usize::from(repeats));
        for row in 0..area.height {
            let y = self.origin_y + i32::from(area.y) + i32::from(row);
            // A wide pattern may overhang; `write_str` clips it at the region edge.
            let clipped = clamp_to_width(&tiled, area.width);
            self.write_str(self.origin_x + i32::from(area.x), y, clipped, style);
        }
    }

    /// A horizontal rule.
    pub fn rule(&mut self, x: i32, y: i32, length: u16, role: Role) {
        self.run(x, y, line::HORIZONTAL, length, role);
    }

    /// A vertical rule.
    pub fn rule_vertical(&mut self, x: i32, y: i32, length: u16, role: Role) {
        let style = self.role(role);
        for offset in 0..i32::from(length) {
            self.write_str(
                self.origin_x + x,
                self.origin_y + y + offset,
                &line::VERTICAL.to_string(),
                style,
            );
        }
    }

    /// A run of one character.
    pub fn run(&mut self, x: i32, y: i32, character: char, length: u16, role: Role) {
        if length == 0 {
            return;
        }
        let style = self.role(role);
        let text: String = std::iter::repeat_n(character, usize::from(length)).collect();
        self.write_str(self.origin_x + x, self.origin_y + y, &text, style);
    }

    /// A single-line box around `area`, drawn on its edges.
    pub fn border(&mut self, area: Rect, role: Role) {
        self.border_with(area, role, false);
    }

    /// A box with rounded corners, which reads as softer and less structural.
    pub fn border_rounded(&mut self, area: Rect, role: Role) {
        self.border_with(area, role, true);
    }

    fn border_with(&mut self, area: Rect, role: Role, rounded: bool) {
        if area.width < 2 || area.height < 1 {
            return;
        }
        let (top_left, top_right, bottom_left, bottom_right) = if rounded {
            (
                line::ROUND_TOP_LEFT,
                line::ROUND_TOP_RIGHT,
                line::ROUND_BOTTOM_LEFT,
                line::ROUND_BOTTOM_RIGHT,
            )
        } else {
            (line::TOP_LEFT, line::TOP_RIGHT, line::BOTTOM_LEFT, line::BOTTOM_RIGHT)
        };
        let inner_width = area.width - 2;
        let (x, y) = (i32::from(area.x), i32::from(area.y));
        let last = y + i32::from(area.height) - 1;

        let top: String = format!(
            "{top_left}{}{top_right}",
            line::HORIZONTAL.to_string().repeat(usize::from(inner_width))
        );
        self.put(x, y, &top, role);
        if area.height >= 2 {
            let bottom: String = format!(
                "{bottom_left}{}{bottom_right}",
                line::HORIZONTAL.to_string().repeat(usize::from(inner_width))
            );
            self.put(x, last, &bottom, role);
        }
        for row in (y + 1)..last {
            self.set(x, row, line::VERTICAL, role);
            self.set(x + i32::from(area.width) - 1, row, line::VERTICAL, role);
        }
    }

    // ---- Quantities ----------------------------------------------------------------------

    /// A horizontal bar showing `value` in `0.0..=1.0`, in the default bar style.
    pub fn bar(&mut self, x: i32, y: i32, value: f32, length: u16, role: Role) {
        self.bar_with(x, y, value, length, BarStyle::default(), role, Role::Dim);
    }

    /// A bar with an explicit glyph style and separate fill and track colours.
    // Eight arguments, and each one is a separate fact a caller has to state: where, how much, how
    // long, drawn how, in what, over what. A params struct would move the noise to the call site
    // and make this primitive read unlike every other one on the canvas, which are all positional.
    #[allow(clippy::too_many_arguments)]
    pub fn bar_with(
        &mut self,
        x: i32,
        y: i32,
        value: f32,
        length: u16,
        style: BarStyle,
        fill: Role,
        track: Role,
    ) {
        if length == 0 {
            return;
        }
        if let Some(character) = style.track() {
            self.run(x, y, character, length, track);
        }
        let clamped = if value.is_nan() { 0.0 } else { value.clamp(0.0, 1.0) };

        if style.is_smooth() {
            // Eighth-cell resolution, so a slow-moving value still animates rather than
            // sitting still for eight percent and then jumping a whole column.
            let eighths = (clamped * f32::from(length) * 8.0).round() as u32;
            let whole = (eighths / 8) as u16;
            let partial = (eighths % 8) as usize;
            self.run(x, y, style.fill(), whole.min(length), fill);
            if partial > 0 && whole < length {
                self.set(x + i32::from(whole), y, typography::HORIZONTAL_LEVELS[partial], fill);
            }
            return;
        }
        let filled = (clamped * f32::from(length)).round() as u16;
        self.run(x, y, style.fill(), filled.min(length), fill);
    }

    /// A number in three-row block digits, the size a score wants to be.
    ///
    /// `min_digits` zero-pads, so a counter keeps a fixed footprint instead of shifting the
    /// layout every time it crosses a power of ten.
    pub fn number(&mut self, x: i32, y: i32, value: i64, min_digits: usize, role: Role) {
        let text = if value < 0 {
            format!("-{:0min_digits$}", value.unsigned_abs())
        } else {
            format!("{value:0min_digits$}")
        };
        self.digits(x, y, &text, role);
    }

    /// Draw `text` as three-row block glyphs.
    ///
    /// Characters with no large form are skipped rather than substituted, so the advance stays
    /// on the four-column grid and a partially renderable string still lines up. Use
    /// [`typography::is_renderable_large`] if you need to know in advance.
    pub fn digits(&mut self, x: i32, y: i32, text: &str, role: Role) {
        let style = self.role(role);
        for (index, character) in text.chars().enumerate() {
            let Some(rows) = typography::large_glyph(character) else { continue };
            let glyph_x = self.origin_x + x + (index as i32) * i32::from(DIGIT_ADVANCE);
            for (row, glyphs) in rows.iter().enumerate() {
                self.write_str(glyph_x, self.origin_y + y + row as i32, glyphs, style);
            }
        }
    }

    /// Columns and rows a [`Canvas::digits`] call will occupy.
    pub fn digits_size(text: &str) -> (u16, u16) {
        (typography::large_width(text), DIGIT_HEIGHT)
    }

    /// A one-row sparkline: one column per value, eight levels of height per column.
    ///
    /// Values are normalised against `max`; pass the series maximum for a self-scaling chart,
    /// or a fixed ceiling when the absolute level is what matters.
    pub fn sparkline(&mut self, x: i32, y: i32, values: &[f32], max: f32, role: Role) {
        if max <= 0.0 {
            return;
        }
        let style = self.role(role);
        for (index, value) in values.iter().enumerate() {
            let normalised = (value / max).clamp(0.0, 1.0);
            let level = (normalised * 8.0).round().clamp(0.0, 8.0) as usize;
            self.write_str(
                self.origin_x + x + index as i32,
                self.origin_y + y,
                &typography::VERTICAL_LEVELS[level].to_string(),
                style,
            );
        }
    }

    /// A run of cells whose colour sweeps from `from` to `to`.
    ///
    /// Drawn per-cell rather than as one span, which is what gives a snake body or a heat
    /// scale its smooth ramp. The diff engine still coalesces the output, so the cost is a few
    /// extra SGR sequences on the row that changed, not a full repaint.
    pub fn gradient(
        &mut self,
        x: i32,
        y: i32,
        character: char,
        length: u16,
        from: Color,
        to: Color,
    ) {
        if length == 0 {
            return;
        }
        let text = character.to_string();
        let last = f32::from(length.saturating_sub(1)).max(1.0);
        for index in 0..length {
            let progress = f32::from(index) / last;
            let style = Style::EMPTY.fg(from.lerp(to, progress)).bg(self.theme.background);
            self.write_str(self.origin_x + x + i32::from(index), self.origin_y + y, &text, style);
        }
    }

    // ---- Internals ----------------------------------------------------------------------

    /// Translate a local rect into buffer coordinates, clipped to this canvas.
    fn absolute(&self, area: Rect) -> Rect {
        let x = self.origin_x + i32::from(area.x);
        let y = self.origin_y + i32::from(area.y);
        // Clamp the origin into range before narrowing, then let `intersection` do the rest.
        let clamped = Rect::new(
            x.clamp(0, i32::from(u16::MAX)) as u16,
            y.clamp(0, i32::from(u16::MAX)) as u16,
            area.width,
            area.height,
        );
        clamped.intersection(self.clip)
    }

    /// Write text at absolute coordinates, clipping at both edges of the clip rect.
    fn write_str(&mut self, abs_x: i32, abs_y: i32, text: &str, style: Style) -> u16 {
        if self.clip.is_empty() {
            return 0;
        }
        let top = i32::from(self.clip.y);
        let bottom = i32::from(self.clip.bottom());
        if abs_y < top || abs_y >= bottom {
            return 0;
        }
        let y = abs_y as u16;
        let left = i32::from(self.clip.x);
        let right = i32::from(self.clip.right());

        let mut cursor = abs_x;
        let mut written = 0u16;
        for grapheme in text.graphemes(true) {
            if grapheme == "\n" || grapheme == "\r" {
                break;
            }
            if cursor >= right {
                break;
            }
            let symbol = Symbol::new(grapheme);
            let width = i32::from(symbol.width());
            if cursor + width <= left {
                // Entirely left of the clip; advance without drawing.
                cursor += width;
                continue;
            }
            if cursor < left || cursor + width > right {
                // A double-width glyph with only one half inside the clip. Neither half can be
                // drawn on its own, so the visible column gets a space: the grid stays aligned
                // and no partial glyph appears.
                let column = cursor.max(left);
                if column < right {
                    self.buffer.set_symbol(column as u16, y, Symbol::SPACE, style);
                    written += 1;
                }
                cursor += width;
                continue;
            }
            written += self.buffer.set_symbol(cursor as u16, y, symbol, style);
            cursor += width;
        }
        written
    }
}

/// Display width of `text` in terminal columns, respecting grapheme clusters.
pub fn text_width(text: &str) -> u16 {
    text.graphemes(true).map(|grapheme| Symbol::new(grapheme).width()).sum()
}

/// The longest prefix of `text` that fits in `max_width` columns.
fn clamp_to_width(text: &str, max_width: u16) -> &str {
    let mut used = 0u16;
    for (offset, grapheme) in text.grapheme_indices(true) {
        let width = Symbol::new(grapheme).width();
        if used + width > max_width {
            return &text[..offset];
        }
        used += width;
    }
    text
}

/// The block characters a filled cell can be drawn with, re-exported for convenience.
pub use crate::typography::block::FULL as SOLID;

/// Two full blocks: one logical square cell, since a terminal cell is twice as tall as wide.
///
/// Anything that wants square pixels — a game board, a bitmap, a maze — draws two columns per
/// logical cell. The reference design does exactly this, which is why its board is `width * 2`
/// columns wide.
pub const SQUARE: &str = "██";

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas_of(width: u16, height: u16) -> Buffer {
        Buffer::new(width, height)
    }

    fn draw(buffer: &mut Buffer, body: impl FnOnce(&mut Canvas<'_>)) {
        let mut canvas = Canvas::full(buffer, Theme::LAYA);
        body(&mut canvas);
    }

    #[test]
    fn text_lands_where_it_was_put() {
        let mut buffer = canvas_of(10, 2);
        draw(&mut buffer, |canvas| {
            canvas.text(2, 1, "hi");
        });
        assert_eq!(buffer.row_text(1), "  hi      ");
    }

    #[test]
    fn a_role_becomes_the_theme_colour() {
        let mut buffer = canvas_of(4, 1);
        draw(&mut buffer, |canvas| {
            canvas.put(0, 0, "x", Role::Accent);
        });
        let cell = buffer.get(0, 0).unwrap();
        assert_eq!(cell.style.fg, Some(Theme::LAYA.accent));
        assert_eq!(cell.style.bg, Some(Theme::LAYA.background));
    }

    #[test]
    fn negative_coordinates_clip_rather_than_panicking() {
        // This is the whole reason the API takes `i32`: `x - label.len()` underflows a `u16`,
        // and a panic in a draw call takes down the terminal mid-frame.
        let mut buffer = canvas_of(6, 1);
        draw(&mut buffer, |canvas| {
            canvas.text(-3, 0, "abcdef");
        });
        assert_eq!(buffer.row_text(0), "def   ");
    }

    #[test]
    fn writes_past_the_right_edge_are_truncated() {
        let mut buffer = canvas_of(5, 1);
        draw(&mut buffer, |canvas| {
            canvas.text(3, 0, "abcdef");
        });
        assert_eq!(buffer.row_text(0), "   ab");
    }

    #[test]
    fn writes_off_the_vertical_edges_are_dropped() {
        let mut buffer = canvas_of(4, 2);
        draw(&mut buffer, |canvas| {
            canvas.text(0, -1, "no");
            canvas.text(0, 9, "no");
            canvas.text(0, 0, "ok");
        });
        assert_eq!(buffer.row_text(0), "ok  ");
        assert_eq!(buffer.row_text(1), "    ");
    }

    #[test]
    fn a_sub_canvas_cannot_draw_outside_its_parent() {
        let mut buffer = canvas_of(10, 3);
        draw(&mut buffer, |canvas| {
            let mut inner = canvas.sub(Rect::new(2, 1, 3, 1));
            // Deliberately overrunning in both directions.
            inner.text(-5, 0, "XXXXXXXXXXXX");
            inner.text(0, -1, "up");
            inner.text(0, 5, "down");
        });
        assert_eq!(buffer.row_text(0), "          ", "the parent row above stayed clean");
        assert_eq!(buffer.row_text(1), "  XXX     ");
        assert_eq!(buffer.row_text(2), "          ");
    }

    #[test]
    fn a_sub_canvas_reports_its_own_coordinate_space() {
        let mut buffer = canvas_of(10, 4);
        draw(&mut buffer, |canvas| {
            let inner = canvas.sub(Rect::new(3, 1, 4, 2));
            assert_eq!(inner.area(), Rect::sized(4, 2));
            assert_eq!((inner.width(), inner.height()), (4, 2));
        });
    }

    #[test]
    fn nested_sub_canvases_compose_their_offsets() {
        let mut buffer = canvas_of(10, 4);
        draw(&mut buffer, |canvas| {
            let mut first = canvas.sub(Rect::new(2, 1, 6, 3));
            let mut second = first.sub(Rect::new(1, 1, 4, 1));
            second.text(0, 0, "ab");
        });
        assert_eq!(buffer.row_text(2), "   ab     ");
    }

    #[test]
    fn a_shifted_canvas_draws_its_whole_self_and_shows_a_window_of_it() {
        let mut buffer = canvas_of(6, 2);
        draw(&mut buffer, |canvas| {
            // Six rows of content, shifted up by two, into a two-row window.
            let mut inner = canvas.shifted(0, -2, 6, 6);
            assert_eq!(inner.height(), 6, "the child is told its real height");
            for row in 0..6 {
                inner.text(0, row, &format!("row {row}"));
            }
        });
        assert_eq!(buffer.row_text(0), "row 2 ");
        assert_eq!(buffer.row_text(1), "row 3 ");
    }

    #[test]
    fn a_shifted_canvas_is_still_confined_to_its_parent() {
        // The containment guarantee has to survive the one sub-canvas that is allowed to be
        // bigger than its region, or a scrolled panel could scribble on the rest of the screen.
        let mut buffer = canvas_of(10, 5);
        draw(&mut buffer, |canvas| {
            let mut middle = canvas.sub(Rect::new(2, 2, 4, 1));
            let mut inner = middle.shifted(0, -1, 4, 9);
            for row in 0..9 {
                inner.run(0, row, '#', 4, Role::Text);
            }
            inner.text(-4, 1, "left");
        });
        assert_eq!(buffer.row_text(1), "          ", "nothing above the region");
        assert_eq!(buffer.row_text(2), "  ####    ");
        assert_eq!(buffer.row_text(3), "          ", "nothing below it either");
    }

    #[test]
    fn a_shifted_canvas_scrolled_past_its_content_shows_nothing_rather_than_panicking() {
        let mut buffer = canvas_of(6, 2);
        draw(&mut buffer, |canvas| {
            let mut inner = canvas.shifted(0, -99, 6, 3);
            inner.text(0, 0, "gone");
        });
        assert_eq!(buffer.row_text(0), "      ");
    }

    #[test]
    fn right_alignment_measures_display_width_not_byte_length() {
        let mut buffer = canvas_of(10, 1);
        draw(&mut buffer, |canvas| {
            // Four bytes, two columns.
            canvas.put_right(10, 0, "界", Role::Text);
        });
        assert_eq!(buffer.row_text(0), "        界");
    }

    #[test]
    fn centering_splits_the_slack() {
        let mut buffer = canvas_of(9, 1);
        draw(&mut buffer, |canvas| {
            canvas.put_centered(0, "abc", Role::Text);
        });
        assert_eq!(buffer.row_text(0), "   abc   ");
    }

    #[test]
    fn truncation_marks_itself() {
        let mut buffer = canvas_of(10, 1);
        draw(&mut buffer, |canvas| {
            canvas.put_truncated(0, 0, "a very long label", 6, Role::Text);
        });
        assert_eq!(buffer.row_text(0), "a ver…    ");
    }

    #[test]
    fn truncation_leaves_a_fitting_string_alone() {
        let mut buffer = canvas_of(10, 1);
        draw(&mut buffer, |canvas| {
            canvas.put_truncated(0, 0, "short", 6, Role::Text);
        });
        assert_eq!(buffer.row_text(0), "short     ");
    }

    #[test]
    fn a_wide_glyph_half_outside_the_clip_becomes_a_space() {
        // Drawing half of `界` would desynchronise every column after it, so the canvas
        // substitutes a space and keeps the grid intact.
        let mut buffer = canvas_of(3, 1);
        draw(&mut buffer, |canvas| {
            canvas.text(2, 0, "界x");
        });
        assert_eq!(buffer.row_text(0), "   ");
    }

    // ---- Quantities ----------------------------------------------------------------------

    #[test]
    fn a_bar_fills_proportionally_over_a_full_track() {
        let mut buffer = canvas_of(10, 1);
        draw(&mut buffer, |canvas| {
            canvas.bar(0, 0, 0.4, 10, Role::Accent);
        });
        assert_eq!(buffer.row_text(0), "━━━━━━━━━━", "the track spans the whole length");
        assert_eq!(buffer.get(3, 0).unwrap().style.fg, Some(Theme::LAYA.accent));
        assert_eq!(buffer.get(4, 0).unwrap().style.fg, Some(Theme::LAYA.dim));
    }

    #[test]
    fn a_bar_clamps_out_of_range_and_non_finite_values() {
        let mut buffer = canvas_of(4, 3);
        draw(&mut buffer, |canvas| {
            canvas.bar(0, 0, 5.0, 4, Role::Accent);
            canvas.bar(0, 1, -2.0, 4, Role::Accent);
            canvas.bar(0, 2, f32::NAN, 4, Role::Accent);
        });
        let accent = Some(Theme::LAYA.accent);
        let dim = Some(Theme::LAYA.dim);
        assert_eq!(buffer.get(3, 0).unwrap().style.fg, accent, "over-range fills completely");
        assert_eq!(buffer.get(0, 1).unwrap().style.fg, dim, "under-range fills nothing");
        assert_eq!(buffer.get(0, 2).unwrap().style.fg, dim, "NaN is treated as zero");
    }

    #[test]
    fn a_shaded_bar_distinguishes_fill_from_track_by_glyph() {
        let mut buffer = canvas_of(8, 1);
        draw(&mut buffer, |canvas| {
            canvas.bar_with(0, 0, 0.5, 8, BarStyle::Shaded, Role::Accent, Role::Dim);
        });
        assert_eq!(buffer.row_text(0), "████░░░░");
    }

    #[test]
    fn a_block_bar_leaves_no_track_behind() {
        let mut buffer = canvas_of(6, 1);
        draw(&mut buffer, |canvas| {
            canvas.bar_with(0, 0, 0.5, 6, BarStyle::Blocks, Role::Accent, Role::Dim);
        });
        assert_eq!(buffer.row_text(0), "███   ");
    }

    #[test]
    fn a_smooth_bar_moves_within_a_single_column() {
        let mut buffer = canvas_of(4, 2);
        draw(&mut buffer, |canvas| {
            canvas.bar_with(0, 0, 0.05, 4, BarStyle::Smooth, Role::Accent, Role::Dim);
            canvas.bar_with(0, 1, 0.20, 4, BarStyle::Smooth, Role::Accent, Role::Dim);
        });
        // Both are under one full column, but they are visibly different.
        assert_ne!(buffer.row_text(0), buffer.row_text(1));
        assert_eq!(buffer.row_text(0).trim_end(), "▎");
        assert_eq!(buffer.row_text(1).trim_end(), "▊");
    }

    #[test]
    fn a_zero_length_bar_draws_nothing() {
        let mut buffer = canvas_of(4, 1);
        draw(&mut buffer, |canvas| {
            canvas.bar(0, 0, 1.0, 0, Role::Accent);
        });
        assert_eq!(buffer.row_text(0), "    ");
    }

    #[test]
    fn block_digits_render_three_rows_on_a_four_column_advance() {
        let mut buffer = canvas_of(12, 3);
        draw(&mut buffer, |canvas| {
            canvas.number(0, 0, 7, 3, Role::Accent);
        });
        assert_eq!(buffer.row_text(0), "█▀█ █▀█ ▀▀█ ");
        assert_eq!(buffer.row_text(1), "█ █ █ █   █ ");
        assert_eq!(buffer.row_text(2), "▀▀▀ ▀▀▀   ▀ ");
    }

    #[test]
    fn a_number_keeps_a_fixed_footprint_as_it_grows() {
        assert_eq!(Canvas::digits_size("007"), (11, 3));
        // Zero padding is the point: a counter must occupy the same columns at 7 as at 123,
        // or the whole panel jumps sideways every time it crosses a power of ten.
        let mut narrow = canvas_of(16, 3);
        let mut wide = canvas_of(16, 3);
        draw(&mut narrow, |canvas| canvas.number(0, 0, 7, 3, Role::Text));
        draw(&mut wide, |canvas| canvas.number(0, 0, 123, 3, Role::Text));
        for buffer in [&narrow, &wide] {
            for row in 0..3 {
                let text = buffer.row_text(row);
                let beyond: String = text.chars().skip(11).collect();
                assert!(beyond.trim().is_empty(), "row {row} spilled past 11 columns: {text:?}");
            }
        }
    }

    #[test]
    fn an_unrenderable_character_is_skipped_without_shifting_the_rest() {
        let mut buffer = canvas_of(12, 3);
        draw(&mut buffer, |canvas| {
            canvas.digits(0, 0, "1x2", Role::Text);
        });
        // The `x` occupies its slot but draws nothing, so `2` stays on the grid.
        assert_eq!(buffer.row_text(0), "▄█      ▀▀█ ");
    }

    #[test]
    fn a_mesh_tiles_a_pattern_across_a_region() {
        let mut buffer = canvas_of(8, 2);
        draw(&mut buffer, |canvas| {
            canvas.mesh(Rect::new(1, 0, 6, 2), "· ", Role::Surface);
        });
        assert_eq!(buffer.row_text(0), " · · ·  ");
        assert_eq!(buffer.row_text(1), " · · ·  ");
    }

    #[test]
    fn a_mesh_with_an_odd_width_does_not_overrun_its_region() {
        let mut buffer = canvas_of(8, 1);
        draw(&mut buffer, |canvas| {
            canvas.mesh(Rect::new(0, 0, 5, 1), "· ", Role::Surface);
        });
        assert_eq!(buffer.row_text(0), "· · ·   ", "the sixth column stays untouched");
    }

    #[test]
    fn a_border_frames_a_region_without_filling_it() {
        let mut buffer = canvas_of(6, 4);
        draw(&mut buffer, |canvas| {
            canvas.border(Rect::new(0, 0, 5, 3), Role::Dim);
        });
        assert_eq!(buffer.row_text(0), "┌───┐ ");
        assert_eq!(buffer.row_text(1), "│   │ ");
        assert_eq!(buffer.row_text(2), "└───┘ ");
    }

    #[test]
    fn a_rounded_border_uses_soft_corners() {
        let mut buffer = canvas_of(5, 2);
        draw(&mut buffer, |canvas| {
            canvas.border_rounded(Rect::new(0, 0, 4, 2), Role::Dim);
        });
        assert_eq!(buffer.row_text(0), "╭──╮ ");
        assert_eq!(buffer.row_text(1), "╰──╯ ");
    }

    #[test]
    fn a_degenerate_border_draws_nothing_rather_than_a_stray_corner() {
        let mut buffer = canvas_of(4, 2);
        draw(&mut buffer, |canvas| {
            canvas.border(Rect::new(0, 0, 1, 2), Role::Dim);
            canvas.border(Rect::new(0, 0, 4, 0), Role::Dim);
        });
        assert_eq!(buffer.row_text(0), "    ");
    }

    #[test]
    fn a_sparkline_uses_eight_levels_per_column() {
        let mut buffer = canvas_of(5, 1);
        draw(&mut buffer, |canvas| {
            canvas.sparkline(0, 0, &[0.0, 0.25, 0.5, 0.75, 1.0], 1.0, Role::Info);
        });
        assert_eq!(buffer.row_text(0), " ▂▄▆█");
    }

    #[test]
    fn a_sparkline_with_no_scale_draws_nothing_rather_than_dividing_by_zero() {
        let mut buffer = canvas_of(3, 1);
        draw(&mut buffer, |canvas| {
            canvas.sparkline(0, 0, &[1.0, 2.0, 3.0], 0.0, Role::Info);
        });
        assert_eq!(buffer.row_text(0), "   ");
    }

    #[test]
    fn a_gradient_gives_each_cell_its_own_colour() {
        let mut buffer = canvas_of(5, 1);
        let (from, to) = (Color::rgb(0, 0, 0), Color::rgb(255, 255, 255));
        draw(&mut buffer, |canvas| {
            canvas.gradient(0, 0, '█', 5, from, to);
        });
        assert_eq!(buffer.get(0, 0).unwrap().style.fg, Some(from));
        assert_eq!(buffer.get(4, 0).unwrap().style.fg, Some(to));
        let middle = buffer.get(2, 0).unwrap().style.fg.unwrap();
        assert_ne!(middle, from);
        assert_ne!(middle, to);
    }

    #[test]
    fn clearing_paints_only_this_canvas_region() {
        let mut buffer = Buffer::filled(6, 2, Cell::new(Symbol::new("#"), Style::EMPTY));
        draw(&mut buffer, |canvas| {
            let mut inner = canvas.sub(Rect::new(1, 0, 2, 1));
            inner.clear();
        });
        assert_eq!(buffer.row_text(0), "#  ###");
        assert_eq!(buffer.row_text(1), "######");
    }

    #[test]
    fn a_canvas_larger_than_its_buffer_clips_instead_of_growing_it() {
        let mut buffer = canvas_of(4, 1);
        let mut canvas = Canvas::new(&mut buffer, Rect::sized(40, 10), Theme::LAYA);
        canvas.text(0, 0, "abcdefgh");
        assert_eq!(buffer.width(), 4, "the buffer must not be resized by a draw call");
        assert_eq!(buffer.row_text(0), "abcd");
    }

    #[test]
    fn a_canvas_entirely_outside_the_buffer_is_a_no_op() {
        let mut buffer = canvas_of(4, 1);
        let mut canvas = Canvas::new(&mut buffer, Rect::new(10, 10, 5, 5), Theme::LAYA);
        canvas.text(0, 0, "abc");
        canvas.bar(0, 0, 1.0, 5, Role::Accent);
        canvas.clear();
        assert_eq!(buffer.row_text(0), "    ");
    }

    #[test]
    fn text_width_counts_columns_not_characters() {
        assert_eq!(text_width("abc"), 3);
        assert_eq!(text_width("界界"), 4);
        assert_eq!(text_width(""), 0);
    }

    #[test]
    fn a_canvas_knows_where_it_is_even_though_it_draws_as_if_it_were_at_the_origin() {
        let mut buffer = Buffer::new(20, 10);
        let mut canvas = Canvas::new(&mut buffer, Rect::new(2, 3, 8, 4), Theme::LAYA);
        assert_eq!(canvas.area(), Rect::sized(8, 4), "local coordinates start at the origin");
        assert_eq!(canvas.screen_area(), Rect::new(2, 3, 8, 4));

        let nested = canvas.sub(Rect::new(1, 1, 4, 2));
        assert_eq!(nested.screen_area(), Rect::new(3, 4, 4, 2), "nesting accumulates");
    }

    #[test]
    fn a_canvas_pushed_off_the_left_edge_reports_a_position_on_screen() {
        let mut buffer = Buffer::new(20, 10);
        let mut canvas = Canvas::full(&mut buffer, Theme::LAYA);
        let nested = canvas.sub(Rect::new(0, 0, 4, 1));
        // A negative origin is reachable through repeated insets; it must not wrap to 65535.
        assert_eq!(nested.screen_area().x, 0);
    }

    #[test]
    fn a_canvas_that_fits_is_visible_exactly_where_it_says_it_is() {
        let mut buffer = Buffer::new(20, 10);
        let canvas = Canvas::new(&mut buffer, Rect::new(2, 3, 8, 4), Theme::LAYA);
        assert_eq!(canvas.visible_area(), canvas.screen_area());
    }

    #[test]
    fn a_canvas_clipped_by_its_parent_is_visible_only_where_it_shows() {
        let mut buffer = Buffer::new(20, 10);
        let mut canvas = Canvas::new(&mut buffer, Rect::new(0, 0, 6, 2), Theme::LAYA);
        // A child taller and wider than the region it was given: it draws inside, but only the
        // part inside can be seen, and a click on the rest would land on whatever is out there.
        let child = canvas.sub(Rect::new(0, 0, 10, 5));
        assert_eq!(child.screen_area(), Rect::new(0, 0, 10, 5), "what the region claims");
        assert_eq!(child.visible_area(), Rect::new(0, 0, 6, 2), "what shows");
    }

    #[test]
    fn a_canvas_above_the_fold_of_a_scrolled_region_is_visible_nowhere() {
        let mut buffer = Buffer::new(8, 6);
        let mut canvas = Canvas::new(&mut buffer, Rect::new(0, 2, 8, 3), Theme::LAYA);
        let mut content = canvas.shifted(0, -4, 8, 9);
        // The first two rows of the content are scrolled past the top of the pane.
        let gone = content.sub(Rect::new(0, 0, 8, 2));
        assert!(gone.visible_area().is_empty(), "got {:?}", gone.visible_area());
        // Clamping instead of intersecting in signed space would have said row 0, which on this
        // screen is two rows above the pane and belongs to whatever is drawn there.
        assert_eq!(gone.screen_area(), Rect::new(0, 0, 8, 2), "which is why this one is a lie");

        let straddling = content.sub(Rect::new(0, 3, 8, 3));
        assert_eq!(straddling.visible_area(), Rect::new(0, 2, 8, 2), "the rows inside the pane");
    }
}
