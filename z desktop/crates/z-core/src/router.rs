//! prov-004 (ADR-0011 D2.4): static model capability registry.
//!
//! Pure data plus one lookup function — no behavior of its own. Consumed
//! today by the runtime's tool-skip hook (prov-005 slice); later router
//! tasks (prov-006..008) grow here without changing this schema.

/// What the registry knows about one model family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub context_window: usize,
    pub supports_tools: bool,
}

impl Default for Capabilities {
    /// The "*" fallback: unknown models are treated as small-window and
    /// tool-less so we degrade to plain chat instead of failing mid-stream.
    fn default() -> Self {
        Self { context_window: 16_384, supports_tools: false }
    }
}

/// Model-name pattern (lowercase prefix) → capabilities. Longest matching
/// prefix wins; no match resolves to `Capabilities::default()`.
pub struct Registry {
    entries: Vec<(String, Capabilities)>,
}

impl Default for Registry {
    fn default() -> Self {
        let caps = |ctx: usize| Capabilities { context_window: ctx, supports_tools: true };
        Self {
            entries: vec![
                ("gpt-".into(), caps(128_000)),
                ("o3".into(), caps(200_000)),
                ("o4-mini".into(), caps(200_000)),
                ("claude-".into(), caps(200_000)),
                // Family-level granularity is deliberate for v0.1 (ADR-0011
                // D2.4 fixes the schema, not per-model accuracy).
                ("llama".into(), caps(32_768)),
            ],
        }
    }
}

impl Registry {
    /// Longest case-insensitive prefix match; `Capabilities::default()`
    /// when nothing matches.
    pub fn lookup(&self, model: &str) -> Capabilities {
        let m = model.to_lowercase();
        self.entries
            .iter()
            .filter(|(p, _)| m.starts_with(p.as_str()))
            .max_by_key(|(p, _)| p.len())
            .map(|(_, c)| *c)
            .unwrap_or_default()
    }
}

/// Lookup against the well-known seed registry (the only one in v0.1).
pub fn lookup(model: &str) -> Capabilities {
    static REGISTRY: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(Registry::default).lookup(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_families_resolve_with_expected_shapes() {
        let r = Registry::default();
        assert_eq!(r.lookup("gpt-4o").context_window, 128_000);
        assert!(r.lookup("gpt-4-turbo").supports_tools);
        assert_eq!(r.lookup("claude-sonnet-4").context_window, 200_000);
        assert!(r.lookup("o3-mini").supports_tools);
        assert!(r.lookup("o4-mini").supports_tools);
        assert!(r.lookup("llama-3-8b-instruct").supports_tools);
        assert_eq!(r.lookup("llama-3-70b").context_window, 32_768);
    }

    #[test]
    fn longest_prefix_wins_over_family() {
        let mut r = Registry::default();
        r.entries.push((
            "gpt-4o-mini".into(),
            Capabilities { context_window: 1, supports_tools: false },
        ));
        assert_eq!(r.lookup("gpt-4o-mini-2024").context_window, 1);
        assert!(!r.lookup("gpt-4o-mini-2024").supports_tools);
        // Sibling still resolves via the shorter family entry.
        assert_eq!(r.lookup("gpt-4o").context_window, 128_000);
    }

    #[test]
    fn lookup_is_case_insensitive() {
        assert_eq!(
            Registry::default().lookup("GPT-4O"),
            Registry::default().lookup("gpt-4o")
        );
        assert_eq!(
            Registry::default().lookup("CLAUDE-3-Opus").context_window,
            200_000
        );
        assert!(Registry::default().lookup("LLaMA-3").supports_tools);
    }

    #[test]
    fn unknown_models_fall_back_conservatively() {
        let fallback = Registry::default().lookup("mystery-model-v9");
        assert_eq!(fallback, Capabilities::default());
        assert!(!fallback.supports_tools);
        assert_eq!(fallback.context_window, 16_384);
        // Empty name (nothing configured yet) behaves the same way.
        assert_eq!(Registry::default().lookup(""), Capabilities::default());
    }
}
