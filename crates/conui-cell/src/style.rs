//! Text attributes and the [`Style`] that a cell carries.
//!
//! Styles compose by patching: a child states only what it overrides, and everything else
//! is inherited from its parent. That is what lets a theme own the palette while a widget
//! says nothing more than "bold".

use crate::Color;

/// A set of SGR text attributes.
///
/// A hand-rolled bitset rather than a `bitflags` dependency: the operations we need are a
/// union, a difference and a test, and keeping it inline means this crate's dependency list
/// stays down to the two Unicode crates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Attrs(u16);

impl Attrs {
    /// No attributes.
    pub const NONE: Self = Self(0);
    /// SGR 1. Brighter or heavier, depending on the terminal.
    pub const BOLD: Self = Self(1 << 0);
    /// SGR 2. Faint; what the `Dim` role leans on.
    pub const DIM: Self = Self(1 << 1);
    /// SGR 3. Not honoured everywhere, so never the only thing carrying a meaning.
    pub const ITALIC: Self = Self(1 << 2);
    /// SGR 4.
    pub const UNDERLINE: Self = Self(1 << 3);
    /// SGR 5. Widely ignored, and unkind where it is not.
    pub const BLINK: Self = Self(1 << 4);
    /// SGR 7. Swaps foreground and background.
    pub const REVERSE: Self = Self(1 << 5);
    /// SGR 8. The cell keeps its width but shows nothing.
    pub const HIDDEN: Self = Self(1 << 6);
    /// SGR 9.
    pub const STRIKETHROUGH: Self = Self(1 << 7);

    /// Every attribute at once. Mostly useful as a mask.
    pub const ALL: Self = Self(0xff);

    /// The raw bitset, for a caller that needs to store or compare it as a number.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// A set from raw bits, dropping any that name no attribute.
    pub const fn from_bits_truncate(bits: u16) -> Self {
        Self(bits & Self::ALL.0)
    }

    /// True when no attribute is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when every attribute in `other` is present in `self`.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// True when the two sets share at least one attribute.
    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// Everything in either set.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Everything in `self` that is not in `other`.
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Everything in both sets.
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Iterate the individual attributes that are set, lowest bit first.
    pub fn iter(self) -> impl Iterator<Item = Self> {
        (0..16).map(|bit| Self(1 << bit)).filter(move |flag| self.intersects(*flag))
    }
}

impl std::ops::BitOr for Attrs {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for Attrs {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

impl std::ops::BitAnd for Attrs {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self {
        self.intersection(rhs)
    }
}

impl std::ops::Sub for Attrs {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        self.difference(rhs)
    }
}

impl std::ops::Not for Attrs {
    type Output = Self;
    fn not(self) -> Self {
        Self(!self.0 & Self::ALL.0)
    }
}

/// The full appearance of a cell: a foreground, a background, and text attributes.
///
/// `None` for a color means "inherit" during a [`Style::patch`], and "the terminal default"
/// once it reaches the writer. Attributes are tracked as an add set and a remove set so that
/// patching is associative: `a.patch(b).patch(c)` equals `a.patch(b.patch(c))`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    /// Foreground color, or `None` to inherit.
    pub fg: Option<Color>,
    /// Background color, or `None` to inherit.
    pub bg: Option<Color>,
    /// Attributes this style turns on.
    pub attrs: Attrs,
    /// Attributes this style explicitly turns off, overriding an inherited value.
    pub cleared: Attrs,
}

impl Style {
    /// A style that changes nothing. Patching with it is a no-op.
    pub const EMPTY: Self = Self { fg: None, bg: None, attrs: Attrs::NONE, cleared: Attrs::NONE };

    /// An empty style, to build on. The same as [`Style::EMPTY`].
    pub const fn new() -> Self {
        Self::EMPTY
    }

    /// State a foreground color.
    pub const fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    /// State a background color.
    pub const fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    /// Turn attributes on, cancelling any matching entry in the remove set.
    pub const fn add(mut self, attrs: Attrs) -> Self {
        self.attrs = self.attrs.union(attrs);
        self.cleared = self.cleared.difference(attrs);
        self
    }

    /// Turn attributes off, even if a parent style had switched them on.
    pub const fn remove(mut self, attrs: Attrs) -> Self {
        self.cleared = self.cleared.union(attrs);
        self.attrs = self.attrs.difference(attrs);
        self
    }

    /// Add [`Attrs::BOLD`].
    pub const fn bold(self) -> Self {
        self.add(Attrs::BOLD)
    }

    /// Add [`Attrs::DIM`].
    pub const fn dim(self) -> Self {
        self.add(Attrs::DIM)
    }

    /// Add [`Attrs::ITALIC`].
    pub const fn italic(self) -> Self {
        self.add(Attrs::ITALIC)
    }

    /// Add [`Attrs::UNDERLINE`].
    pub const fn underline(self) -> Self {
        self.add(Attrs::UNDERLINE)
    }

    /// Add [`Attrs::REVERSE`].
    pub const fn reverse(self) -> Self {
        self.add(Attrs::REVERSE)
    }

    /// Add [`Attrs::STRIKETHROUGH`].
    pub const fn strikethrough(self) -> Self {
        self.add(Attrs::STRIKETHROUGH)
    }

    /// Lay `over` on top of `self`: its colors win where it states one, and its attribute
    /// add/remove sets are applied in that order.
    pub fn patch(mut self, over: Self) -> Self {
        if let Some(fg) = over.fg {
            self.fg = Some(fg);
        }
        if let Some(bg) = over.bg {
            self.bg = Some(bg);
        }
        self.attrs = self.attrs.union(over.attrs).difference(over.cleared);
        self.cleared = self.cleared.union(over.cleared).difference(over.attrs);
        self
    }

    /// Resolve to concrete values for the writer, filling unset colors from `fallback`.
    pub fn resolve(self, fallback: ResolvedStyle) -> ResolvedStyle {
        ResolvedStyle {
            fg: self.fg.unwrap_or(fallback.fg),
            bg: self.bg.unwrap_or(fallback.bg),
            attrs: fallback.attrs.union(self.attrs).difference(self.cleared),
        }
    }
}

impl From<Color> for Style {
    /// A bare color reads as a foreground, which is what makes `c.text(.., MUTED)` work.
    fn from(color: Color) -> Self {
        Self::EMPTY.fg(color)
    }
}

/// A style with every field decided, as handed to the terminal writer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ResolvedStyle {
    /// Foreground color; [`Color::Reset`] where nothing stated one.
    pub fg: Color,
    /// Background color; [`Color::Reset`] where nothing stated one.
    pub bg: Color,
    /// Attributes to switch on for this cell.
    pub attrs: Attrs,
}

impl Default for ResolvedStyle {
    fn default() -> Self {
        Self { fg: Color::Reset, bg: Color::Reset, attrs: Attrs::NONE }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_overrides_stated_colors_and_inherits_the_rest() {
        let base = Style::new().fg(Color::hex("#e3f3ef")).bg(Color::hex("#090f13")).bold();
        let patched = base.patch(Style::new().fg(Color::hex("#62f5b5")));
        assert_eq!(patched.fg, Some(Color::hex("#62f5b5")));
        assert_eq!(patched.bg, Some(Color::hex("#090f13")), "background should be inherited");
        assert!(patched.attrs.contains(Attrs::BOLD), "bold should be inherited");
    }

    #[test]
    fn remove_beats_an_inherited_add() {
        let base = Style::new().bold().italic();
        let patched = base.patch(Style::new().remove(Attrs::BOLD));
        assert!(!patched.attrs.contains(Attrs::BOLD));
        assert!(patched.attrs.contains(Attrs::ITALIC));
        assert!(patched.cleared.contains(Attrs::BOLD));
    }

    #[test]
    fn add_and_remove_are_mutually_exclusive() {
        let style = Style::new().bold().remove(Attrs::BOLD).bold();
        assert!(style.attrs.contains(Attrs::BOLD));
        assert!(!style.cleared.contains(Attrs::BOLD));
    }

    #[test]
    fn patch_is_associative() {
        let a = Style::new().fg(Color::RED).bold();
        let b = Style::new().bg(Color::BLUE).remove(Attrs::BOLD);
        let c = Style::new().italic().fg(Color::GREEN);
        assert_eq!(a.patch(b).patch(c), a.patch(b.patch(c)));
    }

    #[test]
    fn patching_with_empty_changes_nothing() {
        let style = Style::new().fg(Color::CYAN).dim().remove(Attrs::BOLD);
        assert_eq!(style.patch(Style::EMPTY), style);
        assert_eq!(Style::EMPTY.patch(style), style);
    }

    #[test]
    fn resolve_fills_gaps_from_the_fallback() {
        let fallback = ResolvedStyle { fg: Color::WHITE, bg: Color::BLACK, attrs: Attrs::DIM };
        let resolved = Style::new().fg(Color::RED).bold().resolve(fallback);
        assert_eq!(resolved.fg, Color::RED);
        assert_eq!(resolved.bg, Color::BLACK);
        assert!(resolved.attrs.contains(Attrs::BOLD | Attrs::DIM));
    }

    #[test]
    fn attrs_iter_yields_each_set_flag_once() {
        let attrs = Attrs::BOLD | Attrs::UNDERLINE | Attrs::REVERSE;
        let flags: Vec<_> = attrs.iter().collect();
        assert_eq!(flags, vec![Attrs::BOLD, Attrs::UNDERLINE, Attrs::REVERSE]);
    }
}
