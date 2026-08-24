//! Context engine core (ADR-0013, ctx-001..003): the typed candidate-item
//! stream and ONE pure allocator. Nothing here touches I/O or thread state —
//! Session items are views over StoredMessage; assembly happens per send.
//!
//! Layer names are §8.13 verbatim. The allocator implements ADR-0013 D2's
//! priority ladder as a drop order: when over budget, Ephemeral goes first,
//! then oldest Turn items, then oldest non-pinned Session history; Prefix and
//! the pinned latest-user message are never dropped. build_request wiring is
//! a later slice — this module is the core the wiring will call.

use crate::memory::RankedMemory;
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
    /// ctx-004: pinned Session items are never drop candidates in assemble.
    pub pinned: bool,
}

/// Pure allocation walk (ADR-0013 D2/D3, ctx-002): keep items in the given
/// order; if their total exceeds `budget`, drop stale Ephemeral first, then
/// remaining Ephemeral, then oldest Turn items, then oldest Session items —
/// never Prefix, never the last Session item (the live user message; its
/// result must survive), never a ctx-004 pinned item. Returns kept items;
/// total fits whenever prefix + pin alone do.
pub fn assemble(items: Vec<ContextItem>, budget: usize) -> Vec<ContextItem> {
    let mut total: usize = items.iter().map(|i| i.est_tokens).sum();
    if total <= budget {
        return items;
    }
    // ponytail: "last USER session message" ≈ last Session item — ContextItem
    // carries no role field yet; refine at build_request integration when the
    // mapping knows roles.
    let last_session = items.iter().rposition(|i| i.layer == Layer::Session);
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
            if item.layer == layer
                && item.stale == only_stale
                && Some(idx) != last_session
                && !item.pinned
            {
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

/// ctx-004: set `pinned` on every item matching `predicate`. Pure toggle —
/// pinned Session items survive any [`assemble`] budget.
pub fn set_pinned(
    items: &mut [ContextItem],
    predicate: impl Fn(&ContextItem) -> bool,
    pinned: bool,
) {
    for item in items.iter_mut() {
        if predicate(item) {
            item.pinned = pinned;
        }
    }
}

/// mem-009 (ADR-0014): appends ranked memories as Turn-layer items
/// ("[memory] {content}") while the cumulative added estimate stays within
/// `budget_tokens`; ranked order means the first overflow ends injection.
/// Pure — callers insert before [`assemble`]; runtime wiring is a later slice.
pub fn inject_memories(
    items: &mut Vec<ContextItem>,
    memories: &[RankedMemory],
    budget_tokens: usize,
) {
    let mut added = 0usize;
    for m in memories {
        let text = format!("[memory] {}", m.content);
        let est_tokens = crate::tokens::estimate(&text);
        if added + est_tokens > budget_tokens {
            break;
        }
        added += est_tokens;
        items.push(ContextItem {
            layer: Layer::Turn,
            text,
            est_tokens,
            stale: false,
            pinned: false,
        });
    }
}

/// ctx-008: read-only inspector snapshot over a candidate-item slice.
/// `by_layer` is indexed by [`Layer`] in enum order (prefix, session,
/// turn, ephemeral) and holds (item count, summed est tokens) per layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextStats {
    pub total_items: usize,
    pub est_tokens_total: usize,
    pub by_layer: [(usize, usize); 4],
    pub stale_count: usize,
}

/// ctx-008: aggregate counts/tokens/staleness for the inspector. Pure.
pub fn stats(items: &[ContextItem]) -> ContextStats {
    let mut s = ContextStats {
        total_items: items.len(),
        est_tokens_total: 0,
        by_layer: [(0, 0); 4],
        stale_count: 0,
    };
    for i in items {
        let slot = &mut s.by_layer[i.layer as usize];
        *slot = (slot.0 + 1, slot.1 + i.est_tokens);
        s.est_tokens_total += i.est_tokens;
        s.stale_count += i.stale as usize;
    }
    s
}

fn layer_name(layer: Layer) -> &'static str {
    match layer {
        Layer::Prefix => "prefix",
        Layer::Session => "session",
        Layer::Turn => "turn",
        Layer::Ephemeral => "ephemeral",
    }
}

fn preview_line(item: &ContextItem) -> String {
    const HEAD_CHARS: usize = 40;
    let mut head: String = item.text.chars().take(HEAD_CHARS).collect();
    if item.text.chars().count() > HEAD_CHARS {
        head.push('…');
    }
    format!(
        "[{}] {} ({} tok)",
        layer_name(item.layer),
        head,
        item.est_tokens
    )
}

/// ctx-008: compact text listing for the inspector, one
/// `[layer] first-40-chars… (N tok)` line per item, never exceeding
/// `max_chars`. Items that do not fit are summarized by a final
/// `+N more items` line (whole earlier lines are dropped to make room
/// for it, so the marker is always last when present).
pub fn preview(items: &[ContextItem], max_chars: usize) -> String {
    let mut out = String::new();
    let mut shown = 0usize;
    for item in items {
        let line = preview_line(item);
        let need = out.chars().count() + usize::from(!out.is_empty()) + line.chars().count();
        if need > max_chars {
            break;
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&line);
        shown += 1;
    }
    if shown < items.len() {
        let marker = format!("+{} more items", items.len() - shown);
        while !out.is_empty() && out.chars().count() + 1 + marker.chars().count() > max_chars {
            let pos = out.rfind('\n').unwrap_or(0);
            out.truncate(pos);
        }
        if out.chars().count() + usize::from(!out.is_empty()) + marker.chars().count() <= max_chars
        {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&marker);
        } else {
            // Budget smaller than the marker itself; hard-truncate.
            out = marker.chars().take(max_chars).collect();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(layer: Layer, text: &str, est_tokens: usize) -> ContextItem {
        ContextItem {
            layer,
            text: text.into(),
            est_tokens,
            stale: false,
            pinned: false,
        }
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

    fn mem(id: &str, content: &str, score: f32) -> RankedMemory {
        RankedMemory {
            record_id: id.into(),
            content: content.into(),
            score,
        }
    }

    #[test]
    fn inject_memories_prefixes_and_respects_token_budget() {
        let mems = vec![
            mem("a", "alpha fact", 1.0),
            mem("b", "beta fact", 0.5),
            mem("c", "gamma fact", 0.1),
        ];
        let est = |s: &str| crate::tokens::estimate(&format!("[memory] {s}"));
        // Budget fits exactly two of the three.
        let budget = est("alpha fact") + est("beta fact");
        let mut items = vec![item(Layer::Prefix, "sys", 5)];
        inject_memories(&mut items, &mems, budget);
        assert_eq!(items.len(), 3, "third memory must not fit");
        for injected in &items[1..] {
            assert_eq!(injected.layer, Layer::Turn);
            assert!(injected.text.starts_with("[memory] "));
            assert_eq!(injected.est_tokens, crate::tokens::estimate(&injected.text));
        }
        let added: usize = items[1..].iter().map(|i| i.est_tokens).sum();
        assert!(added <= budget);
        // Zero budget injects nothing.
        let mut none = Vec::new();
        inject_memories(&mut none, &mems, 0);
        assert!(none.is_empty());
    }

    #[test]
    fn injected_memories_keep_assemble_output_under_budget() {
        let mems = vec![mem("m1", "the deploy target is staging", 0.9)];
        let mut items = vec![
            item(Layer::Prefix, "sys", 10),
            item(Layer::Session, "history", 20),
            item(Layer::Session, "latest", 20),
        ];
        let memory_budget = 10usize;
        inject_memories(&mut items, &mems, memory_budget);
        let total_budget = items.iter().map(|i| i.est_tokens).sum::<usize>();
        let kept = assemble(items, total_budget);
        assert!(kept.iter().map(|i| i.est_tokens).sum::<usize>() <= total_budget);
        assert!(kept.iter().any(|i| i.text.starts_with("[memory] ")));
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
                pinned: false,
            },
            ContextItem {
                layer: Layer::Ephemeral,
                text: "stale dump".into(),
                est_tokens: 10,
                stale: true,
                pinned: false,
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

    #[test]
    fn stats_counts_items_tokens_and_staleness_per_layer() {
        let items = vec![
            item(Layer::Prefix, "sys", 10),
            item(Layer::Session, "old", 5),
            item(Layer::Session, "latest", 7),
            item(Layer::Turn, "now", 3),
            ContextItem {
                layer: Layer::Ephemeral,
                text: "dump".into(),
                est_tokens: 4,
                stale: true,
                pinned: false,
            },
            ContextItem {
                layer: Layer::Ephemeral,
                text: "fresh dump".into(),
                est_tokens: 6,
                stale: false,
                pinned: false,
            },
        ];
        let s = stats(&items);
        assert_eq!(s.total_items, 6);
        assert_eq!(s.est_tokens_total, 35);
        assert_eq!(
            s.by_layer,
            [(1, 10), (2, 12), (1, 3), (2, 10)] // prefix, session, turn, ephemeral
        );
        assert_eq!(s.stale_count, 1);
    }

    #[test]
    fn stats_on_empty_input_is_all_zeros() {
        let s = stats(&[]);
        assert_eq!(
            s,
            ContextStats {
                total_items: 0,
                est_tokens_total: 0,
                by_layer: [(0, 0); 4],
                stale_count: 0
            }
        );
    }

    #[test]
    fn preview_renders_layer_head_and_tokens_per_line() {
        let long = "a".repeat(50);
        let items = vec![item(Layer::Session, &long, 12)];
        let out = preview(&items, 200);
        assert_eq!(out, format!("[session] {}… (12 tok)", "a".repeat(40)));
        // Short text gets no ellipsis.
        let out = preview(&[item(Layer::Turn, "hi", 2)], 200);
        assert_eq!(out, "[turn] hi (2 tok)");
    }

    #[test]
    fn preview_truncates_with_more_marker_within_budget() {
        let items = vec![
            item(Layer::Prefix, "sys prompt here", 10),
            item(Layer::Session, "history one", 5),
            item(Layer::Session, "history two", 5),
            item(Layer::Ephemeral, "tool dump", 40),
        ];
        // Budget fits only the first line plus the marker.
        let first_len = preview_line(&items[0]).chars().count();
        let marker = "+3 more items";
        let budget = first_len + 1 + marker.len();
        let out = preview(&items, budget);
        assert_eq!(out, format!("{}\n{}", preview_line(&items[0]), marker));
        assert!(out.chars().count() <= budget);
        // Generous budget lists everything with no marker.
        let all = preview(&items, 400);
        assert_eq!(all.lines().count(), 4);
        assert!(!all.contains("more items"));
    }

    #[test]
    fn preview_on_empty_input_is_empty() {
        assert_eq!(preview(&[], 80), "");
    }

    #[test]
    fn pinned_session_survives_budget_that_drops_unpinned_ones() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 5),
            item(Layer::Session, "keep me", 100),
            item(Layer::Session, "oldest", 100),
            item(Layer::Session, "latest", 100),
        ];
        set_pinned(&mut items, |i| i.text == "keep me", true);
        // Budget fits prefix + only one session; the unpinned oldest goes.
        let kept = assemble(items, 110);
        let texts: Vec<_> = kept.iter().map(|i| i.text.as_str()).collect();
        assert_eq!(texts, vec!["sys", "keep me", "latest"]);
        assert!(!texts.contains(&"oldest"));
    }

    #[test]
    fn unpinning_restores_droppability() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 5),
            item(Layer::Session, "pinned then freed", 100),
            item(Layer::Session, "latest", 100),
        ];
        set_pinned(
            &mut items,
            |i| i.layer == Layer::Session && i.text != "latest",
            true,
        );
        assert!(items[1].pinned);
        set_pinned(
            &mut items,
            |i| i.layer == Layer::Session && i.text != "latest",
            false,
        );
        assert!(!items[1].pinned);
        let kept = assemble(items, 10);
        assert_eq!(
            kept.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            vec!["sys", "latest"]
        );
    }

    #[test]
    fn default_items_are_unpinned() {
        let it = item(Layer::Session, "plain", 5);
        assert!(!it.pinned);
    }
}
