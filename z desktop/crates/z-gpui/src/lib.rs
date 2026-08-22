//! ZeroGPUI — the Personal-mode UI runtime.
//!
//! Owns input, layout, scene construction, text and rendering. It owns none of
//! the product: no agent, no router, no project model, no business logic. Those
//! subsystems reach the UI only as data, through versioned contracts, which is
//! what allows the UI to be restarted without killing running work.
//!
//! ```text
//! Input → Update → Layout → Scene diff → Render → Present
//! ```
//!
//! Each stage carries a budget. The measure of success is the distribution of
//! frame times and input-to-present latency, not average FPS.

#![forbid(unsafe_code)]

pub mod a11y;
pub mod a11y_platform;
pub mod geometry;
pub mod offscreen;
pub mod renderer;
pub mod scene;
pub mod text;
pub mod timing;
pub mod virtual_list;
pub mod window;

pub use a11y::{AccessNode, AccessTree, NodeId, NodeState, Role};
pub use a11y_platform::AccessRequest;
pub use geometry::{Point, Rect, Size};
pub use offscreen::{Capture, OffscreenRenderer};
pub use renderer::{BackendInfo, FrameStats};
pub use scene::{Align, Direction, Layer, Quad, Scene, TextRun};
pub use text::{contains_rtl_script, isolate, strip_directional_overrides, TextSystem};
pub use timing::{FrameBudget, FrameHistory, FrameTimer, FrameTiming, Stage, TimingSummary};
pub use virtual_list::{ItemMetrics, UniformHeights, VariableHeights, VirtualList, VisibleRange};
pub use window::{run, SceneSource, WindowConfig};

/// Scheduling priority for a unit of work.
///
/// Every task declares one. The default is [`Priority::BackgroundPreparation`],
/// not something frame-critical — work has to earn its way up the queue.
/// Nothing below `FrameCritical` may block `Input`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum Priority {
    /// P0 — never blocked by anything below it.
    Input,
    /// P1 — visible, needed for this frame.
    FrameCritical,
    /// P2 — interactive animation.
    Animation,
    /// P3 — visible, but the frame is correct without it.
    VisibleNonCritical,
    /// P4 — the default.
    #[default]
    BackgroundPreparation,
    /// P5 — speculative.
    Prefetch,
    /// P6 — housekeeping.
    CacheMaintenance,
}

impl Priority {
    /// Whether work at `self` is allowed to delay work at `other`.
    pub fn may_delay(self, other: Priority) -> bool {
        self <= other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_work_can_never_delay_input() {
        for priority in [
            Priority::Animation,
            Priority::VisibleNonCritical,
            Priority::BackgroundPreparation,
            Priority::Prefetch,
            Priority::CacheMaintenance,
        ] {
            assert!(!priority.may_delay(Priority::Input), "{priority:?} must never block P0 input");
            assert!(
                !priority.may_delay(Priority::FrameCritical),
                "{priority:?} must never block P1 frame-critical work"
            );
        }
    }

    #[test]
    fn input_outranks_everything() {
        for priority in [Priority::FrameCritical, Priority::Animation, Priority::CacheMaintenance] {
            assert!(Priority::Input.may_delay(priority));
        }
    }

    #[test]
    fn unstated_priority_is_background_not_critical() {
        assert_eq!(Priority::default(), Priority::BackgroundPreparation);
    }
}
