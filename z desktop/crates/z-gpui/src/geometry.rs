//! Geometry primitives, in logical pixels.
//!
//! ZeroGPUI works in logical units throughout and converts to physical pixels
//! once, in the renderer, using the window's scale factor. Doing it anywhere
//! else is how a layout ends up correct on one display and blurry on another.

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };

    /// A clip region that excludes nothing.
    ///
    /// Large but finite: an infinite extent would produce NaN once it reached
    /// the shader's arithmetic.
    pub const UNBOUNDED: Rect = Rect { x: -1.0e6, y: -1.0e6, width: 2.0e6, height: 2.0e6 };

    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub const fn from_parts(origin: Point, size: Size) -> Self {
        Self { x: origin.x, y: origin.y, width: size.width, height: size.height }
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn center(&self) -> Point {
        Point::new(self.x + self.width / 2.0, self.y + self.height / 2.0)
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    pub fn contains(&self, point: Point) -> bool {
        point.x >= self.x && point.x < self.right() && point.y >= self.y && point.y < self.bottom()
    }

    pub fn inset(&self, amount: f32) -> Rect {
        self.inset_xy(amount, amount)
    }

    pub fn inset_xy(&self, horizontal: f32, vertical: f32) -> Rect {
        Rect {
            x: self.x + horizontal,
            y: self.y + vertical,
            width: (self.width - horizontal * 2.0).max(0.0),
            height: (self.height - vertical * 2.0).max(0.0),
        }
    }

    /// Shift the origin without changing the size.
    pub fn translate(&self, dx: f32, dy: f32) -> Rect {
        Rect { x: self.x + dx, y: self.y + dy, ..*self }
    }

    /// Intersection, or `None` when the two do not meet.
    ///
    /// This is the operation damage tracking is built on: a quad whose bounds do
    /// not intersect the damaged region is skipped entirely.
    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = self.right().min(other.right());
        let bottom = self.bottom().min(other.bottom());
        if right <= x || bottom <= y {
            None
        } else {
            Some(Rect::new(x, y, right - x, bottom - y))
        }
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.intersection(other).is_some()
    }

    /// Smallest rectangle containing both.
    pub fn union(&self, other: &Rect) -> Rect {
        if self.is_empty() {
            return *other;
        }
        if other.is_empty() {
            return *self;
        }
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        let right = self.right().max(other.right());
        let bottom = self.bottom().max(other.bottom());
        Rect::new(x, y, right - x, bottom - y)
    }

    /// Snap to whole physical pixels so edges stay crisp at any scale factor.
    pub fn round_to_pixel(&self, scale: f32) -> Rect {
        let px = |v: f32| (v * scale).round() / scale;
        let x = px(self.x);
        let y = px(self.y);
        Rect::new(x, y, px(self.right()) - x, px(self.bottom()) - y)
    }

    /// Take the left `width` and return it with the remainder.
    pub fn split_left(&self, width: f32) -> (Rect, Rect) {
        let width = width.clamp(0.0, self.width);
        (
            Rect::new(self.x, self.y, width, self.height),
            Rect::new(self.x + width, self.y, self.width - width, self.height),
        )
    }

    /// Take the top `height` and return it with the remainder.
    pub fn split_top(&self, height: f32) -> (Rect, Rect) {
        let height = height.clamp(0.0, self.height);
        (
            Rect::new(self.x, self.y, self.width, height),
            Rect::new(self.x, self.y + height, self.width, self.height - height),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intersection_of_disjoint_rects_is_none() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(20.0, 20.0, 10.0, 10.0);
        assert!(a.intersection(&b).is_none());
        assert!(!a.intersects(&b));
    }

    #[test]
    fn touching_edges_do_not_count_as_intersecting() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(10.0, 0.0, 10.0, 10.0);
        assert!(a.intersection(&b).is_none(), "adjacent panels must not both redraw");
    }

    #[test]
    fn intersection_is_the_overlapping_area() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 10.0, 10.0);
        assert_eq!(a.intersection(&b), Some(Rect::new(5.0, 5.0, 5.0, 5.0)));
    }

    #[test]
    fn union_with_an_empty_rect_is_the_other_rect() {
        let a = Rect::new(4.0, 4.0, 12.0, 12.0);
        assert_eq!(a.union(&Rect::ZERO), a);
        assert_eq!(Rect::ZERO.union(&a), a);
    }

    #[test]
    fn contains_excludes_the_far_edges() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert!(r.contains(Point::new(0.0, 0.0)));
        assert!(r.contains(Point::new(9.9, 9.9)));
        assert!(!r.contains(Point::new(10.0, 5.0)), "the right edge belongs to the next rect");
    }

    #[test]
    fn insetting_past_the_middle_clamps_to_zero_instead_of_inverting() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).inset(50.0);
        assert_eq!(r.width, 0.0);
        assert_eq!(r.height, 0.0);
    }

    #[test]
    fn pixel_snapping_preserves_adjacency_at_fractional_scale() {
        let scale = 1.25;
        let (left, right) = Rect::new(0.0, 0.0, 100.0, 50.0).split_left(37.3);
        let l = left.round_to_pixel(scale);
        let r = right.round_to_pixel(scale);
        assert!(
            (l.right() - r.x).abs() < 1.0 / scale,
            "snapping must not open a seam between adjacent panels"
        );
    }

    #[test]
    fn splitting_clamps_to_the_available_extent() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        let (taken, rest) = r.split_left(500.0);
        assert_eq!(taken.width, 100.0);
        assert_eq!(rest.width, 0.0);
    }

    #[test]
    fn split_top_partitions_the_height_exactly() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        let (top, bottom) = r.split_top(20.0);
        assert_eq!(top.height + bottom.height, r.height);
        assert_eq!(top.bottom(), bottom.y);
    }
}
