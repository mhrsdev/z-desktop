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

    /// Bench evidence from a timing capture (sup-021). Always ok — a recorded
    /// duration is a measurement, not a pass/fail verdict.
    pub fn bench(thread_id: &str, turn_id: &str, name: &str, value_ms: u64) -> Evidence {
        Evidence {
            id: crate::new_id("ev"),
            kind: EvidenceKind::Bench,
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            ok: true,
            summary: format!("{name}: {value_ms}ms"),
        }
    }

    /// Regression-test linkage evidence (sup-022). `ok == passed`; a failed
    /// regression test is still recorded — it's a measurement of the suite.
    pub fn regression(thread_id: &str, turn_id: &str, test_name: &str, passed: bool) -> Evidence {
        Evidence {
            id: crate::new_id("ev"),
            kind: EvidenceKind::Regression,
            thread_id: thread_id.to_string(),
            turn_id: turn_id.to_string(),
            ok: passed,
            summary: format!(
                "regression: {test_name} {}",
                if passed { "PASS" } else { "FAIL" }
            ),
        }
    }

    /// Convenience: one record per `(test_name, passed)` pair, in order.
    pub fn regression_batch(
        thread_id: &str,
        turn_id: &str,
        results: &[(&str, bool)],
    ) -> Vec<Evidence> {
        results
            .iter()
            .map(|(name, passed)| Self::regression(thread_id, turn_id, name, *passed))
            .collect()
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

// ---------------------------------------------------------------------------
// sup-009/010/011 (ADR-0016): observability detectors — warn-first, none of
// these change gating on their own.
// ---------------------------------------------------------------------------

/// Shared shape of sup-009/sup-010: a success claim of `kind` with ZERO
/// same-kind evidence recorded this turn — the run never happened.
fn claimed_without_execution(
    claims: &[ClaimSpan],
    evidence: &[Evidence],
    kind: EvidenceKind,
) -> bool {
    claims.iter().any(|c| c.kind == kind) && !evidence.iter().any(|e| e.kind == kind)
}

/// sup-009: Tests success claimed but no Tests-kind evidence at all was
/// recorded this turn. Failed Tests evidence still counts as "executed"
/// (narrating over it is [`detect_ignored_failures`]' job).
pub fn detect_unexecuted_tests(claims: &[ClaimSpan], evidence: &[Evidence]) -> bool {
    claimed_without_execution(claims, evidence, EvidenceKind::Tests)
}

/// sup-010: same rule for Build claims vs Build evidence.
pub fn detect_unexecuted_build(claims: &[ClaimSpan], evidence: &[Evidence]) -> bool {
    claimed_without_execution(claims, evidence, EvidenceKind::Build)
}

/// sup-011: Tests-kind evidence exists with `ok == false` AND the turn's
/// final text still contains a success phrase — the failure was narrated
/// over instead of reported.
pub fn detect_ignored_failures(evidence: &[Evidence], final_text: &str) -> bool {
    evidence
        .iter()
        .any(|e| e.kind == EvidenceKind::Tests && !e.ok)
        && !extract_claims(final_text).is_empty()
}

/// sup-014: placeholder-code detector (diff scan). Fires when written content
/// contains ANY classic placeholder marker — deliberately any-single-marker
/// (documented tradeoff: legitimate TODO comments will trip it; ADR-0016 says
/// these are tripwires, not verdicts, and callers decide severity).
pub fn detect_placeholder_code(new_text: &str) -> bool {
    const MARKERS: [&str; 7] = [
        "todo: implement",
        "fixme",
        "not implemented",
        "unimplemented!",
        "todo!()",
        "<insert",
        "...rest of implementation",
    ];
    scan_markers(new_text, &MARKERS)
}

/// sup-015: mock-in-prod detector. Fires on mock/stub/dummy identifier
/// markers in written content. Approximation by design: no `#[cfg(test)]`
/// scoping, just the identifier list — same warn-first policy as above.
pub fn detect_mock_in_prod(new_text: &str) -> bool {
    const MARKERS: [&str; 4] = ["mock_response", "dummy_data", "stub_impl", "fake_"];
    scan_markers(new_text, &MARKERS)
}

/// sup-016: requirement-skew detector. Fires when the request pinned an exact
/// item — a quoted string (`"..."`) or an ALL-CAPS word of >=4 chars — that
/// never appears in the delivered text. ponytail: bare substring containment,
/// no stemming/synonyms/paraphrase handling (tripwire, not verdict).
pub fn detect_requirement_skew(requested: &str, delivered: &str) -> bool {
    let lower_delivered = delivered.to_lowercase();
    let mut demanded: Vec<String> = Vec::new();
    // Quoted strings, consumed pairwise left-to-right.
    let mut rest = requested;
    while let Some(open) = rest.find('"') {
        let inner = &rest[open + 1..];
        match inner.find('"') {
            Some(close) => {
                if !inner[..close].trim().is_empty() {
                    demanded.push(inner[..close].to_lowercase());
                }
                rest = &inner[close + 1..];
            }
            None => break,
        }
    }
    // ALL-CAPS runs of >=4 chars (digits/punctuation act as separators, so
    // "MUST," and "(EXACT)" qualify but "Api"/"ALL4" never do).
    demanded.extend(
        requested
            .split(|c: char| !c.is_ascii_uppercase())
            .filter(|token| token.len() >= 4)
            .map(|token| token.to_lowercase()),
    );
    demanded
        .iter()
        .any(|token| !lower_delivered.contains(token))
}

/// Case-insensitive any-marker match at identifier boundaries: neither side
/// may glue to `[A-Za-z0-9_]`, except prefix-style markers (trailing `_`,
/// e.g. `fake_`) which only need a clean left edge (`fake_data` yes,
/// `refake_data` no).
fn scan_markers(text: &str, markers: &[&str]) -> bool {
    let lowered = text.to_lowercase();
    markers.iter().any(|m| {
        lowered.match_indices(m).any(|(i, found)| {
            let glued = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            let left_ok = !glued(lowered[..i].chars().next_back());
            let right_ok =
                found.ends_with('_') || !glued(lowered[i + found.len()..].chars().next());
            left_ok && right_ok
        })
    })
}

/// sup-012: stricter sibling of sup-007's gate. Fires when at least one
/// success claim exists, EVERY claim went unlinked (no ok same-kind evidence),
/// and the final text declares completion — even when unrelated ok evidence
/// exists (sup-007 passes those as ambiguous; this doesn't).
pub fn detect_fake_completion(
    claims: &[ClaimSpan],
    evidence: &[Evidence],
    final_text: &str,
) -> bool {
    const MARKERS: [&str; 4] = ["done", "complete", "finished", "all set"];
    !claims.is_empty()
        && link_claims(claims, evidence).unlinked.len() == claims.len()
        && scan_markers(final_text, &MARKERS)
}

/// sup-013: one item of a caller-supplied completion checklist — how many
/// evidence records of `kind` the task demanded.
#[derive(Debug, Clone, PartialEq)]
pub struct ChecklistExpectation {
    pub kind: EvidenceKind,
    pub count: usize,
}

/// sup-013 premature-stop detector: fires when ANY checklist expectation's
/// same-kind evidence count falls short (e.g. expected 1 test run, zero
/// found) — the turn stopped before covering its checklist. Coverage check
/// against an explicit checklist only; claims are not consulted.
pub fn detect_premature_stop(
    _claims: &[ClaimSpan],
    evidence: &[Evidence],
    expectations: &[ChecklistExpectation],
) -> bool {
    expectations
        .iter()
        .any(|exp| evidence.iter().filter(|e| e.kind == exp.kind).count() < exp.count)
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
    fn unexecuted_detectors_fire_only_on_claim_without_same_kind_evidence() {
        let tests_claim = vec![ClaimSpan {
            text: "tests pass".into(),
            kind: EvidenceKind::Tests,
        }];
        let build_claim = vec![ClaimSpan {
            text: "build succeeds".into(),
            kind: EvidenceKind::Build,
        }];
        // Claim + zero evidence of its kind -> fires.
        assert!(detect_unexecuted_tests(&tests_claim, &[]));
        assert!(detect_unexecuted_build(&build_claim, &[]));
        // FAILED evidence of the kind still means "executed" — no fire
        // (narrating over a failure is sup-011's job).
        assert!(!detect_unexecuted_tests(
            &tests_claim,
            &[Evidence::tests("t", "u", 1, 1, "cargo test")]
        ));
        assert!(!detect_unexecuted_build(
            &build_claim,
            &[Evidence::build("t", "u", Some(1), "cargo build")]
        ));
        // Wrong-kind claim never fires; clean turns with no claims stay silent.
        assert!(!detect_unexecuted_build(&tests_claim, &[]));
        assert!(!detect_unexecuted_tests(&build_claim, &[]));
        assert!(!detect_unexecuted_tests(&[], &[]));
        assert!(!detect_unexecuted_build(&[], &[]));
    }

    #[test]
    fn ignored_failures_need_failing_tests_and_a_success_phrase() {
        let failed = vec![Evidence::tests("t", "u", 1, 1, "cargo test")];
        assert!(detect_ignored_failures(&failed, "All good, tests pass."));
        // Passing tests + success phrase -> fine.
        let passed = vec![Evidence::tests("t", "u", 5, 0, "cargo test")];
        assert!(!detect_ignored_failures(&passed, "tests pass"));
        // Failing tests + honest text -> no fire.
        assert!(!detect_ignored_failures(
            &failed,
            "1 test failed; fixing next."
        ));
        // Clean turn (no evidence, no phrase) -> no false positive.
        assert!(!detect_ignored_failures(&[], ""));
    }

    #[test]
    fn placeholder_detector_fires_on_each_marker_class() {
        assert!(detect_placeholder_code("// TODO: implement pagination"));
        assert!(detect_placeholder_code("// FIXME: racy under load"));
        assert!(detect_placeholder_code("return not implemented yet"));
        assert!(detect_placeholder_code("fn f() { unimplemented!() }"));
        assert!(detect_placeholder_code("let x = todo!();"));
        assert!(detect_placeholder_code("<insert your API key here>"));
        assert!(detect_placeholder_code("// ...rest of implementation"));
    }

    #[test]
    fn clean_content_does_not_trip_placeholder_detector() {
        assert!(!detect_placeholder_code("fn main() { println!(\"hi\"); }"));
        assert!(!detect_placeholder_code(""), "empty write");
        assert!(
            !detect_placeholder_code("todo app readme"),
            "plain 'todo' prose is not a marker"
        );
    }

    #[test]
    fn mock_detector_fires_on_identifier_markers_at_boundaries() {
        assert!(detect_mock_in_prod("let r = mock_response();"));
        assert!(detect_mock_in_prod("const D: u8 = dummy_data;"));
        assert!(detect_mock_in_prod("fn stub_impl() {}"));
        assert!(detect_mock_in_prod("fn fake_user() {}"), "fake_ prefix");
        // Boundary rules keep prose and glued identifiers quiet.
        assert!(
            !detect_mock_in_prod("// a mock comment"),
            "bare 'mock' is not a marker"
        );
        assert!(
            !detect_mock_in_prod("counterfeit_fake_thing"),
            "glued left edge"
        );
        assert!(!detect_mock_in_prod("fn real_response() {}"));
        assert!(!detect_mock_in_prod(""));
    }

    #[test]
    fn requirement_skew_fires_on_missing_quoted_token() {
        assert!(detect_requirement_skew(
            "must include \"dark mode\" support",
            "we added light mode only"
        ));
    }

    #[test]
    fn requirement_skew_quiet_when_all_quoted_tokens_present() {
        assert!(!detect_requirement_skew(
            "must include \"dark mode\" and \"sync\" exactly",
            "Dark Mode was added, sync included."
        ));
    }

    #[test]
    fn requirement_skew_quiet_without_any_exact_tokens() {
        assert!(!detect_requirement_skew(
            "please make it nicer please",
            "done, it is nicer now"
        ));
        assert!(!detect_requirement_skew("", ""));
    }

    #[test]
    fn requirement_skew_fires_on_missing_caps_word() {
        assert!(detect_requirement_skew(
            "output must be JSON only",
            "here is some yaml instead"
        ));
        // Present caps word stays quiet; short acronyms (<4 chars) never count.
        assert!(!detect_requirement_skew(
            "emit JSON output via API",
            "the JSON output follows"
        ));
    }

    #[test]
    fn fake_completion_needs_claims_all_unlinked_and_a_marker() {
        let claims = vec![
            ClaimSpan { text: "tests pass".into(), kind: EvidenceKind::Tests },
            ClaimSpan { text: "build succeeds".into(), kind: EvidenceKind::Build },
        ];
        // All unlinked + completion marker -> fires.
        assert!(detect_fake_completion(&claims, &[], "All done."));
        assert!(detect_fake_completion(
            &claims,
            &[Evidence::diff("t", "u", true, "wrote file")],
            "Everything is finished."
        ));
        assert!(detect_fake_completion(&claims, &[], "we are all set"));
        // A claim linked (ok same-kind evidence exists) -> quiet.
        assert!(!detect_fake_completion(
            &claims,
            &[Evidence::tests("t", "u", 5, 0, "cargo test")],
            "All done."
        ));
        // No completion marker -> quiet even when fully unlinked.
        assert!(!detect_fake_completion(&claims, &[], "Tests still failing."));
        // Whole-word rule: glued words are not markers.
        assert!(!detect_fake_completion(&claims, &[], "undone business abounds"));
        // Clean text, no claims -> quiet.
        assert!(!detect_fake_completion(&[], &[], "All done."));
    }

    #[test]
    fn premature_stop_fires_only_on_checklist_shortfall() {
        let claims = vec![ClaimSpan {
            text: "tests pass".into(),
            kind: EvidenceKind::Tests,
        }];
        let evidence = vec![
            Evidence::tests("t", "u", 5, 0, "cargo test"),
            Evidence::build("t", "u", Some(0), "cargo build"),
        ];
        // Every expectation met -> quiet (claims irrelevant by design).
        assert!(!detect_premature_stop(
            &claims,
            &evidence,
            &[
                ChecklistExpectation { kind: EvidenceKind::Tests, count: 1 },
                ChecklistExpectation { kind: EvidenceKind::Build, count: 1 },
            ]
        ));
        // Shortfall: expected a test run, none recorded -> fires.
        assert!(detect_premature_stop(
            &claims,
            &[],
            &[ChecklistExpectation { kind: EvidenceKind::Tests, count: 1 }]
        ));
        // Kinds are independent: met Tests does not cover missing Bench.
        assert!(detect_premature_stop(
            &claims,
            &evidence,
            &[
                ChecklistExpectation { kind: EvidenceKind::Tests, count: 1 },
                ChecklistExpectation { kind: EvidenceKind::Bench, count: 2 },
            ]
        ));
        // Empty checklist = nothing demanded -> never fires.
        assert!(!detect_premature_stop(&claims, &[], &[]));
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

    // sup-021: Bench constructor shape — kind, always-ok, "{name}: {ms}ms".
    #[test]
    fn bench_evidence_has_bench_kind_and_ms_summary() {
        let e = Evidence::bench("t", "u1", "provider_first_round", 1234);
        assert_eq!(e.kind, EvidenceKind::Bench);
        assert!(e.ok);
        assert_eq!(e.summary, "provider_first_round: 1234ms");
        let zero = Evidence::bench("t", "u2", "x", 0);
        assert_eq!(zero.summary, "x: 0ms");
        assert!(zero.ok, "a recorded duration is a measurement, not a verdict");
    }

    #[test]
    fn bench_evidence_round_trips_through_the_fold() {
        let dir = temp_dir("sup-021");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let b = Evidence::bench("t", "u1", "provider_first_round", 42);
        record(&journal, &b);
        drop(journal);

        let view = EvidenceView::fold(&path).expect("fold");
        assert_eq!(view.items, vec![b]);
        assert_eq!(view.items[0].kind, EvidenceKind::Bench);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // sup-022: Regression constructor shape — kind, ok=passed,
    // "regression: {name} {PASS|FAIL}".
    #[test]
    fn regression_evidence_formats_pass_and_fail_summaries() {
        let pass = Evidence::regression("t", "u1", "evidence_folds_back", true);
        assert_eq!(pass.kind, EvidenceKind::Regression);
        assert!(pass.ok);
        assert_eq!(pass.summary, "regression: evidence_folds_back PASS");
        let fail = Evidence::regression("t", "u2", "detect_fake_completion", false);
        assert!(!fail.ok);
        assert_eq!(fail.summary, "regression: detect_fake_completion FAIL");
    }

    #[test]
    fn regression_batch_preserves_order_and_fields() {
        let batch = Evidence::regression_batch(
            "t",
            "u1",
            &[("a_test", true), ("b_test", false), ("c_test", true)],
        );
        assert_eq!(batch.len(), 3);
        for e in &batch {
            assert_eq!(e.kind, EvidenceKind::Regression);
            assert_eq!(e.thread_id, "t");
            assert_eq!(e.turn_id, "u1");
            assert!(e.id.starts_with("ev-"));
        }
        let summaries: Vec<&str> = batch.iter().map(|e| e.summary.as_str()).collect();
        assert_eq!(
            summaries,
            vec![
                "regression: a_test PASS",
                "regression: b_test FAIL",
                "regression: c_test PASS",
            ]
        );
        let oks: Vec<bool> = batch.iter().map(|e| e.ok).collect();
        assert_eq!(oks, vec![true, false, true]);
    }

    #[test]
    fn regression_evidence_round_trips_through_the_fold() {
        let dir = temp_dir("sup-022");
        let path = dir.join("runtime.jsonl");
        let journal = Mutex::new(Journal::open(&dir, "runtime").expect("open"));
        let r = Evidence::regression("t", "u1", "sup_022_linkage", true);
        record(&journal, &r);
        drop(journal);

        let view = EvidenceView::fold(&path).expect("fold");
        assert_eq!(view.items, vec![r]);
        assert_eq!(view.items[0].kind, EvidenceKind::Regression);
        assert_eq!(view.items[0].summary, "regression: sup_022_linkage PASS");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
