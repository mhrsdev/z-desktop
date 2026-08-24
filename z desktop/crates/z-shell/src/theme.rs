//! Shell-level design tokens (theme-001).
//!
//! Role-named colours only. The raw palette and its approved values stay owned
//! by `z-tokens` (ADR-0022); `Tokens` is the small role surface shell chrome
//! consumes, mirroring the calm-dark charcoal ladder and the scarce coral
//! accent rather than inventing a second palette.

use z_tokens::Rgba;

/// Role-named colour tokens for shell chrome. Data, never code.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tokens {
    pub bg: Rgba,
    pub surface: Rgba,
    pub surface_elevated: Rgba,
    pub border: Rgba,
    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub text_muted: Rgba,
    pub accent: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub danger: Rgba,
}

impl Tokens {
    /// Calm dark default. Values match the approved `Theme::zero_dark()`
    /// palette so the two token layers cannot drift apart.
    pub const DARK: Self = Self {
        bg: Rgba::hex(0x151515),
        surface: Rgba::hex(0x1E1E1E),
        surface_elevated: Rgba::hex(0x20201F),
        // White at low alpha, as upstream borders are, so it reads over any tone.
        border: Rgba::new(255, 255, 255, 20),
        text_primary: Rgba::hex(0xC1C1C1),
        text_secondary: Rgba::hex(0xB9B9B9),
        text_muted: Rgba::hex(0x898781),
        accent: Rgba::hex(0xD87656),
        success: Rgba::hex(0x3FB950),
        warning: Rgba::hex(0xD87656), // upstream maps warning to the same warm coral
        danger: Rgba::hex(0xF85149),
    };
}

impl Default for Tokens {
    fn default() -> Self {
        Self::DARK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_roles_are_distinct_so_elevation_reads_from_tone() {
        let t = Tokens::DARK;
        assert_ne!(t.bg, t.surface);
        assert_ne!(t.surface, t.surface_elevated);
        assert_ne!(t.bg, t.surface_elevated);
    }

    #[test]
    fn text_primary_is_high_contrast_against_bg() {
        let delta = Tokens::DARK.text_primary.relative_luminance()
            - Tokens::DARK.bg.relative_luminance();
        assert!(delta > 0.5, "text_primary/bg luminance delta {delta} must exceed 0.5");
    }
}
