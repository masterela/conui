# The terminal handshake: a checklist to drive by hand

CI proves a great deal about conui and nothing at all about this. Every test in the workspace renders
into a `Buffer` and asserts on text, which is the right way to test a layout and is structurally
incapable of testing the part where a real terminal is put into raw mode, written escape sequences,
and handed back. `windows-latest` compiles `platform/windows.rs` and runs its unit tests; it never
once attaches a console to it.

So this is the part a person has to do. Ten minutes on each machine, and what it buys is the
difference between "the Windows backend compiles" and "the Windows backend works" — which is the
only claim that matters to somebody typing `cargo add conui`.

Work through it on **Linux** and on a **Windows console**. macOS is driven interactively every day
here, so it is already covered; run it there too if you want a control to compare against.

## Before you start

```sh
git clone https://github.com/masterela/conui && cd conui
cargo build --examples
```

Record the terminal you are in, because the answer is a property of the terminal and not of the OS:

```sh
echo "$TERM / $COLORTERM / $TERM_PROGRAM"     # Linux
```

On Windows, note which of conhost (`cmd.exe`), Windows Terminal and PowerShell you are in, and
whether it is PowerShell 5 or 7. They are three different terminals with three different bugs.

Run each item in a **fresh** terminal window. A window that has already been left in a bad state
will make the next item look broken for the wrong reason.

---

## 1. Enter and leave cleanly

The base case, and the one every other item is a variation on.

```sh
cargo run -p conui --example monitor
```

**Expect:** the screen is replaced (your scrollback is untouched, not scrolled away), the cursor is
invisible, colour is on, and the process list is populating. Press `Q`.

**Then, back at the prompt, check all five:**

- Your previous scrollback is exactly as it was, with the app's screen gone entirely.
- The cursor is visible and blinking.
- What you type echoes.
- `Ctrl+C` at the prompt interrupts, as it always did.
- Nothing stray was printed — no `[?1049l`, no `0m`, no leftover escape text.

On Linux you can check the mode rather than trusting your eyes:

```sh
stty -a | tr ';' '\n' | grep -E '^ *-?(icanon|echo)$'
```

Both must be **positive** — `icanon` and `echo`, not `-icanon` and `-echo`. A minus sign here is raw
mode left on, which is the single worst failure on this page.

> Exercises `Terminal::enter` / `Terminal::leave` (`crates/conui-term/src/lib.rs:147`), and
> `platform::enter_raw_mode` / `restore_mode`.

## 2. Ctrl+C

Raw mode means the kernel stops turning `Ctrl+C` into a signal, so the app has to recognise the key
itself. `Config::quit_on_ctrl_c` is on by default and every example leaves it on.

```sh
cargo run -p conui --example monitor
# press Ctrl+C
```

**Expect:** it quits, and leaves the terminal exactly as item 1 did. Re-run the five checks above.

Then the same on an example that is mid-edit, because a `Ctrl+C` that a text field swallows is a
program you cannot get out of:

```sh
cargo run -p conui --example todo
# press A to add a task, type a few letters, then Ctrl+C
```

**Expect:** still quits.

**If it does not quit:** you are stuck, and that is the bug — note it, then get out with the terminal
emulator's own close-window, and run `reset` in the next window.

> Exercises `App::poll`'s interrupt check (`crates/conui/src/app.rs:300`) and the parser's mapping of
> the C0 range back to `Ctrl`+letter (`crates/conui-input/src/parser.rs:585`).

## 3. A panic

Not on the original list, but it belongs directly after `Ctrl+C`: it is the same question — does the
terminal come back — asked on the path where the app has stopped cooperating. There is a panic hook
that restores the screen before the default hook prints anything
(`crates/conui-term/src/lib.rs:79`), and if it is broken you will only find out on the day something
else is already broken.

Nothing in the box panics on purpose, so this needs four lines of throwaway code. Write it, run it,
delete it:

```sh
cat > crates/conui/examples/scratch-panic.rs <<'RUST'
use std::time::Duration;

use conui::App;
use conui::widget::Text;

fn main() -> std::io::Result<()> {
    let mut app = App::new()?;
    app.draw(|frame| frame.render_full(&Text::new("about to panic").accent()))?;
    std::thread::sleep(Duration::from_millis(500));
    panic!("on purpose, to check the terminal comes back");
}
RUST
RUST_BACKTRACE=1 cargo run -p conui --example scratch-panic
rm crates/conui/examples/scratch-panic.rs
```

**Expect:** half a second of a cleared screen, then the panic message *and* the backtrace on your
normal screen, scrollable, with the terminal in the state item 1 describes — echo on, cursor visible.

**A failure looks like:** the backtrace flashing past and vanishing, which means it was printed into
the alternate screen buffer before that buffer was discarded — the hook running after the default one
rather than before it. Or the message being visible but the terminal left in raw mode, which is the
restore half not running at all.

While you are here, check the degenerate frame does *not* panic:

```sh
cargo run -p conui --example monitor -- --dump 0 0
```

**Expect:** no output and exit status 0. A zero-column buffer is a thing a resize can produce for one
frame, and it has to be survivable rather than interesting.

## 4. Resize mid-frame

Two separate things: that a resize is noticed at all, and that resizing *while a frame is being
written* does not tear or corrupt.

```sh
cargo run -p conui --example monitor
```

- **Grow and shrink slowly.** The layout reflows. At 96 columns the detail pane on the right appears
  and disappears; the sparklines get longer as the window widens, reaching further back through the
  history.
- **Drag the corner continuously for a good five seconds**, fast, while the once-a-second samples are
  arriving. This is the actual test. **Expect:** no torn rows, no text from a previous size left
  behind, no scrollback pollution, no crash.
- **Shrink below 54 × 14.** **Expect:** the whole view is replaced by `Resize to at least 54 × 14`.
  Grow it back and the app returns, with the process list intact and the cursor where you left it.
- **Maximise, then restore.** No leftovers along the edges.

Then check that it is not a one-off by resizing an app with a scrolled list in it:

```sh
cargo run -p conui --example settings
# TAB to the ABOUT tab, scroll into the middle of it, then resize
```

**Expect:** the pane stays scrolled to roughly where it was, and nothing is drawn outside its panel.

> Exercises `Terminal::sync_size`, the resize event in `App::poll`
> (`crates/conui/src/app.rs:279`), `Config::min_size`, and — on Linux — `SIGWINCH` not being used:
> conui polls the window size rather than trapping the signal, so this is also a check that polling
> keeps up.

## 5. Paste

Bracketed paste is requested on entry, which turns a pasted paragraph into one `Event::Paste` instead
of several hundred key events. When it is not working you get the several hundred key events, so the
failure is not silent — it is spectacular.

First, a single word. Copy `WindowServer` to your clipboard, then:

```sh
cargo run -p conui --example monitor
# press / then paste
```

**Expect:** the field fills instantly and the list narrows to one row. **Not expected:** the
characters arriving one at a time, or — the giveaway — the paste being read as a burst of *sort keys*,
with `r` reversing the order and `m` switching to memory before the field ever opens. Either means the
`?2004` markers were not recognised and the text came through as ordinary key events.

Then a paste with a newline in it, which is the case a single-line field has to survive. Copy these
two lines, newline and all:

```
WindowServer
rustc
```

**Expect:** one line reading `WindowServer rustc` and a list narrowed to nothing — the newline is
deliberately flattened to a space (`Editor::insert_str`), because a stray control glyph in a
single-line field is worse than a wrong search. **Not expected:** the newline acting as `Enter` and
closing the field, or a `^J` appearing in it.

And into the field that writes to a file, where a smuggled newline would corrupt it:

```sh
CONUI_TODO_FILE=/tmp/handshake.md cargo run -p conui --example todo
# press A, then paste the two lines, then ENTER, then Q
cat /tmp/handshake.md
```

**Expect:** one task on one line in the app, and one `- [ ]` line in the file — not two. (The env var
keeps this out of your real `~/.conui-todo.md`.)

On Windows, paste with both `Ctrl+V` and right-click, and in Windows Terminal try
`Ctrl+Shift+V` — they go through different code in the console host.

> Exercises `Capabilities::bracketed_paste`, the `?2004h` request in `Painter::enter_screen`
> (`crates/conui-term/src/writer.rs:112`) and the parser's paste accumulator
> (`crates/conui-input/src/parser.rs:105`).

## 6. Mouse

SGR encoding (`?1006`) with button and drag reporting (`?1000`, `?1002`). The two failure modes worth
watching for are no reporting at all, and reporting that arrives with the coordinates off by one —
which is invisible until you click near an edge.

```sh
cargo run -p conui --example monitor
```

- **Click a process row.** That row is selected and the detail pane on the right changes to it.
  Click the *first* row and the *last visible* row: an off-by-one shows up at the ends, not in the
  middle.
- **Click a column heading** — `PID`, `CPU%`, `MEM`. The table sorts by it, and clicking the same one
  again turns it round. Clicking the panel's title row must do **nothing**.
- **Scroll the wheel over the table.** The cursor moves. Scroll over the meters at the top and
  nothing happens. The wheel over the scrollbar itself counts as the table, so that works too.
- **Shrink the window until the process list no longer fits**, so a thumb appears in the column to
  the right of it, then **drag it**. The table scrolls and the cursor travels with it — the detail
  pane on the right changes as you drag, which is the whole point of `Selection::scroll_to`. Press
  the track above and below the thumb: each is one page, not a jump to where you pressed. Drag the
  pointer well past the bottom of the bar and back: the bar must keep following it rather than
  sticking at the moment you left the region.

Then the harder screen, which has overlapping regions:

```sh
cargo run -p conui --example settings
```

- Click a tab, a field, a button.
- Click the `Palette` select to open its dropdown, click an option to choose it, click elsewhere to
  dismiss it. A dropdown is drawn over its neighbours, so a click landing on what is *underneath* is
  a real bug.
- Click into the text field near its right-hand end and check the caret lands where you pointed.
- On the `ABOUT` tab, **drag the scrollbar thumb**, and press the track above and below it.

Finally, and this is the one that bites users:

```sh
cargo run -p conui --example monitor
# press Q, then click around in your shell
```

**Expect:** clicking in the shell does nothing unusual. If escape sequences appear when you click, the
terminal was left reporting mouse events — `?1006l ?1002l ?1000l` on the way out did not land.

One more, and it needs a scratch program because nothing in the box calls it. `App::set_mouse` turns
reporting on and off *while running*, which is the only method on `App` that no test and no example
reaches — a unit test cannot build an `App` without taking over the terminal, and none of the four
examples has a reason to toggle it:

```sh
cat > crates/conui/examples/scratch-mouse.rs <<'RUST'
use conui::widget::Text;
use conui::{App, Config, Event, KeyCode};

fn main() -> std::io::Result<()> {
    let mut app = App::with(Config::new().mouse(true))?;
    let mut on = true;
    let mut last = String::from("move the pointer");
    while app.is_running() {
        for event in app.poll()? {
            match event {
                Event::Key(key) if key.code == KeyCode::Char('m') => {
                    on = !on;
                    app.set_mouse(on)?;
                    last = format!("reporting {}", if on { "on" } else { "off" });
                }
                Event::Key(key) if key.code == KeyCode::Char('q') => app.quit(),
                Event::Mouse(mouse) => last = format!("{:?} at {},{}", mouse.kind, mouse.column, mouse.row),
                _ => {}
            }
        }
        app.draw(|frame| frame.render_full(&Text::new(&last).accent()))?;
    }
    app.leave()
}
RUST
cargo run -p conui --example scratch-mouse
rm crates/conui/examples/scratch-mouse.rs
```

**Expect:** moving the pointer updates the line. `M` says `reporting off` and the line then stops
changing however much you move or click. `M` again and it resumes. `Q` quits, and the shell is clean
afterwards — the check at the end of this item applies doubly here, since the last thing this program
did to the terminal may have been to turn reporting *on*.

> Exercises `Config::mouse`, `Painter::enter_screen`/`leave_screen`, the SGR mouse parser, and every
> `Hits` region in the examples.

## 7. `kill -9`

This one cannot pass, and knowing exactly how it fails is the point. `SIGKILL` cannot be caught, so
no restore code runs — the terminal is left in raw mode with the alternate screen active and the
cursor hidden. Every full-screen program in existence has this property.

```sh
cargo run -p conui --example monitor &
# in a second terminal:
pkill -9 -f 'examples/monitor'
```

**Expect:** the first terminal is wrecked. Now confirm that the standard recovery works:

```sh
reset
```

**Expect:** a normal prompt, echo back on, cursor visible. If `reset` is not enough, try
`stty sane` then `printf '\033[?1049l\033[?25h\033[0m'`, and **note which was needed** — if `reset`
alone does not fix it, conui is putting the terminal into a state stranger than it should.

On Windows, `taskkill /F /PID <pid>` is the equivalent, and the recovery is closing the tab.
Console modes are per-process there rather than per-terminal, so a new window should be clean
regardless; if it is *not*, that is worth knowing, because it would mean the mode leaked.

## 8. While you are in there

Cheap, and each one has caught something in a terminal app before:

- **Redirected.** `cargo run -q -p conui --example monitor -- --dump | od -c | grep 033` — no matches.
  `--dump` renders into a plain `Buffer` and prints its rows, so it is text whatever it is piped into,
  which is what makes it usable in a shell pipeline and in this README.
- **`NO_COLOR=1 cargo run -p conui --example monitor`** — the layout is identical, in no colour.
- **`TERM=dumb cargo run -p conui --example monitor`** — readable, monochrome, no crash.
- **Inside tmux**, then **inside screen**. Both multiplex the sequences and both get them subtly
  wrong; `TERM=tmux-256color` in particular.
- **An 8-colour terminal:** `TERM=xterm cargo run -p conui --example settings`. Colours are supposed
  to degrade at write time, so the screen should still be legible rather than uniformly grey.
- **A very small window and a very large one** — 40 × 10, and full-screen on a 4K display.
- **Over ssh**, which is where `TERM` is most likely to be something nobody tested.

---

## Report

Copy this in, fill it in, and paste it into an issue. A "no" with the terminal named is worth more
than a page of description.

```
Terminal:            (e.g. GNOME Terminal 3.50 / TERM=xterm-256color / COLORTERM=truecolor)
OS:                  (e.g. Ubuntu 24.04, x86_64)
conui commit:

1  enter and leave cleanly      pass / fail —
2  Ctrl+C                       pass / fail —
3  a panic restores the screen   pass / fail —
4  resize, including mid-drag    pass / fail —
5  paste as one event            pass / fail —
6  mouse, and no leak on exit    pass / fail —
7  kill -9, and `reset` recovers pass / fail —
8  anything from section 8:
```

What a failure report needs, in order of usefulness: the terminal and its `TERM`, which item, what
you saw instead, and — if the terminal was left in a state — what it took to recover. A screenshot of
a torn frame is worth having; so is a `cat -v` of any stray sequence that ended up in your shell.
