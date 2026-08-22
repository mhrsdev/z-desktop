//! Semantic colour tokens — the only colour surface UI code is allowed to touch.

use crate::color::{Rgba, AA_LARGE_TEXT, AA_NORMAL_TEXT};

/// Raw palette. Private on purpose: widgets reference [`Semantic`], never these.
mod primitive {
    use crate::color::Rgba;

    // Extracted from the approved product reference. The small differences
    // between adjacent charcoals are intentional: elevation must read from
    // surface tone, not from heavy shadows.
    pub const CHARCOAL_000: Rgba = Rgba::hex(0x151515);
    pub const CHARCOAL_050: Rgba = Rgba::hex(0x1E1E1E);
    pub const CHARCOAL_100: Rgba = Rgba::hex(0x20201F);
    pub const CHARCOAL_150: Rgba = Rgba::hex(0x282827);
    pub const CHARCOAL_200: Rgba = Rgba::hex(0x2D2D2D);

    pub const GREY_400: Rgba = Rgba::hex(0x6D6B67);
    pub const GREY_500: Rgba = Rgba::hex(0x898781);
    pub const GREY_600: Rgba = Rgba::hex(0xB9B9B9);
    pub const GREY_800: Rgba = Rgba::hex(0xC1C1C1);
    pub const WHITE_000: Rgba = Rgba::hex(0xC1C1C1);

    /// The reference's restrained warm accent. It is reserved for product
    /// identity, selection and keyboard focus rather than used decoratively.
    pub const CORAL_500: Rgba = Rgba::hex(0xD87656);

    pub const GREEN_500: Rgba = Rgba::hex(0x3FB950);
    pub const RED_500: Rgba = Rgba::hex(0xF85149);
    pub const BLUE_500: Rgba = Rgba::hex(0x58A6FF);
}

/// Role-named colours. Every value here has a job, not a hue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Semantic {
    pub canvas: Rgba,
    pub surface: Rgba,
    pub surface_raised: Rgba,
    pub surface_overlay: Rgba,
    pub surface_hover: Rgba,

    pub border_subtle: Rgba,
    pub border_default: Rgba,
    pub border_strong: Rgba,

    pub text_primary: Rgba,
    pub text_secondary: Rgba,
    pub text_tertiary: Rgba,
    pub text_disabled: Rgba,
    pub text_inverse: Rgba,

    pub accent: Rgba,
    pub accent_muted: Rgba,

    pub focus_ring: Rgba,
    pub selection: Rgba,

    pub status_success: Rgba,
    pub status_warning: Rgba,
    pub status_danger: Rgba,
    pub status_info: Rgba,
    /// Deliberately neutral: "running" is a state, not an alarm.
    pub status_running: Rgba,
}

/// A complete appearance. Themes are data — a preset or extension supplies one,
/// it never supplies code.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    pub colors: Semantic,
}

impl Theme {
    /// The product default: the charcoal layers and warm coral accent from the
    /// approved desktop reference. The accent stays scarce so work surfaces
    /// remain calm and readable.
    pub const fn zero_dark() -> Self {
        use primitive::*;
        Self {
            name: "Zero Dark",
            colors: Semantic {
                canvas: CHARCOAL_000,
                surface: CHARCOAL_050,
                surface_raised: CHARCOAL_100,
                surface_overlay: CHARCOAL_150,
                surface_hover: CHARCOAL_200,

                // White at low alpha rather than fixed greys, so a border reads
                // correctly over any surface tone it happens to sit on.
                border_subtle: Rgba::new(255, 255, 255, 17),
                border_default: Rgba::new(255, 255, 255, 20),
                border_strong: Rgba::new(255, 255, 255, 36),

                text_primary: WHITE_000,
                text_secondary: GREY_600,
                text_tertiary: GREY_500,
                text_disabled: GREY_400,
                text_inverse: CHARCOAL_000,

                accent: CORAL_500,
                accent_muted: Rgba::new(216, 118, 86, 42),

                focus_ring: CORAL_500,
                selection: Rgba::new(216, 118, 86, 42),

                status_success: GREEN_500,
                status_warning: CORAL_500,
                status_danger: RED_500,
                status_info: BLUE_500,
                status_running: GREY_800,
            },
        }
    }

    /// Every (foreground, background) pair the visual QA checklist has to clear,
    /// with the ratio each pair is held to.
    ///
    /// Exposed rather than kept inside the test so `ui-visual-qa` and any future
    /// theme (user preset, extension) can be checked through the same gate.
    pub fn contrast_pairs(&self) -> Vec<ContrastPair> {
        let c = &self.colors;
        vec![
            ContrastPair::text("text_primary on surface", c.text_primary, c.surface),
            ContrastPair::text("text_primary on canvas", c.text_primary, c.canvas),
            ContrastPair::text("text_primary on raised", c.text_primary, c.surface_raised),
            ContrastPair::text("text_primary on overlay", c.text_primary, c.surface_overlay),
            ContrastPair::text("text_secondary on surface", c.text_secondary, c.surface),
            ContrastPair::text("text_secondary on raised", c.text_secondary, c.surface_raised),
            ContrastPair::text("accent on surface", c.accent, c.surface),
            ContrastPair::text("accent on raised", c.accent, c.surface_raised),
            ContrastPair::large("text_tertiary on surface", c.text_tertiary, c.surface),
            ContrastPair::large("focus_ring on surface", c.focus_ring, c.surface),
            ContrastPair::large("focus_ring on raised", c.focus_ring, c.surface_raised),
            ContrastPair::large("status_success on raised", c.status_success, c.surface_raised),
            ContrastPair::large("status_warning on raised", c.status_warning, c.surface_raised),
            ContrastPair::large("status_danger on raised", c.status_danger, c.surface_raised),
            ContrastPair::large("status_info on raised", c.status_info, c.surface_raised),
        ]
    }
}

/// One contrast requirement, resolved against the surface it actually sits on.
#[derive(Debug, Clone, Copy)]
pub struct ContrastPair {
    pub label: &'static str,
    pub foreground: Rgba,
    pub background: Rgba,
    pub minimum: f32,
}

impl ContrastPair {
    fn text(label: &'static str, fg: Rgba, bg: Rgba) -> Self {
        Self { label, foreground: fg, background: bg, minimum: AA_NORMAL_TEXT }
    }

    fn large(label: &'static str, fg: Rgba, bg: Rgba) -> Self {
        Self { label, foreground: fg, background: bg, minimum: AA_LARGE_TEXT }
    }

    /// Actual ratio, with translucent foregrounds composited over the backdrop first.
    pub fn ratio(&self) -> f32 {
        self.foreground.over(self.background).contrast_ratio(self.background)
    }

    pub fn passes(&self) -> bool {
        self.ratio() >= self.minimum
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::zero_dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_meets_wcag_aa_everywhere() {
        let theme = Theme::zero_dark();
        let failures: Vec<String> = theme
            .contrast_pairs()
            .iter()
            .filter(|pair| !pair.passes())
            .map(|pair| format!("{}: {:.2} < {:.1}", pair.label, pair.ratio(), pair.minimum))
            .collect();
        assert!(failures.is_empty(), "contrast failures:\n  {}", failures.join("\n  "));
    }

    #[test]
    fn default_theme_preserves_the_approved_charcoal_and_coral_values() {
        let c = Theme::zero_dark().colors;
        assert_eq!(c.canvas, Rgba::hex(0x151515));
        assert_eq!(c.surface, Rgba::hex(0x1E1E1E));
        assert_eq!(c.surface_raised, Rgba::hex(0x20201F));
        assert_eq!(c.surface_overlay, Rgba::hex(0x282827));
        assert_eq!(c.surface_hover, Rgba::hex(0x2D2D2D));
        assert_eq!(c.accent, Rgba::hex(0xD87656));
        assert_eq!(c.focus_ring, c.accent);
        assert_eq!(c.selection, c.accent_muted);
    }

    #[test]
    fn surface_tones_are_ordered_and_distinguishable() {
        let c = Theme::zero_dark().colors;
        let tones = [c.canvas, c.surface, c.surface_raised, c.surface_overlay, c.surface_hover];
        for pair in tones.windows(2) {
            let (lower, upper) = (pair[0], pair[1]);
            assert!(
                upper.relative_luminance() > lower.relative_luminance(),
                "surface tones must ascend so elevation reads without shadow"
            );
        }
    }

    #[test]
    fn no_token_is_a_dominant_purple() {
        // The spec bans a purple accent. Flag any hue where blue leads red and
        // green by a wide margin while red still outruns green.
        let c = Theme::zero_dark().colors;
        for (name, color) in [
            ("focus_ring", c.focus_ring),
            ("status_running", c.status_running),
            ("text_primary", c.text_primary),
            ("accent", c.accent),
            ("accent_muted", c.accent_muted),
            ("selection", c.selection),
        ] {
            let purple = color.b as i32 - color.g as i32 > 40 && color.r as i32 > color.g as i32;
            assert!(!purple, "{name} reads as purple");
        }
    }

    #[test]
    fn borders_are_translucent_so_they_work_on_any_surface() {
        let c = Theme::zero_dark().colors;
        for (name, border) in [
            ("subtle", c.border_subtle),
            ("default", c.border_default),
            ("strong", c.border_strong),
        ] {
            assert!(border.a < 255, "border_{name} must be translucent");
        }
    }
}
