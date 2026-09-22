//! The state a widget needs you to keep.
//!
//! Views in conui are values rebuilt every frame, which is what makes them cheap and impossible
//! to desynchronise from your data. That works right up to the first widget with a memory: a list
//! remembers which row is selected and how far it has scrolled, a text field remembers where the
//! cursor is. Those cannot live in a value that is thrown away sixty times a second.
//!
//! So they live here, in small plain structs *you* own and mutate, which the matching view
//! borrows to draw. The widget keeps no secrets: selection is a `usize` you can set, and an
//! editor's text is a `String` you can read. Nothing is hidden behind a handle, and a keybinding
//! you want to work differently is a method call away rather than a fork.
//!
//! ```
//! use conui::state::{Editor, Selection};
//!
//! let mut selection = Selection::new();
//! selection.down(3);
//! assert_eq!(selection.selected(), 1);
//!
//! let mut editor = Editor::with("milk");
//! editor.insert(' ');
//! editor.insert_str("and eggs");
//! assert_eq!(editor.value(), "milk and eggs");
//! ```

use std::cell::{Cell, RefCell};
use std::ops::Range;

use conui_cell::{Padding, Pos, Rect};
use conui_input::{KeyCode, KeyEvent, KeyEventKind, Modifiers, MouseEvent};
use unicode_segmentation::UnicodeSegmentation;

use crate::canvas::text_width;

// ---- Selection --------------------------------------------------------------------------

/// Which row of a list is current, and where the visible window sits.
///
/// The scroll offset is deliberately not yours to manage. It is only knowable at render time —
/// it depends on the region's height, which layout decides after you have finished handling
/// input — so [`Selection::window`] computes it during the draw and remembers it here. Move the
/// selection anywhere you like and the view scrolls to follow it; there is no
/// `scroll_into_view` to forget to call.
#[derive(Debug, Default)]
pub struct Selection {
    selected: usize,
    /// Index of the first visible row. Interior mutability because rendering, which takes
    /// `&self`, is the only place the window's height is known.
    offset: Cell<usize>,
    /// Visible rows, as of the last draw. Recorded for the same reason and used for the same
    /// thing: so that [`Selection::page_up`] means a page rather than a number the caller had to
    /// invent.
    height: Cell<usize>,
}

impl Clone for Selection {
    fn clone(&self) -> Self {
        Self {
            selected: self.selected,
            offset: Cell::new(self.offset.get()),
            height: Cell::new(self.height.get()),
        }
    }
}

impl Selection {
    /// Row zero selected, nothing scrolled.
    pub fn new() -> Self {
        Self::default()
    }

    /// A selection starting on a given row.
    pub fn at(index: usize) -> Self {
        Self { selected: index, ..Self::default() }
    }

    /// The selected row.
    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// The first visible row, as of the last render.
    pub fn offset(&self) -> usize {
        self.offset.get()
    }

    /// Select a row without checking it exists; [`Selection::clamp`] is what checks.
    pub fn set_selected(&mut self, index: usize) {
        self.selected = index;
    }

    /// Pull the selection back inside a list of `len` rows.
    ///
    /// Worth calling after the data changes under you — a filter narrowing, a refresh returning
    /// fewer rows — so the selection cannot point past the end.
    pub fn clamp(&mut self, len: usize) {
        self.selected = self.selected.min(len.saturating_sub(1));
    }

    /// Up one row, stopping at the first. Does not wrap: an arrow key that jumps to the far end
    /// of a list reads as a glitch rather than as a feature.
    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Down one row, stopping at the last of `len`.
    pub fn down(&mut self, len: usize) {
        if self.selected + 1 < len {
            self.selected += 1;
        }
    }

    /// Move up, wrapping to the last row from the first.
    ///
    /// The right feel for a short menu, where running off the top and stopping feels broken; the
    /// wrong one for a thousand-line log, where it loses your place.
    pub fn cycle_up(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        self.selected = if self.selected == 0 { len - 1 } else { self.selected - 1 };
    }

    /// Move down, wrapping to the first row from the last.
    pub fn cycle_down(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        self.selected = (self.selected + 1) % len;
    }

    /// Select the first row. What `HOME` does.
    pub fn first(&mut self) {
        self.selected = 0;
    }

    /// Select the last of `len` rows. What `END` does.
    pub fn last(&mut self, len: usize) {
        self.selected = len.saturating_sub(1);
    }

    /// Up a windowful, less one row of overlap so the eye has something to land on.
    ///
    /// The page is the height the list was last drawn at, which is the only number that is
    /// actually a page: it is the region layout handed the list, and nothing outside the draw knows
    /// it. A caller passing its own row count is guessing, and the guess is wrong the moment the
    /// window is resized. Before the first draw, and for a selection no list ever drew, a page is
    /// one row.
    pub fn page_up(&mut self) {
        self.selected = self.selected.saturating_sub(self.page());
    }

    /// Down a windowful of the last drawn height, less one row of overlap, stopping at the last of
    /// `len`.
    pub fn page_down(&mut self, len: usize) {
        self.selected = (self.selected + self.page()).min(len.saturating_sub(1));
    }

    /// Visible rows, as of the last draw. Zero until a list has drawn this selection.
    pub fn height(&self) -> usize {
        self.height.get()
    }

    /// Rows a page moves by: the window, less a row of context, and never nothing.
    fn page(&self) -> usize {
        self.height.get().saturating_sub(1).max(1)
    }

    /// Adjust after the row at `index` was removed, `len` being the length that remains.
    ///
    /// Selection stays put so the next item slides under it, which is what you want when
    /// clearing a list one row at a time — except at the end, where it steps back instead.
    pub fn removed(&mut self, index: usize, len: usize) {
        if index < self.selected {
            self.selected -= 1;
        }
        self.clamp(len);
    }

    /// Move by `delta` rows, negative up, clamped at both ends. What a mouse wheel does.
    ///
    /// The wheel moves the *selection*, not a window of its own, because [`Selection::window`]
    /// always contains the selection: an offset nudged on its own would be pulled straight back by
    /// the next draw, so a wheel that only scrolled would look broken. A list that scrolls away
    /// from its cursor is a different widget — it needs a viewport holding an offset nothing else
    /// owns — and this is not it.
    pub fn step(&mut self, delta: i32, len: usize) {
        if delta < 0 {
            self.selected = self.selected.saturating_sub(delta.unsigned_abs() as usize);
        } else {
            self.selected = self.selected.saturating_add(delta as usize).min(len.saturating_sub(1));
        }
    }

    /// Put row `offset` at the top of the window, bringing the selection with it.
    ///
    /// The counterpart to [`Selection::offset`], and what a draggable scrollbar beside a list needs:
    /// `Scrollbar::offset_at` turns a pointer row into an offset, and this is what accepts one.
    ///
    /// It has to move the selection too, and that is the whole reason this is not a plain
    /// `set_offset`. [`Selection::window`] scrolls to keep the selection visible, so an offset set on
    /// its own would be undone by the very next draw — the window would snap back to wherever the
    /// cursor still was, and the thumb would spring out from under the pointer. So the cursor comes
    /// along: it moves by the smallest amount that puts it inside the new window, and no further.
    /// Drag a list's bar and the selection travels with the view, which is the behaviour you would
    /// have had to write by hand anyway.
    ///
    /// Clamped against the height and length of the last draw, like everything else here. Before the
    /// first draw there is no window to scroll, and this does nothing.
    pub fn scroll_to(&mut self, offset: usize, len: usize) {
        let height = self.height.get();
        if height == 0 || len == 0 {
            return;
        }
        let offset = offset.min(len.saturating_sub(height));
        self.offset.set(offset);
        let last = (offset + height - 1).min(len - 1);
        self.selected = self.selected.clamp(offset, last);
    }

    /// The rows a list of `len` items should draw into `height` rows, scrolling to keep the
    /// selection visible, and recording the offset and the height for next frame.
    pub fn window(&self, height: u16, len: usize) -> Range<usize> {
        let height = usize::from(height);
        // Recorded even when there is nothing to draw into it, because the next key press asks
        // what a page is and an empty list still has a height.
        self.height.set(height);
        if height == 0 || len == 0 {
            return 0..0;
        }
        let mut offset = self.offset.get();
        // Follow the selection, by the smallest scroll that brings it back into view.
        if self.selected < offset {
            offset = self.selected;
        } else if self.selected >= offset + height {
            offset = self.selected + 1 - height;
        }
        // Never leave blank rows at the bottom while rows remain above: a list that can show
        // ten of twelve items should never be scrolled to show only the last two.
        offset = offset.min(len.saturating_sub(height));
        self.offset.set(offset);
        offset..(offset + height).min(len)
    }

    /// Which item a click at `pos` landed on, given the region the list drew into.
    ///
    /// Uses the offset recorded by the last [`Selection::window`], which is the only honest source:
    /// the row under the pointer means nothing without knowing how far the list had scrolled when
    /// it was drawn. `None` for a click outside the region or below the last item — clicking the
    /// blank space under a short list should do nothing, not select the last row.
    pub fn row_at(&self, area: Rect, pos: Pos, len: usize) -> Option<usize> {
        if !area.contains(pos) {
            return None;
        }
        let index = self.offset.get() + usize::from(pos.y - area.y);
        (index < len).then_some(index)
    }
}

// ---- Viewport ---------------------------------------------------------------------------

/// How far a scrolled region has been scrolled.
///
/// The counterpart to [`Selection`], for the case where there is no selection: a pane of text, a
/// log, a form longer than its panel. Here the offset genuinely is the state — nothing else owns a
/// cursor for it to follow — which is why this is the one thing in conui with an offset you move
/// directly, and why a mouse wheel over a [`Scroll`](crate::view::Scroll) does what a wheel
/// normally does instead of moving a highlight.
///
/// The bounds are not yours to restate. How many rows are visible depends on the region layout
/// hands the viewport, and how many rows exist depends on the content, so both are only knowable
/// during the draw: [`Viewport::window`] records them, and every method here clamps against what
/// the last frame actually showed. Scroll it wherever you like between frames; it cannot end up
/// past the end.
#[derive(Debug, Default)]
pub struct Viewport {
    /// First visible row of the content.
    offset: Cell<u16>,
    /// The visible height and the content height, as of the last draw. Interior mutability for
    /// the same reason [`Selection::offset`] needs it.
    height: Cell<u16>,
    content: Cell<u16>,
}

impl Clone for Viewport {
    fn clone(&self) -> Self {
        Self {
            offset: Cell::new(self.offset.get()),
            height: Cell::new(self.height.get()),
            content: Cell::new(self.content.get()),
        }
    }
}

impl Viewport {
    /// Scrolled to the top, with nothing yet known about the height or the content.
    pub fn new() -> Self {
        Self::default()
    }

    /// A viewport already scrolled to a row. Clamped at the next draw if the content is shorter.
    pub fn at(row: u16) -> Self {
        Self { offset: Cell::new(row), ..Self::default() }
    }

    /// The first visible row of the content.
    pub fn offset(&self) -> u16 {
        self.offset.get()
    }

    /// Rows of content that the region could not show, as of the last draw. Zero when it all fits.
    pub fn overflow(&self) -> u16 {
        self.content.get().saturating_sub(self.height.get())
    }

    /// Visible rows, as of the last draw.
    pub fn height(&self) -> u16 {
        self.height.get()
    }

    /// Rows of content, as of the last draw.
    pub fn content(&self) -> u16 {
        self.content.get()
    }

    /// Whether the first row of content is showing.
    pub fn is_at_top(&self) -> bool {
        self.offset.get() == 0
    }

    /// Whether the last row of content is showing, as of the last draw.
    pub fn is_at_bottom(&self) -> bool {
        self.offset.get() >= self.overflow()
    }

    /// Whether anything is out of sight in either direction — whether a scrollbar has anything to
    /// say, and whether the keys that scroll are worth putting in the legend.
    pub fn is_scrollable(&self) -> bool {
        self.overflow() > 0
    }

    /// Jump to a row, clamped to what the last draw could show.
    pub fn set_offset(&self, row: u16) {
        self.offset.set(row.min(self.limit()));
    }

    /// The furthest the offset may go, or unbounded before the first draw has said.
    ///
    /// Scrolling before anything has been drawn is legitimate — restoring a saved position, opening
    /// at a line named on the command line — and the alternative is silently doing nothing until the
    /// second frame, which is the kind of bug that gets blamed on the terminal. [`Viewport::window`]
    /// clamps either way, so nothing can survive to the screen out of range.
    fn limit(&self) -> u16 {
        if self.height.get() == 0 { u16::MAX } else { self.overflow() }
    }

    /// Move by `delta` rows, negative up. What a mouse wheel does.
    pub fn scroll(&self, delta: i32) {
        let row = i64::from(self.offset.get()) + i64::from(delta);
        self.set_offset(row.clamp(0, i64::from(u16::MAX)) as u16);
    }

    /// Up or down by a windowful, less one row of overlap so the eye has something to land on.
    pub fn page_up(&self) {
        self.scroll(-i32::from(self.page()));
    }

    /// Down by a windowful, less the same row of overlap.
    pub fn page_down(&self) {
        self.scroll(i32::from(self.page()));
    }

    fn page(&self) -> u16 {
        self.height.get().saturating_sub(1).max(1)
    }

    /// Scroll to the first row of content.
    pub fn top(&self) {
        self.offset.set(0);
    }

    /// Scroll to the last row of content, or as far as it will go before the first draw has said
    /// how far that is.
    pub fn bottom(&self) {
        self.offset.set(self.limit());
    }

    /// Scroll the least that brings content row `row` into view, and no further.
    ///
    /// For following something the user did not scroll to themselves — a search hit, a new line in
    /// a log, the field an error is attached to. Jumping further than needed loses their place.
    pub fn reveal(&self, row: u16) {
        let (offset, height) = (self.offset.get(), self.height.get());
        if row < offset {
            self.set_offset(row);
        } else if height > 0 && row >= offset + height {
            self.set_offset(row + 1 - height);
        }
    }

    /// Record the geometry of a draw and return the offset to draw at.
    ///
    /// Called by [`Scroll`](crate::view::Scroll), not by you. Clamps as it goes, so a viewport left
    /// scrolled to the bottom of a long document shows the *end* of a short one rather than nothing
    /// at all — the failure mode of a stale offset is a blank pane, which reads as a broken program.
    pub fn window(&self, height: u16, content: u16) -> u16 {
        self.height.set(height);
        self.content.set(content);
        let offset = self.offset.get().min(content.saturating_sub(height));
        self.offset.set(offset);
        offset
    }
}

// ---- Editor -----------------------------------------------------------------------------

/// A single line of editable text and a cursor within it.
///
/// Movement and deletion work in grapheme clusters, not bytes or `char`s, so an emoji or a
/// combining accent deletes as the one thing the user sees rather than coming apart.
#[derive(Clone, Debug, Default)]
pub struct Editor {
    value: String,
    /// Byte offset of the cursor, always on a grapheme boundary.
    cursor: usize,
}

impl Editor {
    /// An empty field.
    pub fn new() -> Self {
        Self::default()
    }

    /// An editor holding `text`, cursor at the end.
    pub fn with(text: impl Into<String>) -> Self {
        let value: String = text.into();
        let cursor = value.len();
        Self { value, cursor }
    }

    /// The text as it stands.
    pub fn value(&self) -> &str {
        &self.value
    }

    /// Byte offset of the cursor within [`Editor::value`].
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// Whether there is no text. What decides between a value and a placeholder.
    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Display width of the text before the cursor: where to draw it, in cells.
    pub fn cursor_column(&self) -> u16 {
        text_width(&self.value[..self.cursor])
    }

    /// Put the cursor at the grapheme boundary nearest display `column`.
    ///
    /// What a click in a text field means. A column inside a wide grapheme resolves to whichever
    /// end of it is nearer, and a column past the end of the text puts the cursor at the end —
    /// clicking the empty space to the right of a short value is a request to type at the end of
    /// it, not a miss.
    pub fn set_cursor_column(&mut self, column: u16) {
        let mut at = 0u16;
        let mut cursor = 0usize;
        for grapheme in self.value.graphemes(true) {
            let width = text_width(grapheme);
            if column < at + width {
                // Inside this grapheme: round to the nearer of its two edges.
                if column >= at + width.div_ceil(2) {
                    cursor += grapheme.len();
                }
                self.cursor = cursor;
                return;
            }
            at += width;
            cursor += grapheme.len();
        }
        self.cursor = self.value.len();
    }

    /// The part of the value a field `width` cells wide shows, and how many display columns are
    /// hidden off its left edge.
    ///
    /// A field narrower than its text scrolls horizontally to keep the cursor in view. Both the
    /// widget drawing it and the code turning a click into a caret position need to agree on
    /// exactly how far it scrolled, so that rule lives here rather than in either of them.
    ///
    /// Resolve a click against the hidden width returned *before* moving the cursor: the frame the
    /// user clicked on was drawn with that scroll, and moving the cursor first changes it.
    ///
    /// ```
    /// use conui::state::Editor;
    ///
    /// let mut editor = Editor::with("a long heading that does not fit");
    /// // The cursor is at the end, so the field shows the tail of the text.
    /// let (shown, hidden) = editor.view_from(10);
    /// assert_eq!(shown, "s not fit");
    /// assert_eq!(hidden, 23);
    ///
    /// // A click on the seventh cell of that field lands just before "fit".
    /// editor.set_cursor_column(hidden + 6);
    /// assert_eq!(&editor.value()[..editor.cursor()], "a long heading that does not ");
    /// ```
    pub fn view_from(&self, width: u16) -> (&str, u16) {
        if width == 0 {
            return ("", 0);
        }
        // One column past the last character is where the cursor sits while typing at the end, so
        // the field has to keep room for it.
        let target = self.cursor_column().saturating_sub(width - 1);
        let mut hidden = 0u16;
        let mut start = 0usize;
        for grapheme in self.value.graphemes(true) {
            if hidden >= target {
                break;
            }
            hidden += text_width(grapheme);
            start += grapheme.len();
        }
        (&self.value[start..], hidden)
    }

    /// Replace the text, putting the cursor at the end.
    pub fn set_value(&mut self, text: impl Into<String>) {
        self.value = text.into();
        self.cursor = self.value.len();
    }

    /// Empty the field, cursor back to the start.
    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// Take the text, leaving the editor empty. The shape a "submit" handler wants.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.value)
    }

    /// Insert one character at the cursor, and step over it.
    pub fn insert(&mut self, character: char) {
        self.value.insert(self.cursor, character);
        self.cursor += character.len_utf8();
    }

    /// Insert a whole string, as a paste does.
    pub fn insert_str(&mut self, text: &str) {
        // Newlines would make a single-line field render as one long line with a stray glyph in
        // it; a pasted paragraph becomes spaces, which is at least honest about the shape.
        let flattened: String = text
            .chars()
            .map(|c| if c == '\n' || c == '\r' || c == '\t' { ' ' } else { c })
            .collect();
        self.value.insert_str(self.cursor, &flattened);
        self.cursor += flattened.len();
    }

    /// Delete the grapheme before the cursor. Returns whether anything was there.
    pub fn backspace(&mut self) -> bool {
        let Some(previous) = self.previous_boundary() else { return false };
        self.value.replace_range(previous..self.cursor, "");
        self.cursor = previous;
        true
    }

    /// Delete the grapheme after the cursor.
    pub fn delete(&mut self) -> bool {
        let Some(next) = self.next_boundary() else { return false };
        self.value.replace_range(self.cursor..next, "");
        true
    }

    /// Delete from the cursor back to the start of the line.
    pub fn delete_to_start(&mut self) {
        self.value.replace_range(..self.cursor, "");
        self.cursor = 0;
    }

    /// Delete from the cursor to the end of the line.
    pub fn delete_to_end(&mut self) {
        self.value.truncate(self.cursor);
    }

    /// Delete the word before the cursor, along with any space between.
    pub fn delete_word(&mut self) {
        let head = &self.value[..self.cursor];
        let trimmed = head.trim_end();
        let start = match trimmed.rfind(char::is_whitespace) {
            Some(index) => index + head[index..].chars().next().map_or(1, char::len_utf8),
            None => 0,
        };
        self.value.replace_range(start..self.cursor, "");
        self.cursor = start;
    }

    /// Back one grapheme, not one byte — an accented letter or an emoji moves as the one thing
    /// the user sees, and a cursor parked mid-codepoint would panic the next slice.
    pub fn left(&mut self) {
        if let Some(previous) = self.previous_boundary() {
            self.cursor = previous;
        }
    }

    /// Forward one grapheme.
    pub fn right(&mut self) {
        if let Some(next) = self.next_boundary() {
            self.cursor = next;
        }
    }

    /// Cursor to the start of the text.
    pub fn home(&mut self) {
        self.cursor = 0;
    }

    /// Cursor to the end of the text.
    pub fn end(&mut self) {
        self.cursor = self.value.len();
    }

    /// Apply a keypress, returning whether it was one this editor handles.
    ///
    /// A `false` means the key is yours: `Enter`, `Escape` and `Tab` are never consumed, because
    /// what they mean — submit, cancel, next field — is the app's decision, not a text field's.
    pub fn handle(&mut self, key: &KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        // The readline bindings every terminal user has in their fingers already.
        if key.modifiers.contains(Modifiers::CTRL) {
            match key.code {
                KeyCode::Char('u' | 'U') => self.delete_to_start(),
                KeyCode::Char('k' | 'K') => self.delete_to_end(),
                KeyCode::Char('w' | 'W') => self.delete_word(),
                KeyCode::Char('a' | 'A') => self.home(),
                KeyCode::Char('e' | 'E') => self.end(),
                KeyCode::Char('b' | 'B') => self.left(),
                KeyCode::Char('f' | 'F') => self.right(),
                _ => return false,
            }
            return true;
        }
        match key.code {
            // ALT-modified letters are window-manager and app shortcuts, not text.
            KeyCode::Char(character) if !key.modifiers.contains(Modifiers::ALT) => {
                self.insert(character);
            }
            KeyCode::Backspace => {
                self.backspace();
            }
            KeyCode::Delete => {
                self.delete();
            }
            KeyCode::Left => self.left(),
            KeyCode::Right => self.right(),
            KeyCode::Home => self.home(),
            KeyCode::End => self.end(),
            _ => return false,
        }
        true
    }

    fn previous_boundary(&self) -> Option<usize> {
        let head = &self.value[..self.cursor];
        head.graphemes(true).next_back().map(|last| self.cursor - last.len())
    }

    fn next_boundary(&self) -> Option<usize> {
        let tail = &self.value[self.cursor..];
        tail.graphemes(true).next().map(|first| self.cursor + first.len())
    }
}

/// Which component input goes to, and the order `Tab` walks them in.
///
/// This is deliberately not a focus *manager*: there is no registry a widget signs itself into
/// during render, and no widget ever asks "am I focused?" behind your back. You name the focusable
/// things in your own type, you say what order they cycle in, and each widget is told the answer:
///
/// ```
/// use conui::state::Focus;
///
/// #[derive(Clone, Copy, PartialEq)]
/// enum Field {
///     Name,
///     Colour,
///     Save,
/// }
///
/// let mut focus = Focus::new([Field::Name, Field::Colour, Field::Save]);
/// assert!(focus.is(Field::Name));
/// focus.next();
/// assert!(focus.is(Field::Colour));
/// focus.prev();
/// assert!(focus.is(Field::Name));
/// ```
///
/// The payoff is that focus stays a value you can reason about. Whether `Tab` even *means* "next
/// field" is yours to decide — inside a text area it might mean indent — and a screen whose
/// focusable set changes (a tab switch, a row that only exists while editing) calls [`set_ring`]
/// rather than fighting a registry that was populated by whatever happened to render last frame.
///
/// There is no notion of a disabled entry on purpose. A control that cannot be used should not be
/// in the ring, and leaving it out is one `set_ring` call.
///
/// [`set_ring`]: Focus::set_ring
#[derive(Clone, Debug)]
pub struct Focus<T> {
    ring: Vec<T>,
    current: usize,
}

/// An empty ring, which focuses nothing. Derived `Default` would demand `T: Default` for no
/// reason — there is no entry to construct.
impl<T> Default for Focus<T> {
    fn default() -> Self {
        Self { ring: Vec::new(), current: 0 }
    }
}

impl<T: Copy + PartialEq> Focus<T> {
    /// A ring focused on its first entry.
    pub fn new(ring: impl IntoIterator<Item = T>) -> Self {
        Self { ring: ring.into_iter().collect(), current: 0 }
    }

    /// A ring focused on `id`, or on the first entry if it is not in the ring.
    pub fn starting_at(ring: impl IntoIterator<Item = T>, id: T) -> Self {
        let mut focus = Self::new(ring);
        focus.focus(id);
        focus
    }

    /// What has focus, or `None` if the ring is empty.
    pub fn current(&self) -> Option<T> {
        self.ring.get(self.current).copied()
    }

    /// Whether `id` has focus. The question every widget's `.focused(..)` argument is answering.
    pub fn is(&self, id: T) -> bool {
        self.current() == Some(id)
    }

    /// Move focus to `id`, reporting whether it was in the ring at all.
    ///
    /// Returning `false` rather than panicking matters for mouse and shortcut handling, where the
    /// id you were handed may well belong to a control that is not focusable right now.
    pub fn focus(&mut self, id: T) -> bool {
        match self.ring.iter().position(|entry| *entry == id) {
            Some(index) => {
                self.current = index;
                true
            }
            None => false,
        }
    }

    /// The next entry, wrapping. A no-op on an empty ring.
    pub fn next(&mut self) {
        if !self.ring.is_empty() {
            self.current = (self.current + 1) % self.ring.len();
        }
    }

    /// The previous entry, wrapping.
    pub fn prev(&mut self) {
        if !self.ring.is_empty() {
            self.current = (self.current + self.ring.len() - 1) % self.ring.len();
        }
    }

    /// Focus the first entry.
    pub fn first(&mut self) {
        self.current = 0;
    }

    /// Focus the last entry.
    pub fn last(&mut self) {
        self.current = self.ring.len().saturating_sub(1);
    }

    /// Replace the focusable set, keeping focus where it is if that entry still exists.
    ///
    /// Without the "keep" part, switching tabs would silently drop focus back to the first field
    /// of the screen, and a user who had tabbed three fields in would lose their place for
    /// reasons they cannot see.
    pub fn set_ring(&mut self, ring: impl IntoIterator<Item = T>) {
        let was = self.current();
        self.ring = ring.into_iter().collect();
        self.current = was
            .and_then(|id| self.ring.iter().position(|entry| *entry == id))
            .unwrap_or(0)
            .min(self.ring.len().saturating_sub(1));
    }

    /// The focusable set, in `Tab` order.
    pub fn ring(&self) -> &[T] {
        &self.ring
    }

    /// How many entries are focusable.
    pub fn len(&self) -> usize {
        self.ring.len()
    }

    /// Whether nothing is focusable, in which case [`Focus::current`] is `None`.
    pub fn is_empty(&self) -> bool {
        self.ring.is_empty()
    }

    /// Apply `Tab` and `Shift+Tab`, reporting whether the key was used.
    ///
    /// Nothing else, and in particular not the arrow keys: in a list or a text field they mean
    /// something else entirely, and a focus ring that swallowed them would make both unusable.
    pub fn handle(&mut self, key: &KeyEvent) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        match key.code {
            KeyCode::Tab if !key.modifiers.contains(Modifiers::SHIFT) => self.next(),
            KeyCode::Tab | KeyCode::BackTab => self.prev(),
            _ => return false,
        }
        true
    }
}

// ---- Hits -------------------------------------------------------------------------------

/// Where each control landed, so a click can find it.
///
/// [`Focus`] deliberately refuses to learn anything at render time, because the tab order is a
/// decision you make, not a consequence of draw order. Hit-testing is the opposite: a click
/// resolves against pixels that were actually on screen, so last frame's geometry is the *only*
/// thing that can answer it. Hence a second, separate structure that does record during render.
///
/// Fill it with [`ViewExt::hit`](crate::view::ViewExt::hit) — a decorator that notes its child's
/// region and then draws the child — and read it in your event handler:
///
/// ```
/// use conui::view::{Column, ViewExt};
/// use conui::widget::Button;
/// use conui::{Buffer, Frame, Hits, Pos, Theme};
///
/// #[derive(Clone, Copy, PartialEq, Debug)]
/// enum Id {
///     Ok,
///     Cancel,
/// }
///
/// let hits = Hits::new();
/// let screen = Column::new()
///     .child(Button::new("OK").hit(&hits, Id::Ok).length(1))
///     .child(Button::new("Cancel").hit(&hits, Id::Cancel).length(1));
///
/// let mut buffer = Buffer::new(20, 2);
/// Frame::new(&mut buffer, Theme::LAYA).render_full(&screen);
///
/// assert_eq!(hits.at(Pos::new(3, 0)), Some(Id::Ok));
/// assert_eq!(hits.at(Pos::new(3, 1)), Some(Id::Cancel));
/// assert_eq!(hits.at(Pos::new(3, 9)), None);
/// ```
///
/// Entries are what was *visible*. A control clipped away by a window too small for it, or scrolled
/// out of its pane, records nothing — see [`Canvas::visible_area`](crate::Canvas::visible_area) — so
/// a click can only ever resolve to something that was on screen to be clicked.
///
/// Clear it at the top of every frame. A stale entry is worse than a missing one: it points at
/// where a control used to be, which is exactly the bug that makes a UI feel haunted.
#[derive(Debug, Default)]
pub struct Hits<T> {
    /// Interior mutability for the same reason [`Selection::offset`] needs it: rendering takes
    /// `&self`, and rendering is when a region is known.
    regions: RefCell<Vec<(T, Rect)>>,
}

impl<T: Copy> Clone for Hits<T> {
    fn clone(&self) -> Self {
        Self { regions: RefCell::new(self.regions.borrow().clone()) }
    }
}

impl<T: Copy> Hits<T> {
    /// An empty table, which answers `None` to everything until a frame has been composed.
    pub fn new() -> Self {
        Self { regions: RefCell::new(Vec::new()) }
    }

    /// Forget last frame's geometry. Call this before composing.
    pub fn clear(&self) {
        self.regions.borrow_mut().clear();
    }

    /// Note that `id` occupies `area`, in buffer coordinates.
    pub fn record(&self, id: T, area: Rect) {
        self.regions.borrow_mut().push((id, area));
    }

    /// Which control is at `pos`, if any.
    ///
    /// Searched last-recorded first, so the answer agrees with what the eye sees: an overlay drawn
    /// after the form takes the click, because it is the thing covering that cell.
    pub fn at(&self, pos: Pos) -> Option<T> {
        self.regions.borrow().iter().rev().find(|(_, area)| area.contains(pos)).map(|(id, _)| *id)
    }

    /// Where `id` last drew itself, if it drew at all.
    pub fn area_of(&self, id: T) -> Option<Rect>
    where
        T: PartialEq,
    {
        self.regions.borrow().iter().find(|(entry, _)| *entry == id).map(|(_, area)| *area)
    }

    /// `pos` relative to `id`'s own origin, or `None` if it fell outside.
    ///
    /// For a control with internal structure — which tab label, which row of a list — where the
    /// widget can answer "at this offset, that one" but only the caller knows the offset.
    pub fn local(&self, id: T, pos: Pos) -> Option<Pos>
    where
        T: PartialEq,
    {
        let area = self.area_of(id)?;
        area.contains(pos).then(|| Pos::new(pos.x - area.x, pos.y - area.y))
    }

    /// How many regions were recorded this frame.
    pub fn len(&self) -> usize {
        self.regions.borrow().len()
    }

    /// Whether nothing was recorded — no frame composed yet, or one that drew no controls.
    pub fn is_empty(&self) -> bool {
        self.regions.borrow().is_empty()
    }
}

/// An open-or-closed list of choices: the state behind a [`Select`](crate::widget::Select).
///
/// The choices themselves are not in here. They are your data, passed to the widget each frame,
/// which means a dropdown over a list that changes — files in a directory, branches in a repo —
/// needs no invalidation step. What this owns is the three things that survive a frame: whether
/// the list is showing, which entry is highlighted, and what to go back to if the user changes
/// their mind.
///
/// A dropdown is also the first widget here that has to draw *outside* its own region, and a view
/// in conui structurally cannot: it is handed a sub-canvas and clipped to it. So it does not try.
/// The closed field records where it landed during render, and the app draws the list as a second
/// pass over the same frame:
///
/// ```no_run
/// # use conui::state::Dropdown;
/// # use conui::widget::Menu;
/// # use conui::{Frame, Theme};
/// # use conui_cell::Buffer;
/// # let options = ["dark", "light"];
/// # let dropdown = Dropdown::new();
/// # let mut buffer = Buffer::new(40, 10);
/// # let mut frame = Frame::new(&mut buffer, Theme::LAYA);
/// # let screen = conui::widget::Text::new("");
/// frame.render_full(&screen);
/// if dropdown.is_open() {
///     let area = dropdown.popup_area(options.len(), frame.area());
///     frame.render(&Menu::new(dropdown.selection(), options), area);
/// }
/// ```
///
/// Two passes instead of one, and in exchange there is no z-order to configure, no overlay stack
/// to flush, and the thing on top is on top because you drew it last.
#[derive(Debug, Default)]
pub struct Dropdown {
    open: bool,
    selection: Selection,
    /// What to restore if the list is dismissed rather than committed.
    restore: usize,
    /// Where the closed field last drew itself, in buffer coordinates.
    field: Cell<Rect>,
    /// How tall the list is allowed to get.
    rows: u16,
}

impl Clone for Dropdown {
    fn clone(&self) -> Self {
        Self {
            open: self.open,
            selection: self.selection.clone(),
            restore: self.restore,
            field: Cell::new(self.field.get()),
            rows: self.rows,
        }
    }
}

impl Dropdown {
    /// Closed, on the first choice.
    pub fn new() -> Self {
        Self { rows: 8, ..Default::default() }
    }

    /// Closed, on choice `index`.
    pub fn at(index: usize) -> Self {
        Self { selection: Selection::at(index), restore: index, ..Self::new() }
    }

    /// Cap how many choices the open list shows at once. It scrolls beyond that.
    pub fn rows(mut self, rows: u16) -> Self {
        self.rows = rows;
        self
    }

    /// Whether the list is showing. What tells an app to route keys to the dropdown first.
    pub const fn is_open(&self) -> bool {
        self.open
    }

    /// The chosen index. Always current: dismissing the list restores the previous choice rather
    /// than leaving a half-made decision behind, so there is no "committed value" to track
    /// separately.
    pub fn selected(&self) -> usize {
        self.selection.selected()
    }

    /// Choose `index` directly, as loading a saved setting does.
    pub fn set_selected(&mut self, index: usize) {
        self.selection.set_selected(index);
    }

    /// The highlight the open list draws, for the widget.
    pub const fn selection(&self) -> &Selection {
        &self.selection
    }

    /// Show the list, remembering the current choice in case it is dismissed.
    pub fn open(&mut self) {
        self.open = true;
        self.restore = self.selection.selected();
    }

    /// Accept the highlighted choice.
    pub fn commit(&mut self) {
        self.open = false;
    }

    /// Put the choice back to what it was before the list opened.
    pub fn dismiss(&mut self) {
        self.open = false;
        self.selection.set_selected(self.restore);
    }

    /// Open the list, or accept the highlighted choice if it is already open. What `Enter` and a
    /// click on the field both mean.
    pub fn toggle(&mut self) {
        if self.open {
            self.commit();
        } else {
            self.open();
        }
    }

    /// Where the closed field drew itself. Set by the widget during render.
    pub fn field(&self) -> Rect {
        self.field.get()
    }

    /// Record where the closed field drew itself, so the list can be placed under it next frame.
    ///
    /// Called by the widget during render, not by an app: only the draw knows where the field
    /// ended up, and the list is positioned a pass later.
    pub fn set_field(&self, area: Rect) {
        self.field.set(area);
    }

    /// Where the open list should go: under the field if it fits, over it if it does not.
    ///
    /// Flipping matters more than it sounds like. A dropdown on the last row of a full-screen app
    /// is not an edge case, it is where the Apply button lives.
    pub fn popup_area(&self, len: usize, screen: Rect) -> Rect {
        let field = self.field.get();
        // Two rows of border plus the rows themselves, capped and never zero-height.
        let wanted = u16::try_from(len).unwrap_or(u16::MAX).clamp(1, self.rows.max(1));
        let height = wanted.saturating_add(2).min(screen.height.max(1));

        let below = field.bottom();
        let y = if below.saturating_add(height) <= screen.bottom() {
            below
        } else {
            // Above, or pinned to the top edge if there is no room either way.
            field.y.checked_sub(height).unwrap_or(screen.y)
        };

        let width = field.width.max(4).min(screen.width.max(4));
        let x = field.x.min(screen.right().saturating_sub(width));
        Rect::new(x, y, width, height)
    }

    /// Which option the open list is showing at `pos`, if any.
    ///
    /// Nobody has to record the list's geometry for this: [`Dropdown::popup_area`] computes it the
    /// same way the second pass drew it, and the rows inside it are resolved by the same
    /// [`Selection`] the [`Menu`](crate::widget::Menu) scrolled.
    pub fn option_at(&self, pos: Pos, len: usize, screen: Rect) -> Option<usize> {
        if !self.open {
            return None;
        }
        let rows = self.popup_area(len, screen).inset(Padding::all(1));
        self.selection.row_at(rows, pos, len)
    }

    /// Apply the mouse events a dropdown owns, reporting whether the event was used.
    ///
    /// Closed, only a click on the field does anything — a wheel over a closed select is left
    /// alone, because a form that silently changes a value while the user scrolls past it is a
    /// classic way to lose someone's data. Open, it takes *everything*, for the same reason its
    /// keyboard handler does: a click outside dismisses the list rather than falling through to
    /// whatever happens to be under it.
    ///
    /// The wheel over an open list moves the highlight, which is what [`Selection::step`] does and
    /// what a native select does. It changes nothing: a value is only taken on a click or an
    /// `Enter`, so a wheel cannot commit anything by accident.
    pub fn handle_mouse(&mut self, mouse: &MouseEvent, len: usize, screen: Rect) -> bool {
        let at = Pos::new(mouse.column, mouse.row);
        if self.open {
            if let Some(delta) = mouse.kind.scroll() {
                self.selection.step(delta, len);
            } else if mouse.is_click() {
                match self.option_at(at, len, screen) {
                    Some(index) => {
                        self.selection.set_selected(index);
                        self.commit();
                    }
                    None => self.dismiss(),
                }
            }
            return true;
        }
        if mouse.is_click() && self.field.get().contains(at) {
            self.open();
            return true;
        }
        false
    }

    /// Apply the keys a dropdown owns, reporting whether the key was used.
    ///
    /// Closed, it opens on `Enter` or `Space` and otherwise takes nothing — so a screen can still
    /// use the arrow keys to move between fields. Open, it takes the arrows and `Enter`/`Escape`,
    /// and takes them *all*, because a list covering half the screen must be dismissed before
    /// anything else can be reached.
    pub fn handle(&mut self, key: &KeyEvent, len: usize) -> bool {
        if key.kind == KeyEventKind::Release {
            return false;
        }
        if !self.open {
            return match key.code {
                KeyCode::Enter | KeyCode::Char(' ') => {
                    self.open();
                    true
                }
                _ => false,
            };
        }
        match key.code {
            KeyCode::Up => self.selection.up(),
            KeyCode::Down => self.selection.down(len),
            KeyCode::Home => self.selection.first(),
            KeyCode::End => self.selection.last(len),
            // `rows` is the cap on the popup's height, not the height it got: a three-choice list
            // inside a cap of eight paged by eight. The selection knows what the menu actually
            // drew, borders excluded, because the menu is what asked it for a window.
            KeyCode::PageUp => self.selection.page_up(),
            KeyCode::PageDown => self.selection.page_down(len),
            KeyCode::Enter | KeyCode::Char(' ') => self.commit(),
            KeyCode::Escape => self.dismiss(),
            // Deliberately greedy: see above.
            _ => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Selection ----------------------------------------------------------------------

    #[test]
    fn moving_down_stops_at_the_last_row() {
        let mut selection = Selection::new();
        for _ in 0..10 {
            selection.down(3);
        }
        assert_eq!(selection.selected(), 2);
    }

    #[test]
    fn moving_up_stops_at_the_first_row() {
        let mut selection = Selection::at(1);
        selection.up();
        selection.up();
        assert_eq!(selection.selected(), 0);
    }

    #[test]
    fn cycling_wraps_at_both_ends() {
        let mut selection = Selection::new();
        selection.cycle_up(3);
        assert_eq!(selection.selected(), 2);
        selection.cycle_down(3);
        assert_eq!(selection.selected(), 0);
    }

    #[test]
    fn cycling_an_empty_list_does_nothing() {
        let mut selection = Selection::new();
        selection.cycle_down(0);
        selection.cycle_up(0);
        assert_eq!(selection.selected(), 0);
    }

    #[test]
    fn a_window_shows_the_top_of_a_list_that_fits() {
        let selection = Selection::new();
        assert_eq!(selection.window(10, 4), 0..4);
        assert_eq!(selection.offset(), 0);
    }

    #[test]
    fn a_window_scrolls_down_to_follow_the_selection() {
        let selection = Selection::at(7);
        assert_eq!(selection.window(3, 20), 5..8);
    }

    #[test]
    fn a_window_scrolls_up_to_follow_the_selection() {
        let selection = Selection::at(12);
        selection.window(3, 20);
        let mut selection = selection;
        selection.set_selected(2);
        assert_eq!(selection.window(3, 20), 2..5);
    }

    #[test]
    fn a_window_never_leaves_blank_rows_below_content() {
        // At the end of the list in a three-row window, then asked for ten: it has to back up
        // rather than show ten rows of which seven are past the end.
        let mut selection = Selection::at(19);
        selection.window(3, 20);
        assert_eq!(selection.window(10, 20), 10..20);
        // And the offset left up there is abandoned as soon as the selection is above it.
        selection.set_selected(0);
        assert_eq!(selection.window(10, 20), 0..10);
    }

    #[test]
    fn a_step_moves_the_selection_and_stops_at_both_ends() {
        let mut selection = Selection::new();
        selection.step(3, 10);
        assert_eq!(selection.selected(), 3);
        selection.step(-1, 10);
        assert_eq!(selection.selected(), 2);
        selection.step(-9, 10);
        assert_eq!(selection.selected(), 0, "no wrapping, and no underflow");
        selection.step(99, 10);
        assert_eq!(selection.selected(), 9);
        let mut empty = Selection::new();
        empty.step(1, 0);
        assert_eq!(empty.selected(), 0);
    }

    #[test]
    fn scrolling_to_an_offset_drags_the_selection_into_the_new_window() {
        let mut selection = Selection::new();
        selection.window(5, 40);
        selection.scroll_to(20, 40);
        assert_eq!(selection.offset(), 20);
        // The cursor was on row 0, which is now twenty rows above the window: it comes to the
        // nearest edge and no further.
        assert_eq!(selection.selected(), 20);
        // And the next draw leaves both alone, which is the point — an offset the window has to
        // undo is an offset nobody can drag to.
        assert_eq!(selection.window(5, 40), 20..25);
        assert_eq!(selection.offset(), 20);
    }

    #[test]
    fn scrolling_leaves_a_selection_that_is_already_in_the_window_alone() {
        let mut selection = Selection::at(22);
        selection.window(5, 40);
        assert_eq!(selection.offset(), 18);
        selection.scroll_to(20, 40);
        assert_eq!(selection.selected(), 22, "already visible, so it does not move");
        assert_eq!(selection.window(5, 40), 20..25);
    }

    #[test]
    fn scrolling_past_the_end_stops_at_the_last_windowful() {
        let mut selection = Selection::new();
        selection.window(10, 40);
        selection.scroll_to(999, 40);
        assert_eq!(selection.offset(), 30, "never blank rows below content");
        assert_eq!(selection.selected(), 30);
        assert_eq!(selection.window(10, 40), 30..40);
    }

    #[test]
    fn scrolling_a_list_shorter_than_its_window_does_nothing_it_could_regret() {
        let mut selection = Selection::at(2);
        selection.window(10, 4);
        selection.scroll_to(3, 4);
        assert_eq!(selection.offset(), 0, "there is nowhere to scroll to");
        assert_eq!(selection.selected(), 2);
    }

    #[test]
    fn scrolling_before_the_first_draw_does_nothing() {
        // No window has been drawn, so no height is known, so there is no scroll to make. Better
        // than guessing a height and moving the cursor somewhere the user cannot see.
        let mut selection = Selection::new();
        selection.scroll_to(20, 40);
        assert_eq!(selection.offset(), 0);
        assert_eq!(selection.selected(), 0);
    }

    #[test]
    fn scrolling_an_empty_list_is_harmless() {
        let mut selection = Selection::new();
        selection.window(5, 0);
        selection.scroll_to(3, 0);
        assert_eq!(selection.offset(), 0);
        assert_eq!(selection.selected(), 0);
    }

    #[test]
    fn a_scrollbar_offset_round_trips_through_scroll_to() {
        // The two halves of a drag, end to end: the bar says which offset a pointer row means, and
        // the selection accepts it — so the thumb lands where it was dragged rather than near it.
        use crate::widget::Scrollbar;

        let mut selection = Selection::new();
        selection.window(10, 40);
        // Forty rows in ten gives a three-row thumb, so its top can only reach row seven. A
        // pointer past that is asking for the bottom, and gets it.
        let furthest = Scrollbar::new(40, 40).thumb(10).expect("it overflows").start;
        assert_eq!(furthest, 7);

        for row in 0..10u16 {
            let offset = Scrollbar::new(selection.offset(), 40).offset_at(row, 10);
            selection.scroll_to(offset, 40);
            let thumb =
                Scrollbar::new(selection.offset(), 40).thumb(10).expect("forty rows in ten");
            assert_eq!(
                thumb.start,
                row.min(furthest),
                "asked for the thumb at {row}, got it at {}",
                thumb.start
            );
        }
    }

    #[test]
    fn an_empty_or_zero_height_window_is_empty() {
        let selection = Selection::new();
        assert_eq!(selection.window(0, 5), 0..0);
        assert_eq!(selection.window(5, 0), 0..0);
    }

    #[test]
    fn removing_a_row_above_the_selection_shifts_it_up() {
        let mut selection = Selection::at(3);
        selection.removed(1, 5);
        assert_eq!(selection.selected(), 2);
    }

    #[test]
    fn removing_the_selected_row_keeps_the_index_so_the_next_slides_under_it() {
        let mut selection = Selection::at(2);
        selection.removed(2, 5);
        assert_eq!(selection.selected(), 2);
    }

    #[test]
    fn removing_the_last_row_steps_the_selection_back() {
        let mut selection = Selection::at(4);
        selection.removed(4, 4);
        assert_eq!(selection.selected(), 3);
    }

    #[test]
    fn clamping_pulls_a_stale_selection_inside() {
        let mut selection = Selection::at(9);
        selection.clamp(3);
        assert_eq!(selection.selected(), 2);
        selection.clamp(0);
        assert_eq!(selection.selected(), 0);
    }

    #[test]
    fn paging_moves_by_a_screen_and_stops_at_the_edges() {
        let mut selection = Selection::new();
        // Eleven visible rows: a page is ten, leaving the eleventh as the row the eye lands on.
        selection.window(11, 25);
        selection.page_down(25);
        assert_eq!(selection.selected(), 10);
        selection.page_down(25);
        selection.page_down(25);
        assert_eq!(selection.selected(), 24);
        selection.page_up();
        assert_eq!(selection.selected(), 14);
        for _ in 0..10 {
            selection.page_up();
        }
        assert_eq!(selection.selected(), 0);
    }

    #[test]
    fn a_page_is_the_height_the_list_was_drawn_at() {
        let mut selection = Selection::new();
        selection.window(6, 100);
        selection.page_down(100);
        assert_eq!(selection.selected(), 5, "five rows, with the sixth carried over");
        // The window a resize gave it, not the one it had when the key was bound.
        selection.window(21, 100);
        selection.page_down(100);
        assert_eq!(selection.selected(), 25);
    }

    #[test]
    fn a_selection_no_list_has_drawn_pages_by_one_row() {
        let mut selection = Selection::new();
        assert_eq!(selection.height(), 0, "nothing has drawn it");
        // A page of zero would leave PageDown doing nothing at all, which reads as a dead key.
        selection.page_down(10);
        assert_eq!(selection.selected(), 1);
        selection.page_up();
        assert_eq!(selection.selected(), 0);
    }

    #[test]
    fn a_window_records_its_height_even_with_nothing_to_show() {
        let selection = Selection::new();
        selection.window(9, 0);
        assert_eq!(selection.height(), 9, "an empty list still has a height to page by");
    }

    // ---- Viewport -----------------------------------------------------------------------

    /// Stand in for a draw: what `Scroll` does before anything is scrolled.
    fn drawn(viewport: &Viewport, height: u16, content: u16) -> u16 {
        viewport.window(height, content)
    }

    #[test]
    fn a_viewport_scrolls_within_what_the_last_draw_could_show() {
        let viewport = Viewport::new();
        drawn(&viewport, 10, 30);
        assert_eq!(viewport.overflow(), 20);
        viewport.scroll(5);
        assert_eq!(viewport.offset(), 5);
        viewport.scroll(-99);
        assert_eq!(viewport.offset(), 0, "no underflow past the top");
        viewport.scroll(9_999);
        assert_eq!(viewport.offset(), 20, "and no scrolling past the end");
        assert!(viewport.is_at_bottom());
    }

    #[test]
    fn a_viewport_with_room_to_spare_has_nothing_to_scroll() {
        let viewport = Viewport::new();
        drawn(&viewport, 10, 4);
        assert!(!viewport.is_scrollable());
        viewport.scroll(3);
        assert_eq!(viewport.offset(), 0);
        assert!(viewport.is_at_top() && viewport.is_at_bottom());
    }

    #[test]
    fn scrolling_before_the_first_draw_is_kept_and_clamped_when_it_happens() {
        // The trap this avoids: a position restored at startup silently doing nothing, because
        // nothing has been drawn yet and so the bounds read as zero.
        let viewport = Viewport::at(12);
        assert_eq!(viewport.offset(), 12);
        viewport.bottom();
        assert_eq!(drawn(&viewport, 5, 9), 4, "clamped to the end of the real content");
        assert_eq!(viewport.offset(), 4);
    }

    #[test]
    fn a_stale_offset_shows_the_end_of_shorter_content_not_a_blank_pane() {
        let viewport = Viewport::new();
        drawn(&viewport, 5, 100);
        viewport.bottom();
        assert_eq!(viewport.offset(), 95);
        // The document was replaced by a much shorter one.
        assert_eq!(drawn(&viewport, 5, 8), 3);
    }

    #[test]
    fn a_page_is_a_windowful_less_one_row_of_overlap() {
        let viewport = Viewport::new();
        drawn(&viewport, 10, 100);
        viewport.page_down();
        assert_eq!(viewport.offset(), 9, "one row carries over so the eye can land");
        viewport.page_down();
        assert_eq!(viewport.offset(), 18);
        viewport.page_up();
        assert_eq!(viewport.offset(), 9);
        viewport.top();
        assert_eq!(viewport.offset(), 0);
    }

    #[test]
    fn a_one_row_viewport_still_pages() {
        // `height - 1` is zero here, and a page of zero rows would hang the user pressing PageDown.
        let viewport = Viewport::new();
        drawn(&viewport, 1, 10);
        viewport.page_down();
        assert_eq!(viewport.offset(), 1);
    }

    #[test]
    fn revealing_a_row_scrolls_the_least_that_brings_it_into_view() {
        let viewport = Viewport::new();
        drawn(&viewport, 10, 100);
        viewport.reveal(5);
        assert_eq!(viewport.offset(), 0, "already visible, so nothing moves");
        viewport.reveal(14);
        assert_eq!(viewport.offset(), 5, "just far enough that row 14 is the last one");
        viewport.reveal(3);
        assert_eq!(viewport.offset(), 3, "and back up to the row itself");
    }

    // ---- Editor -------------------------------------------------------------------------

    #[test]
    fn typing_inserts_at_the_cursor() {
        let mut editor = Editor::new();
        editor.insert('a');
        editor.insert('c');
        editor.left();
        editor.insert('b');
        assert_eq!(editor.value(), "abc");
        assert_eq!(editor.cursor(), 2);
    }

    #[test]
    fn backspace_removes_the_grapheme_before_the_cursor() {
        let mut editor = Editor::with("ab");
        assert!(editor.backspace());
        assert_eq!(editor.value(), "a");
        assert!(editor.backspace());
        assert!(!editor.backspace(), "an empty editor has nothing to delete");
    }

    #[test]
    fn delete_removes_the_grapheme_after_the_cursor() {
        let mut editor = Editor::with("ab");
        editor.home();
        assert!(editor.delete());
        assert_eq!(editor.value(), "b");
        assert_eq!(editor.cursor(), 0);
        editor.end();
        assert!(!editor.delete());
    }

    #[test]
    fn a_combining_sequence_deletes_as_one_unit() {
        // "e" plus a combining acute: two chars, three bytes, one grapheme, one keypress.
        let mut editor = Editor::with("cafe\u{301}");
        editor.backspace();
        assert_eq!(editor.value(), "caf");
    }

    #[test]
    fn a_wide_grapheme_moves_the_cursor_two_columns() {
        let mut editor = Editor::with("日本");
        assert_eq!(editor.cursor_column(), 4);
        editor.left();
        assert_eq!(editor.cursor_column(), 2);
    }

    #[test]
    fn a_multibyte_grapheme_steps_as_one() {
        let mut editor = Editor::with("añb");
        editor.home();
        editor.right();
        editor.right();
        assert_eq!(editor.cursor(), 3, "a is one byte, ñ is two");
    }

    #[test]
    fn a_click_puts_the_cursor_between_two_characters_not_on_one() {
        let mut editor = Editor::with("hello");
        editor.set_cursor_column(0);
        assert_eq!(editor.cursor(), 0);
        editor.set_cursor_column(3);
        assert_eq!(&editor.value()[..editor.cursor()], "hel");
    }

    #[test]
    fn a_click_inside_a_wide_grapheme_rounds_to_its_nearer_edge() {
        // "a日b": the ideograph occupies columns 1 and 2.
        let mut editor = Editor::with("a日b");
        editor.set_cursor_column(1);
        assert_eq!(editor.cursor(), 1, "the left half belongs to the character before it");
        editor.set_cursor_column(2);
        assert_eq!(editor.cursor(), 4, "the right half belongs to the one after");
        editor.set_cursor_column(3);
        assert_eq!(editor.cursor(), 4);
    }

    #[test]
    fn a_click_past_the_end_of_the_text_lands_at_the_end() {
        // Clicking the empty half of a field is a request to type at the end, not a miss.
        let mut editor = Editor::with("short");
        editor.home();
        editor.set_cursor_column(400);
        assert_eq!(editor.cursor(), 5);
    }

    #[test]
    fn a_field_scrolls_only_as_far_as_the_cursor_requires() {
        let mut editor = Editor::with("abcdefgh");
        assert_eq!(editor.view_from(20), ("abcdefgh", 0), "a field with room scrolls not at all");
        // Eight characters, a five-cell field: the cursor sits one past the text, at column 8, so
        // four columns are hidden to keep it visible.
        assert_eq!(editor.view_from(5), ("efgh", 4));
        editor.home();
        assert_eq!(editor.view_from(5), ("abcdefgh", 0), "and scrolls back when the cursor does");
        assert_eq!(editor.view_from(0), ("", 0), "a field with no width shows nothing");
    }

    #[test]
    fn clicking_the_cell_the_cursor_is_drawn_in_does_not_move_it() {
        // The invariant that makes a click land where the eye says it should: `Input` draws with
        // `view_from` and a click resolves with `set_cursor_column`, so the two have to agree at
        // every position and every field width.
        for width in 1..=12u16 {
            let mut editor = Editor::with("a日b cdé");
            editor.home();
            loop {
                let cursor = editor.cursor();
                let (_, hidden) = editor.view_from(width);
                let drawn = editor.cursor_column().saturating_sub(hidden);
                let mut clicked = editor.clone();
                clicked.set_cursor_column(hidden + drawn);
                assert_eq!(clicked.cursor(), cursor, "width {width}, cursor {cursor}");
                editor.right();
                if editor.cursor() == cursor {
                    break;
                }
            }
        }
    }

    #[test]
    fn deleting_a_word_takes_the_space_with_it() {
        let mut editor = Editor::with("buy some milk");
        editor.delete_word();
        assert_eq!(editor.value(), "buy some ");
        editor.delete_word();
        assert_eq!(editor.value(), "buy ");
    }

    #[test]
    fn deleting_a_word_from_the_start_is_harmless() {
        let mut editor = Editor::new();
        editor.delete_word();
        assert_eq!(editor.value(), "");
    }

    #[test]
    fn deleting_to_the_start_keeps_the_tail() {
        let mut editor = Editor::with("hello world");
        editor.home();
        for _ in 0..6 {
            editor.right();
        }
        editor.delete_to_start();
        assert_eq!(editor.value(), "world");
        assert_eq!(editor.cursor(), 0);
    }

    #[test]
    fn deleting_to_the_end_keeps_the_head() {
        let mut editor = Editor::with("hello world");
        editor.home();
        for _ in 0..5 {
            editor.right();
        }
        editor.delete_to_end();
        assert_eq!(editor.value(), "hello");
    }

    #[test]
    fn taking_the_value_empties_the_editor() {
        let mut editor = Editor::with("milk");
        assert_eq!(editor.take(), "milk");
        assert!(editor.is_empty());
        assert_eq!(editor.cursor(), 0);
    }

    #[test]
    fn a_pasted_newline_becomes_a_space_rather_than_breaking_the_line() {
        let mut editor = Editor::new();
        editor.insert_str("one\ntwo\tthree");
        assert_eq!(editor.value(), "one two three");
    }

    #[test]
    fn handling_a_character_key_types_it() {
        let mut editor = Editor::new();
        assert!(editor.handle(&KeyEvent::plain(KeyCode::Char('x'))));
        assert_eq!(editor.value(), "x");
    }

    #[test]
    fn enter_escape_and_tab_are_left_for_the_app() {
        let mut editor = Editor::with("text");
        for code in [KeyCode::Enter, KeyCode::Escape, KeyCode::Tab, KeyCode::Up] {
            assert!(!editor.handle(&KeyEvent::plain(code)), "{code:?} should not be consumed");
        }
        assert_eq!(editor.value(), "text");
    }

    #[test]
    fn a_key_release_is_never_treated_as_typing() {
        let mut editor = Editor::new();
        let mut key = KeyEvent::plain(KeyCode::Char('x'));
        key.kind = KeyEventKind::Release;
        assert!(!editor.handle(&key));
        assert!(editor.is_empty());
    }

    #[test]
    fn the_readline_bindings_work() {
        let mut editor = Editor::with("buy some milk");
        assert!(editor.handle(&KeyEvent::new(KeyCode::Char('w'), Modifiers::CTRL)));
        assert_eq!(editor.value(), "buy some ");
        assert!(editor.handle(&KeyEvent::new(KeyCode::Char('a'), Modifiers::CTRL)));
        assert_eq!(editor.cursor(), 0);
        assert!(editor.handle(&KeyEvent::new(KeyCode::Char('e'), Modifiers::CTRL)));
        assert_eq!(editor.cursor(), 9);
        assert!(editor.handle(&KeyEvent::new(KeyCode::Char('u'), Modifiers::CTRL)));
        assert!(editor.is_empty());
    }

    #[test]
    fn an_unknown_control_key_is_not_swallowed() {
        let mut editor = Editor::new();
        assert!(!editor.handle(&KeyEvent::new(KeyCode::Char('z'), Modifiers::CTRL)));
    }

    #[test]
    fn an_alt_letter_is_a_shortcut_not_text() {
        let mut editor = Editor::new();
        assert!(!editor.handle(&KeyEvent::new(KeyCode::Char('f'), Modifiers::ALT)));
        assert!(editor.is_empty());
    }

    #[test]
    fn a_capital_letter_arrives_as_text() {
        let mut editor = Editor::new();
        assert!(editor.handle(&KeyEvent::new(KeyCode::Char('A'), Modifiers::SHIFT)));
        assert_eq!(editor.value(), "A");
    }

    // ---- Focus --------------------------------------------------------------------------

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Id {
        A,
        B,
        C,
    }

    fn ring() -> Focus<Id> {
        Focus::new([Id::A, Id::B, Id::C])
    }

    #[test]
    fn a_new_ring_focuses_its_first_entry() {
        let focus = ring();
        assert_eq!(focus.current(), Some(Id::A));
        assert!(focus.is(Id::A));
        assert!(!focus.is(Id::B));
        assert_eq!(focus.len(), 3);
    }

    #[test]
    fn a_ring_can_start_somewhere_other_than_the_beginning() {
        // What restoring a screen needs: the focused field was saved, and the ring is rebuilt
        // around it. Starting at the first entry and tabbing forwards N times would work and would
        // be wrong the day the order changes.
        let focus = Focus::starting_at([Id::A, Id::B, Id::C], Id::C);
        assert_eq!(focus.current(), Some(Id::C));
        assert_eq!(focus.len(), 3, "the whole ring is still there, only the cursor moved");
    }

    #[test]
    fn starting_at_something_that_is_not_there_falls_back_to_the_first_entry() {
        // A saved id can outlive the screen that had it — a field removed between releases, or a
        // ring that varies with a setting. Landing on nothing would be worse than landing at the
        // start, because a ring with no focus takes no keys.
        let focus = Focus::starting_at([Id::A, Id::B], Id::C);
        assert_eq!(focus.current(), Some(Id::A));

        let empty = Focus::starting_at([], Id::A);
        assert_eq!(empty.current(), None, "there is no first entry to fall back to");
    }

    #[test]
    fn next_and_prev_wrap_in_both_directions() {
        let mut focus = ring();
        focus.prev();
        assert_eq!(focus.current(), Some(Id::C), "prev from the first entry wraps to the last");
        focus.next();
        assert_eq!(focus.current(), Some(Id::A));
    }

    #[test]
    fn focusing_something_outside_the_ring_changes_nothing_and_says_so() {
        let mut focus = Focus::new([Id::A, Id::B]);
        assert!(focus.focus(Id::B));
        assert!(!focus.focus(Id::C));
        assert_eq!(focus.current(), Some(Id::B));
    }

    #[test]
    fn an_empty_ring_focuses_nothing_and_does_not_panic() {
        let mut focus: Focus<Id> = Focus::new([]);
        assert_eq!(focus.current(), None);
        assert!(focus.is_empty());
        focus.next();
        focus.prev();
        focus.last();
        assert_eq!(focus.current(), None);
    }

    #[test]
    fn replacing_the_ring_keeps_focus_where_it_was_if_it_survives() {
        let mut focus = ring();
        focus.focus(Id::C);
        focus.set_ring([Id::A, Id::C]);
        assert_eq!(focus.current(), Some(Id::C));
    }

    #[test]
    fn replacing_the_ring_falls_back_to_the_first_entry_when_focus_is_gone() {
        let mut focus = ring();
        focus.focus(Id::C);
        focus.set_ring([Id::A, Id::B]);
        assert_eq!(focus.current(), Some(Id::A));
    }

    #[test]
    fn shrinking_to_nothing_leaves_no_focus_rather_than_an_index_past_the_end() {
        let mut focus = ring();
        focus.focus(Id::C);
        focus.set_ring([]);
        assert_eq!(focus.current(), None);
    }

    #[test]
    fn focus_takes_tab_and_shift_tab_and_nothing_else() {
        let mut focus = ring();
        assert!(focus.handle(&KeyEvent::plain(KeyCode::Tab)));
        assert_eq!(focus.current(), Some(Id::B));
        assert!(focus.handle(&KeyEvent::plain(KeyCode::BackTab)));
        assert_eq!(focus.current(), Some(Id::A));
        assert!(focus.handle(&KeyEvent::new(KeyCode::Tab, Modifiers::SHIFT)));
        assert_eq!(
            focus.current(),
            Some(Id::C),
            "shift+tab reported as a modifier still goes back"
        );
    }

    #[test]
    fn focus_leaves_the_arrow_keys_for_whatever_has_focus() {
        let mut focus = ring();
        for code in [KeyCode::Up, KeyCode::Down, KeyCode::Left, KeyCode::Right, KeyCode::Enter] {
            assert!(!focus.handle(&KeyEvent::plain(code)), "{code:?} should not move focus");
        }
        assert_eq!(focus.current(), Some(Id::A));
    }

    // ---- Dropdown -----------------------------------------------------------------------

    #[test]
    fn a_dropdown_starts_closed_on_the_choice_it_was_given() {
        let dropdown = Dropdown::at(2);
        assert!(!dropdown.is_open());
        assert_eq!(dropdown.selected(), 2);
    }

    #[test]
    fn enter_opens_a_closed_dropdown_and_nothing_else_does() {
        let mut dropdown = Dropdown::new();
        assert!(!dropdown.handle(&KeyEvent::plain(KeyCode::Down), 3));
        assert!(!dropdown.is_open(), "an arrow key must not open a closed dropdown");
        assert!(dropdown.handle(&KeyEvent::plain(KeyCode::Enter), 3));
        assert!(dropdown.is_open());
    }

    #[test]
    fn an_open_dropdown_moves_on_the_arrows_and_commits_on_enter() {
        let mut dropdown = Dropdown::new();
        dropdown.open();
        dropdown.handle(&KeyEvent::plain(KeyCode::Down), 3);
        dropdown.handle(&KeyEvent::plain(KeyCode::Down), 3);
        assert_eq!(dropdown.selected(), 2);
        dropdown.handle(&KeyEvent::plain(KeyCode::Enter), 3);
        assert!(!dropdown.is_open());
        assert_eq!(dropdown.selected(), 2, "a committed choice stays");
    }

    #[test]
    fn dismissing_an_open_dropdown_restores_the_previous_choice() {
        let mut dropdown = Dropdown::at(1);
        dropdown.open();
        dropdown.handle(&KeyEvent::plain(KeyCode::Down), 3);
        assert_eq!(dropdown.selected(), 2);
        dropdown.handle(&KeyEvent::plain(KeyCode::Escape), 3);
        assert!(!dropdown.is_open());
        assert_eq!(dropdown.selected(), 1);
    }

    #[test]
    fn an_open_dropdown_swallows_every_key_because_it_is_covering_the_screen() {
        let mut dropdown = Dropdown::new();
        dropdown.open();
        for code in [KeyCode::Tab, KeyCode::Char('q'), KeyCode::F(1)] {
            assert!(
                dropdown.handle(&KeyEvent::plain(code), 3),
                "{code:?} leaked past an open list"
            );
        }
        assert!(dropdown.is_open());
    }

    #[test]
    fn a_list_opens_below_its_field_when_there_is_room() {
        let dropdown = Dropdown::new();
        dropdown.set_field(Rect::new(4, 2, 20, 1));
        let area = dropdown.popup_area(3, Rect::sized(40, 20));
        assert_eq!(area, Rect::new(4, 3, 20, 5), "three choices plus two rows of border");
    }

    #[test]
    fn a_list_with_no_room_below_flips_above_its_field() {
        let dropdown = Dropdown::new();
        dropdown.set_field(Rect::new(0, 18, 10, 1));
        let area = dropdown.popup_area(3, Rect::sized(40, 20));
        assert_eq!(area.bottom(), 18, "it should sit directly on top of the field");
        assert_eq!(area.y, 13);
    }

    #[test]
    fn a_list_taller_than_the_screen_is_capped_rather_than_drawn_off_the_edge() {
        let dropdown = Dropdown::new().rows(4);
        dropdown.set_field(Rect::new(0, 0, 10, 1));
        let area = dropdown.popup_area(100, Rect::sized(40, 8));
        assert_eq!(area.height, 6, "four rows plus the border");
        assert!(area.bottom() <= 8);
    }

    #[test]
    fn a_list_at_the_right_edge_is_pulled_back_on_screen() {
        let dropdown = Dropdown::new();
        dropdown.set_field(Rect::new(34, 0, 20, 1));
        let area = dropdown.popup_area(2, Rect::sized(40, 20));
        assert_eq!(area.right(), 40);
        assert_eq!(area.x, 20);
    }

    #[test]
    fn a_dropdown_over_nothing_still_produces_a_drawable_area() {
        let dropdown = Dropdown::new();
        dropdown.set_field(Rect::new(0, 0, 8, 1));
        let area = dropdown.popup_area(0, Rect::sized(20, 10));
        assert!(!area.is_empty());
    }

    // ---- Hit-testing --------------------------------------------------------------------

    /// A screen-sized frame the dropdown tests can resolve a popup against.
    const SCREEN: Rect = Rect::new(0, 0, 40, 20);

    fn click(column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind: conui_input::MouseKind::Down(conui_input::MouseButton::Left),
            column,
            row,
            modifiers: Modifiers::NONE,
        }
    }

    fn wheel(down: bool) -> MouseEvent {
        MouseEvent {
            kind: if down {
                conui_input::MouseKind::ScrollDown
            } else {
                conui_input::MouseKind::ScrollUp
            },
            column: 0,
            row: 0,
            modifiers: Modifiers::NONE,
        }
    }

    #[test]
    fn a_row_is_resolved_through_the_offset_the_list_last_drew_with() {
        let selection = Selection::at(9);
        let area = Rect::new(2, 3, 10, 4);
        // Nothing has rendered yet, so the window has not slid: row 0 of the region is item 0.
        assert_eq!(selection.row_at(area, Pos::new(4, 3), 12), Some(0));
        // After a draw that had to scroll to show item 9, the same cell is a different item.
        assert_eq!(selection.window(4, 12), 6..10);
        assert_eq!(selection.row_at(area, Pos::new(4, 3), 12), Some(6));
        assert_eq!(selection.row_at(area, Pos::new(4, 6), 12), Some(9));
    }

    #[test]
    fn a_click_past_the_last_item_or_outside_the_list_selects_nothing() {
        let selection = Selection::new();
        let area = Rect::new(0, 0, 10, 6);
        assert_eq!(selection.row_at(area, Pos::new(0, 2), 3), Some(2), "the last item");
        assert_eq!(selection.row_at(area, Pos::new(0, 4), 3), None, "blank row below a short list");
        assert_eq!(selection.row_at(area, Pos::new(0, 9), 10), None, "outside the region");
        assert_eq!(selection.row_at(area, Pos::new(20, 0), 10), None, "right of the region");
    }

    #[test]
    fn clicking_a_closed_field_opens_it_and_clicking_elsewhere_does_not() {
        let mut dropdown = Dropdown::new();
        dropdown.set_field(Rect::new(4, 2, 20, 1));
        assert!(!dropdown.handle_mouse(&click(4, 5), 3, SCREEN));
        assert!(!dropdown.is_open());
        assert!(dropdown.handle_mouse(&click(6, 2), 3, SCREEN));
        assert!(dropdown.is_open());
    }

    #[test]
    fn a_wheel_over_a_closed_field_is_left_alone() {
        // Deliberate: a form that changes a value while the user scrolls past it is a data-loss
        // bug wearing a convenience costume.
        let mut dropdown = Dropdown::new();
        dropdown.set_field(Rect::new(0, 0, 20, 1));
        assert!(!dropdown.handle_mouse(&wheel(true), 3, SCREEN));
        assert_eq!(dropdown.selected(), 0);
    }

    #[test]
    fn clicking_an_option_selects_it_and_closes_the_list() {
        let mut dropdown = Dropdown::new();
        dropdown.set_field(Rect::new(4, 2, 20, 1));
        dropdown.open();
        // The list is below the field, with a row of border: option 0 is at y = 4.
        assert_eq!(dropdown.option_at(Pos::new(6, 4), 3, SCREEN), Some(0));
        assert_eq!(dropdown.option_at(Pos::new(6, 6), 3, SCREEN), Some(2));
        assert_eq!(dropdown.option_at(Pos::new(6, 3), 3, SCREEN), None, "the border is not a row");
        assert!(dropdown.handle_mouse(&click(6, 6), 3, SCREEN));
        assert!(!dropdown.is_open());
        assert_eq!(dropdown.selected(), 2);
    }

    #[test]
    fn clicking_outside_an_open_list_dismisses_it_and_the_click_goes_no_further() {
        let mut dropdown = Dropdown::at(1);
        dropdown.set_field(Rect::new(4, 2, 20, 1));
        dropdown.open();
        dropdown.set_selected(2);
        // Consumed, so the control that happens to be under the pointer does not also act.
        assert!(dropdown.handle_mouse(&click(30, 15), 3, SCREEN));
        assert!(!dropdown.is_open());
        assert_eq!(dropdown.selected(), 1, "dismissing restores, the same as Escape");
    }

    #[test]
    fn a_wheel_over_an_open_list_moves_the_highlight_without_choosing_anything() {
        let mut dropdown = Dropdown::new().rows(3);
        dropdown.set_field(Rect::new(0, 0, 20, 1));
        dropdown.open();
        assert!(dropdown.handle_mouse(&wheel(true), 9, SCREEN));
        assert_eq!(dropdown.selected(), 1);
        assert!(dropdown.is_open(), "the wheel does not commit a value");
        // And because it does not, the wheel is free: dismissing still restores what was there.
        dropdown.dismiss();
        assert_eq!(dropdown.selected(), 0);
    }

    #[test]
    fn a_closed_dropdown_has_no_options_anywhere() {
        let dropdown = Dropdown::new();
        dropdown.set_field(Rect::new(0, 0, 20, 1));
        assert_eq!(dropdown.option_at(Pos::new(2, 2), 3, SCREEN), None);
    }

    #[test]
    fn hits_answer_with_the_last_region_recorded_over_a_cell() {
        #[derive(Clone, Copy, PartialEq, Debug)]
        enum Id {
            Form,
            Overlay,
        }
        let hits = Hits::new();
        hits.record(Id::Form, Rect::new(0, 0, 20, 10));
        hits.record(Id::Overlay, Rect::new(4, 4, 6, 3));
        // The overlay was drawn second, so it is what the eye sees and what the click hits.
        assert_eq!(hits.at(Pos::new(5, 5)), Some(Id::Overlay));
        assert_eq!(hits.at(Pos::new(1, 1)), Some(Id::Form));
        assert_eq!(hits.at(Pos::new(30, 1)), None);
        assert_eq!(hits.area_of(Id::Overlay), Some(Rect::new(4, 4, 6, 3)));
        assert_eq!(hits.local(Id::Overlay, Pos::new(5, 5)), Some(Pos::new(1, 1)));
        assert_eq!(hits.local(Id::Overlay, Pos::new(1, 1)), None);
        assert_eq!(hits.len(), 2);
        hits.clear();
        assert!(hits.is_empty());
        assert_eq!(hits.at(Pos::new(5, 5)), None, "a stale hit is worse than a missing one");
    }
}
