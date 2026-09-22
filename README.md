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

Nothing on that screen is decorative, and 35 tests hold it to that. The snake's safety shield really
does keep it alive until the board is full — three seeds, played out to a win in the test — and with
the shield switched off the same policy dies in about thirty moves, which is the comparison the
panel is there to show you.

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
   │   · set up CI for linux and windows              │  TOTAL    6
   │   · publish 0.1 to crates.io                     │  SHOWN    6
   │                                                  │  FILE     —
   └──────────────────────────────────────────────────┘  6 loaded
   ──────────────────────────────────────────────────────────────────────────────
   ↑/↓ move  SPACE toggle  A add  E edit  D del  F filter  C clear  Q quit
```

```sh
cargo run -p conui --example todo
```

Tasks persist as a markdown checklist in `$CONUI_TODO_FILE`, or `~/.conui-todo.md`. Rows are
clickable, and clicking a task's tick ticks it — resolved against where the list drew itself last
frame, which is the only thing that knows how far it had scrolled. It is a real program in under 700
lines, and its own `#[cfg(test)]` module drives the actual key and mouse handlers and asserts on
rendered rows — 27 tests, no terminal involved.

The third demo is the one with controls: a focus ring walked with `Tab`, tabs, buttons, a text
field, and dropdowns whose lists are drawn over everything else — all of it reachable with the mouse
as well.

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
commit to it, which a `Role` cannot express and a `Paint` closure can. The whole screen works with
the mouse too: click a tab, a field or a button, click a select to open it, click an option to choose
it, click anywhere else to dismiss it. Clicking into the text field puts the caret where you pointed,
even when the field has scrolled sideways. The `ABOUT` tab holds more text than fits and scrolls —
with the wheel, the arrows, `PAGE UP`/`PAGE DOWN`, `HOME` and `END`, or by dragging its scrollbar's
thumb, which pages when you press the track beside it — while the buttons below it stay put. `/` over
that pane opens a find field in the footer and `n` repeats the search: the viewport scrolls the least
that brings the match into view, and the pane is composed from a table so that the search can count
the row a word is on — a layout will not tell you that. `--dump` takes `--open` and `--tab N` so any
state of it can be printed as text, which is how most of its tests assert on the layout.

The fourth is the one with a job. The other three own everything on their screens, which makes them
good demonstrations and weak evidence: nothing under them changes unless a key is pressed. This one
reads the machine once a second.

```
  CONUI  /  MONITOR                                                 studio.local · up 4d 02:11
  ────────────────────────────────────────────────────────────────────────────────────────────
  CPU  ▂▄▆▇▇▇▆▅▄▄▄▃▃▂▂▂▂▂▂▃▃▃▃▃             34%   MEM  ▄▄▄▅▅▅▅▅▅▄▄▄▄▄▄▄▄▅▅▅▅▄▄  9.1 GB / 16 GB
       ████████░░░░░░░░░░░░░░░░                        █████████████░░░░░░░░░░

  PROCESSES · CPU ↓                                                 MACHINE
        PID   CPU%       MEM  COMMAND                               CORES                    8
          0  124.0    1.1 GB  kernel_task                           PROCS                    8
        182   81.0    742 MB  WindowServer                          SHOWN                    8
  ›    4821   62.0    205 MB  cargo                                 VIA              a fixture
       4832   58.0    464 MB  rustc
        311    7.0     92 MB  mds_stores                            cargo · 4821
       1204    0.9     12 MB  monitor                               CPU                   62.0
         97    0.4    6.0 MB  fseventsd                             MEM            205 MB · 1%
         93      —     31 MB  logd                                  THREADS                  9
                                                                    STATE              running

  ────────────────────────────────────────────────────────────────────────────────────────────
  ↑/↓ move  C cpu  M mem  P pid  N name  R reverse  / filter  SPACE pause  Q quit
```

```sh
cargo run -p conui --example monitor
```

Sort by any column with a key or by clicking its heading, `/` to filter by name or pid, `SPACE` to
freeze the figures while still moving about in them. Two things here that a self-contained demo never
has to face:

**The list re-sorts under the cursor.** A `Selection` holds a row *index*, and an index is a claim
about an ordering — so the moment a sample arrives with a process somewhere else, the cursor is on
something the user never chose. The app keeps a pid instead and re-derives the index after every
sample, sort and keystroke, which makes the selection a view of the focus rather than a second thing
to keep in step. Five tests do nothing but move the machine around underneath it.

**Some figures do not exist.** CPU percentage is not a value a machine holds; it is the difference
between two readings of cumulative CPU time. So it is unknown until the second sample, and unknown
for ever on a platform with nothing to read — which is why every figure on that screen is an `Option`
and prints `—` rather than `0.0`. Zero is a claim, and calling a busy process idle is worse than
admitting to not knowing. `logd` in the frame above is the case, and it sorts to the bottom whichever
way the column points.

The sampler is `ps`, `vm_stat` and `sysctl` on macOS, `/proc` on Linux and `tasklist` on Windows —
shelled out to and parsed, because an example that needed a dependency conui does not have would
misrepresent what the library costs. Windows therefore has no CPU figures at all, which is the same
`—` path every platform takes for its first second. `--dump` renders a frozen fixture rather than
this machine, so its output is identical on every platform and something a test can assert on; the
live parsers are covered by tests that read the machine they are running on, in all three CI jobs.

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
                        .child(Stat::new("SCORE", 42))
                        .child(Gauge::new(0.6).label("LOAD").flex(1))
                        .length(4),
                )
                .child(Hints::new().key("Q", "quit"));
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
plus a `constraint(axis)`; `Row`, `Column`, `Panel` and friends resolve a layout and hand each child a
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
| Layout | `Constraint::{Length, Percentage, Ratio, Min, Max, Fill}`, `Row`, `Column`, `Spacer`, `Padded`, `Scroll`, `centered` |
| Widgets | `Text`, `Rule`, `Gauge`, `Stat`, `Sparkline`, `Field`, `Panel`, `Hints`, `List`, `Input`, `Button`, `Tabs`, `Select`, `Menu`, `Scrollbar` |
| State | `Selection` (cursor, its own scroll offset, and the height it was drawn at), `Editor` (grapheme-aware single-line editing), `Focus<T>` (a ring of your own ids), `Dropdown` (open/closed + where it landed), `Hits<T>` (where each control landed), `Viewport` (how far a pane with no cursor has been scrolled) |
| Combinators | `.flex`, `.length`, `.percent`, `.ratio`, `.at_least`, `.at_most`, `.padded`, `.hit`, `.fit` |
| Escapes | `Paint(closure)`, `When`, and the raw `Canvas` |
| Themes | `Theme::LAYA` (default), `Theme::EMBER`, `Theme::INHERIT`; nine semantic `Role`s |

Layout is resolved by an explicit order rather than a constraint solver: rigid sizes are honoured
first, `Fill` absorbs surplus by weight, and splits are apportioned by largest remainder so a row of
three always tiles its width exactly. Under a deficit the elastic classes collapse first (`Fill`,
then `Max`, `Min`, `Percentage`/`Ratio`, and `Length` last), and within a class the cut is
proportional to current size —
so the tallest child in an over-subscribed column is the one that loses the most, which is worth
knowing when you decide what a column asks for.

A view is asked what it wants **per axis**, because its two intrinsic sizes are different
questions: a label is one row tall and as wide as its text, and one number cannot answer both. So
`constraint(axis)` takes the axis its parent is dividing — a `Stat` says four rows to a `Column` and
eleven columns to a `Row`, and a `Row` of buttons needs no widths written beside it. A view with no
intrinsic size along an axis answers `Fill(1)`, an equal share of whatever is spare, which is also
the default; a `Gauge` answers that across and `Length(1)` down. `.length(n)` and its siblings
override whichever axis the parent asks about, so the same call means columns in a `Row` and rows in
a `Column`, and stays useful for what it is actually good at: overriding a view that *does* know its
size, to line two of them up.

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
only known then, so there is no `scroll_into_view` for the caller to forget. It keeps that height
too, which is why `selection.page_up()` takes no page size: the window the list was drawn at is the
only number that is really a page, and a constant in a key handler is wrong at every size but one.
`Editor` moves and
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
        // No widths: a `Row` asks each child how wide it is, and both of these know.
        Row::new()
            .gap(2)
            .child(Select::new(&self.palette, PALETTES).focused(self.focus.is(Id::Palette)))
            .child(Button::new("Apply").focused(self.focus.is(Id::Apply)))
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

### A click is resolved against last frame

Focus is *declared* — you write the ring — so `Focus` never has to learn anything at render time. A
click is the opposite: it has to be resolved against pixels that were actually on screen, and the
only thing that knows where those were is the frame that drew them. So there is a second structure,
and it does record during render:

```rust
use conui::view::{Column, ViewExt};
use conui::widget::Button;
use conui::{Frame, Hits, MouseEvent, Pos};

#[derive(Clone, Copy, PartialEq)]
enum Id { Save, Cancel }

struct Ui { hits: Hits<Id> }

impl Ui {
    fn compose(&self, frame: &mut Frame<'_>) {
        // Last frame's geometry is gone the moment this one starts. A stale hit points at where a
        // control used to be, which is the bug that makes a UI feel haunted.
        self.hits.clear();
        let screen = Column::new()
            .child(Button::new("Save").hit(&self.hits, Id::Save).length(1))
            .child(Button::new("Cancel").hit(&self.hits, Id::Cancel).length(1));
        frame.render_full(&screen);
    }

    fn clicked(&mut self, mouse: &MouseEvent) {
        match self.hits.at(Pos::new(mouse.column, mouse.row)) {
            Some(Id::Save) => { /* … */ }
            Some(Id::Cancel) => { /* … */ }
            // Not every cell belongs to something, and chrome is not a target.
            None => {}
        }
    }
}
```

`.hit(&hits, id)` is a decorator, so the widget learns nothing: `Button` has no `on_click` and no id
field, and a hit-tested view provably asks for and draws exactly what the undecorated one does.
`Hits::at` searches the last region recorded first, so an overlay drawn in the second pass wins over
whatever it covers.

What gets recorded is the part of the region that was *on screen*, not the part layout asked for. The
difference shows up inside a `Scroll`: a child above the fold has a negative origin, which no `Rect`
can hold, so the clamped version of it would sit over the top row of the pane — a row showing
something else entirely. A control scrolled out of sight, or clipped away by a window too small for
it, records nothing and cannot be clicked. `Canvas::visible_area()` is that answer for anything
drawing at the immediate layer, next to `screen_area()`, which is where the region claims to be.

Resolving *within* a control is the widget's own business, and each one that needs it exposes the
one measurement only it can make: `Tabs::index_at` (a click in the gap between two labels belongs to
neither), `Selection::row_at` (which reads the offset the last window recorded, because a row number
means nothing without knowing how far the list had scrolled), `List::text_column` (left of it is the
tick, and clicking a task's tick is how you tick it), `Editor::view_from` paired with
`Editor::set_cursor_column` (a click in a text field is a caret position, and a field narrower than
its text has scrolled — so the same rule that decided what to draw decides what was clicked, which is
why it belongs to the editor rather than to the widget), and `Dropdown::handle_mouse`, which takes
*every* event while it is open for the same reason its keyboard handler does — a click outside
dismisses the list rather than falling through to what is under it.

One rule makes the difference between this working and almost working: build the thing once and use
it twice. Both examples extract the control into a method — `Ui::tab_bar()`, `Todo::task_list()` —
that compose and the click handler both call, because two constructions drift, and the drift shows
up as clicks landing on the wrong thing.

### Scrolling is a shifted origin, not a re-layout

A `List` has always scrolled itself: `Selection` carries the offset, and `List` slides it during
render because the region height is only known then. An arbitrary subtree is the harder case, and the
answer is not to lay it out smaller. It is drawn *in full*, from an origin above the top of the
visible region, and clipped:

```rust
use conui::view::{Column, Scroll};
use conui::widget::Text;
use conui::{Buffer, Frame, Rect, Theme, Viewport};

// The offset is the state here, and nothing else owns it.
let viewport = Viewport::at(2);

let lines = Column::new()
    .child(Text::new("alpha"))
    .child(Text::new("bravo"))
    .child(Text::new("charlie"))
    .child(Text::new("delta"))
    .child(Text::new("echo"))
    // The opt-in: add the children up and ask for that, instead of stretching to the parent.
    .fit();

let mut buffer = Buffer::new(12, 3);
let mut frame = Frame::new(&mut buffer, Theme::LAYA);
frame.render(&Scroll::new(&viewport, lines).bare(), Rect::sized(12, 3));

assert!(buffer.row_text(0).starts_with("charlie"));
// The draw is the only thing that learned the pane was three rows and the text five.
assert_eq!(viewport.overflow(), 2);
assert!(viewport.is_at_bottom());
```

Underneath is `Canvas::shifted(x, y, width, height)`: a nested canvas at a *signed* offset whose
region may be **larger** than its parent's. `shifted(0, -2, w, 5)` hands the content all five of its
rows starting two above the window, and the clip — intersected with the parent's in signed space, so
containment still holds — throws away what is out of sight. Nothing is re-laid-out, and the content
cannot tell it is half off screen.

What makes that possible is content that can state a height. `Scroll` asks the child for its
vertical constraint by name, whatever it is nested in: `Length(n)` means *n* rows to scroll through, and anything elastic means "I adapt to
whatever region I am given" — which is exactly a thing with nothing to scroll. `Row::fit()` and
`Column::fit()` are the opt-in, and a fitted stack only adds up if every child is measurable; one
`Fill` inside and it goes back to asking for a share, because a share of the parent is not a height.

Bounds belong to the draw. `Viewport::window(height, content)` is called during render, and it is the
only thing that knows either number, so every method clamps against what the last frame actually
showed: `scroll`, `page_up`/`page_down`, `top`/`bottom`, `reveal(row)` — which moves as little as it
can, and not at all when the row is already showing, so following a search hit does not cost the
reader their place — and `set_offset` for restoring a position you saved. That last one is kept as
asked until the first draw — the one moment when nothing is known yet — and a pane that outgrows its
offset shows the end of the text rather than blank rows below it.

This is also why the wheel over a `List` moves the *cursor* instead of a window: `Selection::window`
always scrolls to contain the selection, so an offset nudged on its own is pulled straight back by the
next draw. A `Viewport`'s offset is owned by nothing else, which is what lets a wheel over the
settings demo's `ABOUT` pane behave like a wheel.

`Scrollbar` is the indicator, and `Scroll` puts one in its last column unless you ask for `.bare()`.
It reserves that column before the content is laid out rather than painting over the text afterwards,
draws nothing at all when everything fits — a permanently full bar trains the eye to stop seeing it —
and never shows the thumb at an end it has not reached, because a bar that looks finished with a row
still to read is worse than no bar.

It is also draggable, which is two methods rather than a mode: `thumb(height)` gives the rows the
thumb covers, so a press can tell whether it grabbed the thumb or hit the track beside it, and
`offset_at(top, height)` says which offset draws the thumb at a given row. The second is defined by
*asking* the first — a binary search over offsets, since the thumb only moves down as the offset grows
— rather than by inverting its arithmetic somewhere else, which is how a thumb comes to jump out from
under the pointer halfway down a drag. What the app keeps is one `Option<u16>`: how far below the
thumb's top the press landed, so the thumb follows the hand instead of recentring itself on it. A drag
belongs to whatever the press grabbed, so it goes on working when the pointer wanders off a bar one
column wide, and the release ends it.

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
- Mouse reporting is opt-in (`Config::mouse(true)`) and is turned off on the way out along with
  bracketed paste and the alternate screen — a terminal left reporting clicks after the program
  exits spits escape sequences into the user's shell.

## Platform support

| Platform | Status |
|---|---|
| macOS | Developed and tested here (Apple Silicon, Darwin 25), and in CI on `macos-latest`. |
| Linux | Builds and passes the whole suite in CI on `ubuntu-latest`, over the same `rustix` termios path as macOS, and the `monitor` example's `/proc` parser is exercised against the runner's own machine. Not yet driven interactively in a Linux terminal. |
| Windows | `crates/conui-term/src/platform/windows.rs` — `ENABLE_VIRTUAL_TERMINAL_INPUT` and `ENABLE_VIRTUAL_TERMINAL_PROCESSING`, screen-buffer size, a `WaitForSingleObject` readable poll, and mode restore on exit — **compiles, and the whole suite passes**, on `windows-latest` in CI. It has never been driven interactively in a real console, so what remains unproven is the part no headless test can reach: that raw mode, VT mode and the escape output actually behave in conhost, Windows Terminal and PowerShell. |

Every test in this workspace renders into a `Buffer` and asserts on text, which is the right way to
test a layout and is structurally incapable of testing the handshake itself: raw mode, live input
decoding, and giving the terminal back. [`docs/terminal-handshake.md`](docs/terminal-handshake.md) is
the checklist for the part a person has to sit down and do — eight items, about ten minutes per
machine, with a report template at the end. It is what the two rows above are waiting on.

## Development

```sh
cargo test --workspace                  # 426 unit tests + 19 doctests
cargo test -p conui --example snake     # 35 more: the example tests itself
cargo test -p conui --example todo      # 27 more
cargo test -p conui --example settings  # 55 more
cargo test -p conui --example monitor   # 59 more, three of which read the machine they run on
cargo run -p conui --example snake
cargo run -p conui --example todo
cargo run -p conui --example settings
cargo run -p conui --example monitor
cargo fmt --all                         # rustfmt.toml pins use_small_heuristics = "Max"
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps   # every public item is documented
```

The three lower crates carry `#![warn(missing_docs)]`, so an undocumented public item is a build
warning rather than something a reader discovers on docs.rs. `conui` itself is getting there a module
at a time: `app`, `canvas`, `frame`, `layout`, `state`, `theme` and `view` carry
`#[warn(missing_docs)]` and are done; `typography` and `widget` are not yet. A module with the
attribute cannot regress,
which is the part that matters — the alternative was one enormous change, or a crate-level `allow`
that would have made the lint decorative.

`.github/workflows/ci.yml` runs exactly those commands on `macos-latest`, `ubuntu-latest` and
`windows-latest`, plus rustfmt once, rustdoc once and a `1.85` MSRV check — a `rust-version` nothing
verifies is one that drifts the first time a newer API looks convenient. Clippy runs on *every*
platform rather than just Linux, because the first run of this workflow found dead code in
`platform/windows.rs` that a Linux-only lint job structurally cannot see, and that file is the one
with no local compiler to check it. Rustdoc needs no such thing: it reads the same source everywhere,
and it runs with `-D warnings` so a dead intra-doc link fails the build rather than waiting to be
found by a reader.

Test counts by crate: `conui` 289, `conui-cell` 55, `conui-input` 52, `conui-term` 30.

Nothing in the suite needs a terminal. A `Frame` owns nothing but a `Buffer`, so a whole screen
renders into memory and `buffer.row_text(row)` is what the assertions read — which is also what
`--dump` prints, so the text in this README is checked the same way the tests are.

### Publishing goes in dependency order

`cargo package` rewrites path dependencies into registry ones, so a crate cannot be packaged until
everything it depends on is already on crates.io. The order is forced:

```sh
cargo publish -p conui-cell     # unicode-width only
cargo publish -p conui-input    # no dependencies at all
cargo publish -p conui-term     # needs conui-cell published
cargo publish -p conui          # needs all three
```

The first two can be dry-run at any time — `cargo package -p conui-cell` builds the crate from its
own tarball, which catches a file the manifest forgot to include. The last two cannot, which is the
one thing about this layout that costs something.

## Credit

The visual language — flat cell grid, restrained palette on near-black, block-digit stats,
bar-run gauges, the whole instrument-panel feel — is modelled on the terminal UI of
[laya-mlx](https://pypi.org/project/laya-mlx/). conui is an independent Rust implementation of
that aesthetic, not a port of its code.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. This is the convention across the Rust ecosystem, so conui imposes no choice a
project has not already made. Unless you state otherwise, any contribution you intentionally submit
for inclusion in this work is licensed the same way, with no additional terms.
