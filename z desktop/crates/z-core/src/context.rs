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
}

/// Pure allocation walk (ADR-0013 D2/D3, ctx-002): keep items in the given
/// order; if their total exceeds `budget`, drop Ephemeral first, then oldest
/// Turn items, then oldest Session items — never Prefix, never the last
/// Session item (the live user message; its result must survive). Returns
/// kept items; total fits whenever prefix + pin alone do.
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
    for layer in [Layer::Ephemeral, Layer::Turn, Layer::Session] {
        for (idx, item) in items.iter().enumerate() {
            if total <= budget {
                break;
            }
            if item.layer == layer && Some(idx) != pinned {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn item(layer: Layer, text: &str, est_tokens: usize) -> ContextItem {
        ContextItem { layer, text: text.into(), est_tokens }
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
}
