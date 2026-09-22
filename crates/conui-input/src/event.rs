//! What the user did, as a value.

/// Held modifier keys.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const SHIFT: Self = Self(1 << 0);
    pub const ALT: Self = Self(1 << 1);
    pub const CTRL: Self = Self(1 << 2);
    /// Command on macOS, Windows key elsewhere. Only reported by terminals that implement an
    /// extended keyboard protocol; the legacy encoding has no room for it.
    pub const SUPER: Self = Self(1 << 3);
    pub const HYPER: Self = Self(1 << 4);
    pub const META: Self = Self(1 << 5);

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

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
    Enter,
    Tab,
    /// `Shift+Tab`, which terminals report as its own sequence rather than Tab plus a modifier.
    BackTab,
    Backspace,
    Escape,
    Delete,
    Insert,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    /// Function key, 1-based.
    F(u8),
    /// The centre key of the numeric keypad with Num Lock off.
    KeypadBegin,
    Menu,
}

/// Press, repeat or release.
///
/// Legacy terminals only ever report a press; repeat and release require an extended
/// keyboard protocol. An app that cares should treat anything other than
/// [`KeyEventKind::Release`] as "the key is down".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KeyEventKind {
    #[default]
    Press,
    Repeat,
    Release,
}

/// A key, its modifiers, and whether it went down or up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: Modifiers,
    pub kind: KeyEventKind,
}

impl KeyEvent {
    pub const fn new(code: KeyCode, modifiers: Modifiers) -> Self {
        Self { code, modifiers, kind: KeyEventKind::Press }
    }

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
    Left,
    Middle,
    Right,
}

/// What the mouse did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseKind {
    Down(MouseButton),
    Up(MouseButton),
    /// Motion with a button held.
    Drag(MouseButton),
    /// Motion with no button held. Only reported when the app asks for all-motion tracking.
    Moved,
    ScrollUp,
    ScrollDown,
    ScrollLeft,
    ScrollRight,
}

/// A mouse event at a cell position. Coordinates are zero-based, matching [`conui_cell::Rect`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MouseEvent {
    pub kind: MouseKind,
    pub column: u16,
    pub row: u16,
    pub modifiers: Modifiers,
}

/// Anything the terminal can tell us.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    /// A block of pasted text, delivered whole.
    ///
    /// Without bracketed paste this would arrive as hundreds of individual key events, and an
    /// app could not tell a paste from very fast typing — which matters, because a pasted
    /// newline should usually insert a line rather than submit a form.
    Paste(String),
    /// The terminal window gained or lost focus.
    FocusGained,
    FocusLost,
    /// A reply to a cursor-position query, zero-based.
    CursorPosition {
        column: u16,
        row: u16,
    },
    /// The terminal was resized. Synthesised by the event loop, not parsed from the stream.
    Resize {
        width: u16,
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
