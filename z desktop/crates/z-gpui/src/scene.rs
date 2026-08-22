//! The scene: what a frame contains, independent of how it gets drawn.
//!
//! Building a scene is cheap and allocation-light; the renderer consumes it and
//! keeps no reference. Because scenes are comparable, the runtime can tell that
//! nothing changed and skip the frame entirely — the cheapest possible redraw.

use crate::a11y::{AccessNode, AccessTree};
use crate::geometry::Rect;
use z_tokens::{FontRole, Rgba, TextStyle};

/// A filled rectangle with optional rounded corners and a border.
///
/// One primitive covers surfaces, cards, inputs, dividers, focus rings and
/// badges. Keeping it to a single shape means one pipeline and one draw call
/// for the entire structural layer of the UI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quad {
    pub bounds: Rect,
    pub background: Rgba,
    pub border_color: Rgba,
    pub border_width: f32,
    pub corner_radius: f32,
    /// Region outside which fragments are discarded.
    ///
    /// Set from the scene's clip stack when the quad is pushed. Clipping has to
    /// happen in the shader rather than by shrinking `bounds`, because
    /// shrinking would drag the rounded corners inward — a card cut off by the
    /// edge of a scroll area must show a straight cut, not a new corner.
    pub clip: Rect,
}

impl Quad {
    pub fn filled(bounds: Rect, background: Rgba) -> Self {
        Self {
            bounds,
            background,
            border_color: Rgba::TRANSPARENT,
            border_width: 0.0,
            corner_radius: 0.0,
            clip: Rect::UNBOUNDED,
        }
    }

    pub fn with_radius(mut self, radius: f32) -> Self {
        self.corner_radius = radius;
        self
    }

    pub fn with_border(mut self, color: Rgba, width: f32) -> Self {
        self.border_color = color;
        self.border_width = width;
        self
    }

    /// A hairline: one physical pixel regardless of display scale.
    pub fn divider(bounds: Rect, color: Rgba) -> Self {
        Self::filled(bounds, color)
    }

    /// Whether this quad would produce no visible pixels.
    pub fn is_invisible(&self) -> bool {
        self.bounds.is_empty()
            || (self.background.a == 0 && (self.border_color.a == 0 || self.border_width <= 0.0))
            // Fully outside its clip: the GPU would discard every fragment, so
            // do not pay to send it.
            || !self.bounds.intersects(&self.clip)
    }
}

/// Horizontal placement of a text run inside its bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
}

/// Base direction for a run. Resolved from the UI language, never guessed from
/// the first character of the string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Ltr,
    Rtl,
}

/// A run of text to be shaped and drawn inside `bounds`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextRun {
    pub text: String,
    pub bounds: Rect,
    pub style: TextStyle,
    pub color: Rgba,
    pub align: Align,
    pub direction: Direction,
    /// Region outside which glyphs are discarded. See [`Quad::clip`].
    pub clip: Rect,
    /// Direction-neutral content — a path, URL, hash, model name or code
    /// fragment — that must keep its own order inside a run of the opposite
    /// direction. Shaping wraps these in isolate marks.
    pub isolate: bool,
}

impl TextRun {
    pub fn new(text: impl Into<String>, bounds: Rect, style: TextStyle, color: Rgba) -> Self {
        Self {
            text: text.into(),
            bounds,
            style,
            color,
            align: Align::Start,
            direction: Direction::Ltr,
            isolate: false,
            clip: Rect::UNBOUNDED,
        }
    }

    pub fn aligned(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn rtl(mut self) -> Self {
        self.direction = Direction::Rtl;
        self
    }

    /// Mark this run as direction-neutral content that must not be reordered.
    pub fn isolated(mut self) -> Self {
        self.isolate = true;
        self
    }

    pub fn font_role(&self) -> FontRole {
        self.style.role
    }
}

/// Painting order. Within a layer, insertion order decides what sits on top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Layer {
    /// Window background and panel surfaces.
    Background,
    /// Ordinary panel content.
    Content,
    /// Floating tool, dropdowns, popovers.
    Overlay,
    /// Focus rings — above everything, so a focused control is never obscured.
    Focus,
}

impl Layer {
    pub const ALL: &'static [Layer] =
        &[Layer::Background, Layer::Content, Layer::Overlay, Layer::Focus];
}

/// One frame's worth of drawing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    quads: Vec<(Layer, Quad)>,
    texts: Vec<(Layer, TextRun)>,
    /// Bounds of everything added. Used as the damage region when the runtime
    /// cannot prove a tighter one.
    bounds: Rect,
    /// Nested clip regions. Each push intersects with the one below, so a
    /// child can never draw outside its parent.
    clips: Vec<Rect>,
    /// What the visuals *mean*. Declared by the view in reading order; nothing
    /// here is inferred from the quads, because an inferred label is a guess.
    access: AccessTree,
}

impl Scene {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrict everything pushed until the matching [`Scene::pop_clip`].
    ///
    /// Nested pushes intersect, so a scroll area inside a panel cannot draw
    /// outside the panel even if it tries.
    pub fn push_clip(&mut self, rect: Rect) {
        let clipped = match self.clips.last() {
            Some(parent) => parent.intersection(&rect).unwrap_or(Rect::ZERO),
            None => rect,
        };
        self.clips.push(clipped);
    }

    pub fn pop_clip(&mut self) {
        self.clips.pop();
    }

    /// The clip currently in force.
    pub fn current_clip(&self) -> Rect {
        self.clips.last().copied().unwrap_or(Rect::UNBOUNDED)
    }

    /// Run `build` with `rect` clipped in, popping it afterwards even if the
    /// closure returns early.
    pub fn clipped(&mut self, rect: Rect, build: impl FnOnce(&mut Scene)) {
        self.push_clip(rect);
        build(self);
        self.pop_clip();
    }

    /// Declare an accessible element. Order matters: it is the order a
    /// keyboard walks the interface.
    pub fn push_access(&mut self, node: AccessNode) {
        self.access.push(node);
    }

    pub fn access(&self) -> &AccessTree {
        &self.access
    }

    pub fn access_mut(&mut self) -> &mut AccessTree {
        &mut self.access
    }

    pub fn push_quad(&mut self, layer: Layer, mut quad: Quad) {
        quad.clip = self.current_clip();
        if quad.is_invisible() {
            return;
        }
        // Damage covers only what can actually be painted.
        let painted = quad.bounds.intersection(&quad.clip).unwrap_or(quad.bounds);
        self.bounds = self.bounds.union(&painted);
        self.quads.push((layer, quad));
    }

    pub fn push_text(&mut self, layer: Layer, mut run: TextRun) {
        run.clip = self.current_clip();
        if run.bounds.is_empty()
            || run.text.is_empty()
            || run.color.a == 0
            || !run.bounds.intersects(&run.clip)
        {
            return;
        }
        let painted = run.bounds.intersection(&run.clip).unwrap_or(run.bounds);
        self.bounds = self.bounds.union(&painted);
        self.texts.push((layer, run));
    }

    /// Quads in paint order.
    pub fn quads(&self) -> impl Iterator<Item = &Quad> {
        Layer::ALL.iter().flat_map(move |layer| self.quads_in(*layer))
    }

    /// Text runs in paint order.
    pub fn texts(&self) -> impl Iterator<Item = &TextRun> {
        Layer::ALL.iter().flat_map(move |layer| self.texts_in(*layer))
    }

    /// Quads belonging to one layer, in insertion order.
    ///
    /// The renderer draws layer by layer rather than all quads then all text.
    /// Batching by primitive would be one draw call fewer, but it would also
    /// let content text show through an overlay panel that is supposed to
    /// cover it.
    pub fn quads_in(&self, layer: Layer) -> impl Iterator<Item = &Quad> {
        self.quads.iter().filter(move |(l, _)| *l == layer).map(|(_, quad)| quad)
    }

    /// Text runs belonging to one layer, in insertion order.
    pub fn texts_in(&self, layer: Layer) -> impl Iterator<Item = &TextRun> {
        self.texts.iter().filter(move |(l, _)| *l == layer).map(|(_, run)| run)
    }

    pub fn quad_count(&self) -> usize {
        self.quads.len()
    }

    pub fn text_count(&self) -> usize {
        self.texts.len()
    }

    pub fn is_empty(&self) -> bool {
        self.quads.is_empty() && self.texts.is_empty()
    }

    pub fn bounds(&self) -> Rect {
        self.bounds
    }

    pub fn clear(&mut self) {
        self.quads.clear();
        self.texts.clear();
        self.clips.clear();
        self.access.clear();
        self.bounds = Rect::ZERO;
    }

    /// Region that changed between two scenes.
    ///
    /// `None` means the frame is identical and can be skipped outright. This is
    /// the coarse fallback: it unions the bounds of everything that differs.
    /// A per-node invalidation pass will narrow it further, but even this
    /// version already prevents a full-surface repaint on an idle frame.
    pub fn damage_against(&self, previous: &Scene) -> Option<Rect> {
        if self == previous {
            return None;
        }

        let mut damage = Rect::ZERO;
        let mut mark = |rect: &Rect| damage = damage.union(rect);

        let mut changed = false;
        for (index, (layer, quad)) in self.quads.iter().enumerate() {
            match previous.quads.get(index) {
                Some((prev_layer, prev)) if prev_layer == layer && prev == quad => {}
                Some((_, prev)) => {
                    mark(&prev.bounds);
                    mark(&quad.bounds);
                    changed = true;
                }
                None => {
                    mark(&quad.bounds);
                    changed = true;
                }
            }
        }
        for (_, quad) in previous.quads.iter().skip(self.quads.len()) {
            mark(&quad.bounds);
            changed = true;
        }

        for (index, (layer, run)) in self.texts.iter().enumerate() {
            match previous.texts.get(index) {
                Some((prev_layer, prev)) if prev_layer == layer && prev == run => {}
                Some((_, prev)) => {
                    mark(&prev.bounds);
                    mark(&run.bounds);
                    changed = true;
                }
                None => {
                    mark(&run.bounds);
                    changed = true;
                }
            }
        }
        for (_, run) in previous.texts.iter().skip(self.texts.len()) {
            mark(&run.bounds);
            changed = true;
        }

        changed.then_some(damage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use z_tokens::Typography;

    fn quad(x: f32, y: f32) -> Quad {
        Quad::filled(Rect::new(x, y, 10.0, 10.0), Rgba::hex(0x123456))
    }

    #[test]
    fn invisible_quads_are_dropped_before_they_reach_the_gpu() {
        let mut scene = Scene::new();
        scene.push_quad(Layer::Content, Quad::filled(Rect::ZERO, Rgba::hex(0xFFFFFF)));
        scene.push_quad(
            Layer::Content,
            Quad::filled(Rect::new(0.0, 0.0, 10.0, 10.0), Rgba::TRANSPARENT),
        );
        assert_eq!(scene.quad_count(), 0);
    }

    #[test]
    fn a_transparent_fill_with_a_border_still_draws() {
        let mut scene = Scene::new();
        scene.push_quad(
            Layer::Content,
            Quad::filled(Rect::new(0.0, 0.0, 10.0, 10.0), Rgba::TRANSPARENT)
                .with_border(Rgba::hex(0xFFFFFF), 1.0),
        );
        assert_eq!(scene.quad_count(), 1);
    }

    #[test]
    fn empty_text_is_dropped() {
        let mut scene = Scene::new();
        scene.push_text(
            Layer::Content,
            TextRun::new(
                "",
                Rect::new(0.0, 0.0, 100.0, 20.0),
                Typography::BODY,
                Rgba::hex(0xFFFFFF),
            ),
        );
        assert_eq!(scene.text_count(), 0);
    }

    #[test]
    fn layers_paint_from_background_to_focus() {
        let mut scene = Scene::new();
        scene.push_quad(Layer::Focus, quad(3.0, 0.0));
        scene.push_quad(Layer::Background, quad(1.0, 0.0));
        scene.push_quad(Layer::Overlay, quad(2.0, 0.0));

        let order: Vec<f32> = scene.quads().map(|q| q.bounds.x).collect();
        assert_eq!(order, [1.0, 2.0, 3.0], "focus rings must paint last");
    }

    #[test]
    fn insertion_order_is_kept_within_a_layer() {
        let mut scene = Scene::new();
        for i in 0..5 {
            scene.push_quad(Layer::Content, quad(i as f32, 0.0));
        }
        let order: Vec<f32> = scene.quads().map(|q| q.bounds.x).collect();
        assert_eq!(order, [0.0, 1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn an_unchanged_scene_reports_no_damage() {
        let mut a = Scene::new();
        a.push_quad(Layer::Content, quad(0.0, 0.0));
        let b = a.clone();
        assert_eq!(b.damage_against(&a), None, "an idle frame must not repaint");
    }

    #[test]
    fn damage_covers_only_the_quad_that_moved() {
        let mut before = Scene::new();
        before.push_quad(Layer::Content, quad(0.0, 0.0));
        before.push_quad(Layer::Content, quad(500.0, 500.0));

        let mut after = before.clone();
        after.clear();
        after.push_quad(Layer::Content, quad(0.0, 0.0));
        after.push_quad(Layer::Content, quad(500.0, 520.0));

        let damage = after.damage_against(&before).expect("something moved");
        assert!(damage.y >= 500.0, "damage reached back to an untouched quad: {damage:?}");
        assert!(damage.bottom() <= 530.1);
    }

    #[test]
    fn a_removed_quad_still_damages_the_area_it_vacated() {
        let mut before = Scene::new();
        before.push_quad(Layer::Content, quad(0.0, 0.0));
        before.push_quad(Layer::Content, quad(200.0, 200.0));
        let mut after = Scene::new();
        after.push_quad(Layer::Content, quad(0.0, 0.0));

        let damage = after.damage_against(&before).expect("a quad disappeared");
        assert!(damage.contains(crate::geometry::Point::new(205.0, 205.0)));
    }

    #[test]
    fn scene_bounds_grow_to_cover_everything_added() {
        let mut scene = Scene::new();
        scene.push_quad(Layer::Content, quad(0.0, 0.0));
        scene.push_quad(Layer::Content, quad(90.0, 90.0));
        assert_eq!(scene.bounds(), Rect::new(0.0, 0.0, 100.0, 100.0));
    }

    #[test]
    fn direction_neutral_runs_can_be_marked_for_isolation() {
        let run = TextRun::new(
            "src/auth/session.ts",
            Rect::new(0.0, 0.0, 200.0, 20.0),
            Typography::CODE,
            Rgba::hex(0xFFFFFF),
        )
        .isolated();
        assert!(run.isolate, "a path inside RTL prose must not be reordered");
    }
}
