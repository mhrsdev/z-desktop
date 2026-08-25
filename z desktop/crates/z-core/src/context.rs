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
use std::path::Path;

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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextItem {
    pub layer: Layer,
    pub text: String,
    /// tokens::estimate at assembly time.
    pub est_tokens: usize,
    /// ctx-007: set by demote_if_stale when an Ephemeral body quotes a path
    /// whose on-disk contents changed after the thread last read it. Stale
    /// Ephemeral items are the FIRST thing assemble drops. Defaults false.
    #[serde(default)]
    pub stale: bool,
    /// ctx-004: pinned Session items are never drop candidates in assemble.
    #[serde(default)]
    pub pinned: bool,
    /// ctx-016: set by [`compact_once`] on its survivors. Already-compacted
    /// items are never drop candidates — later passes preserve them verbatim,
    /// making repeated compaction at a fixed budget a no-op. Defaults false.
    #[serde(default)]
    pub compacted: bool,
}

/// Pure allocation walk (ADR-0013 D2/D3, ctx-002): keep items in the given
/// order; if their total exceeds `budget`, drop stale Ephemeral first, then
/// remaining Ephemeral, then oldest Turn items, then oldest Session items —
/// never Prefix, never the last Session item (the live user message; its
/// result must survive), never a ctx-004 pinned item, never an
/// already-compacted ctx-016 item. Returns kept items;
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
                && !item.compacted
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

/// ctx-016: ONE idempotent compaction pass. Runs [`assemble`] against
/// `budget_tokens`, then marks every survivor `compacted`. Because assemble
/// never drops already-compacted items, feeding its output back in — at the
/// same or any smaller budget — returns it unchanged: the second call is a
/// no-op and survivors of earlier passes are preserved verbatim.
pub fn compact_once(items: Vec<ContextItem>, budget_tokens: usize) -> Vec<ContextItem> {
    let mut kept = assemble(items, budget_tokens);
    for item in &mut kept {
        item.compacted = true;
    }
    kept
}

/// ctx-010: journal record for one [`compact_with_journal`] pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionEvent {
    pub at_ms: u128,
    pub dropped: usize,
    pub tokens_saved: usize,
}

/// ctx-010: [`compact_once`] plus a journal event, `Some` only when the pass
/// actually dropped items. `tokens_saved` is the dropped est_tokens sum;
/// assemble is order-preserving and drop-only, so the input/output deltas are
/// exact.
pub fn compact_with_journal(
    items: Vec<ContextItem>,
    budget_tokens: usize,
) -> (Vec<ContextItem>, Option<CompactionEvent>) {
    let before_count = items.len();
    let before_tokens: usize = items.iter().map(|i| i.est_tokens).sum();
    let kept = compact_once(items, budget_tokens);
    let dropped = before_count - kept.len();
    if dropped == 0 {
        return (kept, None);
    }
    let tokens_saved =
        before_tokens.saturating_sub(kept.iter().map(|i| i.est_tokens).sum::<usize>());
    (
        kept,
        Some(CompactionEvent {
            at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            dropped,
            tokens_saved,
        }),
    )
}

/// ctx-003 ext: serialize events as JSONL (one event per line) for the caller
/// to append to a journal-side log. Round-trips with [`parse_events_log`].
pub fn compaction_events_log(events: &[CompactionEvent]) -> String {
    let mut out = String::new();
    for e in events {
        if let Ok(line) = serde_json::to_string(e) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// ctx-003 ext: parse a JSONL events log back, skipping malformed lines
/// (and blank lines). Never fails — partial logs still yield the good rows.
pub fn parse_events_log(text: &str) -> Vec<CompactionEvent> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
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
            compacted: false,
        });
    }
}

/// ctx-015: persist Session-layer items as JSON lines (one item per line),
/// overwriting `path`. Round-trips with [`load_session_layer`].
pub fn save_session_layer(path: &Path, items: &[ContextItem]) -> Result<(), String> {
    let mut body = String::new();
    for item in items {
        let line = serde_json::to_string(item)
            .map_err(|e| format!("save_session_layer {}: {e}", path.display()))?;
        body.push_str(&line);
        body.push('\n');
    }
    std::fs::write(path, body).map_err(|e| format!("save_session_layer {}: {e}", path.display()))
}

/// ctx-015: load Session-layer items written by [`save_session_layer`].
/// A missing file is a fresh session: Ok(vec![]). Any unparsable line is a
/// corrupted history: Err (never silently truncated).
pub fn load_session_layer(path: &Path) -> Result<Vec<ContextItem>, String> {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("load_session_layer {}: {e}", path.display())),
    };
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .map_err(|e| format!("load_session_layer {}: {e}", path.display()))
        })
        .collect()
}

/// ctx-008: read-only inspector snapshot over a candidate-item slice.
/// `by_layer` is indexed by [`Layer`] in enum order (prefix, session,
/// turn, ephemeral) and holds (item count, summed est tokens) per layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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

/// ctx-042: count of stale items, [`stats`].stale_count accessor. Pure.
pub fn context_stale_count(items: &[ContextItem]) -> usize {
    stats(items).stale_count
}

/// ctx-044: total estimated tokens, [`stats`].est_tokens_total accessor.
/// Empty slice yields 0. Pure inspector helper.
pub fn context_total_tokens(items: &[ContextItem]) -> usize {
    stats(items).est_tokens_total
}

/// ctx-045: item count, [`stats`].total_items accessor. Empty slice yields 0.
/// Pure inspector helper.
pub fn context_total_items(items: &[ContextItem]) -> usize {
    stats(items).total_items
}

/// ctx-046: true when any item is stale ([`stats`].stale_count > 0).
/// Empty slice yields false. Pure inspector helper.
pub fn context_has_stale(items: &[ContextItem]) -> bool {
    stats(items).stale_count > 0
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

/// ctx-009: per-layer token weights (1.0 = raw estimate). Pure scoring input
/// for [`weighted_tokens`]; callers tune these to bias budget math by layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PriorityWeights {
    pub prefix: f32,
    pub session: f32,
    pub turn: f32,
    pub ephemeral: f32,
}

impl Default for PriorityWeights {
    fn default() -> Self {
        Self {
            prefix: 1.0,
            session: 1.0,
            turn: 1.0,
            ephemeral: 1.0,
        }
    }
}

/// ctx-009: sum of `est_tokens` scaled by each item's layer weight, rounded
/// up per item so fractional weights never under-count against the budget.
pub fn weighted_tokens(items: &[ContextItem], w: &PriorityWeights) -> usize {
    items
        .iter()
        .map(|i| {
            let weight = match i.layer {
                Layer::Prefix => w.prefix,
                Layer::Session => w.session,
                Layer::Turn => w.turn,
                Layer::Ephemeral => w.ephemeral,
            };
            (i.est_tokens as f32 * weight).ceil() as usize
        })
        .sum()
}

/// ctx-009 ext: validated constructor — every weight must be finite and >= 0.0.
pub fn weights_from_parts(
    prefix: f32,
    session: f32,
    turn: f32,
    ephemeral: f32,
) -> Result<PriorityWeights, String> {
    for (name, w) in [
        ("prefix", prefix),
        ("session", session),
        ("turn", turn),
        ("ephemeral", ephemeral),
    ] {
        if !w.is_finite() || w < 0.0 {
            return Err(format!("invalid {name} weight: {w}"));
        }
    }
    Ok(PriorityWeights {
        prefix,
        session,
        turn,
        ephemeral,
    })
}

/// ctx-009 ext: alias for [`PriorityWeights::default`] (all 1.0).
pub const NEUTRAL_WEIGHTS: PriorityWeights = PriorityWeights {
    prefix: 1.0,
    session: 1.0,
    turn: 1.0,
    ephemeral: 1.0,
};

/// ctx-018: one-line budget report for the inspector. `used` is the
/// [`weighted_tokens`] total at default weights; pct is clamped to 0-999.
pub fn budget_report(items: &[ContextItem], budget_tokens: usize) -> String {
    let used = weighted_tokens(items, &PriorityWeights::default());
    let pct = if budget_tokens == 0 {
        999 * usize::from(used > 0)
    } else {
        (used * 100 / budget_tokens).min(999)
    };
    format!(
        "context {used}/{budget_tokens} tokens ({pct}%), {} items",
        items.len()
    )
}

/// ctx-020: one-line staleness summary for the inspector.
/// "{n} items, {stale} stale ({pct}%)". `now_ms` reserved for future
/// time-based staleness; current staleness is the ctx-007 flag only. Pure.
pub fn stale_report(items: &[ContextItem], _now_ms: u128) -> String {
    let stale = items.iter().filter(|i| i.stale).count();
    let pct = if items.is_empty() {
        0
    } else {
        stale * 100 / items.len()
    };
    format!("{} items, {} stale ({}%)", items.len(), stale, pct)
}

/// ctx-019: per-layer item counts in enum order (prefix, session, turn,
/// ephemeral), zero-count layers skipped. Pure inspector helper.
pub fn items_by_layer(items: &[ContextItem]) -> Vec<(Layer, usize)> {
    let mut counts = [0usize; 4];
    for i in items {
        counts[i.layer as usize] += 1;
    }
    [
        (Layer::Prefix, counts[0]),
        (Layer::Session, counts[1]),
        (Layer::Turn, counts[2]),
        (Layer::Ephemeral, counts[3]),
    ]
    .into_iter()
    .filter(|&(_, n)| n > 0)
    .collect()
}

/// ctx-021: pretty JSON array export for the inspector/external tooling —
/// one object per item with layer, text, est_tokens, pinned, stale,
/// compacted. Empty slice serializes as "[]". Pure.
pub fn context_export_json(items: &[ContextItem]) -> String {
    serde_json::to_string_pretty(items).unwrap_or_default()
}

/// ctx-028: compact JSONL export for the inspector/external tooling — one
/// item per line, same per-item shape as [`context_export_json`] and
/// round-trippable with [`load_session_layer`]'s line format. Empty slice
/// yields an empty string. Pure.
pub fn context_export_jsonl(items: &[ContextItem]) -> String {
    let mut out = String::new();
    for item in items {
        if let Ok(line) = serde_json::to_string(item) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// ctx-035: compact JSONL export of stale items only — one line per item
/// where [`ContextItem::stale`] is set, same per-item shape as
/// [`context_export_jsonl`] (round-trips with [`load_session_layer`]'s line
/// format). No stale items yields an empty string. Pure.
pub fn context_stale_jsonl(items: &[ContextItem]) -> String {
    let mut out = String::new();
    for item in items.iter().filter(|i| i.stale) {
        if let Ok(line) = serde_json::to_string(item) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// ctx-039: compact JSONL export of pinned items only — one line per item
/// where [`ContextItem::pinned`] is set, same per-item shape as
/// [`context_export_jsonl`] (round-trips with [`load_session_layer`]'s line
/// format). No pinned items yields an empty string. Pure.
pub fn context_pinned_jsonl(items: &[ContextItem]) -> String {
    let mut out = String::new();
    for item in items.iter().filter(|i| i.pinned) {
        if let Ok(line) = serde_json::to_string(item) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// ctx-048: single-line compact JSON pinned-count summary —
/// {"pinned":P,"total":T} — the count twin of [`context_pinned_jsonl`]
/// for journal/inspector export. Empty slice yields zeros. Pure.
pub fn context_pinned_count_jsonl(items: &[ContextItem]) -> String {
    format!(
        "{{\"pinned\":{},\"total\":{}}}",
        items.iter().filter(|i| i.pinned).count(),
        items.len()
    )
}

/// ctx-049: single-line compact JSON stale-count summary — {"stale":S,"total":T}
/// — the count twin of [`context_stale_count`] for journal/inspector export.
/// Empty slice yields zeros. Pure.
pub fn context_stale_count_jsonl(items: &[ContextItem]) -> String {
    format!(
        "{{\"stale\":{},\"total\":{}}}",
        items.iter().filter(|i| i.stale).count(),
        items.len()
    )
}

/// ctx-036: pretty JSON array export of stale items only — one {layer, text}
/// object per item where [`ContextItem::stale`] is set. No stale items
/// serializes as "[]". Pure inspector helper.
pub fn context_stale_json(items: &[ContextItem]) -> String {
    let rows: Vec<serde_json::Value> = items
        .iter()
        .filter(|i| i.stale)
        .map(|i| serde_json::json!({ "layer": layer_name(i.layer), "text": i.text }))
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_default()
}

/// ctx-037: pretty JSON staleness summary for the inspector —
/// {stale_count, total, pct} computed from [`stats`]. Empty slice yields all
/// zeros with pct 0. Pure.
pub fn context_stale_report_json(items: &[ContextItem]) -> String {
    let s = stats(items);
    let pct = if s.total_items == 0 {
        0
    } else {
        s.stale_count * 100 / s.total_items
    };
    serde_json::to_string_pretty(&serde_json::json!({
        "stale_count": s.stale_count,
        "total": s.total_items,
        "pct": pct
    }))
    .unwrap_or_default()
}

/// ctx-043: single-line compact JSONL staleness summary —
/// {"total":N,"stale":S,"pct":P} — the one-line twin of
/// [`context_stale_report_json`] (pretty) for journal/inspector export.
/// Empty slice yields all zeros with pct 0. Pure.
pub fn context_stale_jsonl_report(items: &[ContextItem]) -> String {
    let s = stats(items);
    let pct = if s.total_items == 0 {
        0
    } else {
        s.stale_count * 100 / s.total_items
    };
    format!(
        "{{\"total\":{},\"stale\":{},\"pct\":{}}}",
        s.total_items, s.stale_count, pct
    )
}

/// ctx-022: one-line combined health report for the inspector —
/// [`budget_report`] and [`stale_report`] joined with " | ". Pure.
pub fn context_health_line(items: &[ContextItem], budget_tokens: usize, now_ms: u128) -> String {
    format!(
        "{} | {}",
        budget_report(items, budget_tokens),
        stale_report(items, now_ms)
    )
}

/// ctx-023: one line per non-empty layer for the inspector —
/// "{layer}: {n} items, {tok} tokens" in enum order via [`stats`].by_layer,
/// zero-count layers skipped. Pure.
pub fn context_layer_report(items: &[ContextItem]) -> String {
    let s = stats(items);
    [
        (Layer::Prefix, s.by_layer[0]),
        (Layer::Session, s.by_layer[1]),
        (Layer::Turn, s.by_layer[2]),
        (Layer::Ephemeral, s.by_layer[3]),
    ]
    .into_iter()
    .filter(|&(_, (n, _))| n > 0)
    .map(|(l, (n, tok))| format!("{}: {} items, {} tokens", layer_name(l), n, tok))
    .collect::<Vec<_>>()
    .join("\n")
}

/// ctx-024: the items of one layer, in input order — the slice behind
/// [`items_by_layer`]'s per-layer counts. Pure inspector helper.
pub fn context_by_layer(items: &[ContextItem], layer: Layer) -> Vec<&ContextItem> {
    items.iter().filter(|i| i.layer == layer).collect()
}

/// ctx-025: per-layer token totals in enum order (prefix, session, turn,
/// ephemeral), zero-token layers skipped. Pure inspector helper.
pub fn context_tokens_by_layer(items: &[ContextItem]) -> Vec<(Layer, usize)> {
    let mut tokens = [0usize; 4];
    for i in items {
        tokens[i.layer as usize] += i.est_tokens;
    }
    [
        (Layer::Prefix, tokens[0]),
        (Layer::Session, tokens[1]),
        (Layer::Turn, tokens[2]),
        (Layer::Ephemeral, tokens[3]),
    ]
    .into_iter()
    .filter(|&(_, n)| n > 0)
    .collect()
}

/// ctx-026: top-N layers by item count, largest first — [`items_by_layer`]
/// sorted by count descending, truncation to n. Stable sort keeps enum order
/// for count ties. n=0 or empty input yields empty. Pure inspector helper.
pub fn context_top_layers(items: &[ContextItem], n: usize) -> Vec<(Layer, usize)> {
    let mut rows = items_by_layer(items);
    // ponytail: stable sort on ≤4 rows; no heap needed.
    rows.sort_by(|a, b| b.1.cmp(&a.1));
    rows.truncate(n);
    rows
}

/// ctx-027: pretty JSON of [`stats`] for the inspector/external tooling —
/// total_items, est_tokens_total, by_layer (four [count, tokens] pairs in
/// enum order: prefix, session, turn, ephemeral), stale_count. Empty input
/// serializes the all-zero stats. Pure.
pub fn context_stats_json(items: &[ContextItem]) -> String {
    serde_json::to_string_pretty(&stats(items)).unwrap_or_default()
}

/// ctx-029: pretty JSON health snapshot combining [`stats`] with the budget
/// picture — {"stats": {...}, "budget": {budget, used, pct}}. `used` is the
/// [`weighted_tokens`] total at default weights; pct matches
/// [`budget_report`]'s clamped 0-999 percentage. Pure.
pub fn context_health_json(items: &[ContextItem], budget_tokens: usize) -> String {
    let used = weighted_tokens(items, &PriorityWeights::default());
    let pct = if budget_tokens == 0 {
        999 * usize::from(used > 0)
    } else {
        (used * 100 / budget_tokens).min(999)
    };
    serde_json::to_string_pretty(&serde_json::json!({
        "stats": stats(items),
        "budget": { "budget": budget_tokens, "used": used, "pct": pct },
    }))
    .unwrap_or_default()
}

/// ctx-030: pretty JSON array export for the inspector/external tooling —
/// one {layer, items, tokens} object per non-empty layer in enum order via
/// [`stats`].by_layer. Empty slice serializes as "[]". Pure.
pub fn context_layer_json(items: &[ContextItem]) -> String {
    let s = stats(items);
    let rows: Vec<serde_json::Value> = [
        (Layer::Prefix, s.by_layer[0]),
        (Layer::Session, s.by_layer[1]),
        (Layer::Turn, s.by_layer[2]),
        (Layer::Ephemeral, s.by_layer[3]),
    ]
    .into_iter()
    .filter(|&(_, (n, _))| n > 0)
    .map(|(l, (n, tok))| serde_json::json!({ "layer": layer_name(l), "items": n, "tokens": tok }))
    .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_default()
}

/// ctx-031: compact JSONL export of one layer's items in input order —
/// [`context_by_layer`] filtered through the same per-item line format as
/// [`context_export_jsonl`]. Empty layer yields an empty string. Pure.
pub fn context_layer_jsonl(items: &[ContextItem], layer: Layer) -> String {
    let mut out = String::new();
    for item in context_by_layer(items, layer) {
        if let Ok(line) = serde_json::to_string(item) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// ctx-032: pretty JSON array export of [`context_tokens_by_layer`] — one
/// {layer, tokens} object per non-empty layer in enum order. Empty input
/// serializes as "[]". Pure inspector helper.
pub fn context_tokens_by_layer_json(items: &[ContextItem]) -> String {
    let rows: Vec<serde_json::Value> = context_tokens_by_layer(items)
        .into_iter()
        .map(|(l, tok)| serde_json::json!({ "layer": layer_name(l), "tokens": tok }))
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_default()
}

/// ctx-033: pretty JSON array export of [`context_top_layers`] — one
/// {layer, count} object per row, largest count first, truncated to n.
/// n=0 or empty input serializes as "[]". Pure inspector helper.
pub fn context_top_layers_json(items: &[ContextItem], n: usize) -> String {
    let rows: Vec<serde_json::Value> = context_top_layers(items, n)
        .into_iter()
        .map(|(l, count)| serde_json::json!({ "layer": layer_name(l), "count": count }))
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_default()
}

/// ctx-034: JSON wrapper around [`preview`] — {"preview": "...",
/// "truncated": bool} where truncated is true when [`preview`] dropped any
/// item. Pure inspector helper.
pub fn context_preview_json(items: &[ContextItem], max_chars: usize) -> String {
    let p = preview(items, max_chars);
    // Truncated iff fewer item lines survived than there were items
    // (covers the "+N more" marker and its hard-truncated variants).
    let shown = p.lines().filter(|l| l.starts_with('[')).count();
    let truncated = shown < items.len();
    serde_json::to_string_pretty(&serde_json::json!({
        "preview": p,
        "truncated": truncated,
    }))
    .unwrap_or_default()
}

/// ctx-038: single-line compact JSON budget record — {used, budget, pct}.
/// `used` is the [`weighted_tokens`] total at default weights; pct matches
/// [`budget_report`]'s clamped 0-999 percentage (0 when budget is empty and
/// nothing is used). Pure inspector helper.
pub fn context_budget_jsonl(items: &[ContextItem], budget_tokens: usize) -> String {
    let used = weighted_tokens(items, &PriorityWeights::default());
    let pct = if budget_tokens == 0 {
        999 * usize::from(used > 0)
    } else {
        (used * 100 / budget_tokens).min(999)
    };
    serde_json::to_string(&serde_json::json!({
        "used": used,
        "budget": budget_tokens,
        "pct": pct
    }))
    .unwrap_or_default()
}

/// ctx-040: pretty JSON array export of pinned items only — one {layer, text}
/// object per item where [`ContextItem::pinned`] is set. No pinned items
/// serializes as "[]". Pure inspector helper.
pub fn context_pinned_json(items: &[ContextItem]) -> String {
    let rows: Vec<serde_json::Value> = items
        .iter()
        .filter(|i| i.pinned)
        .map(|i| serde_json::json!({ "layer": layer_name(i.layer), "text": i.text }))
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_default()
}

/// ctx-041: count of items where [`ContextItem::pinned`] is set.
/// Empty slice yields 0. Pure inspector helper.
pub fn context_pinned_count(items: &[ContextItem]) -> usize {
    items.iter().filter(|i| i.pinned).count()
}

/// ctx-047: true when at least one item has [`ContextItem::pinned`] set.
/// Pure inspector helper over [`context_pinned_count`].
pub fn context_has_pinned(items: &[ContextItem]) -> bool {
    context_pinned_count(items) > 0
}

/// ctx-050: pretty JSON pinned-count summary — {pinned, total} — the pretty
/// twin of [`context_pinned_count_jsonl`] for inspector/external tooling.
/// Empty slice yields zeros. Pure.
pub fn context_pinned_count_json(items: &[ContextItem]) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "pinned": context_pinned_count(items),
        "total": items.len()
    }))
    .unwrap_or_default()
}

/// ctx-051: pretty JSON stale-count summary — {stale, total} — the pretty
/// twin of [`context_stale_count_jsonl`] for inspector/external tooling.
/// Empty slice yields zeros. Pure.
pub fn context_stale_count_json(items: &[ContextItem]) -> String {
    serde_json::to_string_pretty(&serde_json::json!({
        "stale": context_stale_count(items),
        "total": items.len()
    }))
    .unwrap_or_default()
}

/// ctx-052: single-line compact JSON pinned report —
/// {"pinned":P,"total":T,"pct":N} where pct is pinned share of total
/// (0 when empty). Pure.
pub fn context_pinned_count_report(items: &[ContextItem]) -> String {
    let pinned = context_pinned_count(items);
    let total = items.len();
    let pct = if total == 0 { 0 } else { pinned * 100 / total };
    format!("{{\"pinned\":{pinned},\"total\":{total},\"pct\":{pct}}}")
}

/// ctx-053: pretty JSON pinned report — {pinned, total, pct} where pct is
/// pinned share of total (0 when empty) — the pretty twin of
/// [`context_pinned_count_report`] for inspector/external tooling. Pure.
pub fn context_pinned_report_json(items: &[ContextItem]) -> String {
    let pinned = context_pinned_count(items);
    let total = items.len();
    let pct = if total == 0 { 0 } else { pinned * 100 / total };
    serde_json::to_string_pretty(&serde_json::json!({
        "pinned": pinned,
        "total": total,
        "pct": pct
    }))
    .unwrap_or_default()
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
            compacted: false,
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
                compacted: false,
            },
            ContextItem {
                layer: Layer::Ephemeral,
                text: "stale dump".into(),
                est_tokens: 10,
                stale: true,
                pinned: false,
                compacted: false,
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
                compacted: false,
            },
            ContextItem {
                layer: Layer::Ephemeral,
                text: "fresh dump".into(),
                est_tokens: 6,
                stale: false,
                pinned: false,
                compacted: false,
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

    // ctx-042
    #[test]
    fn context_stale_count_seeded_matches_stats() {
        let mut items = vec![item(Layer::Ephemeral, "a", 1), item(Layer::Session, "b", 2)];
        items[0].stale = true;
        items[1].stale = true;
        assert_eq!(context_stale_count(&items), 2);
        assert_eq!(context_stale_count(&items), stats(&items).stale_count);
    }

    #[test]
    fn context_stale_count_empty_is_zero() {
        assert_eq!(context_stale_count(&[]), 0);
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

    // ctx-009
    #[test]
    fn uniform_weights_equal_raw_total() {
        let items = vec![
            item(Layer::Prefix, "sys", 10),
            item(Layer::Session, "old", 5),
            item(Layer::Turn, "now", 7),
            item(Layer::Ephemeral, "scratch", 3),
        ];
        assert_eq!(weighted_tokens(&items, &PriorityWeights::default()), 25);
    }

    #[test]
    fn doubled_turn_weight_doubles_only_turn_tokens() {
        let items = vec![
            item(Layer::Prefix, "sys", 10),
            item(Layer::Session, "old", 5),
            item(Layer::Turn, "now", 7),
        ];
        let mut w = PriorityWeights::default();
        w.turn = 2.0;
        assert_eq!(weighted_tokens(&items, &w), 10 + 5 + 14);
    }

    #[test]
    fn weighted_tokens_empty_input_is_zero() {
        assert_eq!(weighted_tokens(&[], &PriorityWeights::default()), 0);
    }

    // ctx-009 ext
    #[test]
    fn weights_from_parts_valid_passthrough() {
        let w = weights_from_parts(0.5, 1.5, 0.0, 2.0).unwrap();
        assert_eq!(w, PriorityWeights { prefix: 0.5, session: 1.5, turn: 0.0, ephemeral: 2.0 });
        assert_eq!(NEUTRAL_WEIGHTS, PriorityWeights::default());
    }

    #[test]
    fn weights_from_parts_rejects_negative() {
        assert!(weights_from_parts(-0.1, 1.0, 1.0, 1.0).is_err());
        assert!(weights_from_parts(1.0, -1.0, 1.0, 1.0).is_err());
        assert!(weights_from_parts(1.0, 1.0, -1.0, 1.0).is_err());
        assert!(weights_from_parts(1.0, 1.0, 1.0, -1.0).is_err());
    }

    #[test]
    fn weights_from_parts_rejects_nan() {
        assert!(weights_from_parts(f32::NAN, 1.0, 1.0, 1.0).is_err());
        assert!(weights_from_parts(f32::INFINITY, 1.0, 1.0, 1.0).is_err());
    }

    // ctx-015
    #[test]
    fn session_layer_round_trips_content_layer_tokens_pinned() {
        let path =
            std::env::temp_dir().join(format!("zdt-ctx015-roundtrip-{}.jsonl", std::process::id()));
        let items = vec![
            item(Layer::Session, "history line", 12),
            ContextItem {
                layer: Layer::Session,
                text: "pinned fact".into(),
                est_tokens: 7,
                stale: false,
                pinned: true,
                compacted: false,
            },
        ];
        save_session_layer(&path, &items).unwrap();
        let loaded = load_session_layer(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].text, "history line");
        assert_eq!(loaded[0].layer, Layer::Session);
        assert_eq!(loaded[0].est_tokens, 12);
        assert!(!loaded[0].stale && !loaded[0].pinned);
        assert_eq!(loaded[1].text, "pinned fact");
        assert_eq!(loaded[1].est_tokens, 7);
        assert!(loaded[1].pinned);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_missing_session_layer_is_empty() {
        let path = std::env::temp_dir().join("zdt-ctx015-missing-never-created.jsonl");
        let _ = std::fs::remove_file(&path);
        assert_eq!(
            load_session_layer(&path).unwrap(),
            Vec::<ContextItem>::new()
        );
    }

    #[test]
    fn corrupted_session_layer_errors() {
        let path =
            std::env::temp_dir().join(format!("zdt-ctx015-corrupt-{}.jsonl", std::process::id()));
        std::fs::write(&path, "{\"layer\":\"session\",\"text\":\n{not json}\n").unwrap();
        assert!(load_session_layer(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    // ctx-016
    #[test]
    fn double_compaction_is_idempotent() {
        let items = vec![
            item(Layer::Prefix, "sys", 5),
            item(Layer::Session, "oldest", 20),
            item(Layer::Session, "latest", 10), // last session: pinned survivor
            item(Layer::Turn, "t1", 15),
            item(Layer::Ephemeral, "dump", 30),
        ];
        let once = compact_once(items, 40);
        assert!(once.iter().all(|i| i.compacted), "survivors are marked");
        let twice = compact_once(once.clone(), 40);
        assert_eq!(once, twice, "second identical pass is a no-op");
    }

    #[test]
    fn compacted_items_survive_tighter_budgets() {
        let items = vec![
            item(Layer::Prefix, "sys", 5),
            item(Layer::Session, "old1", 20),
            item(Layer::Session, "old2", 20),
            item(Layer::Session, "latest", 20),
            item(Layer::Turn, "t1", 20),
            item(Layer::Turn, "t2", 20),
        ];
        // Total 105; budget drops both Turn items, keeps 65 tokens of Session.
        let once = compact_once(items, 70);
        assert_eq!(
            once.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            vec!["sys", "old1", "old2", "latest"]
        );
        // Budget 30 would normally keep only sys+latest; the already-compacted
        // old1/old2 are preserved verbatim instead of re-dropped.
        let tighter = compact_once(once.clone(), 30);
        assert_eq!(once, tighter);
    }

    // ctx-010
    #[test]
    fn compact_journal_no_drop_returns_none() {
        let items = vec![
            item(Layer::Prefix, "sys", 5),
            item(Layer::Session, "latest", 10),
        ];
        let (kept, event) = compact_with_journal(items, 100);
        assert_eq!(kept.len(), 2);
        assert!(event.is_none(), "nothing dropped => no journal event");
    }

    #[test]
    fn compact_journal_drop_reports_dropped_and_tokens_saved() {
        let items = vec![
            item(Layer::Prefix, "sys", 5),
            item(Layer::Session, "oldest", 20),
            item(Layer::Session, "latest", 10), // last session: survivor
            item(Layer::Turn, "t1", 15),
            item(Layer::Ephemeral, "dump", 30),
        ];
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let (kept, event) = compact_with_journal(items, 40);
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        // Total 80; dropping dump(30)+t1(15) reaches 35 <= 40, so oldest stays.
        assert_eq!(
            kept.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            vec!["sys", "oldest", "latest"]
        );
        let event = event.expect("drops occurred => Some(event)");
        assert_eq!(event.dropped, 2);
        assert_eq!(event.tokens_saved, 15 + 30);
        assert!(
            before <= event.at_ms && event.at_ms <= after,
            "at_ms {} outside [{before},{after}]",
            event.at_ms
        );
    }

    #[test]
    fn compact_journal_event_matches_kept_delta() {
        let items = vec![
            item(Layer::Session, "old1", 20),
            item(Layer::Session, "old2", 20),
            item(Layer::Session, "old3", 20),
            item(Layer::Session, "latest", 20),
        ];
        let (kept, event) = compact_with_journal(items, 50);
        let event = event.expect("over budget => drops");
        assert_eq!(kept.len() + event.dropped, 4);
        let kept_tokens: usize = kept.iter().map(|i| i.est_tokens).sum();
        assert_eq!(kept_tokens + event.tokens_saved, 80);
    }

    #[test]
    fn events_log_round_trips() {
        let events = vec![
            CompactionEvent {
                at_ms: 1_700_000_000_123,
                dropped: 2,
                tokens_saved: 45,
            },
            CompactionEvent {
                at_ms: 1_700_000_001_456,
                dropped: 1,
                tokens_saved: 30,
            },
        ];
        let log = compaction_events_log(&events);
        assert_eq!(log.lines().count(), 2, "one event per line");
        assert_eq!(parse_events_log(&log), events);
    }

    #[test]
    fn events_log_skips_malformed_lines() {
        let good = serde_json::to_string(&CompactionEvent {
            at_ms: 7,
            dropped: 1,
            tokens_saved: 9,
        })
        .unwrap();
        let log = format!("not json\n{{\"broken\": true}}\n{good}\n\n{good}");
        let parsed = parse_events_log(&log);
        assert_eq!(parsed.len(), 2);
        assert_eq!(
            parsed[0],
            CompactionEvent {
                at_ms: 7,
                dropped: 1,
                tokens_saved: 9
            }
        );
    }

    #[test]
    fn events_log_empty_inputs_are_empty() {
        assert_eq!(compaction_events_log(&[]), "");
        assert!(parse_events_log("").is_empty());
        assert!(parse_events_log("\n\n").is_empty());
    }

    #[test]
    fn budget_report_exact_formatting() {
        let items = vec![item(Layer::Prefix, "sys", 30), item(Layer::Turn, "now", 20)];
        // default weights are 1.0 => used = 50 of 100 => 50%.
        assert_eq!(
            budget_report(&items, 100),
            "context 50/100 tokens (50%), 2 items"
        );
    }

    #[test]
    fn budget_report_shows_over_budget_pct() {
        let items = vec![item(Layer::Session, "big", 250)];
        assert_eq!(
            budget_report(&items, 100),
            "context 250/100 tokens (250%), 1 items"
        );
    }

    #[test]
    fn budget_report_pct_is_clamped_to_999() {
        let items = vec![item(Layer::Session, "huge", 10_000)];
        assert_eq!(
            budget_report(&items, 10),
            "context 10000/10 tokens (999%), 1 items"
        );
    }

    #[test]
    fn budget_report_empty_items_are_zero_used_and_pct() {
        assert_eq!(
            budget_report(&[], 500),
            "context 0/500 tokens (0%), 0 items"
        );
        // Degenerate zero budget: no division by zero; anything used pins at 999.
        assert_eq!(budget_report(&[], 0), "context 0/0 tokens (0%), 0 items");
        assert_eq!(
            budget_report(&[item(Layer::Turn, "t", 5)], 0),
            "context 5/0 tokens (999%), 1 items"
        );
    }

    // ctx-019
    #[test]
    fn items_by_layer_seeded_mix_counts_in_enum_order() {
        let items = vec![
            item(Layer::Turn, "t1", 1),
            item(Layer::Prefix, "sys", 1),
            item(Layer::Ephemeral, "e1", 1),
            item(Layer::Session, "s1", 1),
            item(Layer::Turn, "t2", 1),
            item(Layer::Ephemeral, "e2", 1),
        ];
        assert_eq!(
            items_by_layer(&items),
            vec![
                (Layer::Prefix, 1),
                (Layer::Session, 1),
                (Layer::Turn, 2),
                (Layer::Ephemeral, 2)
            ]
        );
    }

    #[test]
    fn items_by_layer_empty_input_is_empty() {
        assert!(items_by_layer(&[]).is_empty());
    }

    #[test]
    fn items_by_layer_single_layer_yields_one_row() {
        let items = vec![
            item(Layer::Ephemeral, "e", 1),
            item(Layer::Ephemeral, "f", 1),
        ];
        assert_eq!(items_by_layer(&items), vec![(Layer::Ephemeral, 2)]);
    }

    #[test]
    fn stale_report_seeded_counts_flagged_items_exactly() {
        let items = vec![
            item(Layer::Session, "a", 1),
            item(Layer::Ephemeral, "b", 1),
            item(Layer::Ephemeral, "c", 1),
        ];
        let mut flagged = items;
        flagged[1].stale = true;
        flagged[2].stale = true;
        assert_eq!(stale_report(&flagged, 0), "3 items, 2 stale (66%)");
    }

    #[test]
    fn stale_report_empty_is_zeros() {
        assert_eq!(stale_report(&[], 0), "0 items, 0 stale (0%)");
    }

    // ctx-021
    #[test]
    fn context_export_json_seeded_is_valid_with_all_fields() {
        let mut items = vec![
            item(Layer::Prefix, "sys prompt", 7),
            item(Layer::Session, "history", 4),
        ];
        items[1].pinned = true;
        items[1].stale = true;
        items[1].compacted = true;
        let json = context_export_json(&items);
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
        let arr = parsed.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        for obj in arr {
            let obj = obj.as_object().unwrap();
            assert_eq!(obj.len(), 6);
            assert!(obj.contains_key("layer"));
            assert!(obj.contains_key("text"));
            assert!(obj.contains_key("est_tokens"));
            assert!(obj.contains_key("pinned"));
            assert!(obj.contains_key("stale"));
            assert!(obj.contains_key("compacted"));
        }
        assert_eq!(arr[0]["layer"], "prefix");
        assert_eq!(arr[0]["text"], "sys prompt");
        assert_eq!(arr[0]["est_tokens"], 7);
        assert_eq!(arr[0]["pinned"], false);
        assert_eq!(arr[1]["layer"], "session");
        assert_eq!(arr[1]["pinned"], true);
        assert_eq!(arr[1]["stale"], true);
        assert_eq!(arr[1]["compacted"], true);
    }

    #[test]
    fn context_export_json_empty_is_empty_array() {
        assert_eq!(context_export_json(&[]), "[]");
    }

    // ctx-028
    #[test]
    fn context_export_jsonl_seeded_yields_valid_line_per_item() {
        let mut items = vec![
            item(Layer::Prefix, "sys prompt", 7),
            item(Layer::Session, "history", 4),
        ];
        items[1].pinned = true;
        items[1].stale = true;
        items[1].compacted = true;
        let jsonl = context_export_jsonl(&items);
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), items.len());
        let parsed: Vec<ContextItem> = lines
            .iter()
            .map(|l| serde_json::from_str(l).expect("valid JSONL line"))
            .collect();
        assert_eq!(parsed, items);
    }

    #[test]
    fn context_export_jsonl_empty_is_empty_string() {
        assert_eq!(context_export_jsonl(&[]), "");
    }

    // ctx-035
    #[test]
    fn context_stale_jsonl_seeded_yields_only_stale_lines() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Ephemeral, "stale body /tmp/a.rs", 5),
            item(Layer::Session, "history", 4),
            item(Layer::Ephemeral, "fresh scratch", 3),
        ];
        items[1].stale = true;
        items[2].stale = true;
        let jsonl = context_stale_jsonl(&items);
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);
        let parsed: Vec<ContextItem> = lines
            .iter()
            .map(|l| serde_json::from_str(l).expect("valid JSONL line"))
            .collect();
        assert_eq!(
            parsed,
            vec![items[1].clone(), items[2].clone()],
            "only stale items, input order preserved"
        );
    }

    #[test]
    fn context_stale_jsonl_no_stale_is_empty_string() {
        let items = vec![item(Layer::Prefix, "sys", 7), item(Layer::Turn, "now", 4)];
        assert_eq!(context_stale_jsonl(&items), "");
        assert_eq!(context_stale_jsonl(&[]), "");
    }

    // ctx-039
    #[test]
    fn context_pinned_jsonl_seeded_yields_only_pinned_lines() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "pinned note", 5),
            item(Layer::Session, "history", 4),
            item(Layer::Turn, "latest", 3),
        ];
        items[0].pinned = true;
        items[1].pinned = true;
        let jsonl = context_pinned_jsonl(&items);
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);
        let parsed: Vec<ContextItem> = lines
            .iter()
            .map(|l| serde_json::from_str(l).expect("valid JSONL line"))
            .collect();
        assert_eq!(
            parsed,
            vec![items[0].clone(), items[1].clone()],
            "only pinned items, input order preserved"
        );
    }

    #[test]
    fn context_pinned_jsonl_no_pinned_is_empty_string() {
        let items = vec![item(Layer::Prefix, "sys", 7), item(Layer::Turn, "now", 4)];
        assert_eq!(context_pinned_jsonl(&items), "");
        assert_eq!(context_pinned_jsonl(&[]), "");
    }

    // ctx-036
    #[test]
    fn context_stale_json_seeded_yields_only_stale_rows() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Ephemeral, "stale body /tmp/a.rs", 5),
            item(Layer::Session, "history", 4),
            item(Layer::Ephemeral, "fresh scratch", 3),
        ];
        items[1].stale = true;
        items[2].stale = true;
        let parsed: Vec<serde_json::Value> =
            serde_json::from_str(&context_stale_json(&items)).expect("valid JSON");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["layer"], "ephemeral");
        assert_eq!(parsed[0]["text"], "stale body /tmp/a.rs");
        assert_eq!(parsed[1]["layer"], "session");
        assert_eq!(parsed[1]["text"], "history");
    }

    #[test]
    fn context_stale_json_no_stale_is_empty_array() {
        let items = vec![item(Layer::Prefix, "sys", 7), item(Layer::Turn, "now", 4)];
        assert_eq!(context_stale_json(&items), "[]");
        assert_eq!(context_stale_json(&[]), "[]");
    }

    // ctx-037
    #[test]
    fn context_stale_report_json_seeded_yields_exact_object() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Ephemeral, "stale body /tmp/a.rs", 5),
            item(Layer::Session, "history", 4),
            item(Layer::Ephemeral, "fresh scratch", 3),
        ];
        items[1].stale = true;
        items[2].stale = true;
        assert_eq!(
            context_stale_report_json(&items),
            r#"{
  "pct": 50,
  "stale_count": 2,
  "total": 4
}"#
        );
    }

    #[test]
    fn context_stale_report_json_empty_yields_zeros() {
        assert_eq!(
            context_stale_report_json(&[]),
            r#"{
  "pct": 0,
  "stale_count": 0,
  "total": 0
}"#
        );
    }

    // ctx-043
    #[test]
    fn context_stale_jsonl_report_seeded_yields_exact_line() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Ephemeral, "stale body /tmp/a.rs", 5),
            item(Layer::Session, "history", 4),
            item(Layer::Ephemeral, "fresh scratch", 3),
        ];
        items[1].stale = true;
        items[2].stale = true;
        assert_eq!(
            context_stale_jsonl_report(&items),
            r#"{"total":4,"stale":2,"pct":50}"#
        );
    }

    #[test]
    fn context_stale_jsonl_report_empty_yields_zeros() {
        assert_eq!(
            context_stale_jsonl_report(&[]),
            r#"{"total":0,"stale":0,"pct":0}"#
        );
    }

    // ctx-048
    #[test]
    fn context_pinned_count_jsonl_seeded_yields_exact_line() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "history", 4),
            item(Layer::Ephemeral, "scratch", 3),
        ];
        items[0].pinned = true;
        items[1].pinned = true;
        assert_eq!(
            context_pinned_count_jsonl(&items),
            r#"{"pinned":2,"total":3}"#
        );
    }

    #[test]
    fn context_pinned_count_jsonl_none_pinned_yields_zeros() {
        let items = vec![item(Layer::Session, "hi", 2)];
        assert_eq!(
            context_pinned_count_jsonl(&items),
            r#"{"pinned":0,"total":1}"#
        );
        assert_eq!(context_pinned_count_jsonl(&[]), r#"{"pinned":0,"total":0}"#);
    }

    // ctx-052
    #[test]
    fn context_pinned_count_report_seeded_yields_exact_line() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "history", 4),
            item(Layer::Ephemeral, "scratch", 3),
        ];
        items[0].pinned = true;
        assert_eq!(
            context_pinned_count_report(&items),
            r#"{"pinned":1,"total":3,"pct":33}"#
        );
    }

    #[test]
    fn context_pinned_count_report_none_pinned_yields_zeros() {
        let items = vec![item(Layer::Session, "hi", 2)];
        assert_eq!(
            context_pinned_count_report(&items),
            r#"{"pinned":0,"total":1,"pct":0}"#
        );
        assert_eq!(
            context_pinned_count_report(&[]),
            r#"{"pinned":0,"total":0,"pct":0}"#
        );
    }

    // ctx-053
    #[test]
    fn context_pinned_report_json_seeded_yields_exact_object() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "history", 4),
            item(Layer::Ephemeral, "scratch", 3),
        ];
        items[0].pinned = true;
        assert_eq!(
            context_pinned_report_json(&items),
            r#"{
  "pct": 33,
  "pinned": 1,
  "total": 3
}"#
        );
    }

    #[test]
    fn context_pinned_report_json_empty_yields_zeros() {
        assert_eq!(
            context_pinned_report_json(&[]),
            r#"{
  "pct": 0,
  "pinned": 0,
  "total": 0
}"#
        );
    }

    // ctx-050
    #[test]
    fn context_pinned_count_json_seeded_yields_exact_pretty_object() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "history", 4),
            item(Layer::Ephemeral, "scratch", 3),
        ];
        items[0].pinned = true;
        items[1].pinned = true;
        assert_eq!(
            context_pinned_count_json(&items),
            "{\n  \"pinned\": 2,\n  \"total\": 3\n}"
        );
    }

    #[test]
    fn context_pinned_count_json_none_pinned_yields_zeros() {
        let items = vec![item(Layer::Session, "hi", 2)];
        assert_eq!(
            context_pinned_count_json(&items),
            "{\n  \"pinned\": 0,\n  \"total\": 1\n}"
        );
        assert_eq!(
            context_pinned_count_json(&[]),
            "{\n  \"pinned\": 0,\n  \"total\": 0\n}"
        );
    }

    // ctx-049
    #[test]
    fn context_stale_count_jsonl_seeded_yields_exact_line() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "history", 4),
            item(Layer::Ephemeral, "scratch", 3),
        ];
        items[0].stale = true;
        items[1].stale = true;
        items[2].stale = true;
        assert_eq!(
            context_stale_count_jsonl(&items),
            r#"{"stale":3,"total":3}"#
        );
    }

    #[test]
    fn context_stale_count_jsonl_none_stale_yields_zeros() {
        let items = vec![item(Layer::Session, "hi", 2)];
        assert_eq!(
            context_stale_count_jsonl(&items),
            r#"{"stale":0,"total":1}"#
        );
        assert_eq!(context_stale_count_jsonl(&[]), r#"{"stale":0,"total":0}"#);
    }

    // ctx-051
    #[test]
    fn context_stale_count_json_seeded_yields_exact_pretty() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "history", 4),
            item(Layer::Ephemeral, "scratch", 3),
        ];
        items[1].stale = true;
        items[2].stale = true;
        assert_eq!(
            context_stale_count_json(&items),
            "{\n  \"stale\": 2,\n  \"total\": 3\n}"
        );
    }

    #[test]
    fn context_stale_count_json_none_stale_yields_zeros() {
        let items = vec![item(Layer::Session, "hi", 2)];
        assert_eq!(
            context_stale_count_json(&items),
            "{\n  \"stale\": 0,\n  \"total\": 1\n}"
        );
        assert_eq!(
            context_stale_count_json(&[]),
            "{\n  \"stale\": 0,\n  \"total\": 0\n}"
        );
    }

    // ctx-022
    #[test]
    fn context_health_line_seeded_combines_both_halves() {
        let mut items = vec![item(Layer::Prefix, "sys", 10), item(Layer::Session, "hi", 2)];
        items[1].stale = true;
        assert_eq!(
            context_health_line(&items, 20, 0),
            "context 12/20 tokens (60%), 2 items | 2 items, 1 stale (50%)"
        );
    }

    #[test]
    fn context_health_line_empty_is_zeros_line() {
        assert_eq!(
            context_health_line(&[], 100, 0),
            "context 0/100 tokens (0%), 0 items | 0 items, 0 stale (0%)"
        );
    }

    // ctx-023
    #[test]
    fn context_layer_report_seeded_is_exact_lines() {
        let items = vec![
            item(Layer::Prefix, "sys", 10),
            item(Layer::Turn, "t1", 3),
            item(Layer::Turn, "t2", 4),
            item(Layer::Ephemeral, "e1", 5),
        ];
        assert_eq!(
            context_layer_report(&items),
            "prefix: 1 items, 10 tokens\nturn: 2 items, 7 tokens\nephemeral: 1 items, 5 tokens"
        );
    }

    #[test]
    fn context_layer_report_empty_is_empty_string() {
        assert_eq!(context_layer_report(&[]), "");
    }

    // ctx-024
    #[test]
    fn context_by_layer_seeded_multi_layer_returns_only_matching_in_order() {
        let items = vec![
            item(Layer::Prefix, "sys", 1),
            item(Layer::Turn, "t1", 1),
            item(Layer::Session, "s1", 1),
            item(Layer::Turn, "t2", 1),
            item(Layer::Ephemeral, "e1", 1),
        ];
        let turns = context_by_layer(&items, Layer::Turn);
        assert_eq!(
            turns.iter().map(|i| i.text.as_str()).collect::<Vec<_>>(),
            vec!["t1", "t2"]
        );
        assert!(turns.iter().all(|i| i.layer == Layer::Turn));
        // Other layers still filter cleanly from the same seed.
        assert_eq!(
            context_by_layer(&items, Layer::Prefix)
                .iter()
                .map(|i| i.text.as_str())
                .collect::<Vec<_>>(),
            vec!["sys"]
        );
    }

    #[test]
    fn context_by_layer_empty_input_is_empty() {
        assert!(context_by_layer(&[], Layer::Session).is_empty());
    }

    // ctx-025
    #[test]
    fn context_tokens_by_layer_seeded_mix_sums_in_enum_order() {
        let items = vec![
            item(Layer::Turn, "t1", 3),
            item(Layer::Prefix, "sys", 10),
            item(Layer::Ephemeral, "e1", 5),
            item(Layer::Session, "s1", 2),
            item(Layer::Turn, "t2", 4),
            item(Layer::Ephemeral, "e2", 5),
        ];
        assert_eq!(
            context_tokens_by_layer(&items),
            vec![
                (Layer::Prefix, 10),
                (Layer::Session, 2),
                (Layer::Turn, 7),
                (Layer::Ephemeral, 10)
            ]
        );
    }

    #[test]
    fn context_tokens_by_layer_empty_input_is_empty() {
        assert!(context_tokens_by_layer(&[]).is_empty());
    }

    #[test]
    fn context_tokens_by_layer_single_layer_yields_one_row() {
        let items = vec![
            item(Layer::Ephemeral, "e", 4),
            item(Layer::Ephemeral, "f", 6),
        ];
        assert_eq!(
            context_tokens_by_layer(&items),
            vec![(Layer::Ephemeral, 10)]
        );
    }

    // ctx-026
    #[test]
    fn context_top_layers_seeded_orders_by_count_desc() {
        let items = vec![
            item(Layer::Turn, "t1", 1),
            item(Layer::Prefix, "sys", 1),
            item(Layer::Ephemeral, "e1", 1),
            item(Layer::Session, "s1", 1),
            item(Layer::Turn, "t2", 1),
            item(Layer::Ephemeral, "e2", 1),
        ];
        assert_eq!(
            context_top_layers(&items, 2),
            vec![(Layer::Turn, 2), (Layer::Ephemeral, 2)]
        );
        // Ties keep enum order: Turn before Ephemeral at count 2.
        assert_eq!(context_top_layers(&items, 4).len(), 4);
    }

    #[test]
    fn context_top_layers_n_zero_is_empty() {
        let items = vec![item(Layer::Session, "s", 1)];
        assert!(context_top_layers(&items, 0).is_empty());
    }

    #[test]
    fn context_top_layers_n_exceeding_total_yields_all_rows() {
        let items = vec![
            item(Layer::Ephemeral, "e", 1),
            item(Layer::Ephemeral, "f", 1),
        ];
        assert_eq!(context_top_layers(&items, 99), vec![(Layer::Ephemeral, 2)]);
        assert!(context_top_layers(&[], 5).is_empty());
    }

    // ctx-027
    #[test]
    fn context_stats_json_seeded_matches_exact_values() {
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
                compacted: false,
            },
        ];
        let parsed: serde_json::Value =
            serde_json::from_str(&context_stats_json(&items)).expect("valid JSON");
        assert_eq!(parsed["total_items"], 5);
        assert_eq!(parsed["est_tokens_total"], 29);
        assert_eq!(parsed["stale_count"], 1);
        assert_eq!(
            parsed["by_layer"],
            serde_json::json!([[1, 10], [2, 12], [1, 3], [1, 4]]) // prefix, session, turn, ephemeral
        );
    }

    #[test]
    fn context_stats_json_empty_is_all_zeros() {
        let parsed: serde_json::Value =
            serde_json::from_str(&context_stats_json(&[])).expect("valid JSON");
        assert_eq!(parsed["total_items"], 0);
        assert_eq!(parsed["est_tokens_total"], 0);
        assert_eq!(
            parsed["by_layer"],
            serde_json::json!([[0, 0], [0, 0], [0, 0], [0, 0]])
        );
        assert_eq!(parsed["stale_count"], 0);
    }

    // ctx-029
    #[test]
    fn context_health_json_seeded_matches_exact_values() {
        let items = vec![
            item(Layer::Prefix, "sys", 10),
            item(Layer::Session, "latest", 7),
            ContextItem {
                layer: Layer::Ephemeral,
                text: "dump".into(),
                est_tokens: 4,
                stale: true,
                pinned: false,
                compacted: false,
            },
        ];
        let parsed: serde_json::Value =
            serde_json::from_str(&context_health_json(&items, 50)).expect("valid JSON");
        assert_eq!(parsed["stats"]["total_items"], 3);
        assert_eq!(parsed["stats"]["est_tokens_total"], 21);
        assert_eq!(parsed["stats"]["stale_count"], 1);
        assert_eq!(
            parsed["stats"]["by_layer"],
            serde_json::json!([[1, 10], [1, 7], [0, 0], [1, 4]])
        );
        assert_eq!(
            parsed["budget"],
            serde_json::json!({"budget": 50, "used": 21, "pct": 42})
        );
    }

    #[test]
    fn context_health_json_empty_is_all_zeros() {
        let parsed: serde_json::Value =
            serde_json::from_str(&context_health_json(&[], 100)).expect("valid JSON");
        assert_eq!(parsed["stats"]["total_items"], 0);
        assert_eq!(parsed["stats"]["est_tokens_total"], 0);
        assert_eq!(parsed["stats"]["stale_count"], 0);
        assert_eq!(
            parsed["budget"],
            serde_json::json!({"budget": 100, "used": 0, "pct": 0})
        );
    }

    // ctx-030
    #[test]
    fn context_layer_json_seeded_matches_exact_values() {
        let items = vec![
            item(Layer::Prefix, "sys", 10),
            item(Layer::Session, "old", 5),
            item(Layer::Session, "latest", 7),
            item(Layer::Turn, "now", 3),
        ];
        let parsed: serde_json::Value =
            serde_json::from_str(&context_layer_json(&items)).expect("valid JSON");
        assert_eq!(
            parsed,
            serde_json::json!([
                { "layer": "prefix", "items": 1, "tokens": 10 },
                { "layer": "session", "items": 2, "tokens": 12 },
                { "layer": "turn", "items": 1, "tokens": 3 },
            ])
        );
    }

    #[test]
    fn context_layer_json_empty_is_empty_array() {
        assert_eq!(context_layer_json(&[]), "[]");
    }

    // ctx-031
    #[test]
    fn context_layer_jsonl_seeded_returns_only_that_layers_lines_in_order() {
        let items = vec![
            item(Layer::Prefix, "sys", 10),
            item(Layer::Session, "old", 5),
            item(Layer::Turn, "now", 3),
            item(Layer::Session, "latest", 7),
        ];
        let out = context_layer_jsonl(&items, Layer::Session);
        let parsed: Vec<ContextItem> = out
            .lines()
            .map(|l| serde_json::from_str(l).expect("valid JSON line"))
            .collect();
        assert_eq!(out.lines().count(), 2);
        assert_eq!(parsed[0].text, "old");
        assert_eq!(parsed[1].text, "latest");
        assert_eq!(parsed[0].layer, Layer::Session);
    }

    #[test]
    fn context_layer_jsonl_empty_layer_is_empty_string() {
        assert_eq!(context_layer_jsonl(&[], Layer::Turn), "");
        let items = vec![item(Layer::Prefix, "sys", 1)];
        assert_eq!(context_layer_jsonl(&items, Layer::Ephemeral), "");
    }

    // ctx-032
    #[test]
    fn context_tokens_by_layer_json_seeded_matches_tokens_by_layer() {
        let items = vec![
            item(Layer::Turn, "t1", 3),
            item(Layer::Prefix, "sys", 10),
            item(Layer::Ephemeral, "e1", 5),
            item(Layer::Session, "s1", 2),
            item(Layer::Turn, "t2", 4),
        ];
        let parsed: serde_json::Value =
            serde_json::from_str(&context_tokens_by_layer_json(&items)).expect("valid JSON");
        assert_eq!(
            parsed,
            serde_json::json!([
                { "layer": "prefix", "tokens": 10 },
                { "layer": "session", "tokens": 2 },
                { "layer": "turn", "tokens": 7 },
                { "layer": "ephemeral", "tokens": 5 },
            ])
        );
    }

    #[test]
    fn context_tokens_by_layer_json_empty_is_empty_array() {
        assert_eq!(context_tokens_by_layer_json(&[]), "[]");
    }

    // ctx-033
    #[test]
    fn context_top_layers_json_seeded_matches_top_layers() {
        let items = vec![
            item(Layer::Turn, "t1", 1),
            item(Layer::Prefix, "sys", 1),
            item(Layer::Ephemeral, "e1", 1),
            item(Layer::Session, "s1", 1),
            item(Layer::Turn, "t2", 1),
            item(Layer::Ephemeral, "e2", 1),
        ];
        let parsed: serde_json::Value =
            serde_json::from_str(&context_top_layers_json(&items, 2)).expect("valid JSON");
        assert_eq!(
            parsed,
            serde_json::json!([
                { "layer": "turn", "count": 2 },
                { "layer": "ephemeral", "count": 2 },
            ])
        );
    }

    #[test]
    fn context_top_layers_json_n_zero_is_empty_array() {
        let items = vec![item(Layer::Session, "s", 1)];
        assert_eq!(context_top_layers_json(&items, 0), "[]");
        assert_eq!(context_top_layers_json(&[], 5), "[]");
    }

    // ctx-034
    #[test]
    fn context_preview_json_fits_is_not_truncated() {
        let items = vec![item(Layer::Turn, "hi", 2)];
        let v: serde_json::Value =
            serde_json::from_str(&context_preview_json(&items, 200)).expect("valid JSON");
        assert_eq!(v["preview"], preview(&items, 200));
        assert_eq!(v["truncated"], false);
    }

    #[test]
    fn context_preview_json_over_is_truncated() {
        let items = vec![
            item(Layer::Turn, "aaaa", 2),
            item(Layer::Turn, "bbbb", 3),
            item(Layer::Turn, "cccc", 4),
        ];
        let budget = preview_line(&items[0]).chars().count() + 5;
        assert!(
            items.len() * preview_line(&items[0]).chars().count() > budget,
            "test must actually overflow"
        );
        let v: serde_json::Value =
            serde_json::from_str(&context_preview_json(&items, budget)).expect("valid JSON");
        assert_eq!(v["truncated"], true);
        assert_eq!(v["preview"], preview(&items, budget));
    }

    // ctx-038
    #[test]
    fn context_budget_jsonl_seeded_is_exact_line() {
        let items = vec![item(Layer::Turn, "t", 3), item(Layer::Prefix, "sys", 7)];
        assert_eq!(
            context_budget_jsonl(&items, 100),
            r#"{"budget":100,"pct":10,"used":10}"#
        );
    }

    #[test]
    fn context_budget_jsonl_empty_is_zeros_line() {
        assert_eq!(
            context_budget_jsonl(&[], 500),
            r#"{"budget":500,"pct":0,"used":0}"#
        );
        assert_eq!(context_budget_jsonl(&[], 0), r#"{"budget":0,"pct":0,"used":0}"#);
    }

    // ctx-040
    #[test]
    fn context_pinned_json_seeded_yields_only_pinned_rows() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "pinned note", 5),
            item(Layer::Session, "history", 4),
            item(Layer::Turn, "latest", 3),
        ];
        items[0].pinned = true;
        items[1].pinned = true;
        let parsed: Vec<serde_json::Value> =
            serde_json::from_str(&context_pinned_json(&items)).expect("valid JSON");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["layer"], "prefix");
        assert_eq!(parsed[0]["text"], "sys");
        assert_eq!(parsed[1]["layer"], "session");
        assert_eq!(parsed[1]["text"], "pinned note");
    }

    #[test]
    fn context_pinned_json_no_pinned_is_empty_array() {
        let items = vec![item(Layer::Prefix, "sys", 7), item(Layer::Turn, "now", 4)];
        assert_eq!(context_pinned_json(&items), "[]");
        assert_eq!(context_pinned_json(&[]), "[]");
    }

    // ctx-041
    #[test]
    fn context_pinned_count_seeded_is_exact() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "pinned note", 5),
            item(Layer::Session, "history", 4),
            item(Layer::Turn, "latest", 3),
        ];
        items[0].pinned = true;
        items[1].pinned = true;
        assert_eq!(context_pinned_count(&items), 2);
    }

    #[test]
    fn context_pinned_count_none_and_empty_are_zero() {
        let items = vec![item(Layer::Prefix, "sys", 7), item(Layer::Turn, "now", 4)];
        assert_eq!(context_pinned_count(&items), 0);
        assert_eq!(context_pinned_count(&[]), 0);
    }

    // ctx-047
    #[test]
    fn context_has_pinned_true_when_any_item_is_pinned() {
        let mut items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "note", 5),
            item(Layer::Turn, "latest", 3),
        ];
        items[1].pinned = true;
        assert!(context_has_pinned(&items));
    }

    #[test]
    fn context_has_pinned_false_when_none_or_empty() {
        let items = vec![item(Layer::Prefix, "sys", 7), item(Layer::Turn, "now", 4)];
        assert!(!context_has_pinned(&items));
        assert!(!context_has_pinned(&[]));
    }

    // ctx-044
    #[test]
    fn context_total_tokens_seeded_is_exact() {
        let items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "history", 4),
            item(Layer::Turn, "latest", 3),
            item(Layer::Ephemeral, "scratch", 11),
        ];
        assert_eq!(context_total_tokens(&items), 25);
    }

    #[test]
    fn context_total_tokens_empty_is_zero() {
        assert_eq!(context_total_tokens(&[]), 0);
    }

    // ctx-045
    #[test]
    fn context_total_items_seeded_is_exact() {
        let items = vec![
            item(Layer::Prefix, "sys", 7),
            item(Layer::Session, "history", 4),
            item(Layer::Turn, "latest", 3),
            item(Layer::Ephemeral, "scratch", 11),
        ];
        assert_eq!(context_total_items(&items), 4);
    }

    #[test]
    fn context_total_items_empty_is_zero() {
        assert_eq!(context_total_items(&[]), 0);
    }

    // ctx-046
    #[test]
    fn context_has_stale_true_when_any_stale() {
        let mut items = vec![item(Layer::Session, "a", 1), item(Layer::Turn, "b", 1)];
        items[1].stale = true;
        assert!(context_has_stale(&items));
    }

    #[test]
    fn context_has_stale_false_when_none() {
        assert!(!context_has_stale(&[]));
        let items = vec![item(Layer::Ephemeral, "fresh", 1)];
        assert!(!context_has_stale(&items));
    }
}
