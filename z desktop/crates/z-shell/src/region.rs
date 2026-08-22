//! Resolving panel placements into concrete rectangles.
//!
//! Pure geometry over [`LayoutState`] — no GPU, no window handle — so the whole
//! responsive behaviour is testable without opening anything.
//!
//! The one rule worth stating plainly: when the window is too narrow, panels
//! **collapse by priority**; they are never clipped. Clipping produces a shell
//! where a control exists but cannot be reached, which is worse than a shell
//! where it is honestly absent.

use crate::layout::LayoutState;
use crate::panel::{PanelId, PanelRegistry};
use z_tokens::metrics::Shell;

/// Axis-aligned rectangle in logical pixels, origin at the window's top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub const ZERO: Rect = Rect { x: 0.0, y: 0.0, width: 0.0, height: 0.0 };

    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0.0 || self.height <= 0.0
    }

    pub fn right(&self) -> f32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.height
    }

    pub fn center_x(&self) -> f32 {
        self.x + self.width / 2.0
    }

    pub fn center_y(&self) -> f32 {
        self.y + self.height / 2.0
    }

    /// Whether two rectangles share any interior area.
    pub fn overlaps(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }

    pub fn inset(&self, amount: f32) -> Rect {
        Rect {
            x: self.x + amount,
            y: self.y + amount,
            width: (self.width - amount * 2.0).max(0.0),
            height: (self.height - amount * 2.0).max(0.0),
        }
    }
}

/// Resolved rectangles for one frame.
///
/// Two rectangles overlap on purpose and are excluded from the tiling checks:
///
/// - `tab_bar` is a **zone inside** `top_bar`, not a row of its own. The
///   reference workspace puts the brand mark, the tab strip and the account
///   controls on one 56px band, aligned with the columns beneath them. A second
///   header row would cost another 44px of vertical space in a product whose
///   whole point is the surface below.
/// - `floating_tool` is an overlay above `chat`.
#[derive(Debug, Clone, PartialEq)]
pub struct ShellFrame {
    pub window: Rect,
    pub top_bar: Rect,
    pub sidebar: Rect,
    /// The tab strip's zone within `top_bar`, horizontally aligned with `chat`.
    pub tab_bar: Rect,
    pub chat: Rect,
    pub context_panel: Rect,
    pub performance_strip: Rect,
    pub floating_tool: Rect,
    /// Panels dropped because the window was too narrow to hold them. Surfaced
    /// so the UI can tell the user what was folded away instead of leaving them
    /// to wonder where a panel went.
    pub collapsed: Vec<PanelId>,
}

impl ShellFrame {
    /// Rectangles that must tile without overlapping. Excludes the two overlays
    /// documented on the struct.
    ///
    /// Public because it is the contract a caller checks against: these are the
    /// regions guaranteed not to sit on top of each other.
    pub fn docked(&self) -> [(PanelId, Rect); 5] {
        [
            (PanelId::TopBar, self.top_bar),
            (PanelId::Sidebar, self.sidebar),
            (PanelId::Chat, self.chat),
            (PanelId::ContextPanel, self.context_panel),
            (PanelId::PerformanceStrip, self.performance_strip),
        ]
    }
}

/// Lay the shell out.
///
/// Vertical bands are claimed first (top bar, performance strip), then the side
/// rails inside what remains, then the tab bar, and whatever is left belongs to
/// chat. Chat is resolved last precisely because it is the dominant surface: it
/// absorbs the slack rather than competing for a fixed share.
pub fn solve(
    registry: &PanelRegistry,
    layout: &LayoutState,
    width: f32,
    height: f32,
) -> ShellFrame {
    let width = width.max(Shell::WINDOW_MIN_WIDTH);
    let height = height.max(Shell::WINDOW_MIN_HEIGHT);
    let window = Rect::new(0.0, 0.0, width, height);
    let mut collapsed = Vec::new();

    // Horizontal bands.
    let top_bar_h = layout.effective_size(PanelId::TopBar);
    let strip_h = layout.effective_size(PanelId::PerformanceStrip);
    let top_bar = Rect::new(0.0, 0.0, width, top_bar_h);
    let performance_strip = Rect::new(0.0, height - strip_h, width, strip_h);

    let body_y = top_bar.bottom();
    let body_h = (performance_strip.y - body_y).max(0.0);

    // Side rails, narrowed then dropped by priority until chat has room to breathe.
    const RAILS: [PanelId; 2] = [PanelId::Sidebar, PanelId::ContextPanel];
    let rail_min = [Shell::SIDEBAR_ICON, Shell::CONTEXT_SLIM];
    let mut rail_w =
        [layout.effective_size(PanelId::Sidebar), layout.effective_size(PanelId::ContextPanel)];

    let chat_min = registry.get(PanelId::Chat).map(|spec| spec.constraints.min).unwrap_or(320.0);
    let chat_fits = |rails: &[f32; 2]| width - rails[0] - rails[1] >= chat_min;

    // Ordered by collapse_priority: whichever rail is cheapest to lose goes first.
    let order: Vec<PanelId> = registry
        .by_collapse_order()
        .into_iter()
        .map(|spec| spec.id)
        .filter(|id| RAILS.contains(id))
        .collect();

    for id in order {
        if chat_fits(&rail_w) {
            break;
        }
        let Some(i) = RAILS.iter().position(|rail| *rail == id) else { continue };
        if rail_w[i] <= 0.0 {
            continue;
        }
        // Try the narrow mode before removing the panel entirely.
        if rail_w[i] > rail_min[i] {
            rail_w[i] = rail_min[i];
            if chat_fits(&rail_w) {
                break;
            }
        }
        rail_w[i] = 0.0;
        collapsed.push(id);
    }

    let [sidebar_w, context_w] = rail_w;

    let sidebar = Rect::new(0.0, body_y, sidebar_w, body_h);
    let context_panel = Rect::new(width - context_w, body_y, context_w, body_h);

    let centre_x = sidebar.right();
    let centre_w = (context_panel.x - centre_x).max(0.0);

    // The tab strip is a zone within the top band, aligned with the centre
    // column beneath it — see the note on `ShellFrame`.
    let tab_bar = if layout.is_visible(PanelId::TabBar) {
        Rect::new(centre_x, top_bar.y, centre_w, top_bar.height)
    } else {
        Rect::ZERO
    };

    let chat = Rect::new(centre_x, body_y, centre_w, body_h);

    // Overlay, anchored near the lower-left of the centre surface.
    let floating_tool = if layout.is_visible(PanelId::FloatingTool) {
        let size = layout.effective_size(PanelId::FloatingTool);
        let margin = z_tokens::metrics::Spacing::S6;
        let side = size.min(chat.width).max(0.0);
        Rect::new(
            chat.x + margin,
            (chat.bottom() - margin - side).max(chat.y),
            side.min((chat.width - margin * 2.0).max(0.0)),
            side.min(chat.height),
        )
    } else {
        Rect::ZERO
    };

    ShellFrame {
        window,
        top_bar,
        sidebar,
        tab_bar,
        chat,
        context_panel,
        performance_strip,
        floating_tool,
        collapsed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Preset;

    fn frame(width: f32, height: f32) -> ShellFrame {
        let registry = PanelRegistry::personal_default();
        let layout = LayoutState::personal_default(&registry);
        solve(&registry, &layout, width, height)
    }

    #[test]
    fn docked_panels_tile_without_overlapping() {
        let frame = frame(1536.0, 1024.0);
        let docked = frame.docked();
        for (i, (id_a, a)) in docked.iter().enumerate() {
            for (id_b, b) in docked.iter().skip(i + 1) {
                if a.is_empty() || b.is_empty() {
                    continue;
                }
                assert!(!a.overlaps(b), "{id_a:?} overlaps {id_b:?}: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn every_panel_stays_inside_the_window() {
        let frame = frame(1536.0, 1024.0);
        for (id, rect) in frame.docked() {
            if rect.is_empty() {
                continue;
            }
            assert!(rect.x >= 0.0 && rect.y >= 0.0, "{id:?} starts outside the window");
            assert!(rect.right() <= frame.window.width + 0.01, "{id:?} runs past the right edge");
            assert!(rect.bottom() <= frame.window.height + 0.01, "{id:?} runs past the bottom");
        }
    }

    #[test]
    fn the_reference_layout_matches_the_spec_dimensions() {
        let frame = frame(1536.0, 1024.0);
        assert_eq!(frame.top_bar.height, Shell::TOP_BAR);
        assert_eq!(frame.sidebar.width, Shell::SIDEBAR_FULL);
        assert_eq!(frame.context_panel.width, Shell::CONTEXT_FULL);
        assert_eq!(frame.performance_strip.height, Shell::PERFORMANCE_STRIP);
    }

    #[test]
    fn the_tab_strip_sits_inside_the_top_band_above_chat() {
        let frame = frame(1536.0, 1024.0);
        assert_eq!(frame.tab_bar.y, frame.top_bar.y);
        assert_eq!(frame.tab_bar.height, frame.top_bar.height);
        // Horizontally aligned with the surface whose tabs it carries.
        assert_eq!(frame.tab_bar.x, frame.chat.x);
        assert_eq!(frame.tab_bar.width, frame.chat.width);
        assert!(frame.tab_bar.bottom() <= frame.chat.y);
    }

    #[test]
    fn the_header_costs_only_one_band_of_vertical_space() {
        let frame = frame(1536.0, 1024.0);
        assert_eq!(
            frame.chat.y,
            Shell::TOP_BAR,
            "chat should start directly below the single header band"
        );
    }

    #[test]
    fn chat_absorbs_the_remaining_width() {
        let frame = frame(1536.0, 1024.0);
        let expected = 1536.0 - Shell::SIDEBAR_FULL - Shell::CONTEXT_FULL;
        assert!((frame.chat.width - expected).abs() < 0.01, "got {}", frame.chat.width);
    }

    #[test]
    fn chat_is_the_largest_region() {
        let frame = frame(1536.0, 1024.0);
        let area = |r: Rect| r.width * r.height;
        for (id, rect) in frame.docked() {
            if id == PanelId::Chat {
                continue;
            }
            assert!(area(frame.chat) > area(rect), "chat must dominate, but {id:?} is bigger");
        }
    }

    #[test]
    fn a_narrow_window_collapses_rails_instead_of_clipping_chat() {
        let frame = frame(760.0, 700.0);
        let chat_min =
            PanelRegistry::personal_default().get(PanelId::Chat).unwrap().constraints.min;
        assert!(frame.chat.width >= chat_min, "chat got squeezed below its minimum");
        assert!(frame.chat.right() <= frame.window.width + 0.01);
    }

    #[test]
    fn collapsing_is_reported_rather_than_silent() {
        let frame = frame(Shell::WINDOW_MIN_WIDTH, 600.0);
        let rails_gone = frame.sidebar.is_empty() || frame.context_panel.is_empty();
        if rails_gone {
            assert!(!frame.collapsed.is_empty(), "a panel vanished with no record of it");
        }
    }

    #[test]
    fn a_window_below_the_minimum_is_treated_as_the_minimum() {
        let frame = frame(100.0, 100.0);
        assert_eq!(frame.window.width, Shell::WINDOW_MIN_WIDTH);
        assert_eq!(frame.window.height, Shell::WINDOW_MIN_HEIGHT);
        assert!(!frame.chat.is_empty(), "chat must survive even at the smallest window");
    }

    #[test]
    fn hiding_the_context_panel_gives_its_width_to_chat() {
        let registry = PanelRegistry::personal_default();
        let wide = solve(&registry, &LayoutState::personal_default(&registry), 1536.0, 1024.0);

        let mut hidden = LayoutState::personal_default(&registry);
        hidden.placements.get_mut(&PanelId::ContextPanel).unwrap().visible = false;
        let narrow = solve(&registry, &hidden, 1536.0, 1024.0);

        assert!(narrow.chat.width > wide.chat.width);
        assert!(narrow.context_panel.is_empty());
        assert!((narrow.chat.width - (wide.chat.width + Shell::CONTEXT_FULL)).abs() < 0.01);
    }

    #[test]
    fn the_floating_tool_overlays_chat_without_displacing_it() {
        let registry = PanelRegistry::personal_default();
        let layout = LayoutState::personal_default(&registry);
        let with_tool = solve(&registry, &layout, 1536.0, 1024.0);

        let mut without = layout.clone();
        without.placements.get_mut(&PanelId::FloatingTool).unwrap().visible = false;
        let plain = solve(&registry, &without, 1536.0, 1024.0);

        assert_eq!(with_tool.chat, plain.chat, "an overlay must not resize the surface below it");
        assert!(with_tool.floating_tool.overlaps(&with_tool.chat));
    }

    #[test]
    fn chat_focus_widens_the_centre_surface() {
        let registry = PanelRegistry::personal_default();
        let default = solve(&registry, &LayoutState::personal_default(&registry), 1536.0, 1024.0);
        let focus = solve(
            &registry,
            &LayoutState::from_preset(Preset::ChatFocus, &registry),
            1536.0,
            1024.0,
        );
        assert!(focus.chat.width > default.chat.width, "Chat Focus should give chat more room");
    }

    #[test]
    fn minimal_focus_drops_the_context_panel_and_strip() {
        let registry = PanelRegistry::personal_default();
        let frame = solve(
            &registry,
            &LayoutState::from_preset(Preset::MinimalFocus, &registry),
            1536.0,
            1024.0,
        );
        assert!(frame.context_panel.is_empty());
        assert!(frame.performance_strip.is_empty());
        assert!(!frame.chat.is_empty());
    }

    #[test]
    fn layout_is_deterministic() {
        let a = frame(1440.0, 900.0);
        let b = frame(1440.0, 900.0);
        assert_eq!(a, b, "the same inputs must produce the same frame");
    }

    #[test]
    fn wide_and_tall_windows_do_not_produce_negative_geometry() {
        for (w, h) in [(3840.0, 2160.0), (5120.0, 1440.0), (800.0, 2400.0)] {
            let frame = frame(w, h);
            for (id, rect) in frame.docked() {
                assert!(rect.width >= 0.0 && rect.height >= 0.0, "{id:?} has negative geometry");
            }
        }
    }
}
