//! Terminal input, decoded.
//!
//! The terminal hands an application a byte stream, not events. `Up` might be three bytes,
//! `Ctrl+Left` six, a pasted paragraph a few thousand, and any of them can be split across
//! reads. This crate turns those bytes into [`Event`] values, incrementally and without
//! blocking: feed it whatever arrived, take out whatever is now unambiguous.
//!
//! ```
//! use conui_input::{Event, KeyCode, Parser};
//!
//! let mut parser = Parser::new();
//! parser.feed(b"\x1b[1;5C");
//! let event = parser.next_event().unwrap();
//! let key = event.as_key().unwrap();
//! assert_eq!(key.code, KeyCode::Right);
//! assert!(key.modifiers.contains(conui_input::Modifiers::CTRL));
//! ```
//!
//! # The Escape key
//!
//! One ambiguity cannot be resolved from the bytes alone: `ESC` is both the Escape key and the
//! first byte of every escape sequence. The parser holds a trailing `ESC` rather than guessing,
//! and the event loop calls [`Parser::flush_timeout`] once input has been idle for
//! [`ESCAPE_TIMEOUT`] to settle it. That is the whole reason Escape feels a few milliseconds
//! slower than every other key in every terminal application ever written.
//!
//! # Reading the bytes
//!
//! Getting bytes off the terminal is `conui-term`'s job, not this crate's. The pairing is:
//!
//! ```ignore
//! let mut buffer = [0u8; 1024];
//! if terminal.wait_readable(Some(ESCAPE_TIMEOUT))? {
//!     let count = terminal.read_input(&mut buffer)?;
//!     parser.feed(&buffer[..count]);
//! } else {
//!     parser.flush_timeout();
//! }
//! for event in parser.drain() { /* ... */ }
//! ```

mod event;
mod parser;

pub use event::{
    Event, KeyCode, KeyEvent, KeyEventKind, Modifiers, MouseButton, MouseEvent, MouseKind,
};
pub use parser::Parser;

use std::time::Duration;

/// How long to wait for a byte after `ESC` before calling it the Escape key.
///
/// The bytes of a real escape sequence are written by the terminal in one go, so they arrive
/// together; a human pressing Escape produces silence. Long enough to survive a scheduling
/// hiccup or a slow ssh link, short enough that Escape does not feel broken.
pub const ESCAPE_TIMEOUT: Duration = Duration::from_millis(25);
