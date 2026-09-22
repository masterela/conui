//! A devkit for console applications with a proper user interface.
//!
//! conui is two APIs over one renderer, and you are meant to use both.
//!
//! The lower one is a [`Canvas`]: a clipped view into a grid of cells with an immediate-mode
//! drawing API — put this text here, draw a bar of this length, paint these cells this colour.
//! It is what every widget in the box is written against, it is always reachable, and it is the
//! right answer whenever a screen contains something nobody's widget set anticipated.
//!
//! The higher one is a tree of [`View`]s: values you rebuild every frame and hand to a
//! [`Frame`], which resolves a layout and gives each one a region to draw into. Views nest,
//! clip their children, and compose without coordination — a child cannot corrupt a sibling
//! even if its arithmetic is wrong.
//!
//! ```no_run
//! use conui::view::{Column, Row, ViewExt};
//! use conui::widget::{Gauge, Hints, Panel, Stat, Text};
//! use conui::{App, Role};
//!
//! # fn main() -> std::io::Result<()> {
//! let mut app = App::new()?;
//! while app.is_running() {
//!     for event in app.poll()? {
//!         if let Some(key) = event.as_key() {
//!             if key.is_char('q') {
//!                 app.quit();
//!             }
//!         }
//!     }
//!     app.draw(|frame| {
//!         let screen = Column::new()
//!             .child(Text::new("DASHBOARD").accent())
//!             .child(
//!                 Row::new()
//!                     .gap(2)
//!                     .child(Stat::new("SCORE", 42))
//!                     .child(Gauge::new(0.6).label("LOAD").flex(1))
//!                     .length(4),
//!             )
//!             .child(Hints::new().key("Q", "quit"));
//!         frame.render_full(&screen);
//!     })?;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! # The crates underneath
//!
//! conui is the top of a stack that is useful on its own, so that nothing here is a dead end:
//!
//! - [`conui_cell`] — cells, styles, colours, and the buffer that diffs one frame against the
//!   last. No I/O; a pure data structure you can render in a test.
//! - [`conui_term`] — the terminal itself: raw mode, the alternate screen, capability detection,
//!   and the escape-sequence writer. Hand-written against `rustix` and `windows-sys`, with no
//!   TUI dependency anywhere in the tree.
//! - [`conui_input`] — an incremental parser from bytes to [`Event`](conui_input::Event)s.
//!
//! Pick any level. A program that only wants "print a table with colour" can use `conui_cell`
//! and its own `print!`; one that wants a full-screen app uses [`App`].

pub mod app;
pub mod canvas;
pub mod frame;
pub mod layout;
pub mod state;
pub mod theme;
pub mod typography;
pub mod view;
pub mod widget;

pub use app::{App, Config};
pub use canvas::{Canvas, text_width};
pub use frame::Frame;
pub use layout::{Constraint, Direction, Layout, centered};
pub use state::{Dropdown, Editor, Focus, Hits, Selection, Viewport};
pub use theme::{Role, Theme};
pub use typography::BarStyle;
pub use view::{View, ViewExt};

// The layers below are re-exported wholesale: a conui app inevitably touches a `Rect`, a
// `Color` and a `KeyCode`, and making people add three more dependencies to their manifest to
// name the types their own callbacks receive is a papercut with no upside.
pub use conui_cell::{self, Buffer, Cell, Color, Padding, Pos, Rect, Style, Symbol};
pub use conui_input::{
    self, Event, KeyCode, KeyEvent, Modifiers, MouseButton, MouseEvent, MouseKind,
};
pub use conui_term::{self, Terminal};
