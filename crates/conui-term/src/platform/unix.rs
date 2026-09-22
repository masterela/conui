//! macOS, Linux and the BSDs, via `rustix`.
//!
//! No C is compiled and no `unsafe` is written here beyond the one `select` call that rustix
//! cannot make safe, because `select`'s interface is inherently unchecked.

use std::io;
use std::os::fd::BorrowedFd;
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec};
use rustix::termios::{self, OptionalActions, Termios};

/// The terminal settings to restore on exit.
pub type SavedMode = Termios;

fn stdin() -> BorrowedFd<'static> {
    rustix::stdio::stdin()
}

fn stdout() -> BorrowedFd<'static> {
    rustix::stdio::stdout()
}

pub fn is_input_tty() -> bool {
    termios::isatty(stdin())
}

pub fn is_output_tty() -> bool {
    termios::isatty(stdout())
}

/// Put the terminal into raw mode, returning the settings that were replaced.
///
/// Raw mode is what lets a TUI see individual keypresses: the kernel stops buffering until
/// newline, stops echoing, and stops turning Ctrl-C into a signal, so the app decides what
/// those mean. The caller is responsible for restoring the returned value.
pub fn enter_raw_mode() -> io::Result<SavedMode> {
    let original = termios::tcgetattr(stdin())?;
    let mut raw = original.clone();
    raw.make_raw();
    // `TCSADRAIN` waits for pending output to drain first, so anything already written
    // reaches the screen under the old settings instead of being reinterpreted.
    termios::tcsetattr(stdin(), OptionalActions::Drain, &raw)?;
    Ok(original)
}

pub fn restore_mode(saved: &SavedMode) -> io::Result<()> {
    termios::tcsetattr(stdin(), OptionalActions::Drain, saved)?;
    Ok(())
}

/// The terminal size in character cells.
///
/// Queried from the tty rather than cached, because a resize arrives as a `SIGWINCH` that a
/// library has no business installing a handler for. An `ioctl` per frame is cheap enough
/// that polling is the simpler and more robust design.
pub fn window_size() -> io::Result<(u16, u16)> {
    // Prefer stdout: output is what we are sizing, and stdin may be a pipe.
    let fd = if is_output_tty() { stdout() } else { stdin() };
    let size = termios::tcgetwinsize(fd)?;
    if size.ws_col == 0 || size.ws_row == 0 {
        // Some environments report zeroes before the pty is fully set up. A plausible
        // default beats propagating a zero-sized screen into the layout engine.
        return Ok((80, 24));
    }
    Ok((size.ws_col, size.ws_row))
}

/// Block until stdin has bytes to read, or `timeout` elapses. `None` waits indefinitely.
///
/// Returns `true` when input is ready.
pub fn wait_readable(timeout: Option<Duration>) -> io::Result<bool> {
    match poll_readable(timeout) {
        // `poll` is documented as unreliable on macOS for some character devices, where it
        // reports the descriptor as invalid rather than failing outright. `select` works on
        // those, so fall back instead of surfacing an error the caller cannot act on.
        Err(PollUnsupported) => select_readable(timeout),
        Ok(ready) => ready,
    }
}

/// Marker for "this descriptor cannot be polled", distinct from a real I/O failure.
struct PollUnsupported;

fn poll_readable(timeout: Option<Duration>) -> Result<io::Result<bool>, PollUnsupported> {
    let fd = stdin();
    let mut fds = [PollFd::new(&fd, PollFlags::IN)];
    let spec = timeout.map(duration_to_timespec);

    loop {
        match rustix::event::poll(&mut fds, spec.as_ref()) {
            Ok(0) => return Ok(Ok(false)),
            Ok(_) => {
                let revents = fds[0].revents();
                if revents.contains(PollFlags::NVAL) {
                    return Err(PollUnsupported);
                }
                // HUP and ERR both mean "read will not block"; the read itself reports why.
                return Ok(Ok(revents.intersects(PollFlags::IN | PollFlags::HUP | PollFlags::ERR)));
            }
            // A signal interrupted the wait. Retrying is correct: the caller asked to wait
            // for input, not to be told that a signal happened.
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Ok(Err(error.into())),
        }
    }
}

fn select_readable(timeout: Option<Duration>) -> io::Result<bool> {
    use rustix::event::{FdSetElement, fd_set_insert, fd_set_num_elements};
    use std::os::fd::AsRawFd;

    let raw = stdin().as_raw_fd();
    let nfds = raw + 1;
    let mut read_set = vec![FdSetElement::default(); fd_set_num_elements(1, nfds)];
    let spec = timeout.map(duration_to_timespec);

    loop {
        fd_set_insert(&mut read_set, raw);
        // SAFETY: `read_set` is sized by `fd_set_num_elements` for `nfds`, exactly as
        // `select` requires, and the write/except sets are absent.
        let result =
            unsafe { rustix::event::select(nfds, Some(&mut read_set), None, None, spec.as_ref()) };
        match result {
            Ok(count) => return Ok(count > 0),
            Err(rustix::io::Errno::INTR) => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

fn duration_to_timespec(duration: Duration) -> Timespec {
    Timespec {
        tv_sec: duration.as_secs().min(i64::MAX as u64) as _,
        tv_nsec: duration.subsec_nanos() as _,
    }
}

/// Read whatever is currently available on stdin.
///
/// Only call after [`wait_readable`] has returned `true`, otherwise this blocks.
pub fn read_input(buffer: &mut [u8]) -> io::Result<usize> {
    loop {
        match rustix::io::read(stdin(), &mut *buffer) {
            Ok(count) => return Ok(count),
            Err(rustix::io::Errno::INTR) => continue,
            // Raw mode with a non-blocking fd: nothing to read is not an error.
            Err(rustix::io::Errno::AGAIN) => return Ok(0),
            Err(error) => return Err(error.into()),
        }
    }
}
