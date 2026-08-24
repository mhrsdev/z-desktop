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

/// prov-006: what the router resolved for one model, plus why. Pure data —
/// the runtime logs it once per turn; nothing else consumes it yet.
#[derive(Debug, Clone)]
pub struct Decision {
    pub model: String,
    pub caps: Capabilities,
    pub reason: String,
}

/// prov-006: resolve `model` against `registry` with a human-readable reason
/// ("family match '<prefix>'", or "fallback" when nothing matched).
pub fn decide(registry: &Registry, model: &str) -> Decision {
    let caps = registry.lookup(model);
    let m = model.to_lowercase();
    let reason = match registry
        .entries
        .iter()
        .filter(|(p, _)| m.starts_with(p.as_str()))
        .max_by_key(|(p, _)| p.len())
        .map(|(p, _)| p.as_str())
    {
        Some(p) => format!("family match '{p}'"),
        None => "fallback".into(),
    };
    Decision { model: model.to_string(), caps, reason }
}

/// prov-007: one slot of an evaluated fallback chain. Pure data for now —
/// runtime wiring waits on multi-provider config (prov-008/prov-024).
#[derive(Debug, Clone, PartialEq)]
pub struct FallbackEntry {
    pub model: String,
}

/// prov-007: order `available` models for failover. Exact requested match
/// first (case-insensitive, when actually offered), then models whose
/// capabilities are a superset of the requested model's (context window and
/// supports_tools), then the rest; each bucket keeps its input order. A
/// requested model that is not among `available` never appears in the chain.
pub fn fallback_chain(
    registry: &Registry,
    requested_model: &str,
    available: &[String],
) -> Vec<String> {
    let req_lower = requested_model.to_lowercase();
    let req_caps = registry.lookup(requested_model);
    let mut exact = Vec::new();
    let mut capable = Vec::new();
    let mut rest = Vec::new();
    for m in available {
        if m.to_lowercase() == req_lower {
            exact.push(m.clone());
        } else {
            let c = registry.lookup(m);
            if c.context_window >= req_caps.context_window
                && c.supports_tools >= req_caps.supports_tools
            {
                capable.push(m.clone());
            } else {
                rest.push(m.clone());
            }
        }
    }
    exact.extend(capable);
    exact.extend(rest);
    exact
}

/// prov-008: minimum capabilities a model must offer to be usable for a
/// turn. Downgrades below it are never suggested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HardRequirement {
    pub min_context: usize,
    pub needs_tools: bool,
}

/// prov-008: does `caps` satisfy `requirement`?
pub fn meets(requirement: &HardRequirement, caps: &Capabilities) -> bool {
    caps.context_window >= requirement.min_context
        && (!requirement.needs_tools || caps.supports_tools)
}

/// prov-008: `fallback_chain` restricted to models meeting `requirement`.
/// Same ordering; sub-par models (including a non-meeting requested match)
/// are excluded entirely rather than demoted to the tail.
pub fn fallback_chain_filtered(
    registry: &Registry,
    requested_model: &str,
    available: &[String],
    requirement: &HardRequirement,
) -> Vec<String> {
    let kept: Vec<String> = available
        .iter()
        .filter(|m| meets(requirement, &registry.lookup(m)))
        .cloned()
        .collect();
    fallback_chain(registry, requested_model, &kept)
}

/// prov-002: result of one provider health probe. Pure data — the runtime
/// performs the actual network probing and fills this in; nothing here does IO.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthProbe {
    pub model: String,
    pub ok: bool,
    pub latency_ms: u64,
    pub checked_at_ms: u128,
}

/// prov-002: latency bucket for a successful probe. "fast" <500ms,
/// "normal" <2000ms, otherwise "slow".
pub fn probe_verdict(latency_ms: u64) -> &'static str {
    if latency_ms < 500 {
        "fast"
    } else if latency_ms < 2_000 {
        "normal"
    } else {
        "slow"
    }
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

    #[test]
    fn decide_carries_matched_family_in_reason() {
        let d = decide(&Registry::default(), "gpt-4o");
        assert_eq!(d.model, "gpt-4o");
        assert!(d.reason.contains("family match 'gpt-'"), "{}", d.reason);
        assert_eq!(d.caps.context_window, 128_000);
        // Longest prefix wins in the reason too.
        let mut r = Registry::default();
        r.entries.push((
            "gpt-4o-mini".into(),
            Capabilities { context_window: 1, supports_tools: false },
        ));
        assert!(decide(&r, "gpt-4o-mini-x").reason.contains("'gpt-4o-mini'"));
    }

    #[test]
    fn decide_unknown_model_reports_fallback() {
        let d = decide(&Registry::default(), "mystery-model-v9");
        assert_eq!(d.reason, "fallback");
        assert_eq!(d.caps, Capabilities::default());
        assert_eq!(decide(&Registry::default(), "").reason, "fallback");
    }

    #[test]
    fn requested_model_present_goes_first() {
        let avail = vec![
            "other-a".to_string(),
            "gpt-4o".to_string(),
            "claude-x".to_string(),
        ];
        let chain = fallback_chain(&Registry::default(), "gpt-4o", &avail);
        assert_eq!(chain[0], "gpt-4o");
        // Appears exactly once even though it also fits the capable bucket.
        assert_eq!(chain.iter().filter(|m| *m == "gpt-4o").count(), 1);
    }

    #[test]
    fn tool_requiring_request_pushes_toolless_models_to_back() {
        let r = Registry::default();
        // plain-chat resolves to default caps (16k, no tools) — too small and
        // tool-less for a gpt-4o request; claude-x is a full superset.
        let avail = vec![
            "plain-chat".to_string(),
            "gpt-4o".to_string(),
            "claude-x".to_string(),
        ];
        assert_eq!(
            fallback_chain(&r, "gpt-4o", &avail),
            vec!["gpt-4o", "claude-x", "plain-chat"]
        );
    }

    #[test]
    fn unavailable_requested_model_is_excluded() {
        let r = Registry::default();
        let avail = vec!["llama-3-8b".to_string(), "plain-chat".to_string()];
        assert_eq!(
            fallback_chain(&r, "gpt-4o-not-offered", &avail),
            vec!["llama-3-8b", "plain-chat"]
        );
    }

    #[test]
    fn empty_available_yields_empty_chain() {
        assert!(fallback_chain(&Registry::default(), "gpt-4o", &[]).is_empty());
    }

    #[test]
    fn filtered_chain_excludes_sub_par_models() {
        let r = Registry::default();
        let avail = vec![
            "plain-chat".to_string(), // 16k, no tools — sub-par
            "llama-3-8b".to_string(), // 32k, tools
            "gpt-4o".to_string(),     // 128k, tools
        ];
        let req = HardRequirement { min_context: 100_000, needs_tools: true };
        assert_eq!(
            fallback_chain_filtered(&r, "gpt-4o", &avail, &req),
            vec!["gpt-4o"]
        );
        // A requested model below requirements is dropped too.
        let req = HardRequirement { min_context: 150_000, needs_tools: true };
        assert!(fallback_chain_filtered(&r, "gpt-4o", &avail, &req).is_empty());
    }

    #[test]
    fn filtered_chain_matches_unfiltered_when_nothing_is_sub_par() {
        let r = Registry::default();
        let avail = vec!["claude-x".to_string(), "gpt-4o".to_string()];
        let req = HardRequirement { min_context: 0, needs_tools: false };
        assert_eq!(
            fallback_chain_filtered(&r, "gpt-4o", &avail, &req),
            fallback_chain(&r, "gpt-4o", &avail)
        );
    }

    #[test]
    fn filtered_chain_empty_when_nothing_meets() {
        let r = Registry::default();
        let avail = vec!["plain-chat".to_string(), "llama-3-8b".to_string()];
        let req = HardRequirement { min_context: 500_000, needs_tools: true };
        assert!(fallback_chain_filtered(&r, "gpt-4o", &avail, &req).is_empty());
    }

    #[test]
    fn probe_verdict_thresholds() {
        assert_eq!(probe_verdict(0), "fast");
        assert_eq!(probe_verdict(499), "fast");
        assert_eq!(probe_verdict(500), "normal");
        assert_eq!(probe_verdict(1_999), "normal");
        assert_eq!(probe_verdict(2_000), "slow");
        assert_eq!(probe_verdict(u64::MAX), "slow");
    }

    #[test]
    fn health_probe_construction() {
        let p = HealthProbe {
            model: "gpt-4o".into(),
            ok: true,
            latency_ms: 320,
            checked_at_ms: 1_724_500_000_000,
        };
        assert_eq!(p.model, "gpt-4o");
        assert!(p.ok);
        assert_eq!(probe_verdict(p.latency_ms), "fast");
    }
}
