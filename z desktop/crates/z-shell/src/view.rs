//! View State — what is *inside* each panel.
//!
//! Held apart from [`crate::layout::LayoutState`] so that resizing a panel,
//! switching preset or hiding a region never costs the user their scroll
//! position, their active tab or their selection.
//!
//! The separation is the whole point of this module: if you find yourself
//! wanting to put a pixel width in here, or a scroll offset in `LayoutState`,
//! the boundary has been crossed.

use crate::panel::PanelId;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A surface that can occupy the centre region.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SurfaceKind {
    /// Carries the thread's display name, e.g. `Chat · Auth Flow`.
    Chat {
        title: String,
    },
    Ide,
    Live3d,
    Preview,
    Diff,
}

impl SurfaceKind {
    pub fn label(&self) -> String {
        match self {
            SurfaceKind::Chat { title } => format!("Chat · {title}"),
            SurfaceKind::Ide => "IDE".into(),
            SurfaceKind::Live3d => "Live 3D".into(),
            SurfaceKind::Preview => "Preview".into(),
            SurfaceKind::Diff => "Diff".into(),
        }
    }

    /// Whether a tab of this kind can be closed. The last chat is kept, because
    /// a workspace with no surface at all has nothing to return the user to.
    pub fn closable(&self) -> bool {
        !matches!(self, SurfaceKind::Chat { .. })
    }
}

/// The IDE tool currently shown beside the editor surface.
///
/// This is view state, not a project capability: choosing a tool changes what
/// the user sees even before a project is connected. Running code, reading
/// files and version-control operations remain separate runtime capabilities.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdeTool {
    #[default]
    Explorer,
    Search,
    Problems,
    SourceControl,
    RunDebug,
    Terminal,
    Extensions,
}

impl IdeTool {
    pub const ALL: &'static [IdeTool] = &[
        IdeTool::Explorer,
        IdeTool::Search,
        IdeTool::Problems,
        IdeTool::SourceControl,
        IdeTool::RunDebug,
        IdeTool::Terminal,
        IdeTool::Extensions,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            IdeTool::Explorer => "Explorer",
            IdeTool::Search => "Search",
            IdeTool::Problems => "Problems",
            IdeTool::SourceControl => "Source Control",
            IdeTool::RunDebug => "Run & Debug",
            IdeTool::Terminal => "Terminal",
            IdeTool::Extensions => "Extensions",
        }
    }
}

/// The 3D workspace control or information panel currently selected.
///
/// The set takes the common, task-oriented vocabulary from professional 3D
/// editors while remaining a Zero-owned view model rather than a copy of any
/// third-party product's UI.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ThreeDTool {
    #[default]
    Select,
    Move,
    Rotate,
    Scale,
    Frame,
    Orbit,
    Pan,
    Zoom,
    Outliner,
    Inspector,
}

impl ThreeDTool {
    pub const ALL: &'static [ThreeDTool] = &[
        ThreeDTool::Select,
        ThreeDTool::Move,
        ThreeDTool::Rotate,
        ThreeDTool::Scale,
        ThreeDTool::Frame,
        ThreeDTool::Orbit,
        ThreeDTool::Pan,
        ThreeDTool::Zoom,
        ThreeDTool::Outliner,
        ThreeDTool::Inspector,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            ThreeDTool::Select => "Select",
            ThreeDTool::Move => "Move",
            ThreeDTool::Rotate => "Rotate",
            ThreeDTool::Scale => "Scale",
            ThreeDTool::Frame => "Frame",
            ThreeDTool::Orbit => "Orbit",
            ThreeDTool::Pan => "Pan",
            ThreeDTool::Zoom => "Zoom",
            ThreeDTool::Outliner => "Outliner",
            ThreeDTool::Inspector => "Inspector",
        }
    }
}

/// Presentation controls that are safe before a live preview process exists.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewTool {
    #[default]
    Fit,
    Device,
    Inspect,
}

impl PreviewTool {
    pub const ALL: &'static [PreviewTool] =
        &[PreviewTool::Fit, PreviewTool::Device, PreviewTool::Inspect];

    pub const fn label(self) -> &'static str {
        match self {
            PreviewTool::Fit => "Fit",
            PreviewTool::Device => "Device",
            PreviewTool::Inspect => "Inspect",
        }
    }
}

/// A diff-view presentation mode. Applying or rejecting changes is deliberately
/// not represented here, because it requires a real reviewed diff backend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DiffTool {
    #[default]
    Files,
    Unified,
    Split,
}

impl DiffTool {
    pub const ALL: &'static [DiffTool] = &[DiffTool::Files, DiffTool::Unified, DiffTool::Split];

    pub const fn label(self) -> &'static str {
        match self {
            DiffTool::Files => "Files",
            DiffTool::Unified => "Unified",
            DiffTool::Split => "Split",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tab {
    pub id: u64,
    pub kind: SurfaceKind,
    /// Preserved per tab so switching away and back lands where the user left off.
    pub scroll_offset: f32,
}

/// Open tabs and which one is active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabStrip {
    pub tabs: Vec<Tab>,
    pub active: u64,
    next_id: u64,
}

impl TabStrip {
    pub fn with_reference_surfaces() -> Self {
        let kinds = [
            SurfaceKind::Chat { title: "Auth Flow".into() },
            SurfaceKind::Ide,
            SurfaceKind::Live3d,
            SurfaceKind::Preview,
            SurfaceKind::Diff,
        ];
        let tabs: Vec<Tab> = kinds
            .into_iter()
            .enumerate()
            .map(|(i, kind)| Tab { id: i as u64, kind, scroll_offset: 0.0 })
            .collect();
        let next_id = tabs.len() as u64;
        Self { tabs, active: 0, next_id }
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == self.active)
    }

    pub fn open(&mut self, kind: SurfaceKind) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab { id, kind, scroll_offset: 0.0 });
        self.active = id;
        id
    }

    /// Close a tab, moving focus to its neighbour.
    ///
    /// Refuses to close the last remaining tab, and refuses tabs whose kind is
    /// not closable — the caller gets `false` rather than an empty workspace.
    pub fn close(&mut self, id: u64) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        let Some(index) = self.tabs.iter().position(|tab| tab.id == id) else {
            return false;
        };
        if !self.tabs[index].kind.closable() {
            return false;
        }

        self.tabs.remove(index);
        if self.active == id {
            let neighbour = index.min(self.tabs.len() - 1);
            self.active = self.tabs[neighbour].id;
        }
        true
    }

    pub fn activate(&mut self, id: u64) -> bool {
        if self.tabs.iter().any(|tab| tab.id == id) {
            self.active = id;
            true
        } else {
            false
        }
    }
}

/// Which grouping in the Context Panel is expanded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextSection {
    Project,
    Branch,
    ContextUsage,
    Plan,
    Agents,
    Changes,
    Threads,
    Resources,
    Notes,
}

impl ContextSection {
    pub const ALL: &'static [ContextSection] = &[
        ContextSection::Project,
        ContextSection::Branch,
        ContextSection::ContextUsage,
        ContextSection::Plan,
        ContextSection::Agents,
        ContextSection::Changes,
        ContextSection::Threads,
        ContextSection::Resources,
        ContextSection::Notes,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            ContextSection::Project => "Project",
            ContextSection::Branch => "Branch",
            ContextSection::ContextUsage => "Context Usage",
            ContextSection::Plan => "Plan",
            ContextSection::Agents => "Agents",
            ContextSection::Changes => "Changes",
            ContextSection::Threads => "Threads",
            ContextSection::Resources => "Resources",
            ContextSection::Notes => "Notes",
        }
    }
}

/// Entries in the left navigation rail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NavItem {
    Home,
    Projects,
    Chat,
    Agents,
    Threads,
    Files,
    Search,
    Skills,
    Extensions,
    Settings,
}

impl NavItem {
    /// Settings is deliberately excluded here: it is pinned to the foot of the
    /// rail rather than sitting in the main scroll list.
    pub const PRIMARY: &'static [NavItem] = &[
        NavItem::Home,
        NavItem::Projects,
        NavItem::Chat,
        NavItem::Agents,
        NavItem::Threads,
        NavItem::Files,
        NavItem::Search,
        NavItem::Skills,
        NavItem::Extensions,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            NavItem::Home => "Home",
            NavItem::Projects => "Projects",
            NavItem::Chat => "Chat",
            NavItem::Agents => "Agents",
            NavItem::Threads => "Threads",
            NavItem::Files => "Files",
            NavItem::Search => "Search",
            NavItem::Skills => "Skills",
            NavItem::Extensions => "Extensions",
            NavItem::Settings => "Settings",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    pub tabs: TabStrip,
    pub nav_selection: NavItem,
    /// Tool choices belong to the visible surface and survive layout changes.
    #[serde(default)]
    pub ide_tool: IdeTool,
    #[serde(default)]
    pub three_d_tool: ThreeDTool,
    #[serde(default)]
    pub preview_tool: PreviewTool,
    #[serde(default)]
    pub diff_tool: DiffTool,
    pub expanded_sections: Vec<ContextSection>,
    /// Scroll offset per panel, retained across layout changes.
    pub scroll: BTreeMap<PanelId, f32>,
    pub floating_tool_open: bool,
}

impl ViewState {
    pub fn personal_default() -> Self {
        Self {
            tabs: TabStrip::with_reference_surfaces(),
            nav_selection: NavItem::Home,
            ide_tool: IdeTool::default(),
            three_d_tool: ThreeDTool::default(),
            preview_tool: PreviewTool::default(),
            diff_tool: DiffTool::default(),
            expanded_sections: vec![ContextSection::Project, ContextSection::Branch],
            scroll: BTreeMap::new(),
            floating_tool_open: false,
        }
    }

    pub fn is_expanded(&self, section: ContextSection) -> bool {
        self.expanded_sections.contains(&section)
    }

    pub fn toggle_section(&mut self, section: ContextSection) {
        if let Some(index) = self.expanded_sections.iter().position(|s| *s == section) {
            self.expanded_sections.remove(index);
        } else {
            self.expanded_sections.push(section);
        }
    }
}

impl Default for ViewState {
    fn default() -> Self {
        Self::personal_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reference_workspace_opens_with_chat_active() {
        let view = ViewState::personal_default();
        let active = view.tabs.active_tab().unwrap();
        assert!(matches!(active.kind, SurfaceKind::Chat { .. }));
        assert_eq!(active.kind.label(), "Chat · Auth Flow");
    }

    #[test]
    fn the_reference_workspace_shows_all_five_surfaces() {
        let view = ViewState::personal_default();
        let labels: Vec<String> = view.tabs.tabs.iter().map(|t| t.kind.label()).collect();
        assert_eq!(labels, ["Chat · Auth Flow", "IDE", "Live 3D", "Preview", "Diff"]);
    }

    #[test]
    fn closing_the_active_tab_moves_focus_to_a_neighbour() {
        let mut view = ViewState::personal_default();
        let ide = view.tabs.tabs[1].id;
        view.tabs.activate(ide);

        assert!(view.tabs.close(ide));

        assert_ne!(view.tabs.active, ide);
        assert!(view.tabs.active_tab().is_some(), "focus must land somewhere real");
    }

    #[test]
    fn the_last_tab_cannot_be_closed() {
        let mut view = ViewState::personal_default();
        let ids: Vec<u64> = view.tabs.tabs.iter().map(|t| t.id).collect();
        for id in ids {
            view.tabs.close(id);
        }
        assert_eq!(view.tabs.tabs.len(), 1, "a workspace always keeps one surface");
    }

    #[test]
    fn a_reopened_tab_gets_a_fresh_id() {
        let mut view = ViewState::personal_default();
        let first = view.tabs.open(SurfaceKind::Preview);
        view.tabs.close(first);
        let second = view.tabs.open(SurfaceKind::Preview);
        assert_ne!(first, second, "recycled ids would resurrect stale view state");
    }

    #[test]
    fn scroll_offsets_survive_being_read_back() {
        let mut view = ViewState::personal_default();
        view.scroll.insert(PanelId::Chat, 1240.0);
        let json = serde_json::to_string(&view).unwrap();
        let restored: ViewState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.scroll[&PanelId::Chat], 1240.0);
    }

    #[test]
    fn toggling_a_section_is_reversible() {
        let mut view = ViewState::personal_default();
        assert!(!view.is_expanded(ContextSection::Plan));
        view.toggle_section(ContextSection::Plan);
        assert!(view.is_expanded(ContextSection::Plan));
        view.toggle_section(ContextSection::Plan);
        assert!(!view.is_expanded(ContextSection::Plan));
    }

    #[test]
    fn the_context_panel_offers_every_group_from_the_spec() {
        let labels: Vec<&str> = ContextSection::ALL.iter().map(|s| s.label()).collect();
        assert_eq!(
            labels,
            [
                "Project",
                "Branch",
                "Context Usage",
                "Plan",
                "Agents",
                "Changes",
                "Threads",
                "Resources",
                "Notes"
            ]
        );
    }

    #[test]
    fn the_sidebar_offers_every_entry_point_from_the_spec() {
        let mut labels: Vec<&str> = NavItem::PRIMARY.iter().map(|n| n.label()).collect();
        labels.push(NavItem::Settings.label());
        assert_eq!(
            labels,
            [
                "Home",
                "Projects",
                "Chat",
                "Agents",
                "Threads",
                "Files",
                "Search",
                "Skills",
                "Extensions",
                "Settings"
            ]
        );
    }

    #[test]
    fn surface_tool_choices_have_stable_safe_defaults() {
        let view = ViewState::personal_default();
        assert_eq!(view.ide_tool, IdeTool::Explorer);
        assert_eq!(view.three_d_tool, ThreeDTool::Select);
        assert_eq!(view.preview_tool, PreviewTool::Fit);
        assert_eq!(view.diff_tool, DiffTool::Files);

        let restored: ViewState =
            serde_json::from_str(&serde_json::to_string(&view).unwrap()).unwrap();
        assert_eq!(restored, view, "tool choices must persist with the rest of View State");
    }
}
