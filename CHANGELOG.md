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
