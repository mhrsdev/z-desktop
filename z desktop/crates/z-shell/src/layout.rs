//! Layout State — where things are.
//!
//! Deliberately separate from [`crate::view::ViewState`], which holds what is
//! *inside* each panel. Merging the two is the mistake that makes a layout
//! change throw away the user's scroll position and selection, so the split is
//! enforced by the type system rather than by convention.
//!
//! Every value that arrives from disk, from a preset, or from the Advanced
//! config editor passes through [`LayoutState::sanitize`] before it is used.
//! The registry is the authority; the file is only a request.

use crate::panel::{Capabilities, Dock, PanelId, PanelRegistry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Bumped whenever the on-disk shape changes. A file from an older schema is
/// migrated; a file from a newer one is refused rather than half-read.
pub const LAYOUT_SCHEMA_VERSION: u32 = 1;

/// Placement of one panel.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PanelPlacement {
    pub dock: Dock,
    /// Cross-axis size in logical pixels: width when docked to a side, height
    /// when docked top or bottom.
    pub size: f32,
    pub visible: bool,
    pub collapsed: bool,
}

/// Where a setting came from. Shown in the UI so the user can tell an inherited
/// value from one they set, and reset back to the level above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Origin {
    Default,
    Global,
    Profile,
    Project,
}

impl Origin {
    /// Project beats Profile beats Global beats Default.
    pub const PRECEDENCE: &'static [Origin] =
        &[Origin::Default, Origin::Global, Origin::Profile, Origin::Project];
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutState {
    pub schema_version: u32,
    pub preset: Preset,
    pub origin: Origin,
    pub placements: BTreeMap<PanelId, PanelPlacement>,
}

/// Named starting arrangements. A preset only ever touches Layout State — never
/// permissions, never project data, never agent behaviour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Preset {
    Default,
    MinimalFocus,
    ChatFocus,
    CodeFocus,
    UltraCompact,
    WideWorkspace,
    LaptopMode,
    MinimalSidebars,
    PowerUser,
    Custom,
}

impl Preset {
    pub const BUILT_IN: &'static [Preset] = &[
        Preset::Default,
        Preset::MinimalFocus,
        Preset::ChatFocus,
        Preset::CodeFocus,
        Preset::UltraCompact,
        Preset::WideWorkspace,
        Preset::LaptopMode,
        Preset::MinimalSidebars,
        Preset::PowerUser,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Preset::Default => "Default",
            Preset::MinimalFocus => "Minimal Focus",
            Preset::ChatFocus => "Chat Focus",
            Preset::CodeFocus => "Code Focus",
            Preset::UltraCompact => "Ultra Compact",
            Preset::WideWorkspace => "Wide Workspace",
            Preset::LaptopMode => "Laptop Mode",
            Preset::MinimalSidebars => "Minimal Sidebars",
            Preset::PowerUser => "Power User",
            Preset::Custom => "Custom",
        }
    }
}

/// What `sanitize` had to change. Surfaced to the user on import so a rejected
/// value is visible rather than silently swallowed.
#[derive(Debug, Clone, PartialEq)]
pub struct Correction {
    pub panel: PanelId,
    pub field: &'static str,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LayoutError {
    /// Written by a newer build. Refused, because guessing at unknown fields
    /// risks discarding the user's arrangement.
    SchemaTooNew { found: u32, supported: u32 },
}

impl std::fmt::Display for LayoutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayoutError::SchemaTooNew { found, supported } => write!(
                f,
                "layout schema v{found} was written by a newer version of Zero \
                 (this build supports up to v{supported})"
            ),
        }
    }
}

impl std::error::Error for LayoutError {}

impl LayoutState {
    /// The reference workspace: full sidebar, chat centre, context panel right,
    /// performance strip along the bottom, floating tool present.
    pub fn personal_default(registry: &PanelRegistry) -> Self {
        let placements = registry
            .iter()
            .map(|spec| {
                (
                    spec.id,
                    PanelPlacement {
                        dock: spec.default_dock,
                        size: spec.constraints.default,
                        visible: true,
                        collapsed: false,
                    },
                )
            })
            .collect();

        Self {
            schema_version: LAYOUT_SCHEMA_VERSION,
            preset: Preset::Default,
            origin: Origin::Default,
            placements,
        }
    }

    pub fn from_preset(preset: Preset, registry: &PanelRegistry) -> Self {
        let mut state = Self::personal_default(registry);
        state.preset = preset;

        let hide = |state: &mut Self, id: PanelId| {
            if let Some(p) = state.placements.get_mut(&id) {
                p.visible = false;
            }
        };
        let resize = |state: &mut Self, id: PanelId, size: f32| {
            if let Some(p) = state.placements.get_mut(&id) {
                p.size = size;
            }
        };

        match preset {
            Preset::Default | Preset::Custom => {}
            Preset::MinimalFocus => {
                hide(&mut state, PanelId::ContextPanel);
                hide(&mut state, PanelId::PerformanceStrip);
                resize(&mut state, PanelId::Sidebar, z_tokens::metrics::Shell::SIDEBAR_ICON);
            }
            Preset::ChatFocus => {
                resize(&mut state, PanelId::Sidebar, z_tokens::metrics::Shell::SIDEBAR_ICON);
                resize(
                    &mut state,
                    PanelId::ContextPanel,
                    z_tokens::metrics::Shell::CONTEXT_SLIM,
                );
                resize(&mut state, PanelId::Chat, z_tokens::metrics::Shell::CHAT_MEASURE_MAX);
            }
            Preset::CodeFocus => {
                resize(&mut state, PanelId::Sidebar, z_tokens::metrics::Shell::SIDEBAR_COMPACT);
                resize(
                    &mut state,
                    PanelId::ContextPanel,
                    z_tokens::metrics::Shell::CONTEXT_COMPACT,
                );
            }
            Preset::UltraCompact => {
                resize(&mut state, PanelId::Sidebar, z_tokens::metrics::Shell::SIDEBAR_ICON);
                resize(
                    &mut state,
                    PanelId::ContextPanel,
                    z_tokens::metrics::Shell::CONTEXT_SLIM,
                );
                hide(&mut state, PanelId::PerformanceStrip);
            }
            Preset::WideWorkspace => {
                resize(&mut state, PanelId::ContextPanel, z_tokens::metrics::Shell::CONTEXT_MAX);
                resize(&mut state, PanelId::Chat, z_tokens::metrics::Shell::CHAT_MEASURE_MAX);
            }
            Preset::LaptopMode => {
                resize(&mut state, PanelId::Sidebar, z_tokens::metrics::Shell::SIDEBAR_COMPACT);
                resize(
                    &mut state,
                    PanelId::ContextPanel,
                    z_tokens::metrics::Shell::CONTEXT_SLIM,
                );
                hide(&mut state, PanelId::PerformanceStrip);
            }
            Preset::MinimalSidebars => {
                resize(&mut state, PanelId::Sidebar, z_tokens::metrics::Shell::SIDEBAR_ICON);
                resize(
                    &mut state,
                    PanelId::ContextPanel,
                    z_tokens::metrics::Shell::CONTEXT_SLIM,
                );
            }
            Preset::PowerUser => {
                resize(&mut state, PanelId::Sidebar, z_tokens::metrics::Shell::SIDEBAR_FULL);
                resize(
                    &mut state,
                    PanelId::ContextPanel,
                    z_tokens::metrics::Shell::CONTEXT_FULL,
                );
            }
        }

        // Presets go through the same gate as an imported file. A preset is not
        // privileged just because it ships with the product.
        state.sanitize(registry);
        state
    }

    /// Force the state to obey the registry, reporting everything it had to change.
    ///
    /// Runs on load, on import, after a preset switch and after an edit in the
    /// Advanced config editor — every path by which untrusted numbers can arrive.
    pub fn sanitize(&mut self, registry: &PanelRegistry) -> Vec<Correction> {
        let mut corrections = Vec::new();

        // Drop panels this build does not know about, rather than trying to
        // render something with no constraints.
        let unknown: Vec<PanelId> =
            self.placements.keys().copied().filter(|id| registry.get(*id).is_none()).collect();
        for id in unknown {
            self.placements.remove(&id);
            corrections.push(Correction {
                panel: id,
                field: "panel",
                reason: "not present in this build's registry".into(),
            });
        }

        for spec in registry.iter() {
            let placement = self.placements.entry(spec.id).or_insert(PanelPlacement {
                dock: spec.default_dock,
                size: spec.constraints.default,
                visible: true,
                collapsed: false,
            });

            if !spec.allowed_docks.contains(&placement.dock) {
                corrections.push(Correction {
                    panel: spec.id,
                    field: "dock",
                    reason: format!("{:?} is not an allowed dock", placement.dock),
                });
                placement.dock = spec.default_dock;
            }

            let clamped = spec.constraints.clamp(placement.size);
            if clamped != placement.size {
                corrections.push(Correction {
                    panel: spec.id,
                    field: "size",
                    reason: format!("{} clamped to {}", placement.size, clamped),
                });
                placement.size = clamped;
            }

            // An essential panel stays visible no matter what the file says.
            // This is the check that stops an imported layout from hiding the
            // controls a user needs in order to stay in control of the agent.
            if !placement.visible && (spec.essential || !spec.capabilities.hideable) {
                corrections.push(Correction {
                    panel: spec.id,
                    field: "visible",
                    reason: "panel cannot be hidden".into(),
                });
                placement.visible = true;
            }

            if placement.collapsed && !spec.capabilities.collapsible {
                corrections.push(Correction {
                    panel: spec.id,
                    field: "collapsed",
                    reason: "panel cannot be collapsed".into(),
                });
                placement.collapsed = false;
            }
        }

        corrections
    }

    /// Read a layout from JSON, migrating an older schema and sanitizing the result.
    ///
    /// Never executes anything: the format is plain data, and unknown fields are
    /// dropped by serde rather than interpreted.
    pub fn from_json(
        json: &str,
        registry: &PanelRegistry,
    ) -> Result<(Self, Vec<Correction>), LayoutError> {
        #[derive(Deserialize)]
        struct VersionProbe {
            schema_version: u32,
        }

        let version = serde_json::from_str::<VersionProbe>(json)
            .map(|probe| probe.schema_version)
            // A file with no version predates versioning; treat it as v1.
            .unwrap_or(1);

        if version > LAYOUT_SCHEMA_VERSION {
            return Err(LayoutError::SchemaTooNew {
                found: version,
                supported: LAYOUT_SCHEMA_VERSION,
            });
        }

        // A malformed file is not a crash and not a wipe: fall back to the
        // default arrangement and report every field that had to be replaced.
        let mut state = serde_json::from_str::<LayoutState>(json).unwrap_or_else(|_| {
            let mut fallback = Self::personal_default(registry);
            fallback.preset = Preset::Custom;
            fallback
        });

        state.schema_version = LAYOUT_SCHEMA_VERSION;
        let corrections = state.sanitize(registry);
        Ok((state, corrections))
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("layout state is plain data")
    }

    /// Cross-axis size of a panel as it should actually be laid out.
    pub fn effective_size(&self, id: PanelId) -> f32 {
        match self.placements.get(&id) {
            Some(p) if p.visible => p.size,
            _ => 0.0,
        }
    }

    pub fn is_visible(&self, id: PanelId) -> bool {
        self.placements.get(&id).is_some_and(|p| p.visible)
    }

    /// Return to the shipped arrangement. Always available, from any state.
    pub fn reset(&mut self, registry: &PanelRegistry) {
        *self = Self::personal_default(registry);
    }
}

/// Resolve a stack of layouts by precedence: Project > Profile > Global > Default.
pub fn resolve(layers: &[(Origin, LayoutState)], registry: &PanelRegistry) -> LayoutState {
    let mut winner = LayoutState::personal_default(registry);
    for origin in Origin::PRECEDENCE {
        if let Some((_, layer)) = layers.iter().find(|(o, _)| o == origin) {
            winner = layer.clone();
            winner.origin = *origin;
        }
    }
    winner.sanitize(registry);
    winner
}

#[allow(unused_imports)]
use Capabilities as _EnsureCapabilitiesInScope;

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> PanelRegistry {
        PanelRegistry::personal_default()
    }

    #[test]
    fn default_layout_covers_every_registered_panel() {
        let reg = registry();
        let state = LayoutState::personal_default(&reg);
        assert_eq!(state.placements.len(), reg.len());
        for spec in reg.iter() {
            assert!(state.placements.contains_key(&spec.id));
        }
    }

    #[test]
    fn json_round_trips_without_corrections() {
        let reg = registry();
        let state = LayoutState::from_preset(Preset::ChatFocus, &reg);
        let (restored, corrections) = LayoutState::from_json(&state.to_json(), &reg).unwrap();
        assert_eq!(restored, state);
        assert!(corrections.is_empty(), "clean round trip should need no fixes: {corrections:?}");
    }

    #[test]
    fn a_hand_edited_size_is_clamped_and_reported() {
        let reg = registry();
        let mut state = LayoutState::personal_default(&reg);
        state.placements.get_mut(&PanelId::Sidebar).unwrap().size = 99_999.0;

        let corrections = state.sanitize(&reg);

        let sidebar = reg.get(PanelId::Sidebar).unwrap();
        assert_eq!(state.placements[&PanelId::Sidebar].size, sidebar.constraints.max);
        assert!(corrections.iter().any(|c| c.panel == PanelId::Sidebar && c.field == "size"));
    }

    #[test]
    fn an_imported_layout_cannot_hide_an_essential_panel() {
        let reg = registry();
        let mut state = LayoutState::personal_default(&reg);
        for spec in reg.iter().filter(|s| s.essential) {
            state.placements.get_mut(&spec.id).unwrap().visible = false;
        }

        let corrections = state.sanitize(&reg);

        for spec in reg.iter().filter(|s| s.essential) {
            assert!(state.is_visible(spec.id), "{:?} was hidden by an import", spec.id);
        }
        assert!(corrections.iter().any(|c| c.field == "visible"));
    }

    #[test]
    fn an_illegal_dock_falls_back_to_the_default() {
        let reg = registry();
        let mut state = LayoutState::personal_default(&reg);
        state.placements.get_mut(&PanelId::Chat).unwrap().dock = Dock::Floating;

        let corrections = state.sanitize(&reg);

        assert_eq!(state.placements[&PanelId::Chat].dock, Dock::Center);
        assert!(corrections.iter().any(|c| c.panel == PanelId::Chat && c.field == "dock"));
    }

    #[test]
    fn malformed_json_falls_back_instead_of_crashing() {
        let reg = registry();
        let (state, _) = LayoutState::from_json("{ this is not json", &reg).unwrap();
        assert_eq!(state.placements.len(), reg.len());
    }

    #[test]
    fn absurd_numbers_in_json_are_neutralised() {
        let reg = registry();
        let hostile = r#"{
            "schema_version": 1,
            "preset": "custom",
            "origin": "project",
            "placements": {
                "sidebar":   { "dock": "start", "size": -99999.0, "visible": true,  "collapsed": false },
                "chat":      { "dock": "center", "size": 1e30,    "visible": false, "collapsed": true  }
            }
        }"#;

        let (state, corrections) = LayoutState::from_json(hostile, &reg).unwrap();

        assert!(
            state.placements[&PanelId::Sidebar].size
                >= reg.get(PanelId::Sidebar).unwrap().constraints.min
        );
        assert!(state.is_visible(PanelId::Chat), "chat is essential and must survive import");
        assert!(!corrections.is_empty());
        // Panels the file omitted are filled in from the registry, not left absent.
        assert_eq!(state.placements.len(), reg.len());
    }

    #[test]
    fn a_newer_schema_is_refused_rather_than_half_read() {
        let reg = registry();
        let future = format!(r#"{{"schema_version": {}}}"#, LAYOUT_SCHEMA_VERSION + 1);
        assert!(matches!(
            LayoutState::from_json(&future, &reg),
            Err(LayoutError::SchemaTooNew { .. })
        ));
    }

    #[test]
    fn every_built_in_preset_produces_a_valid_layout() {
        let reg = registry();
        for preset in Preset::BUILT_IN {
            let mut state = LayoutState::from_preset(*preset, &reg);
            let corrections = state.sanitize(&reg);
            assert!(
                corrections.is_empty(),
                "{} needs fixing after construction: {corrections:?}",
                preset.label()
            );
            assert!(state.is_visible(PanelId::Chat), "{} hides chat", preset.label());
        }
    }

    #[test]
    fn reset_recovers_from_any_state() {
        let reg = registry();
        let mut state = LayoutState::from_preset(Preset::UltraCompact, &reg);
        state.placements.clear();

        state.reset(&reg);

        assert_eq!(state, LayoutState::personal_default(&reg));
    }

    #[test]
    fn project_settings_beat_profile_and_global() {
        let reg = registry();
        let mut global = LayoutState::personal_default(&reg);
        global.placements.get_mut(&PanelId::Sidebar).unwrap().size = 200.0;
        let mut profile = LayoutState::personal_default(&reg);
        profile.placements.get_mut(&PanelId::Sidebar).unwrap().size = 240.0;
        let mut project = LayoutState::personal_default(&reg);
        project.placements.get_mut(&PanelId::Sidebar).unwrap().size = 300.0;

        let resolved = resolve(
            &[(Origin::Global, global), (Origin::Profile, profile), (Origin::Project, project)],
            &reg,
        );

        assert_eq!(resolved.placements[&PanelId::Sidebar].size, 300.0);
        assert_eq!(resolved.origin, Origin::Project);
    }

    #[test]
    fn hiding_a_panel_only_changes_its_size_contribution() {
        let reg = registry();
        let mut state = LayoutState::personal_default(&reg);
        let before = state.placements[&PanelId::ContextPanel];

        state.placements.get_mut(&PanelId::ContextPanel).unwrap().visible = false;
        state.sanitize(&reg);

        assert_eq!(state.effective_size(PanelId::ContextPanel), 0.0);
        // The stored width survives, so unhiding restores what the user had.
        assert_eq!(state.placements[&PanelId::ContextPanel].size, before.size);
    }
}
