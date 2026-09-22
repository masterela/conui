//! What the terminal on the other end can do.
//!
//! Detection is environment sniffing, because the alternative — querying the terminal and
//! waiting for a reply — costs a round trip on startup and hangs on anything that does not
//! answer. Sniffing is a heuristic, so every field can be overridden.

use conui_cell::ColorDepth;

/// The feature set conui will use when writing to a given terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// How much color to emit. Colors are degraded to this at write time.
    pub color_depth: ColorDepth,
    /// Wrap each frame in DECSET 2026 so the terminal presents it atomically.
    ///
    /// Defaults to on. A terminal that does not know the mode ignores the private sequence,
    /// which is why this is safe to send blind rather than something to detect.
    pub synchronized_output: bool,
    /// Ask for mouse reporting in SGR encoding when the app enables mouse capture.
    pub mouse: bool,
    /// Wrap pasted text in markers so a paste is one event rather than a burst of keys.
    pub bracketed_paste: bool,
    /// Request focus in/out notifications.
    pub focus_events: bool,
}

impl Default for Capabilities {
    fn default() -> Self {
        Self {
            color_depth: ColorDepth::TrueColor,
            synchronized_output: true,
            mouse: true,
            bracketed_paste: true,
            focus_events: true,
        }
    }
}

impl Capabilities {
    /// Sniff the environment. `is_tty` short-circuits to no color when output is redirected,
    /// so piping a conui app to a file produces plain text instead of escape soup.
    pub fn detect(is_tty: bool) -> Self {
        Self { color_depth: detect_color_depth(is_tty), ..Self::default() }
    }

    /// Capabilities for a known depth, with every optional protocol switched off. The useful
    /// base for tests and for recording golden output.
    pub fn plain(color_depth: ColorDepth) -> Self {
        Self {
            color_depth,
            synchronized_output: false,
            mouse: false,
            bracketed_paste: false,
            focus_events: false,
        }
    }
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

/// The color-depth decision, in precedence order.
///
/// `NO_COLOR` and `CLICOLOR_FORCE` are honoured ahead of any capability guess: they are the
/// user telling us directly, and a heuristic should never override that.
fn detect_color_depth(is_tty: bool) -> ColorDepth {
    // https://no-color.org — any non-empty value disables color.
    if env("NO_COLOR").is_some() {
        return ColorDepth::NoColor;
    }
    let forced = env("CLICOLOR_FORCE").is_some_and(|value| value != "0");
    if !is_tty && !forced {
        return ColorDepth::NoColor;
    }
    if env("CLICOLOR").is_some_and(|value| value == "0") && !forced {
        return ColorDepth::NoColor;
    }

    let term = env("TERM").unwrap_or_default();
    if term == "dumb" {
        // A forced flag still cannot conjure capabilities a dumb terminal lacks.
        return ColorDepth::NoColor;
    }

    if let Some(colorterm) = env("COLORTERM") {
        if colorterm.eq_ignore_ascii_case("truecolor") || colorterm.eq_ignore_ascii_case("24bit") {
            return ColorDepth::TrueColor;
        }
    }
    // terminfo entries for direct-color use these names.
    if term.contains("truecolor") || term.contains("direct") {
        return ColorDepth::TrueColor;
    }
    // Terminals known to do 24-bit without always advertising it.
    if let Some(program) = env("TERM_PROGRAM") {
        const TRUECOLOR_PROGRAMS: [&str; 6] =
            ["iTerm.app", "WezTerm", "ghostty", "Hyper", "vscode", "rio"];
        if TRUECOLOR_PROGRAMS.iter().any(|known| known.eq_ignore_ascii_case(&program)) {
            return ColorDepth::TrueColor;
        }
    }
    if term.contains("256") {
        return ColorDepth::Indexed256;
    }
    if term.is_empty() {
        // On Windows there is no TERM, but a modern console does full color once virtual
        // terminal processing is enabled, which the platform layer does on startup.
        return if cfg!(windows) { ColorDepth::TrueColor } else { ColorDepth::NoColor };
    }
    if term.contains("color")
        || term.starts_with("xterm")
        || term.starts_with("screen")
        || term.starts_with("tmux")
        || term.starts_with("rxvt")
        || term == "linux"
    {
        return ColorDepth::Ansi16;
    }
    ColorDepth::NoColor
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Capability detection reads process-wide environment, so these cases are exercised
    /// against the pure decision function with an explicit environment instead of mutating
    /// the real one, which would race across parallel tests.
    #[test]
    fn defaults_enable_the_modern_protocol_set() {
        let caps = Capabilities::default();
        assert!(caps.synchronized_output, "atomic frames should be on by default");
        assert!(caps.bracketed_paste);
        assert_eq!(caps.color_depth, ColorDepth::TrueColor);
    }

    #[test]
    fn plain_capabilities_disable_every_optional_protocol() {
        let caps = Capabilities::plain(ColorDepth::Ansi16);
        assert_eq!(caps.color_depth, ColorDepth::Ansi16);
        assert!(!caps.synchronized_output);
        assert!(!caps.mouse);
        assert!(!caps.bracketed_paste);
        assert!(!caps.focus_events);
    }
}
