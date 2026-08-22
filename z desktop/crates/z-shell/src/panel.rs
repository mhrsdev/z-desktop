//! Panel Registry.
//!
//! Every region of the shell — Top Bar, Sidebar, Tab Bar, Chat, Context Panel,
//! Performance Strip, Floating Tool — is a registered panel. None of them are
//! hardcoded regions. That is what lets Zero UI Studio rearrange the workspace
//! without a code change.
//!
//! A panel declares what it *can* do; [`crate::layout::LayoutState`] records what
//! the user has actually chosen. The registry is the authority on constraints,
//! so a hand-edited config cannot produce a layout the shell can't render.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Stable identifier for a panel.
///
/// Never change one of these. The user's saved layout is keyed by it; renaming
/// an id silently discards their arrangement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PanelId {
    TopBar,
    Sidebar,
    TabBar,
    Chat,
    ContextPanel,
    PerformanceStrip,
    FloatingTool,
}

impl PanelId {
    pub const ALL: &'static [PanelId] = &[
        PanelId::TopBar,
        PanelId::Sidebar,
        PanelId::TabBar,
        PanelId::Chat,
        PanelId::ContextPanel,
        PanelId::PerformanceStrip,
        PanelId::FloatingTool,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            PanelId::TopBar => "top-bar",
            PanelId::Sidebar => "sidebar",
            PanelId::TabBar => "tab-bar",
            PanelId::Chat => "chat",
            PanelId::ContextPanel => "context-panel",
            PanelId::PerformanceStrip => "performance-strip",
            PanelId::FloatingTool => "floating-tool",
        }
    }
}

/// Where a panel is allowed to sit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Dock {
    Top,
    Bottom,
    Start,
    End,
    Center,
    Floating,
}

/// Size limits, in logical pixels. Enforced in the data layer, not only in the
/// drag handler — an imported preset goes through the same clamp.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Constraints {
    pub min: f32,
    pub max: f32,
    pub default: f32,
}

impl Constraints {
    pub fn new(min: f32, default: f32, max: f32) -> Self {
        Self { min, max, default }
    }

    pub fn fixed(value: f32) -> Self {
        Self { min: value, max: value, default: value }
    }

    pub fn clamp(&self, requested: f32) -> f32 {
        z_tokens::metrics::clamp_width(requested, self.min, self.max)
    }
}

/// What the user is permitted to do to a panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub resizable: bool,
    pub movable: bool,
    pub hideable: bool,
    pub collapsible: bool,
}

impl Capabilities {
    pub const fn all() -> Self {
        Self { resizable: true, movable: true, hideable: true, collapsible: true }
    }

    pub const fn none() -> Self {
        Self { resizable: false, movable: false, hideable: false, collapsible: false }
    }
}

/// A panel's registration record.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelSpec {
    pub id: PanelId,
    /// Translatable label. Not used as an identifier.
    pub title: &'static str,
    pub allowed_docks: &'static [Dock],
    pub default_dock: Dock,
    pub constraints: Constraints,
    pub capabilities: Capabilities,
    /// Lower collapses first when horizontal space runs short. The Chat surface
    /// carries the highest priority: it is the dominant surface and never the
    /// first thing sacrificed.
    pub collapse_priority: u8,
    /// A panel the user must never be able to hide, because it carries security
    /// or state information they need in order to stay in control.
    pub essential: bool,
}

/// The set of panels the shell knows how to render.
#[derive(Debug, Clone)]
pub struct PanelRegistry {
    panels: BTreeMap<PanelId, PanelSpec>,
}

impl PanelRegistry {
    /// The Personal Agent Workspace panel set from the UI spec.
    pub fn personal_default() -> Self {
        use z_tokens::metrics::Shell;

        let specs = [
            PanelSpec {
                id: PanelId::TopBar,
                title: "Top Bar",
                allowed_docks: &[Dock::Top],
                default_dock: Dock::Top,
                constraints: Constraints::fixed(Shell::TOP_BAR),
                // Carries the mode indicator and the account control; hiding it
                // would remove the user's view of which mode they are in.
                capabilities: Capabilities::none(),
                collapse_priority: 90,
                essential: true,
            },
            PanelSpec {
                id: PanelId::Sidebar,
                title: "Sidebar",
                allowed_docks: &[Dock::Start, Dock::End],
                default_dock: Dock::Start,
                constraints: Constraints::new(
                    Shell::SIDEBAR_MIN,
                    Shell::SIDEBAR_FULL,
                    Shell::SIDEBAR_MAX,
                ),
                capabilities: Capabilities::all(),
                collapse_priority: 30,
                essential: false,
            },
            PanelSpec {
                id: PanelId::TabBar,
                title: "Tab Bar",
                allowed_docks: &[Dock::Top],
                default_dock: Dock::Top,
                constraints: Constraints::fixed(Shell::TAB_BAR),
                capabilities: Capabilities { hideable: true, ..Capabilities::none() },
                collapse_priority: 70,
                essential: false,
            },
            PanelSpec {
                id: PanelId::Chat,
                title: "Chat",
                allowed_docks: &[Dock::Center],
                default_dock: Dock::Center,
                constraints: Constraints::new(320.0, Shell::CHAT_MEASURE, f32::INFINITY),
                capabilities: Capabilities::none(),
                collapse_priority: 100,
                essential: true,
            },
            PanelSpec {
                id: PanelId::ContextPanel,
                title: "Context",
                allowed_docks: &[Dock::End, Dock::Start],
                default_dock: Dock::End,
                constraints: Constraints::new(
                    Shell::CONTEXT_MIN,
                    Shell::CONTEXT_FULL,
                    Shell::CONTEXT_MAX,
                ),
                capabilities: Capabilities::all(),
                collapse_priority: 20,
                essential: false,
            },
            PanelSpec {
                id: PanelId::PerformanceStrip,
                title: "Performance",
                allowed_docks: &[Dock::Bottom, Dock::Floating],
                default_dock: Dock::Bottom,
                constraints: Constraints::fixed(Shell::PERFORMANCE_STRIP),
                capabilities: Capabilities { resizable: false, ..Capabilities::all() },
                collapse_priority: 10,
                essential: false,
            },
            PanelSpec {
                id: PanelId::FloatingTool,
                title: "Tools",
                allowed_docks: &[Dock::Floating],
                default_dock: Dock::Floating,
                constraints: Constraints::new(48.0, 280.0, 420.0),
                capabilities: Capabilities::all(),
                collapse_priority: 40,
                essential: false,
            },
        ];

        Self { panels: specs.into_iter().map(|spec| (spec.id, spec)).collect() }
    }

    pub fn get(&self, id: PanelId) -> Option<&PanelSpec> {
        self.panels.get(&id)
    }

    pub fn iter(&self) -> impl Iterator<Item = &PanelSpec> {
        self.panels.values()
    }

    pub fn len(&self) -> usize {
        self.panels.len()
    }

    pub fn is_empty(&self) -> bool {
        self.panels.is_empty()
    }

    /// Panels ordered by which should give up space first.
    pub fn by_collapse_order(&self) -> Vec<&PanelSpec> {
        let mut ordered: Vec<&PanelSpec> = self.panels.values().collect();
        ordered.sort_by_key(|spec| (spec.collapse_priority, spec.id));
        ordered
    }
}

impl Default for PanelRegistry {
    fn default() -> Self {
        Self::personal_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_panel_is_registered() {
        let registry = PanelRegistry::personal_default();
        for id in PanelId::ALL {
            assert!(registry.get(*id).is_some(), "{id:?} is missing from the registry");
        }
        assert_eq!(registry.len(), PanelId::ALL.len());
    }

    #[test]
    fn a_panels_default_dock_is_one_it_is_allowed_to_use() {
        for spec in PanelRegistry::personal_default().iter() {
            assert!(
                spec.allowed_docks.contains(&spec.default_dock),
                "{:?} defaults to a dock it cannot legally occupy",
                spec.id
            );
        }
    }

    #[test]
    fn essential_panels_cannot_be_hidden() {
        for spec in PanelRegistry::personal_default().iter() {
            if spec.essential {
                assert!(
                    !spec.capabilities.hideable,
                    "{:?} is essential but the registry lets the user hide it",
                    spec.id
                );
            }
        }
    }

    #[test]
    fn chat_is_the_last_surface_to_give_up_space() {
        let registry = PanelRegistry::personal_default();
        let order = registry.by_collapse_order();
        let chat = order.iter().position(|spec| spec.id == PanelId::Chat).unwrap();
        assert_eq!(chat, order.len() - 1, "chat must be the dominant surface");
    }

    #[test]
    fn constraints_are_internally_consistent() {
        for spec in PanelRegistry::personal_default().iter() {
            let c = spec.constraints;
            assert!(c.min <= c.default, "{:?}: default is below min", spec.id);
            assert!(c.default <= c.max, "{:?}: default is above max", spec.id);
            assert!(c.min >= 0.0, "{:?}: negative min", spec.id);
        }
    }

    #[test]
    fn a_hand_edited_width_is_clamped_by_the_registry() {
        let registry = PanelRegistry::personal_default();
        let sidebar = registry.get(PanelId::Sidebar).unwrap();
        assert_eq!(sidebar.constraints.clamp(-500.0), sidebar.constraints.min);
        assert_eq!(sidebar.constraints.clamp(99_999.0), sidebar.constraints.max);
        assert_eq!(sidebar.constraints.clamp(f32::NAN), sidebar.constraints.min);
    }

    #[test]
    fn a_non_resizable_panel_has_a_fixed_size_band() {
        for spec in PanelRegistry::personal_default().iter() {
            if !spec.capabilities.resizable && spec.constraints.max.is_finite() {
                assert_eq!(
                    spec.constraints.min, spec.constraints.max,
                    "{:?} is not resizable, so its band should be fixed",
                    spec.id
                );
            }
        }
    }

    #[test]
    fn panel_ids_have_unique_stable_strings() {
        let mut seen = std::collections::BTreeSet::new();
        for id in PanelId::ALL {
            assert!(seen.insert(id.as_str()), "duplicate panel id string: {}", id.as_str());
        }
    }
}
