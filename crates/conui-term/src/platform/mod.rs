//! The one platform-dependent seam in conui.
//!
//! Everything above this module is portable. Each backend provides the same six operations —
//! is-it-a-tty, enter raw mode, restore, size, wait for input, read input — and nothing else
//! in the kit needs a `cfg`.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;

#[cfg(not(any(unix, windows)))]
compile_error!(
    "conui-term supports unix and windows targets. \
     Rendering to a string with conui_cell::Buffer needs no platform support and works anywhere."
);
