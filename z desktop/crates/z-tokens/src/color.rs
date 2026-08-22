//! Colour primitives and the WCAG contrast maths the visual QA checklist relies on.

/// Straight (non-premultiplied) sRGB colour with alpha, 8 bits per channel.
///
/// Kept as u8 so tokens stay exact and comparable; conversion to linear float
/// happens once, at the point the renderer builds its vertex data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const TRANSPARENT: Rgba = Rgba::new(0, 0, 0, 0);

    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// `0xRRGGBB` — the form design tokens are written in.
    pub const fn hex(value: u32) -> Self {
        Self {
            r: ((value >> 16) & 0xFF) as u8,
            g: ((value >> 8) & 0xFF) as u8,
            b: (value & 0xFF) as u8,
            a: 255,
        }
    }

    /// Same colour at a different alpha. Used for the border tokens, which are
    /// white at low alpha rather than a separate grey — that way they stay
    /// correct over any surface tone.
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// Premultiplied linear-space RGBA, ready for the GPU.
    pub fn to_linear_premultiplied(self) -> [f32; 4] {
        let a = self.a as f32 / 255.0;
        let [r, g, b] = [self.r, self.g, self.b].map(srgb_channel_to_linear);
        [r * a, g * a, b * a, a]
    }

    /// Composite `self` over `backdrop`, honouring `self.a`.
    ///
    /// Needed because contrast has to be measured against what the user actually
    /// sees: a translucent border over `surface` is not the same colour as the
    /// border token in isolation.
    pub fn over(self, backdrop: Rgba) -> Rgba {
        if self.a == 255 {
            return self;
        }
        let alpha = self.a as f32 / 255.0;
        let mix = |fg: u8, bg: u8| (fg as f32 * alpha + bg as f32 * (1.0 - alpha)).round() as u8;
        Rgba {
            r: mix(self.r, backdrop.r),
            g: mix(self.g, backdrop.g),
            b: mix(self.b, backdrop.b),
            a: 255,
        }
    }

    /// WCAG 2.1 relative luminance.
    pub fn relative_luminance(self) -> f32 {
        let [r, g, b] = [self.r, self.g, self.b].map(srgb_channel_to_linear);
        0.2126 * r + 0.7152 * g + 0.0722 * b
    }

    /// WCAG 2.1 contrast ratio, from 1.0 (identical) to 21.0 (black on white).
    ///
    /// Both colours must already be opaque — composite with [`Rgba::over`] first.
    pub fn contrast_ratio(self, other: Rgba) -> f32 {
        let a = self.relative_luminance();
        let b = other.relative_luminance();
        let (lighter, darker) = if a > b { (a, b) } else { (b, a) };
        (lighter + 0.05) / (darker + 0.05)
    }
}

fn srgb_channel_to_linear(channel: u8) -> f32 {
    let c = channel as f32 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Minimum contrast for body-size text under WCAG AA.
pub const AA_NORMAL_TEXT: f32 = 4.5;
/// Minimum contrast for large text and for non-text UI boundaries under WCAG AA.
pub const AA_LARGE_TEXT: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        assert_eq!(Rgba::hex(0xF2F2F5), Rgba::rgb(0xF2, 0xF2, 0xF5));
    }

    #[test]
    fn black_on_white_is_the_maximum_ratio() {
        let ratio = Rgba::hex(0x000000).contrast_ratio(Rgba::hex(0xFFFFFF));
        assert!((ratio - 21.0).abs() < 0.01, "got {ratio}");
    }

    #[test]
    fn identical_colours_have_ratio_one() {
        let c = Rgba::hex(0x0F0F11);
        assert!((c.contrast_ratio(c) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn contrast_is_symmetric() {
        let a = Rgba::hex(0xF2F2F5);
        let b = Rgba::hex(0x08080A);
        assert_eq!(a.contrast_ratio(b), b.contrast_ratio(a));
    }

    #[test]
    fn compositing_a_translucent_colour_lands_between_the_two() {
        let over = Rgba::new(255, 255, 255, 26).over(Rgba::hex(0x0F0F11));
        assert!(over.r > 0x0F && over.r < 0xFF);
        assert_eq!(over.a, 255);
    }

    #[test]
    fn opaque_colour_ignores_backdrop() {
        let c = Rgba::hex(0x123456);
        assert_eq!(c.over(Rgba::hex(0xFFFFFF)), c);
    }

    #[test]
    fn premultiplied_alpha_scales_channels() {
        let half = Rgba::new(255, 255, 255, 128);
        let [r, _, _, a] = half.to_linear_premultiplied();
        assert!((a - 128.0 / 255.0).abs() < 0.001);
        assert!(r < 0.51, "premultiply should scale the channel down, got {r}");
    }
}
