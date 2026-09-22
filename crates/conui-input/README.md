# conui-input

Terminal input, decoded. The input layer of
[conui](https://github.com/masterela/conui).

The terminal hands an application a byte stream, not events. `Up` might be three bytes, `Ctrl+Left`
six, a pasted paragraph a few thousand, and any of them can arrive split across reads. This crate
turns those bytes into `Event` values, incrementally and without blocking: feed it whatever arrived,
take out whatever is now unambiguous.

```rust
use conui_input::{KeyCode, Modifiers, Parser};

let mut parser = Parser::new();
parser.feed(b"\x1b[1;5C");

let event = parser.next_event().unwrap();
let key = event.as_key().unwrap();
assert_eq!(key.code, KeyCode::Right);
assert!(key.modifiers.contains(Modifiers::CTRL));
```

Keys, mouse (SGR and the legacy X10 encoding), bracketed paste, focus change and the Kitty keyboard
protocol are all decoded here. No I/O and no dependencies — reading the bytes is `conui-term`'s job,
which makes this crate a pure function from bytes to events, and every one of its tests a table.

One ambiguity cannot be resolved from the bytes alone: `ESC` is both the Escape key and the first
byte of every escape sequence. The parser holds a trailing `ESC` rather than guessing, and the event
loop calls `Parser::flush_timeout` once input has been idle to settle it. That is the whole reason
Escape feels a few milliseconds slower than every other key in every terminal application ever
written.

## License

MIT OR Apache-2.0, at your option.
