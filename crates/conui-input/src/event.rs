//! What the user did, as a value.

/// Held modifier keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    /// Nothing held.
    pub const NONE: Self = Self(0);
    /// Shift. A shifted letter also arrives as its uppercase [`KeyCode::Char`].
    pub const SHIFT: Self = Self(1 << 0);
    /// Alt, which some terminals send as an `Escape` prefix instead.
    pub const ALT: Self = Self(1 << 1);
    /// Control.
    pub const CTRL: Self = Self(1 << 2);
    /// Command on macOS, Windows key elsewhere. Only reported by terminals that implement an
    /// extended keyboard protocol; the legacy encoding has no room for it.
    pub const SUPER: Self = Self(1 << 3);
    /// Hyper. Extended keyboard protocols only, and rare on real keyboards.
    pub const HYPER: Self = Self(1 << 4);
    /// Meta. Extended keyboard protocols only.
    pub const META: Self = Self(1 << 5);

    /// The raw bitset.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// True when nothing is held.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// True when every modifier in `other` is held.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// Everything held in either set.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Everything in `self` that is not in `other`, for ignoring a modifier you do not care
    /// about: `modifiers.difference(Modifiers::SHIFT).is_empty()`.
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Decode the legacy CSI modifier parameter, which is `1 + bitmask`.
    ///
    /// Terminals encode "no modifiers" as 1, so a 0 or absent parameter also means none.
    pub const fn from_csi_param(param: u16) -> Self {
        if param == 0 {
            return Self::NONE;
        }
        let mask = param - 1;
        let mut modifiers = Self::NONE;
        if mask & 0b0000_0001 != 0 {
            modifiers = modifiers.union(Self::SHIFT);
        }
        if mask & 0b0000_0010 != 0 {
            modifiers = modifiers.union(Self::ALT);
        }
        if mask & 0b0000_0100 != 0 {
            modifiers = modifiers.union(Self::CTRL);
        }
        if mask & 0b0000_1000 != 0 {
            modifiers = modifiers.union(Self::SUPER);
        }
        if mask & 0b0001_0000 != 0 {
            modifiers = modifiers.union(Self::HYPER);
        }
        if mask & 0b0010_0000 != 0 {
            modifiers = modifiers.union(Self::META);
        }
        modifiers
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        self.union(rhs)
    }
}

impl std::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        *self = self.union(rhs);
    }
}

/// A physical key, after the escape sequence has been decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    /// A character-producing key. Already case-correct: `Shift+a` arrives as `Char('A')`.
    Char(char),
    /// `Enter` or `Return`; terminals do not distinguish them.
    Enter,
    /// `Tab`.
    Tab,
    /// `Shift+Tab`, which terminals report as its own sequence rather than Tab plus a modifier.
    BackTab,
    /// `Backspace`, however this terminal encodes it.
    Backspace,
    /// `Escape` alone, once the parser has ruled out a sequence beginning with it.
    Escape,
    /// Forward delete.
    Delete,
    /// `Insert`.
    Insert,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// `Home`.
    Home,
    /// `End`.
    End,
    /// `Page Up`.
    PageUp,
    /// `Page Down`.
    PageDown,
    /// Function key, 1-based.
    F(u8),
    /// The centre key of the numeric keypad with Num Lock off.
    KeypadBegin,
    /// The context-menu key.
    Menu,
}

/// Press, repeat or release.
///
/// Legacy terminals only ever report a press; repeat and release require an extended
/// keyboard protocol. An app that cares should treat anything other than
/// [`KeyEventKind::Release`] as "the key is down".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KeyEventKind {
    /// The key went down.
    #[default]
    Press,
    /// The key is held and the terminal is auto-repeating it.
    Repeat,
    /// The key came up.
    Release,
}

/// A key, its modifiers, and whether it went down or up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    /// Which key.
    pub code: KeyCode,
    /// What was held with it.
    pub modifiers: Modifiers,
    /// Down, held, or up.
    pub kind: KeyEventKind,
}

impl KeyEvent {
    /// A press of `code` with `modifiers` held.
    pub const fn new(code: KeyCode, modifiers: Modifiers) -> Self {
        Self { code, modifiers, kind: KeyEventKind::Press }
    }

    /// A press of `code` with nothing held. What a key table or a test wants.
    pub const fn plain(code: KeyCode) -> Self {
        Self::new(code, Modifiers::NONE)
    }

    /// Match a plain, unmodified character, case-sensitively.
    pub fn is_char(&self, expected: char) -> bool {
        self.modifiers.is_empty() && self.code == KeyCode::Char(expected)
    }

    /// Match a character ignoring case and ignoring Shift, which is what a single-letter
    /// keyboard shortcut usually wants.
    pub fn is_key(&self, expected: char) -> bool {
        let KeyCode::Char(actual) = self.code else { return false };
        self.modifiers.difference(Modifiers::SHIFT).is_empty()
            && actual.eq_ignore_ascii_case(&expected)
    }

    /// Match `Ctrl` plus a character, ignoring case.
    pub fn is_ctrl(&self, expected: char) -> bool {
        let KeyCode::Char(actual) = self.code else { return false };
        self.modifiers.contains(Modifiers::CTRL) && actual.eq_ignore_ascii_case(&expected)
    }
}

/// Which button a mouse event concerns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// The primary button.
    Left,
    /// The wheel button.
    Middle,
    /// The secondary button.
    Right,
}

/// What the mouse did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseKind {
    /// A button went down. This is the one a click should act on.
    Down(MouseButton),
    /// A button came up.
    Up(MouseButton),
    /// Motion with a button held.
    Drag(MouseButton),
    /// Motion with no button held. Only reported when the app asks for all-motion tracking.
    Moved,
    /// One notch of the wheel away from the user.
    ScrollUp,
    /// One notch of the wheel towards the user.
    ScrollDown,
    /// One notch of horizontal scrolling left, where the hardware has it.
    ScrollLeft,
    /// One notch of horizontal scrolling right, where the hardware has it.
    ScrollRight,
}

impl MouseKind {
    /// The button this concerns, for a press, release or drag.
    pub const fn button(self) -> Option<MouseButton> {
        match self {
            Self::Down(button) | Self::Up(button) | Self::Drag(button) => Some(button),
            _ => None,
        }
    }

    /// Vertical wheel movement in rows: negative up, positive down, `None` if this is not a
    /// vertical scroll. One notch is one event; the terminal reports no magnitude.
    pub const fn scroll(self) -> Option<i32> {
        match self {
            Self::ScrollUp => Some(-1),
            Self::ScrollDown => Some(1),
            _ => None,
        }
    }
}

/// A mouse event at a cell position. Coordinates are zero-based, like every other cell
/// coordinate in the kit — the terminal's own reporting is 1-based, and the parser subtracts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MouseEvent {
    /// What the mouse did.
    pub kind: MouseKind,
    /// Column it happened in, zero-based.
    pub column: u16,
    /// Row it happened in, zero-based.
    pub row: u16,
    /// Modifiers held at the time.
    pub modifiers: Modifiers,
}

impl MouseEvent {
    /// A left-button press: "the user clicked here".
    ///
    /// Press rather than release, which is what makes a click feel immediate. The difference only
    /// shows on a drag out of a control before releasing — a distinction worth having in a form
    /// with a Delete button, and available by matching [`MouseKind`] directly when you want it.
    pub const fn is_click(&self) -> bool {
        matches!(self.kind, MouseKind::Down(MouseButton::Left))
    }
}

/// Anything the terminal can tell us.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A key went down, is repeating, or came up.
    Key(KeyEvent),
    /// The mouse was clicked, moved, or scrolled.
    Mouse(MouseEvent),
    /// A block of pasted text, delivered whole.
    ///
    /// Without bracketed paste this would arrive as hundreds of individual key events, and an
    /// app could not tell a paste from very fast typing — which matters, because a pasted
    /// newline should usually insert a line rather than submit a form.
    Paste(String),
    /// The terminal window gained or lost focus.
    FocusGained,
    /// The terminal window lost focus.
    FocusLost,
    /// A reply to a cursor-position query, zero-based.
    CursorPosition {
        /// Column the cursor is in.
        column: u16,
        /// Row the cursor is in.
        row: u16,
    },
    /// The terminal was resized. Synthesised by the event loop, not parsed from the stream.
    Resize {
        /// New width in columns.
        width: u16,
        /// New height in rows.
        height: u16,
    },
}

impl Event {
    /// The key event, if this is one.
    pub fn as_key(&self) -> Option<&KeyEvent> {
        match self {
            Self::Key(key) => Some(key),
            _ => None,
        }
    }

    /// The mouse event, if this is one.
    pub fn as_mouse(&self) -> Option<&MouseEvent> {
        match self {
            Self::Mouse(mouse) => Some(mouse),
            _ => None,
        }
    }

    /// True for a key press or repeat, false for a release or any non-key event. The usual
    /// guard for "the user asked for this action", which should not fire twice per keystroke.
    pub fn is_press(&self) -> bool {
        matches!(self, Self::Key(key) if key.kind != KeyEventKind::Release)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csi_param_one_means_no_modifiers() {
        assert_eq!(Modifiers::from_csi_param(1), Modifiers::NONE);
        assert_eq!(Modifiers::from_csi_param(0), Modifiers::NONE);
    }

    #[test]
    fn csi_params_decode_the_standard_bitmask() {
        assert_eq!(Modifiers::from_csi_param(2), Modifiers::SHIFT);
        assert_eq!(Modifiers::from_csi_param(3), Modifiers::ALT);
        assert_eq!(Modifiers::from_csi_param(5), Modifiers::CTRL);
        assert_eq!(Modifiers::from_csi_param(4), Modifiers::SHIFT | Modifiers::ALT);
        assert_eq!(
            Modifiers::from_csi_param(8),
            Modifiers::SHIFT | Modifiers::ALT | Modifiers::CTRL
        );
        assert_eq!(Modifiers::from_csi_param(9), Modifiers::SUPER);
    }

    #[test]
    fn is_key_ignores_case_and_shift() {
        let shifted = KeyEvent::new(KeyCode::Char('Q'), Modifiers::SHIFT);
        assert!(shifted.is_key('q'));
        assert!(shifted.is_key('Q'));
        assert!(!shifted.is_char('q'), "is_char is exact and rejects the modifier");
    }

    #[test]
    fn is_key_rejects_a_real_modifier() {
        let with_ctrl = KeyEvent::new(KeyCode::Char('q'), Modifiers::CTRL);
        assert!(!with_ctrl.is_key('q'));
        assert!(with_ctrl.is_ctrl('q'));
    }

    #[test]
    fn a_click_is_a_left_press_and_nothing_else() {
        let click = MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            column: 4,
            row: 2,
            modifiers: Modifiers::NONE,
        };
        assert!(click.is_click());
        assert!(!MouseEvent { kind: MouseKind::Up(MouseButton::Left), ..click }.is_click());
        assert!(!MouseEvent { kind: MouseKind::Down(MouseButton::Right), ..click }.is_click());
        assert_eq!(Event::Mouse(click).as_mouse(), Some(&click));
        assert_eq!(Event::FocusGained.as_mouse(), None);
    }

    #[test]
    fn scroll_reports_a_direction_only_for_the_vertical_wheel() {
        assert_eq!(MouseKind::ScrollUp.scroll(), Some(-1));
        assert_eq!(MouseKind::ScrollDown.scroll(), Some(1));
        assert_eq!(MouseKind::ScrollLeft.scroll(), None);
        assert_eq!(MouseKind::Down(MouseButton::Left).button(), Some(MouseButton::Left));
        assert_eq!(MouseKind::Moved.button(), None);
    }

    #[test]
    fn is_press_excludes_releases() {
        let release = KeyEvent {
            code: KeyCode::Char('a'),
            modifiers: Modifiers::NONE,
            kind: KeyEventKind::Release,
        };
        assert!(!Event::Key(release).is_press());
        assert!(Event::Key(KeyEvent::plain(KeyCode::Char('a'))).is_press());
        assert!(!Event::FocusGained.is_press());
    }
}
