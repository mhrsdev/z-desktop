//! Project memory records (mem-001..004, ADR-0014).
//!
//! Every mutation is a [`JournalKind::MemoryRecorded`] event; the per-layer
//! JSONL files under `data/memory/` are derived views ("views are caches,
//! the journal is truth") rebuilt by fold. A record is live iff it is
//! Promoted and not superseded (ADR-0014 D4) — the predicate retrieval
//! ranks within.

use crate::journal::{Journal, JournalKind, RecordDraft};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Where a record came from: resolvable coordinates denormalized from the
/// originating journal event so ranking never re-reads the journal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provenance {
    /// "message" | "tool" | "user" | "consolidation" (kept as a String so new
    /// writer roles do not need an enum release).
    pub kind: String,
    /// Canonical coordinate: message id / tool-call id / pass id / "user".
    pub r#ref: String,
    pub thread_id: String,
    pub turn_id: String,
    /// Capture time, milliseconds since the Unix epoch.
    pub ts_ms: u128,
}

/// The three persistent layers. Working/session are not stored here:
/// working dies with the turn, session lives in threads + the context
/// engine (ADR-0014 D1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Layer {
    Project,
    Semantic,
    Episodic,
}

impl Layer {
    pub fn as_str(&self) -> &'static str {
        match self {
            Layer::Project => "project",
            Layer::Semantic => "semantic",
            Layer::Episodic => "episodic",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Provisional,
    Promoted,
}

/// One append-only memory record. Construction goes through
/// [`MemoryRecord::new`], which enforces provenance and confidence — there
/// is no call site that can forget them (ADR-0014 D3). `superseded_by` is
/// reducer/backfill-managed; corrections are new records plus an updated
/// line for the predecessor, never in-place edits.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub layer: Layer,
    pub content: String,
    pub provenance: Provenance,
    pub confidence: f32,
    pub status: Status,
    pub superseded_by: Option<String>,
}

impl MemoryRecord {
    pub fn new(
        id: impl Into<String>,
        layer: Layer,
        content: impl Into<String>,
        provenance: Provenance,
        confidence: f32,
        status: Status,
    ) -> Result<Self, String> {
        if provenance.kind.trim().is_empty() || provenance.r#ref.trim().is_empty() {
            return Err(format!(
                "memory {}: provenance must be non-empty (kind, ref)",
                id.into()
            ));
        }
        if !(0.0..=1.0).contains(&confidence) {
            // `contains` is false for NaN too, so this rejects it as well.
            return Err("memory: confidence must be within [0.0, 1.0]".to_string());
        }
        Ok(MemoryRecord {
            id: id.into(),
            layer,
            content: content.into(),
            provenance,
            confidence,
            status,
            superseded_by: None,
        })
    }
}

/// Best-effort append of one memory record. Journal failures are warned and
/// dropped (same policy as evidence::record / Runtime::journal_record).
pub fn record(journal: &Mutex<Journal>, r: &MemoryRecord) {
    let payload = match serde_json::to_value(r) {
        Ok(v) => v,
        Err(err) => {
            log::warn!("memory: record {} does not serialize: {err}", r.id);
            return;
        }
    };
    let draft = RecordDraft::new(
        JournalKind::MemoryRecorded,
        Some(r.provenance.thread_id.clone()),
        payload,
    );
    if let Err(err) = journal.lock().unwrap().append(draft) {
        log::warn!("memory: append failed: {err}");
    }
}

/// Daily write budget (mem-014, ADR-0014 §104.3): ≤10 MB of new memory writes
/// per day; at bounded per-record prose this resolves to a 200-record/day cap.
/// Bulk writers stop early (partial success) rather than error past it.
pub const DAILY_RECORD_CAP: usize = 200;

/// Number of folded `memory_recorded` records whose provenance capture time
/// falls on the same UTC day as `now_ms` (mem-014). Counts distinct ids
/// post-compaction — superseded/provisional lines still occupy journal space.
/// An unreadable journal returns `usize::MAX`: corrupt storage must block
/// further writes, not invite unbounded ones.
pub fn count_today(journal: &Mutex<Journal>, now_ms: u128) -> usize {
    const MS_PER_DAY: u128 = 86_400_000;
    let path = journal.lock().unwrap().path().to_path_buf();
    match MemoryView::fold(&path) {
        Ok(view) => view
            .records
            .iter()
            .filter(|r| r.provenance.ts_ms / MS_PER_DAY == now_ms / MS_PER_DAY)
            .count(),
        Err(e) => {
            log::warn!("memory: cannot fold journal for daily cap, blocking writes: {e}");
            usize::MAX
        }
    }
}

/// Folded state of all `memory_recorded` events in a journal segment:
/// last line per id wins (log-compaction semantics), insertion order kept.
#[derive(Debug, Default, PartialEq)]
pub struct MemoryView {
    pub records: Vec<MemoryRecord>,
}

impl MemoryView {
    /// Folds `memory_recorded` events in replay order. A corrupt payload
    /// fails loud (TasksView/EvidenceView precedent).
    pub fn fold(path: &Path) -> Result<MemoryView, String> {
        let mut bad_payload: Option<String> = None;
        let mut view = MemoryView::default();
        crate::reducer::fold(path, (), |(), rec| {
            if rec.kind != JournalKind::MemoryRecorded || bad_payload.is_some() {
                return;
            }
            match serde_json::from_value::<MemoryRecord>(rec.payload.clone()) {
                Ok(r) => view.apply(r),
                Err(e) => {
                    bad_payload = Some(format!(
                        "reducer {}: bad memory payload in seq {}: {e}",
                        path.display(),
                        rec.seq
                    ));
                }
            }
        })?;
        match bad_payload {
            Some(e) => Err(e),
            None => Ok(view),
        }
    }

    fn apply(&mut self, r: MemoryRecord) {
        match self.records.iter_mut().find(|existing| existing.id == r.id) {
            Some(existing) => *existing = r,
            None => self.records.push(r),
        }
    }

    /// ADR-0014 D4 live-record predicate: Promoted AND not superseded.
    /// Later lines already won inside `records`, so competing tips resolve
    /// deterministically to the highest seq.
    pub fn live(&self) -> Vec<&MemoryRecord> {
        self.records
            .iter()
            .filter(|r| r.status == Status::Promoted && r.superseded_by.is_none())
            .collect()
    }
}

/// One ranked retrieval hit (mem-008, ADR-0014): a live record plus its
/// heuristic score — 0.3*confidence + query-term overlap ratio.
#[derive(Debug, Clone, PartialEq)]
pub struct RankedMemory {
    pub record_id: String,
    pub content: String,
    pub score: f32,
}

/// Ranks [`MemoryView::live`] records against `query_terms` (mem-008):
/// score = 0.3*confidence + (#terms found case-insensitively in content /
/// #terms; 0 when no terms). Sorted by score desc, then id asc; capped.
/// ponytail: substring overlap, no embeddings/stemming — refine if recall
/// complaints show up.
pub fn retrieve(view: &MemoryView, query_terms: &[&str], cap: usize) -> Vec<RankedMemory> {
    let mut ranked: Vec<RankedMemory> = view
        .live()
        .iter()
        .map(|r| {
            let lower = r.content.to_lowercase();
            let overlap = if query_terms.is_empty() {
                0.0
            } else {
                query_terms
                    .iter()
                    .filter(|t| lower.contains(&t.to_lowercase()))
                    .count() as f32
                    / query_terms.len() as f32
            };
            RankedMemory {
                record_id: r.id.clone(),
                content: r.content.clone(),
                score: 0.3 * r.confidence + overlap,
            }
        })
        .collect();
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.record_id.cmp(&b.record_id))
    });
    ranked.truncate(cap);
    ranked
}

/// Exponential recency decay for one confidence value (mem-013):
/// `confidence * 0.5^(age_days / half_life_days)`, clamped to [0, 1].
pub fn decay_confidence(confidence: f32, age_days: u32, half_life_days: f32) -> f32 {
    if half_life_days <= 0.0 {
        // Degenerate half-life: skip decay rather than produce NaN/inf.
        return confidence.clamp(0.0, 1.0);
    }
    (confidence * 0.5f32.powf(age_days as f32 / half_life_days)).clamp(0.0, 1.0)
}

/// [`retrieve`] with recency-decayed confidence (mem-013): each candidate's
/// `0.3 * confidence` term is recomputed from
/// [`decay_confidence`] using its provenance capture time vs `now_ms`;
/// the query-term overlap term is unchanged. Re-sorted and capped.
pub fn retrieve_with_decay(
    view: &MemoryView,
    query_terms: &[&str],
    cap: usize,
    now_ms: u128,
    half_life_days: f32,
) -> Vec<RankedMemory> {
    const MS_PER_DAY: u128 = 86_400_000;
    let mut ranked = retrieve(view, query_terms, usize::MAX);
    for hit in ranked.iter_mut() {
        let Some(r) = view.records.iter().find(|r| r.id == hit.record_id) else {
            continue;
        };
        let age_days =
            ((now_ms.saturating_sub(r.provenance.ts_ms)) / MS_PER_DAY).min(u32::MAX as u128) as u32;
        hit.score +=
            0.3 * decay_confidence(r.confidence, age_days, half_life_days) - 0.3 * r.confidence;
    }
    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.record_id.cmp(&b.record_id))
    });
    ranked.truncate(cap);
    ranked
}

/// User correction (mem-010, ADR-0014): folds the journal to find the live
/// original, then appends TWO `memory_recorded` events — the original
/// re-recorded with `superseded_by` pointing at the replacement, and a new
/// Promoted replacement `{original_id}-c{ts}` carrying `corrected_content`,
/// the original's layer, and `new_confidence` clamped into [0, 1]. Returns
/// the replacement id. Corrections are new records plus an updated line for
/// the predecessor, never in-place edits.
pub fn correct(
    journal: &Mutex<Journal>,
    original_id: &str,
    corrected_content: &str,
    new_confidence: f32,
    thread_id: &str,
    turn_id: &str,
) -> Result<String, String> {
    let path = journal.lock().unwrap().path().to_path_buf();
    let view = MemoryView::fold(&path)?;
    let original = view
        .records
        .iter()
        .find(|r| r.id == original_id)
        .ok_or_else(|| format!("memory: cannot correct unknown record {original_id}"))?;
    if original.superseded_by.is_some() {
        return Err(format!(
            "memory: record {original_id} is already superseded"
        ));
    }

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let new_id = format!("{original_id}-c{now_ms}");
    let mut superseded_line = original.clone();
    superseded_line.superseded_by = Some(new_id.clone());
    let provenance = Provenance {
        kind: "user".into(),
        r#ref: turn_id.into(),
        thread_id: thread_id.into(),
        turn_id: turn_id.into(),
        ts_ms: now_ms,
    };
    // Built before any append so an invalid confidence fails loud without
    // leaving the original superseded with no replacement behind it.
    // ponytail: f32::clamp passes NaN through; MemoryRecord::new rejects it.
    let replacement = MemoryRecord::new(
        new_id.clone(),
        original.layer,
        corrected_content,
        provenance,
        new_confidence.clamp(0.0, 1.0),
        Status::Promoted,
    )?;
    record(journal, &superseded_line);
    record(journal, &replacement);
    Ok(new_id)
}

/// Dependents of a record (mem-011, ADR-0014): ids of records superseded by
/// `id`, transitively down the chain, nearest first (BFS from `id`). Cycle-
/// safe via visited-set semantics on `out`.
pub fn dependents_of(view: &MemoryView, id: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut frontier = vec![id.to_string()];
    while let Some(current) = frontier.pop() {
        for r in &view.records {
            if r.superseded_by.as_deref() == Some(current.as_str())
                && r.id != id
                && !out.contains(&r.id)
            {
                out.push(r.id.clone());
                frontier.push(r.id.clone());
            }
        }
    }
    out
}

/// Owns the `data/memory/` view directory: one append-only JSONL file per
/// layer, rebuilt from the journal (delete any of them and replaying
/// reproduces equivalent state — ADR-0014 D2).
pub struct MemoryStore {
    dir: PathBuf,
}

impl MemoryStore {
    pub fn open(data_dir: &Path) -> Self {
        MemoryStore {
            dir: data_dir.join("memory"),
        }
    }

    fn layer_path(&self, layer: Layer) -> PathBuf {
        self.dir.join(format!("{}.jsonl", layer.as_str()))
    }

    /// Rewrites `<layer>.jsonl` as the current view (last-line-wins fold of
    /// the journal). Atomic write: readers see old-or-new, never partial.
    pub fn write_layer_view(&self, layer: Layer, records: &[MemoryRecord]) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("memory: cannot create {}: {e}", self.dir.display()))?;
        let mut body = String::new();
        for r in records {
            let line = serde_json::to_string(r)
                .map_err(|e| format!("memory: record {} does not serialize: {e}", r.id))?;
            body.push_str(&line);
            body.push('\n');
        }
        crate::atomic_write::atomic_write(&self.layer_path(layer), body.as_bytes())
    }

    /// Parses a layer view back. Corrupt lines are skipped with a warning,
    /// matching the thread-file tolerance precedent (§30.1 / ADR-0014 D2) —
    /// views are caches; the journal remains the fail-loud source of truth.
    pub fn read_layer(&self, layer: Layer) -> Result<Vec<MemoryRecord>, String> {
        let path = self.layer_path(layer);
        let content = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            // A never-built view is an empty cache, not an error.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(format!("memory: cannot read {}: {e}", path.display())),
        };
        let mut records = Vec::new();
        for (index, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<MemoryRecord>(line) {
                Ok(r) => records.push(r),
                Err(e) => log::warn!(
                    "memory {}: skipping corrupt view line {}: {e}",
                    path.display(),
                    index + 1
                ),
            }
        }
        Ok(records)
    }
}

/// One heuristic hit from post-turn text extraction (mem-005, ADR-0014 D5).
/// Lands Provisional only; promotion is mem-007's job.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedCandidate {
    pub content: String,
    pub layer: Layer,
    pub confidence: f32,
}

/// Cap per extraction pass — well under ADR-0014 D5's ≤100/pass bound.
const MAX_CANDIDATES_PER_PASS: usize = 20;

/// Regex-free heuristics over one turn's final text: explicit-memory phrasing
/// ("remember that", "note that", "keep in mind", "the user prefers",
/// "always use", "never use") becomes Project-layer candidates at confidence
/// 0.6; definitional "X means Y" / "X is a Y" sentences become Semantic-layer
/// candidates at 0.5. Identical contents dedup; capped at
/// [`MAX_CANDIDATES_PER_PASS`].
/// ponytail: sentence-split + substring match — noisy on ordinary prose, safe
/// because everything lands Provisional (never live until mem-006/007).
pub fn extract_candidates(turn_text: &str) -> Vec<ExtractedCandidate> {
    const PROJECT_MARKERS: [&str; 6] = [
        "remember that",
        "note that",
        "keep in mind",
        "the user prefers",
        "always use",
        "never use",
    ];
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for raw in turn_text.split(|c| matches!(c, '.' | '!' | '?' | '\n' | ';')) {
        let content = raw.trim();
        if content.is_empty() {
            continue;
        }
        let lower = content.to_lowercase();
        let candidate = if PROJECT_MARKERS.iter().any(|m| lower.contains(m)) {
            ExtractedCandidate {
                content: content.to_string(),
                layer: Layer::Project,
                confidence: 0.6,
            }
        } else if lower.contains(" means ") || lower.contains(" is a ") {
            ExtractedCandidate {
                content: content.to_string(),
                layer: Layer::Semantic,
                confidence: 0.5,
            }
        } else {
            continue;
        };
        if seen.insert(lower) && out.len() < MAX_CANDIDATES_PER_PASS {
            out.push(candidate);
        }
    }
    out
}

/// Writes each candidate as a Provisional `memory_recorded` event through the
/// runtime journal (best-effort, mem-005 / ADR-0014 D5: supervised extraction,
/// Message/turn refs). Skips candidates whose exact content already exists in
/// any layer view (cheap exact-match slice of D5 step-2 similarity dedup).
/// Returns how many records were written; callers rebuild views via
/// [`MemoryStore`] when needed.
pub fn promote_candidates(
    journal: &Mutex<Journal>,
    dir: &Path,
    candidates: &[ExtractedCandidate],
    thread_id: &str,
    turn_id: &str,
) -> Result<usize, String> {
    let store = MemoryStore::open(dir);
    let mut existing: HashSet<String> = HashSet::new();
    for layer in [Layer::Project, Layer::Semantic, Layer::Episodic] {
        for r in store.read_layer(layer)? {
            existing.insert(r.content.trim().to_lowercase());
        }
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    // Daily budget (mem-014): records already written today count first;
    // past [`DAILY_RECORD_CAP`] we stop before writing and return the partial
    // count — a full day's quota is not an error condition.
    let mut today_count = count_today(journal, now_ms);
    let mut written = 0usize;
    for (i, c) in candidates.iter().enumerate() {
        if today_count >= DAILY_RECORD_CAP {
            break;
        }
        let content = c.content.trim();
        if !existing.insert(content.to_lowercase()) {
            continue;
        }
        let provenance = Provenance {
            kind: "extraction".into(),
            r#ref: turn_id.into(),
            thread_id: thread_id.into(),
            turn_id: turn_id.into(),
            ts_ms: now_ms,
        };
        match MemoryRecord::new(
            format!("mem-ext-{now_ms}-{i}"),
            c.layer,
            content,
            provenance,
            c.confidence,
            Status::Provisional,
        ) {
            Ok(r) => {
                record(journal, &r);
                written += 1;
                today_count += 1;
            }
            Err(e) => log::warn!("memory: skipping invalid extracted candidate: {e}"),
        }
    }
    Ok(written)
}

/// Confidence at which a Provisional record self-promotes during a
/// consolidation pass (mem-006; mem-007 adds user/N-source promotion).
const CONSOLIDATION_PROMOTE_CONFIDENCE: f32 = 0.75;

/// Hard bound on promotions per consolidation pass (ADR-0014 caps ≤100/pass).
const MAX_PROMOTIONS_PER_PASS: usize = 100;

/// One consolidation pass (mem-006, ADR-0014): folds the journal, promotes
/// Provisional records at [`CONSOLIDATION_PROMOTE_CONFIDENCE`] or higher, and
/// supersedes duplicate contents within a layer (keeping the highest-confidence
/// live record). Corrections are follow-up `memory_recorded` events with the
/// same id — last-line-wins fold makes that the whole mechanism. Rebuilds the
/// per-layer views from the re-folded journal; returns (promoted, superseded).
pub fn consolidate(
    journal: &Mutex<Journal>,
    store: &MemoryStore,
) -> Result<(usize, usize), String> {
    let path = journal.lock().unwrap().path().to_path_buf();
    let mut view = MemoryView::fold(&path)?;
    let mut promoted = 0usize;
    let mut superseded = 0usize;
    let mut changed_ids: HashSet<String> = HashSet::new();

    // Promotion: insertion order, capped per pass.
    for r in view.records.iter_mut() {
        if promoted >= MAX_PROMOTIONS_PER_PASS {
            break;
        }
        if r.status == Status::Provisional && r.confidence >= CONSOLIDATION_PROMOTE_CONFIDENCE {
            r.status = Status::Promoted;
            changed_ids.insert(r.id.clone());
            promoted += 1;
        }
    }

    // Dedup: per (layer, normalized content), keep the first-highest-confidence
    // live record; every other live duplicate points at it via superseded_by.
    let mut groups: HashMap<(Layer, String), Vec<usize>> = HashMap::new();
    for (i, r) in view.records.iter().enumerate() {
        if r.status == Status::Promoted && r.superseded_by.is_none() {
            groups
                .entry((r.layer, r.content.trim().to_lowercase()))
                .or_default()
                .push(i);
        }
    }
    for (_, idxs) in groups {
        if idxs.len() < 2 {
            continue;
        }
        let mut keep = idxs[0];
        for &i in &idxs[1..] {
            if view.records[i].confidence > view.records[keep].confidence {
                keep = i;
            }
        }
        for &i in idxs.iter().filter(|&&i| i != keep) {
            view.records[i].superseded_by = Some(view.records[keep].id.clone());
            changed_ids.insert(view.records[i].id.clone());
            superseded += 1;
        }
    }

    // Journal is truth: append the follow-up lines, then rebuild the views
    // from the re-folded journal so the caches can never drift from it.
    for r in view.records.iter() {
        if changed_ids.contains(&r.id) {
            record(journal, r);
        }
    }
    let fresh = MemoryView::fold(&path)?;
    for layer in [Layer::Project, Layer::Semantic, Layer::Episodic] {
        let records: Vec<MemoryRecord> = fresh
            .records
            .iter()
            .filter(|r| r.layer == layer)
            .cloned()
            .collect();
        store.write_layer_view(layer, &records)?;
    }
    Ok((promoted, superseded))
}

#[cfg(test)]
mod memory_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(tag: &str) -> PathBuf {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "z-memory-test-{}-{tag}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    fn prov(ref_: &str) -> Provenance {
        Provenance {
            kind: "user".into(),
            r#ref: ref_.into(),
            thread_id: "thread-1".into(),
            turn_id: "turn-1".into(),
            ts_ms: 1_770_000_000_000,
        }
    }

    fn rec(id: &str, status: Status) -> MemoryRecord {
        MemoryRecord::new(
            id,
            Layer::Project,
            format!("content of {id}"),
            prov("msg-1"),
            0.9,
            status,
        )
        .expect("valid record")
    }

    #[test]
    fn new_rejects_empty_provenance_and_out_of_range_confidence() {
        let mut p = prov("msg-1");
        p.r#ref = "  ".into();
        assert!(
            MemoryRecord::new("m", Layer::Project, "c", p.clone(), 0.5, Status::Promoted).is_err()
        );
        p.r#ref = "ok".into();
        p.kind = "".into();
        assert!(MemoryRecord::new("m", Layer::Project, "c", p, 0.5, Status::Promoted).is_err());
        for bad in [-0.01f32, 1.01, f32::NAN] {
            assert!(
                MemoryRecord::new("m", Layer::Project, "c", prov("r"), bad, Status::Promoted)
                    .is_err(),
                "confidence {bad} must be rejected"
            );
        }
        // Boundaries inclusive.
        for good in [0.0f32, 1.0] {
            let r = MemoryRecord::new(
                "m",
                Layer::Semantic,
                "c",
                prov("r"),
                good,
                Status::Provisional,
            )
            .expect("boundary confidence valid");
            assert_eq!(r.superseded_by, None);
        }
    }

    #[test]
    fn recorded_memories_fold_back_identically_with_wire_kind() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let a = rec("mem-a", Status::Promoted);
        let b = MemoryRecord::new(
            "mem-b",
            Layer::Episodic,
            "decided to use JSONL",
            prov("call-7"),
            0.4,
            Status::Provisional,
        )
        .expect("valid");
        record(&journal, &a);
        record(&journal, &b);
        drop(journal); // release file handle before asserting on-disk state

        // Wire format: snake_case kind string on the raw line.
        let raw = std::fs::read_to_string(&path).expect("read raw");
        assert!(raw.contains("\"kind\":\"memory_recorded\""), "raw: {raw}");

        let view = MemoryView::fold(&path).expect("fold");
        assert_eq!(view.records, vec![a, b]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fold_is_last_line_wins_per_id_and_live_excludes_superseded_and_provisional() {
        let dir = temp_dir("live");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        // Chain: mem-a promoted, later superseded by promoted mem-b via an
        // updated line; mem-c lives untouched; mem-d is provisional.
        let mut a = rec("mem-a", Status::Promoted);
        record(&journal, &a);
        let b = rec("mem-b", Status::Promoted);
        record(&journal, &b);
        record(&journal, &rec("mem-c", Status::Promoted));
        record(&journal, &rec("mem-d", Status::Provisional));
        a.superseded_by = Some("mem-b".into());
        record(&journal, &a); // backfilled predecessor line wins by id
        drop(journal);

        let view = MemoryView::fold(&path).expect("fold");
        assert_eq!(view.records.len(), 4, "one line per id after compaction");
        assert_eq!(view.records[0].superseded_by.as_deref(), Some("mem-b"));

        let live_ids: Vec<&str> = view.live().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(
            live_ids,
            vec!["mem-b", "mem-c"],
            "superseded + provisional excluded"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_memory_payload_fails_loud() {
        let dir = temp_dir("corrupt");
        let mut j = Journal::open(&dir, "runtime").expect("open");
        j.append(RecordDraft::new(
            JournalKind::MemoryRecorded,
            None,
            serde_json::json!({"id": "mem-x"}), // missing required fields
        ))
        .expect("append");
        drop(j);
        let err =
            MemoryView::fold(&dir.join("runtime.jsonl")).expect_err("bad payload must fail loud");
        assert!(err.contains("bad memory payload"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn layer_view_write_read_round_trip_per_layer_file() {
        let data_dir = temp_dir("store");
        let store = MemoryStore::open(&data_dir);
        assert_eq!(store.dir, data_dir.join("memory"));

        let project = vec![rec("mem-p", Status::Promoted)];
        let semantic = vec![MemoryRecord::new(
            "mem-s",
            Layer::Semantic,
            "rust ownership rules",
            prov("pass-3"),
            0.6,
            Status::Provisional,
        )
        .expect("valid")];
        store
            .write_layer_view(Layer::Project, &project)
            .expect("write project");
        store
            .write_layer_view(Layer::Semantic, &semantic)
            .expect("write semantic");

        // One file per layer under data/memory/.
        assert!(data_dir.join("memory").join("project.jsonl").is_file());
        assert!(data_dir.join("memory").join("semantic.jsonl").is_file());

        assert_eq!(
            store.read_layer(Layer::Project).expect("read project"),
            project
        );
        assert_eq!(
            store.read_layer(Layer::Semantic).expect("read semantic"),
            semantic
        );
        // Episodic was never written: empty, not an error.
        assert!(store
            .read_layer(Layer::Episodic)
            .expect("read episodic")
            .is_empty());

        // Rewrite replaces the view atomically (old-or-new, never appended).
        store
            .write_layer_view(Layer::Project, &[])
            .expect("rewrite");
        assert!(store
            .read_layer(Layer::Project)
            .expect("re-read")
            .is_empty());
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    #[test]
    fn read_layer_skips_corrupt_lines_instead_of_failing() {
        let data_dir = temp_dir("tolerant");
        let store = MemoryStore::open(&data_dir);
        store
            .write_layer_view(Layer::Episodic, &[rec("mem-e", Status::Promoted)])
            .expect("seed");
        let path = data_dir.join("memory").join("episodic.jsonl");
        let seeded = std::fs::read_to_string(&path).expect("read seed");
        std::fs::write(&path, format!("not json\n{seeded}\n{{broken\n")).expect("inject junk");

        let records = store.read_layer(Layer::Episodic).expect("skips survive");
        assert_eq!(records, vec![rec("mem-e", Status::Promoted)]);
        let _ = std::fs::remove_dir_all(&data_dir);
    }

    fn cand(content: &str, layer: Layer, confidence: f32) {
        let got = extract_candidates(content);
        assert_eq!(got.len(), 1, "{content:?}");
        assert_eq!(got[0].layer, layer);
        assert_eq!(got[0].confidence, confidence);
        assert_eq!(got[0].content.trim(), content.trim());
    }

    #[test]
    fn marker_phrases_map_to_project_and_definitions_to_semantic() {
        cand("Remember that the build uses pnpm", Layer::Project, 0.6);
        cand("note that the cache is cold", Layer::Project, 0.6);
        cand("keep in mind the deadline is Friday", Layer::Project, 0.6);
        cand("the user prefers dark themes", Layer::Project, 0.6);
        cand("always use rustfmt before committing", Layer::Project, 0.6);
        cand("never use unwrap in library code", Layer::Project, 0.6);
        cand("A mutex means exclusive access", Layer::Semantic, 0.5);
        cand("A semaphore is a counting lock", Layer::Semantic, 0.5);
        // Marker beats definition when both match.
        assert_eq!(
            extract_candidates("Note that a token bucket is a rate limiter")[0].layer,
            Layer::Project
        );
    }

    #[test]
    fn identical_candidate_contents_dedup() {
        let text = "Remember that deploys happen on Tuesday. \
                    Remember that deploys happen on tuesday.";
        let cands = extract_candidates(text);
        assert_eq!(cands.len(), 1, "case-insensitive dedup: {cands:?}");
    }

    #[test]
    fn extraction_is_capped_at_twenty_per_pass() {
        let text: String = (0..50)
            .map(|i| format!("Remember that fact number {i} is true."))
            .collect::<Vec<_>>()
            .join(" ");
        assert_eq!(extract_candidates(&text).len(), MAX_CANDIDATES_PER_PASS);
    }

    #[test]
    fn benign_text_yields_no_candidates() {
        for benign in [
            "The weather looks nice today",
            "I fixed the bug and ran the test suite twice",
            "",
            "Here is your file listing",
        ] {
            assert!(extract_candidates(benign).is_empty(), "{benign:?}");
        }
    }

    #[test]
    fn promoted_extraction_records_are_provisional_in_the_journal_view() {
        let dir = temp_dir("extract-promote");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let candidates = vec![
            ExtractedCandidate {
                content: "Always use pnpm".into(),
                layer: Layer::Project,
                confidence: 0.6,
            },
            ExtractedCandidate {
                content: "A mutex means exclusive access".into(),
                layer: Layer::Semantic,
                confidence: 0.5,
            },
            // Duplicate of what is already stored below -> skipped.
            ExtractedCandidate {
                content: "Always use PNPM".into(),
                layer: Layer::Project,
                confidence: 0.6,
            },
        ];
        let store = MemoryStore::open(&dir);
        store
            .write_layer_view(Layer::Project, &[rec("mem-seed", Status::Promoted)])
            .expect("seed view");

        let written =
            promote_candidates(&journal, &dir, &candidates, "thread-1", "turn-9").expect("promote");
        assert_eq!(written, 2);
        drop(journal); // release handle before folding

        let view = MemoryView::fold(&path).expect("fold");
        assert_eq!(view.records.len(), 2, "duplicate content skipped");
        for r in &view.records {
            assert_eq!(r.status, Status::Provisional, "extraction never promotes");
            assert_eq!(r.provenance.kind, "extraction");
            assert_eq!(r.provenance.r#ref, "turn-9");
            assert_eq!(r.provenance.turn_id, "turn-9");
            assert_eq!(r.provenance.thread_id, "thread-1");
            assert!(r.provenance.ts_ms > 0);
        }
        assert_eq!(view.records[0].layer, Layer::Project);
        assert_eq!(view.records[0].content, "Always use pnpm");
        assert_eq!(view.records[1].layer, Layer::Semantic);
        // Provisional records are never live (D4 predicate holds).
        assert!(view.live().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn rec_at(id: &str, ts_ms: u128) -> MemoryRecord {
        let mut p = prov("msg-1");
        p.ts_ms = ts_ms;
        MemoryRecord::new(
            id,
            Layer::Project,
            format!("content of {id}"),
            p,
            0.9,
            Status::Promoted,
        )
        .expect("valid record")
    }

    fn now_ms() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    }

    #[test]
    fn daily_cap_admits_everything_under_the_budget() {
        let dir = temp_dir("daily-cap-under");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let candidates: Vec<ExtractedCandidate> = (0..DAILY_RECORD_CAP)
            .map(|i| ExtractedCandidate {
                content: format!("unique fact number {i}"),
                layer: Layer::Project,
                confidence: 0.6,
            })
            .collect();

        let written =
            promote_candidates(&journal, &dir, &candidates, "thread-1", "turn-1").expect("promote");
        assert_eq!(written, DAILY_RECORD_CAP, "under-cap pass writes everything");
        assert_eq!(count_today(&journal, now_ms()), DAILY_RECORD_CAP);
        drop(journal);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn full_daily_quota_blocks_next_pass_as_partial_success() {
        let dir = temp_dir("daily-cap-blocked");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let now = now_ms();
        for i in 0..DAILY_RECORD_CAP {
            record(&journal, &rec_at(&format!("mem-seed-{i}"), now));
        }

        let candidates: Vec<ExtractedCandidate> = (0..5)
            .map(|i| ExtractedCandidate {
                content: format!("blocked fact {i}"),
                layer: Layer::Project,
                confidence: 0.6,
            })
            .collect();
        let written =
            promote_candidates(&journal, &dir, &candidates, "thread-1", "turn-1").expect("promote");
        assert_eq!(written, 0, "quota exhausted: stop writing, report partial");
        drop(journal);

        let view = MemoryView::fold(&path).expect("fold");
        assert_eq!(view.records.len(), DAILY_RECORD_CAP, "nothing extra landed");
        assert!(!view.records.iter().any(|r| r.id.starts_with("mem-ext-")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn yesterdays_records_do_not_count_against_todays_cap() {
        const DAY_MS: u128 = 86_400_000;
        let dir = temp_dir("daily-cap-yesterday");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let now = now_ms();
        let yesterday = now - DAY_MS;
        for i in 0..DAILY_RECORD_CAP {
            record(&journal, &rec_at(&format!("mem-old-{i}"), yesterday));
        }
        assert_eq!(count_today(&journal, yesterday), DAILY_RECORD_CAP);
        assert_eq!(count_today(&journal, now), 0, "UTC day boundary respected");

        let candidates: Vec<ExtractedCandidate> = (0..5)
            .map(|i| ExtractedCandidate {
                content: format!("fresh fact {i}"),
                layer: Layer::Project,
                confidence: 0.6,
            })
            .collect();
        let written =
            promote_candidates(&journal, &dir, &candidates, "thread-1", "turn-1").expect("promote");
        assert_eq!(written, 5, "yesterday's quota does not block today");

        let view = MemoryView::fold(&path).expect("fold");
        assert_eq!(view.records.len(), DAILY_RECORD_CAP + 5);
        drop(journal);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn rec_conf(id: &str, content: &str, confidence: f32, status: Status) -> MemoryRecord {
        MemoryRecord::new(
            id,
            Layer::Project,
            content,
            prov("msg-1"),
            confidence,
            status,
        )
        .expect("valid record")
    }

    #[test]
    fn consolidate_promotes_high_confidence_and_leaves_low_provisional() {
        let dir = temp_dir("consolidate-promote");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let store = MemoryStore::open(&dir);
        record(
            &journal,
            &rec_conf(
                "mem-hi",
                "the deploy target is staging",
                0.8,
                Status::Provisional,
            ),
        );
        record(
            &journal,
            &rec_conf(
                "mem-lo",
                "the cache is cold today",
                0.5,
                Status::Provisional,
            ),
        );

        let (promoted, superseded) = consolidate(&journal, &store).expect("pass");
        assert_eq!((promoted, superseded), (1, 0));

        let view = MemoryView::fold(&path).expect("fold");
        assert_eq!(view.live().len(), 1, "only the promoted record is live");
        assert_eq!(view.records[0].id, "mem-hi");
        assert_eq!(view.records[0].status, Status::Promoted);
        assert_eq!(view.records[1].id, "mem-lo");
        assert_eq!(
            view.records[1].status,
            Status::Provisional,
            "low-confidence stays provisional"
        );
        // Layer view was rebuilt from the folded journal.
        let project = store.read_layer(Layer::Project).expect("read project");
        assert_eq!(
            project
                .iter()
                .filter(|r| r.status == Status::Promoted)
                .count(),
            1,
            "{project:?}"
        );
        drop(journal);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn consolidate_supersedes_duplicate_contents_keeping_highest_confidence() {
        let dir = temp_dir("consolidate-dedup");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let store = MemoryStore::open(&dir);
        record(
            &journal,
            &rec_conf("mem-a", "deploys happen on Tuesday", 0.6, Status::Promoted),
        );
        record(
            &journal,
            &rec_conf("mem-b", "Deploys happen on Tuesday ", 0.9, Status::Promoted),
        );
        record(
            &journal,
            &rec_conf("mem-c", "unique fact", 0.9, Status::Promoted),
        );
        // Same content in another layer must NOT be treated as a duplicate.
        record(
            &journal,
            &MemoryRecord::new(
                "mem-d",
                Layer::Semantic,
                "deploys happen on Tuesday",
                prov("msg-2"),
                0.9,
                Status::Promoted,
            )
            .expect("valid"),
        );

        let (promoted, superseded) = consolidate(&journal, &store).expect("pass");
        assert_eq!((promoted, superseded), (0, 1));

        let view = MemoryView::fold(&dir.join("runtime.jsonl")).expect("fold");
        let live_ids: Vec<&str> = view.live().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(live_ids, vec!["mem-b", "mem-c", "mem-d"]);
        drop(journal);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn consolidate_caps_promotions_at_one_hundred_per_pass() {
        let dir = temp_dir("consolidate-cap");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let store = MemoryStore::open(&dir);
        for i in 0..150 {
            record(
                &journal,
                &rec_conf(
                    &format!("mem-{i}"),
                    &format!("fact number {i}"),
                    0.9,
                    Status::Provisional,
                ),
            );
        }

        let (promoted, superseded) = consolidate(&journal, &store).expect("pass");
        assert_eq!((promoted, superseded), (100, 0));

        let view = MemoryView::fold(&dir.join("runtime.jsonl")).expect("fold");
        assert_eq!(view.live().len(), 100, "cap enforced exactly");
        drop(journal);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn correct_supersedes_original_and_lands_promoted_replacement() {
        let dir = temp_dir("correct");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        record(
            &journal,
            &rec_conf("mem-a", "deploys happen on Tuesday", 0.6, Status::Promoted),
        );

        let new_id =
            correct(&journal, "mem-a", "deploys happen on Wednesday", 1.5, "thread-9", "turn-9")
                .expect("correction");
        assert!(new_id.starts_with("mem-a-c"), "{new_id}");
        drop(journal); // release handle before folding

        let view = MemoryView::fold(&path).expect("fold");
        assert_eq!(view.records.len(), 2);
        let orig = view.records.iter().find(|r| r.id == "mem-a").unwrap();
        assert_eq!(orig.superseded_by.as_deref(), Some(new_id.as_str()));
        let rep = view.records.iter().find(|r| r.id == new_id).expect("replacement");
        assert_eq!(rep.status, Status::Promoted);
        assert_eq!(rep.layer, Layer::Project, "same layer as original");
        assert_eq!(rep.content, "deploys happen on Wednesday");
        assert_eq!(rep.confidence, 1.0, "clamped into [0,1]");
        assert_eq!(rep.provenance.kind, "user");
        assert_eq!(rep.provenance.thread_id, "thread-9");
        assert_eq!(rep.provenance.turn_id, "turn-9");

        // D4 predicate: only the replacement is live.
        let live_ids: Vec<&str> = view.live().iter().map(|r| r.id.as_str()).collect();
        assert_eq!(live_ids, vec![new_id.as_str()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn correct_errors_on_unknown_and_already_superseded_targets() {
        let dir = temp_dir("correct-errors");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        record(&journal, &rec_conf("mem-a", "original fact", 0.6, Status::Promoted));

        assert!(
            correct(&journal, "mem-nope", "x", 0.5, "t", "u")
                .err()
                .unwrap()
                .contains("unknown"),
            "missing id must error"
        );

        correct(&journal, "mem-a", "fixed fact", 0.7, "t", "u").expect("first correction");
        let err = correct(&journal, "mem-a", "again", 0.5, "t", "u").err().unwrap();
        assert!(err.contains("already superseded"), "{err}");
        drop(journal);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dependents_of_walks_the_chain_transitively_nearest_first() {
        let mut view = MemoryView::default();
        let mut a = rec("mem-a", Status::Promoted);
        let mut b = rec("mem-b", Status::Promoted);
        let c = rec("mem-c", Status::Promoted);
        a.superseded_by = Some("mem-b".into());
        b.superseded_by = Some("mem-c".into());
        view.records.extend([a, b, c]);

        assert_eq!(
            dependents_of(&view, "mem-c"),
            vec!["mem-b".to_string(), "mem-a".to_string()],
            "whole chain, nearest first"
        );
        assert_eq!(dependents_of(&view, "mem-b"), vec!["mem-a".to_string()]);
        assert!(dependents_of(&view, "mem-a").is_empty(), "tip has no dependents");
        assert!(dependents_of(&view, "mem-absent").is_empty());
    }

    #[test]
    fn retrieve_ranks_matching_terms_higher_and_respects_cap() {
        let mut view = MemoryView::default();
        view.records.push(rec_conf("mem-a", "deploys happen on Tuesday", 0.9, Status::Promoted));
        view.records.push(rec_conf("mem-b", "the cache is cold", 0.9, Status::Promoted));
        view.records.push(rec_conf("mem-c", "deploys on friday", 0.2, Status::Promoted));
        // Never retrieved: Provisional is not live (D4).
        view.records.push(rec_conf("mem-p", "deploys deploys deploys", 1.0, Status::Provisional));

        let hits = retrieve(&view, &["DEPLOYS", "tuesday"], 10);
        // mem-a matches both terms; beats the higher-confidence non-match.
        assert_eq!(hits[0].record_id, "mem-a");
        assert!((hits[0].score - (0.3 * 0.9 + 1.0)).abs() < 1e-5);
        assert!(hits[0].score > hits[1].score, "{hits:?}");
        assert!(!hits.iter().any(|h| h.record_id == "mem-p"), "{hits:?}");

        assert_eq!(retrieve(&view, &[], 2).len(), 2, "cap enforced");
    }

    #[test]
    fn retrieve_with_no_terms_ranks_all_live_records_by_confidence() {
        let mut view = MemoryView::default();
        view.records.push(rec_conf("lo", "x", 0.1, Status::Promoted));
        view.records.push(rec_conf("hi", "y", 0.9, Status::Promoted));
        view.records.push(rec_conf("mid", "z", 0.5, Status::Promoted));

        let hits = retrieve(&view, &[], 10);
        let ids: Vec<&str> = hits.iter().map(|h| h.record_id.as_str()).collect();
        assert_eq!(ids, vec!["hi", "mid", "lo"]);
        for h in &hits {
            let conf = view.records.iter().find(|r| r.id == h.record_id).unwrap().confidence;
            assert!((h.score - 0.3 * conf).abs() < 1e-5);
        }
    }

    #[test]
    fn decay_halves_at_half_life_and_zero_age_is_identity() {
        assert!((decay_confidence(0.8, 7, 7.0) - 0.4).abs() < 1e-5, "halves");
        assert!((decay_confidence(0.8, 14, 7.0) - 0.2).abs() < 1e-5, "quarters");
        assert_eq!(decay_confidence(0.8, 0, 7.0), 0.8, "zero age unchanged");
        // Degenerate half-life must not produce NaN/inf.
        assert_eq!(decay_confidence(0.8, 5, 0.0), 0.8);
    }

    #[test]
    fn decay_confidence_stays_within_unit_range() {
        for (c, age, hl) in [
            (1.0, 0, 0.001),
            (1.0, 50_000, 0.5),
            (1.0, u32::MAX, 1.0),
            (0.0, 10, 1.0),
            (0.37, 123, 456.0),
        ] {
            let d = decay_confidence(c, age, hl);
            assert!((0.0..=1.0).contains(&d), "{c}/{age}/{hl} -> {d}");
        }
    }

    #[test]
    fn retrieve_with_decay_ranks_fresh_above_old_on_equal_match() {
        const DAY_MS: u128 = 86_400_000;
        let rec_at = |id: &str, ts_ms: u128| {
            let mut p = prov("msg-1");
            p.ts_ms = ts_ms;
            MemoryRecord::new(id, Layer::Project, "deploys on tuesday", p, 0.9, Status::Promoted)
                .expect("valid record")
        };
        let mut view = MemoryView::default();
        view.records.push(rec_at("old", 0));
        view.records.push(rec_at("fresh", DAY_MS));

        let hits = retrieve_with_decay(&view, &["deploys"], 10, 2 * DAY_MS, 1.0);
        assert_eq!(hits[0].record_id, "fresh", "{hits:?}");
        // score = 0.3 * 0.9 * 0.5^(age/half_life) + 1.0 overlap.
        assert!((hits[0].score - (0.3 * 0.9 * 0.5 + 1.0)).abs() < 1e-5);
        assert!((hits[1].score - (0.3 * 0.9 * 0.25 + 1.0)).abs() < 1e-5);
        // Cap still enforced through the decay path.
        assert_eq!(retrieve_with_decay(&view, &[], 1, 2 * DAY_MS, 1.0).len(), 1);
    }
}
