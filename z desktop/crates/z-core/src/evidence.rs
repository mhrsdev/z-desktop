//! Supervision evidence records (sup-001/sup-002, ADR-0016).
//!
//! Evidence is captured at execution time by the system, never narrated by
//! the model: hooks append [`JournalKind::EvidenceRecorded`] events and the
//! [`EvidenceView`] fold replays them. Best-effort writes mirror
//! `Runtime::journal_record` — a journal failure is warned about and dropped,
//! it can never break the tool call that produced the evidence.

use crate::journal::{Journal, JournalKind, RecordDraft};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Mutex;

/// One captured proof record (ADR-0016 envelope; per-kind bodies land with
/// their own sup tasks — this shape covers all five kinds).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    /// The only handle UI/detectors hold ("ev-...").
    pub id: String,
    pub kind: EvidenceKind,
    pub thread_id: String,
    /// Turn whose execution produced it.
    pub turn_id: String,
    pub ok: bool,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Build,
    Tests,
    Diff,
    Bench,
    Regression,
}

impl Evidence {
    /// Build evidence from a terminal_exec outcome (`None` exit code means
    /// killed/timed-out, i.e. not ok).
    pub fn build(
        thread_id: &str,
        turn_id: &str,
        exit_code: Option<i32>,
        summary: impl Into<String>,
    ) -> Evidence {
        Evidence {
            id: crate::new_id("ev"),
            kind: EvidenceKind::Build,
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            ok: exit_code == Some(0),
            summary: summary.into(),
        }
    }

    /// Tests evidence from a runner parse.
    pub fn tests(
        thread_id: &str,
        turn_id: &str,
        passed: usize,
        failed: usize,
        summary: impl Into<String>,
    ) -> Evidence {
        Evidence {
            id: crate::new_id("ev"),
            kind: EvidenceKind::Tests,
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            ok: failed == 0,
            summary: summary.into(),
        }
    }
}

/// Best-effort append of one evidence record. Journal failures are warned
/// and dropped (same policy as every other lifecycle append).
pub fn record(journal: &Mutex<Journal>, e: &Evidence) {
    let payload = match serde_json::to_value(e) {
        Ok(v) => v,
        Err(err) => {
            log::warn!("evidence: record {} does not serialize: {err}", e.id);
            return;
        }
    };
    let draft = RecordDraft::new(
        JournalKind::EvidenceRecorded,
        Some(e.thread_id.clone()),
        payload,
    );
    if let Err(err) = journal.lock().unwrap().append(draft) {
        log::warn!("evidence: append failed: {err}");
    }
}

/// Folded state of all evidence records seen in a journal segment.
#[derive(Debug, Default, PartialEq)]
pub struct EvidenceView {
    pub items: Vec<Evidence>,
}

impl EvidenceView {
    /// Folds `evidence_recorded` events in replay order. A corrupt payload
    /// fails loud (same policy as TasksView / malformed lines).
    pub fn fold(path: &Path) -> Result<EvidenceView, String> {
        let mut bad_payload: Option<String> = None;
        let mut view = EvidenceView::default();
        crate::reducer::fold(path, (), |(), record| {
            if record.kind != JournalKind::EvidenceRecorded || bad_payload.is_some() {
                return;
            }
            match serde_json::from_value::<Evidence>(record.payload.clone()) {
                Ok(e) => view.items.push(e),
                Err(e) => {
                    bad_payload = Some(format!(
                        "reducer {}: bad evidence payload in seq {}: {e}",
                        path.display(),
                        record.seq
                    ));
                }
            }
        })?;
        match bad_payload {
            Some(e) => Err(e),
            None => Ok(view),
        }
    }
}

/// Parses the `[exit code: N]` trailer `terminal_exec` appends to its tool
/// output. Returns `None` when the marker is absent (tool-level error path).
pub(crate) fn parse_exit_code(text: &str) -> Option<i32> {
    const MARKER: &str = "[exit code: ";
    let start = text.rfind(MARKER)?;
    let tail = &text[start + MARKER.len()..];
    let end = tail.find(']')?;
    tail[..end].trim().parse().ok()
}

#[cfg(test)]
mod evidence_tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let unique = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "z-evidence-test-{}-{tag}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn recorded_evidence_folds_back_identically() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let e = Evidence::build("thread-1", "turn-1", Some(0), "cargo build");
        record(&journal, &e);
        drop(journal); // release file handle before asserting on-disk state

        // Wire format: snake_case kind string + typed evidence payload.
        let records = Journal::replay(&path).expect("replay");
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, JournalKind::EvidenceRecorded);
        assert_eq!(records[0].thread_id.as_deref(), Some("thread-1"));

        let view = EvidenceView::fold(&path).expect("fold");
        assert_eq!(view.items, vec![e]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn non_evidence_kinds_are_ignored_by_the_fold() {
        let dir = temp_dir("mixed");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        record(
            &journal,
            &Evidence::tests("t", "u1", 3, 0, "cargo test --workspace"),
        );
        journal
            .lock()
            .unwrap()
            .append(RecordDraft::new(JournalKind::TurnStarted, None, json!({})))
            .expect("append noise");
        record(&journal, &Evidence::build("t", "u2", Some(2), "make"));
        drop(journal);

        let view = EvidenceView::fold(&path).expect("fold");
        assert_eq!(view.items.len(), 2);
        assert_eq!(view.items[0].kind, EvidenceKind::Tests);
        assert!(view.items[0].ok);
        assert_eq!(view.items[1].kind, EvidenceKind::Build);
        assert!(!view.items[1].ok);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn corrupt_evidence_payload_fails_loud() {
        let dir = temp_dir("corrupt");
        let mut j = Journal::open(&dir, "runtime").expect("open");
        j.append(RecordDraft::new(
            JournalKind::EvidenceRecorded,
            None,
            json!({"id": "ev-x"}), // missing required fields
        ))
        .expect("append");
        drop(j);
        let err =
            EvidenceView::fold(&dir.join("runtime.jsonl")).expect_err("bad payload must fail loud");
        assert!(err.contains("bad evidence payload"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn helpers_derive_ok_from_outcomes() {
        assert!(Evidence::build("t", "u", Some(0), "").ok);
        assert!(!Evidence::build("t", "u", Some(1), "").ok);
        assert!(!Evidence::build("t", "u", None, "").ok, "killed/timeout");
        assert!(Evidence::tests("t", "u", 5, 0, "").ok);
        assert!(!Evidence::tests("t", "u", 344, 1, "").ok);
    }

    #[test]
    fn exit_code_marker_parses_and_tolerates_absence() {
        let text = "[stderr] boom\n[exit code: 101]";
        assert_eq!(parse_exit_code(text), Some(101));
        assert_eq!(parse_exit_code("[exit code: -1]"), Some(-1));
        assert_eq!(parse_exit_code("terminal_exec failed: no sandbox"), None);
    }
}
