//! Glyphs used as a type system, not decoration.
//!
//! A terminal cell is roughly twice as tall as it is wide, and Unicode provides half-blocks
//! that fill exactly the top or bottom half of one. Combine them and a character cell grid
//! becomes a coarse pixel grid at double vertical resolution — enough to draw a legible
//! seven-segment digit in three rows, which is how a console app displays a score at a size
//! the eye reads from across a desk without any graphics at all.
//!
//! Everything here is a plain `&str` or `char`, so the immediate-mode canvas and the
//! declarative widgets draw from the same vocabulary.

/// Digits drawn three rows tall, three columns wide.
///
/// Each entry is top, middle, bottom. Read `█` as "both halves of this cell", `▀` as "top
/// half only" and `▄` as "bottom half only"; a `▀` on the bottom row is therefore a segment
/// sitting at the cell's vertical midpoint, which is what makes a three-row digit possible at
/// all — the glyph's baseline lands halfway through the last row rather than below it.
const GLYPHS: [(char, [&str; 3]); 17] = [
    ('0', ["█▀█", "█ █", "▀▀▀"]),
    ('1', ["▄█ ", " █ ", "▀▀▀"]),
    ('2', ["▀▀█", "█▀▀", "▀▀▀"]),
    ('3', ["▀▀█", "▀▀█", "▀▀▀"]),
    ('4', ["█ █", "▀▀█", "  ▀"]),
    ('5', ["█▀▀", "▀▀█", "▀▀▀"]),
    ('6', ["█▀▀", "█▀█", "▀▀▀"]),
    ('7', ["▀▀█", "  █", "  ▀"]),
    ('8', ["█▀█", "█▀█", "▀▀▀"]),
    ('9', ["█▀█", "▀▀█", "▀▀▀"]),
    (' ', ["   ", "   ", "   "]),
    ('-', ["   ", "▀▀▀", "   "]),
    ('.', ["   ", "   ", "▀  "]),
    (',', ["   ", "   ", " ▀ "]),
    (':', [" ▀ ", "   ", " ▀ "]),
    ('/', ["  █", " ▀ ", "█  "]),
    ('%', ["▀ █", " ▀ ", "█ ▀"]),
];

/// Rows in a large digit.
pub const DIGIT_HEIGHT: u16 = 3;
/// Columns one large digit occupies.
pub const DIGIT_WIDTH: u16 = 3;
/// Columns from one digit's left edge to the next: the glyph plus one column of tracking.
pub const DIGIT_ADVANCE: u16 = 4;

/// The three rows of a large glyph, or `None` if this character has no large form.
pub fn large_glyph(character: char) -> Option<[&'static str; 3]> {
    GLYPHS.iter().find(|(candidate, _)| *candidate == character).map(|(_, rows)| *rows)
}

/// Whether [`large_glyph`] can render every character of `text`.
pub fn is_renderable_large(text: &str) -> bool {
    text.chars().all(|character| large_glyph(character).is_some())
}

/// Columns a string occupies when drawn as large glyphs.
pub fn large_width(text: &str) -> u16 {
    let count = text.chars().count() as u16;
    if count == 0 { 0 } else { count * DIGIT_ADVANCE - (DIGIT_ADVANCE - DIGIT_WIDTH) }
}

/// Solid and shaded fills, in increasing weight.
pub mod block {
    pub const FULL: char = '█';
    pub const UPPER_HALF: char = '▀';
    pub const LOWER_HALF: char = '▄';
    pub const LEFT_HALF: char = '▌';
    pub const RIGHT_HALF: char = '▐';
    pub const LIGHT_SHADE: char = '░';
    pub const MEDIUM_SHADE: char = '▒';
    pub const DARK_SHADE: char = '▓';
}

/// Box-drawing pieces, in light and heavy weights.
pub mod line {
    pub const HORIZONTAL: char = '─';
    pub const VERTICAL: char = '│';
    pub const TOP_LEFT: char = '┌';
    pub const TOP_RIGHT: char = '┐';
    pub const BOTTOM_LEFT: char = '└';
    pub const BOTTOM_RIGHT: char = '┘';
    pub const CROSS: char = '┼';
    pub const TEE_DOWN: char = '┬';
    pub const TEE_UP: char = '┴';
    pub const TEE_RIGHT: char = '├';
    pub const TEE_LEFT: char = '┤';

    pub const HEAVY_HORIZONTAL: char = '━';
    pub const HEAVY_VERTICAL: char = '┃';

    pub const DOUBLE_HORIZONTAL: char = '═';
    pub const DOUBLE_VERTICAL: char = '║';

    pub const ROUND_TOP_LEFT: char = '╭';
    pub const ROUND_TOP_RIGHT: char = '╮';
    pub const ROUND_BOTTOM_LEFT: char = '╰';
    pub const ROUND_BOTTOM_RIGHT: char = '╯';
}

/// Eight levels of partial vertical fill, for sparklines and column charts.
///
/// Index 0 is empty; index 8 is a full cell. Because each step is an eighth of a cell, a
/// sparkline in a single row has eight levels of resolution rather than two.
pub const VERTICAL_LEVELS: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Eight levels of partial horizontal fill, for bars that need sub-cell precision.
pub const HORIZONTAL_LEVELS: [char; 9] = [' ', '▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

/// How a bar draws its filled and empty portions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BarStyle {
    /// A heavy rule throughout, the fill distinguished only by colour. The quietest option,
    /// and the one that reads as a measurement rather than as a widget.
    #[default]
    Rule,
    /// Solid blocks over a light shade. Reads as a level or a quantity.
    Shaded,
    /// Solid blocks over nothing, so the track is invisible.
    Blocks,
    /// Solid blocks with an eighth-cell partial at the leading edge, for a smooth bar that
    /// still moves when the value changes by less than one column.
    Smooth,
}

impl BarStyle {
    /// The character for the filled run.
    pub const fn fill(self) -> char {
        match self {
            Self::Rule => line::HEAVY_HORIZONTAL,
            Self::Shaded | Self::Blocks | Self::Smooth => block::FULL,
        }
    }

    /// The character for the empty run, or `None` to leave it untouched.
    pub const fn track(self) -> Option<char> {
        match self {
            Self::Rule => Some(line::HEAVY_HORIZONTAL),
            Self::Shaded => Some(block::LIGHT_SHADE),
            Self::Blocks | Self::Smooth => None,
        }
    }

    /// Whether the leading edge gets a partial-cell glyph.
    pub const fn is_smooth(self) -> bool {
        matches!(self, Self::Smooth)
    }
}

/// Small marks that carry meaning without a whole widget around them.
pub mod mark {
    /// Marks the selected row of a list. Reads as a cursor without stealing a whole column
    /// the way `>` does.
    pub const SELECTED: char = '›';
    pub const BULLET: char = '·';
    pub const DOT: char = '●';
    pub const RING: char = '○';
    pub const DIAMOND: char = '◆';
    pub const ARROW_UP: char = '↑';
    pub const ARROW_DOWN: char = '↓';
    pub const ARROW_LEFT: char = '←';
    pub const ARROW_RIGHT: char = '→';
    pub const ELLIPSIS: char = '…';
    /// Stands in for a value that is absent rather than zero.
    pub const EMPTY: char = '—';
    pub const CHECK: char = '✓';
    pub const CROSS: char = '✗';
    /// Separates items on a status line.
    pub const SEPARATOR: char = '·';
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn every_large_glyph_is_exactly_three_by_three() {
        for (character, rows) in GLYPHS {
            for (index, row) in rows.iter().enumerate() {
                assert_eq!(
                    row.width(),
                    DIGIT_WIDTH as usize,
                    "glyph {character:?} row {index} is {row:?}, which is not 3 columns wide"
                );
            }
        }
    }

    #[test]
    fn large_glyphs_use_only_single_width_characters() {
        // A double-width character would silently shift every digit after it, so the whole
        // number would come out misaligned. Cheaper to assert than to debug.
        for (character, rows) in GLYPHS {
            for row in rows {
                for glyph in row.chars() {
                    assert_eq!(
                        glyph.to_string().width(),
                        1,
                        "glyph {character:?} contains a non-single-width character {glyph:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn all_ten_digits_have_a_large_form() {
        for digit in '0'..='9' {
            assert!(large_glyph(digit).is_some(), "{digit} has no large form");
        }
    }

    #[test]
    fn large_width_accounts_for_tracking_but_not_a_trailing_gap() {
        assert_eq!(large_width(""), 0);
        assert_eq!(large_width("7"), 3);
        // Two digits: three columns, a gap, three columns.
        assert_eq!(large_width("42"), 7);
        assert_eq!(large_width("000"), 11);
    }

    #[test]
    fn renderability_is_reported_honestly() {
        assert!(is_renderable_large("12:34"));
        assert!(!is_renderable_large("12ms"));
    }

    #[test]
    fn bar_styles_agree_on_which_have_a_track() {
        assert_eq!(BarStyle::Rule.track(), Some('━'));
        assert_eq!(BarStyle::Shaded.track(), Some('░'));
        assert_eq!(BarStyle::Blocks.track(), None);
        assert!(BarStyle::Smooth.is_smooth());
        assert!(!BarStyle::Rule.is_smooth());
    }

    #[test]
    fn the_partial_fill_ramps_are_monotone_and_bounded() {
        assert_eq!(VERTICAL_LEVELS[0], ' ');
        assert_eq!(VERTICAL_LEVELS[8], '█');
        assert_eq!(HORIZONTAL_LEVELS[0], ' ');
        assert_eq!(HORIZONTAL_LEVELS[8], '█');
        for ramp in [VERTICAL_LEVELS, HORIZONTAL_LEVELS] {
            for glyph in ramp {
                assert_eq!(glyph.to_string().width(), 1, "{glyph:?} is not one column");
            }
        }
    }
}
