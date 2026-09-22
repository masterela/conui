//! Colors, written the way a designer hands them over: `Color::hex("#62f5b5")`.
//!
//! A [`Color`] keeps full 24-bit precision all the way to the writer. Terminals that cannot
//! do truecolor get an approximation at the last possible moment, via [`Color::degrade`], so
//! the palette a theme declares stays the single source of truth.

/// How much color the output terminal can actually render.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ColorDepth {
    /// No color at all; everything degrades to [`Color::Reset`] and attributes carry the design.
    NoColor,
    /// The original eight, plus their bright variants.
    Ansi16,
    /// The xterm 256-color cube and grayscale ramp.
    Indexed256,
    /// Full 24-bit RGB. What we assume on a modern terminal.
    #[default]
    TrueColor,
}

/// A foreground or background color.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Color {
    /// The terminal's own default. Distinct from black: it is whatever the user configured.
    #[default]
    Reset,
    /// One of the 16 ANSI colors, `0..=15`. Follows the user's terminal theme.
    Ansi(u8),
    /// An xterm 256-palette index.
    Indexed(u8),
    /// A literal 24-bit color.
    Rgb(u8, u8, u8),
}

impl Color {
    pub const BLACK: Self = Self::Ansi(0);
    pub const RED: Self = Self::Ansi(1);
    pub const GREEN: Self = Self::Ansi(2);
    pub const YELLOW: Self = Self::Ansi(3);
    pub const BLUE: Self = Self::Ansi(4);
    pub const MAGENTA: Self = Self::Ansi(5);
    pub const CYAN: Self = Self::Ansi(6);
    pub const WHITE: Self = Self::Ansi(7);
    pub const BRIGHT_BLACK: Self = Self::Ansi(8);
    pub const BRIGHT_RED: Self = Self::Ansi(9);
    pub const BRIGHT_GREEN: Self = Self::Ansi(10);
    pub const BRIGHT_YELLOW: Self = Self::Ansi(11);
    pub const BRIGHT_BLUE: Self = Self::Ansi(12);
    pub const BRIGHT_MAGENTA: Self = Self::Ansi(13);
    pub const BRIGHT_CYAN: Self = Self::Ansi(14);
    pub const BRIGHT_WHITE: Self = Self::Ansi(15);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::Rgb(r, g, b)
    }

    /// Parse `#rgb`, `#rrggbb`, `rgb` or `rrggbb`.
    ///
    /// Usable in a `const` so a theme's palette is validated at compile time: a malformed
    /// literal fails the build rather than silently rendering as black.
    ///
    /// ```
    /// # use conui_cell::Color;
    /// const ACCENT: Color = Color::hex("#62f5b5");
    /// assert_eq!(ACCENT, Color::Rgb(0x62, 0xf5, 0xb5));
    /// assert_eq!(Color::hex("#0bc"), Color::Rgb(0x00, 0xbb, 0xcc));
    /// ```
    pub const fn hex(text: &str) -> Self {
        let bytes = text.as_bytes();
        let body = if !bytes.is_empty() && bytes[0] == b'#' { bytes.split_at(1).1 } else { bytes };
        match body.len() {
            // #rgb shorthand: each nibble is doubled, so `f` means `ff`.
            3 => {
                let r = hex_digit(body[0]);
                let g = hex_digit(body[1]);
                let b = hex_digit(body[2]);
                Self::Rgb(r * 17, g * 17, b * 17)
            }
            6 => Self::Rgb(
                hex_digit(body[0]) * 16 + hex_digit(body[1]),
                hex_digit(body[2]) * 16 + hex_digit(body[3]),
                hex_digit(body[4]) * 16 + hex_digit(body[5]),
            ),
            _ => panic!("color hex must be 3 or 6 digits, optionally prefixed with '#'"),
        }
    }

    /// Mix toward `other`, where `t` of `0.0` is `self` and `1.0` is `other`.
    ///
    /// Only defined between two [`Color::Rgb`] values; anything else has no numeric
    /// meaning to interpolate, so the endpoint nearest `t` is returned unchanged. This is
    /// what draws the snake's tail gradient and any sequential scale.
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        match (self, other) {
            (Self::Rgb(r1, g1, b1), Self::Rgb(r2, g2, b2)) => {
                Self::Rgb(lerp_channel(r1, r2, t), lerp_channel(g1, g2, t), lerp_channel(b1, b2, t))
            }
            _ if t < 0.5 => self,
            _ => other,
        }
    }

    /// Scale a color toward black, keeping hue. `factor` above 1.0 brightens.
    pub fn scale(self, factor: f32) -> Self {
        match self {
            Self::Rgb(r, g, b) => Self::Rgb(
                scale_channel(r, factor),
                scale_channel(g, factor),
                scale_channel(b, factor),
            ),
            other => other,
        }
    }

    /// Composite `self` over `backdrop` at `alpha`.
    ///
    /// Terminals have no alpha channel, so translucency has to be resolved against a known
    /// backdrop before it reaches a cell. This is how a dim overlay or a selection wash works.
    pub fn over(self, backdrop: Self, alpha: f32) -> Self {
        backdrop.lerp(self, alpha)
    }

    /// Relative luminance per WCAG, in `0.0..=1.0`. Non-RGB colors are unknowable here and
    /// report `None`, since their appearance depends on the user's terminal theme.
    pub fn luminance(self) -> Option<f32> {
        let Self::Rgb(r, g, b) = self else { return None };
        fn channel(value: u8) -> f32 {
            let v = value as f32 / 255.0;
            if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        }
        Some(0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b))
    }

    /// WCAG contrast ratio against `other`, from `1.0` (identical) to `21.0` (black on white).
    /// `None` when either side is theme-dependent.
    pub fn contrast_ratio(self, other: Self) -> Option<f32> {
        let a = self.luminance()?;
        let b = other.luminance()?;
        let (lighter, darker) = if a >= b { (a, b) } else { (b, a) };
        Some((lighter + 0.05) / (darker + 0.05))
    }

    /// Approximate this color within what `depth` can express.
    ///
    /// Called by the writer, not by application code: a widget always states the color it
    /// means and lets the terminal's capability decide the rendering.
    pub fn degrade(self, depth: ColorDepth) -> Self {
        match depth {
            ColorDepth::TrueColor => self,
            ColorDepth::NoColor => Self::Reset,
            ColorDepth::Indexed256 => match self {
                Self::Rgb(r, g, b) => Self::Indexed(rgb_to_xterm256(r, g, b)),
                other => other,
            },
            ColorDepth::Ansi16 => match self {
                Self::Rgb(r, g, b) => Self::Ansi(rgb_to_ansi16(r, g, b)),
                Self::Indexed(index) => {
                    let (r, g, b) = xterm256_to_rgb(index);
                    Self::Ansi(rgb_to_ansi16(r, g, b))
                }
                other => other,
            },
        }
    }
}

const fn hex_digit(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        b'A'..=b'F' => byte - b'A' + 10,
        _ => panic!("invalid hex digit in color literal"),
    }
}

fn lerp_channel(from: u8, to: u8, t: f32) -> u8 {
    let value = from as f32 + (to as f32 - from as f32) * t;
    value.round().clamp(0.0, 255.0) as u8
}

fn scale_channel(value: u8, factor: f32) -> u8 {
    (value as f32 * factor).round().clamp(0.0, 255.0) as u8
}

/// The 6 levels of the xterm color cube, as actual rendered intensities.
const CUBE_LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];

fn nearest_cube_level(value: u8) -> usize {
    let mut best = 0;
    let mut best_delta = u16::MAX;
    for (index, level) in CUBE_LEVELS.iter().enumerate() {
        let delta = value.abs_diff(*level) as u16;
        if delta < best_delta {
            best_delta = delta;
            best = index;
        }
    }
    best
}

/// Map a 24-bit color onto the xterm 256 palette, choosing between the 6x6x6 cube and the
/// 24-step gray ramp by whichever lands closer.
pub fn rgb_to_xterm256(r: u8, g: u8, b: u8) -> u8 {
    let (ri, gi, bi) = (nearest_cube_level(r), nearest_cube_level(g), nearest_cube_level(b));
    let cube_index = 16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8;
    let cube_error = squared_error((r, g, b), (CUBE_LEVELS[ri], CUBE_LEVELS[gi], CUBE_LEVELS[bi]));

    // Gray ramp 232..=255 runs 8, 18, ... 238 in steps of 10.
    let average = (r as u32 + g as u32 + b as u32) / 3;
    let step = ((average as i32 - 8).clamp(0, 238) as f32 / 10.0).round().clamp(0.0, 23.0) as u8;
    let gray_value = 8 + step * 10;
    let gray_error = squared_error((r, g, b), (gray_value, gray_value, gray_value));

    if gray_error < cube_error { 232 + step } else { cube_index }
}

/// Inverse of [`rgb_to_xterm256`] for the cube and ramp; the first 16 use the standard
/// xterm defaults, which is the best we can do without querying the terminal.
pub fn xterm256_to_rgb(index: u8) -> (u8, u8, u8) {
    const BASE16: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (128, 0, 0),
        (0, 128, 0),
        (128, 128, 0),
        (0, 0, 128),
        (128, 0, 128),
        (0, 128, 128),
        (192, 192, 192),
        (128, 128, 128),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (0, 0, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    match index {
        0..=15 => BASE16[index as usize],
        16..=231 => {
            let offset = index - 16;
            let r = CUBE_LEVELS[(offset / 36) as usize];
            let g = CUBE_LEVELS[((offset % 36) / 6) as usize];
            let b = CUBE_LEVELS[(offset % 6) as usize];
            (r, g, b)
        }
        _ => {
            let value = 8 + (index - 232) * 10;
            (value, value, value)
        }
    }
}

fn squared_error(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let dr = a.0.abs_diff(b.0) as u32;
    let dg = a.1.abs_diff(b.1) as u32;
    let db = a.2.abs_diff(b.2) as u32;
    dr * dr + dg * dg + db * db
}

/// Collapse to the 16 ANSI slots, via hue/saturation/value rather than per-channel bits.
///
/// Decomposing first is what makes the fallback legible. A naive "is this channel above
/// half the maximum" test lights up two bits for any slightly desaturated color, so a mint
/// accent lands on cyan and a grey-teal lands on cyan as well — the palette loses both its
/// hue and its hierarchy. Working in HSV lets three separate judgements be made:
///
/// - **Saturation** decides chromatic versus neutral, so muted greys stay grey.
/// - **Value** picks the normal or bright variant, and sends near-black to black.
/// - **Hue** picks the color family, and only then.
pub fn rgb_to_ansi16(r: u8, g: u8, b: u8) -> u8 {
    let (hue, saturation, value) = rgb_to_hsv(r, g, b);

    if value < 0.10 {
        return 0;
    }
    // Neutral: render on the black/white axis. Mapping these onto a hue would invent a
    // color the design never asked for, which is far more jarring than losing the tint.
    if saturation < 0.30 {
        return match value {
            v if v < 0.35 => 0,
            v if v < 0.65 => 8,
            v if v < 0.90 => 7,
            _ => 15,
        };
    }
    // Dark but chromatic: these are the rules, gutters and hairlines of a dark theme, and
    // they want to read as shadow rather than as a saturated blue.
    if value < 0.28 {
        return 8;
    }

    // Sectors are deliberately uneven. Green is widened because the perceptual span from
    // spring-green to green is wide, and a 150 degree cutoff would throw mint into cyan.
    let base = match hue {
        h if h < 25.0 => 1,  // red
        h if h < 75.0 => 3,  // yellow
        h if h < 165.0 => 2, // green
        h if h < 200.0 => 6, // cyan
        h if h < 260.0 => 4, // blue
        h if h < 345.0 => 5, // magenta
        _ => 1,              // red, wrapping past 345
    };
    if value > 0.66 { base + 8 } else { base }
}

/// Hue in degrees `0.0..360.0`, saturation and value in `0.0..=1.0`.
fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (rf, gf, bf) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let delta = max - min;

    let saturation = if max <= 0.0 { 0.0 } else { delta / max };
    if delta <= 0.0 {
        return (0.0, saturation, max);
    }
    let hue = if max == rf {
        60.0 * (((gf - bf) / delta).rem_euclid(6.0))
    } else if max == gf {
        60.0 * ((bf - rf) / delta + 2.0)
    } else {
        60.0 * ((rf - gf) / delta + 4.0)
    };
    (hue.rem_euclid(360.0), saturation, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_accepts_both_lengths_and_optional_hash() {
        assert_eq!(Color::hex("#090f13"), Color::Rgb(9, 15, 19));
        assert_eq!(Color::hex("090f13"), Color::Rgb(9, 15, 19));
        assert_eq!(Color::hex("#FFF"), Color::Rgb(255, 255, 255));
        assert_eq!(Color::hex("f0a"), Color::Rgb(255, 0, 170));
    }

    #[test]
    fn lerp_hits_both_endpoints_exactly() {
        let a = Color::hex("#000000");
        let b = Color::hex("#ffffff");
        assert_eq!(a.lerp(b, 0.0), a);
        assert_eq!(a.lerp(b, 1.0), b);
        assert_eq!(a.lerp(b, 0.5), Color::Rgb(128, 128, 128));
    }

    #[test]
    fn lerp_clamps_out_of_range_t() {
        let a = Color::hex("#000000");
        let b = Color::hex("#ffffff");
        assert_eq!(a.lerp(b, -3.0), a);
        assert_eq!(a.lerp(b, 9.0), b);
    }

    #[test]
    fn lerp_of_non_rgb_picks_the_nearer_endpoint() {
        assert_eq!(Color::RED.lerp(Color::BLUE, 0.1), Color::RED);
        assert_eq!(Color::RED.lerp(Color::BLUE, 0.9), Color::BLUE);
    }

    #[test]
    fn degrade_to_truecolor_is_identity() {
        let color = Color::hex("#62f5b5");
        assert_eq!(color.degrade(ColorDepth::TrueColor), color);
    }

    #[test]
    fn degrade_to_nocolor_erases_everything() {
        assert_eq!(Color::hex("#62f5b5").degrade(ColorDepth::NoColor), Color::Reset);
        assert_eq!(Color::RED.degrade(ColorDepth::NoColor), Color::Reset);
    }

    #[test]
    fn xterm256_roundtrips_palette_colors() {
        // Any color that is already exactly on the cube must survive the round trip.
        for index in 16u8..=255 {
            let (r, g, b) = xterm256_to_rgb(index);
            assert_eq!(rgb_to_xterm256(r, g, b), index, "index {index} did not round trip");
        }
    }

    #[test]
    fn xterm256_picks_gray_ramp_for_neutral_tones() {
        // #808080 is much closer to the gray ramp than to any cube level.
        let index = rgb_to_xterm256(0x80, 0x80, 0x80);
        assert!((232..=255).contains(&index), "expected gray ramp, got {index}");
    }

    #[test]
    fn ansi16_maps_the_primaries_to_their_own_slots() {
        assert_eq!(rgb_to_ansi16(0x00, 0x00, 0x00), 0);
        assert_eq!(rgb_to_ansi16(0xff, 0x00, 0x00), 9);
        assert_eq!(rgb_to_ansi16(0x00, 0xff, 0x00), 10);
        assert_eq!(rgb_to_ansi16(0xff, 0xff, 0x00), 11);
        assert_eq!(rgb_to_ansi16(0x00, 0x00, 0xff), 12);
        assert_eq!(rgb_to_ansi16(0xff, 0x00, 0xff), 13);
        assert_eq!(rgb_to_ansi16(0x00, 0xff, 0xff), 14);
        assert_eq!(rgb_to_ansi16(0xff, 0xff, 0xff), 15);
    }

    #[test]
    fn ansi16_keeps_the_hue_family_of_the_default_palette() {
        assert_eq!(rgb_to_ansi16(0x62, 0xf5, 0xb5), 10, "mint accent should stay green");
        assert_eq!(rgb_to_ansi16(0xff, 0x7c, 0x8c), 9, "salmon should stay red");
        assert_eq!(rgb_to_ansi16(0xff, 0xce, 0x73), 11, "amber should stay yellow");
        assert_eq!(rgb_to_ansi16(0x8a, 0xd8, 0xe9), 14, "sky should stay cyan");
    }

    #[test]
    fn ansi16_sends_neutral_tones_to_the_grey_axis() {
        // A muted grey-teal must not acquire a hue it never had.
        assert_eq!(rgb_to_ansi16(0x68, 0x86, 0x8c), 8, "muted should read as dim grey");
        assert_eq!(rgb_to_ansi16(0xe3, 0xf3, 0xef), 15, "foreground should read as white");
        assert_eq!(rgb_to_ansi16(0x80, 0x80, 0x80), 8);
    }

    #[test]
    fn ansi16_sends_near_black_and_dark_rules_to_shadow() {
        assert_eq!(rgb_to_ansi16(0x09, 0x0f, 0x13), 0, "app background is effectively black");
        assert_eq!(rgb_to_ansi16(0x20, 0x35, 0x3c), 8, "hairline rule should read as shadow");
    }

    #[test]
    fn ansi16_distinguishes_bright_from_normal_by_value() {
        // Same hue, different value: the fallback must preserve the light/dark relationship.
        let dark = rgb_to_ansi16(0x00, 0x60, 0x00);
        let light = rgb_to_ansi16(0x00, 0xe0, 0x00);
        assert_eq!(dark, 2);
        assert_eq!(light, 10);
    }

    #[test]
    fn hsv_roundtrips_the_hue_of_the_primaries() {
        assert_eq!(rgb_to_hsv(255, 0, 0).0, 0.0);
        assert_eq!(rgb_to_hsv(0, 255, 0).0, 120.0);
        assert_eq!(rgb_to_hsv(0, 0, 255).0, 240.0);
        let (_, saturation, value) = rgb_to_hsv(128, 128, 128);
        assert_eq!(saturation, 0.0, "grey has no saturation");
        assert!((value - 0.502).abs() < 0.01);
    }

    #[test]
    fn contrast_ratio_matches_wcag_extremes() {
        let ratio = Color::hex("#000").contrast_ratio(Color::hex("#fff")).unwrap();
        assert!((ratio - 21.0).abs() < 0.01, "black on white should be 21:1, got {ratio}");
        let same = Color::hex("#62f5b5").contrast_ratio(Color::hex("#62f5b5")).unwrap();
        assert!((same - 1.0).abs() < 0.001);
    }

    #[test]
    fn theme_foreground_clears_wcag_aa_on_its_background() {
        // Guards the default palette: body text must stay readable on the app background.
        let ratio = Color::hex("#e3f3ef").contrast_ratio(Color::hex("#090f13")).unwrap();
        assert!(ratio >= 4.5, "foreground/background contrast is only {ratio}");
    }

    #[test]
    fn over_resolves_translucency_against_a_backdrop() {
        let white = Color::hex("#ffffff");
        let black = Color::hex("#000000");
        assert_eq!(white.over(black, 0.0), black);
        assert_eq!(white.over(black, 1.0), white);
    }
}
