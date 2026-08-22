//! Spacing, radius and shell dimensions — expressed in logical pixels.
//!
//! Physical pixels are the renderer's problem. Everything here is scale-independent
//! so the same tokens hold on a 1x laptop panel and a 2x HiDPI display.

/// Spacing rhythm on a 4px base.
///
/// Rule of thumb from the design system: 4–8 inside a related group, 16–24
/// between independent sections, 12–16 for surface padding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spacing;

impl Spacing {
    pub const BASE: f32 = 4.0;

    pub const S1: f32 = 4.0;
    pub const S2: f32 = 8.0;
    pub const S3: f32 = 12.0;
    pub const S4: f32 = 16.0;
    pub const S5: f32 = 20.0;
    pub const S6: f32 = 24.0;
    pub const S8: f32 = 32.0;
    pub const S10: f32 = 40.0;

    /// Nth step of the rhythm. Prefer the named constants; this exists for
    /// generated layouts and for validating imported presets.
    pub const fn step(n: u32) -> f32 {
        Self::BASE * n as f32
    }
}

/// Corner radii. Controlled and slightly rounded — never fully pill-shaped
/// except where a control is genuinely circular.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Radius;

impl Radius {
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 10.0;
    pub const LG: f32 = 14.0;
    pub const XL: f32 = 20.0;
    pub const FULL: f32 = 9999.0;
}

/// Default dimensions of the Personal Agent Workspace shell.
///
/// These are defaults, not locks: every one of them is overridable through the
/// Panel Registry. The `*_MIN` / `*_MAX` values are the clamps that a user
/// preset — or a hand-edited config file — cannot escape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Shell;

impl Shell {
    pub const TOP_BAR: f32 = 56.0;
    pub const TAB_BAR: f32 = 44.0;
    pub const PERFORMANCE_STRIP: f32 = 40.0;

    pub const SIDEBAR_FULL: f32 = 212.0;
    pub const SIDEBAR_COMPACT: f32 = 160.0;
    pub const SIDEBAR_ICON: f32 = 56.0;
    pub const SIDEBAR_MIN: f32 = 56.0;
    pub const SIDEBAR_MAX: f32 = 420.0;

    pub const CONTEXT_FULL: f32 = 280.0;
    pub const CONTEXT_COMPACT: f32 = 220.0;
    pub const CONTEXT_SLIM: f32 = 56.0;
    pub const CONTEXT_MIN: f32 = 56.0;
    pub const CONTEXT_MAX: f32 = 480.0;

    /// Readable measure for chat. Chat Focus raises it; it is never unbounded,
    /// because a full-width line of prose is hostile to read.
    pub const CHAT_MEASURE: f32 = 760.0;
    pub const CHAT_MEASURE_MAX: f32 = 1100.0;

    /// Below this the shell must collapse panels by priority rather than clip them.
    pub const WINDOW_MIN_WIDTH: f32 = 720.0;
    pub const WINDOW_MIN_HEIGHT: f32 = 480.0;

    /// Smallest interactive target. Anything below this fails the visual QA pass.
    pub const MIN_HIT_TARGET: f32 = 28.0;
}

/// Discrete widths a side panel can take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SidebarMode {
    Full,
    Compact,
    IconOnly,
    Hidden,
}

impl SidebarMode {
    pub const fn width(self) -> f32 {
        match self {
            SidebarMode::Full => Shell::SIDEBAR_FULL,
            SidebarMode::Compact => Shell::SIDEBAR_COMPACT,
            SidebarMode::IconOnly => Shell::SIDEBAR_ICON,
            SidebarMode::Hidden => 0.0,
        }
    }
}

/// Discrete widths the right-hand Context Panel can take.
///
/// Hiding it changes visibility only — never the agent's context, the project
/// data, or any granted permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMode {
    Full,
    Compact,
    Slim,
    Hidden,
}

impl ContextMode {
    pub const fn width(self) -> f32 {
        match self {
            ContextMode::Full => Shell::CONTEXT_FULL,
            ContextMode::Compact => Shell::CONTEXT_COMPACT,
            ContextMode::Slim => Shell::CONTEXT_SLIM,
            ContextMode::Hidden => 0.0,
        }
    }
}

/// Clamp a requested panel width into the range the Panel Registry allows.
///
/// Applied in the data layer, not just in the drag handler, so a hand-edited
/// config cannot produce a negative or absurd width.
pub fn clamp_width(requested: f32, min: f32, max: f32) -> f32 {
    if !requested.is_finite() {
        return min;
    }
    requested.clamp(min, max)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spacing_steps_follow_the_four_pixel_base() {
        for value in [Spacing::S1, Spacing::S2, Spacing::S3, Spacing::S4, Spacing::S6, Spacing::S8]
        {
            assert_eq!(value % Spacing::BASE, 0.0, "{value} is off the rhythm");
        }
    }

    #[test]
    fn step_matches_the_named_constants() {
        assert_eq!(Spacing::step(1), Spacing::S1);
        assert_eq!(Spacing::step(6), Spacing::S6);
    }

    #[test]
    fn sidebar_modes_descend_in_width() {
        assert!(SidebarMode::Full.width() > SidebarMode::Compact.width());
        assert!(SidebarMode::Compact.width() > SidebarMode::IconOnly.width());
        assert_eq!(SidebarMode::Hidden.width(), 0.0);
    }

    #[test]
    fn context_modes_descend_in_width() {
        assert!(ContextMode::Full.width() > ContextMode::Compact.width());
        assert!(ContextMode::Compact.width() > ContextMode::Slim.width());
        assert_eq!(ContextMode::Hidden.width(), 0.0);
    }

    #[test]
    fn every_visible_mode_clears_the_minimum_hit_target() {
        for width in [SidebarMode::IconOnly.width(), ContextMode::Slim.width()] {
            assert!(width >= Shell::MIN_HIT_TARGET);
        }
    }

    #[test]
    fn hand_edited_widths_cannot_escape_the_clamp() {
        let min = Shell::SIDEBAR_MIN;
        let max = Shell::SIDEBAR_MAX;
        assert_eq!(clamp_width(-9000.0, min, max), min);
        assert_eq!(clamp_width(1e9, min, max), max);
        assert_eq!(clamp_width(f32::NAN, min, max), min);
        assert_eq!(clamp_width(200.0, min, max), 200.0);
    }
}
