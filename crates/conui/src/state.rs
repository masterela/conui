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

use std::cell::Cell;
use std::ops::Range;

use conui_input::{KeyCode, KeyEvent, KeyEventKind, Modifiers};
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
}

impl Clone for Selection {
    fn clone(&self) -> Self {
        Self { selected: self.selected, offset: Cell::new(self.offset.get()) }
    }
}

impl Selection {
    pub fn new() -> Self {
        Self::default()
    }

    /// A selection starting on a given row.
    pub fn at(index: usize) -> Self {
        Self { selected: index, offset: Cell::new(0) }
    }

    pub const fn selected(&self) -> usize {
        self.selected
    }

    /// The first visible row, as of the last render.
    pub fn offset(&self) -> usize {
        self.offset.get()
    }

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

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

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

    pub fn first(&mut self) {
        self.selected = 0;
    }

    pub fn last(&mut self, len: usize) {
        self.selected = len.saturating_sub(1);
    }

    pub fn page_up(&mut self, rows: usize) {
        self.selected = self.selected.saturating_sub(rows);
    }

    pub fn page_down(&mut self, rows: usize, len: usize) {
        self.selected = (self.selected + rows).min(len.saturating_sub(1));
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

    /// Scroll the window without moving the selection, for a mouse wheel.
    ///
    /// The selection is left where it is even if it scrolls out of sight, matching what every
    /// other scrollable thing does; the next keypress brings the window back to it.
    pub fn scroll(&self, delta: i32, len: usize) {
        let offset = self.offset.get() as i64 + i64::from(delta);
        self.offset.set(offset.clamp(0, len.saturating_sub(1) as i64) as usize);
    }

    /// The rows a list of `len` items should draw into `height` rows, scrolling to keep the
    /// selection visible, and recording the offset for next frame.
    pub fn window(&self, height: u16, len: usize) -> Range<usize> {
        let height = usize::from(height);
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
    pub fn new() -> Self {
        Self::default()
    }

    /// An editor holding `text`, cursor at the end.
    pub fn with(text: impl Into<String>) -> Self {
        let value: String = text.into();
        let cursor = value.len();
        Self { value, cursor }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    /// Byte offset of the cursor within [`Editor::value`].
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    /// Display width of the text before the cursor: where to draw it, in cells.
    pub fn cursor_column(&self) -> u16 {
        text_width(&self.value[..self.cursor])
    }

    /// Replace the text, putting the cursor at the end.
    pub fn set_value(&mut self, text: impl Into<String>) {
        self.value = text.into();
        self.cursor = self.value.len();
    }

    pub fn clear(&mut self) {
        self.value.clear();
        self.cursor = 0;
    }

    /// Take the text, leaving the editor empty. The shape a "submit" handler wants.
    pub fn take(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.value)
    }

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

    pub fn left(&mut self) {
        if let Some(previous) = self.previous_boundary() {
            self.cursor = previous;
        }
    }

    pub fn right(&mut self) {
        if let Some(next) = self.next_boundary() {
            self.cursor = next;
        }
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

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
        let selection = Selection::at(0);
        selection.scroll(18, 20);
        // Scrolled near the end, then asked for ten rows: it must back up to show ten.
        assert_eq!(selection.window(10, 20), 0..10, "should also follow the selection back up");

        let selection = Selection::at(19);
        selection.scroll(19, 20);
        assert_eq!(selection.window(10, 20), 10..20);
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
        selection.page_down(10, 25);
        assert_eq!(selection.selected(), 10);
        selection.page_down(10, 25);
        selection.page_down(10, 25);
        assert_eq!(selection.selected(), 24);
        selection.page_up(10);
        assert_eq!(selection.selected(), 14);
        selection.page_up(100);
        assert_eq!(selection.selected(), 0);
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
}
