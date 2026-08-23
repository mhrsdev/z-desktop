//! Context engine core (ADR-0013, ctx-001..003): the typed candidate-item
//! stream and ONE pure allocator. Nothing here touches I/O or thread state —
//! Session items are views over StoredMessage; assembly happens per send.
//!
//! Layer names are §8.13 verbatim. The allocator implements ADR-0013 D2's
//! priority ladder as a drop order: when over budget, Ephemeral goes first,
//! then oldest Turn items, then oldest non-pinned Session history; Prefix and
//! the pinned latest-user message are never dropped. build_request wiring is
//! a later slice — this module is the core the wiring will call.

use serde::{Deserialize, Serialize};

/// Context layer, snake_case on the wire for journal/inspector export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Prefix,
    Session,
    Turn,
    Ephemeral,
}

/// One candidate unit of model context.
#[derive(Debug, Clone)]
pub struct ContextItem {
    pub layer: Layer,
    pub text: String,
    /// tokens::estimate at assembly time.
    pub est_tokens: usize,
    /// ctx-007: set by demote_if_stale when an Ephemeral body quotes a path
    /// whose on-disk contents changed after the thread last read it. Stale
    /// Ephemeral items are the FIRST thing assemble drops. Defaults false;
    /// nothing serializes ContextItem today, so no #[serde(default)] needed.
    pub stale: bool,
}

/// Pure allocation walk (ADR-0013 D2/D3, ctx-002): keep items in the given
/// order; if their total exceeds `budget`, drop stale Ephemeral first, then
/// remaining Ephemeral, then oldest Turn items, then oldest Session items —
/// never Prefix, never the last Session item (the live user message; its
/// result must survive). Returns kept items; total fits whenever prefix +
/// pin alone do.
pub fn assemble(items: Vec<ContextItem>, budget: usize) -> Vec<ContextItem> {
    let mut total: usize = items.iter().map(|i| i.est_tokens).sum();
    if total <= budget {
        return items;
    }
    // ponytail: "last USER session message" ≈ last Session item — ContextItem
    // carries no role field yet; refine at build_request integration when the
    // mapping knows roles.
    let pinned = items.iter().rposition(|i| i.layer == Layer::Session);
    let mut dropped = vec![false; items.len()];
    for (layer, only_stale) in [
        (Layer::Ephemeral, true),
        (Layer::Ephemeral, false),
        (Layer::Turn, false),
        (Layer::Session, false),
    ] {
        for (idx, item) in items.iter().enumerate() {
            if total <= budget {
                break;
            }
            if item.layer == layer && item.stale == only_stale && Some(idx) != pinned {
                dropped[idx] = true;
                total = total.saturating_sub(item.est_tokens);
            }
        }
    }
    items
        .into_iter()
        .zip(dropped)
        .filter(|(_, d)| !*d)
        .map(|(item, _)| item)
        .collect()
}

/// ctx-007 (ADR-0013 stale rule): mark Ephemeral items whose text quotes a
/// path from `stale_paths`. The caller computes that list by diffing the
/// thread's recorded fingerprints against current disk state at turn start
/// (`fingerprint::stale_reads`). Only Ephemeral tool-result bodies are
/// marked — Session/Turn narrative is not a re-read contract; edit-003's
/// stale-write refusal stays the hard enforcement for writes.
pub fn demote_if_stale(items: Vec<ContextItem>, stale_paths: &[String]) -> Vec<ContextItem> {
    if stale_paths.is_empty() {
        return items;
    }
    items
        .into_iter()
        .map(|mut item| {
            if item.layer == Layer::Ephemeral
                && stale_paths.iter().any(|p| item.text.contains(p.as_str()))
            {
                item.stale = true;
            }
            item
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(layer: Layer, text: &str, est_tokens: usize) -> ContextItem {
        ContextItem { layer, text: text.into(), est_tokens, stale: false }
    }

    #[test]
    fn empty_input_yields_empty_output() {
        assert!(assemble(Vec::new(), 100).is_empty());
    }

    #[test]
    fn under_budget_passes_everything_through_in_order() {
        let items = vec![
            item(Layer::Prefix, "sys", 10),
            item(Layer::Session, "old", 5),
            item(Layer::Turn, "now", 5),
            item(Layer::Ephemeral, "scratch", 5),
        ];
        let kept = assemble(items.clone(), 25);
        assert_eq!(kept.len(), 4);
        assert!(kept.iter().zip(items.iter()).all(|(a, b)| a.text == b.text));
    }

    #[test]
    fn prefix_is_never_dropped_even_when_it_alone_exceeds_the_budget() {
        let items = vec![
            item(Layer::Prefix, "sys", 100),
            item(Layer::Session, "old", 50),
            item(Layer::Session, "latest", 50),
        ];
        let kept = assemble(items, 40);
        assert_eq!(kept.len(), 2); // prefix + pinned latest session
        assert_eq!(kept[0].layer, Layer::Prefix);
        assert_eq!(kept[1].text, "latest");
    }

    #[test]
    fn over_budget_drops_ephemeral_before_turn_items() {
        // Budget fits everything except the ephemeral body.
        let items = vec![
            item(Layer::Prefix, "sys", 10),
            item(Layer::Session, "history", 20),
            item(Layer::Turn, "current turn", 30),
            item(Layer::Ephemeral, "tool dump", 40),
        ];
        let kept = assemble(items, 60);
        assert_eq!(kept.len(), 3);
        assert!(!kept.iter().any(|i| i.layer == Layer::Ephemeral));
        assert!(kept.iter().any(|i| i.text == "current turn"));
    }

    #[test]
    fn turn_items_drop_oldest_first_between_ephemeral_and_session() {
        let items = vec![
            item(Layer::Prefix, "sys", 5),
            item(Layer::Session, "oldest", 10),
            item(Layer::Session, "newer", 10),
            item(Layer::Session, "latest", 10), // pinned
            item(Layer::Turn, "t1", 10),
            item(Layer::Turn, "t2", 10),
        ];
        // Room for all but the two turn items.
        let kept = assemble(items, 35);
        assert_eq!(
            kept.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            vec!["sys", "oldest", "newer", "latest"]
        );
    }

    #[test]
    fn last_session_item_survives_no_matter_how_tight_the_budget() {
        let items = vec![
            item(Layer::Prefix, "sys", 5),
            item(Layer::Session, "a", 100),
            item(Layer::Session, "b", 100),
            item(Layer::Session, "latest", 100),
        ];
        let kept = assemble(items, 10);
        assert_eq!(
            kept.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            vec!["sys", "latest"]
        );
    }

    #[test]
    fn demote_if_stale_marks_only_matching_ephemeral_items() {
        let stale_path = "/tmp/changed.txt".to_string();
        let items = vec![
            item(Layer::Session, "history mentions /tmp/changed.txt", 5),
            item(Layer::Ephemeral, "fs_read /tmp/changed.txt: old body", 5),
            item(Layer::Ephemeral, "fs_read /tmp/fresh.txt: body", 5),
            item(Layer::Turn, "turn text quoting /tmp/changed.txt", 5),
        ];
        let marked = demote_if_stale(items, &[stale_path]);
        assert!(!marked[0].stale, "Session narrative is never marked");
        assert!(marked[1].stale, "matching ephemeral body is marked");
        assert!(!marked[2].stale, "unrelated ephemeral stays fresh");
        assert!(!marked[3].stale, "Turn layer is never marked");
    }

    #[test]
    fn demote_if_stale_with_no_paths_is_identity() {
        let items = vec![item(Layer::Ephemeral, "body", 5)];
        let out = demote_if_stale(items.clone(), &[]);
        assert_eq!(out.len(), 1);
        assert!(!out[0].stale);
    }

    #[test]
    fn assemble_drops_stale_ephemeral_first_when_over_budget() {
        let items = vec![
            item(Layer::Prefix, "sys", 5),
            item(Layer::Session, "oldest", 10),
            item(Layer::Session, "latest", 10), // pinned
            ContextItem {
                layer: Layer::Ephemeral,
                text: "fresh dump".into(),
                est_tokens: 10,
                stale: false,
            },
            ContextItem {
                layer: Layer::Ephemeral,
                text: "stale dump".into(),
                est_tokens: 10,
                stale: true,
            },
        ];
        // Total 45; budget fits all but one ephemeral — the STALE one goes.
        let kept = assemble(items.clone(), 35);
        assert_eq!(
            kept.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            vec!["sys", "oldest", "latest", "fresh dump"]
        );
        // Tighter budget: stale AND fresh ephemeral go before any Turn/Session.
        let kept = assemble(items, 25);
        assert_eq!(
            kept.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            vec!["sys", "oldest", "latest"]
        );
    }
}
