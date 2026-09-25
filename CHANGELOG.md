# Changelog

All four crates share one version and are released together. While the major is `0`, read a minor bump
as possibly breaking — that is what Cargo does with it.

## 0.1.0 — unreleased

First release.

### What it is

- **`conui-cell`** — grapheme-aware cells with display width, colours that degrade to the depth the
  terminal actually has, styles, and the buffer that diffs one frame against the last. No I/O.
- **`conui-input`** — an incremental byte-to-`Event` parser: keys with modifiers, SGR mouse, bracketed
  paste, focus. Ambiguous bytes are held rather than guessed at, and a lone `ESC` is resolved by an
  idle timeout.
- **`conui-term`** — raw mode, the alternate screen, capability detection, size by `ioctl`, and a
  writer that emits the minimum SGR and cursor motion for a given diff.
- **`conui`** — `Canvas` for immediate-mode drawing, a declarative `View` tree over the same renderer,
  typography, layout, widgets, retained state (`Selection`, `Editor`, `Focus`, `Dropdown`, `Hits`,
  `Viewport`), and `App` to run the loop.

Four dependencies in the whole tree: `unicode-width`, `unicode-segmentation`, `rustix` on Unix and
`windows-sys` on Windows. No TUI framework underneath it.

### `Table`

`conui::widget::Table` — columns stated once as `Constraint`s, and the heading, the cells and the
answer to "which column was clicked" all derived from the same resolved widths. Added because the
`monitor` example had hand-rolled all three out of `format!("{:>7}")` and three `const`s, and the
three could drift apart without the compiler noticing.

Two things fall out of it that the hand-rolled version could not have. Cells are placed by display
width, so a process named in Japanese no longer pushes every column to its right out by one cell per
ideograph — `{:>7}` counts characters and cannot know. And `Table::hit_at` returns
`TableHit::Heading(i)` or `TableHit::Row(i)` from one call, replacing a `column_at` function, a
`Selection::row_at` call and the caller's own arithmetic about how far the heading was indented.

The `monitor` example is built on it, which is how the API was settled. Its three width constants,
two `format!` strings, hand-rolled `column_at`, second hit region and second mouse handler became one
`const COLUMNS` and one `hit_at`, with the sort arrow moving off the panel title and onto the column
it describes.

### `Progress`, `Buttons` and `Checklist`

Three additions that a long job asked for, in that order: how far through it you are, how to answer a
question about it, and how to say which parts of it to do at all.

`conui::widget::Progress` — work done out of a known total, which is not what `Gauge` measures. A
gauge shows a *level*, and takes an `f32`; this counts whole items, and the difference shows up in
three places. It fills by truncation, so the last column arrives with the last item rather than with
the rounding — `9/10` in a ten-column bar is nine columns, where a rounded fill would show ten and
claim to be finished. The number beside it comes from the same two integers as the bar, so they cannot
disagree. And `total` is optional: `Progress::indeterminate(tick)` marches a block back and forth for
the part of a job spent finding out how big it is, showing the count so far and no percentage, because
there is nothing yet to be a percentage of. A `caption` takes a second row to name the item being
worked on now, which is what turns a bar into something worth watching.

`conui::widget::Buttons` — a row of `Button`s with one of them focused, so a question can be answered
two ways at once. `←`/`→` walk the row and `Enter` takes the focused one, for somebody reading it;
`Buttons::index_for('n')` turns a typed letter straight into the same index, for somebody who already
knows the answer. Both end in a `usize`, so the code that acts on it is written once. Which button is
current is a plain `Selection` — the same state a list uses, because "which of these N" is the same
question whether the N are stacked or in a row — and `index_at` answers a click, taking the region
because a centred row is nowhere near where its labels would suggest.

`conui::state::Checklist` — "which ones", where `Selection` answers "which one". It owns the cursor,
one flag per row, *and* the length, which is the point: the cursor moves need no `len` argument to get
wrong, and `resize` is the single call that happens when the data changes underneath — new rows arrive
unticked, and the cursor comes back inside the list. `toggle`, `check_all`, `clear`, `invert` and
`only(i)` are the five things a picker's keys do. `List::checklist(&list)` draws the boxes and takes
the cursor from the same state, so the two cannot disagree about how many rows there are, and
`List::check_column` says which clicks landed on a box rather than on its label.

### A clear paints the theme's ground

`ED` erases with whatever background is current, and the clears on taking the screen over and after a
resize were emitted straight after a style reset — so the screen was blanked to the *terminal's*
default colour while the front buffer claimed it now held the theme's blank cell. The differ then
skipped every cell that stayed blank, and on a theme unlike the terminal's the gaps between the
writing kept the terminal's colour for the whole run. `Painter::set_ground` names the colour an erase
should leave behind, and `Terminal::set_blank_cell` passes it the background it was given, so the two
can no longer disagree. A theme of `Theme::INHERIT` still emits nothing.

### And the padding, which no cell can reach

The other half of the same bug, and the half a grid cannot fix: a terminal draws a few pixels of
padding around its cells and fills them with its *own* background. An app whose theme is lighter than
the terminal it runs in therefore gets a dark frame around the entire screen, invisible on a dark theme
and glaring on a light one, with every cell inside it already correct. So `Painter::set_ground` now also
tells the terminal, with `ansi::set_background` (OSC 11) on the way in and `ansi::RESET_BACKGROUND`
(OSC 111) on the way out — including from the panic path, because a terminal left holding an app's
background is a terminal the user has to restart. A theme switch while running reaches it too.

Only a true-colour terminal and only an `Rgb` ground: sending an indexed ground would mean this crate
deciding what the user's palette index 4 looks like, and getting that wrong paints the padding a colour
that appears nowhere else on screen. `Theme::INHERIT` still emits nothing and so has nothing to undo.

### Four things the apps kept writing for themselves

Two apps outside this repo are built on conui — `chatmend`, which repairs Copilot chat sessions, and
`dash`, a car dashboard that draws a live map in pixels. Reading both against the library found four
places where the app had written something conui should have had, and the evidence for each is the same
shape: it was written more than once, and the copies had drifted.

**`conui::headless::Screen`.** A frame needs nothing but a `Buffer`, so a whole screen is renderable
with no terminal — eight lines to make the buffer, wrap it, draw, and read the rows back trimmed. Both
apps had written those eight lines twice each, once in their tests and once in the `--dump` flag that
prints a frame to stdout, and all four copies differed on whether rows are trimmed and whether the last
one carries a newline. `Screen::new(w, h)`, `.view(&view)` or `.draw(|frame| ..)`, then `text()`,
`rows()`, `row(y)`, `contains`, `find`, `buffer()`, `cursor()`, `resize()`, and `Display` for the form a
`--dump` prints. `assert_shows` and `assert_hides` panic with the whole screen in the message, which is
the thing you actually want when a layout assertion fails. Not behind `cfg(test)`, because a `--dump`
path ships.

**`conui::clip` and `conui::wrap`.** Both were already here and both were private — `clip` in `canvas`
and `wrap` inside `widget`. Three apps had written their own `clip` against `char` counts, which is
wrong for anything east of Greece and for every emoji, while the grapheme-aware one sat unreachable. A
caller wants the *text* at least as often as it wants it drawn: a table cell, a panel title, a label
being measured before anything is placed.

**`conui_term::cell_pixels`.** `window_size` has always read the terminal's `winsize` and thrown away
`ws_xpixel` and `ws_ypixel`, so an app that needed them — anything rasterising an image for a
graphics protocol — had no way to ask and wrote a raw `unsafe extern "C" ioctl` with a per-OS request
constant instead. Now `Terminal::window_pixels`/`cell_pixels` and free functions of the same names for
the common case of needing the answer before there is a screen. `None` rather than a plausible default:
Windows cannot answer, ssh and multiplexers usually lose it, and a caller only asks because it is about
to commit pixels to a decision. The cell size is clamped to a range a font could plausibly have, since
`None` has an obvious fallback and a 2x400 cell does not.

**`widget::Reading` and `Hints::trailing`.** `dash` imports zero widgets from conui and `chatmend`
imports twelve, which locates the gap precisely: forms and tables are served, instruments are not. A
`Reading` is `Field`'s three-column sibling — `LEFT  24.8 km  via A12` — with the columns fixed so a
stack of them is read down the value column, and one step of degradation that drops the note and sends
the value to the right edge. `Hints::trailing` puts a status on the right of the footer and takes the
room out of the legend *before* it is filled, which a caller drawing the status afterwards cannot do;
both apps had written the same reserve-then-place loop by hand.

What was assessed and deliberately left out: a pixel-graphics layer (`Image`, backend selection,
damage tracking) and a software rasteriser. The protocol knowledge is genuinely terminal knowledge and
belongs here eventually; a 2D rasteriser is an unbounded surface that contradicts a four-dependency
library, and `tiny-skia` already exists.

### Four examples, each of which tests itself

`snake`, `todo`, `settings` and `monitor` — the last a process monitor that reads a real machine on
macOS, Linux and Windows using nothing but `std`. Every one takes `--dump WIDTH HEIGHT` to render a
frame into a buffer and print it as text, which is how the screens in the README are kept honest.

### Known limits

- **The Windows console backend has never been driven interactively.** It compiles and the whole suite
  passes on `windows-latest`, but no headless test can reach the part that matters: that raw mode, VT
  mode and the escape output behave in conhost, Windows Terminal and PowerShell. Same for Linux, which
  is exercised in CI but not yet sat in front of. [`docs/terminal-handshake.md`](docs/terminal-handshake.md)
  is the checklist for closing that gap.
- `App::set_mouse` is the one public method no test or example exercises, for the same reason — it is
  covered by hand in the handshake checklist instead.
