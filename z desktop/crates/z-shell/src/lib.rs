//! The Personal workspace shell model.
//!
//! This crate holds the *shape* of the workspace and nothing else. It has no
//! GPU dependency, no window handle and no agent logic, which is what lets the
//! whole layout system be tested headlessly.
//!
//! ```text
//! PanelRegistry   what each region is allowed to do        (authority)
//! LayoutState     where the user has put things            (their choice)
//! ViewState       what is inside each region               (their place in the work)
//! Workspace       the three of them, resolved together
//! ```

#![forbid(unsafe_code)]

pub mod dock_indicators;
pub mod layout;
pub mod panel;
pub mod region;
pub mod view;

pub use dock_indicators::{compute_drop_indicator, DropIndicator, DropZone};

pub use layout::{LayoutError, LayoutState, Origin, PanelPlacement, Preset};
pub use panel::{Capabilities, Constraints, Dock, PanelId, PanelRegistry, PanelSpec};
pub use region::{Rect, ShellFrame};
pub use view::{
    ContextSection, DiffTool, IdeTool, NavItem, PreviewTool, SurfaceKind, Tab, TabStrip,
    ThreeDTool, ViewState,
};

/// Everything the renderer needs in order to draw a frame of the shell.
#[derive(Debug, Clone)]
pub struct Workspace {
    pub registry: PanelRegistry,
    pub layout: LayoutState,
    pub view: ViewState,
}

impl Workspace {
    /// The reference Personal Agent Workspace.
    pub fn personal_default() -> Self {
        let registry = PanelRegistry::personal_default();
        let layout = LayoutState::personal_default(&registry);
        Self { registry, layout, view: ViewState::personal_default() }
    }

    /// Switch preset. Layout changes; view state is untouched by construction,
    /// because the two live in different fields and no code path copies between
    /// them.
    pub fn apply_preset(&mut self, preset: Preset) {
        self.layout = LayoutState::from_preset(preset, &self.registry);
    }

    /// Resolve panel rectangles for a window of this size.
    pub fn frame(&self, width: f32, height: f32) -> ShellFrame {
        region::solve(&self.registry, &self.layout, width, height)
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::personal_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_preset_preserves_view_state() {
        let mut workspace = Workspace::personal_default();
        let ide = workspace.view.tabs.tabs[1].id;
        workspace.view.tabs.activate(ide);
        workspace.view.scroll.insert(PanelId::Chat, 980.0);
        workspace.view.toggle_section(ContextSection::Plan);

        workspace.apply_preset(Preset::UltraCompact);

        assert_eq!(workspace.view.tabs.active, ide, "preset switch lost the active tab");
        assert_eq!(workspace.view.scroll[&PanelId::Chat], 980.0, "preset switch lost scroll");
        assert!(workspace.view.is_expanded(ContextSection::Plan), "preset switch lost disclosure");
    }

    #[test]
    fn every_preset_still_renders_a_usable_frame() {
        let mut workspace = Workspace::personal_default();
        for preset in Preset::BUILT_IN {
            workspace.apply_preset(*preset);
            let frame = workspace.frame(1536.0, 1024.0);
            assert!(
                frame.chat.width > 0.0 && frame.chat.height > 0.0,
                "{} leaves no room for chat",
                preset.label()
            );
        }
    }
}
