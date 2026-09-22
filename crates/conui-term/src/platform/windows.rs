//! Windows Terminal, PowerShell and conhost, via the Win32 console API.
//!
//! The strategy is to make Windows behave like everything else rather than to special-case it
//! throughout the kit. Switching the console into virtual-terminal mode means it *emits* and
//! *accepts* the same escape sequences as a Unix pty, so [`crate::Painter`] and the input
//! parser are shared verbatim. Only this file knows Windows exists.
//!
//! Virtual-terminal input requires Windows 10 1703 or newer, which is also the floor for
//! 24-bit color in conhost. On anything older, enabling the mode fails and we report it
//! rather than silently rendering escape sequences as literal text.

use std::io;
use std::time::Duration;

use windows_sys::Win32::Foundation::{
    HANDLE, INVALID_HANDLE_VALUE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Console::{
    CONSOLE_MODE, CONSOLE_SCREEN_BUFFER_INFO, DISABLE_NEWLINE_AUTO_RETURN, ENABLE_ECHO_INPUT,
    ENABLE_LINE_INPUT, ENABLE_PROCESSED_INPUT, ENABLE_VIRTUAL_TERMINAL_INPUT,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, GetConsoleMode, GetConsoleScreenBufferInfo, GetStdHandle,
    INPUT_RECORD, KEY_EVENT, PeekConsoleInputW, ReadConsoleInputW, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE, SetConsoleMode,
};
use windows_sys::Win32::System::Threading::{INFINITE, WaitForSingleObject};

/// The console modes to restore on exit.
///
/// Deliberately not `Copy`, even though two `DWORD`s would be: the Unix `SavedMode` is a `Termios`
/// and only `Clone`, and the shared code above the seam has to compile against both. A type that is
/// `Copy` on one platform makes `.clone()` correct there and a lint error here.
#[derive(Clone, Debug)]
pub struct SavedMode {
    input: CONSOLE_MODE,
    output: CONSOLE_MODE,
}

fn stdin_handle() -> io::Result<HANDLE> {
    handle(STD_INPUT_HANDLE)
}

fn stdout_handle() -> io::Result<HANDLE> {
    handle(STD_OUTPUT_HANDLE)
}

fn handle(which: u32) -> io::Result<HANDLE> {
    // SAFETY: `GetStdHandle` takes a constant and returns a borrowed pseudo-handle that must
    // not be closed. It has no preconditions.
    let raw = unsafe { GetStdHandle(which) };
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    Ok(raw)
}

fn console_mode(handle: HANDLE) -> io::Result<CONSOLE_MODE> {
    let mut mode: CONSOLE_MODE = 0;
    // SAFETY: `handle` came from `GetStdHandle` and `mode` is a valid out-pointer.
    if unsafe { GetConsoleMode(handle, &mut mode) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(mode)
}

fn set_console_mode(handle: HANDLE, mode: CONSOLE_MODE) -> io::Result<()> {
    // SAFETY: `handle` came from `GetStdHandle`.
    if unsafe { SetConsoleMode(handle, mode) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

pub fn is_input_tty() -> bool {
    stdin_handle().and_then(console_mode).is_ok()
}

pub fn is_output_tty() -> bool {
    stdout_handle().and_then(console_mode).is_ok()
}

/// Switch the console to raw, virtual-terminal behaviour on both streams.
pub fn enter_raw_mode() -> io::Result<SavedMode> {
    let input = stdin_handle()?;
    let output = stdout_handle()?;
    let saved = SavedMode { input: console_mode(input)?, output: console_mode(output)? };

    // Input: stop the console cooking lines, echoing, and translating Ctrl-C into a signal,
    // and ask for keys as escape sequences so the shared parser can read them.
    let raw_input = (saved.input
        & !(ENABLE_LINE_INPUT | ENABLE_ECHO_INPUT | ENABLE_PROCESSED_INPUT))
        | ENABLE_VIRTUAL_TERMINAL_INPUT;
    set_console_mode(input, raw_input).map_err(|error| {
        io::Error::new(
            error.kind(),
            "this console does not support virtual-terminal input; \
             conui needs Windows 10 version 1703 or newer",
        )
    })?;

    // Output: interpret the escape sequences we write, and do not append a carriage return
    // at the right margin, which would otherwise scroll a full-width frame.
    let raw_output =
        saved.output | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN;
    if let Err(error) = set_console_mode(output, raw_output) {
        // Leave the input side as we found it rather than half-configured.
        let _ = set_console_mode(input, saved.input);
        return Err(error);
    }
    Ok(saved)
}

pub fn restore_mode(saved: &SavedMode) -> io::Result<()> {
    // Restore both even if the first fails, so one bad handle cannot strand the other.
    let input = stdin_handle().and_then(|handle| set_console_mode(handle, saved.input));
    let output = stdout_handle().and_then(|handle| set_console_mode(handle, saved.output));
    input.and(output)
}

/// The visible window size, not the scrollback buffer size.
///
/// The console's buffer is usually far taller than the window; sizing to it would draw most
/// of the frame off-screen.
///
/// Windows has no `SIGWINCH`, so a resize is discovered by calling this again — which the event
/// loop does every frame on every platform anyway, for the same reason it does not install a
/// signal handler on Unix.
pub fn window_size() -> io::Result<(u16, u16)> {
    let handle = stdout_handle()?;
    let mut info: CONSOLE_SCREEN_BUFFER_INFO = unsafe { std::mem::zeroed() };
    // SAFETY: `handle` is a console handle and `info` is a valid out-pointer.
    if unsafe { GetConsoleScreenBufferInfo(handle, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let window = info.srWindow;
    let columns = (window.Right - window.Left + 1).max(1) as u16;
    let rows = (window.Bottom - window.Top + 1).max(1) as u16;
    Ok((columns, rows))
}

/// Wait until stdin has bytes a read would return, or `timeout` elapses.
///
/// The console input handle signals for every input record, not just keys: a resize, a focus
/// change or a mouse move all wake the wait. A plain `ReadFile` after such a wake would block
/// with no bytes available, so non-key records are peeked and drained first, and the wait is
/// retried with whatever time is left.
pub fn wait_readable(timeout: Option<Duration>) -> io::Result<bool> {
    let handle = stdin_handle()?;
    let deadline = timeout.map(|limit| std::time::Instant::now() + limit);

    loop {
        let remaining = match deadline {
            None => INFINITE,
            Some(deadline) => {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    return Ok(false);
                }
                left.as_millis().min(u128::from(INFINITE - 1)) as u32
            }
        };

        // SAFETY: `handle` is a valid console input handle.
        match unsafe { WaitForSingleObject(handle, remaining) } {
            WAIT_OBJECT_0 => {}
            WAIT_TIMEOUT => return Ok(false),
            WAIT_FAILED => return Err(io::Error::last_os_error()),
            _ => return Err(io::Error::other("unexpected result waiting on console input")),
        }

        if has_key_input(handle)? {
            return Ok(true);
        }
        // Only non-key records were pending. Consume one so the handle stops signalling, then
        // go back to waiting for the remainder of the timeout.
        drain_one_record(handle)?;
    }
}

/// Whether any pending record would produce bytes on a read.
fn has_key_input(handle: HANDLE) -> io::Result<bool> {
    const PEEK: usize = 32;
    let mut records: [INPUT_RECORD; PEEK] = unsafe { std::mem::zeroed() };
    let mut read: u32 = 0;
    // SAFETY: the buffer holds `PEEK` records and the count matches it.
    if unsafe { PeekConsoleInputW(handle, records.as_mut_ptr(), PEEK as u32, &mut read) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(records[..read as usize].iter().any(|record| {
        // Only a key-down carries a character; key-up records are filtered by the console
        // before reaching a read, so treating them as readable would spin the loop.
        record.EventType == KEY_EVENT as u16 && unsafe { record.Event.KeyEvent }.bKeyDown != 0
    }))
}

fn drain_one_record(handle: HANDLE) -> io::Result<()> {
    let mut record: INPUT_RECORD = unsafe { std::mem::zeroed() };
    let mut read: u32 = 0;
    // SAFETY: a single-record buffer with a matching count.
    if unsafe { ReadConsoleInputW(handle, &mut record, 1, &mut read) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Read whatever is available on stdin, as bytes.
///
/// With virtual-terminal input enabled the console delivers UTF-8 escape sequences here, so
/// the platform-independent parser handles the result unchanged.
pub fn read_input(buffer: &mut [u8]) -> io::Result<usize> {
    use std::io::Read;
    // `std::io::Stdin` reads the same console handle and already handles partial reads and
    // UTF-16 to UTF-8 translation, which is fiddly to redo correctly by hand.
    io::stdin().lock().read(buffer)
}
