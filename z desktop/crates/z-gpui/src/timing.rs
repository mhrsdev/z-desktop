//! Frame timing and budgets.
//!
//! Until this module existed, `FrameStats` could say how much work a frame did
//! but not how long it took — which meant no budget could be enforced and no
//! benchmark meant anything. This closes that.
//!
//! Two rules shape the design:
//!
//! - **Measuring must not cost.** Stage records live in a fixed array indexed
//!   by stage; the rolling history is a ring buffer. Nothing allocates in the
//!   frame path.
//! - **The worst case is the number that matters.** A stall of half a second
//!   once a minute is worse than a 10% drop in the average, so the history
//!   reports p50/p95/p99 and missed frames — never a bare mean.

use std::time::{Duration, Instant};

/// A stage of the frame pipeline.
///
/// These are exactly the stages named in the architecture: every one carries a
/// budget, and a stage that is not listed here cannot be measured, which is the
/// point — work has to declare where it belongs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Stage {
    Input,
    Update,
    Layout,
    SceneDiff,
    Render,
    Present,
}

impl Stage {
    pub const ALL: [Stage; 6] = [
        Stage::Input,
        Stage::Update,
        Stage::Layout,
        Stage::SceneDiff,
        Stage::Render,
        Stage::Present,
    ];

    pub const fn index(self) -> usize {
        match self {
            Stage::Input => 0,
            Stage::Update => 1,
            Stage::Layout => 2,
            Stage::SceneDiff => 3,
            Stage::Render => 4,
            Stage::Present => 5,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Stage::Input => "input",
            Stage::Update => "update",
            Stage::Layout => "layout",
            Stage::SceneDiff => "scene-diff",
            Stage::Render => "render",
            Stage::Present => "present",
        }
    }
}

/// How long each stage of one frame took.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameTiming {
    stages: [Duration; Stage::ALL.len()],
    /// Wall time from the start of the frame to the end of present.
    pub total: Duration,
}

impl FrameTiming {
    pub fn stage(&self, stage: Stage) -> Duration {
        self.stages[stage.index()]
    }

    /// The stage that took longest — where to look first when a frame overruns.
    pub fn slowest(&self) -> (Stage, Duration) {
        Stage::ALL
            .iter()
            .map(|stage| (*stage, self.stage(*stage)))
            .max_by_key(|(_, d)| *d)
            .expect("Stage::ALL is never empty")
    }

    /// Time not attributed to any stage. A large value means work is happening
    /// outside the measured pipeline, which is itself worth knowing.
    pub fn unattributed(&self) -> Duration {
        let measured: Duration = self.stages.iter().sum();
        self.total.saturating_sub(measured)
    }
}

/// Records stage durations for a single frame.
///
/// Usage is deliberately explicit — `begin` then `end` — rather than a guard
/// type, so a stage that is never closed shows up as zero rather than silently
/// absorbing the time of whatever ran next.
#[derive(Debug)]
pub struct FrameTimer {
    frame_start: Instant,
    stage_start: Option<(Stage, Instant)>,
    timing: FrameTiming,
}

impl FrameTimer {
    pub fn start() -> Self {
        Self { frame_start: Instant::now(), stage_start: None, timing: FrameTiming::default() }
    }

    /// Begin a stage. Closes the previous one if it was left open.
    pub fn begin(&mut self, stage: Stage) {
        self.end();
        self.stage_start = Some((stage, Instant::now()));
    }

    /// Close the open stage, if any. Accumulates, so a stage entered twice in
    /// one frame sums rather than overwrites.
    pub fn end(&mut self) {
        if let Some((stage, started)) = self.stage_start.take() {
            let elapsed = started.elapsed();
            let slot = &mut self.timing.stages[stage.index()];
            *slot = slot.saturating_add(elapsed);
        }
    }

    /// Close out the frame and return what it cost.
    pub fn finish(mut self) -> FrameTiming {
        self.end();
        self.timing.total = self.frame_start.elapsed();
        self.timing
    }
}

/// Per-stage time limits.
///
/// Budgets are per hardware tier: a single number for every machine is
/// meaningless. `frame` is derived from the display's actual refresh rate, not
/// an assumed 60 Hz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameBudget {
    stages: [Duration; Stage::ALL.len()],
    /// Total time available for one frame.
    pub frame: Duration,
    /// The latency the user actually feels: action to visible result.
    pub input_to_present: Duration,
}

impl FrameBudget {
    /// Budget derived from a display refresh rate and an input-latency target.
    ///
    /// Stage shares are a starting point, not a measurement. They are
    /// deliberately conservative and must be re-derived from benchmark data —
    /// see `docs/z-gpui-BENCHMARK-PLAN.md`.
    pub fn for_refresh_rate(hz: f32, input_to_present: Duration) -> Self {
        let hz = hz.max(1.0);
        let frame = Duration::from_secs_f32(1.0 / hz);
        let share = |fraction: f32| frame.mul_f32(fraction);
        Self {
            stages: [
                share(0.05), // input
                share(0.15), // update
                share(0.15), // layout
                share(0.10), // scene diff
                share(0.45), // render
                share(0.10), // present
            ],
            frame,
            input_to_present,
        }
    }

    /// Tier M default: 120 Hz display, 50 ms input-to-present at p99.
    pub fn tier_m() -> Self {
        Self::for_refresh_rate(120.0, Duration::from_millis(50))
    }

    /// Tier L default: 60 Hz display, 80 ms input-to-present at p99.
    pub fn tier_l() -> Self {
        Self::for_refresh_rate(60.0, Duration::from_millis(80))
    }

    pub fn stage(&self, stage: Stage) -> Duration {
        self.stages[stage.index()]
    }

    /// Stages that overran, with what they cost and what they were allowed.
    pub fn overruns(&self, timing: &FrameTiming) -> Vec<Overrun> {
        Stage::ALL
            .iter()
            .filter_map(|stage| {
                let spent = timing.stage(*stage);
                let allowed = self.stage(*stage);
                (spent > allowed).then_some(Overrun { stage: *stage, spent, allowed })
            })
            .collect()
    }

    pub fn frame_overran(&self, timing: &FrameTiming) -> bool {
        timing.total > self.frame
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Overrun {
    pub stage: Stage,
    pub spent: Duration,
    pub allowed: Duration,
}

impl std::fmt::Display for Overrun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} took {:.2}ms, budget {:.2}ms",
            self.stage.name(),
            self.spent.as_secs_f64() * 1000.0,
            self.allowed.as_secs_f64() * 1000.0
        )
    }
}

/// Rolling window of recent frames.
///
/// A ring buffer rather than a growing vector: a session running for hours must
/// not accumulate timing data forever. Percentiles are computed on demand from
/// a copy, so reading stats never disturbs the recording.
#[derive(Debug)]
pub struct FrameHistory {
    frames: Vec<FrameTiming>,
    capacity: usize,
    next: usize,
    /// Frames whose total exceeded the budget.
    missed: u64,
    observed: u64,
}

impl FrameHistory {
    pub fn new(capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self { frames: Vec::with_capacity(capacity), capacity, next: 0, missed: 0, observed: 0 }
    }

    pub fn record(&mut self, timing: FrameTiming, budget: &FrameBudget) {
        if budget.frame_overran(&timing) {
            self.missed += 1;
        }
        self.observed += 1;

        if self.frames.len() < self.capacity {
            self.frames.push(timing);
        } else {
            self.frames[self.next] = timing;
        }
        self.next = (self.next + 1) % self.capacity;
    }

    pub fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Frames observed since the history was created — not just those retained.
    pub fn observed(&self) -> u64 {
        self.observed
    }

    pub fn missed(&self) -> u64 {
        self.missed
    }

    /// Fraction of observed frames that overran. This, not average FPS, is the
    /// number that tracks whether the interface feels responsive.
    pub fn missed_fraction(&self) -> f32 {
        if self.observed == 0 {
            0.0
        } else {
            self.missed as f32 / self.observed as f32
        }
    }

    /// Percentile of total frame time. `p` is 0.0 to 1.0.
    pub fn percentile(&self, p: f32) -> Duration {
        self.percentile_of(p, |t| t.total)
    }

    /// Percentile of one stage.
    pub fn stage_percentile(&self, stage: Stage, p: f32) -> Duration {
        self.percentile_of(p, |t| t.stage(stage))
    }

    fn percentile_of(&self, p: f32, pick: impl Fn(&FrameTiming) -> Duration) -> Duration {
        if self.frames.is_empty() {
            return Duration::ZERO;
        }
        let mut values: Vec<Duration> = self.frames.iter().map(&pick).collect();
        values.sort_unstable();
        // Nearest-rank: index of the smallest value at or above the percentile.
        let rank = (p.clamp(0.0, 1.0) * (values.len() - 1) as f32).round() as usize;
        values[rank.min(values.len() - 1)]
    }

    /// A reportable summary. Deliberately has no `average` field.
    pub fn summary(&self) -> TimingSummary {
        TimingSummary {
            frames: self.frames.len(),
            observed: self.observed,
            missed: self.missed,
            missed_fraction: self.missed_fraction(),
            p50: self.percentile(0.50),
            p95: self.percentile(0.95),
            p99: self.percentile(0.99),
        }
    }

    pub fn clear(&mut self) {
        self.frames.clear();
        self.next = 0;
        self.missed = 0;
        self.observed = 0;
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimingSummary {
    pub frames: usize,
    pub observed: u64,
    pub missed: u64,
    pub missed_fraction: f32,
    pub p50: Duration,
    pub p95: Duration,
    pub p99: Duration,
}

impl std::fmt::Display for TimingSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        write!(
            f,
            "{} frames · p50 {:.2}ms · p95 {:.2}ms · p99 {:.2}ms · missed {}/{} ({:.1}%)",
            self.frames,
            ms(self.p50),
            ms(self.p95),
            ms(self.p99),
            self.missed,
            self.observed,
            self.missed_fraction * 100.0
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing(stages: [u64; 6], total: u64) -> FrameTiming {
        FrameTiming {
            stages: stages.map(Duration::from_micros),
            total: Duration::from_micros(total),
        }
    }

    #[test]
    fn every_stage_has_a_unique_slot() {
        let mut seen = std::collections::BTreeSet::new();
        for stage in Stage::ALL {
            assert!(seen.insert(stage.index()), "{stage:?} collides with another stage");
        }
        assert_eq!(seen.len(), Stage::ALL.len());
    }

    #[test]
    fn stage_names_are_distinct() {
        let names: std::collections::BTreeSet<_> = Stage::ALL.iter().map(|s| s.name()).collect();
        assert_eq!(names.len(), Stage::ALL.len());
    }

    #[test]
    fn the_timer_attributes_time_to_the_open_stage() {
        let mut timer = FrameTimer::start();
        timer.begin(Stage::Layout);
        std::thread::sleep(Duration::from_millis(2));
        timer.end();
        let timing = timer.finish();

        assert!(timing.stage(Stage::Layout) >= Duration::from_millis(2));
        assert_eq!(timing.stage(Stage::Render), Duration::ZERO);
        assert!(timing.total >= timing.stage(Stage::Layout));
    }

    #[test]
    fn beginning_a_new_stage_closes_the_previous_one() {
        let mut timer = FrameTimer::start();
        timer.begin(Stage::Update);
        std::thread::sleep(Duration::from_millis(2));
        timer.begin(Stage::Render);
        let timing = timer.finish();

        assert!(timing.stage(Stage::Update) >= Duration::from_millis(2));
    }

    #[test]
    fn re_entering_a_stage_accumulates_rather_than_overwrites() {
        let mut timer = FrameTimer::start();
        timer.begin(Stage::Render);
        std::thread::sleep(Duration::from_millis(2));
        timer.end();
        timer.begin(Stage::Render);
        std::thread::sleep(Duration::from_millis(2));
        let timing = timer.finish();

        assert!(
            timing.stage(Stage::Render) >= Duration::from_millis(4),
            "a stage entered twice should sum, got {:?}",
            timing.stage(Stage::Render)
        );
    }

    #[test]
    fn unattributed_time_is_visible_rather_than_hidden() {
        let t = timing([100, 100, 100, 100, 100, 100], 1_000);
        assert_eq!(t.unattributed(), Duration::from_micros(400));
    }

    #[test]
    fn unattributed_never_underflows() {
        // Measured stages summing above the total is possible under clock skew;
        // it must not wrap around to an enormous duration.
        let t = timing([500, 500, 500, 0, 0, 0], 100);
        assert_eq!(t.unattributed(), Duration::ZERO);
    }

    #[test]
    fn slowest_stage_points_at_the_real_culprit() {
        let t = timing([10, 20, 30, 40, 900, 50], 1_050);
        assert_eq!(t.slowest().0, Stage::Render);
    }

    #[test]
    fn budget_derives_from_the_actual_refresh_rate() {
        let sixty = FrameBudget::for_refresh_rate(60.0, Duration::from_millis(80));
        let one_twenty = FrameBudget::for_refresh_rate(120.0, Duration::from_millis(50));

        assert!(sixty.frame > one_twenty.frame, "no artificial 60 Hz cap");
        assert!((sixty.frame.as_secs_f32() - 1.0 / 60.0).abs() < 1e-6);
    }

    #[test]
    fn stage_budgets_do_not_exceed_the_frame() {
        for budget in [FrameBudget::tier_l(), FrameBudget::tier_m()] {
            let sum: Duration = Stage::ALL.iter().map(|s| budget.stage(*s)).sum();
            assert!(
                sum <= budget.frame,
                "stage budgets sum to more than one frame: {sum:?} > {:?}",
                budget.frame
            );
        }
    }

    #[test]
    fn an_overrun_names_the_stage_and_both_numbers() {
        let budget = FrameBudget::for_refresh_rate(60.0, Duration::from_millis(80));
        // Render's share of a 16.6ms frame is well under 10ms.
        let t = timing([0, 0, 0, 0, 10_000, 0], 10_000);

        let overruns = budget.overruns(&t);
        assert_eq!(overruns.len(), 1);
        assert_eq!(overruns[0].stage, Stage::Render);
        assert!(overruns[0].spent > overruns[0].allowed);
        assert!(overruns[0].to_string().contains("render"));
    }

    #[test]
    fn a_frame_within_budget_reports_no_overruns() {
        let budget = FrameBudget::tier_m();
        let t = timing([10, 10, 10, 10, 10, 10], 100);
        assert!(budget.overruns(&t).is_empty());
        assert!(!budget.frame_overran(&t));
    }

    #[test]
    fn history_reports_percentiles_not_an_average() {
        let budget = FrameBudget::tier_m();
        let mut history = FrameHistory::new(100);
        // 99 fast frames and one very slow one: an average would hide the stall.
        for _ in 0..99 {
            history.record(timing([0; 6], 1_000), &budget);
        }
        history.record(timing([0; 6], 500_000), &budget);

        assert_eq!(history.percentile(0.50), Duration::from_micros(1_000));
        assert_eq!(history.percentile(1.0), Duration::from_micros(500_000));
        assert!(history.missed() >= 1, "the stall must be counted as a missed frame");
    }

    #[test]
    fn history_is_bounded_so_a_long_session_cannot_grow_forever() {
        let budget = FrameBudget::tier_m();
        let mut history = FrameHistory::new(8);
        for i in 0..1_000 {
            history.record(timing([0; 6], i), &budget);
        }
        assert_eq!(history.len(), 8);
        assert_eq!(history.observed(), 1_000);
    }

    #[test]
    fn the_ring_buffer_keeps_the_most_recent_frames() {
        let budget = FrameBudget::for_refresh_rate(60.0, Duration::from_millis(80));
        let mut history = FrameHistory::new(4);
        for i in 1..=10u64 {
            history.record(timing([0; 6], i * 100), &budget);
        }
        // The last four recorded were 700..1000 microseconds.
        assert_eq!(history.percentile(0.0), Duration::from_micros(700));
        assert_eq!(history.percentile(1.0), Duration::from_micros(1_000));
    }

    #[test]
    fn an_empty_history_reports_zero_rather_than_panicking() {
        let history = FrameHistory::new(16);
        assert!(history.is_empty());
        assert_eq!(history.percentile(0.99), Duration::ZERO);
        assert_eq!(history.missed_fraction(), 0.0);
    }

    #[test]
    fn percentile_arguments_outside_the_range_are_clamped() {
        let budget = FrameBudget::tier_m();
        let mut history = FrameHistory::new(4);
        for i in 1..=4u64 {
            history.record(timing([0; 6], i * 100), &budget);
        }
        assert_eq!(history.percentile(-5.0), history.percentile(0.0));
        assert_eq!(history.percentile(9.0), history.percentile(1.0));
    }

    #[test]
    fn stage_percentiles_are_tracked_separately() {
        let budget = FrameBudget::tier_m();
        let mut history = FrameHistory::new(16);
        history.record(timing([0, 0, 0, 0, 100, 0], 100), &budget);
        history.record(timing([0, 0, 0, 0, 900, 0], 900), &budget);

        assert_eq!(history.stage_percentile(Stage::Render, 1.0), Duration::from_micros(900));
        assert_eq!(history.stage_percentile(Stage::Layout, 1.0), Duration::ZERO);
    }

    #[test]
    fn the_summary_carries_no_average_field() {
        let budget = FrameBudget::tier_m();
        let mut history = FrameHistory::new(4);
        history.record(timing([0; 6], 1_000), &budget);
        let rendered = history.summary().to_string();
        assert!(rendered.contains("p99"));
        assert!(!rendered.contains("avg"), "a bare mean hides exactly the stalls we care about");
    }
}
