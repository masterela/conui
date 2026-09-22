# conui-term

The terminal layer of [conui](https://github.com/masterela/conui): raw mode, capability detection,
and frame presentation.

`Terminal` is the only thing in the kit that touches real file descriptors. It owns the *front
buffer* — conui's model of what is currently on screen — so that `present` can diff a freshly
rendered frame against it and write only the difference, as the shortest escape sequence that says
it.

```rust,no_run
use conui_cell::{Buffer, Color, Style};
use conui_term::Terminal;

fn main() -> std::io::Result<()> {
    let mut terminal = Terminal::new()?;
    terminal.enter()?;

    let (width, height) = terminal.size();
    let mut frame = Buffer::new(width, height);
    frame.set_str(2, 1, "hello", Style::new().fg(Color::hex("#62f5b5")), width);
    terminal.present(&frame, None)?;

    terminal.leave()
}
```

Everything platform-specific lives behind a seam of seven functions and one type — is-a-tty for each
stream, enter raw mode, restore it, window size, wait-readable, read, and the saved mode itself —
implemented once for Unix with `rustix` and once for Windows with `windows-sys`, and nowhere else. On
Windows the console is switched into virtual-terminal mode,
so it emits and accepts the same escape sequences as a pty and the rest of the kit is shared
verbatim.

`enter` installs a panic hook that restores the terminal first, because a panic in raw mode with the
alternate screen up otherwise leaves the user with an unusable shell.

## License

MIT OR Apache-2.0, at your option.
