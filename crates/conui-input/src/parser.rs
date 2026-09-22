//! An incremental parser for the terminal input stream.
//!
//! Terminal input is a byte stream with no framing. A keypress may arrive as one byte, as a
//! seven-byte escape sequence, or split across two reads with a scheduler gap in the middle.
//! [`Parser`] therefore never blocks and never guesses: [`Parser::feed`] takes whatever bytes
//! arrived, emits every event that is unambiguously complete, and keeps the remainder for
//! next time.
//!
//! ```
//! use conui_input::{Event, KeyCode, Parser};
//!
//! let mut parser = Parser::new();
//! // An arrow key, arriving in two pieces.
//! parser.feed(b"\x1b[");
//! assert!(parser.next_event().is_none(), "an incomplete sequence yields nothing");
//! parser.feed(b"A");
//! assert_eq!(parser.next_event().unwrap().as_key().unwrap().code, KeyCode::Up);
//! ```

use std::collections::VecDeque;

use crate::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, Modifiers, MouseButton, MouseEvent, MouseKind,
};

/// Bytes that close a bracketed paste. The opening `CSI 200 ~` is recognised by the ordinary
/// CSI path, but the close has to be found by scanning, since paste content is arbitrary bytes.
const PASTE_END: &[u8] = b"\x1b[201~";

/// Guard against a runaway sequence consuming unbounded memory.
///
/// A malformed or hostile stream could otherwise open a paste or an OSC string and never
/// close it. Past this many buffered bytes the parser gives up on the sequence and resyncs.
const MAX_PENDING: usize = 1 << 20;

/// The outcome of attempting to parse one event from the front of the buffer.
enum Step {
    /// An event, and how many bytes it consumed.
    Emit(Event, usize),
    /// Well-formed but carrying nothing an app needs; discard these bytes.
    Skip(usize),
    /// A bracketed paste starts after this many bytes.
    BeginPaste(usize),
    /// Not enough bytes yet. Wait for more.
    Incomplete,
}

/// Decodes terminal input bytes into [`Event`]s.
#[derive(Debug, Default)]
pub struct Parser {
    /// Bytes received but not yet resolved into an event.
    pending: Vec<u8>,
    ready: VecDeque<Event>,
    /// Text accumulated so far inside a bracketed paste, if one is open.
    paste: Option<String>,
}

impl Parser {
    /// A parser with nothing buffered.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add bytes from the terminal and decode everything now complete.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        self.pump();
    }

    /// Take the next decoded event.
    pub fn next_event(&mut self) -> Option<Event> {
        self.ready.pop_front()
    }

    /// Drain every decoded event.
    pub fn drain(&mut self) -> impl Iterator<Item = Event> + '_ {
        self.ready.drain(..)
    }

    /// Whether bytes are buffered awaiting more input.
    pub fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    /// Resolve a buffered sequence that will never be completed.
    ///
    /// This exists for one genuine ambiguity: a lone `ESC` is both the Escape key and the
    /// first byte of every escape sequence, and nothing in the stream distinguishes them. The
    /// only signal is time — a real Escape keypress is followed by silence, while a sequence
    /// arrives in one burst. The event loop therefore calls this after a short idle period,
    /// which is why Escape is the one key with any perceptible latency.
    ///
    /// Returns the number of events this produced.
    pub fn flush_timeout(&mut self) -> usize {
        if self.pending.is_empty() {
            return 0;
        }
        let before = self.ready.len();
        // An open paste that has stopped arriving is delivered as-is rather than lost. The
        // tail `pump_paste` held back in case it was a split terminator is content after all,
        // since no terminator is coming.
        if let Some(mut text) = self.paste.take() {
            text.push_str(&String::from_utf8_lossy(&self.pending));
            self.pending.clear();
            self.ready.push_back(Event::Paste(text));
            return self.ready.len() - before;
        }
        if self.pending[0] == 0x1b {
            // Emit the Escape, then reconsider whatever followed it as fresh input.
            self.pending.remove(0);
            self.ready.push_back(Event::Key(KeyEvent::plain(KeyCode::Escape)));
            self.pump();
        } else {
            // A truncated UTF-8 sequence or other garbage; drop one byte and resync.
            self.pending.remove(0);
            self.pump();
        }
        self.ready.len() - before
    }

    fn pump(&mut self) {
        loop {
            if self.pending.len() > MAX_PENDING {
                // Resync rather than grow without bound.
                self.pending.clear();
                self.paste = None;
                return;
            }
            if self.paste.is_some() {
                if !self.pump_paste() {
                    return;
                }
                continue;
            }
            if self.pending.is_empty() {
                return;
            }
            match parse_one(&self.pending) {
                Step::Emit(event, consumed) => {
                    self.pending.drain(..consumed);
                    self.ready.push_back(event);
                }
                Step::Skip(consumed) => {
                    self.pending.drain(..consumed.max(1));
                }
                Step::BeginPaste(consumed) => {
                    self.pending.drain(..consumed);
                    self.paste = Some(String::new());
                }
                Step::Incomplete => return,
            }
        }
    }

    /// Consume paste content. Returns true when the paste completed.
    fn pump_paste(&mut self) -> bool {
        let Some(text) = self.paste.as_mut() else { return false };

        if let Some(at) = find(&self.pending, PASTE_END) {
            text.push_str(&String::from_utf8_lossy(&self.pending[..at]));
            self.pending.drain(..at + PASTE_END.len());
            let finished = self.paste.take().unwrap_or_default();
            self.ready.push_back(Event::Paste(finished));
            return true;
        }

        // The terminator may be split across reads, so hold back enough bytes that a partial
        // one is never mistaken for content. Everything before that is safe to absorb now,
        // which keeps memory flat for a large paste instead of buffering all of it.
        let keep = PASTE_END.len() - 1;
        if self.pending.len() > keep {
            let take = self.pending.len() - keep;
            // Only absorb up to a UTF-8 boundary; a split multi-byte character would
            // otherwise become two replacement characters.
            let take = floor_utf8_boundary(&self.pending, take);
            if take > 0 {
                text.push_str(&String::from_utf8_lossy(&self.pending[..take]));
                self.pending.drain(..take);
            }
        }
        false
    }
}

/// Largest index at or below `limit` that is not inside a UTF-8 multi-byte sequence.
///
/// Callers must keep at least one byte in reserve past `limit`, so that a sequence still
/// arriving is recognisable as one rather than looking like a complete tail.
fn floor_utf8_boundary(bytes: &[u8], limit: usize) -> usize {
    let mut index = limit.min(bytes.len());
    // A continuation byte at `index` means the split would land mid-character, so walk back
    // to the sequence's leading byte and cut in front of it instead.
    while index > 0 && index < bytes.len() && bytes[index] & 0b1100_0000 == 0b1000_0000 {
        index -= 1;
    }
    index
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

/// How many bytes the UTF-8 sequence starting with `leader` occupies.
fn utf8_length(leader: u8) -> usize {
    match leader {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        // A continuation byte or an invalid leader: treat as one byte so the parser resyncs.
        _ => 1,
    }
}

fn parse_one(bytes: &[u8]) -> Step {
    match bytes.first() {
        None => Step::Incomplete,
        Some(0x1b) => parse_escape(bytes),
        Some(_) => parse_text(bytes, Modifiers::NONE),
    }
}

fn parse_escape(bytes: &[u8]) -> Step {
    let Some(&second) = bytes.get(1) else { return Step::Incomplete };
    match second {
        b'[' => parse_csi(bytes),
        b'O' => parse_ss3(bytes),
        // DCS, OSC, APC and PM carry terminal replies, not user input. Skip the whole string
        // rather than let its payload be misread as keystrokes.
        b'P' | b']' | b'_' | b'^' => skip_string(bytes),
        // Alt+Escape.
        0x1b => Step::Emit(Event::Key(KeyEvent::new(KeyCode::Escape, Modifiers::ALT)), 2),
        // Anything else is the Alt modifier applied to the following key, which is how a
        // terminal in `metaSendsEscape` mode reports it.
        _ => match parse_text(&bytes[1..], Modifiers::ALT) {
            Step::Emit(event, consumed) => Step::Emit(event, consumed + 1),
            Step::Skip(consumed) => Step::Skip(consumed + 1),
            other => other,
        },
    }
}

/// Skip a string-terminated control sequence: OSC ends at `BEL` or `ST`, the rest at `ST`.
fn skip_string(bytes: &[u8]) -> Step {
    let mut index = 2;
    while index < bytes.len() {
        if bytes[index] == 0x07 {
            return Step::Skip(index + 1);
        }
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
            return Step::Skip(index + 2);
        }
        index += 1;
    }
    Step::Incomplete
}

/// `ESC O <final>`, the single-shift form used for F1-F4 and application-mode arrows.
fn parse_ss3(bytes: &[u8]) -> Step {
    let Some(&final_byte) = bytes.get(2) else { return Step::Incomplete };
    let code = match final_byte {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        b'M' => KeyCode::Enter,
        b' ' => KeyCode::Char(' '),
        _ => return Step::Skip(3),
    };
    Step::Emit(Event::Key(KeyEvent::plain(code)), 3)
}

/// Parameters of a CSI sequence: semicolon-separated groups of colon-separated numbers.
struct Params {
    groups: Vec<Vec<u16>>,
}

impl Params {
    fn parse(bytes: &[u8]) -> Self {
        let mut groups = Vec::new();
        if bytes.is_empty() {
            return Self { groups };
        }
        for group in bytes.split(|byte| *byte == b';') {
            let numbers = group
                .split(|byte| *byte == b':')
                .map(|digits| {
                    digits.iter().fold(0u16, |accumulator, digit| {
                        if digit.is_ascii_digit() {
                            accumulator.saturating_mul(10).saturating_add(u16::from(digit - b'0'))
                        } else {
                            accumulator
                        }
                    })
                })
                .collect();
            groups.push(numbers);
        }
        Self { groups }
    }

    /// The `index`-th parameter group's first number, or `default` when absent or empty.
    fn get(&self, index: usize, default: u16) -> u16 {
        match self.groups.get(index).and_then(|group| group.first()) {
            Some(0) | None => default,
            Some(value) => *value,
        }
    }

    /// A sub-parameter, as used by the extended keyboard protocol's event type.
    fn sub(&self, index: usize, sub: usize, default: u16) -> u16 {
        self.groups.get(index).and_then(|group| group.get(sub)).copied().unwrap_or(default)
    }

    fn len(&self) -> usize {
        self.groups.len()
    }

    /// Modifiers from the conventional second parameter.
    fn modifiers(&self) -> Modifiers {
        Modifiers::from_csi_param(self.get(1, 1))
    }
}

fn parse_csi(bytes: &[u8]) -> Step {
    let length = bytes.len();
    let mut index = 2;

    // An optional private-marker byte selects a sequence family, such as `<` for SGR mouse.
    let private = match bytes.get(index) {
        Some(byte @ (b'?' | b'<' | b'=' | b'>')) => {
            index += 1;
            Some(*byte)
        }
        _ => None,
    };

    // The legacy X10 mouse encoding puts three raw bytes *after* the final byte, so it has to
    // be recognised before the generic scan, which would treat those bytes as a new sequence.
    if private.is_none() && bytes.get(index) == Some(&b'M') {
        return parse_x10_mouse(bytes, index + 1);
    }

    let params_start = index;
    while index < length && (bytes[index].is_ascii_digit() || matches!(bytes[index], b';' | b':')) {
        index += 1;
    }
    let params_end = index;

    // Intermediate bytes, which none of the sequences we handle use but must still be skipped.
    while index < length && (0x20..=0x2f).contains(&bytes[index]) {
        index += 1;
    }

    let Some(&final_byte) = bytes.get(index) else { return Step::Incomplete };
    if !(0x40..=0x7e).contains(&final_byte) {
        // Not a valid terminator: the sequence is malformed, so resync past what we scanned.
        return Step::Skip(index + 1);
    }
    let consumed = index + 1;
    let params = Params::parse(&bytes[params_start..params_end]);

    if private == Some(b'<') {
        return parse_sgr_mouse(&params, final_byte, consumed);
    }
    if private.is_some() {
        // A reply to a mode query, not user input.
        return Step::Skip(consumed);
    }

    let key =
        |code: KeyCode| Step::Emit(Event::Key(KeyEvent::new(code, params.modifiers())), consumed);

    match final_byte {
        b'A' => key(KeyCode::Up),
        b'B' => key(KeyCode::Down),
        b'C' => key(KeyCode::Right),
        b'D' => key(KeyCode::Left),
        b'E' => key(KeyCode::KeypadBegin),
        b'H' => key(KeyCode::Home),
        b'F' => key(KeyCode::End),
        b'Z' => {
            Step::Emit(Event::Key(KeyEvent::new(KeyCode::BackTab, params.modifiers())), consumed)
        }
        b'P' => key(KeyCode::F(1)),
        b'Q' => key(KeyCode::F(2)),
        b'S' => key(KeyCode::F(4)),
        // `CSI row ; col R` is a cursor-position report, but `CSI 1 ; mods R` is a modified F3.
        // They are genuinely ambiguous when the cursor sits on row 1; a first parameter of 1
        // is read as the function key, since an app that issued no query expects no report.
        b'R' => {
            if params.len() >= 2 && params.get(0, 1) != 1 {
                Step::Emit(
                    Event::CursorPosition {
                        column: params.get(1, 1).saturating_sub(1),
                        row: params.get(0, 1).saturating_sub(1),
                    },
                    consumed,
                )
            } else {
                key(KeyCode::F(3))
            }
        }
        b'I' => Step::Emit(Event::FocusGained, consumed),
        b'O' => Step::Emit(Event::FocusLost, consumed),
        b'u' => parse_kitty_key(&params, consumed),
        b'~' => parse_tilde(&params, consumed),
        b'M' | b'm' => Step::Skip(consumed),
        _ => Step::Skip(consumed),
    }
}

/// `CSI <number> ~`, the VT220 family: navigation and higher function keys.
fn parse_tilde(params: &Params, consumed: usize) -> Step {
    let number = params.get(0, 0);
    if number == 200 {
        return Step::BeginPaste(consumed);
    }
    if number == 201 {
        // A stray close with no open paste; nothing to deliver.
        return Step::Skip(consumed);
    }
    let code = match number {
        1 | 7 => KeyCode::Home,
        2 => KeyCode::Insert,
        3 => KeyCode::Delete,
        4 | 8 => KeyCode::End,
        5 => KeyCode::PageUp,
        6 => KeyCode::PageDown,
        11..=15 => KeyCode::F((number - 10) as u8),
        // 16 is unassigned, so the numbering shifts by one from here.
        17..=21 => KeyCode::F((number - 11) as u8),
        // 22 is unassigned as well.
        23..=26 => KeyCode::F((number - 12) as u8),
        28 => KeyCode::F(15),
        29 => KeyCode::Menu,
        31..=34 => KeyCode::F((number - 14) as u8),
        _ => return Step::Skip(consumed),
    };
    Step::Emit(Event::Key(KeyEvent::new(code, params.modifiers())), consumed)
}

/// `CSI <codepoint> ; <modifiers> : <event type> u`, from the Kitty keyboard protocol.
///
/// Worth supporting because it resolves things the legacy encoding cannot express at all:
/// key releases, auto-repeat, `Ctrl` combined with a non-letter, and the Super modifier.
fn parse_kitty_key(params: &Params, consumed: usize) -> Step {
    let codepoint = u32::from(params.get(0, 0));
    let modifiers = params.modifiers();
    let kind = match params.sub(1, 1, 1) {
        2 => KeyEventKind::Repeat,
        3 => KeyEventKind::Release,
        _ => KeyEventKind::Press,
    };

    let code = match codepoint {
        // The protocol reuses C0 values for these, rather than functional codes.
        9 => KeyCode::Tab,
        13 => KeyCode::Enter,
        27 => KeyCode::Escape,
        127 => KeyCode::Backspace,
        // Functional keys live in a private-use block.
        57358 => KeyCode::Menu,
        57399..=57408 => KeyCode::Char((b'0' + (codepoint - 57399) as u8) as char),
        _ => match char::from_u32(codepoint) {
            Some(character) => KeyCode::Char(character),
            None => return Step::Skip(consumed),
        },
    };
    Step::Emit(Event::Key(KeyEvent { code, modifiers, kind }), consumed)
}

/// `CSI < Cb ; Cx ; Cy M|m`, the SGR mouse encoding.
///
/// Preferred over the original scheme because coordinates are decimal rather than offset
/// bytes, so it works past column 223, and release events report which button was released.
fn parse_sgr_mouse(params: &Params, final_byte: u8, consumed: usize) -> Step {
    let button_bits = params.get(0, 0);
    let column = params.get(1, 1).saturating_sub(1);
    let row = params.get(2, 1).saturating_sub(1);
    let pressed = final_byte == b'M';

    let kind = decode_mouse_kind(button_bits, pressed);
    let modifiers = decode_mouse_modifiers(button_bits);
    Step::Emit(Event::Mouse(MouseEvent { kind, column, row, modifiers }), consumed)
}

/// `CSI M Cb Cx Cy`, the original X10 encoding, where each field is its value plus 32.
fn parse_x10_mouse(bytes: &[u8], data_start: usize) -> Step {
    let end = data_start + 3;
    if bytes.len() < end {
        return Step::Incomplete;
    }
    let button_bits = u16::from(bytes[data_start].saturating_sub(32));
    let column = u16::from(bytes[data_start + 1].saturating_sub(32)).saturating_sub(1);
    let row = u16::from(bytes[data_start + 2].saturating_sub(32)).saturating_sub(1);

    // This encoding cannot say which button was released: a release is reported as button 3.
    let kind = if button_bits & 0b11 == 0b11 && button_bits & 0b0110_0000 == 0 {
        MouseKind::Up(MouseButton::Left)
    } else {
        decode_mouse_kind(button_bits, true)
    };
    Step::Emit(
        Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers: decode_mouse_modifiers(button_bits),
        }),
        end,
    )
}

fn decode_mouse_kind(bits: u16, pressed: bool) -> MouseKind {
    let button = match bits & 0b11 {
        0 => MouseButton::Left,
        1 => MouseButton::Middle,
        _ => MouseButton::Right,
    };
    // Bit 6 marks a wheel event, and the low bits then select an axis and direction.
    if bits & 0b0100_0000 != 0 {
        return match bits & 0b11 {
            0 => MouseKind::ScrollUp,
            1 => MouseKind::ScrollDown,
            2 => MouseKind::ScrollLeft,
            _ => MouseKind::ScrollRight,
        };
    }
    // Bit 5 marks motion. With all button bits set, no button is held.
    if bits & 0b0010_0000 != 0 {
        return if bits & 0b11 == 0b11 { MouseKind::Moved } else { MouseKind::Drag(button) };
    }
    if pressed { MouseKind::Down(button) } else { MouseKind::Up(button) }
}

fn decode_mouse_modifiers(bits: u16) -> Modifiers {
    let mut modifiers = Modifiers::NONE;
    if bits & 0b0000_0100 != 0 {
        modifiers |= Modifiers::SHIFT;
    }
    if bits & 0b0000_1000 != 0 {
        modifiers |= Modifiers::ALT;
    }
    if bits & 0b0001_0000 != 0 {
        modifiers |= Modifiers::CTRL;
    }
    modifiers
}

/// A literal character or a C0 control byte, with `extra` modifiers folded in.
fn parse_text(bytes: &[u8], extra: Modifiers) -> Step {
    let Some(&first) = bytes.first() else { return Step::Incomplete };

    if first < 0x80 {
        let (code, modifiers) = decode_ascii(first);
        return Step::Emit(Event::Key(KeyEvent::new(code, modifiers.union(extra))), 1);
    }

    let needed = utf8_length(first);
    if needed == 1 {
        // A stray continuation byte. Drop it and resync rather than emit a wrong character.
        return Step::Skip(1);
    }
    if bytes.len() < needed {
        return Step::Incomplete;
    }
    match std::str::from_utf8(&bytes[..needed]).ok().and_then(|text| text.chars().next()) {
        Some(character) => {
            Step::Emit(Event::Key(KeyEvent::new(KeyCode::Char(character), extra)), needed)
        }
        None => Step::Skip(1),
    }
}

/// Map a 7-bit byte to a key, recovering the `Ctrl` modifier the C0 range encodes.
fn decode_ascii(byte: u8) -> (KeyCode, Modifiers) {
    match byte {
        // Ctrl+Space and Ctrl+@ both produce NUL.
        0x00 => (KeyCode::Char(' '), Modifiers::CTRL),
        0x08 => (KeyCode::Backspace, Modifiers::CTRL),
        0x09 => (KeyCode::Tab, Modifiers::NONE),
        // Enter sends CR in raw mode; LF is Ctrl+J, which users expect to act as Enter.
        0x0a | 0x0d => (KeyCode::Enter, Modifiers::NONE),
        0x1b => (KeyCode::Escape, Modifiers::NONE),
        // The rest of the C0 range is Ctrl plus a letter, offset from `a`.
        0x01..=0x1a => (KeyCode::Char((byte - 1 + b'a') as char), Modifiers::CTRL),
        // Ctrl with the four punctuation keys that follow `z`.
        0x1c => (KeyCode::Char('\\'), Modifiers::CTRL),
        0x1d => (KeyCode::Char(']'), Modifiers::CTRL),
        0x1e => (KeyCode::Char('^'), Modifiers::CTRL),
        0x1f => (KeyCode::Char('_'), Modifiers::CTRL),
        // DEL is what the Backspace key actually sends on a modern terminal.
        0x7f => (KeyCode::Backspace, Modifiers::NONE),
        _ => (KeyCode::Char(byte as char), Modifiers::NONE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a complete byte string and return every event it produced.
    fn events(bytes: &[u8]) -> Vec<Event> {
        let mut parser = Parser::new();
        parser.feed(bytes);
        parser.drain().collect()
    }

    fn one(bytes: &[u8]) -> Event {
        let parsed = events(bytes);
        assert_eq!(parsed.len(), 1, "expected exactly one event from {bytes:?}, got {parsed:?}");
        parsed.into_iter().next().unwrap()
    }

    fn key(bytes: &[u8]) -> KeyEvent {
        match one(bytes) {
            Event::Key(key) => key,
            other => panic!("expected a key event, got {other:?}"),
        }
    }

    // ---- Plain text ---------------------------------------------------------------------

    #[test]
    fn ascii_letters_arrive_as_characters() {
        assert_eq!(key(b"a"), KeyEvent::plain(KeyCode::Char('a')));
        assert_eq!(key(b"Z"), KeyEvent::plain(KeyCode::Char('Z')));
        assert_eq!(key(b" "), KeyEvent::plain(KeyCode::Char(' ')));
    }

    #[test]
    fn a_burst_of_typing_yields_one_event_per_character() {
        let parsed = events(b"hello");
        assert_eq!(parsed.len(), 5);
        assert_eq!(parsed[4].as_key().unwrap().code, KeyCode::Char('o'));
    }

    #[test]
    fn multibyte_characters_decode_to_one_key() {
        assert_eq!(key("é".as_bytes()), KeyEvent::plain(KeyCode::Char('é')));
        assert_eq!(key("→".as_bytes()), KeyEvent::plain(KeyCode::Char('→')));
        assert_eq!(key("🦀".as_bytes()), KeyEvent::plain(KeyCode::Char('🦀')));
    }

    #[test]
    fn a_multibyte_character_split_across_reads_is_held_then_completed() {
        let mut parser = Parser::new();
        let bytes = "🦀".as_bytes();
        parser.feed(&bytes[..2]);
        assert!(parser.next_event().is_none(), "a partial character must not be emitted");
        assert!(parser.has_pending());
        parser.feed(&bytes[2..]);
        assert_eq!(parser.next_event().unwrap().as_key().unwrap().code, KeyCode::Char('🦀'));
    }

    #[test]
    fn a_stray_continuation_byte_is_dropped_without_stalling() {
        let parsed = events(&[0x80, b'a']);
        assert_eq!(parsed.len(), 1, "the invalid byte is skipped, the valid one survives");
        assert_eq!(parsed[0].as_key().unwrap().code, KeyCode::Char('a'));
    }

    // ---- Control bytes ------------------------------------------------------------------

    #[test]
    fn control_bytes_recover_the_ctrl_modifier() {
        assert_eq!(key(&[0x03]), KeyEvent::new(KeyCode::Char('c'), Modifiers::CTRL));
        assert_eq!(key(&[0x01]), KeyEvent::new(KeyCode::Char('a'), Modifiers::CTRL));
        assert_eq!(key(&[0x1a]), KeyEvent::new(KeyCode::Char('z'), Modifiers::CTRL));
        assert_eq!(key(&[0x00]), KeyEvent::new(KeyCode::Char(' '), Modifiers::CTRL));
    }

    #[test]
    fn the_named_control_keys_are_not_reported_as_ctrl_letters() {
        // Tab is Ctrl+I and Enter is Ctrl+M at the byte level, but users mean the named key.
        assert_eq!(key(&[0x09]), KeyEvent::plain(KeyCode::Tab));
        assert_eq!(key(&[0x0d]), KeyEvent::plain(KeyCode::Enter));
        assert_eq!(key(&[0x0a]), KeyEvent::plain(KeyCode::Enter));
        assert_eq!(key(&[0x7f]), KeyEvent::plain(KeyCode::Backspace));
    }

    // ---- Arrows and navigation ----------------------------------------------------------

    #[test]
    fn csi_arrows_decode() {
        assert_eq!(key(b"\x1b[A"), KeyEvent::plain(KeyCode::Up));
        assert_eq!(key(b"\x1b[B"), KeyEvent::plain(KeyCode::Down));
        assert_eq!(key(b"\x1b[C"), KeyEvent::plain(KeyCode::Right));
        assert_eq!(key(b"\x1b[D"), KeyEvent::plain(KeyCode::Left));
    }

    #[test]
    fn ss3_arrows_decode_the_same_as_csi() {
        // Application cursor mode uses SS3; the app should not have to care which it got.
        assert_eq!(key(b"\x1bOA"), KeyEvent::plain(KeyCode::Up));
        assert_eq!(key(b"\x1bOD"), KeyEvent::plain(KeyCode::Left));
    }

    #[test]
    fn modified_arrows_carry_their_modifiers() {
        assert_eq!(key(b"\x1b[1;5A"), KeyEvent::new(KeyCode::Up, Modifiers::CTRL));
        assert_eq!(key(b"\x1b[1;2C"), KeyEvent::new(KeyCode::Right, Modifiers::SHIFT));
        assert_eq!(
            key(b"\x1b[1;6D"),
            KeyEvent::new(KeyCode::Left, Modifiers::SHIFT | Modifiers::CTRL)
        );
    }

    #[test]
    fn navigation_keys_decode_from_the_tilde_family() {
        assert_eq!(key(b"\x1b[2~"), KeyEvent::plain(KeyCode::Insert));
        assert_eq!(key(b"\x1b[3~"), KeyEvent::plain(KeyCode::Delete));
        assert_eq!(key(b"\x1b[5~"), KeyEvent::plain(KeyCode::PageUp));
        assert_eq!(key(b"\x1b[6~"), KeyEvent::plain(KeyCode::PageDown));
        assert_eq!(key(b"\x1b[1~"), KeyEvent::plain(KeyCode::Home));
        assert_eq!(key(b"\x1b[4~"), KeyEvent::plain(KeyCode::End));
    }

    #[test]
    fn home_and_end_decode_from_both_encodings() {
        assert_eq!(key(b"\x1b[H"), KeyEvent::plain(KeyCode::Home));
        assert_eq!(key(b"\x1b[F"), KeyEvent::plain(KeyCode::End));
        assert_eq!(key(b"\x1bOH"), KeyEvent::plain(KeyCode::Home));
    }

    #[test]
    fn shift_tab_is_its_own_key() {
        assert_eq!(key(b"\x1b[Z"), KeyEvent::plain(KeyCode::BackTab));
    }

    #[test]
    fn function_keys_decode_across_both_numbering_schemes() {
        assert_eq!(key(b"\x1bOP"), KeyEvent::plain(KeyCode::F(1)));
        assert_eq!(key(b"\x1bOS"), KeyEvent::plain(KeyCode::F(4)));
        assert_eq!(key(b"\x1b[15~"), KeyEvent::plain(KeyCode::F(5)));
        assert_eq!(key(b"\x1b[17~"), KeyEvent::plain(KeyCode::F(6)));
        assert_eq!(key(b"\x1b[21~"), KeyEvent::plain(KeyCode::F(10)));
        assert_eq!(key(b"\x1b[24~"), KeyEvent::plain(KeyCode::F(12)));
    }

    // ---- Escape and Alt -----------------------------------------------------------------

    #[test]
    fn alt_plus_a_letter_is_esc_prefixed() {
        assert_eq!(key(b"\x1ba"), KeyEvent::new(KeyCode::Char('a'), Modifiers::ALT));
    }

    #[test]
    fn a_lone_escape_waits_for_disambiguation_then_resolves_on_timeout() {
        let mut parser = Parser::new();
        parser.feed(b"\x1b");
        assert!(parser.next_event().is_none(), "ESC alone is still ambiguous");
        assert_eq!(parser.flush_timeout(), 1);
        assert_eq!(parser.next_event().unwrap().as_key().unwrap().code, KeyCode::Escape);
    }

    #[test]
    fn an_escape_sequence_arriving_in_pieces_is_not_mistaken_for_escape() {
        let mut parser = Parser::new();
        for chunk in [&b"\x1b"[..], &b"["[..], &b"1;5"[..], &b"A"[..]] {
            parser.feed(chunk);
        }
        let parsed: Vec<_> = parser.drain().collect();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].as_key().unwrap(), &KeyEvent::new(KeyCode::Up, Modifiers::CTRL));
    }

    #[test]
    fn flushing_an_incomplete_sequence_recovers_the_escape_and_the_rest() {
        let mut parser = Parser::new();
        parser.feed(b"\x1bO");
        parser.flush_timeout();
        let parsed: Vec<_> = parser.drain().collect();
        // Escape, then the stranded `O` as an ordinary character.
        assert_eq!(parsed[0].as_key().unwrap().code, KeyCode::Escape);
        assert_eq!(parsed[1].as_key().unwrap().code, KeyCode::Char('O'));
    }

    #[test]
    fn alt_escape_decodes_as_one_event() {
        assert_eq!(key(b"\x1b\x1b"), KeyEvent::new(KeyCode::Escape, Modifiers::ALT));
    }

    // ---- Extended keyboard protocol -----------------------------------------------------

    #[test]
    fn kitty_sequences_decode_keys_the_legacy_encoding_cannot() {
        assert_eq!(key(b"\x1b[99;5u"), KeyEvent::new(KeyCode::Char('c'), Modifiers::CTRL));
        // Ctrl with a digit has no C0 representation at all.
        assert_eq!(key(b"\x1b[49;5u"), KeyEvent::new(KeyCode::Char('1'), Modifiers::CTRL));
        assert_eq!(key(b"\x1b[97;9u"), KeyEvent::new(KeyCode::Char('a'), Modifiers::SUPER));
    }

    #[test]
    fn kitty_reports_repeat_and_release() {
        let repeat = key(b"\x1b[97;1:2u");
        assert_eq!(repeat.kind, KeyEventKind::Repeat);
        let release = key(b"\x1b[97;1:3u");
        assert_eq!(release.kind, KeyEventKind::Release);
        assert_eq!(release.code, KeyCode::Char('a'));
        let press = key(b"\x1b[97;1:1u");
        assert_eq!(press.kind, KeyEventKind::Press);
    }

    #[test]
    fn kitty_maps_the_named_keys_onto_their_c0_codes() {
        assert_eq!(key(b"\x1b[27u").code, KeyCode::Escape);
        assert_eq!(key(b"\x1b[13u").code, KeyCode::Enter);
        assert_eq!(key(b"\x1b[9u").code, KeyCode::Tab);
        assert_eq!(key(b"\x1b[127u").code, KeyCode::Backspace);
    }

    // ---- Mouse ---------------------------------------------------------------------------

    fn mouse(bytes: &[u8]) -> MouseEvent {
        match one(bytes) {
            Event::Mouse(event) => event,
            other => panic!("expected a mouse event, got {other:?}"),
        }
    }

    #[test]
    fn sgr_mouse_press_and_release_report_the_same_button() {
        let down = mouse(b"\x1b[<0;10;5M");
        assert_eq!(down.kind, MouseKind::Down(MouseButton::Left));
        // Coordinates are reported 1-based and converted to conui's 0-based cell space.
        assert_eq!((down.column, down.row), (9, 4));
        let up = mouse(b"\x1b[<0;10;5m");
        assert_eq!(up.kind, MouseKind::Up(MouseButton::Left));
    }

    #[test]
    fn sgr_mouse_decodes_every_button() {
        assert_eq!(mouse(b"\x1b[<0;1;1M").kind, MouseKind::Down(MouseButton::Left));
        assert_eq!(mouse(b"\x1b[<1;1;1M").kind, MouseKind::Down(MouseButton::Middle));
        assert_eq!(mouse(b"\x1b[<2;1;1M").kind, MouseKind::Down(MouseButton::Right));
    }

    #[test]
    fn sgr_mouse_decodes_the_wheel() {
        assert_eq!(mouse(b"\x1b[<64;1;1M").kind, MouseKind::ScrollUp);
        assert_eq!(mouse(b"\x1b[<65;1;1M").kind, MouseKind::ScrollDown);
        assert_eq!(mouse(b"\x1b[<66;1;1M").kind, MouseKind::ScrollLeft);
        assert_eq!(mouse(b"\x1b[<67;1;1M").kind, MouseKind::ScrollRight);
    }

    #[test]
    fn sgr_mouse_distinguishes_drag_from_bare_motion() {
        assert_eq!(mouse(b"\x1b[<32;4;4M").kind, MouseKind::Drag(MouseButton::Left));
        assert_eq!(mouse(b"\x1b[<35;4;4M").kind, MouseKind::Moved);
    }

    #[test]
    fn sgr_mouse_carries_modifiers() {
        assert_eq!(mouse(b"\x1b[<16;1;1M").modifiers, Modifiers::CTRL);
        assert_eq!(mouse(b"\x1b[<4;1;1M").modifiers, Modifiers::SHIFT);
    }

    #[test]
    fn sgr_mouse_handles_coordinates_beyond_the_legacy_limit() {
        // The X10 encoding tops out at 223; SGR is decimal and has no such ceiling.
        let event = mouse(b"\x1b[<0;400;300M");
        assert_eq!((event.column, event.row), (399, 299));
    }

    #[test]
    fn x10_mouse_is_still_understood() {
        // `CSI M` then three offset-by-32 bytes: button 0 at column 1, row 1.
        let event = mouse(&[0x1b, b'[', b'M', 32, 33, 33]);
        assert_eq!(event.kind, MouseKind::Down(MouseButton::Left));
        assert_eq!((event.column, event.row), (0, 0));
    }

    #[test]
    fn a_truncated_x10_mouse_sequence_waits_for_the_rest() {
        let mut parser = Parser::new();
        parser.feed(&[0x1b, b'[', b'M', 32, 33]);
        assert!(parser.next_event().is_none());
        parser.feed(&[33]);
        assert!(matches!(parser.next_event(), Some(Event::Mouse(_))));
    }

    // ---- Focus ---------------------------------------------------------------------------

    #[test]
    fn focus_changes_decode() {
        assert_eq!(one(b"\x1b[I"), Event::FocusGained);
        assert_eq!(one(b"\x1b[O"), Event::FocusLost);
    }

    // ---- Bracketed paste -----------------------------------------------------------------

    #[test]
    fn a_paste_arrives_as_a_single_event() {
        assert_eq!(one(b"\x1b[200~hello world\x1b[201~"), Event::Paste("hello world".into()));
    }

    #[test]
    fn a_paste_preserves_newlines_rather_than_submitting() {
        // This is the whole reason bracketed paste exists: a pasted newline is text.
        assert_eq!(
            one(b"\x1b[200~line one\nline two\x1b[201~"),
            Event::Paste("line one\nline two".into())
        );
    }

    #[test]
    fn a_paste_split_across_reads_is_reassembled() {
        let mut parser = Parser::new();
        parser.feed(b"\x1b[200~abc");
        assert!(parser.next_event().is_none(), "an open paste emits nothing yet");
        parser.feed(b"def");
        assert!(parser.next_event().is_none());
        parser.feed(b"\x1b[201~");
        assert_eq!(parser.next_event(), Some(Event::Paste("abcdef".into())));
    }

    #[test]
    fn a_paste_terminator_split_across_reads_is_not_treated_as_content() {
        let mut parser = Parser::new();
        parser.feed(b"\x1b[200~abc\x1b[20");
        assert!(parser.next_event().is_none());
        parser.feed(b"1~");
        assert_eq!(parser.next_event(), Some(Event::Paste("abc".into())));
    }

    #[test]
    fn a_paste_containing_multibyte_text_split_mid_character_is_intact() {
        let mut parser = Parser::new();
        let text = "héllo 🦀 wörld and then some more text to exceed the holdback";
        let bytes = text.as_bytes();
        parser.feed(b"\x1b[200~");
        // Feed one byte at a time, which splits every multi-byte character.
        for byte in bytes {
            parser.feed(&[*byte]);
        }
        parser.feed(b"\x1b[201~");
        assert_eq!(parser.next_event(), Some(Event::Paste(text.into())));
    }

    #[test]
    fn keys_before_and_after_a_paste_survive() {
        let parsed = events(b"a\x1b[200~pasted\x1b[201~b");
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].as_key().unwrap().code, KeyCode::Char('a'));
        assert_eq!(parsed[1], Event::Paste("pasted".into()));
        assert_eq!(parsed[2].as_key().unwrap().code, KeyCode::Char('b'));
    }

    #[test]
    fn an_unterminated_paste_is_delivered_on_timeout_rather_than_lost() {
        let mut parser = Parser::new();
        parser.feed(b"\x1b[200~half a paste");
        assert!(parser.next_event().is_none());
        parser.flush_timeout();
        assert_eq!(parser.next_event(), Some(Event::Paste("half a paste".into())));
    }

    // ---- Robustness ----------------------------------------------------------------------

    #[test]
    fn terminal_replies_are_skipped_rather_than_read_as_keys() {
        // An OSC reply to a colour query must not turn into a dozen spurious keystrokes.
        let parsed = events(b"\x1b]11;rgb:0909/0f0f/1313\x07a");
        assert_eq!(parsed.len(), 1, "got {parsed:?}");
        assert_eq!(parsed[0].as_key().unwrap().code, KeyCode::Char('a'));
    }

    #[test]
    fn a_dcs_string_terminated_by_st_is_skipped() {
        let parsed = events(b"\x1bP1$r0m\x1b\\x");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].as_key().unwrap().code, KeyCode::Char('x'));
    }

    #[test]
    fn a_cursor_position_report_is_its_own_event() {
        assert_eq!(one(b"\x1b[24;80R"), Event::CursorPosition { column: 79, row: 23 });
    }

    #[test]
    fn a_malformed_sequence_does_not_stall_the_parser() {
        let parsed = events(b"\x1b[999999999999Za");
        // The overlong parameter saturates and the sequence still terminates cleanly.
        assert!(!parsed.is_empty());
        assert_eq!(parsed.last().unwrap().as_key().unwrap().code, KeyCode::Char('a'));
    }

    #[test]
    fn an_unknown_final_byte_is_skipped_without_losing_what_follows() {
        let parsed = events(b"\x1b[1;2\x01a");
        // `\x01` is not a valid CSI terminator, so the sequence is dropped and parsing resumes.
        assert_eq!(parsed.last().unwrap().as_key().unwrap().code, KeyCode::Char('a'));
    }

    #[test]
    fn a_runaway_stream_does_not_grow_the_buffer_without_bound() {
        let mut parser = Parser::new();
        // An OSC that never terminates.
        parser.feed(b"\x1b]");
        for _ in 0..(MAX_PENDING / 1024 + 2) {
            parser.feed(&[b'x'; 1024]);
        }
        assert!(parser.pending.len() <= MAX_PENDING, "buffer grew to {}", parser.pending.len());
    }

    #[test]
    fn realistic_mixed_input_decodes_in_order() {
        let parsed = events(b"q\x1b[A \x1b[<0;3;7M\x1b[200~x\x1b[201~\x1b[3~");
        let kinds: Vec<String> = parsed.iter().map(|event| format!("{event:?}")).collect();
        assert_eq!(parsed.len(), 6, "got {kinds:?}");
        assert_eq!(parsed[0].as_key().unwrap().code, KeyCode::Char('q'));
        assert_eq!(parsed[1].as_key().unwrap().code, KeyCode::Up);
        assert_eq!(parsed[2].as_key().unwrap().code, KeyCode::Char(' '));
        assert!(matches!(parsed[3], Event::Mouse(_)));
        assert_eq!(parsed[4], Event::Paste("x".into()));
        assert_eq!(parsed[5].as_key().unwrap().code, KeyCode::Delete);
    }
}
