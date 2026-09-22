# conui

A devkit for building console applications with a proper user interface, in pure Rust.

No curses, no crossterm, no ratatui. conui talks to the terminal itself — termios through
`rustix` on Unix, the console API through `windows-sys` on Windows — and puts two APIs over one
renderer: an immediate-mode cell canvas, and a declarative view tree. You are meant to use both.

```
   CONUI  /  LOCAL INTELLIGENCE                                                                  LIVE
   ──────────────────────────────────────────────────────────────────────────────────────────────────

   S N A K E                      ROUND 01                conui · snake
                                                          macos aarch64 · Rust
   ┌────────────────────────────────────────────────┐
   │· · · · · · · · · · · · · · · · · · · · · · · · │     NEXT MOVE      POLICY WEIGHTS
   │· · · · · · · · · · · · · · · · · · · · · · · · │
   │· · · · · · · · · · · · · · · · · · · · · · · · │       UP     ████░░░░░░░░░░░░░░  0.23
   │· · · · · · · · · · · · · · · · · · · · · · · · │       DOWN   █████░░░░░░░░░░░░░  0.25
   │· · · · · · · · · · · · · · · · · · · · · · · · │       LEFT   ░░░░░░░░░░░░░░░░░░  0.00
   │· · · · · · · · · · · · · · · · · · · · · · · · │     › RIGHT  █████████░░░░░░░░░  0.51
   │· · · · · · · · · · · · · · · · · · · · · · · · │
   │· · · · · · · · · · · · · · · · · · · · · · · · │     EXECUTING   RIGHT
   │· · · · · · · · · · · · · · · · · · · · · · · · │
   │· · · · · · · · · · · · · · · · · · · · · · · · │     DEAD-END RISK
   │██████████████████████· · · · ● · · · · · · · · │     ━━━━━━━━━━━━━━━━━━━━━━━━     0.00
   │██· · · · · · · · · · · · · · · · · · · · · · · │
   │██· · · · · · · · · · · · · · · · · · · · · · · │     FOOD REACHABLE
   │██· · · · · · · · · · · · · · · · · · · · · · · │     ━━━━━━━━━━━━━━━━━━━━━━━━     1.00
   │██· · · · · · · · · · · · · · · · · · · · · · · │
   │██████████████████████████████████████████· · · │     INFERENCE          0.18 ms
   └────────────────────────────────────────────────┘     DECISIONS         900.0 /s
                                                          CELLS VISITED     1396
   SCORE             LENGTH            BEST               NETWORK           OFFLINE
   █▀█ ▀▀█ █▀█       █▀█ ▀▀█ █▀▀       █▀█ ▀▀█ █▀█        ENGINE            conui · Rust
   █ █ ▀▀█ █ █       █ █ ▀▀█ █▀█       █ █ ▀▀█ █ █
   ▀▀▀ ▀▀▀ ▀▀▀       ▀▀▀ ▀▀▀ ▀▀▀       ▀▀▀ ▀▀▀ ▀▀▀        conui + cycle safety
                                                          Shield interventions  0001
   ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━   9.4%

   ──────────────────────────────────────────────────────────────────────────────────────────────────
   SPACE pause   ↑/↓ speed   R reset   Q quit   G shield  LOCAL HEURISTIC POLICY       00:00
```

That is the bundled demo, in truecolor on a near-black ground. Run it:

```sh
cargo run -p conui --example snake
```

`--dump [steps]` prints one composed frame as plain text and exits, which is how the layout above
was checked without a terminal.

The second demo is the same toolkit used the other way round — no coordinates anywhere, just a tree
of views over a list you can scroll, filter and edit:

```
   CONUI  /  TODO                                                     3 of 6 done
   ──────────────────────────────────────────────────────────────────────────────
   ┌─ TASKS · ALL ────────────────────────────────────┐  DONE         OPEN
   │   ✓ wire the focus layer                         │  █▀█ █▀█ ▀▀█  █▀█ █▀█ ▀▀█
   │ › ✓ port the laya snake UI                       │  █ █ █ █ ▀▀█  █ █ █ █ ▀▀█
   │   ✓ ship List and Input                          │  ▀▀▀ ▀▀▀ ▀▀▀  ▀▀▀ ▀▀▀ ▀▀▀
   │   · verify the windows backend on windows        │  PROGRESS  █████░░░░░ 50%
   │   · write a scroll viewport                      │  TOTAL    6
   │   · publish 0.1 to crates.io                     │  SHOWN    6
   │                                                  │  FILE     —
   └──────────────────────────────────────────────────┘  6 loaded
   ──────────────────────────────────────────────────────────────────────────────
   ↑/↓ move  SPACE toggle  A add  E edit  D del  F filter  C clear  Q quit
```

```sh
cargo run -p conui --example todo
```

Tasks persist as a markdown checklist in `$CONUI_TODO_FILE`, or `~/.conui-todo.md`. It is a real
program in under 600 lines, and its own `#[cfg(test)]` module drives the actual key handler and
asserts on rendered rows — 18 tests, no terminal involved.

The third demo is the one with controls: a focus ring walked with `Tab`, tabs, buttons, a text
field, and dropdowns whose lists are drawn over everything else.

```
  CONUI  /  SETTINGS                                                      MODIFIED
  APPEARANCE   LAYOUT   ABOUT
  ──────────

  Palette     [ LAYA             ▴ ]                ┌─ PREVIEW ──────────────────┐
              ┌────────────────────┐                │ ████ accent                │
  Bars        │› LAYA              │                │ ████ warn                  │
              │  EMBER             │                │ ████ danger                │
  Heading     │  INHERIT           │                │ ████ info                  │
              └────────────────────┘                │ ████ text                  │
                                                    │ ████ muted                 │
                                                    │                            │
                                                    │ BARS   █████████▎      62% │
                                                    │                            │
                                                    │ LOCAL INTELLIGENCE         │
  ‹ Revert ›  ‹ Apply ›                             └────────────────────────────┘
  ────────────────────────────────────────────────────────────────────────────────
  ←/→ change   ↵ open   TAB next                                             ready
```

```sh
cargo run -p conui --example settings
```

Applying a theme re-themes the live app; the `PREVIEW` panel shows the *draft* palette before you
commit to it, which a `Role` cannot express and a `Paint` closure can. `--dump` takes `--open` and
`--tab N` so any state of it can be printed as text; 17 of its tests do exactly that.

## Quick start

```rust
use conui::view::{Column, Row, ViewExt};
use conui::widget::{Gauge, Hints, Stat, Text};
use conui::App;

fn main() -> std::io::Result<()> {
    let mut app = App::new()?;
    while app.is_running() {
        for event in app.poll()? {
            if let Some(key) = event.as_key() {
                if key.is_char('q') {
                    app.quit();
                }
            }
        }
        app.draw(|frame| {
            let screen = Column::new()
                .child(Text::new("DASHBOARD").accent())
                .child(
                    Row::new()
                        .gap(2)
                        .child(Stat::new("SCORE", 42).length(4))
                        .child(Gauge::new(0.6).label("LOAD").flex(1))
                        .length(4),
                )
                .child(Hints::new().key("Q", "quit").length(1));
            frame.render_full(&screen);
        })?;
    }
    Ok(())
}
```

`App` owns the terminal lifecycle and nothing else. There is deliberately no `run(closure)` that
owns your state: `poll` blocks correctly (until the next tick, or until the escape timeout when
the input parser is holding an ambiguous byte) and `draw` presents one frame atomically. The loop
stays yours, so awaiting something, owning a channel, or stepping a simulation at its own rate
never turns into a fight with the framework.

## The two layers

**The canvas** is a clipped, origin-shifted view into a cell buffer. Coordinates are local and
signed, so arithmetic that runs off an edge clips instead of panicking or wrapping.

```rust
use conui::{Buffer, Canvas, Rect, Role, Theme};

let mut buffer = Buffer::new(20, 3);
let mut canvas = Canvas::new(&mut buffer, Rect::sized(20, 3), Theme::LAYA);
canvas.text(0, 0, "SCORE");
canvas.bar(0, 1, 0.5, 10, Role::Accent);
assert!(buffer.row_text(0).starts_with("SCORE"));
```

Beyond text and bars it has `mesh`, `rule`, `run`, `border`, `gradient`, `sparkline`, `style_area`
and `number` — three-row half-block seven-segment digits, the typography that makes the stats in
the demo read as instrumentation rather than as text.

**The view tree** is plain values, rebuilt every frame. A `View` is `render(&self, &mut Canvas)`
plus a `constraint()`; `Row`, `Column`, `Panel` and friends resolve a layout and hand each child a
*sub-canvas*, which is what makes composition safe — a child that miscalculates cannot corrupt a
sibling, only its own region.

The two meet at `Canvas::sub(rect)`. Every widget only ever needs a region, so the demo places
`Stat`, `Gauge`, `Field` and `Hints` at absolute coordinates on a hand-drawn board:

```rust
fn place(canvas: &mut Canvas<'_>, x: u16, y: u16, w: u16, h: u16, view: &dyn View) {
    let mut slot = canvas.sub(Rect::new(x, y, w, h));
    view.render(&mut slot);
}
```

## What is in the box

| | |
|---|---|
| Layout | `Constraint::{Length, Percentage, Ratio, Min, Max, Fill}`, `Row`, `Column`, `Spacer`, `Padded`, `centered` |
| Widgets | `Text`, `Rule`, `Gauge`, `Stat`, `Sparkline`, `Field`, `Panel`, `Hints`, `List`, `Input`, `Button`, `Tabs`, `Select`, `Menu` |
| State | `Selection` (cursor + its own scroll offset), `Editor` (grapheme-aware single-line editing), `Focus<T>` (a ring of your own ids), `Dropdown` (open/closed + where it landed) |
| Escapes | `Paint(closure)`, `When`, and the raw `Canvas` |
| Themes | `Theme::LAYA` (default), `Theme::EMBER`, `Theme::INHERIT`; nine semantic `Role`s |

Layout is resolved by an explicit order rather than a constraint solver: rigid sizes are honoured
first, `Fill` absorbs surplus by weight, and splits are apportioned by largest remainder so a row of
three always tiles its width exactly. Under a deficit the elastic classes collapse first (`Fill`,
then `Max`, `Min`, `Percentage`/`Ratio`, and `Length` last), and within a class the cut is
proportional to current size —
so the tallest child in an over-subscribed column is the one that loses the most, which is worth
knowing when you decide what a column asks for.

One sharp edge: a widget's default `constraint()` is its *vertical* preference — `Stat` asks for the
four rows its block digits need. `Row` reads the same method for a width, so a `Stat` in a `Row`
gets four columns unless you say `.length(11)`. Give children in a `Row` an explicit width.

The widget set is small on purpose, and most of them are a few dozen lines over the canvas — which
is the point: a widget you need that is missing is a `Paint` closure away, not a framework
extension.

### Widgets are stateless; the state is yours

A view tree is rebuilt every frame, so a view cannot be where a selected index or a text cursor
lives. Those go in plain structs your app owns, and the view borrows one for the frame:

```rust
use conui::view::ViewExt;
use conui::widget::{Input, List};
use conui::{Editor, Selection};

struct Screen {
    tasks: Vec<String>,
    selection: Selection,
    editor: Editor,
}

impl Screen {
    fn key(&mut self, key: &conui::KeyEvent) {
        match key.code {
            conui::KeyCode::Up => self.selection.up(),
            conui::KeyCode::Down => self.selection.down(self.tasks.len()),
            // Editor takes text, movement and the Ctrl line-editing keys, and deliberately never
            // consumes Enter, Escape or Tab — what those mean is the app's decision, not a field's.
            _ => {
                self.editor.handle(key);
            }
        }
    }

    fn view(&self) -> impl conui::View + '_ {
        conui::view::Column::new()
            .child(List::new(self.tasks.iter().map(String::as_str)).selection(&self.selection).highlight())
            .child(Input::new(&self.editor).prompt("+").length(1))
    }
}
```

`Selection` keeps its own scroll offset, and `List` slides it during render — the region height is
only known then, so there is no `scroll_into_view` for the caller to forget. `Editor` moves and
deletes by grapheme cluster, not by byte or `char`, and reports a display *column* for the cursor,
so an emoji or a combining accent in a field does not desynchronise the caret. `Input` paints its
own block cursor as a reversed cell rather than parking the terminal cursor, so a field nested six
levels deep in a layout needs no cooperation from anything above it.

### Focus is a value, not a manager

`Focus<T>` is a ring of *your* ids — a `Copy + PartialEq` enum you declare — and nothing more. It
does not register widgets, so focus never depends on what happened to draw last frame, and a widget
is *told* it is focused rather than asking:

```rust
use conui::view::{Row, ViewExt};
use conui::widget::{Button, Select};
use conui::{Dropdown, Focus, KeyCode, KeyEvent, View};

#[derive(Clone, Copy, PartialEq)]
enum Id {
    Palette,
    Apply,
}

const PALETTES: [&str; 3] = ["LAYA", "EMBER", "INHERIT"];

struct Screen {
    focus: Focus<Id>,
    palette: Dropdown,
}

impl Screen {
    fn new() -> Self {
        Self { focus: Focus::new([Id::Palette, Id::Apply]), palette: Dropdown::new() }
    }

    fn key(&mut self, key: &KeyEvent) {
        // An open list is modal, and modality is an early return: it answers first and swallows
        // everything, Tab included. No modal stack, no event-capture phase.
        if self.palette.is_open() {
            self.palette.handle(key, PALETTES.len());
            return;
        }
        if self.focus.handle(key) {
            return; // Tab and Shift-Tab, and nothing else.
        }
        match self.focus.current() {
            Some(Id::Palette) => {
                self.palette.handle(key, PALETTES.len());
            }
            Some(Id::Apply) if key.code == KeyCode::Enter => { /* apply */ }
            _ => {}
        }
    }

    fn view(&self) -> impl View + '_ {
        Row::new()
            .gap(2)
            .child(
                Select::new(&self.palette, PALETTES)
                    .focused(self.focus.is(Id::Palette))
                    .length(Select::width(&PALETTES)),
            )
            .child(
                Button::new("Apply")
                    .focused(self.focus.is(Id::Apply))
                    .length(Button::width("Apply")),
            )
    }
}
```

There are no disabled entries: a control that cannot be reached is simply not in the ring, and
`set_ring` rebuilds it — keeping the current focus if that entry survived — which is how the
settings demo changes the tab order when you change tab.

### Overlays are a second pass

A view is clipped to its sub-canvas and structurally *cannot* draw outside it. That is the property
that makes composition safe, and it means a dropdown's list cannot be part of the field that owns
it. So `Select` records where it landed during render, and the app draws the list afterwards:

```rust,no_run
# use conui::view::Column;
# use conui::widget::Menu;
# use conui::{App, Dropdown};
# const PALETTES: [&str; 3] = ["LAYA", "EMBER", "INHERIT"];
# let mut app = App::new().unwrap();
# let dropdown = Dropdown::new();
# let screen = Column::new();
app.draw(|frame| {
    frame.render_full(&screen); // The field records its absolute rect here.
    if dropdown.is_open() {
        let menu = Menu::new(dropdown.selection(), PALETTES);
        frame.render(&menu, dropdown.popup_area(PALETTES.len(), frame.area()));
    }
})?;
# Ok::<(), std::io::Error>(())
```

Z-order is draw order, `popup_area` flips the list above the field when there is no room below it
and clamps to the screen, and `Menu` clears its own region first — an overlay that lets the frame
show through is not one. The absolute rect comes from `Canvas::screen_area()`, the only thing in the
canvas API that speaks buffer coordinates, because the only two things that need them are overlays
and hit-testing.

There is still no hit-testing: nothing routes a click to a widget, so every screen here is
keyboard-driven. Nor is there a general scroll viewport — a `List` scrolls itself, but an arbitrary
subtree taller than its region is clipped, not scrolled.

## The crates

conui is the top of a stack that is useful at every level, so no layer is a dead end.

| Crate | What it is |
|---|---|
| `conui-cell` | Cells, grapheme symbols with display width, colours with depth degradation, styles, and the buffer that diffs one frame against the last. No I/O — a pure data structure you can assert on in a test. |
| `conui-term` | Raw mode, the alternate screen, capability detection, size via `ioctl`, and an ANSI writer that emits the minimum SGR and cursor motion for a diff. |
| `conui-input` | An incremental byte-to-`Event` parser: keys with modifiers, mouse, bracketed paste, focus. No framing assumptions — ambiguous bytes are held, and a lone `ESC` is resolved by an idle timeout. |
| `conui` | Canvas, typography, layout, views, widgets, `App`. |

A program that only wants "print a table with colour" can depend on `conui-cell` and its own
`print!`. One that wants a full-screen app uses `App`.

### Things it gets right that are easy to get wrong

- Raw mode is restored on **every** exit path, including a panic.
- The terminal size is re-read every frame via `ioctl`, not trusted from startup and not tracked
  with a `SIGWINCH` handler — a library has no business claiming a process-wide signal disposition.
- Frames are wrapped in synchronized output (DECSET 2026), so a resize mid-paint cannot tear.
- A double-width glyph half outside a clip is replaced by a space, so the grid never shears.
- Below `Config::min_size` the app's view is replaced by a resize prompt that says what is needed
  and what there is, instead of scrambling.
- `Ctrl+C` quits by default, because in raw mode the kernel no longer turns it into a signal and
  an app that ignores the key cannot be interrupted at all.

## Platform support

| Platform | Status |
|---|---|
| macOS | Developed and tested here (Apple Silicon, Darwin 25). |
| Linux | Same `rustix` termios path as macOS; the platform differences are covered but it has not been run on a Linux box yet. |
| Windows | `crates/conui-term/src/platform/windows.rs` is written against the Console API — `ENABLE_VIRTUAL_TERMINAL_INPUT` and `ENABLE_VIRTUAL_TERMINAL_PROCESSING`, screen-buffer size, a `WaitForSingleObject` readable poll, and mode restore on exit — but **has not been compiled or run**: there is no Windows toolchain on the development machine. Treat it as unverified. |

## Development

```sh
cargo test --workspace                  # 355 unit tests + 15 doctests
cargo test -p conui --example todo      # 18 more: the example tests itself
cargo test -p conui --example settings  # 17 more
cargo run -p conui --example snake
cargo run -p conui --example todo
cargo run -p conui --example settings
cargo fmt --all                         # rustfmt.toml pins use_small_heuristics = "Max"
```

Test counts by crate: `conui` 220, `conui-cell` 55, `conui-input` 50, `conui-term` 30.

Nothing in the suite needs a terminal. A `Frame` owns nothing but a `Buffer`, so a whole screen
renders into memory and `buffer.row_text(row)` is what the assertions read — which is also what
`--dump` prints, so the text in this README is checked the same way the tests are.

## Credit

The visual language — flat cell grid, restrained palette on near-black, block-digit stats,
bar-run gauges, the whole instrument-panel feel — is modelled on the terminal UI of
[laya-mlx](https://pypi.org/project/laya-mlx/). conui is an independent Rust implementation of
that aesthetic, not a port of its code.

## License

MIT OR Apache-2.0.
