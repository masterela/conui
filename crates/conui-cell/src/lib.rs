//! The cell layer: geometry, color, style, and the grid that a frame is rendered into.
//!
//! This crate knows nothing about terminals. It is a pure, testable model of "what should be
//! on screen", which is why every test in it runs without a TTY. [`conui-term`] turns a
//! [`Buffer`] into bytes; [`conui`] provides the drawing and layout API on top.
//!
//! The two ideas worth knowing:
//!
//! - A cell holds a **grapheme cluster**, not a `char`, and knows its display **width**.
//!   Emoji and CJK take two columns, and getting that wrong shears every subsequent column.
//! - A frame is **diffed**, not redrawn. [`Buffer::diff`] yields only the cells that changed,
//!   so a full-screen 60 fps app sends a few hundred bytes per frame instead of tens of
//!   thousands, and never blanks the screen in between.
//!
//! [`conui-term`]: https://docs.rs/conui-term
//! [`conui`]: https://docs.rs/conui

// An undocumented public item is a promise someone has to read the source to understand, and
// this crate is the one every other crate's types come from.
#![warn(missing_docs)]

mod buffer;
mod cell;
mod color;
mod geom;
mod style;

pub use buffer::{Buffer, Patch};
pub use cell::{Cell, Symbol};
pub use color::{Color, ColorDepth, rgb_to_ansi16, rgb_to_xterm256, xterm256_to_rgb};
pub use geom::{Padding, Pos, Rect};
pub use style::{Attrs, ResolvedStyle, Style};
