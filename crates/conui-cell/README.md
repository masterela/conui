# conui-cell

The cell layer of [conui](https://github.com/masterela/conui): geometry, color, style, and the grid
a frame is rendered into.

This crate knows nothing about terminals. It is a pure, testable model of "what should be on
screen", which is why every test in it runs without a TTY. Two ideas are worth knowing:

- A cell holds a **grapheme cluster**, not a `char`, and knows its display **width**. Emoji and CJK
  take two columns, and getting that wrong shears every subsequent column of the frame.
- A frame is **diffed**, not redrawn. `Buffer::diff` yields only the cells that changed, so a
  full-screen app at 60 fps sends a few hundred bytes per frame instead of tens of thousands, and
  never blanks the screen in between.

```rust
use conui_cell::{Buffer, Color, Style};

let mut frame = Buffer::new(20, 1);
frame.set_str(0, 0, "score 42", Style::new().fg(Color::hex("#62f5b5")), 20);

// What actually changed, relative to what is already on screen.
let patches = frame.diff(&Buffer::new(20, 1));
assert_eq!(patches.len(), 8);
assert_eq!(frame.row_text(0).trim_end(), "score 42");
```

`conui-term` turns a `Buffer` into bytes; `conui` provides the drawing, layout and widget API on
top. You can also use this crate on its own, with your own writer.

## License

MIT OR Apache-2.0, at your option.
