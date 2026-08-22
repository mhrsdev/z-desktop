//! Type scale and font roles.
//!
//! Two families only: a neutral UI sans, and an independent monospace for code
//! and for any number that changes while the user is looking at it.

/// Which family a piece of text belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontRole {
    /// Neutral, readable, tolerates tight line heights.
    Ui,
    /// Code, terminal, and the Performance Strip readouts.
    Mono,
}

impl FontRole {
    /// Ordered fallback stack. First match on the system wins; the last entry
    /// must be a family every target platform actually ships, so a missing
    /// font degrades to something readable instead of tofu.
    pub const fn families(self) -> &'static [&'static str] {
        match self {
            FontRole::Ui => &[
                "Inter",
                "Segoe UI Variable Text",
                "Segoe UI",
                "SF Pro Text",
                "Noto Sans",
                "DejaVu Sans",
                "sans-serif",
            ],
            FontRole::Mono => &[
                "JetBrains Mono",
                "Cascadia Mono",
                "SF Mono",
                "Noto Sans Mono",
                "DejaVu Sans Mono",
                "monospace",
            ],
        }
    }

    /// Families carrying Arabic-script coverage, appended for RTL runs so
    /// Persian and Arabic never fall through to a box glyph.
    pub const fn rtl_fallbacks() -> &'static [&'static str] {
        &["Vazirmatn", "Segoe UI", "Geeza Pro", "Noto Naskh Arabic", "Tahoma"]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FontWeight {
    Regular = 400,
    Medium = 500,
    Semibold = 600,
}

/// One resolved text style.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    pub role: FontRole,
    pub size: f32,
    pub line_height: f32,
    pub weight: FontWeight,
    /// Fixed-advance digits. Required anywhere a number updates in place, so
    /// the layout does not shudder once a second.
    pub tabular_numbers: bool,
}

impl TextStyle {
    const fn ui(size: f32, line_height: f32, weight: FontWeight) -> Self {
        Self { role: FontRole::Ui, size, line_height, weight, tabular_numbers: false }
    }

    pub const fn with_tabular_numbers(mut self) -> Self {
        self.tabular_numbers = true;
        self
    }

    pub const fn mono(mut self) -> Self {
        self.role = FontRole::Mono;
        self
    }

    /// Ratio of line height to font size — the check that catches a style
    /// which would render cramped or unreadably loose.
    pub fn leading_ratio(&self) -> f32 {
        self.line_height / self.size
    }
}

/// The product type scale.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Typography;

impl Typography {
    /// Strip labels, timestamps.
    pub const XS: TextStyle = TextStyle::ui(11.0, 16.0, FontWeight::Regular);
    /// Metadata, secondary labels.
    pub const SM: TextStyle = TextStyle::ui(12.0, 18.0, FontWeight::Regular);
    /// Navigation and general UI.
    pub const BASE: TextStyle = TextStyle::ui(13.0, 20.0, FontWeight::Regular);
    /// Chat message body — the most-read text in the product.
    pub const BODY: TextStyle = TextStyle::ui(14.0, 22.0, FontWeight::Regular);
    /// Section headings.
    pub const LG: TextStyle = TextStyle::ui(16.0, 24.0, FontWeight::Semibold);
    /// Page headings.
    pub const XL: TextStyle = TextStyle::ui(20.0, 28.0, FontWeight::Semibold);

    /// Control and group labels.
    pub const LABEL: TextStyle = TextStyle::ui(12.0, 16.0, FontWeight::Medium);
    /// Code blocks and the IDE surface.
    pub const CODE: TextStyle = TextStyle::ui(13.0, 20.0, FontWeight::Regular).mono();
    /// CPU / GPU / RAM / FPS readouts.
    pub const METRIC: TextStyle =
        TextStyle::ui(11.0, 16.0, FontWeight::Medium).mono().with_tabular_numbers();

    pub const ALL: &'static [TextStyle] = &[
        Self::XS,
        Self::SM,
        Self::BASE,
        Self::BODY,
        Self::LG,
        Self::XL,
        Self::LABEL,
        Self::CODE,
        Self::METRIC,
    ];
}

/// Compile-time gate: the Performance Strip readouts change once a second, and
/// without fixed-advance digits the whole strip shudders as they do. Asserting
/// this at build time rather than in a test means it can never regress.
const _: () =
    assert!(Typography::METRIC.tabular_numbers, "Typography::METRIC must use tabular figures");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_style_has_a_comfortable_leading_ratio() {
        for style in Typography::ALL {
            let ratio = style.leading_ratio();
            assert!(
                (1.2..=1.7).contains(&ratio),
                "leading ratio {ratio:.2} at size {} is outside the readable band",
                style.size
            );
        }
    }

    #[test]
    fn the_scale_ascends_without_gaps() {
        let sizes = [
            Typography::XS.size,
            Typography::SM.size,
            Typography::BASE.size,
            Typography::BODY.size,
            Typography::LG.size,
            Typography::XL.size,
        ];
        for pair in sizes.windows(2) {
            assert!(pair[1] > pair[0], "type scale must ascend: {pair:?}");
        }
    }

    #[test]
    fn changing_readouts_use_tabular_figures() {
        assert_eq!(Typography::METRIC.role, FontRole::Mono);
    }

    #[test]
    fn code_uses_the_independent_mono_family() {
        assert_eq!(Typography::CODE.role, FontRole::Mono);
        assert_ne!(FontRole::Ui.families()[0], FontRole::Mono.families()[0]);
    }

    #[test]
    fn every_fallback_stack_ends_in_a_generic_family() {
        for role in [FontRole::Ui, FontRole::Mono] {
            let families = role.families();
            let last = families[families.len() - 1];
            assert!(
                last == "sans-serif" || last == "monospace",
                "{role:?} stack must end in a generic family, ends in {last}"
            );
        }
    }

    #[test]
    fn rtl_fallbacks_are_available_for_arabic_script() {
        assert!(!FontRole::rtl_fallbacks().is_empty());
    }
}
