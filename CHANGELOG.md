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
