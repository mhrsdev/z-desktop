//! ZeroText — shaping, the glyph atlas, and the BiDi rules.
//!
//! Two things here are deliberate and easy to get wrong later:
//!
//! 1. **Base direction comes from the UI language, never from the first
//!    character of the string.** A Persian sentence that happens to start with
//!    a file path is still a Persian sentence.
//! 2. **Direction-neutral content is isolated.** Paths, URLs, hashes, model
//!    names and code fragments get wrapped in Unicode isolate marks so they keep
//!    their own order inside prose running the other way.

use crate::geometry::Rect;
use crate::scene::{Align, Direction, TextRun};
use cosmic_text::{
    Attrs, Buffer, Family, FontSystem, Metrics, Shaping, SwashCache, Weight as CosmicWeight,
};
use std::collections::HashMap;
use z_tokens::{FontRole, Rgba};

/// Unicode bidirectional isolate marks.
pub const LRI: char = '\u{2066}'; // LEFT-TO-RIGHT ISOLATE
pub const RLI: char = '\u{2067}'; // RIGHT-TO-LEFT ISOLATE
pub const FSI: char = '\u{2068}'; // FIRST STRONG ISOLATE
pub const PDI: char = '\u{2069}'; // POP DIRECTIONAL ISOLATE

/// Explicit override marks. These can make text render in an order that does
/// not match its logical content, which is a spoofing vector in filenames and
/// links, so they are stripped rather than honoured.
const DIRECTIONAL_OVERRIDES: [char; 5] = [
    '\u{202A}', // LRE
    '\u{202B}', // RLE
    '\u{202C}', // PDF
    '\u{202D}', // LRO
    '\u{202E}', // RLO
];

/// Wrap direction-neutral content so it keeps its own order inside surrounding
/// prose of the opposite direction.
///
/// `FSI` rather than `LRI`: the isolate should take the direction of the
/// content's own first strong character, which is what makes it correct for
/// both `src/auth/session.ts` and a mixed identifier.
pub fn isolate(content: &str) -> String {
    format!("{FSI}{content}{PDI}")
}

/// Remove explicit directional overrides from untrusted text.
///
/// Applied to anything that arrives from outside the core — filenames, links,
/// tool output, model responses — so a crafted string cannot display in an
/// order that misrepresents what it actually says.
pub fn strip_directional_overrides(text: &str) -> String {
    text.chars().filter(|c| !DIRECTIONAL_OVERRIDES.contains(c)).collect()
}

/// Whether a string contains characters from a right-to-left script.
///
/// Used for choosing font fallbacks, **not** for choosing base direction.
pub fn contains_rtl_script(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c as u32,
            0x0590..=0x05FF   // Hebrew
            | 0x0600..=0x06FF // Arabic
            | 0x0700..=0x074F // Syriac
            | 0x0750..=0x077F // Arabic Supplement
            | 0x08A0..=0x08FF // Arabic Extended-A
            | 0xFB1D..=0xFDFF // Hebrew / Arabic presentation forms
            | 0xFE70..=0xFEFF // Arabic presentation forms-B
        )
    })
}

/// Where one glyph lives in the atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphSlot {
    /// Normalised atlas coordinates: u0, v0, u1, v1.
    pub uv: [f32; 4],
    /// Size in physical pixels.
    pub size: [f32; 2],
    /// Offset from the glyph origin to its top-left, in physical pixels.
    pub offset: [f32; 2],
}

/// A glyph positioned and ready to draw.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionedGlyph {
    /// Bounds in logical pixels.
    pub bounds: Rect,
    pub uv: [f32; 4],
    pub color: Rgba,
}

/// Shelf-packing coverage atlas.
///
/// Shelf packing rather than something cleverer because UI glyphs arrive in a
/// small number of sizes and pack tightly by row. If the atlas fills, it is
/// cleared and repopulated — a visible cost, but a bounded one, and the budget
/// check below is what stops that becoming a silent per-frame stall.
pub struct GlyphAtlas {
    width: u32,
    height: u32,
    shelf_y: u32,
    shelf_height: u32,
    cursor_x: u32,
    pixels: Vec<u8>,
    dirty: bool,
    /// Number of times the atlas has been reset. A steadily rising count means
    /// the atlas is too small for the working set.
    pub eviction_count: u32,
}

impl GlyphAtlas {
    pub const PADDING: u32 = 1;

    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            shelf_y: 0,
            shelf_height: 0,
            cursor_x: 0,
            pixels: vec![0; (width * height) as usize],
            dirty: true,
            eviction_count: 0,
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }

    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    /// Fraction of the atlas consumed so far.
    pub fn utilisation(&self) -> f32 {
        if self.height == 0 {
            return 0.0;
        }
        (self.shelf_y + self.shelf_height) as f32 / self.height as f32
    }

    fn reset(&mut self) {
        self.pixels.fill(0);
        self.shelf_y = 0;
        self.shelf_height = 0;
        self.cursor_x = 0;
        self.dirty = true;
        self.eviction_count += 1;
    }

    /// Copy a coverage bitmap in, returning where it landed.
    ///
    /// `None` means the glyph is larger than the atlas itself and will never fit.
    fn insert(&mut self, width: u32, height: u32, data: &[u8]) -> Option<GlyphSlot> {
        if width == 0 || height == 0 {
            return Some(GlyphSlot { uv: [0.0; 4], size: [0.0, 0.0], offset: [0.0, 0.0] });
        }
        if width > self.width || height > self.height {
            return None;
        }

        if self.cursor_x + width > self.width {
            // Next shelf.
            self.shelf_y += self.shelf_height + Self::PADDING;
            self.shelf_height = 0;
            self.cursor_x = 0;
        }
        if self.shelf_y + height > self.height {
            self.reset();
        }

        let x = self.cursor_x;
        let y = self.shelf_y;
        for row in 0..height {
            let src = (row * width) as usize;
            let dst = ((y + row) * self.width + x) as usize;
            self.pixels[dst..dst + width as usize]
                .copy_from_slice(&data[src..src + width as usize]);
        }

        self.cursor_x += width + Self::PADDING;
        self.shelf_height = self.shelf_height.max(height);
        self.dirty = true;

        let (aw, ah) = (self.width as f32, self.height as f32);
        Some(GlyphSlot {
            uv: [x as f32 / aw, y as f32 / ah, (x + width) as f32 / aw, (y + height) as f32 / ah],
            size: [width as f32, height as f32],
            offset: [0.0, 0.0],
        })
    }
}

/// Shaping, font fallback and glyph caching.
pub struct TextSystem {
    font_system: FontSystem,
    swash: SwashCache,
    atlas: GlyphAtlas,
    slots: HashMap<cosmic_text::CacheKey, Option<GlyphSlot>>,
    scale: f32,
}

impl TextSystem {
    pub fn new(scale: f32) -> Self {
        Self {
            font_system: FontSystem::new(),
            swash: SwashCache::new(),
            atlas: GlyphAtlas::new(2048, 2048),
            slots: HashMap::new(),
            scale: scale.max(0.1),
        }
    }

    pub fn atlas(&self) -> &GlyphAtlas {
        &self.atlas
    }

    pub fn atlas_mut(&mut self) -> &mut GlyphAtlas {
        &mut self.atlas
    }

    pub fn set_scale(&mut self, scale: f32) {
        let scale = scale.max(0.1);
        if (scale - self.scale).abs() > f32::EPSILON {
            // Cached coverage is rasterised for a specific scale, so a display
            // change invalidates every slot. Missing this is how text ends up
            // blurry after a window moves between monitors.
            self.scale = scale;
            self.slots.clear();
            self.atlas.reset();
        }
    }

    /// The text as it will actually be shaped, after isolation and stripping.
    pub fn prepare(&self, run: &TextRun) -> String {
        let cleaned = strip_directional_overrides(&run.text);
        if run.isolate {
            isolate(&cleaned)
        } else {
            cleaned
        }
    }

    fn attrs_for(role: FontRole, rtl: bool) -> Attrs<'static> {
        let family = match (role, rtl) {
            (FontRole::Mono, _) => Family::Monospace,
            (FontRole::Ui, _) => Family::SansSerif,
        };
        Attrs::new().family(family).weight(CosmicWeight::NORMAL)
    }

    /// Shape a run and return its glyphs, positioned in logical pixels.
    pub fn layout(&mut self, run: &TextRun) -> Vec<PositionedGlyph> {
        if run.bounds.is_empty() || run.text.is_empty() {
            return Vec::new();
        }

        let text = self.prepare(run);
        let metrics = Metrics::new(run.style.size, run.style.line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(run.bounds.width), Some(run.bounds.height));

        let rtl = run.direction == Direction::Rtl;
        let attrs = Self::attrs_for(run.style.role, rtl);
        buffer.set_text(&text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        // Alignment is applied here rather than through the shaper so that a
        // run's box stays authoritative: the caller decided the bounds, and
        // text is placed inside them.
        let mut glyphs = Vec::new();
        for layout_run in buffer.layout_runs() {
            let free = (run.bounds.width - layout_run.line_w).max(0.0);
            let x_offset = match run.align {
                Align::Start => {
                    if rtl {
                        free
                    } else {
                        0.0
                    }
                }
                Align::Center => free / 2.0,
                Align::End => {
                    if rtl {
                        0.0
                    } else {
                        free
                    }
                }
            };

            for glyph in layout_run.glyphs {
                let physical = glyph.physical((0.0, 0.0), self.scale);
                let Some(slot) = self.slot_for(physical.cache_key) else { continue };
                if slot.size[0] <= 0.0 || slot.size[1] <= 0.0 {
                    continue;
                }

                // Physical raster back to logical space, anchored on the baseline.
                let x = run.bounds.x + x_offset + (physical.x as f32 + slot.offset[0]) / self.scale;
                let y = run.bounds.y
                    + layout_run.line_y
                    + (physical.y as f32 - slot.offset[1]) / self.scale;

                glyphs.push(PositionedGlyph {
                    bounds: Rect::new(x, y, slot.size[0] / self.scale, slot.size[1] / self.scale),
                    uv: slot.uv,
                    color: run.color,
                });
            }
        }
        glyphs
    }

    /// Rasterise and cache a glyph, or return the cached slot.
    fn slot_for(&mut self, key: cosmic_text::CacheKey) -> Option<GlyphSlot> {
        if let Some(cached) = self.slots.get(&key) {
            return *cached;
        }

        let image = self.swash.get_image(&mut self.font_system, key).clone();
        let slot = image.and_then(|image| {
            let width = image.placement.width;
            let height = image.placement.height;
            let mut slot = self.atlas.insert(width, height, &image.data)?;
            slot.offset = [image.placement.left as f32, image.placement.top as f32];
            Some(slot)
        });

        self.slots.insert(key, slot);
        slot
    }

    /// Advance width of a run, in logical pixels. Used for measurement passes
    /// that need a width before committing to a layout.
    pub fn measure(&mut self, run: &TextRun) -> f32 {
        let text = self.prepare(run);
        let metrics = Metrics::new(run.style.size, run.style.line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(None, None);
        let attrs = Self::attrs_for(run.style.role, run.direction == Direction::Rtl);
        buffer.set_text(&text, &attrs, Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.font_system, false);
        buffer.layout_runs().map(|r| r.line_w).fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use z_tokens::Typography;

    #[test]
    fn isolation_wraps_content_in_a_matched_pair() {
        let wrapped = isolate("src/auth/session.ts");
        assert!(wrapped.starts_with(FSI));
        assert!(wrapped.ends_with(PDI));
        assert!(wrapped.contains("src/auth/session.ts"));
    }

    #[test]
    fn directional_overrides_are_stripped_from_untrusted_text() {
        // The classic filename-spoofing shape.
        let hostile = "invoice\u{202E}gpj.exe";
        let safe = strip_directional_overrides(hostile);
        assert!(!safe.contains('\u{202E}'));
        assert_eq!(safe, "invoicegpj.exe");
    }

    #[test]
    fn isolate_marks_survive_stripping() {
        // Isolates are how we keep mixed text correct, so they must not be
        // removed alongside the overrides.
        let text = isolate("main");
        assert_eq!(strip_directional_overrides(&text), text);
    }

    #[test]
    fn rtl_script_detection_covers_persian_and_arabic() {
        assert!(contains_rtl_script("پروژه"));
        assert!(contains_rtl_script("مرحبا"));
        assert!(!contains_rtl_script("src/auth/session.ts"));
        assert!(!contains_rtl_script("Zero Desktop"));
    }

    #[test]
    fn a_path_inside_persian_prose_is_detected_as_mixed() {
        let mixed = "فایل src/auth/session.ts تغییر کرد";
        assert!(contains_rtl_script(mixed));
        assert!(mixed.contains("src/auth/session.ts"));
    }

    #[test]
    fn atlas_packs_glyphs_onto_shelves_without_overlap() {
        let mut atlas = GlyphAtlas::new(64, 64);
        let a = atlas.insert(10, 10, &[255; 100]).unwrap();
        let b = atlas.insert(10, 10, &[255; 100]).unwrap();
        assert_ne!(a.uv, b.uv, "two glyphs must not claim the same region");
        assert!(b.uv[0] > a.uv[0], "the second glyph should sit to the right");
    }

    #[test]
    fn atlas_wraps_to_a_new_shelf_when_a_row_fills() {
        let mut atlas = GlyphAtlas::new(32, 64);
        let first = atlas.insert(20, 10, &[255; 200]).unwrap();
        let second = atlas.insert(20, 10, &[255; 200]).unwrap();
        assert!(second.uv[1] > first.uv[1], "the second glyph should drop to a new shelf");
    }

    #[test]
    fn a_full_atlas_resets_rather_than_corrupting() {
        let mut atlas = GlyphAtlas::new(32, 32);
        for _ in 0..20 {
            atlas.insert(16, 16, &[255; 256]);
        }
        assert!(atlas.eviction_count > 0, "the atlas should have recycled");
        assert!(atlas.utilisation() <= 1.0);
    }

    #[test]
    fn a_glyph_larger_than_the_atlas_is_refused_not_wrapped() {
        let mut atlas = GlyphAtlas::new(16, 16);
        assert!(atlas.insert(64, 64, &[255; 4096]).is_none());
    }

    #[test]
    fn an_empty_bitmap_is_accepted_as_a_zero_sized_slot() {
        let mut atlas = GlyphAtlas::new(16, 16);
        let slot = atlas.insert(0, 0, &[]).unwrap();
        assert_eq!(slot.size, [0.0, 0.0]);
    }

    #[test]
    fn preparing_an_isolated_run_adds_the_marks() {
        let system = TextSystem::new(1.0);
        let run = TextRun::new(
            "src/auth/session.ts",
            Rect::new(0.0, 0.0, 200.0, 20.0),
            Typography::CODE,
            Rgba::hex(0xFFFFFF),
        )
        .isolated();
        let prepared = system.prepare(&run);
        assert!(prepared.starts_with(FSI) && prepared.ends_with(PDI));
    }

    #[test]
    fn preparing_a_plain_run_leaves_it_alone() {
        let system = TextSystem::new(1.0);
        let run = TextRun::new(
            "Refactor the authentication flow",
            Rect::new(0.0, 0.0, 400.0, 20.0),
            Typography::BODY,
            Rgba::hex(0xFFFFFF),
        );
        assert_eq!(system.prepare(&run), "Refactor the authentication flow");
    }
}
