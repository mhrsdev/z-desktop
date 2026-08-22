//! Virtualization.
//!
//! The rule from the architecture: any surface that can show more than a screen
//! of content renders only what is visible. Chat history, the file tree, the
//! code surface and the terminal all need this, and retrofitting it later means
//! rewriting whatever was built without it.
//!
//! The core question a virtual list answers is *which range of items overlaps
//! the viewport*, and it has to answer it without touching every item — that is
//! the whole point. Two strategies:
//!
//! - [`UniformHeights`]: every row the same height. Answering is arithmetic.
//! - [`VariableHeights`]: rows differ. Answers from a prefix-sum table, with
//!   estimates for rows not yet measured, so an unmeasured list still scrolls.
//!
//! Both deliberately return a small over-scan beyond the viewport. Rendering a
//! couple of rows that are not strictly visible is far cheaper than a blank
//! band appearing during a fast scroll.

use crate::geometry::Rect;

/// The slice of a list that should be built for the current scroll position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VisibleRange {
    /// First item to build, inclusive.
    pub start: usize,
    /// One past the last item to build.
    pub end: usize,
}

impl VisibleRange {
    pub const EMPTY: VisibleRange = VisibleRange { start: 0, end: 0 };

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.end <= self.start
    }

    pub fn contains(&self, index: usize) -> bool {
        index >= self.start && index < self.end
    }

    pub fn iter(&self) -> std::ops::Range<usize> {
        self.start..self.end
    }
}

/// How a list decides where each row sits.
pub trait ItemMetrics {
    /// Total scrollable extent.
    fn total_height(&self) -> f32;

    /// Offset of an item from the top of the content.
    fn offset_of(&self, index: usize) -> f32;

    /// Height of one item.
    fn height_of(&self, index: usize) -> f32;

    fn count(&self) -> usize;

    /// Index of the item containing `offset`, clamped into range.
    ///
    /// Must not be linear in `count`, or virtualization buys nothing.
    fn index_at(&self, offset: f32) -> usize;
}

/// Rows of identical height.
///
/// Every query is arithmetic, so cost is independent of how many items exist.
/// Chat message groups, list rows and terminal lines all fit this once their
/// row height is fixed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UniformHeights {
    row_height: f32,
    count: usize,
}

impl UniformHeights {
    pub fn new(row_height: f32, count: usize) -> Self {
        Self { row_height: row_height.max(1.0), count }
    }

    pub fn set_count(&mut self, count: usize) {
        self.count = count;
    }

    pub fn row_height(&self) -> f32 {
        self.row_height
    }
}

impl ItemMetrics for UniformHeights {
    fn total_height(&self) -> f32 {
        self.row_height * self.count as f32
    }

    fn offset_of(&self, index: usize) -> f32 {
        self.row_height * index.min(self.count) as f32
    }

    fn height_of(&self, _index: usize) -> f32 {
        self.row_height
    }

    fn count(&self) -> usize {
        self.count
    }

    fn index_at(&self, offset: f32) -> usize {
        if self.count == 0 {
            return 0;
        }
        let index = (offset.max(0.0) / self.row_height).floor() as usize;
        index.min(self.count - 1)
    }
}

/// Rows of differing height.
///
/// Chat messages are the motivating case: a one-line reply and a message with a
/// plan block are wildly different heights, and neither is known until the row
/// has been laid out.
///
/// Unmeasured rows use an estimate, so a list scrolls correctly from the first
/// frame. As rows get measured the estimates are replaced and the prefix sums
/// rebuilt — which means scroll position shifts slightly as real heights
/// arrive. That is a real cost, and the alternative (measuring everything up
/// front) defeats the purpose.
#[derive(Debug, Clone, PartialEq)]
pub struct VariableHeights {
    /// Heights, estimated until measured.
    heights: Vec<f32>,
    /// Whether each entry is a real measurement.
    measured: Vec<bool>,
    /// Running offsets; `offsets[i]` is the top of item `i`, and the final entry
    /// is the total height.
    offsets: Vec<f32>,
    estimate: f32,
    dirty: bool,
}

impl VariableHeights {
    pub fn new(count: usize, estimate: f32) -> Self {
        let estimate = estimate.max(1.0);
        let mut metrics = Self {
            heights: vec![estimate; count],
            measured: vec![false; count],
            offsets: Vec::with_capacity(count + 1),
            estimate,
            dirty: true,
        };
        metrics.rebuild();
        metrics
    }

    /// Record a row's real height. Returns true if this changed anything.
    pub fn measure(&mut self, index: usize, height: f32) -> bool {
        let Some(slot) = self.heights.get_mut(index) else { return false };
        let height = height.max(0.0);
        if self.measured[index] && (*slot - height).abs() < f32::EPSILON {
            return false;
        }
        *slot = height;
        self.measured[index] = true;
        self.dirty = true;
        true
    }

    /// Grow or shrink the list, keeping heights already measured.
    pub fn set_count(&mut self, count: usize) {
        if count == self.heights.len() {
            return;
        }
        self.heights.resize(count, self.estimate);
        self.measured.resize(count, false);
        self.dirty = true;
    }

    /// How much of the list has real measurements. A low fraction means the
    /// scrollbar is still an approximation, which the UI may want to say.
    pub fn measured_fraction(&self) -> f32 {
        if self.measured.is_empty() {
            return 1.0;
        }
        let measured = self.measured.iter().filter(|m| **m).count();
        measured as f32 / self.measured.len() as f32
    }

    pub fn is_measured(&self, index: usize) -> bool {
        self.measured.get(index).copied().unwrap_or(false)
    }

    /// Recompute prefix sums. Called lazily so a burst of measurements in one
    /// frame costs one rebuild, not one per row.
    fn rebuild(&mut self) {
        self.offsets.clear();
        self.offsets.reserve(self.heights.len() + 1);
        let mut running = 0.0;
        self.offsets.push(0.0);
        for height in &self.heights {
            running += *height;
            self.offsets.push(running);
        }
        self.dirty = false;
    }

    /// Bring prefix sums up to date if measurements have landed since the last
    /// query. Cheap when nothing changed.
    pub fn settle(&mut self) {
        if self.dirty {
            self.rebuild();
        }
    }
}

impl ItemMetrics for VariableHeights {
    fn total_height(&self) -> f32 {
        self.offsets.last().copied().unwrap_or(0.0)
    }

    fn offset_of(&self, index: usize) -> f32 {
        self.offsets.get(index).copied().unwrap_or_else(|| self.total_height())
    }

    fn height_of(&self, index: usize) -> f32 {
        self.heights.get(index).copied().unwrap_or(0.0)
    }

    fn count(&self) -> usize {
        self.heights.len()
    }

    fn index_at(&self, offset: f32) -> usize {
        if self.heights.is_empty() {
            return 0;
        }
        let offset = offset.max(0.0);
        // Binary search over the prefix sums: logarithmic, not linear.
        match self.offsets.binary_search_by(|probe| {
            probe.partial_cmp(&offset).unwrap_or(std::cmp::Ordering::Equal)
        }) {
            Ok(exact) => exact.min(self.heights.len() - 1),
            Err(insert) => insert.saturating_sub(1).min(self.heights.len() - 1),
        }
    }
}

/// A scrollable list that builds only what is visible.
#[derive(Debug, Clone, PartialEq)]
pub struct VirtualList<M: ItemMetrics> {
    metrics: M,
    scroll: f32,
    viewport_height: f32,
    /// Extra rows built above and below the viewport, so a fast scroll does not
    /// expose a blank band before the next frame lands.
    overscan: usize,
    /// When true, new content keeps the view pinned to the end.
    stick_to_end: bool,
}

impl<M: ItemMetrics> VirtualList<M> {
    pub const DEFAULT_OVERSCAN: usize = 3;

    pub fn new(metrics: M) -> Self {
        Self {
            metrics,
            scroll: 0.0,
            viewport_height: 0.0,
            overscan: Self::DEFAULT_OVERSCAN,
            stick_to_end: false,
        }
    }

    pub fn with_overscan(mut self, rows: usize) -> Self {
        self.overscan = rows;
        self
    }

    pub fn metrics(&self) -> &M {
        &self.metrics
    }

    pub fn metrics_mut(&mut self) -> &mut M {
        &mut self.metrics
    }

    pub fn scroll_offset(&self) -> f32 {
        self.scroll
    }

    pub fn viewport_height(&self) -> f32 {
        self.viewport_height
    }

    pub fn set_viewport_height(&mut self, height: f32) {
        self.viewport_height = height.max(0.0);
        self.clamp_scroll();
    }

    /// Largest valid scroll offset. Zero when the content fits.
    pub fn max_scroll(&self) -> f32 {
        (self.metrics.total_height() - self.viewport_height).max(0.0)
    }

    pub fn set_scroll(&mut self, offset: f32) {
        self.scroll = offset;
        self.clamp_scroll();
        // Scrolling away from the bottom releases the pin; scrolling back to it
        // re-establishes it. This is what stops a chat yanking the reader to the
        // bottom while they are reading history further up.
        self.stick_to_end = self.is_at_end();
    }

    pub fn scroll_by(&mut self, delta: f32) {
        self.set_scroll(self.scroll + delta);
    }

    pub fn scroll_to_end(&mut self) {
        self.scroll = self.max_scroll();
        self.stick_to_end = true;
    }

    /// Whether the view is at the very bottom, within a pixel.
    pub fn is_at_end(&self) -> bool {
        (self.max_scroll() - self.scroll).abs() < 1.0
    }

    /// Whether new content will keep the view pinned to the bottom.
    pub fn sticks_to_end(&self) -> bool {
        self.stick_to_end
    }

    /// Call after content changes. Honours the pin if the reader was at the end.
    pub fn content_changed(&mut self) {
        if self.stick_to_end {
            self.scroll = self.max_scroll();
        } else {
            self.clamp_scroll();
        }
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.clamp(0.0, self.max_scroll());
    }

    /// The range of items to build for the current position.
    pub fn visible_range(&self) -> VisibleRange {
        let count = self.metrics.count();
        if count == 0 || self.viewport_height <= 0.0 {
            return VisibleRange::EMPTY;
        }

        let first = self.metrics.index_at(self.scroll);
        let last = self.metrics.index_at(self.scroll + self.viewport_height);

        VisibleRange {
            start: first.saturating_sub(self.overscan),
            end: (last + 1 + self.overscan).min(count),
        }
    }

    /// Where item `index` should be drawn, relative to the viewport's origin.
    ///
    /// The y may be negative or past the viewport for over-scanned rows: those
    /// are built on purpose and simply fall outside.
    pub fn item_bounds(&self, index: usize, viewport: Rect) -> Rect {
        let y = viewport.y + self.metrics.offset_of(index) - self.scroll;
        Rect::new(viewport.x, y, viewport.width, self.metrics.height_of(index))
    }

    /// Fraction of the content currently on screen, for a scrollbar thumb.
    /// `1.0` means everything fits.
    pub fn visible_fraction(&self) -> f32 {
        let total = self.metrics.total_height();
        if total <= 0.0 {
            return 1.0;
        }
        (self.viewport_height / total).clamp(0.0, 1.0)
    }

    /// Scroll so `index` is visible, moving as little as possible.
    pub fn scroll_into_view(&mut self, index: usize) {
        let top = self.metrics.offset_of(index);
        let bottom = top + self.metrics.height_of(index);
        if top < self.scroll {
            self.set_scroll(top);
        } else if bottom > self.scroll + self.viewport_height {
            self.set_scroll(bottom - self.viewport_height);
        }
    }
}

impl VirtualList<VariableHeights> {
    /// Record a measured row height and keep the view stable.
    ///
    /// Without the correction, measuring a row above the viewport shifts
    /// everything below it and the content appears to jump under the reader.
    pub fn measure_item(&mut self, index: usize, height: f32) {
        // Which row the viewport currently starts on, resolved against the old
        // offsets — after the measurement lands, this index means something
        // different.
        let anchor = self.metrics.index_at(self.scroll);
        let before = self.metrics.height_of(index);

        if !self.metrics.measure(index, height) {
            return;
        }
        self.metrics.settle();

        // Measuring a row changes its own height, not its own offset: what
        // moves is everything *below* it. So the correction applies only when
        // the measured row sits above the viewport, and it is the change in
        // height, not in offset.
        if index < anchor {
            self.scroll += self.metrics.height_of(index) - before;
        }
        self.content_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniform(count: usize) -> VirtualList<UniformHeights> {
        let mut list = VirtualList::new(UniformHeights::new(20.0, count)).with_overscan(0);
        list.set_viewport_height(100.0);
        list
    }

    #[test]
    fn only_the_visible_slice_is_built() {
        let list = uniform(10_000);
        let range = list.visible_range();
        assert!(
            range.len() <= 8,
            "a 100px viewport of 20px rows should build a handful, got {}",
            range.len()
        );
    }

    #[test]
    fn the_built_count_does_not_grow_with_the_dataset() {
        let small = uniform(100).visible_range().len();
        let huge = uniform(1_000_000).visible_range().len();
        assert_eq!(small, huge, "virtualization must be independent of list length");
    }

    #[test]
    fn scrolling_moves_the_window_not_its_size() {
        let mut list = uniform(1_000);
        let at_top = list.visible_range();
        list.set_scroll(5_000.0);
        let scrolled = list.visible_range();

        assert_eq!(at_top.len(), scrolled.len());
        assert!(scrolled.start > at_top.start);
        assert!(scrolled.contains(250), "offset 5000 / 20px per row lands on item 250");
    }

    #[test]
    fn an_empty_list_produces_an_empty_range() {
        let list = uniform(0);
        assert!(list.visible_range().is_empty());
        assert_eq!(list.max_scroll(), 0.0);
    }

    #[test]
    fn a_zero_height_viewport_builds_nothing() {
        let mut list = uniform(1_000);
        list.set_viewport_height(0.0);
        assert!(list.visible_range().is_empty());
    }

    #[test]
    fn scroll_is_clamped_to_the_content() {
        let mut list = uniform(10);
        list.set_scroll(-500.0);
        assert_eq!(list.scroll_offset(), 0.0);
        list.set_scroll(999_999.0);
        assert_eq!(list.scroll_offset(), list.max_scroll());
    }

    #[test]
    fn content_that_fits_cannot_scroll() {
        let mut list = VirtualList::new(UniformHeights::new(20.0, 3));
        list.set_viewport_height(500.0);
        assert_eq!(list.max_scroll(), 0.0);
        assert_eq!(list.visible_fraction(), 1.0);
    }

    #[test]
    fn overscan_builds_a_margin_beyond_the_viewport() {
        let mut plain = VirtualList::new(UniformHeights::new(20.0, 1_000)).with_overscan(0);
        plain.set_viewport_height(100.0);
        let mut padded = VirtualList::new(UniformHeights::new(20.0, 1_000)).with_overscan(3);
        padded.set_viewport_height(100.0);

        plain.set_scroll(1_000.0);
        padded.set_scroll(1_000.0);

        assert!(padded.visible_range().len() > plain.visible_range().len());
        assert!(padded.visible_range().start < plain.visible_range().start);
    }

    #[test]
    fn item_bounds_place_the_first_visible_row_at_the_viewport_top() {
        let mut list = uniform(1_000);
        list.set_scroll(200.0); // exactly item 10
        let viewport = Rect::new(0.0, 50.0, 300.0, 100.0);
        let bounds = list.item_bounds(10, viewport);
        assert!((bounds.y - viewport.y).abs() < 0.01);
        assert_eq!(bounds.height, 20.0);
    }

    #[test]
    fn sticking_to_the_end_survives_new_content() {
        let mut list = uniform(100);
        list.scroll_to_end();
        assert!(list.sticks_to_end());

        list.metrics_mut().set_count(200);
        list.content_changed();

        assert!(list.is_at_end(), "a reader at the bottom should stay at the bottom");
    }

    #[test]
    fn reading_history_is_not_yanked_to_the_bottom() {
        // The behaviour that makes a chat usable: new messages must not drag
        // the reader away from what they are reading.
        let mut list = uniform(100);
        list.set_scroll(200.0);
        assert!(!list.sticks_to_end());

        list.metrics_mut().set_count(200);
        list.content_changed();

        assert_eq!(list.scroll_offset(), 200.0, "the reader's position must not move");
    }

    #[test]
    fn scroll_into_view_moves_as_little_as_possible() {
        let mut list = uniform(1_000);
        list.set_scroll(400.0);
        let before = list.scroll_offset();

        // Already visible: no movement.
        list.scroll_into_view(22);
        assert_eq!(list.scroll_offset(), before);

        // Below: scroll down just far enough.
        list.scroll_into_view(30);
        assert!(list.scroll_offset() > before);
        assert!(list.visible_range().contains(30));

        // Above: scroll up to put it at the top.
        list.scroll_into_view(2);
        assert_eq!(list.scroll_offset(), 40.0);
    }

    // --- variable heights ---------------------------------------------------

    #[test]
    fn variable_heights_start_from_an_estimate() {
        let metrics = VariableHeights::new(100, 30.0);
        assert_eq!(metrics.total_height(), 3_000.0);
        assert_eq!(metrics.measured_fraction(), 0.0);
    }

    #[test]
    fn measuring_replaces_the_estimate() {
        let mut metrics = VariableHeights::new(3, 30.0);
        metrics.measure(1, 120.0);
        metrics.settle();

        assert_eq!(metrics.height_of(1), 120.0);
        assert_eq!(metrics.total_height(), 30.0 + 120.0 + 30.0);
        assert!(metrics.is_measured(1));
        assert!(!metrics.is_measured(0));
    }

    #[test]
    fn index_at_finds_the_right_row_with_mixed_heights() {
        let mut metrics = VariableHeights::new(4, 10.0);
        metrics.measure(0, 100.0);
        metrics.measure(1, 50.0);
        metrics.settle();
        // offsets: 0, 100, 150, 160, 170

        assert_eq!(metrics.index_at(0.0), 0);
        assert_eq!(metrics.index_at(99.0), 0);
        assert_eq!(metrics.index_at(100.0), 1);
        assert_eq!(metrics.index_at(149.0), 1);
        assert_eq!(metrics.index_at(155.0), 2);
        assert_eq!(metrics.index_at(9_999.0), 3, "past the end clamps to the last row");
    }

    #[test]
    fn measuring_above_the_viewport_does_not_shift_the_content() {
        // The bug this prevents: a row above the viewport gets its real height,
        // everything below moves, and the text the reader is looking at jumps.
        let mut list = VirtualList::new(VariableHeights::new(500, 30.0));
        list.set_viewport_height(300.0);
        list.set_scroll(3_000.0);

        let anchor = list.visible_range().start;
        let anchor_screen_y = list.item_bounds(anchor, Rect::new(0.0, 0.0, 100.0, 300.0)).y;

        // A row well above the viewport turns out to be much taller.
        list.measure_item(5, 200.0);

        let after = list.item_bounds(anchor, Rect::new(0.0, 0.0, 100.0, 300.0)).y;
        assert!(
            (after - anchor_screen_y).abs() < 1.0,
            "content jumped by {}px under the reader",
            (after - anchor_screen_y).abs()
        );
    }

    #[test]
    fn measuring_the_same_height_twice_is_a_no_op() {
        let mut metrics = VariableHeights::new(3, 30.0);
        assert!(metrics.measure(0, 55.0));
        metrics.settle();
        assert!(!metrics.measure(0, 55.0), "an unchanged measurement should not dirty the table");
    }

    #[test]
    fn growing_the_list_keeps_existing_measurements() {
        let mut metrics = VariableHeights::new(2, 30.0);
        metrics.measure(0, 90.0);
        metrics.set_count(5);
        metrics.settle();

        assert_eq!(metrics.height_of(0), 90.0);
        assert_eq!(metrics.count(), 5);
        assert_eq!(metrics.total_height(), 90.0 + 30.0 * 4.0);
    }

    #[test]
    fn a_variable_list_of_ten_thousand_still_builds_a_handful() {
        let mut list = VirtualList::new(VariableHeights::new(10_000, 40.0)).with_overscan(2);
        list.set_viewport_height(800.0);
        list.set_scroll(150_000.0);

        let range = list.visible_range();
        assert!(range.len() <= 26, "built {} rows for an 800px viewport", range.len());
        assert!(range.start > 3_000, "the window should have moved deep into the list");
    }

    #[test]
    fn an_out_of_range_measurement_is_ignored_rather_than_panicking() {
        let mut metrics = VariableHeights::new(3, 30.0);
        assert!(!metrics.measure(99, 100.0));
        assert_eq!(metrics.count(), 3);
    }
}
