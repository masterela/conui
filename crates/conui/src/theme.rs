//! A small, fixed palette.
//!
//! Terminal UIs look cheap when they reach for all sixteen colours at once. The discipline
//! that makes a console app look designed is a palette of about eight roles on a near-black
//! ground, used consistently: one accent for "good", one for "attention", one for "bad", and
//! three levels of grey for hierarchy. Everything else is the absence of colour.
//!
//! Widgets never name a colour. They name a [`Role`], and the theme decides. That is what lets
//! one line — swapping the [`Theme`] — restyle a whole application, and what lets a monochrome
//! terminal fall back to attributes without every call site knowing.

use conui_cell::{Color, Style};

/// What a colour is *for*, rather than what it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Role {
    /// Body text. The default.
    #[default]
    Text,
    /// Labels, units, secondary readings: present but not competing for attention.
    Muted,
    /// Rules, frames, empty track: structure the eye should skip.
    Dim,
    /// The one colour that means "this is the thing". Use sparingly.
    Accent,
    /// Something the user should look at soon.
    Warn,
    /// Something that is wrong now.
    Danger,
    /// Neutral emphasis, for a second data series that must not read as an alert.
    Info,
    /// Fills behind content, such as a board mesh or an inactive panel.
    Surface,
    /// The window ground.
    Background,
}

/// A palette plus the rules for using it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Theme {
    /// The window ground, on every cell, so a cleared region is the app's colour and not the
    /// terminal's.
    pub background: Color,
    /// Body text.
    pub text: Color,
    /// Labels and secondary readings.
    pub muted: Color,
    /// Rules, frames and empty track.
    pub dim: Color,
    /// The one colour that means "this is the thing".
    pub accent: Color,
    /// Something the user should look at soon.
    pub warn: Color,
    /// Something that is wrong now.
    pub danger: Color,
    /// Neutral emphasis, for data that is not an alert.
    pub info: Color,
    /// Fills behind content: a board mesh, an inactive panel, a focused button.
    pub surface: Color,
}

impl Theme {
    /// The palette this toolkit was designed around: mint on deep petrol-black.
    ///
    /// Note how little saturation it spends. The accent reads as bright only because
    /// everything near it is a desaturated teal-grey, and the ground is not pure black — a
    /// slight blue cast makes the whites look intentional rather than blown out.
    pub const LAYA: Self = Self {
        background: Color::hex("#090f13"),
        text: Color::hex("#e3f3ef"),
        muted: Color::hex("#68868c"),
        dim: Color::hex("#20353c"),
        accent: Color::hex("#62f5b5"),
        warn: Color::hex("#ffce73"),
        danger: Color::hex("#ff7c8c"),
        info: Color::hex("#8ad8e9"),
        surface: Color::hex("#13272e"),
    };

    /// The same structure in warm amber, for a terminal that already has a blue-heavy theme.
    pub const EMBER: Self = Self {
        background: Color::hex("#14100c"),
        text: Color::hex("#f6ece0"),
        muted: Color::hex("#8f7a63"),
        dim: Color::hex("#3a2c20"),
        accent: Color::hex("#ffb454"),
        warn: Color::hex("#ffd980"),
        danger: Color::hex("#f2707a"),
        info: Color::hex("#8fc7b8"),
        surface: Color::hex("#231a12"),
    };

    /// Inherit the terminal's own colours: no background paint, default foreground.
    ///
    /// The polite choice for a tool that runs inside someone else's carefully themed terminal,
    /// and the only honest choice for output that may be piped.
    pub const INHERIT: Self = Self {
        background: Color::Reset,
        text: Color::Reset,
        muted: Color::BRIGHT_BLACK,
        dim: Color::BRIGHT_BLACK,
        accent: Color::BRIGHT_GREEN,
        warn: Color::BRIGHT_YELLOW,
        danger: Color::BRIGHT_RED,
        info: Color::BRIGHT_CYAN,
        surface: Color::Reset,
    };

    /// The colour for a role.
    pub const fn color(&self, role: Role) -> Color {
        match role {
            Role::Text => self.text,
            Role::Muted => self.muted,
            Role::Dim => self.dim,
            Role::Accent => self.accent,
            Role::Warn => self.warn,
            Role::Danger => self.danger,
            Role::Info => self.info,
            Role::Surface => self.surface,
            Role::Background => self.background,
        }
    }

    /// Foreground for a role over the theme background: the style most widgets want.
    pub const fn style(&self, role: Role) -> Style {
        Style::EMPTY.fg(self.color(role)).bg(self.background)
    }

    /// The style every cell starts as, and what a cleared region is filled with.
    pub const fn ground(&self) -> Style {
        self.style(Role::Text)
    }

    /// Pick `Warn` below `threshold` and `Danger` at or above it.
    ///
    /// A convenience for the very common "this gauge should change colour when it gets bad"
    /// case, so that the threshold lives in one place instead of at every call site.
    pub const fn escalate(&self, value: f32, threshold: f32) -> Role {
        if value < threshold { Role::Warn } else { Role::Danger }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::LAYA
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_resolves() {
        let theme = Theme::LAYA;
        for role in [
            Role::Text,
            Role::Muted,
            Role::Dim,
            Role::Accent,
            Role::Warn,
            Role::Danger,
            Role::Info,
            Role::Surface,
            Role::Background,
        ] {
            assert_ne!(theme.color(role), Color::Reset, "{role:?} must be defined");
        }
    }

    #[test]
    fn the_hierarchy_greys_are_actually_ordered() {
        // If `dim` were brighter than `muted`, structure would compete with labels and the
        // whole visual hierarchy would invert. Worth asserting rather than eyeballing.
        let theme = Theme::LAYA;
        assert!(theme.text.luminance() > theme.muted.luminance());
        assert!(theme.muted.luminance() > theme.dim.luminance());
        assert!(theme.dim.luminance() > theme.surface.luminance());
        assert!(theme.surface.luminance() > theme.background.luminance());
    }

    #[test]
    fn text_is_readable_on_the_background() {
        // WCAG AA for body text is 4.5:1. A terminal theme that misses this is unusable in
        // daylight, which is a design bug the type system cannot catch.
        for theme in [Theme::LAYA, Theme::EMBER] {
            let ratio = theme.text.contrast_ratio(theme.background).expect("RGB palette");
            assert!(ratio >= 4.5, "text contrast was only {ratio:.1}:1");
        }
    }

    #[test]
    fn accents_stay_distinguishable_from_the_background() {
        for theme in [Theme::LAYA, Theme::EMBER] {
            for role in [Role::Accent, Role::Warn, Role::Danger, Role::Info] {
                let ratio =
                    theme.color(role).contrast_ratio(theme.background).expect("RGB palette");
                assert!(ratio >= 3.0, "{role:?} contrast was only {ratio:.1}:1");
            }
        }
    }

    #[test]
    fn escalate_switches_at_the_threshold() {
        let theme = Theme::LAYA;
        assert_eq!(theme.escalate(0.4, 0.5), Role::Warn);
        assert_eq!(theme.escalate(0.5, 0.5), Role::Danger);
    }
}
