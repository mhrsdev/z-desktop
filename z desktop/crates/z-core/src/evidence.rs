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

    /// Diff evidence from a fs_write/edit_patch result (sup-004).
    pub fn diff(thread_id: &str, turn_id: &str, ok: bool, summary: impl Into<String>) -> Evidence {
        Evidence {
            id: crate::new_id("ev"),
            kind: EvidenceKind::Diff,
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            ok,
            summary: summary.into(),
        }
    }
}

/// sup-003 (partial): a terminal_exec command that looks like a test run
/// lands [`EvidenceKind::Tests`] evidence instead of Build.
pub fn classify_command(cmd: &str) -> EvidenceKind {
    const TEST_RUNNERS: [&str; 4] = ["cargo test", "npm test", "pytest", "go test"];
    if TEST_RUNNERS.iter().any(|runner| cmd.contains(runner)) {
        EvidenceKind::Tests
    } else {
        EvidenceKind::Build
    }
}

// sup-005: a success claim found in assistant text ("tests pass", "build
// succeeds", …). Conservative phrase classes only — regex-free per house
// style; misses paraphrases by design (ADR-0016: tripwire, not lie detector).
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimSpan {
    pub text: String,
    pub kind: EvidenceKind,
}

/// Scans assistant text sentence-by-sentence for known success phrases and
/// maps them to the matching evidence kind. False-positive tolerance: only
/// exact phrase classes match, so "the tests passed last week" still links —
/// acceptable noise, measured later by sup-025.
pub fn extract_claims(text: &str) -> Vec<ClaimSpan> {
    const PATTERNS: [(&str, EvidenceKind); 8] = [
        ("tests pass", EvidenceKind::Tests),
        ("test suite passes", EvidenceKind::Tests),
        ("build succeeds", EvidenceKind::Build),
        ("build succeeded", EvidenceKind::Build),
        ("compiles successfully", EvidenceKind::Build),
        ("compiled successfully", EvidenceKind::Build),
        ("benchmark shows", EvidenceKind::Bench),
        ("no regressions", EvidenceKind::Regression),
    ];
    let mut claims = Vec::new();
    for sentence in text.split(['.', '!', '?', '\n']) {
        let lower = sentence.to_lowercase();
        // A bare "<number> ms" reads like a benchmark result (sup-025 will
        // measure whether this is too eager).
        let squeezed = lower.replace(" ms", "ms");
        let bench_ms = squeezed.split_whitespace().any(|w| {
            w.strip_suffix("ms").is_some_and(|digits| {
                !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit())
            })
        });
        let kind = PATTERNS
            .iter()
            .find(|(p, _)| lower.contains(p))
            .map(|(_, k)| *k)
            .or(bench_ms.then_some(EvidenceKind::Bench));
        if let Some(kind) = kind {
            claims.push(ClaimSpan {
                text: sentence.trim().to_string(),
                kind,
            });
        }
    }
    claims
}

/// sup-006 result: how many claims found ok same-turn evidence of their own
/// kind, and which spans went unlinked (fake-completion detector input).
#[derive(Debug, PartialEq)]
pub struct LinkReport {
    pub linked: usize,
    pub unlinked: Vec<ClaimSpan>,
}

/// Links claims to evidence: a claim is linked when any `ok == true` evidence
/// of the same kind exists. Same-turn/thread filtering happens at the call
/// site (`turn_id` is caller context) — an earlier green build never
/// whitewashes a later claim (ADR-0016 linking window).
pub fn link_claims(claims: &[ClaimSpan], evidence: &[Evidence]) -> LinkReport {
    let mut report = LinkReport {
        linked: 0,
        unlinked: Vec::new(),
    };
    for claim in claims {
        if evidence.iter().any(|e| e.kind == claim.kind && e.ok) {
            report.linked += 1;
        } else {
            report.unlinked.push(claim.clone());
        }
    }
    report
}

/// sup-007 verdict (ADR-0016): unlinked claims with ZERO ok evidence of any
/// kind this turn are the fake-completion signature. Ambiguous cases (some
/// claim linked, or some ok evidence exists) pass — strictness comes later.
#[derive(Debug, PartialEq)]
pub struct SupervisionVerdict {
    pub blocked: bool,
    pub reason: Option<String>,
}

pub fn evaluate_claims(report: &LinkReport, evidence_count_ok: usize) -> SupervisionVerdict {
    if report.unlinked.is_empty() || report.linked > 0 || evidence_count_ok > 0 {
        return SupervisionVerdict {
            blocked: false,
            reason: None,
        };
    }
    let mut kinds: Vec<String> = Vec::new();
    for claim in &report.unlinked {
        let k = format!("{:?}", claim.kind);
        if !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    SupervisionVerdict {
        blocked: true,
        reason: Some(format!(
            "claimed {} success without recorded evidence",
            kinds.join("/")
        )),
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
    fn extract_claims_maps_phrases_to_kinds() {
        let cases = [
            ("All 384 tests pass in the workspace.", EvidenceKind::Tests),
            ("The test suite passes locally.", EvidenceKind::Tests),
            ("Build succeeds on release profile.", EvidenceKind::Build),
            ("The build succeeded after the fix.", EvidenceKind::Build),
            ("It compiles successfully now.", EvidenceKind::Build),
            ("Everything compiled successfully.", EvidenceKind::Build),
            ("Benchmark shows 12 ms p50 latency.", EvidenceKind::Bench),
            ("The run finished in 250 ms.", EvidenceKind::Bench),
            ("No regressions were introduced.", EvidenceKind::Regression),
        ];
        for (text, want) in cases {
            let claims = extract_claims(text);
            assert_eq!(claims.len(), 1, "text: {text:?}");
            assert_eq!(claims[0].kind, want, "text: {text:?}");
        }
    }

    #[test]
    fn benign_text_yields_no_claims() {
        assert!(extract_claims("").is_empty());
        assert!(extract_claims("I looked at the file and thought about it.").is_empty());
        assert!(extract_claims("The build is still running; tests pending.").is_empty());
        assert!(extract_claims("Ran the tests, results below.").is_empty());
    }

    #[test]
    fn link_claims_needs_ok_evidence_of_same_kind() {
        let claims = vec![
            ClaimSpan { text: "tests pass".into(), kind: EvidenceKind::Tests },
            ClaimSpan { text: "build succeeds".into(), kind: EvidenceKind::Build },
            ClaimSpan { text: "no regressions".into(), kind: EvidenceKind::Regression },
        ];
        // ok Tests + failed Build + wrong-turn Tests: only the Tests claim links.
        let evidence = vec![
            Evidence::tests("t", "u1", 5, 0, "cargo test"),
            Evidence::build("t", "u1", Some(1), "cargo build"),
            Evidence::tests("t", "u2", 5, 0, "cargo test"), // different turn
        ];
        let report = link_claims(&claims, &evidence);
        assert_eq!(report.linked, 1);
        assert_eq!(report.unlinked.len(), 2);
        assert_eq!(report.unlinked[0].kind, EvidenceKind::Build);
        assert_eq!(report.unlinked[1].kind, EvidenceKind::Regression);
        // Same-turn ok evidence of matching kinds links everything.
        let ok_evidence = vec![
            Evidence::tests("t", "u1", 5, 0, "cargo test"),
            Evidence::build("t", "u1", Some(0), "cargo build"),
        ];
        let report = link_claims(&claims[..2], &ok_evidence);
        assert_eq!(report.linked, 2);
        assert!(report.unlinked.is_empty());
    }

    #[test]
    fn evaluate_claims_gates_only_total_unlink_with_zero_ok_evidence() {
        let claims = vec![
            ClaimSpan {
                text: "tests pass".into(),
                kind: EvidenceKind::Tests,
            },
            ClaimSpan {
                text: "build succeeds".into(),
                kind: EvidenceKind::Build,
            },
        ];
        // No claims at all -> pass.
        let empty = link_claims(&[], &[]);
        assert!(!evaluate_claims(&empty, 0).blocked);

        // Some claim linked -> pass (even with zero ok evidence counted).
        let linked = link_claims(&claims[..1], &[Evidence::tests("t", "u1", 3, 0, "cargo test")]);
        assert!(!evaluate_claims(&linked, 0).blocked);

        // Everything unlinked, zero ok evidence of any kind -> blocked, with
        // a reason naming every unlinked claim kind exactly once.
        let unlinked = link_claims(&claims, &[Evidence::build("t", "u1", Some(1), "make")]);
        let verdict = evaluate_claims(&unlinked, 0);
        assert!(verdict.blocked);
        let reason = verdict.reason.expect("blocked carries a reason");
        assert!(reason.contains("Tests"), "{reason}");
        assert!(reason.contains("Build"), "{reason}");
        assert!(reason.contains("without recorded evidence"), "{reason}");
        assert_eq!(reason.matches("Tests").count(), 1, "{reason}");

        // Unlinked but SOME ok evidence exists this turn -> ambiguous, passes.
        assert!(!evaluate_claims(&unlinked, 1).blocked);
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

    #[test]
    fn classify_command_routes_test_runners_to_tests() {
        let cases = [
            ("cargo test --workspace", EvidenceKind::Tests),
            ("cargo build --release", EvidenceKind::Build),
            ("pytest -q", EvidenceKind::Tests),
            ("npm test", EvidenceKind::Tests),
            ("go test ./...", EvidenceKind::Tests),
            ("ls -la", EvidenceKind::Build),
            ("", EvidenceKind::Build),
        ];
        for (cmd, want) in cases {
            assert_eq!(classify_command(cmd), want, "cmd: {cmd:?}");
        }
    }

    #[test]
    fn diff_and_tests_evidence_round_trip_through_the_fold() {
        let dir = temp_dir("sup-004");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let d = Evidence::diff("t", "u1", true, "wrote 42 bytes to src/lib.rs");
        record(&journal, &d);
        // sup-003 partial: Tests kind captured from an exit-code parse.
        let mut e = Evidence::build("t", "u2", Some(0), "running 353 tests");
        e.kind = classify_command("cargo test --workspace");
        assert_eq!(e.kind, EvidenceKind::Tests);
        assert!(e.ok, "exit 0 means ok regardless of kind");
        record(&journal, &e);
        drop(journal);

        let view = EvidenceView::fold(&path).expect("fold");
        assert_eq!(view.items, vec![d, e]);
        assert_eq!(view.items[0].kind, EvidenceKind::Diff);
        assert_eq!(
            view.items[0].summary,
            "wrote 42 bytes to src/lib.rs"
        );
        assert_eq!(view.items[1].kind, EvidenceKind::Tests);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
