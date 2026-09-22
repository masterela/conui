//! A single terminal cell.
//!
//! A cell holds a whole grapheme cluster, not a `char`. `é` written as `e` + U+0301, a flag
//! emoji, or an emoji with a skin-tone modifier are each one cell's worth of text made of
//! several `char`s, and splitting them would corrupt the output.

use unicode_width::UnicodeWidthStr;

use crate::Style;

/// An inline grapheme cluster.
///
/// Stored inline rather than as a `String` so that a cell is `Copy` and a full-screen buffer
/// is one flat allocation. 15 bytes covers every practical cluster, including emoji with
/// variation selectors and zero-width joiners; anything longer is truncated to its leading
/// scalar value, which is the same fallback a terminal applies when it cannot compose.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Symbol {
    bytes: [u8; Self::CAPACITY],
    len: u8,
}

impl Symbol {
    /// Bytes a cluster can occupy before it is truncated to its base character.
    pub const CAPACITY: usize = 15;

    /// The absence of a symbol, used for the trailing half of a double-width cell.
    /// The writer skips these: the wide grapheme to the left already covered the column.
    pub const EMPTY: Self = Self { bytes: [0; Self::CAPACITY], len: 0 };

    /// A single space. The default content of a cell.
    pub const SPACE: Self = Self::from_ascii(b' ');

    /// One 7-bit byte as a symbol, `const` so the widget set can name its glyphs as constants.
    ///
    /// # Panics
    ///
    /// When `byte` is not ASCII: a lone UTF-8 continuation byte is not a cluster.
    pub const fn from_ascii(byte: u8) -> Self {
        assert!(byte < 0x80, "from_ascii requires a 7-bit value");
        let mut bytes = [0; Self::CAPACITY];
        bytes[0] = byte;
        Self { bytes, len: 1 }
    }

    /// Store `text` as one cluster, truncating to the first scalar value if it cannot fit.
    pub fn new(text: &str) -> Self {
        let source = text.as_bytes();
        if source.len() <= Self::CAPACITY {
            let mut bytes = [0; Self::CAPACITY];
            bytes[..source.len()].copy_from_slice(source);
            return Self { bytes, len: source.len() as u8 };
        }
        // Too long to inline: keep the base character so the cell still renders something
        // recognisable instead of being dropped.
        match text.chars().next() {
            Some(first) => {
                let mut buffer = [0u8; 4];
                Self::new(first.encode_utf8(&mut buffer))
            }
            None => Self::EMPTY,
        }
    }

    /// The cluster as text. Empty for a continuation cell.
    pub fn as_str(&self) -> &str {
        // Safe: every constructor copies from a `&str` or a single ASCII byte, so the
        // occupied prefix is always valid UTF-8.
        std::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or("")
    }

    /// True for the empty symbol, which is to say: a continuation, not a space.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Columns this symbol occupies: 0 for a continuation, 2 for a wide grapheme, else 1.
    pub fn width(&self) -> u16 {
        if self.is_empty() {
            return 0;
        }
        // Control characters report width 0 but still consume a cell in our buffer; clamp so
        // that a stray control byte cannot desynchronise the layout.
        UnicodeWidthStr::width(self.as_str()).clamp(1, 2) as u16
    }
}

impl Default for Symbol {
    fn default() -> Self {
        Self::SPACE
    }
}

impl std::fmt::Debug for Symbol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_empty() {
            f.write_str("Symbol(∅)")
        } else {
            write!(f, "Symbol({:?})", self.as_str())
        }
    }
}

impl From<char> for Symbol {
    fn from(value: char) -> Self {
        let mut buffer = [0u8; 4];
        Self::new(value.encode_utf8(&mut buffer))
    }
}

impl From<&str> for Symbol {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

/// One cell: what to draw, and how it looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Cell {
    /// The grapheme cluster drawn in this cell.
    pub symbol: Symbol,
    /// How it is drawn. Unset colors are resolved against the theme by the writer.
    pub style: Style,
}

impl Cell {
    /// A blank cell with no styling.
    pub const BLANK: Self = Self { symbol: Symbol::SPACE, style: Style::EMPTY };

    /// The trailing column of a double-width grapheme.
    pub const CONTINUATION: Self = Self { symbol: Symbol::EMPTY, style: Style::EMPTY };

    /// A cell showing `symbol` in `style`.
    pub fn new(symbol: impl Into<Symbol>, style: Style) -> Self {
        Self { symbol: symbol.into(), style }
    }

    /// True when this cell is the tail of a wide grapheme rather than content of its own.
    pub const fn is_continuation(&self) -> bool {
        self.symbol.is_empty()
    }

    /// Columns this cell's symbol occupies: 0 for a continuation, 2 for a wide grapheme, else 1.
    pub fn width(&self) -> u16 {
        self.symbol.width()
    }

    /// Reset to blank, keeping nothing. Used when clearing a region.
    pub fn reset(&mut self) {
        *self = Self::BLANK;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_and_box_glyphs_are_one_column() {
        assert_eq!(Symbol::new("a").width(), 1);
        assert_eq!(Symbol::new("█").width(), 1);
        assert_eq!(Symbol::new("━").width(), 1);
        assert_eq!(Symbol::new("·").width(), 1);
    }

    #[test]
    fn cjk_and_emoji_are_two_columns() {
        assert_eq!(Symbol::new("界").width(), 2);
        assert_eq!(Symbol::new("🦀").width(), 2);
    }

    #[test]
    fn combining_sequences_survive_as_one_symbol() {
        let combined = Symbol::new("e\u{301}");
        assert_eq!(combined.as_str(), "e\u{301}");
        assert_eq!(combined.width(), 1);
    }

    #[test]
    fn zwj_emoji_sequence_is_stored_whole_when_it_fits() {
        // Woman + ZWJ + rocket: 11 bytes, inside the inline budget.
        let sequence = "\u{1f469}\u{200d}\u{1f680}";
        assert!(sequence.len() <= Symbol::CAPACITY);
        assert_eq!(Symbol::new(sequence).as_str(), sequence);
    }

    #[test]
    fn oversized_cluster_degrades_to_its_base_character() {
        // A four-person family emoji is 25 bytes, past what we inline.
        let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}";
        assert!(family.len() > Symbol::CAPACITY);
        assert_eq!(Symbol::new(family).as_str(), "\u{1f468}");
    }

    #[test]
    fn continuation_has_no_width_and_is_flagged() {
        assert_eq!(Symbol::EMPTY.width(), 0);
        assert!(Cell::CONTINUATION.is_continuation());
        assert!(!Cell::BLANK.is_continuation());
    }

    #[test]
    fn control_characters_never_report_zero_width() {
        // A tab or a bell must still be accounted one column so the grid stays aligned.
        assert_eq!(Symbol::new("\t").width(), 1);
        assert_eq!(Symbol::new("\u{7}").width(), 1);
    }

    #[test]
    fn a_cell_stays_copy_and_compact() {
        assert!(std::mem::size_of::<Cell>() <= 32, "cell grew to {}", std::mem::size_of::<Cell>());
        fn assert_copy<T: Copy>() {}
        assert_copy::<Cell>();
    }
}
