//! Project memory records (mem-001..004, ADR-0014).
//!
//! Every mutation is a [`JournalKind::MemoryRecorded`] event; the per-layer
//! JSONL files under `data/memory/` are derived views ("views are caches,
//! the journal is truth") rebuilt by fold. A record is live iff it is
//! Promoted and not superseded (ADR-0014 D4) — the predicate retrieval
//! ranks within.

use crate::journal::{Journal, JournalKind, RecordDraft};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    let mut written = 0usize;
    for (i, c) in candidates.iter().enumerate() {
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
            }
            Err(e) => log::warn!("memory: skipping invalid extracted candidate: {e}"),
        }
    }
    Ok(written)
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
}
