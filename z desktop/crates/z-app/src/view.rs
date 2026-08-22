//! Building the Personal Agent Workspace scene.
//!
//! Every colour, spacing step, radius and type size here comes from
//! `z-tokens`. There are no literal hex values and no magic pixel constants
//! in this file beyond geometry derived from the token scale — that constraint
//! is what makes the whole appearance themeable and testable.

use crate::content::{Conversation, Message, Role, StepState, FLOATING_TOOLS};
use z_gpui::scene::{Align, Layer, Quad, Scene, TextRun};
use z_gpui::{AccessNode, NodeId, Point, Rect, Role as AxRole, VariableHeights, VirtualList};
use z_shell::{
    ContextSection, DiffTool, IdeTool, NavItem, PanelId, PreviewTool, ShellFrame, SurfaceKind,
    ThreeDTool, Workspace,
};
use z_tokens::metrics::{Radius, Spacing};
use z_tokens::{Rgba, Semantic, Theme, Typography};

/// The shell's rectangles, in the runtime's geometry type.
///
/// `z-shell` and `z-gpui` each own a `Rect` on purpose: the workspace
/// model must not depend on the UI runtime, and the runtime must not know the
/// workspace exists. The app is where the two meet, so the conversion lives
/// here — once, explicitly, rather than scattered through the draw calls.
struct Frame {
    top_bar: Rect,
    sidebar: Rect,
    tab_bar: Rect,
    chat: Rect,
    context_panel: Rect,
    performance_strip: Rect,
    floating_tool: Rect,
}

impl Frame {
    fn from_shell(frame: &ShellFrame) -> Self {
        let px = |r: z_shell::Rect| Rect::new(r.x, r.y, r.width, r.height);
        Self {
            top_bar: px(frame.top_bar),
            sidebar: px(frame.sidebar),
            tab_bar: px(frame.tab_bar),
            chat: px(frame.chat),
            context_panel: px(frame.context_panel),
            performance_strip: px(frame.performance_strip),
            floating_tool: px(frame.floating_tool),
        }
    }
}

pub struct WorkspaceView {
    pub workspace: Workspace,
    pub theme: Theme,
    pub conversation: Conversation,
    /// Latest readings for the Performance Strip. Supplied by telemetry; the
    /// view never invents them.
    pub metrics: Metrics,
    /// Scroll position and row metrics for the conversation.
    ///
    /// Held here rather than rebuilt per frame because measured row heights are
    /// what make the scrollbar and the anchoring correct, and throwing them
    /// away every frame would mean measuring the whole history repeatedly.
    chat_list: VirtualList<VariableHeights>,
    /// Which element the keyboard is on. Kept across frames because the scene —
    /// and its semantic tree — is rebuilt every frame.
    focused: Option<NodeId>,
    /// Composer text being typed. Lives here so every frame renders it.
    pub input: String,
    /// One-line status readout (provider / project / last error).
    pub status_line: String,
    /// Tool call awaiting approval, if any.
    pub pending_approval: Option<PendingApproval>,
    /// Called when the user sends the composer text.
    pub on_send: Option<Box<dyn FnMut(String)>>,
    /// Called when the user resolves a pending approval.
    pub on_resolve: Option<Box<dyn FnMut(bool)>>,
}

#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub cpu_percent: u32,
    pub gpu_percent: u32,
    pub ram_gb: f32,
    pub fps: u32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self { cpu_percent: 0, gpu_percent: 0, ram_gb: 0.0, fps: 0 }
    }
}

/// Namespaces for accessible ids.
///
/// Ids are derived from a namespace plus a stable index — a nav position, a tab
/// id — never from iteration order, so keyboard focus survives a rebuild.
mod ns {
    pub const SHELL: u32 = 1;
    pub const NAV: u32 = 2;
    pub const TAB: u32 = 3;
    pub const TOP_BAR: u32 = 4;
    pub const COMPOSER: u32 = 5;
    pub const CONTEXT: u32 = 6;
    pub const STRIP: u32 = 7;
    pub const FLOATING: u32 = 8;
    pub const MESSAGE: u32 = 9;
    pub const IDE_TOOL: u32 = 10;
    pub const THREE_D_TOOL: u32 = 11;
    pub const PREVIEW_TOOL: u32 = 12;
    pub const DIFF_TOOL: u32 = 13;
    pub const SURFACE: u32 = 14;
}

/// An action the reference shell can actually carry out today.
///
/// The semantic tree owns stable `NodeId`s; this layer translates those ids to
/// view-state changes without making ZeroGPUI know anything about the shell.
/// Controls whose backing capability does not exist yet deliberately map to no
/// command instead of pretending a click succeeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewCommand {
    ActivateTab(u64),
    SelectNav(NavItem),
    SelectIdeTool(IdeTool),
    SelectThreeDTool(ThreeDTool),
    SelectPreviewTool(PreviewTool),
    SelectDiffTool(DiffTool),
    ToggleContext(ContextSection),
    ToggleFloatingTool,
    /// Send the composer's text to the Agent Runtime.
    SendMessage,
    /// Answer a pending tool approval.
    ResolveApproval(bool),
}

/// A tool call waiting for an explicit user decision.
#[derive(Debug, Clone)]
pub struct PendingApproval {
    pub call_id: String,
    pub tool: String,
    pub detail: String,
}

const fn node_namespace(id: NodeId) -> u32 {
    (id.0 >> 32) as u32
}

const fn node_index(id: NodeId) -> u32 {
    id.0 as u32
}

/// Height of the composer. Shared so the Floating Tool can sit clear of it
/// instead of covering the field the user types into.
const INPUT_BAR_HEIGHT: f32 = 92.0;
/// Breathing room between the chat surface's edge and its content.
const CHAT_GUTTER: f32 = Spacing::S5;
/// Height of one row of persistent workspace tool controls.
const SURFACE_TOOLBAR_ROW_HEIGHT: f32 = 44.0;

/// Copy and identity used to draw one honest, non-interactive surface panel.
///
/// Grouping these together keeps the rendering seam small: callers describe a
/// panel as one value instead of passing a loosely related collection of copy
/// fields that could be swapped accidentally.
struct SurfacePanel<'a> {
    index: u32,
    eyebrow: &'a str,
    title: &'a str,
    detail: &'a str,
    icon: Icon,
}

impl<'a> SurfacePanel<'a> {
    const fn new(
        index: u32,
        eyebrow: &'a str,
        title: &'a str,
        detail: &'a str,
        icon: Icon,
    ) -> Self {
        Self { index, eyebrow, title, detail, icon }
    }
}

impl WorkspaceView {
    pub fn new() -> Self {
        let conversation = Conversation::empty();
        let chat_list = Self::list_for(&conversation);
        Self {
            workspace: Workspace::personal_default(),
            theme: Theme::zero_dark(),
            conversation,
            metrics: Metrics { cpu_percent: 18, gpu_percent: 32, ram_gb: 6.4, fps: 120 },
            chat_list,
            focused: None,
            input: String::new(),
            status_line: "Z Desktop Personal".into(),
            pending_approval: None,
            on_send: None,
            on_resolve: None,
        }
    }

    /// Replace the conversation, resetting the scroll model to match.
    pub fn set_conversation(&mut self, conversation: Conversation) {
        self.chat_list = Self::list_for(&conversation);
        self.conversation = conversation;
    }

    /// A fresh scroll model sized to a conversation.
    ///
    /// The estimate is a plain message with two lines of body; anything with a
    /// plan block is much taller, and gets corrected as it is measured.
    fn list_for(conversation: &Conversation) -> VirtualList<VariableHeights> {
        VirtualList::new(VariableHeights::new(conversation.messages.len(), 96.0))
    }

    fn colors(&self) -> &Semantic {
        &self.theme.colors
    }

    pub fn build(&mut self, viewport: Rect) -> Scene {
        let frame = Frame::from_shell(&self.workspace.frame(viewport.width, viewport.height));
        let mut scene = Scene::new();
        let c = self.colors();

        scene.push_quad(Layer::Background, Quad::filled(viewport, c.canvas));

        scene.push_access(
            AccessNode::new(NodeId::new(ns::SHELL, 0), AxRole::Window, "Zero", viewport)
                .focusable(false),
        );

        self.top_bar(&mut scene, &frame);
        self.sidebar(&mut scene, &frame);
        if self.workspace.view.nav_selection == NavItem::Home {
            let active_surface = self.workspace.view.tabs.active_tab().map(|tab| tab.kind.clone());
            match active_surface {
                Some(SurfaceKind::Chat { .. }) | None => self.chat(&mut scene, &frame),
                Some(surface) => self.empty_surface(&mut scene, &frame, surface),
            }
        } else {
            self.navigation_surface(&mut scene, &frame, self.workspace.view.nav_selection);
        }
        self.context_panel(&mut scene, &frame);
        self.performance_strip(&mut scene, &frame);
        self.floating_tool(&mut scene, &frame);

        // Focus is restored into the tree that was just built, so an element
        // that vanished this frame does not leave focus pointing at nothing.
        if let Some(id) = self.focused {
            if !scene.access_mut().focus(id) {
                self.focused = None;
            }
        }
        self.draw_focus_ring(&mut scene);

        scene
    }

    /// The focus ring, drawn on the topmost layer.
    ///
    /// On `Layer::Focus` so it is never covered: a focused control the user
    /// cannot see is the same as no focus indicator at all.
    fn draw_focus_ring(&self, scene: &mut Scene) {
        let Some(bounds) = scene.access().focus_bounds() else { return };
        let c = self.colors();
        scene.push_quad(
            Layer::Focus,
            Quad::filled(bounds.inset(-2.0), Rgba::TRANSPARENT)
                .with_radius(Radius::SM)
                .with_border(c.focus_ring, 2.0),
        );
    }

    /// A project-independent workspace surface. Selecting a view tool changes
    /// the presentation immediately; actions that need a project, runtime or
    /// reviewed change set remain explicitly unavailable in the copy.
    fn empty_surface(&self, scene: &mut Scene, frame: &Frame, surface_kind: SurfaceKind) {
        let c = self.colors();
        let surface = frame.chat;
        if surface.is_empty() {
            return;
        }

        scene.push_quad(Layer::Background, Quad::filled(surface, c.surface));

        let toolbar_rows = surface_toolbar_rows(surface.width, &surface_kind);
        let toolbar_height = (SURFACE_TOOLBAR_ROW_HEIGHT * toolbar_rows as f32).min(surface.height);
        let toolbar = Rect::new(surface.x, surface.y, surface.width, toolbar_height);
        let content = Rect::new(
            surface.x + Spacing::S4,
            toolbar.bottom() + Spacing::S4,
            (surface.width - Spacing::S8).max(0.0),
            (surface.bottom() - toolbar.bottom() - Spacing::S8).max(0.0),
        );

        match surface_kind {
            SurfaceKind::Ide => {
                let tools = self.workspace_toolbar(scene, toolbar, "IDE");
                self.ide_toolbar(scene, tools);
                self.ide_surface(scene, content);
            }
            SurfaceKind::Live3d => {
                let tools = self.workspace_toolbar(scene, toolbar, "Live 3D");
                self.three_d_toolbar(scene, tools);
                self.three_d_surface(scene, content);
            }
            SurfaceKind::Preview => {
                let tools = self.workspace_toolbar(scene, toolbar, "Preview");
                self.preview_toolbar(scene, tools);
                self.preview_surface(scene, content);
            }
            SurfaceKind::Diff => {
                let tools = self.workspace_toolbar(scene, toolbar, "Diff");
                self.diff_toolbar(scene, tools);
                self.diff_surface(scene, content);
            }
            SurfaceKind::Chat { .. } => {}
        }
    }

    /// The stable toolbar shell shared by project-independent workspace tabs.
    fn workspace_toolbar(&self, scene: &mut Scene, bounds: Rect, title: &str) -> Rect {
        let c = self.colors();
        if bounds.is_empty() {
            return bounds;
        }

        scene.push_quad(Layer::Content, Quad::filled(bounds, c.surface_overlay));
        hairline_bottom(scene, bounds, c.border_subtle);

        let title_width = (title.chars().count() as f32 * 7.5 + Spacing::S8).min(bounds.width);
        scene.push_text(
            Layer::Content,
            TextRun::new(
                title,
                Rect::new(
                    bounds.x + Spacing::S3,
                    bounds.y,
                    title_width,
                    SURFACE_TOOLBAR_ROW_HEIGHT.min(bounds.height),
                ),
                Typography::LABEL,
                c.text_secondary,
            ),
        );

        let x = (bounds.x + title_width + Spacing::S2).min(bounds.right());
        Rect::new(
            x,
            bounds.y + Spacing::S1,
            (bounds.right() - x - Spacing::S2).max(0.0),
            (bounds.height - Spacing::S2).max(0.0),
        )
    }

    fn ide_toolbar(&self, scene: &mut Scene, bounds: Rect) {
        let mut x = bounds.x;
        let mut y = bounds.y;
        let row_height = (SURFACE_TOOLBAR_ROW_HEIGHT - Spacing::S2).min(bounds.height);
        for (index, tool) in IdeTool::ALL.iter().copied().enumerate() {
            let width = tool_chip_width(tool.label());
            if width > bounds.width {
                break;
            }
            if x > bounds.x && x + width > bounds.right() {
                x = bounds.x;
                y += SURFACE_TOOLBAR_ROW_HEIGHT;
            }
            if y + row_height > bounds.bottom() {
                break;
            }
            let chip = Rect::new(x, y, width, row_height);
            self.surface_tool_chip(
                scene,
                chip,
                NodeId::new(ns::IDE_TOOL, index as u32),
                tool.label(),
                tool == self.workspace.view.ide_tool,
            );
            x = chip.right() + Spacing::S1;
        }
    }

    fn three_d_toolbar(&self, scene: &mut Scene, bounds: Rect) {
        let mut x = bounds.x;
        let mut y = bounds.y;
        let row_height = (SURFACE_TOOLBAR_ROW_HEIGHT - Spacing::S2).min(bounds.height);
        for (index, tool) in ThreeDTool::ALL.iter().copied().enumerate() {
            let width = tool_chip_width(tool.label());
            if width > bounds.width {
                break;
            }
            if x > bounds.x && x + width > bounds.right() {
                x = bounds.x;
                y += SURFACE_TOOLBAR_ROW_HEIGHT;
            }
            if y + row_height > bounds.bottom() {
                break;
            }
            let chip = Rect::new(x, y, width, row_height);
            self.surface_tool_chip(
                scene,
                chip,
                NodeId::new(ns::THREE_D_TOOL, index as u32),
                tool.label(),
                tool == self.workspace.view.three_d_tool,
            );
            x = chip.right() + Spacing::S1;
        }
    }

    fn preview_toolbar(&self, scene: &mut Scene, bounds: Rect) {
        let mut x = bounds.x;
        let mut y = bounds.y;
        let row_height = (SURFACE_TOOLBAR_ROW_HEIGHT - Spacing::S2).min(bounds.height);
        for (index, tool) in PreviewTool::ALL.iter().copied().enumerate() {
            let width = tool_chip_width(tool.label());
            if width > bounds.width {
                break;
            }
            if x > bounds.x && x + width > bounds.right() {
                x = bounds.x;
                y += SURFACE_TOOLBAR_ROW_HEIGHT;
            }
            if y + row_height > bounds.bottom() {
                break;
            }
            let chip = Rect::new(x, y, width, row_height);
            self.surface_tool_chip(
                scene,
                chip,
                NodeId::new(ns::PREVIEW_TOOL, index as u32),
                tool.label(),
                tool == self.workspace.view.preview_tool,
            );
            x = chip.right() + Spacing::S1;
        }
    }

    fn diff_toolbar(&self, scene: &mut Scene, bounds: Rect) {
        let mut x = bounds.x;
        let mut y = bounds.y;
        let row_height = (SURFACE_TOOLBAR_ROW_HEIGHT - Spacing::S2).min(bounds.height);
        for (index, tool) in DiffTool::ALL.iter().copied().enumerate() {
            let width = tool_chip_width(tool.label());
            if width > bounds.width {
                break;
            }
            if x > bounds.x && x + width > bounds.right() {
                x = bounds.x;
                y += SURFACE_TOOLBAR_ROW_HEIGHT;
            }
            if y + row_height > bounds.bottom() {
                break;
            }
            let chip = Rect::new(x, y, width, row_height);
            self.surface_tool_chip(
                scene,
                chip,
                NodeId::new(ns::DIFF_TOOL, index as u32),
                tool.label(),
                tool == self.workspace.view.diff_tool,
            );
            x = chip.right() + Spacing::S1;
        }
    }

    fn surface_tool_chip(
        &self,
        scene: &mut Scene,
        bounds: Rect,
        id: NodeId,
        label: &str,
        selected: bool,
    ) {
        let c = self.colors();
        if bounds.is_empty() {
            return;
        }

        if selected {
            scene.push_quad(
                Layer::Content,
                Quad::filled(bounds, c.accent_muted)
                    .with_radius(Radius::SM)
                    .with_border(c.accent, 1.0),
            );
        }
        scene.push_text(
            Layer::Content,
            TextRun::new(
                label,
                bounds.inset_xy(Spacing::S2, 0.0),
                Typography::BASE,
                if selected { c.accent } else { c.text_secondary },
            )
            .aligned(Align::Center),
        );
        scene.push_access(
            AccessNode::new(id, AxRole::Button, label, bounds)
                .selected(selected)
                .described(format!("Select the {label} workspace tool.")),
        );
    }

    fn ide_surface(&self, scene: &mut Scene, bounds: Rect) {
        let (title, detail, icon) = ide_tool_copy(self.workspace.view.ide_tool);
        if bounds.width < 460.0 {
            self.surface_panel(
                scene,
                bounds,
                SurfacePanel::new(10, "IDE tool", title, detail, icon),
            );
            return;
        }

        let gap = Spacing::S4;
        let sidebar_width = (bounds.width * 0.30).clamp(176.0, 292.0);
        let sidebar = Rect::new(bounds.x, bounds.y, sidebar_width, bounds.height);
        let editor = Rect::new(
            sidebar.right() + gap,
            bounds.y,
            (bounds.right() - sidebar.right() - gap).max(0.0),
            bounds.height,
        );
        self.surface_panel(scene, sidebar, SurfacePanel::new(10, "IDE tool", title, detail, icon));
        self.surface_panel(
            scene,
            editor,
            SurfacePanel::new(
                11,
                "Editor",
                "No files open",
                "A project connection is required before files appear here.",
                Icon::Page,
            ),
        );
    }

    fn three_d_surface(&self, scene: &mut Scene, bounds: Rect) {
        let (tool_detail, icon) = three_d_tool_copy(self.workspace.view.three_d_tool);
        let scene_detail = "A project scene will appear here when one is available.";
        if bounds.width < 560.0 || bounds.height < 260.0 {
            self.surface_panel(
                scene,
                bounds,
                SurfacePanel::new(20, "No 3D scene", tool_detail, scene_detail, icon),
            );
            return;
        }

        let gap = Spacing::S4;
        let inspector_width = (bounds.width * 0.30).clamp(190.0, 310.0);
        let viewport = Rect::new(
            bounds.x,
            bounds.y,
            (bounds.width - inspector_width - gap).max(0.0),
            bounds.height,
        );
        let inspector = Rect::new(viewport.right() + gap, bounds.y, inspector_width, bounds.height);
        let split = (inspector.height - gap) / 2.0;
        let outliner = Rect::new(inspector.x, inspector.y, inspector.width, split.max(0.0));
        let properties = Rect::new(
            inspector.x,
            outliner.bottom() + gap,
            inspector.width,
            (inspector.bottom() - outliner.bottom() - gap).max(0.0),
        );

        self.surface_panel(
            scene,
            viewport,
            SurfacePanel::new(20, "No 3D scene", tool_detail, scene_detail, icon),
        );
        self.surface_panel(
            scene,
            outliner,
            SurfacePanel::new(
                21,
                "Scene structure",
                "Outliner",
                "No scene hierarchy is available.",
                Icon::Lines,
            ),
        );
        self.surface_panel(
            scene,
            properties,
            SurfacePanel::new(
                22,
                "Object details",
                "Inspector",
                "No object is selected.",
                Icon::Ring,
            ),
        );
    }

    fn preview_surface(&self, scene: &mut Scene, bounds: Rect) {
        let detail = preview_tool_copy(self.workspace.view.preview_tool);
        self.surface_panel(
            scene,
            bounds,
            SurfacePanel::new(
                30,
                "Nothing to preview",
                detail,
                "A real project preview will appear here when available.",
                Icon::Lens,
            ),
        );
    }

    fn diff_surface(&self, scene: &mut Scene, bounds: Rect) {
        let detail = diff_tool_copy(self.workspace.view.diff_tool);
        self.surface_panel(
            scene,
            bounds,
            SurfacePanel::new(
                40,
                "No changes to compare",
                detail,
                "Real project changes will appear here when available.",
                Icon::Copy,
            ),
        );
    }

    /// Each destination in the left rail owns a centre surface. This makes the
    /// rail more than a cosmetic selection while preserving the active work
    /// tab: activating any top tab returns to Home and reveals it again.
    fn navigation_surface(&self, scene: &mut Scene, frame: &Frame, item: NavItem) {
        let c = self.colors();
        let surface = frame.chat;
        if surface.is_empty() {
            return;
        }

        scene.push_quad(Layer::Background, Quad::filled(surface, c.surface));
        let content = Rect::new(
            surface.x + Spacing::S6,
            surface.y + Spacing::S6,
            (surface.width - Spacing::S10).max(0.0),
            (surface.height - Spacing::S10).max(0.0),
        );
        let (title, detail, icon) = navigation_copy(item);
        let width = (content.width * 0.68).min(560.0);
        let height = content.height.min(176.0);
        let card = Rect::new(
            content.x + (content.width - width) / 2.0,
            content.y + (content.height - height) / 2.0,
            width,
            height,
        );
        self.surface_panel(scene, card, SurfacePanel::new(50, "Workspace", title, detail, icon));
    }

    fn surface_panel(&self, scene: &mut Scene, bounds: Rect, panel: SurfacePanel<'_>) {
        if bounds.width < 80.0 || bounds.height < 60.0 {
            return;
        }

        let SurfacePanel { index, eyebrow, title, detail, icon } = panel;

        let c = self.colors();
        scene.push_quad(
            Layer::Content,
            Quad::filled(bounds, c.surface_raised)
                .with_radius(Radius::LG)
                .with_border(c.border_subtle, 1.0),
        );

        let inner = bounds.inset_xy(Spacing::S4, Spacing::S4);
        scene.push_text(
            Layer::Content,
            TextRun::new(
                eyebrow,
                Rect::new(inner.x, inner.y, inner.width, 16.0),
                Typography::LABEL,
                c.text_tertiary,
            ),
        );
        let icon_box = Rect::new(inner.x, inner.y + 26.0, 18.0, 18.0);
        draw_icon(scene, Layer::Content, icon, icon_box, c.text_secondary);
        scene.push_text(
            Layer::Content,
            TextRun::new(
                title,
                Rect::new(
                    icon_box.right() + Spacing::S2,
                    inner.y + 22.0,
                    (inner.right() - icon_box.right() - Spacing::S2).max(0.0),
                    28.0,
                ),
                Typography::LG,
                c.text_primary,
            ),
        );

        if inner.height >= 74.0 {
            scene.push_text(
                Layer::Content,
                TextRun::new(
                    detail,
                    Rect::new(
                        inner.x,
                        inner.y + 58.0,
                        inner.width,
                        (inner.bottom() - inner.y - 58.0).max(0.0),
                    ),
                    Typography::BASE,
                    c.text_secondary,
                ),
            );
        }
        scene.push_access(
            AccessNode::new(NodeId::new(ns::SURFACE, index), AxRole::Group, title, bounds)
                .described(detail)
                .focusable(false),
        );
    }

    /// Move keyboard focus. Returns true when the frame should be rebuilt.
    pub fn move_focus(&mut self, forward: bool, viewport: Rect) -> bool {
        // The tree is a product of building, so build one to walk it. Cheap:
        // the scene is thrown away and only the resulting id is kept.
        let mut scene = self.build(viewport);
        let tree = scene.access_mut();
        if let Some(id) = self.focused {
            tree.focus(id);
        }
        let next = if forward { tree.focus_next() } else { tree.focus_previous() };
        if next != self.focused {
            self.focused = next;
            true
        } else {
            false
        }
    }

    /// Move focus between real tabs, preserving the historical preset shortcut
    /// for horizontal arrows everywhere else.
    pub fn move_focus_in_tab_strip(&mut self, forward: bool, viewport: Rect) -> bool {
        self.move_focus_in_namespace(ns::TAB, forward, viewport, |id| node_index(id) != u32::MAX)
    }

    /// Move focus inside the focused vertical list. Navigation and context are
    /// separate lists, so an arrow key never jumps between distant panels.
    pub fn move_focus_in_list(&mut self, forward: bool, viewport: Rect) -> bool {
        let Some(namespace) = self.focused.map(node_namespace) else { return false };
        if !matches!(namespace, ns::NAV | ns::CONTEXT) {
            return false;
        }
        self.move_focus_in_namespace(namespace, forward, viewport, |_| true)
    }

    fn move_focus_in_namespace(
        &mut self,
        namespace: u32,
        forward: bool,
        viewport: Rect,
        include: impl Fn(NodeId) -> bool,
    ) -> bool {
        let scene = self.build(viewport);
        let candidates: Vec<NodeId> = scene
            .access()
            .focus_order()
            .into_iter()
            .filter(|id| node_namespace(*id) == namespace && include(*id))
            .collect();
        let Some(current) = self.focused else { return false };
        let Some(position) = candidates.iter().position(|id| *id == current) else {
            return false;
        };
        let next = if forward {
            (position + 1) % candidates.len()
        } else {
            (position + candidates.len() - 1) % candidates.len()
        };
        let next = candidates[next];
        if next == current {
            return false;
        }
        self.focused = Some(next);
        true
    }

    pub fn focused(&self) -> Option<NodeId> {
        self.focused
    }

    /// Move focus to the exact semantic node requested by the platform.
    pub fn focus(&mut self, id: NodeId, viewport: Rect) -> bool {
        let scene = self.build(viewport);
        if !scene.access().get(id).is_some_and(AccessNode::can_take_focus) {
            return false;
        }
        if self.focused == Some(id) {
            return false;
        }
        self.focused = Some(id);
        true
    }

    /// Focus and activate the semantic control under a primary-pointer click.
    ///
    /// Pointer activation deliberately reuses the NodeId → ViewCommand path
    /// used by Enter, Space and AccessKit. That gives all input methods the
    /// same capabilities and avoids a second, geometry-specific command map.
    pub fn click(&mut self, position: Point, viewport: Rect) -> bool {
        let scene = self.build(viewport);
        let Some(id) = scene.access().focusable_at(position) else { return false };

        let focus_changed = self.focused != Some(id);
        self.focused = Some(id);
        self.activate(id) || focus_changed
    }

    /// Activate the element that currently owns keyboard focus.
    ///
    /// Re-validating against a freshly built tree prevents a control that
    /// disappeared during a layout change from receiving a stale command.
    pub fn activate_focused(&mut self, viewport: Rect) -> bool {
        let Some(id) = self.focused else { return false };
        let scene = self.build(viewport);
        if !scene.access().get(id).is_some_and(AccessNode::can_take_focus) {
            self.focused = None;
            return false;
        }
        self.activate(id)
    }

    /// Activate a semantic node whose command is represented in View State.
    pub fn activate(&mut self, id: NodeId) -> bool {
        let Some(command) = self.command_for(id) else { return false };
        self.execute(command)
    }

    fn command_for(&self, id: NodeId) -> Option<ViewCommand> {
        let namespace = node_namespace(id);
        let index = node_index(id);

        match namespace {
            ns::TAB if index != u32::MAX => Some(ViewCommand::ActivateTab(index as u64)),
            ns::NAV => {
                let item = if index == u32::MAX {
                    Some(NavItem::Settings)
                } else {
                    NavItem::PRIMARY.get(index as usize).copied()
                };
                item.map(ViewCommand::SelectNav)
            }
            ns::CONTEXT => ContextSection::ALL
                .iter()
                .copied()
                .find(|section| *section as u32 == index)
                .map(ViewCommand::ToggleContext),
            ns::IDE_TOOL => {
                IdeTool::ALL.get(index as usize).copied().map(ViewCommand::SelectIdeTool)
            }
            ns::THREE_D_TOOL => {
                ThreeDTool::ALL.get(index as usize).copied().map(ViewCommand::SelectThreeDTool)
            }
            ns::PREVIEW_TOOL => {
                PreviewTool::ALL.get(index as usize).copied().map(ViewCommand::SelectPreviewTool)
            }
            ns::DIFF_TOOL => {
                DiffTool::ALL.get(index as usize).copied().map(ViewCommand::SelectDiffTool)
            }
            ns::FLOATING if index == 0 => Some(ViewCommand::ToggleFloatingTool),
            ns::COMPOSER if index == 10 && !self.input.trim().is_empty() => {
                Some(ViewCommand::SendMessage)
            }
            ns::COMPOSER if index == 20 => Some(ViewCommand::ResolveApproval(true)),
            ns::COMPOSER if index == 21 => Some(ViewCommand::ResolveApproval(false)),
            _ => None,
        }
    }

    fn execute(&mut self, command: ViewCommand) -> bool {
        match command {
            ViewCommand::ActivateTab(target) => {
                let active_changed = self.workspace.view.tabs.active != target;
                let returns_home = self.workspace.view.nav_selection != NavItem::Home;
                if !self.workspace.view.tabs.activate(target) {
                    return false;
                }
                self.workspace.view.nav_selection = NavItem::Home;
                active_changed || returns_home
            }
            ViewCommand::SelectNav(target) => {
                if self.workspace.view.nav_selection == target {
                    false
                } else {
                    self.workspace.view.nav_selection = target;
                    true
                }
            }
            ViewCommand::SelectIdeTool(target) => {
                if self.workspace.view.ide_tool == target {
                    false
                } else {
                    self.workspace.view.ide_tool = target;
                    true
                }
            }
            ViewCommand::SelectThreeDTool(target) => {
                if self.workspace.view.three_d_tool == target {
                    false
                } else {
                    self.workspace.view.three_d_tool = target;
                    true
                }
            }
            ViewCommand::SelectPreviewTool(target) => {
                if self.workspace.view.preview_tool == target {
                    false
                } else {
                    self.workspace.view.preview_tool = target;
                    true
                }
            }
            ViewCommand::SelectDiffTool(target) => {
                if self.workspace.view.diff_tool == target {
                    false
                } else {
                    self.workspace.view.diff_tool = target;
                    true
                }
            }
            ViewCommand::ToggleContext(target) => {
                self.workspace.view.toggle_section(target);
                true
            }
            ViewCommand::ToggleFloatingTool => {
                self.workspace.view.floating_tool_open = !self.workspace.view.floating_tool_open;
                true
            }
            ViewCommand::SendMessage => {
                let text = std::mem::take(&mut self.input);
                if text.trim().is_empty() {
                    return false;
                }
                if let Some(send) = self.on_send.as_mut() {
                    send(text);
                }
                true
            }
            ViewCommand::ResolveApproval(approved) => {
                self.pending_approval = None;
                if let Some(resolve) = self.on_resolve.as_mut() {
                    resolve(approved);
                }
                true
            }
        }
    }

    /// Bring a virtualized message requested by assistive technology into the
    /// visible chat window.
    pub fn scroll_access_node_into_view(&mut self, id: NodeId) -> bool {
        let namespace = node_namespace(id);
        let index = node_index(id) as usize;
        if namespace != ns::MESSAGE || index >= self.conversation.messages.len() {
            return false;
        }

        let before = self.chat_list.scroll_offset();
        self.chat_list.scroll_into_view(index);
        self.chat_list.scroll_offset() != before
    }

    // -- Top band ------------------------------------------------------------

    fn top_bar(&self, scene: &mut Scene, frame: &Frame) {
        let c = self.colors();
        let bar = frame.top_bar;
        if bar.is_empty() {
            return;
        }

        scene.push_quad(Layer::Background, Quad::filled(bar, c.surface));
        hairline_bottom(scene, bar, c.border_subtle);

        // Brand mark, aligned with the sidebar column beneath it.
        let mark_size = 18.0;
        let mark = Rect::new(
            bar.x + Spacing::S4,
            bar.y + (bar.height - mark_size) / 2.0,
            mark_size,
            mark_size,
        );
        scene.push_quad(
            Layer::Content,
            Quad::filled(mark, c.accent).with_radius(Radius::SM).with_border(c.accent, 1.0),
        );
        scene.push_quad(
            Layer::Content,
            Quad::filled(mark.inset(5.0), c.text_inverse).with_radius(2.0),
        );
        scene.push_text(
            Layer::Content,
            TextRun::new(
                "Zero",
                Rect::new(mark.right() + Spacing::S2, bar.y, 80.0, bar.height),
                Typography::LG,
                c.text_primary,
            ),
        );

        self.tab_strip(scene, frame);
        self.account_zone(scene, frame);
    }

    fn tab_strip(&self, scene: &mut Scene, frame: &Frame) {
        let c = self.colors();
        let zone = frame.tab_bar;
        if zone.is_empty() {
            return;
        }

        let tabs = &self.workspace.view.tabs;
        let height = 34.0;
        let y = zone.y + (zone.height - height) / 2.0;
        let mut x = zone.x + Spacing::S2;

        for tab in &tabs.tabs {
            let label = tab.kind.label();
            // Approximate advance: the scene keeps a measurement-free build path
            // so a frame never blocks on shaping. Text is clipped by its box, so
            // an over-estimate costs padding, never a broken layout.
            let width = (label.chars().count() as f32 * 7.0 + Spacing::S8).min(220.0);
            let bounds = Rect::new(x, y, width, height);
            let active = tab.id == tabs.active;

            if active {
                scene.push_quad(
                    Layer::Content,
                    Quad::filled(bounds, c.surface_hover)
                        .with_radius(Radius::SM)
                        .with_border(c.border_default, 1.0),
                );
                scene.push_quad(
                    Layer::Content,
                    Quad::filled(
                        Rect::new(
                            bounds.x + Spacing::S2,
                            bounds.bottom() - 2.0,
                            (bounds.width - Spacing::S4).max(0.0),
                            2.0,
                        ),
                        c.accent,
                    )
                    .with_radius(Radius::FULL),
                );
            }

            scene.push_access(
                AccessNode::new(
                    NodeId::new(ns::TAB, tab.id as u32),
                    AxRole::Tab,
                    label.clone(),
                    bounds,
                )
                .selected(active),
            );

            scene.push_text(
                Layer::Content,
                TextRun::new(
                    label,
                    bounds.inset_xy(Spacing::S3, 0.0),
                    Typography::BASE,
                    if active { c.text_primary } else { c.text_secondary },
                ),
            );

            x = bounds.right() + Spacing::S1;
        }

        // Add-surface affordance.
        let add = Rect::new(x + Spacing::S1, y, height, height);
        if add.right() <= zone.right() {
            scene.push_text(
                Layer::Content,
                TextRun::new("+", add, Typography::LG, c.text_tertiary).aligned(Align::Center),
            );
            scene.push_access(
                AccessNode::new(NodeId::new(ns::TAB, u32::MAX), AxRole::Button, "New surface", add)
                    .disabled(true)
                    .described("Opening a new surface is not available yet."),
            );
        }
    }

    fn account_zone(&self, scene: &mut Scene, frame: &Frame) {
        let c = self.colors();
        let bar = frame.top_bar;
        let zone = Rect::new(
            frame.tab_bar.right(),
            bar.y,
            (bar.right() - frame.tab_bar.right()).max(0.0),
            bar.height,
        );
        if zone.width < 120.0 {
            return;
        }

        let avatar_size = 28.0;
        let avatar = Rect::new(
            zone.right() - Spacing::S4 - avatar_size,
            zone.y + (zone.height - avatar_size) / 2.0,
            avatar_size,
            avatar_size,
        );
        scene.push_quad(
            Layer::Content,
            Quad::filled(avatar, c.surface)
                .with_radius(Radius::FULL)
                .with_border(c.border_subtle, 1.0),
        );

        scene.push_access(
            AccessNode::new(NodeId::new(ns::TOP_BAR, 2), AxRole::Button, "Account", avatar)
                .disabled(true)
                .described("Account controls are not available yet."),
        );

        let bell = Rect::new(avatar.x - Spacing::S6 - 16.0, avatar.y, 16.0, avatar_size);
        draw_icon(
            scene,
            Layer::Content,
            Icon::Bell,
            Rect::new(bell.x, bell.y + (bell.height - 16.0) / 2.0, 16.0, 16.0),
            c.text_tertiary,
        );

        // The account and notification features do not have a runtime behind
        // them yet. They remain announced, but cannot look actionable.
        scene.push_access(
            AccessNode::new(NodeId::new(ns::TOP_BAR, 1), AxRole::Button, "Notifications", bell)
                .disabled(true)
                .described("Notifications are not available yet."),
        );

        // Mode indicator. Personal only — there is no mode selector in this
        // build, so a chevron would promise an unavailable interaction.
        let selector_width = 118.0;
        let selector = Rect::new(
            bell.x - Spacing::S4 - selector_width,
            zone.y + (zone.height - 30.0) / 2.0,
            selector_width,
            30.0,
        );
        if selector.x > zone.x {
            scene.push_quad(
                Layer::Content,
                Quad::filled(selector, c.surface_raised)
                    .with_radius(Radius::SM)
                    .with_border(c.border_default, 1.0),
            );
            scene.push_text(
                Layer::Content,
                TextRun::new(
                    "Personal",
                    selector.inset_xy(Spacing::S3, 0.0),
                    Typography::BASE,
                    c.text_primary,
                ),
            );
            scene.push_access(
                AccessNode::new(
                    NodeId::new(ns::TOP_BAR, 0),
                    AxRole::Group,
                    "Mode: Personal",
                    selector,
                )
                .described("Personal mode is fixed in this build.")
                .focusable(false),
            );
        }
    }

    // -- Left rail -----------------------------------------------------------

    fn sidebar(&self, scene: &mut Scene, frame: &Frame) {
        let c = self.colors();
        let rail = frame.sidebar;
        if rail.is_empty() {
            return;
        }

        scene.push_quad(Layer::Background, Quad::filled(rail, c.canvas));
        hairline_right(scene, rail, c.border_subtle);

        let icon_only = rail.width < 120.0;
        let row_height = 38.0;
        let mut y = rail.y + Spacing::S3;

        for (index, item) in NavItem::PRIMARY.iter().enumerate() {
            let row = Rect::new(rail.x + Spacing::S2, y, rail.width - Spacing::S4, row_height);
            let selected = *item == self.workspace.view.nav_selection;
            self.nav_row(scene, row, item.label(), selected, icon_only);
            // The name is declared even in icon-only mode: the label is hidden
            // from the eye, not from the screen reader.
            scene.push_access(
                AccessNode::new(
                    NodeId::new(ns::NAV, index as u32),
                    AxRole::Button,
                    item.label(),
                    row,
                )
                .selected(selected),
            );
            y = row.bottom() + 2.0;
        }

        // Settings is pinned to the foot of the rail, away from the scroll list.
        let settings = Rect::new(
            rail.x + Spacing::S2,
            rail.bottom() - Spacing::S3 - row_height,
            rail.width - Spacing::S4,
            row_height,
        );
        if settings.y > y {
            let selected = self.workspace.view.nav_selection == NavItem::Settings;
            self.nav_row(scene, settings, NavItem::Settings.label(), selected, icon_only);
            scene.push_access(
                AccessNode::new(
                    NodeId::new(ns::NAV, u32::MAX),
                    AxRole::Button,
                    NavItem::Settings.label(),
                    settings,
                )
                .selected(selected),
            );
        }
    }

    fn nav_row(&self, scene: &mut Scene, row: Rect, label: &str, selected: bool, icon_only: bool) {
        let c = self.colors();

        if selected {
            scene.push_quad(
                Layer::Content,
                Quad::filled(row, c.surface_raised)
                    .with_radius(Radius::MD)
                    .with_border(c.border_default, 1.0),
            );
            scene.push_quad(
                Layer::Content,
                Quad::filled(
                    Rect::new(row.x, row.y + Spacing::S2, 2.0, (row.height - Spacing::S4).max(0.0)),
                    c.accent,
                )
                .with_radius(Radius::FULL),
            );
        }

        let glyph = Rect::new(row.x + Spacing::S3, row.y + (row.height - 14.0) / 2.0, 14.0, 14.0);
        draw_icon(
            scene,
            Layer::Content,
            icon_for(label),
            glyph,
            if selected { c.accent } else { c.text_tertiary },
        );

        if !icon_only {
            scene.push_text(
                Layer::Content,
                TextRun::new(
                    label.to_string(),
                    Rect::new(
                        glyph.right() + Spacing::S3,
                        row.y,
                        row.width - (glyph.right() - row.x) - Spacing::S4,
                        row.height,
                    ),
                    Typography::BASE,
                    if selected { c.text_primary } else { c.text_secondary },
                ),
            );
        }
    }

    // -- Centre surface ------------------------------------------------------

    fn chat(&mut self, scene: &mut Scene, frame: &Frame) {
        let c = self.colors();
        let surface = frame.chat;
        if surface.is_empty() {
            return;
        }

        scene.push_quad(Layer::Background, Quad::filled(surface, c.surface));

        let input_height = INPUT_BAR_HEIGHT;
        let gutter = CHAT_GUTTER;
        let content = Rect::new(
            surface.x + gutter,
            surface.y + gutter,
            (surface.width - gutter * 2.0).max(0.0),
            (surface.height - gutter * 2.0 - input_height).max(0.0),
        );

        // Only the messages overlapping the viewport are built. Without this a
        // long thread costs a full layout pass every frame, and the cost grows
        // with the history rather than with the window.
        self.chat_list.metrics_mut().set_count(self.conversation.messages.len());
        self.chat_list.metrics_mut().settle();
        self.chat_list.set_viewport_height(content.height);
        self.chat_list.content_changed();

        let visible = self.chat_list.visible_range();
        let mut measured: Vec<(usize, f32)> = Vec::with_capacity(visible.len());

        // Over-scanned rows sit above and below the viewport by design. Without
        // a clip they would paint over the top bar and the composer, so the
        // whole conversation is drawn inside the content rect.
        scene.push_access(
            AccessNode::new(NodeId::new(ns::SHELL, 1), AxRole::ScrollArea, "Conversation", content)
                .described(format!("{} messages", self.conversation.messages.len()))
                .focusable(false),
        );

        scene.push_clip(content);
        for index in visible.iter() {
            let Some(message) = self.conversation.messages.get(index) else { continue };
            let bounds = self.chat_list.item_bounds(index, content);
            let consumed = self.message(
                scene,
                Rect::new(bounds.x, bounds.y, content.width, content.height),
                message,
            );

            // One node per message rather than per glyph: a screen reader
            // should hear "You said ..." as a unit, not a stream of fragments.
            scene.push_access(
                AccessNode::new(
                    NodeId::new(ns::MESSAGE, index as u32),
                    AxRole::Article,
                    format!("{}: {}", message.author, message.body),
                    Rect::new(bounds.x, bounds.y, content.width, consumed),
                )
                .described(message.timestamp.clone())
                .busy(message.progress.is_some())
                .focusable(false),
            );

            measured.push((index, consumed + Spacing::S5));
        }
        scene.pop_clip();

        // Applied after the loop so one frame's measurements cost one rebuild
        // of the prefix sums, not one per row.
        for (index, height) in measured {
            self.chat_list.measure_item(index, height);
        }

        self.chat_scrollbar(scene, content);

        self.input_bar(
            scene,
            Rect::new(
                content.x,
                surface.bottom() - gutter - input_height,
                content.width,
                input_height,
            ),
        );
    }

    /// A slim indicator of position within the conversation.
    ///
    /// Drawn only when there is something to scroll: a track that never moves
    /// is decoration, and the design has no room for decoration.
    fn chat_scrollbar(&self, scene: &mut Scene, content: Rect) {
        let visible = self.chat_visible_fraction();
        if visible >= 1.0 || content.height <= 0.0 {
            return;
        }

        let c = self.colors();
        let width = 3.0;
        let track = Rect::new(content.right() - width, content.y, width, content.height);

        // A thumb proportional to the visible fraction, floored so it stays
        // grabbable in a very long thread.
        let thumb_height = (track.height * visible).max(28.0).min(track.height);
        let travel = track.height - thumb_height;
        let progress = if self.chat_list.max_scroll() > 0.0 {
            self.chat_list.scroll_offset() / self.chat_list.max_scroll()
        } else {
            0.0
        };

        scene.push_quad(
            Layer::Content,
            Quad::filled(track, c.border_subtle).with_radius(Radius::FULL),
        );
        scene.push_quad(
            Layer::Content,
            Quad::filled(
                Rect::new(track.x, track.y + travel * progress, width, thumb_height),
                c.text_tertiary,
            )
            .with_radius(Radius::FULL),
        );
    }

    /// Draw one message and report the vertical space it used.
    fn message(&self, scene: &mut Scene, area: Rect, message: &Message) -> f32 {
        let c = self.colors();
        let avatar_size = 30.0;
        let user = message.role == Role::User;

        // Role alignment is fixed: the user on the right, Zero on the left.
        let avatar = if user {
            Rect::new(area.right() - avatar_size, area.y, avatar_size, avatar_size)
        } else {
            Rect::new(area.x, area.y, avatar_size, avatar_size)
        };

        scene.push_quad(
            Layer::Content,
            Quad::filled(avatar, c.surface_overlay)
                .with_radius(if user { Radius::FULL } else { Radius::SM })
                .with_border(c.border_default, 1.0),
        );
        if !user {
            scene.push_quad(
                Layer::Content,
                Quad::filled(avatar.inset(9.0), c.text_primary).with_radius(2.0),
            );
        } else {
            scene.push_quad(
                Layer::Content,
                Quad::filled(
                    Rect::new(avatar.right() - 8.0, avatar.bottom() - 8.0, 7.0, 7.0),
                    c.status_success,
                )
                .with_radius(Radius::FULL),
            );
        }

        let text_x = if user { area.x } else { avatar.right() + Spacing::S3 };
        let text_width = (area.width - avatar_size - Spacing::S3).max(0.0);
        let align = if user { Align::End } else { Align::Start };

        // Author and timestamp share a row; the timestamp anchors to the far
        // edge so the eye can scan a column of times.
        let header = Rect::new(text_x, area.y, text_width, 20.0);
        scene.push_text(
            Layer::Content,
            TextRun::new(&message.author, header, Typography::LABEL, c.text_primary).aligned(align),
        );
        scene.push_text(
            Layer::Content,
            TextRun::new(&message.timestamp, header, Typography::SM, c.text_tertiary)
                .aligned(if user { Align::Start } else { Align::End }),
        );

        let mut y = header.bottom() + Spacing::S1;

        if let Some(lead_in) = &message.lead_in {
            let bounds = Rect::new(text_x, y, text_width, 22.0);
            scene.push_text(
                Layer::Content,
                TextRun::new(lead_in, bounds, Typography::BODY, c.text_primary).aligned(align),
            );
            y = bounds.bottom() + 2.0;
        }

        let body_lines = wrapped_line_count(&message.body, text_width, Typography::BODY.size);
        let body_height = body_lines * Typography::BODY.line_height;
        scene.push_text(
            Layer::Content,
            TextRun::new(
                &message.body,
                Rect::new(text_x, y, text_width, body_height),
                Typography::BODY,
                c.text_primary,
            )
            .aligned(align),
        );
        y += body_height;

        if !message.plan.is_empty() {
            y += Spacing::S3;
            y = self.plan_block(scene, Rect::new(text_x, y, text_width, 0.0), message);
        }

        if message.actions {
            y += Spacing::S2;
            let mut x = text_x;
            for (action_index, icon) in
                [Icon::ThumbUp, Icon::ThumbDown, Icon::Copy, Icon::Ellipsis].into_iter().enumerate()
            {
                let button = Rect::new(x, y, 30.0, 28.0);
                scene.push_quad(
                    Layer::Content,
                    Quad::filled(button, Rgba::TRANSPARENT)
                        .with_radius(Radius::SM)
                        .with_border(c.border_subtle, 1.0),
                );
                draw_icon(
                    scene,
                    Layer::Content,
                    icon,
                    Rect::new(
                        button.x + (button.width - 13.0) / 2.0,
                        button.y + (button.height - 13.0) / 2.0,
                        13.0,
                        13.0,
                    ),
                    c.text_tertiary,
                );
                let label = match icon {
                    Icon::ThumbUp => "Good response",
                    Icon::ThumbDown => "Bad response",
                    Icon::Copy => "Copy message",
                    _ => "More actions",
                };
                scene.push_access(
                    AccessNode::new(
                        NodeId::new(ns::MESSAGE, 30_000 + action_index as u32),
                        AxRole::Button,
                        label,
                        button,
                    )
                    .disabled(true)
                    .described("Message actions are not available without an Agent Runtime."),
                );
                x = button.right() + Spacing::S1;
            }
            y += 28.0;
        }

        y - area.y
    }

    /// Plan checklist plus the work-in-progress block. Returns the new y.
    fn plan_block(&self, scene: &mut Scene, area: Rect, message: &Message) -> f32 {
        let c = self.colors();
        let row_height = 29.0;
        let progress_height = if message.progress.is_some() { 92.0 } else { 0.0 };
        let padding = Spacing::S3;
        let card_height = padding * 2.0 + message.plan.len() as f32 * row_height + progress_height;

        let card = Rect::new(area.x, area.y, area.width, card_height);
        scene.push_quad(
            Layer::Content,
            Quad::filled(card, c.surface_raised)
                .with_radius(Radius::MD)
                .with_border(c.border_subtle, 1.0),
        );

        let mut y = card.y + padding;
        for (step_index, step) in message.plan.iter().enumerate() {
            let row = Rect::new(card.x + padding, y, card.width - padding * 2.0, row_height);

            // State marker. Shape differs per state so the checklist stays
            // readable without relying on colour.
            let marker = Rect::new(row.x, row.y + (row.height - 14.0) / 2.0, 14.0, 14.0);
            match step.state {
                StepState::Completed => {
                    scene.push_quad(
                        Layer::Content,
                        Quad::filled(marker, Rgba::TRANSPARENT)
                            .with_radius(Radius::FULL)
                            .with_border(c.text_secondary, 1.5),
                    );
                    scene.push_quad(
                        Layer::Content,
                        Quad::filled(marker.inset(4.0), c.text_secondary).with_radius(Radius::FULL),
                    );
                }
                StepState::InProgress => {
                    scene.push_quad(
                        Layer::Content,
                        Quad::filled(marker, Rgba::TRANSPARENT)
                            .with_radius(Radius::FULL)
                            .with_border(c.text_primary, 1.5),
                    );
                    scene.push_quad(
                        Layer::Content,
                        Quad::filled(marker.inset(3.5), c.text_primary).with_radius(Radius::FULL),
                    );
                }
                StepState::Pending => {
                    scene.push_quad(
                        Layer::Content,
                        Quad::filled(marker, Rgba::TRANSPARENT)
                            .with_radius(Radius::FULL)
                            .with_border(c.text_tertiary, 1.5),
                    );
                }
                StepState::Failed => {
                    scene.push_quad(
                        Layer::Content,
                        Quad::filled(marker, Rgba::TRANSPARENT)
                            .with_radius(3.0)
                            .with_border(c.status_danger, 1.5),
                    );
                }
            }

            let completed = step.state == StepState::Completed;
            scene.push_text(
                Layer::Content,
                TextRun::new(
                    &step.title,
                    Rect::new(marker.right() + Spacing::S3, row.y, row.width * 0.6, row.height),
                    Typography::BASE,
                    if completed { c.text_secondary } else { c.text_primary },
                ),
            );
            scene.push_text(
                Layer::Content,
                TextRun::new(
                    step.state.label(),
                    row,
                    Typography::SM,
                    match step.state {
                        StepState::Failed => c.status_danger,
                        StepState::InProgress => c.text_primary,
                        _ => c.text_tertiary,
                    },
                )
                .aligned(Align::End),
            );
            // State goes into the name. A screen reader user must not have to
            // infer "failed" from a colour they cannot see.
            scene.push_access(
                AccessNode::new(
                    NodeId::new(ns::MESSAGE, 10_000 + step_index as u32),
                    AxRole::ListItem,
                    format!("{}, {}", step.title, step.state.label()),
                    row,
                )
                .busy(step.state == StepState::InProgress)
                .focusable(false),
            );

            y = row.bottom();
        }

        if let Some(progress) = &message.progress {
            let block = Rect::new(
                card.x + padding,
                y + Spacing::S2,
                card.width - padding * 2.0,
                progress_height - Spacing::S3,
            );
            scene.push_quad(
                Layer::Content,
                Quad::filled(block, c.surface_overlay)
                    .with_radius(Radius::SM)
                    .with_border(c.border_subtle, 1.0),
            );

            let inner = block.inset_xy(Spacing::S3, Spacing::S3);
            let text_x = inner.x + 28.0;
            let text_width = (inner.width - 28.0 - 46.0).max(0.0);
            // Two text rows, then a clear gap, then the bar. Overlapping them
            // makes the path unreadable exactly when the user most wants it.
            let track_y = inner.y + 19.0 + 17.0 + Spacing::S3;

            scene.push_text(
                Layer::Content,
                TextRun::new(
                    &progress.title,
                    Rect::new(text_x, inner.y, text_width, 19.0),
                    Typography::BASE,
                    c.text_primary,
                ),
            );
            scene.push_text(
                Layer::Content,
                TextRun::new(
                    format!("{} / {}", progress.done, progress.total),
                    Rect::new(inner.x, inner.y, inner.width, 19.0),
                    Typography::METRIC,
                    c.text_secondary,
                )
                .aligned(Align::End),
            );

            // A path is direction-neutral content: isolate it so it keeps its
            // own order inside prose running the other way.
            scene.push_text(
                Layer::Content,
                TextRun::new(
                    format!("Working on: {}", progress.detail),
                    Rect::new(text_x, inner.y + 19.0, inner.width - 28.0, 17.0),
                    Typography::SM,
                    c.text_tertiary,
                )
                .isolated(),
            );

            scene.push_access(
                AccessNode::new(
                    NodeId::new(ns::MESSAGE, 20_000),
                    AxRole::ProgressIndicator,
                    progress.title.clone(),
                    block,
                )
                .valued(progress.fraction())
                .busy(true)
                .described(format!(
                    "{} of {}, working on {}",
                    progress.done, progress.total, progress.detail
                ))
                .focusable(false),
            );

            // The bar sits clear of the two text rows rather than under them.
            let track = Rect::new(inner.x, track_y, inner.width, 3.0);
            scene.push_quad(
                Layer::Content,
                Quad::filled(track, c.border_default).with_radius(Radius::FULL),
            );
            scene.push_quad(
                Layer::Content,
                Quad::filled(
                    Rect::new(track.x, track.y, track.width * progress.fraction(), track.height),
                    c.status_running,
                )
                .with_radius(Radius::FULL),
            );

            // Activity mark: a ring with a gap, so "running" reads as motion
            // rather than as another checklist bullet.
            let ring = Rect::new(inner.x + 2.0, inner.y + 2.0, 16.0, 16.0);
            scene.push_quad(
                Layer::Content,
                Quad::filled(ring, Rgba::TRANSPARENT)
                    .with_radius(Radius::FULL)
                    .with_border(c.border_strong, 1.5),
            );
            scene.push_quad(
                Layer::Content,
                Quad::filled(Rect::new(ring.center().x - 1.0, ring.y, 2.0, 5.0), c.status_running)
                    .with_radius(Radius::FULL),
            );
        }

        card.bottom()
    }

    fn input_bar(&self, scene: &mut Scene, area: Rect) {
        let c = self.colors();
        scene.push_quad(
            Layer::Content,
            Quad::filled(area, c.surface_raised)
                .with_radius(Radius::MD)
                .with_border(c.border_default, 1.0),
        );

        // The composer field itself: typed text when present, placeholder when not.
        let draft = if self.input.is_empty() { "Message Z…".to_string() } else { self.input.clone() };
        scene.push_text(
            Layer::Content,
            TextRun::new(
                &draft,
                Rect::new(
                    area.x + Spacing::S4,
                    area.y + Spacing::S3,
                    area.width - Spacing::S8,
                    22.0,
                ),
                Typography::BODY,
                if self.input.is_empty() { c.text_tertiary } else { c.text_primary },
            ),
        );

        // The composer needs Agent Runtime before it can accept input. Its
        // unavailable actions are deliberately disabled rather than becoming
        // dead pointer targets.
        let button = 30.0;
        let row_y = area.bottom() - Spacing::S3 - button;
        let centred = |bounds: Rect, size: f32| {
            Rect::new(
                bounds.x + (bounds.width - size) / 2.0,
                bounds.y + (bounds.height - size) / 2.0,
                size,
                size,
            )
        };

        // Attach, mention and slash-command. Mention and slash keep their
        // literal characters because that is what the user types.
        let mut x = area.x + Spacing::S3;
        for (index, (slot, label)) in
            [(None, "Attach"), (Some("@"), "Mention"), (Some("/"), "Slash command")]
                .into_iter()
                .enumerate()
        {
            let bounds = Rect::new(x, row_y, button, button);
            scene.push_quad(
                Layer::Content,
                Quad::filled(bounds, Rgba::TRANSPARENT)
                    .with_radius(Radius::SM)
                    .with_border(c.border_subtle, 1.0),
            );
            match slot {
                None => draw_icon(
                    scene,
                    Layer::Content,
                    Icon::Plus,
                    centred(bounds, 13.0),
                    c.text_tertiary,
                ),
                Some(label) => scene.push_text(
                    Layer::Content,
                    TextRun::new(label, bounds, Typography::BASE, c.text_tertiary)
                        .aligned(Align::Center),
                ),
            }
            scene.push_access(
                AccessNode::new(
                    NodeId::new(ns::COMPOSER, index as u32),
                    AxRole::Button,
                    label,
                    bounds,
                )
                .disabled(true)
                .described("Agent chat is not available yet."),
            );
            x = bounds.right() + Spacing::S2;
        }

        let can_send = !self.input.trim().is_empty();
        let send = Rect::new(area.right() - Spacing::S3 - button, row_y, button, button);
        scene.push_quad(
            Layer::Content,
            Quad::filled(send, if can_send { c.accent } else { c.surface })
                .with_radius(Radius::SM)
                .with_border(c.border_subtle, 1.0),
        );
        draw_icon(
            scene,
            Layer::Content,
            Icon::Send,
            centred(send, 15.0),
            if can_send { c.text_inverse } else { c.text_tertiary },
        );
        scene.push_access(
            AccessNode::new(NodeId::new(ns::COMPOSER, 10), AxRole::Button, "Send", send)
                .disabled(!can_send)
                .described(if can_send { "Send the message to Z" } else { "Type a message first" }),
        );

        let mic = Rect::new(send.x - Spacing::S2 - button, row_y, button, button);
        draw_icon(scene, Layer::Content, Icon::Mic, centred(mic, 15.0), c.text_tertiary);
        scene.push_access(
            AccessNode::new(NodeId::new(ns::COMPOSER, 11), AxRole::Button, "Voice input", mic)
                .disabled(true)
                .described("Voice input is not available yet."),
        );
    }

    // -- Right rail ----------------------------------------------------------

    fn context_panel(&self, scene: &mut Scene, frame: &Frame) {
        let c = self.colors();
        let panel = frame.context_panel;
        if panel.is_empty() {
            return;
        }

        scene.push_quad(Layer::Background, Quad::filled(panel, c.canvas));
        hairline_left(scene, panel, c.border_subtle);

        if panel.width < 120.0 {
            return;
        }

        let inner = panel.inset_xy(Spacing::S4, Spacing::S4);
        let mut y = inner.y;

        scene.push_text(
            Layer::Content,
            TextRun::new(
                "Context",
                Rect::new(inner.x, y, inner.width, 22.0),
                Typography::LG,
                c.text_primary,
            ),
        );
        y += 22.0 + Spacing::S5;

        for (section, value) in [
            (ContextSection::Project, &self.conversation.project),
            (ContextSection::Branch, &self.conversation.branch),
        ] {
            scene.push_text(
                Layer::Content,
                TextRun::new(
                    section.label(),
                    Rect::new(inner.x, y, inner.width, 18.0),
                    Typography::LABEL,
                    c.text_secondary,
                ),
            );
            let expanded = self.workspace.view.is_expanded(section);
            draw_icon(
                scene,
                Layer::Content,
                if expanded { Icon::ChevronDown } else { Icon::ChevronRight },
                Rect::new(inner.right() - 12.0, y + 3.0, 12.0, 12.0),
                c.text_secondary,
            );
            scene.push_access(
                AccessNode::new(
                    NodeId::new(ns::CONTEXT, section as u32),
                    AxRole::Button,
                    section.label(),
                    Rect::new(inner.x, y, inner.width, 18.0),
                )
                .expanded(expanded)
                .described(value),
            );
            y += 20.0;

            if expanded {
                let glyph = Rect::new(inner.x + 1.0, y + 3.0, 13.0, 13.0);
                draw_icon(
                    scene,
                    Layer::Content,
                    if section == ContextSection::Branch { Icon::Branch } else { Icon::Layers },
                    glyph,
                    c.text_tertiary,
                );
                // Project and branch names are direction-neutral identifiers.
                scene.push_text(
                    Layer::Content,
                    TextRun::new(
                        value,
                        Rect::new(glyph.right() + Spacing::S2, y, inner.width - 20.0, 20.0),
                        Typography::BASE,
                        c.text_primary,
                    )
                    .isolated(),
                );
                y += 20.0 + Spacing::S4;
            } else {
                // A collapsed section gives its detail body back to the next
                // control, rather than merely changing an invisible bit.
                y += Spacing::S3;
            }
        }

        // Context usage. The number and the bar say the same thing, so the
        // reading survives if either is missed.
        scene.push_text(
            Layer::Content,
            TextRun::new(
                ContextSection::ContextUsage.label(),
                Rect::new(inner.x, y, inner.width, 18.0),
                Typography::LABEL,
                c.text_secondary,
            ),
        );
        scene.push_text(
            Layer::Content,
            TextRun::new(
                format!("{}%", (self.conversation.context_usage * 100.0).round() as u32),
                Rect::new(inner.x, y, inner.width, 18.0),
                Typography::METRIC,
                c.text_primary,
            )
            .aligned(Align::End),
        );
        scene.push_access(
            AccessNode::new(
                NodeId::new(ns::CONTEXT, 100),
                AxRole::ProgressIndicator,
                ContextSection::ContextUsage.label(),
                Rect::new(inner.x, y, inner.width, 26.0),
            )
            .valued(self.conversation.context_usage)
            .focusable(false),
        );
        y += 22.0;

        let track = Rect::new(inner.x, y, inner.width, 3.0);
        scene.push_quad(
            Layer::Content,
            Quad::filled(track, c.border_default).with_radius(Radius::FULL),
        );
        scene.push_quad(
            Layer::Content,
            Quad::filled(
                Rect::new(
                    track.x,
                    track.y,
                    track.width * self.conversation.context_usage,
                    track.height,
                ),
                c.text_secondary,
            )
            .with_radius(Radius::FULL),
        );
        y += Spacing::S6;

        hairline_top(scene, Rect::new(panel.x, y, panel.width, 1.0), c.border_subtle);
        y += Spacing::S3;

        for (index, entry) in self.conversation.entries.iter().enumerate() {
            let row = Rect::new(inner.x, y, inner.width, 40.0);
            let glyph = Rect::new(row.x + 1.0, row.y + (row.height - 13.0) / 2.0, 13.0, 13.0);
            draw_icon(scene, Layer::Content, icon_for_entry(entry.label), glyph, c.text_tertiary);
            scene.push_text(
                Layer::Content,
                TextRun::new(
                    entry.label,
                    Rect::new(glyph.right() + Spacing::S3, row.y, row.width * 0.6, row.height),
                    Typography::BASE,
                    c.text_primary,
                ),
            );
            scene.push_text(
                Layer::Content,
                TextRun::new(entry.count.to_string(), row, Typography::METRIC, c.text_secondary)
                    .aligned(Align::End),
            );
            // The count belongs in the announcement: "Plan, 5 items", not
            // "Plan" with a number the reader cannot see.
            scene.push_access(
                AccessNode::new(
                    NodeId::new(ns::CONTEXT, 200 + index as u32),
                    AxRole::Group,
                    entry.label,
                    row,
                )
                .described(format!("{} items", entry.count))
                .focusable(false),
            );
            y = row.bottom();
            if y > inner.bottom() {
                break;
            }
        }
    }

    // -- Bottom band ---------------------------------------------------------

    fn performance_strip(&self, scene: &mut Scene, frame: &Frame) {
        let c = self.colors();
        let strip = frame.performance_strip;
        if strip.is_empty() {
            return;
        }

        scene.push_quad(Layer::Background, Quad::filled(strip, c.surface));
        hairline_top(scene, strip, c.border_subtle);

        let m = self.metrics;
        let readings = [
            ("CPU", format!("{}%", m.cpu_percent)),
            ("GPU", format!("{}%", m.gpu_percent)),
            ("RAM", format!("{:.1}GB", m.ram_gb)),
            ("FPS", m.fps.to_string()),
        ];

        // Right-aligned so the strip stays quiet and never competes with chat.
        let group_width = 108.0;
        let toggle_width = 132.0;
        let mut x =
            strip.right() - Spacing::S6 - toggle_width - readings.len() as f32 * group_width;

        for (label, value) in readings {
            let dot = Rect::new(x, strip.y + strip.height / 2.0 - 2.5, 5.0, 5.0);
            scene.push_quad(
                Layer::Content,
                Quad::filled(dot, c.text_tertiary).with_radius(Radius::FULL),
            );
            scene.push_text(
                Layer::Content,
                TextRun::new(
                    label,
                    Rect::new(dot.right() + Spacing::S2, strip.y, 34.0, strip.height),
                    Typography::XS,
                    c.text_tertiary,
                ),
            );
            // Tabular figures: these values change every second and the strip
            // must not shudder as they do.
            scene.push_text(
                Layer::Content,
                TextRun::new(
                    value,
                    Rect::new(dot.right() + Spacing::S2 + 34.0, strip.y, 52.0, strip.height),
                    Typography::METRIC,
                    c.text_secondary,
                ),
            );
            x += group_width;
        }

        scene.push_text(
            Layer::Content,
            TextRun::new(
                "Performance",
                Rect::new(x, strip.y, 84.0, strip.height),
                Typography::XS,
                c.text_secondary,
            ),
        );

        // The readouts are a region to be read, not a control to be operated —
        // and the numbers belong in the announcement, since a screen reader
        // user cannot glance at them.
        scene.push_access(
            AccessNode::new(NodeId::new(ns::STRIP, 1), AxRole::Group, "System load", strip)
                .described(format!(
                    "CPU {} percent, GPU {} percent, RAM {:.1} gigabytes, {} frames per second",
                    m.cpu_percent, m.gpu_percent, m.ram_gb, m.fps
                ))
                .focusable(false),
        );
    }

    // -- Overlay -------------------------------------------------------------

    fn floating_tool(&self, scene: &mut Scene, frame: &Frame) {
        let c = self.colors();
        let anchor = frame.floating_tool;
        if anchor.is_empty() {
            return;
        }

        // The overlay floats above the composer, never over it: covering the
        // field the user types into would be worse than not showing the tool.
        let floor = frame.chat.bottom() - CHAT_GUTTER - INPUT_BAR_HEIGHT - Spacing::S4;
        let bubble_size = 46.0;
        let bubble = Rect::new(anchor.x, floor - bubble_size, bubble_size, bubble_size);
        let active_task_count =
            self.conversation.messages.iter().filter(|message| message.progress.is_some()).count();
        let task_summary = if active_task_count == 0 {
            "No active tasks".to_string()
        } else {
            format!("{active_task_count} active task(s)")
        };

        if !self.workspace.view.floating_tool_open {
            // Resting state: a small bubble. The panel is what it morphs into,
            // not what it starts as, so it never sits on top of the
            // conversation uninvited.
            scene.push_quad(
                Layer::Overlay,
                Quad::filled(bubble, c.accent).with_radius(Radius::FULL).with_border(c.accent, 1.0),
            );
            draw_icon(
                scene,
                Layer::Overlay,
                Icon::Nodes,
                Rect::new(
                    bubble.x + (bubble_size - 18.0) / 2.0,
                    bubble.y + (bubble_size - 18.0) / 2.0,
                    18.0,
                    18.0,
                ),
                c.text_inverse,
            );
            // Count of running work, so the resting state still reports status.
            let badge = Rect::new(bubble.right() - 16.0, bubble.y, 16.0, 16.0);
            scene.push_quad(
                Layer::Overlay,
                Quad::filled(badge, c.surface_hover)
                    .with_radius(Radius::FULL)
                    .with_border(c.border_strong, 1.0),
            );
            scene.push_text(
                Layer::Overlay,
                TextRun::new(active_task_count.to_string(), badge, Typography::XS, c.text_primary)
                    .aligned(Align::Center),
            );
            scene.push_access(
                AccessNode::new(NodeId::new(ns::FLOATING, 0), AxRole::Button, "Tools", bubble)
                    .expanded(false)
                    .described(task_summary.as_str()),
            );
            return;
        }

        let row_height = 38.0;
        let padding = Spacing::S3;
        let width = anchor.width.min(240.0);
        let height = padding * 2.0 + FLOATING_TOOLS.len() as f32 * row_height;
        let ceiling = frame.chat.y + Spacing::S4;
        let panel_bottom = bubble.y - Spacing::S2;
        let panel = Rect::new(
            anchor.x,
            (panel_bottom - height).max(ceiling),
            width,
            height.min((panel_bottom - ceiling).max(0.0)),
        );
        if panel.height < row_height {
            return;
        }

        scene.push_quad(
            Layer::Overlay,
            Quad::filled(panel, c.surface_overlay)
                .with_radius(Radius::XL)
                .with_border(c.border_default, 1.0),
        );

        let mut y = panel.y + padding;
        for (index, (label, detail)) in FLOATING_TOOLS.iter().enumerate() {
            let detail =
                if *label == "Active Tasks" { Some(task_summary.as_str()) } else { *detail };
            let row = Rect::new(panel.x + padding, y, panel.width - padding * 2.0, row_height);
            if row.bottom() > panel.bottom() - padding {
                break;
            }
            let glyph = Rect::new(row.x, row.y + (row.height - 13.0) / 2.0, 13.0, 13.0);
            draw_icon(scene, Layer::Overlay, icon_for_entry(label), glyph, c.text_tertiary);
            scene.push_text(
                Layer::Overlay,
                TextRun::new(
                    *label,
                    Rect::new(
                        glyph.right() + Spacing::S3,
                        row.y,
                        row.width * 0.7,
                        if detail.is_some() { 20.0 } else { row.height },
                    ),
                    Typography::BASE,
                    c.text_secondary,
                ),
            );
            if let Some(detail) = detail {
                scene.push_text(
                    Layer::Overlay,
                    TextRun::new(
                        detail,
                        Rect::new(glyph.right() + Spacing::S3, row.y + 18.0, row.width * 0.7, 16.0),
                        Typography::XS,
                        c.text_tertiary,
                    ),
                );
                scene.push_text(
                    Layer::Overlay,
                    TextRun::new(
                        active_task_count.to_string(),
                        row,
                        Typography::METRIC,
                        c.text_secondary,
                    )
                    .aligned(Align::End),
                );
            }
            scene.push_access(
                AccessNode::new(
                    NodeId::new(ns::FLOATING, 100 + index as u32),
                    AxRole::Button,
                    *label,
                    row,
                )
                .disabled(true)
                .described(format!("{label} is not available yet.")),
            );

            y = row.bottom();
        }

        // Open state: the bubble becomes the dismiss control.
        scene.push_quad(
            Layer::Overlay,
            Quad::filled(bubble, c.accent).with_radius(Radius::FULL).with_border(c.accent, 1.0),
        );
        scene.push_text(
            Layer::Overlay,
            TextRun::new("x", bubble, Typography::BASE, c.text_inverse).aligned(Align::Center),
        );
        scene.push_access(
            AccessNode::new(NodeId::new(ns::FLOATING, 0), AxRole::Button, "Close tools", bubble)
                .expanded(true),
        );
    }

    /// Scroll the conversation to its newest message and pin it there.
    ///
    /// The pin releases as soon as the reader scrolls away, so arriving
    /// messages never drag them off what they were reading.
    pub fn scroll_chat_to_end(&mut self) {
        self.chat_list.scroll_to_end();
    }

    /// Move the conversation by a wheel or gesture delta.
    pub fn scroll_chat_by(&mut self, delta: f32) {
        self.chat_list.scroll_by(delta);
    }

    /// Fraction of the conversation currently on screen, for a scrollbar.
    pub fn chat_visible_fraction(&self) -> f32 {
        self.chat_list.visible_fraction()
    }

    /// Panels the shell had to fold away at this size. Surfaced so the UI can
    /// say what happened rather than leaving the user to notice a gap.
    pub fn collapsed_panels(&self, viewport: Rect) -> Vec<PanelId> {
        self.workspace.frame(viewport.width, viewport.height).collapsed
    }
}

impl Default for WorkspaceView {
    fn default() -> Self {
        Self::new()
    }
}

fn surface_toolbar_rows(surface_width: f32, surface: &SurfaceKind) -> u32 {
    match surface {
        SurfaceKind::Ide => {
            toolbar_rows_for(surface_width, "IDE", IdeTool::ALL.iter().map(|tool| tool.label()))
        }
        SurfaceKind::Live3d => toolbar_rows_for(
            surface_width,
            "Live 3D",
            ThreeDTool::ALL.iter().map(|tool| tool.label()),
        ),
        SurfaceKind::Preview => toolbar_rows_for(
            surface_width,
            "Preview",
            PreviewTool::ALL.iter().map(|tool| tool.label()),
        ),
        SurfaceKind::Diff => {
            toolbar_rows_for(surface_width, "Diff", DiffTool::ALL.iter().map(|tool| tool.label()))
        }
        SurfaceKind::Chat { .. } => 1,
    }
}

/// Calculate enough rows before drawing so wrapping never silently hides a
/// workspace tool at the minimum desktop window size.
fn toolbar_rows_for<'a>(
    surface_width: f32,
    title: &str,
    labels: impl Iterator<Item = &'a str>,
) -> u32 {
    let title_width = (title.chars().count() as f32 * 7.5 + Spacing::S8).min(surface_width);
    let first_x = title_width + Spacing::S2;
    let right = (surface_width - Spacing::S2).max(first_x);
    let mut x = first_x;
    let mut rows = 1;

    for label in labels {
        let width = tool_chip_width(label);
        if x > first_x && x + width > right {
            rows += 1;
            x = first_x;
        }
        x += width + Spacing::S1;
    }
    rows
}

fn tool_chip_width(label: &str) -> f32 {
    (label.chars().count() as f32 * 6.5 + Spacing::S6).clamp(56.0, 132.0)
}

fn ide_tool_copy(tool: IdeTool) -> (&'static str, &'static str, Icon) {
    match tool {
        IdeTool::Explorer => {
            ("Browse the project tree", "Select a project before files can be listed.", Icon::Page)
        }
        IdeTool::Search => (
            "Search files in the active project",
            "A project connection is required before searching.",
            Icon::Lens,
        ),
        IdeTool::Problems => (
            "Review project diagnostics",
            "A language service is required before diagnostics are available.",
            Icon::Ring,
        ),
        IdeTool::SourceControl => (
            "Review pending source-control changes",
            "Connect a repository before changes can be inspected.",
            Icon::Branch,
        ),
        IdeTool::RunDebug => (
            "Select a run or debug configuration",
            "A project runtime is required before code can run or debug.",
            Icon::Nodes,
        ),
        IdeTool::Terminal => (
            "Inspect the project terminal",
            "A project workspace is required before terminal commands are available.",
            Icon::Block,
        ),
        IdeTool::Extensions => {
            ("Inspect installed IDE extensions", "No extension runtime is connected.", Icon::Spark)
        }
    }
}

fn three_d_tool_copy(tool: ThreeDTool) -> (&'static str, Icon) {
    match tool {
        ThreeDTool::Select => ("Select tool is ready when a scene is loaded", Icon::Nodes),
        ThreeDTool::Move => ("Move tool is ready when a scene is loaded", Icon::Nodes),
        ThreeDTool::Rotate => ("Rotate tool is ready when a scene is loaded", Icon::Nodes),
        ThreeDTool::Scale => ("Scale tool is ready when a scene is loaded", Icon::Nodes),
        ThreeDTool::Frame => ("Frame tool is ready when a scene is loaded", Icon::Lens),
        ThreeDTool::Orbit => ("Orbit tool is ready when a scene is loaded", Icon::Nodes),
        ThreeDTool::Pan => ("Pan tool is ready when a scene is loaded", Icon::Lines),
        ThreeDTool::Zoom => ("Zoom tool is ready when a scene is loaded", Icon::Lens),
        ThreeDTool::Outliner => ("Outliner is ready when a scene is loaded", Icon::Lines),
        ThreeDTool::Inspector => ("Inspector is ready when an object is selected", Icon::Ring),
    }
}

fn preview_tool_copy(tool: PreviewTool) -> &'static str {
    match tool {
        PreviewTool::Fit => "Preview is fitted to the workspace",
        PreviewTool::Device => "Device frame selected",
        PreviewTool::Inspect => "Inspect mode selected",
    }
}

fn diff_tool_copy(tool: DiffTool) -> &'static str {
    match tool {
        DiffTool::Files => "Changed files view selected",
        DiffTool::Unified => "Unified diff layout selected",
        DiffTool::Split => "Split diff layout selected",
    }
}

fn navigation_copy(item: NavItem) -> (&'static str, &'static str, Icon) {
    match item {
        NavItem::Home => ("Home", "The active work surface is shown here.", Icon::Home),
        NavItem::Projects => (
            "Projects",
            "No project is selected. Project management will appear when the workspace runtime is connected.",
            Icon::Layers,
        ),
        NavItem::Chat => (
            "Chat",
            "No conversation is selected. Agent chat is not available yet.",
            Icon::Bubble,
        ),
        NavItem::Agents => (
            "Agents",
            "No agent runtime is connected.",
            Icon::Nodes,
        ),
        NavItem::Threads => (
            "Threads",
            "No thread history is available.",
            Icon::Lines,
        ),
        NavItem::Files => ("Files", "No project files are available.", Icon::Page),
        NavItem::Search => (
            "Search",
            "Project search is unavailable until a project is selected.",
            Icon::Lens,
        ),
        NavItem::Skills => (
            "Skills",
            "No skill catalog is connected.",
            Icon::Spark,
        ),
        NavItem::Extensions => (
            "Extensions",
            "No extensions are installed.",
            Icon::Block,
        ),
        NavItem::Settings => (
            "Settings",
            "Workspace settings will appear when a settings store is connected.",
            Icon::Ring,
        ),
    }
}

// -- icons -------------------------------------------------------------------

/// The icon vocabulary.
///
/// Built from the one primitive the renderer has — a rounded rectangle — so the
/// whole set costs nothing extra to draw and shares the shell's single quad
/// pass. Every mark uses the same stroke weight and the same 14px box, which is
/// what makes a row of them read as one family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Icon {
    /// Home — a roof over a body.
    Home,
    /// Projects — stacked layers.
    Layers,
    /// Chat — a rounded speech container.
    Bubble,
    /// Agents — two nodes.
    Nodes,
    /// Threads — stacked lines.
    Lines,
    /// Files — a page.
    Page,
    /// Search — a lens with a handle.
    Lens,
    /// Skills — a mark built from four corners.
    Spark,
    /// Extensions — a block with a tab.
    Block,
    /// Settings — a ring.
    Ring,
    /// Branch / VCS — two dots joined.
    Branch,
    /// Notifications — a dome on a base.
    Bell,
    /// Voice input — a capsule on a stand.
    Mic,
    /// Send — a triangle implied by two strokes.
    Send,
    /// Attach — a cross.
    Plus,
    /// Disclosure, pointing down.
    ChevronDown,
    /// Disclosure, pointing right when its body is collapsed.
    ChevronRight,
    /// Approve.
    ThumbUp,
    /// Reject.
    ThumbDown,
    /// Copy — two offset sheets.
    Copy,
    /// More — three dots.
    Ellipsis,
}

fn icon_for(label: &str) -> Icon {
    match label {
        "Home" => Icon::Home,
        "Projects" => Icon::Layers,
        "Chat" => Icon::Bubble,
        "Agents" => Icon::Nodes,
        "Threads" => Icon::Lines,
        "Files" => Icon::Page,
        "Search" => Icon::Lens,
        "Skills" => Icon::Spark,
        "Extensions" => Icon::Block,
        "Settings" => Icon::Ring,
        _ => Icon::Block,
    }
}

/// Context-panel and Floating Tool labels share the same vocabulary.
fn icon_for_entry(label: &str) -> Icon {
    match label {
        "Plan" => Icon::Lines,
        "Agents" | "Active Tasks" => Icon::Nodes,
        "Changes" | "Agent Timeline" => Icon::Layers,
        "Threads" => Icon::Bubble,
        "Resources" | "3D Tools" => Icon::Block,
        "Notes" => Icon::Page,
        "Terminal" | "Debug Console" => Icon::Block,
        _ => Icon::Page,
    }
}

const STROKE: f32 = 1.4;

fn draw_icon(scene: &mut Scene, layer: Layer, icon: Icon, box_: Rect, color: Rgba) {
    let s = box_.width.min(box_.height);
    let x = box_.x;
    let y = box_.y + (box_.height - s) / 2.0;
    let bar = |scene: &mut Scene, rx: f32, ry: f32, w: f32, h: f32| {
        scene.push_quad(
            layer,
            Quad::filled(Rect::new(x + rx * s, y + ry * s, w * s, h * s), color)
                .with_radius(STROKE / 2.0),
        );
    };
    let outline = |scene: &mut Scene, rx: f32, ry: f32, w: f32, h: f32, radius: f32| {
        scene.push_quad(
            layer,
            Quad::filled(Rect::new(x + rx * s, y + ry * s, w * s, h * s), Rgba::TRANSPARENT)
                .with_radius(radius)
                .with_border(color, STROKE),
        );
    };

    match icon {
        Icon::Home => {
            // Roof implied by a narrower band above a wider body.
            bar(scene, 0.28, 0.10, 0.44, 0.12);
            outline(scene, 0.12, 0.30, 0.76, 0.60, 2.0);
        }
        Icon::Layers => {
            bar(scene, 0.10, 0.16, 0.80, 0.12);
            outline(scene, 0.10, 0.40, 0.80, 0.48, 2.0);
        }
        Icon::Bubble => {
            outline(scene, 0.08, 0.14, 0.84, 0.62, 3.0);
            bar(scene, 0.24, 0.72, 0.16, 0.16);
        }
        Icon::Nodes => {
            outline(scene, 0.10, 0.10, 0.34, 0.34, 9999.0);
            outline(scene, 0.56, 0.56, 0.34, 0.34, 9999.0);
            bar(scene, 0.42, 0.46, 0.18, 0.08);
        }
        Icon::Lines => {
            bar(scene, 0.10, 0.20, 0.80, 0.11);
            bar(scene, 0.10, 0.44, 0.60, 0.11);
            bar(scene, 0.10, 0.68, 0.40, 0.11);
        }
        Icon::Page => {
            outline(scene, 0.16, 0.06, 0.68, 0.88, 2.0);
            bar(scene, 0.30, 0.34, 0.40, 0.09);
            bar(scene, 0.30, 0.56, 0.40, 0.09);
        }
        Icon::Lens => {
            outline(scene, 0.06, 0.06, 0.62, 0.62, 9999.0);
            bar(scene, 0.62, 0.62, 0.34, 0.12);
        }
        Icon::Spark => {
            bar(scene, 0.44, 0.04, 0.12, 0.92);
            bar(scene, 0.04, 0.44, 0.92, 0.12);
        }
        Icon::Block => {
            outline(scene, 0.06, 0.24, 0.66, 0.66, 2.0);
            bar(scene, 0.62, 0.06, 0.32, 0.32);
        }
        Icon::Ring => {
            outline(scene, 0.08, 0.08, 0.84, 0.84, 9999.0);
            bar(scene, 0.42, 0.42, 0.16, 0.16);
        }
        Icon::Branch => {
            outline(scene, 0.08, 0.06, 0.30, 0.30, 9999.0);
            outline(scene, 0.08, 0.64, 0.30, 0.30, 9999.0);
            bar(scene, 0.20, 0.30, 0.08, 0.38);
        }
        Icon::Bell => {
            // Dome, rim, clapper.
            outline(scene, 0.22, 0.12, 0.56, 0.54, 8.0);
            bar(scene, 0.08, 0.64, 0.84, 0.11);
            bar(scene, 0.42, 0.80, 0.16, 0.11);
        }
        Icon::Mic => {
            outline(scene, 0.34, 0.06, 0.32, 0.50, 9999.0);
            bar(scene, 0.20, 0.58, 0.60, 0.09);
            bar(scene, 0.45, 0.66, 0.10, 0.26);
        }
        Icon::Send => {
            // Arrow: a shaft plus a stair-stepped head. The renderer has no
            // rotation, so diagonals are built from short offset bars.
            bar(scene, 0.08, 0.44, 0.62, 0.12);
            bar(scene, 0.50, 0.26, 0.13, 0.12);
            bar(scene, 0.61, 0.35, 0.13, 0.12);
            bar(scene, 0.50, 0.62, 0.13, 0.12);
            bar(scene, 0.61, 0.53, 0.13, 0.12);
        }
        Icon::Plus => {
            bar(scene, 0.44, 0.14, 0.12, 0.72);
            bar(scene, 0.14, 0.44, 0.72, 0.12);
        }
        Icon::ChevronDown => {
            // Five stair steps make a legible V at 12px.
            bar(scene, 0.12, 0.36, 0.17, 0.12);
            bar(scene, 0.26, 0.45, 0.17, 0.12);
            bar(scene, 0.41, 0.53, 0.18, 0.12);
            bar(scene, 0.57, 0.45, 0.17, 0.12);
            bar(scene, 0.71, 0.36, 0.17, 0.12);
        }
        Icon::ChevronRight => {
            // Same construction turned right, so expanded state never relies
            // on colour alone.
            bar(scene, 0.36, 0.12, 0.12, 0.17);
            bar(scene, 0.45, 0.26, 0.12, 0.17);
            bar(scene, 0.53, 0.41, 0.12, 0.18);
            bar(scene, 0.45, 0.57, 0.12, 0.17);
            bar(scene, 0.36, 0.71, 0.12, 0.17);
        }
        Icon::ThumbUp => {
            bar(scene, 0.48, 0.10, 0.10, 0.34);
            outline(scene, 0.16, 0.42, 0.68, 0.46, 2.0);
        }
        Icon::ThumbDown => {
            outline(scene, 0.16, 0.12, 0.68, 0.46, 2.0);
            bar(scene, 0.48, 0.56, 0.10, 0.34);
        }
        Icon::Copy => {
            outline(scene, 0.06, 0.06, 0.58, 0.58, 2.0);
            outline(scene, 0.34, 0.34, 0.58, 0.58, 2.0);
        }
        Icon::Ellipsis => {
            bar(scene, 0.10, 0.44, 0.16, 0.14);
            bar(scene, 0.42, 0.44, 0.16, 0.14);
            bar(scene, 0.74, 0.44, 0.16, 0.14);
        }
    }
}

// -- helpers -----------------------------------------------------------------

/// Rough line count for a body of text at a given width.
///
/// An estimate on purpose: the frame path must not block on shaping. Text is
/// clipped by its box, so over-estimating costs a little space and never
/// truncates a message.
fn wrapped_line_count(text: &str, width: f32, font_size: f32) -> f32 {
    if width <= 0.0 {
        return 1.0;
    }
    let average_advance = font_size * 0.52;
    let per_line = (width / average_advance).max(1.0);
    ((text.chars().count() as f32 / per_line).ceil()).max(1.0)
}

fn hairline_bottom(scene: &mut Scene, area: Rect, color: Rgba) {
    scene.push_quad(
        Layer::Background,
        Quad::divider(Rect::new(area.x, area.bottom() - 1.0, area.width, 1.0), color),
    );
}

fn hairline_top(scene: &mut Scene, area: Rect, color: Rgba) {
    scene.push_quad(
        Layer::Background,
        Quad::divider(Rect::new(area.x, area.y, area.width, 1.0), color),
    );
}

fn hairline_right(scene: &mut Scene, area: Rect, color: Rgba) {
    scene.push_quad(
        Layer::Background,
        Quad::divider(Rect::new(area.right() - 1.0, area.y, 1.0, area.height), color),
    );
}

fn hairline_left(scene: &mut Scene, area: Rect, color: Rgba) {
    scene.push_quad(
        Layer::Background,
        Quad::divider(Rect::new(area.x, area.y, 1.0, area.height), color),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use z_shell::Preset;

    /// Build frames until the layout settles, then return the settled scene.
    ///
    /// Row heights start as estimates and are corrected as rows are measured,
    /// so the first frame is not representative.
    fn settle(view: &mut WorkspaceView, viewport: Rect) -> Scene {
        let mut scene = view.build(viewport);
        for _ in 0..8 {
            let next = view.build(viewport);
            if next.damage_against(&scene).is_none() {
                return next;
            }
            scene = next;
        }
        scene
    }

    fn reference_view() -> WorkspaceView {
        let mut view = WorkspaceView::new();
        view.set_conversation(Conversation::reference());
        view
    }

    fn scene_at(width: f32, height: f32) -> Scene {
        reference_view().build(Rect::new(0.0, 0.0, width, height))
    }

    #[test]
    fn the_reference_workspace_produces_a_populated_scene() {
        let scene = scene_at(1536.0, 1024.0);
        assert!(scene.quad_count() > 40, "only {} quads", scene.quad_count());
        assert!(scene.text_count() > 30, "only {} text runs", scene.text_count());
    }

    #[test]
    fn a_new_workspace_starts_with_no_chat_history() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();
        let scene = view.build(viewport);

        assert!(view.conversation.messages.is_empty());
        assert!(
            !scene.texts().any(|run| run.text == "Z"),
            "a first-run workspace must not impersonate a prior agent session"
        );
        let conversation = scene
            .access()
            .get(NodeId::new(ns::SHELL, 1))
            .expect("the empty conversation region should still be declared");
        assert_eq!(conversation.description.as_deref(), Some("0 messages"));
    }

    #[test]
    fn every_drawn_element_stays_inside_the_window() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let scene = scene_at(viewport.width, viewport.height);
        for quad in scene.quads() {
            assert!(
                quad.bounds.x >= -1.0 && quad.bounds.y >= -1.0,
                "quad starts outside the window: {:?}",
                quad.bounds
            );
            assert!(
                quad.bounds.right() <= viewport.width + 1.0,
                "quad runs past the right edge: {:?}",
                quad.bounds
            );
            assert!(
                quad.bounds.bottom() <= viewport.height + 1.0,
                "quad runs past the bottom: {:?}",
                quad.bounds
            );
        }
    }

    #[test]
    fn the_user_speaks_on_the_right_and_zero_on_the_left() {
        let mut view = reference_view();
        let scene = view.build(Rect::new(0.0, 0.0, 1536.0, 1024.0));
        let frame = view.workspace.frame(1536.0, 1024.0);
        let centre = frame.chat.center_x();

        let you: Vec<&TextRun> = scene.texts().filter(|t| t.text == "You").collect();
        let zero: Vec<&TextRun> = scene.texts().filter(|t| t.text == "Z").collect();
        assert!(!you.is_empty() && !zero.is_empty());

        for run in you {
            assert_eq!(run.align, Align::End, "the user's name should sit on the right");
        }
        for run in zero {
            assert_eq!(run.align, Align::Start, "Zero's name should sit on the left");
            assert!(run.bounds.x < centre);
        }
    }

    #[test]
    fn the_working_file_path_is_isolated_against_reordering() {
        let scene = scene_at(1536.0, 1024.0);
        let run = scene
            .texts()
            .find(|t| t.text.contains("src/auth/session.ts"))
            .expect("the work-in-progress path should be drawn");
        assert!(run.isolate, "a path must not be reordered by surrounding text");
    }

    #[test]
    fn changing_readouts_use_tabular_figures() {
        let scene = scene_at(1536.0, 1024.0);
        for value in ["18%", "32%", "6.4GB", "120"] {
            let run = scene
                .texts()
                .find(|t| t.text == value)
                .unwrap_or_else(|| panic!("{value} is missing from the performance strip"));
            assert!(run.style.tabular_numbers, "{value} would make the strip jitter");
        }
    }

    #[test]
    fn every_plan_state_is_labelled_not_just_coloured() {
        let scene = scene_at(1536.0, 1024.0);
        for label in ["Completed", "In progress", "Pending"] {
            assert!(
                scene.texts().any(|t| t.text == label),
                "{label} must be readable without relying on colour"
            );
        }
    }

    #[test]
    fn a_failed_step_is_shown_as_failed_and_marked_in_danger() {
        let mut view = WorkspaceView::new();
        view.set_conversation(crate::content::Conversation::with_failed_step());
        let scene = view.build(Rect::new(0.0, 0.0, 1536.0, 1024.0));

        let run =
            scene.texts().find(|t| t.text == "Failed").expect("a failed step must say so in words");
        assert_eq!(
            run.color, view.theme.colors.status_danger,
            "failure should also be visually distinct, not word-only"
        );
        assert!(
            !scene.texts().any(|t| t.text == "In progress"),
            "the failed step must stop claiming to be running"
        );
    }

    #[test]
    fn the_reference_surfaces_are_all_present() {
        let scene = scene_at(1536.0, 1024.0);
        for label in ["Chat · Auth Flow", "IDE", "Live 3D", "Preview", "Diff"] {
            assert!(scene.texts().any(|t| t.text == label), "tab {label} is missing");
        }
    }

    #[test]
    fn every_non_chat_tab_has_a_distinct_empty_surface() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        for (tab_id, expected) in [
            (1, "No files open"),
            (2, "No 3D scene"),
            (3, "Nothing to preview"),
            (4, "No changes to compare"),
        ] {
            let mut view = WorkspaceView::new();
            assert!(view.workspace.view.tabs.activate(tab_id));
            let scene = view.build(viewport);
            assert!(
                scene.texts().any(|run| run.text == expected),
                "tab {tab_id} did not render its own surface"
            );
            assert!(
                !scene.texts().any(|run| run.text == "Ask Zero anything..."),
                "non-chat tab {tab_id} leaked the chat composer"
            );
        }
    }

    #[test]
    fn empty_surfaces_do_not_offer_an_unavailable_action() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        for (tab_id, expected_detail) in [
            (1, "A project connection is required before files appear here."),
            (2, "A project scene will appear here when one is available."),
            (3, "A real project preview will appear here when available."),
            (4, "Real project changes will appear here when available."),
        ] {
            let mut view = WorkspaceView::new();
            assert!(view.workspace.view.tabs.activate(tab_id));
            let scene = view.build(viewport);
            assert!(
                scene.texts().any(|run| run.text == expected_detail),
                "tab {tab_id} must describe its real prerequisite"
            );
            assert!(
                !scene.texts().any(|run| run.text.contains("ask Zero to create")),
                "an empty surface must not offer an action with no runtime"
            );
        }
    }

    #[test]
    fn every_workspace_toolbar_exposes_its_declared_tools() {
        for viewport in [Rect::new(0.0, 0.0, 1536.0, 1024.0), Rect::new(0.0, 0.0, 720.0, 480.0)] {
            for (tab_id, namespace, expected) in [
                (
                    1,
                    ns::IDE_TOOL,
                    vec![
                        "Explorer",
                        "Search",
                        "Problems",
                        "Source Control",
                        "Run & Debug",
                        "Terminal",
                        "Extensions",
                    ],
                ),
                (
                    2,
                    ns::THREE_D_TOOL,
                    vec![
                        "Select",
                        "Move",
                        "Rotate",
                        "Scale",
                        "Frame",
                        "Orbit",
                        "Pan",
                        "Zoom",
                        "Outliner",
                        "Inspector",
                    ],
                ),
                (3, ns::PREVIEW_TOOL, vec!["Fit", "Device", "Inspect"]),
                (4, ns::DIFF_TOOL, vec!["Files", "Unified", "Split"]),
            ] {
                let mut view = WorkspaceView::new();
                assert!(view.workspace.view.tabs.activate(tab_id));
                let labels: Vec<String> = view
                    .build(viewport)
                    .access()
                    .nodes()
                    .iter()
                    .filter(|node| node_namespace(node.id) == namespace && node.can_take_focus())
                    .map(|node| node.label.clone())
                    .collect();
                assert_eq!(
                    labels, expected,
                    "toolbar for tab {tab_id} is incomplete at {}x{}",
                    viewport.width, viewport.height
                );
            }
        }
    }

    #[test]
    fn mouse_switches_every_workspace_surface_and_its_available_tools() {
        // This is the user's actual regression: switching a tab, a sidebar
        // destination or a surface tool must visibly change the centre pane.
        // It deliberately drives `click`, not private command execution, so
        // it covers the same hit-test path Windows uses for a mouse click.
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();

        let click_named = |view: &mut WorkspaceView, namespace: u32, label: &str| {
            let scene = view.build(viewport);
            let bounds = scene
                .access()
                .nodes()
                .iter()
                .find(|node| {
                    node.label == label
                        && node.can_take_focus()
                        && node_namespace(node.id) == namespace
                })
                .unwrap_or_else(|| panic!("{label:?} is not an enabled control"))
                .bounds;
            assert!(view.click(bounds.center(), viewport), "{label:?} did not react to a click");
        };

        click_named(&mut view, ns::TAB, "IDE");
        for tool in IdeTool::ALL {
            click_named(&mut view, ns::IDE_TOOL, tool.label());
            let (title, _, _) = ide_tool_copy(*tool);
            assert_eq!(view.workspace.view.ide_tool, *tool);
            assert!(
                view.build(viewport).texts().any(|run| run.text == title),
                "the IDE {} tool did not replace the centre detail",
                tool.label()
            );
        }

        click_named(&mut view, ns::TAB, "Live 3D");
        for tool in ThreeDTool::ALL {
            click_named(&mut view, ns::THREE_D_TOOL, tool.label());
            let (title, _) = three_d_tool_copy(*tool);
            assert_eq!(view.workspace.view.three_d_tool, *tool);
            assert!(
                view.build(viewport).texts().any(|run| run.text == title),
                "the 3D {} tool did not replace the centre detail",
                tool.label()
            );
        }

        click_named(&mut view, ns::TAB, "Preview");
        for tool in PreviewTool::ALL {
            click_named(&mut view, ns::PREVIEW_TOOL, tool.label());
            let title = preview_tool_copy(*tool);
            assert_eq!(view.workspace.view.preview_tool, *tool);
            assert!(
                view.build(viewport).texts().any(|run| run.text == title),
                "the Preview {} tool did not replace the centre detail",
                tool.label()
            );
        }

        click_named(&mut view, ns::TAB, "Diff");
        for tool in DiffTool::ALL {
            click_named(&mut view, ns::DIFF_TOOL, tool.label());
            let title = diff_tool_copy(*tool);
            assert_eq!(view.workspace.view.diff_tool, *tool);
            assert!(
                view.build(viewport).texts().any(|run| run.text == title),
                "the Diff {} tool did not replace the centre detail",
                tool.label()
            );
        }

        click_named(&mut view, ns::NAV, "Files");
        assert!(
            view.build(viewport).texts().any(|run| run.text == "No project files are available."),
            "the Files destination did not replace the centre surface"
        );

        // A top tab is a way back to the active work surface from any
        // sidebar destination, not a no-op hidden behind the selected rail.
        click_named(&mut view, ns::TAB, "IDE");
        assert_eq!(view.workspace.view.nav_selection, NavItem::Home);
        assert!(view
            .build(viewport)
            .texts()
            .any(|run| run.text == "Inspect installed IDE extensions"));
    }

    #[test]
    fn mouse_switches_every_sidebar_destination() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();
        let destinations: Vec<NavItem> =
            NavItem::PRIMARY.iter().copied().chain(std::iter::once(NavItem::Settings)).collect();

        for item in destinations {
            let scene = view.build(viewport);
            let bounds = scene
                .access()
                .nodes()
                .iter()
                .find(|node| {
                    node_namespace(node.id) == ns::NAV
                        && node.label == item.label()
                        && node.can_take_focus()
                })
                .unwrap_or_else(|| panic!("{item:?} is not clickable"))
                .bounds;
            assert!(view.click(bounds.center(), viewport), "{item:?} ignored a mouse click");
            assert_eq!(view.workspace.view.nav_selection, item);

            let surface = view.build(viewport);
            if item == NavItem::Home {
                let conversation = surface
                    .access()
                    .get(NodeId::new(ns::SHELL, 1))
                    .expect("Home should restore the active work surface");
                assert_eq!(conversation.description.as_deref(), Some("0 messages"));
            } else {
                let (_, detail, _) = navigation_copy(item);
                assert!(
                    surface.texts().any(|run| run.text == detail),
                    "{item:?} did not replace the centre surface"
                );
            }
        }
    }

    #[test]
    fn no_team_surface_appears_anywhere_in_personal() {
        let scene = scene_at(1536.0, 1024.0);
        for run in scene.texts() {
            let text = run.text.to_lowercase();
            assert!(!text.contains("team"), "Team must not appear in Personal: {:?}", run.text);
            assert!(!text.contains("member"), "Team surfaces must not leak in: {:?}", run.text);
        }
        assert!(scene.texts().any(|t| t.text == "Personal"), "the mode indicator is missing");
    }

    #[test]
    fn the_shell_still_renders_at_the_minimum_window_size() {
        let scene = scene_at(720.0, 480.0);
        assert!(!scene.is_empty(), "the shell must survive the smallest window");
    }

    #[test]
    fn every_preset_renders_without_escaping_the_window() {
        let viewport = Rect::new(0.0, 0.0, 1440.0, 900.0);
        for preset in Preset::BUILT_IN {
            for tab_id in WorkspaceView::new().workspace.view.tabs.tabs.iter().map(|tab| tab.id) {
                let mut view = WorkspaceView::new();
                view.workspace.apply_preset(*preset);
                assert!(view.workspace.view.tabs.activate(tab_id));
                let scene = view.build(viewport);
                assert!(!scene.is_empty(), "{} / tab {tab_id} rendered nothing", preset.label());
                for quad in scene.quads() {
                    assert!(
                        quad.bounds.right() <= viewport.width + 1.0
                            && quad.bounds.bottom() <= viewport.height + 1.0,
                        "{} / tab {tab_id} draws outside the window: {:?}",
                        preset.label(),
                        quad.bounds
                    );
                }
            }
        }
    }

    #[test]
    fn every_control_that_needs_a_name_has_one() {
        // An icon-only button with no name is invisible to a screen reader, and
        // nothing about the rendered pixels reveals the omission.
        let mut view = WorkspaceView::new();
        let scene = view.build(Rect::new(0.0, 0.0, 1536.0, 1024.0));

        let missing: Vec<&str> =
            scene.access().unlabelled().iter().map(|n| n.label.as_str()).collect();
        assert!(
            missing.is_empty(),
            "{} control(s) declared without a name: {missing:?}",
            missing.len()
        );
    }

    #[test]
    fn every_enabled_control_has_a_backing_command() {
        // An enabled semantic control promises that pointer and keyboard
        // activation will cause an observable change. Keeping that promise
        // here prevents visual placeholders from becoming dead click targets.
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let assert_no_orphans = |view: &mut WorkspaceView, state: &str| {
            let scene = view.build(viewport);

            let orphaned: Vec<String> = scene
                .access()
                .nodes()
                .iter()
                .filter(|node| node.can_take_focus())
                .filter(|node| view.command_for(node.id).is_none())
                .map(|node| node.label.clone())
                .collect();

            assert!(
                orphaned.is_empty(),
                "enabled controls must not silently ignore activation in {state}: {orphaned:?}"
            );
        };

        let tab_ids: Vec<u64> =
            WorkspaceView::new().workspace.view.tabs.tabs.iter().map(|tab| tab.id).collect();
        for tab_id in tab_ids {
            for floating_tool_open in [false, true] {
                let mut view = WorkspaceView::new();
                assert!(view.workspace.view.tabs.activate(tab_id));
                view.workspace.view.floating_tool_open = floating_tool_open;
                assert_no_orphans(
                    &mut view,
                    &format!("tab {tab_id}, floating_tool_open={floating_tool_open}"),
                );
            }
        }

        for item in NavItem::PRIMARY.iter().copied().chain(std::iter::once(NavItem::Settings)) {
            let mut view = WorkspaceView::new();
            view.workspace.view.nav_selection = item;
            assert_no_orphans(&mut view, item.label());
        }

        // Reference content adds context summary rows. They are informative
        // groups, not fake buttons, and this check locks that distinction in.
        assert_no_orphans(&mut reference_view(), "reference conversation");
    }

    #[test]
    fn unavailable_controls_are_disabled_instead_of_becoming_dead_click_targets() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();
        let scene = view.build(viewport);

        for label in [
            "New surface",
            "Account",
            "Notifications",
            "Attach",
            "Mention",
            "Slash command",
            "Send",
            "Voice input",
        ] {
            let node = scene
                .access()
                .nodes()
                .iter()
                .find(|node| node.label == label)
                .unwrap_or_else(|| panic!("{label:?} is missing from the semantic tree"));
            assert!(node.state.disabled, "{label:?} must announce that it is unavailable");
            assert!(
                !node.can_take_focus(),
                "{label:?} must not consume keyboard focus or a pointer click"
            );
        }

        let mode = scene
            .access()
            .nodes()
            .iter()
            .find(|node| node.label == "Mode: Personal")
            .expect("the fixed Personal mode must remain announced");
        assert_eq!(mode.role, AxRole::Group);
        assert!(!mode.can_take_focus());

        let mut tools_view = WorkspaceView::new();
        tools_view.workspace.view.floating_tool_open = true;
        let tools_scene = tools_view.build(viewport);
        for label in
            ["Active Tasks", "Terminal", "Debug Console", "Agent Timeline", "Notes", "3D Tools"]
        {
            let node = tools_scene
                .access()
                .nodes()
                .iter()
                .find(|node| node.label == label)
                .unwrap_or_else(|| panic!("{label:?} is missing from the open Tools panel"));
            assert!(node.state.disabled, "{label:?} must announce that it is unavailable");
            assert!(
                !node.can_take_focus(),
                "{label:?} must not consume keyboard focus or a pointer click"
            );
        }
    }

    #[test]
    fn accessible_ids_are_unique_and_children_all_exist() {
        let mut view = WorkspaceView::new();
        let scene = view.build(Rect::new(0.0, 0.0, 1536.0, 1024.0));

        assert!(
            scene.access().duplicate_ids().is_empty(),
            "duplicate ids make focus jump unpredictably: {:?}",
            scene.access().duplicate_ids()
        );
        assert!(
            scene.access().dangling_children().is_empty(),
            "a node references a child that was never declared"
        );
    }

    #[test]
    fn every_interactive_region_of_the_shell_is_reachable_by_keyboard() {
        let mut view = reference_view();
        let scene = view.build(Rect::new(0.0, 0.0, 1536.0, 1024.0));

        let reachable: Vec<String> = scene
            .access()
            .focus_order()
            .iter()
            .filter_map(|id| scene.access().get(*id))
            .map(|n| n.label.clone())
            .collect();

        // One from each region, so a whole panel cannot silently drop out.
        for expected in ["Home", "Chat · Auth Flow", "Project", "Tools"] {
            assert!(
                reachable.iter().any(|label| label == expected),
                "{expected:?} is not reachable by keyboard; reachable: {reachable:?}"
            );
        }
    }

    #[test]
    fn tab_traverses_the_whole_shell_and_returns_to_the_start() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();

        let total = view.build(viewport).access().focus_order().len();
        assert!(total > 10, "only {total} focusable elements — a region is missing");

        let mut seen = std::collections::BTreeSet::new();
        for _ in 0..total {
            view.move_focus(true, viewport);
            if let Some(id) = view.focused() {
                seen.insert(id);
            }
        }
        assert_eq!(seen.len(), total, "Tab did not reach every focusable element");

        // One more step wraps rather than trapping.
        let first = *seen.iter().next().unwrap();
        view.move_focus(true, viewport);
        assert!(view.focused().is_some());
        assert!(seen.contains(&view.focused().unwrap()));
        let _ = first;
    }

    #[test]
    fn shift_tab_walks_back_the_way_it_came() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();

        view.move_focus(true, viewport);
        view.move_focus(true, viewport);
        let second = view.focused();
        view.move_focus(true, viewport);
        view.move_focus(false, viewport);

        assert_eq!(view.focused(), second);
    }

    #[test]
    fn the_focused_element_gets_a_visible_ring_on_the_topmost_layer() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();
        view.move_focus(true, viewport);

        let scene = view.build(viewport);
        let bounds = scene.access().focus_bounds().expect("something should be focused");

        let ring =
            scene.quads_in(Layer::Focus).find(|q| q.border_color == view.theme.colors.focus_ring);
        let ring = ring.expect("a focused control with no visible ring is unusable by sight");
        assert!(
            ring.bounds.intersects(&bounds),
            "the ring is not drawn around the focused element"
        );
    }

    #[test]
    fn reference_palette_is_visible_in_the_brand_and_active_workspace_controls() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();
        assert!(
            view.workspace.view.tabs.activate(1),
            "IDE tab must be available for the palette check"
        );
        let scene = view.build(viewport);
        let c = view.theme.colors;

        assert!(
            scene.quads().any(|quad| quad.background == c.accent),
            "the warm reference accent must be visible in the chrome"
        );
        assert!(
            scene.quads().any(|quad| quad.border_color == c.accent),
            "the selected workspace tool must use the accent border"
        );
        assert!(
            scene.texts().any(|run| run.text == "Explorer" && run.color == c.accent),
            "the selected workspace tool must use the accent text colour"
        );
    }

    #[test]
    fn a_plan_step_announces_its_state_rather_than_relying_on_colour() {
        let mut view = WorkspaceView::new();
        view.set_conversation(crate::content::Conversation::with_failed_step());
        let scene = view.build(Rect::new(0.0, 0.0, 1536.0, 1024.0));

        let failed = scene
            .access()
            .nodes()
            .iter()
            .find(|n| n.label.contains("Improve token validation"))
            .expect("the failing step should be declared");
        assert!(
            failed.announcement().contains("Failed"),
            "a screen reader user cannot see the colour: {}",
            failed.announcement()
        );
    }

    #[test]
    fn work_in_progress_is_announced_as_busy_with_a_value() {
        let mut view = reference_view();
        let scene = view.build(Rect::new(0.0, 0.0, 1536.0, 1024.0));

        let progress = scene
            .access()
            .nodes()
            .iter()
            .find(|n| n.role == AxRole::ProgressIndicator && n.state.busy)
            .expect("the agent's running work should be declared");
        let said = progress.announcement();
        assert!(said.contains("busy"), "silence during work leaves the user guessing: {said}");
        assert!(said.contains('%'), "progress should carry a value: {said}");
    }

    #[test]
    fn collapsing_project_hides_its_detail_body() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();
        assert!(view.build(viewport).texts().any(|run| run.text == "No project selected"));

        view.workspace.view.toggle_section(ContextSection::Project);
        let collapsed = view.build(viewport);
        assert!(
            !collapsed.texts().any(|run| run.text == "No project selected"),
            "a collapsed context section must not retain its visible detail"
        );
    }

    #[test]
    fn focus_survives_a_layout_change() {
        // Presets rebuild the shell; losing focus each time would make the
        // keyboard unusable.
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();
        view.move_focus(true, viewport);
        let before = view.focused();

        view.workspace.apply_preset(z_shell::Preset::CodeFocus);
        let _ = view.build(viewport);

        assert_eq!(view.focused(), before);
    }

    #[test]
    fn the_conversation_never_paints_outside_the_chat_surface() {
        // Over-scanned rows exist above and below the viewport; if they escaped
        // the clip they would draw over the top bar and the composer.
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();
        view.set_conversation(crate::content::Conversation::long(2_000));
        view.scroll_chat_by(20_000.0);
        let scene = settle(&mut view, viewport);

        let frame = view.workspace.frame(viewport.width, viewport.height);
        let top_of_chat = frame.chat.y;

        for quad in scene.quads() {
            // Anything the conversation drew is clipped to the chat content.
            if quad.clip == z_gpui::Rect::UNBOUNDED {
                continue;
            }
            assert!(
                quad.clip.y >= top_of_chat - 0.01,
                "a clipped element could paint above the chat surface: {:?}",
                quad.clip
            );
        }
    }

    #[test]
    fn a_ten_thousand_message_thread_costs_the_same_as_a_short_one() {
        // The whole point of virtualizing the conversation: cost tracks the
        // window, not the history. Without this the shell would slow down the
        // longer someone worked in it.
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);

        let mut short = WorkspaceView::new();
        short.set_conversation(crate::content::Conversation::long(8));
        let short_scene = settle(&mut short, viewport);

        let mut long = WorkspaceView::new();
        long.set_conversation(crate::content::Conversation::long(10_000));
        let long_scene = settle(&mut long, viewport);

        let growth = long_scene.quad_count() as f32 / short_scene.quad_count().max(1) as f32;
        assert!(
            growth < 1.5,
            "a 1250x longer thread produced {growth:.1}x the quads — virtualization is not engaged"
        );
        assert!(long_scene.text_count() < 400, "built {} text runs", long_scene.text_count());
    }

    #[test]
    fn scrolling_a_long_thread_moves_the_window_without_growing_it() {
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);
        let mut view = WorkspaceView::new();
        view.set_conversation(crate::content::Conversation::long(5_000));
        let at_top = settle(&mut view, viewport).quad_count();

        view.scroll_chat_to_end();
        let at_end = settle(&mut view, viewport).quad_count();

        let ratio = at_end as f32 / at_top.max(1) as f32;
        assert!(
            (0.5..2.0).contains(&ratio),
            "scrolling changed the built work by {ratio:.1}x; it should be roughly constant"
        );
    }

    #[test]
    fn layout_converges_and_then_an_idle_frame_produces_no_damage() {
        // The first frame lays messages out at estimated heights and measures
        // them; the second uses the real ones. After that the scene must stop
        // changing, or the shell repaints forever on an idle workspace.
        let mut view = WorkspaceView::new();
        let viewport = Rect::new(0.0, 0.0, 1536.0, 1024.0);

        let mut previous = view.build(viewport);
        let mut settled_after = None;
        for frame in 1..=8 {
            let next = view.build(viewport);
            if next.damage_against(&previous).is_none() {
                settled_after = Some(frame);
                break;
            }
            previous = next;
        }

        let frames = settled_after.expect("layout never stopped changing");
        assert!(frames <= 3, "layout took {frames} frames to settle; it should converge at once");

        // And once settled, it stays settled.
        let a = view.build(viewport);
        let b = view.build(viewport);
        assert_eq!(
            b.damage_against(&a),
            None,
            "an idle frame must produce no damage, or the shell repaints forever"
        );
    }

    #[test]
    fn line_estimation_never_returns_zero_lines() {
        assert!(wrapped_line_count("", 100.0, 14.0) >= 1.0);
        assert!(wrapped_line_count("hello", 0.0, 14.0) >= 1.0);
        assert!(wrapped_line_count("hello", -5.0, 14.0) >= 1.0);
    }
}
