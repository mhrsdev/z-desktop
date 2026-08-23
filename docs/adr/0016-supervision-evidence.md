# ADR-0016: Supervision & evidence records (capture-time proof, journal home, claim linking)

Ledger: decides sup-001 directly (record types) and fixes the shapes sup-002
(sandbox exit codes → Build), sup-003 (test-runner parsing → Tests), sup-004
(fs_write/edit_patch diffs → Diff), sup-021 (Bench recorder), sup-022
(Regression linkage) fill in. Decides sup-006's mechanism (claim-to-evidence
linker) given sup-005's regex set, and pre-decides sup-023/jour-023's
EvidenceRecorded event shape. Unblocks: sup-007/008 (verdicts ride linked
claims + evidence), sup-009..013 (detectors query the view this ADR defines),
sup-018/019 (UI renders records by id), sup-025 (FP-rate harness measures the
regex/linker policy fixed here). Out of scope, stated so nobody re-litigates:
ML-based verification and cross-machine attestation (§ Consequences).

## Status

Accepted (2026-08-23). Justification: the direction is binding — §2.1
principle 6 declares "Agents claim done only with verifiable evidence …
Claims without evidence are treated as failures by supervision" (:238-240);
§8.5 fixes the five evidence types (build/tests/diff/bench/regression,
:489-494) and the invariant "evidence is captured at execution time by the
system, not narrated by the model" (:499-500); §29.2 schedules an
EvidenceRecorded event `{turn_id, evidence}` as PLANNED (:1428) under the
"events carry ids, not blobs" rule (:1430-1432); §30.3 lists
`evidence_recorded` among journal kinds (:1466-1470); §30.4's task record
already carries `"evidence": ["evidence-..."]` (:1474-1479); §35.1 orders the
pipeline "journal → capture hooks → linker → supervision verdict → UI"
(:1622-1624) and names the failure modes this ADR must handle (:1625-1626);
§74.H walks the canonical flow (:3161-3167); M6 gates on it (:1504). What the
spec leaves open, and what this ADR decides: the exact record fields per type,
WHERE records persist given ADR-0012 made task state journal-only, HOW a text
claim binds to a record without trusting the model, and how much of the
"verification" ambition survives contact with personal scale. Adds no new
crate or dependency.

## Context

What exists today (repo inspection 2026-08-23): the JSONL journal is
append+replay with seq discipline and a lossy-kind escape hatch — unknown
kinds deserialize to `JournalKind::Other(String)` instead of failing replay
(journal.rs:32-48, :63-72); records carry `{seq, ts_ms, kind, thread_id,
payload: Value}` with free-form JSON payloads so new fields never break old
readers (journal.rs:90-103). Reducers are pure folds over replay with a
fail-loud corrupt-payload policy — `TasksView::fold` is the worked example,
and `TaskStore` shows the segment-per-view convention (reducer.rs:15-27,
:110-118, :198-205). Every turn has an id minted at start (`new_id("turn")`,
runtime.rs:408) and every tool call inside it carries a provider-assigned id
in `StoredToolCall {id, name, arguments_json, ok, summary}` (runtime.rs:23-29);
StepStarted/StepFinished events already announce each call to the UI channel
(runtime.rs:633, :720). The execution funnel is single: `tools::execute`
dispatches every tool (tools.rs:277-281), `terminal_exec` runs through
`sandbox::run`, which returns `ExecOutcome {stdout, stderr, code: Option<i32>,
timed_out}` — code is None exactly when the process was killed or timed out
(sandbox.rs:30-34, :104; tools.rs:545-579). `ToolOutput` is just `{ok, text}`
(tools.rs:13-17). Writes go through `fs_write`/`edit_patch` classified Risk::
Write (tools.rs:49); ADR-0010 journals old/new fnv1a64 fingerprints at
read/write time (:13-16, :209) but produces no textual diff yet — edit-019
(histogram diff generation, tasks :426) owns that. No evidence types, hooks,
or EvidenceRecorded kind exist anywhere yet; sup-001..006 are all PLANNED
(tasks :541-557).

Scale honesty: one user, one desktop process, one journal directory. The
adversary this subsystem defends against is a confused or lazy model narrating
success (the scripted dishonest agent of sup-020), not a hostile kernel
forging journal lines — ADR-0003's threat posture governs what runs outside
the sandbox, and nothing here changes it.

## Considered options

**(1) Record shape: one envelope + five payloads vs five unrelated structs.**

*(a)* Five unrelated structs. Every consumer (linker, detectors, UI) switches
on five disjoint types with no shared id/thread/turn plumbing; serialization
duplicates the common fields five times. Rejected.

*(b)* One envelope, five payload kinds — mirrors how the journal itself works
(one `Record` envelope, kind-specific `payload`). Chosen:

```rust
struct EvidenceRecord {
    id: String,               // "ev-..." — the only handle UI/detectors hold
    thread_id: String,
    turn_id: String,          // turn whose execution produced it
    task_id: Option<String>,  // set when the turn ran under orch task context
    kind: EvidenceKind,       // Build | Tests | Diff | Bench | Regression
    outcome: CaptureOutcome,  // Captured | CaptureFailed(String)
    body: EvidenceBody,       // exactly one variant, per kind
}
```

Per-kind bodies, each field sourced from a capture point that ALREADY exists
at hook time (no new plumbing — this is what makes capture-at-execution
cheap):

| Kind | Body fields | Captured from (hook) |
|---|---|---|
| Build | `command`, `exit_code: Option<i32>`, `timed_out: bool`, `duration_ms` | sup-002 wraps `sandbox::ExecOutcome`; `None` exit_code means killed/timed-out (sandbox.rs:104) |
| Tests | `command`, `counts {passed, failed, skipped}`, `failures: Vec<{name, excerpt}>`, `truncated: bool` | sup-003 parses ExecOutcome stdout/stderr after the run |
| Diff | `path`, `unified_excerpt` (bounded), `added_lines`, `removed_lines` | sup-004 wraps fs_write/edit_patch results; hunks via edit-019 |
| Bench | `name`, `metric`, `unit`, `direction`, `baseline`, `candidate`, `command` | sup-021; before/after pairs per §8.5 (:493) |
| Regression | `test_id`, `bug_ref`, `fails_before`, `passes_after` | sup-022 links a regression test to the bug it pins (:494) |

No `source` enum: each kind maps 1:1 onto exactly one hook (Build↔sup-002,
Tests↔sup-003, Diff↔sup-004, Bench↔sup-021, Regression↔sup-022), so a
provenance field would duplicate `kind`. Bounded payloads: raw output and
diff excerpts truncate at a constant (64 KiB, flagged `truncated`) — the UI
event carries only `{turn_id, evidence_id}` per §29.2's ids-not-blobs rule
(:1430-1432), and drill-down (sup-019) reads the full record from the view.
A failed capture emits the record anyway with `outcome: CaptureFailed(err)` —
§35.1 requires recording the gap because "absence of evidence is itself
evidence" (:1625-1626); a silent skip would be indistinguishable from a hook
that never shipped.

**(2) Where evidence lives.**

*(a)* Separate `data/evidence.json` (or SQLite) written by hooks alongside the
journal. Two writers of the same truth; crash between the two writes leaves a
task claiming evidence that isn't there; recovery needs a merge rule instead
of a replay. This is precisely the shape ADR-0012 rejected for task state
(*option (1b)* there) for precisely the same reasons. Rejected.

*(b)* Only in memory / only as UI events. Lost on restart; sup-024-style
override persistence and post-crash detector runs become impossible;
contradicts §30.3 listing `evidence_recorded` as a journal kind. Rejected.

*(c)* Journal events folded into an `EvidenceView` reducer. Hooks append
EvidenceRecorded records (jour-023 adds the typed `JournalKind` variant; until
that lands the payload round-trips through the `Other("evidence_recorded")`
escape hatch without breaking older binaries — journal.rs:63-72). The view
folds replay into `HashMap<id, EvidenceRecord>` plus secondary indexes by
turn_id and task_id, failing loud on corrupt payloads exactly like
TasksView (reducer.rs:113-118 precedent). Replay rebuilds everything after a
crash; double-replay equality extends for free. Matches §8.5's own sketch —
"evidence records attach to journal events" (:496-497) — and §30.4's task
record referencing evidence by id. Chosen.

Ordering note: the ledger wires jour-023 to depend on sup-001 (tasks :224-225,
:608), so sup-001 lands the serde types first (plain structs in z-core), the
journal variant second. Detectors and the linker read the view, never raw
transcripts.

**(3) Claim→evidence binding: explicit tool vs inference.**

*(a)* Optional `report_evidence` tool the model calls to attach evidence to
its claim. Self-defeating as the mechanism of record: the agent whose claims
are under audit decides whether to file the exhibits — a dishonest agent
simply doesn't call it (this is sup-020's scripted scenario), and an honest
one adds token cost re-stating what hooks already captured. It also violates
the division of labor §8.5 fixes: narration comes from the model, evidence
from the system (:499-500). Rejected as the linking mechanism.

*(b)* Infer claims from tool calls alone (any terminal_exec mentioning "cargo
test" implies a tests-pass claim). Over-links: running tests is not claiming
they pass; detectors fed noise learn nothing. Rejected as sole signal.

*(c)* System-side span matching, no model cooperation required. sup-005's
conservative regexes (exact phrase classes like "tests pass", "build succeeds"
— the examples §35.1 itself uses, :1616-1617) scan assistant message text and
yield `ClaimSpan {thread_id, turn_id, message_index, phrase_class}`. The
linker (sup-006) joins spans to EvidenceView on `(thread_id, turn_id)` with
class-compatible kinds (tests-phrases ↔ Tests, build-phrases ↔ Build, etc.),
emitting `linked(evidence_ids) | unlinked | ambiguous`. Claims about child
work resolve through the artifact evidence refs ADR-0012 decision 3 already
returns to parents (ADR-0012 :180-188) joined with the task record's
`"evidence": [...]` list (§30.4 :1478). Chosen.

Linking window is deliberately narrow: same turn only (plus explicit
artifact-ref resolution for delegated child work). An earlier green build in
the same session does NOT whitewash a later claim — that conservatism creates
known false positives (claim cites last turn's verified run), which is exactly
what sup-025's FP-rate harness measures before anyone widens the window.
Unlinked-but-confident spans feed sup-012 (fake completion); ambiguous ones
force needs-review, never auto-pass (:1625-1626, :3165-3166).

## Decision

1. **Types** (sup-001): one `EvidenceRecord` envelope (`id`, thread/turn/
   optional task ids, `kind`, `outcome: Captured | CaptureFailed`,
   kind-specific body) with exactly five bodies — Build, Tests, Diff, Bench,
   Regression — whose fields are listed in the table above and drawn verbatim
   from existing capture points (ExecOutcome, runner output parse,
   edit-019 diffs, bench pairs, regression test ids). Payloads truncated at
   64 KiB with a flag; UI receives ids only (:1430-1432). Failed captures
   still append a CaptureFailed record — the gap must be visible (:1625).
2. **Home** (jour-023/sup-023): EvidenceRecorded journal records carrying
   these envelopes as payload; jour-023 adds the typed `JournalKind` variant
   (older binaries keep replaying via the Other escape hatch). An
   `EvidenceView` reducer — TasksView's fold pattern, fail-loud on corrupt
   payloads — indexes records by id, turn_id, task_id. No side store, no
   SQLite now; recovery is replay.
3. **Hooks** (sup-002/003/004, feeding 021/022): capture lives at the three
   execution chokepoints that already exist — the terminal_exec result path
   (Build + Tests), the fs_write/edit_patch result path (Diff) — each writing
   its record immediately after the tool result is produced, tagged with the
   current turn_id and StoredToolCall id context. Hooks observe results; they
   never gate them.
4. **Claim linking** (sup-005/006): no new tool. Regex-found ClaimSpans join
   EvidenceView within the same turn (child-work claims resolve via artifact
   evidence refs). Results are linked/unlinked/ambiguous; unlinked feeds the
   fake-completion detector, ambiguous forces needs-review. Linking window
   stays same-turn until sup-025 data justifies widening.
5. **Out of scope**: ML-based claim/diff verification (regexes stay small and
   precise-first; expansion needs sup-025 measurements, not enthusiasm), and
   cross-machine attestation or cryptographic signing of records (single-
   process desktop; the journal is as trustworthy as the process writing it —
   the accepted ceiling everywhere else in z-core).

## Consequences

**Detectors get a clean substrate**: sup-009..013 query EvidenceView +
claim-link results — structured joins, not transcript archaeology; sup-020's
dishonest-agent script exercises exactly the unlinked path end to end.

**Honest ceilings**: regex linking misses paraphrased claims (a dishonest
agent can phrase around "tests pass") — supervision is a tripwire raising the
cost of lying, not a lie detector; the §8.4 invariant that supervision may
fail a turn but never silently edits output (:480-481) keeps false positives
user-visible rather than destructive. Same-turn windows flag legitimate
cross-turn citations as unlinked until measured otherwise.

**Storage**: evidence is the fattest journal content; the truncation constant
plus segment rotation (~16 MiB, :1470) bound worst cases, and Drill-down
fetches from the view on demand. If real-world records grow past rotation's
comfort, the fix is payload trimming, not a second store.

**Testing obligations locked in**: golden-record tests serialize one record
per kind and pin field-for-field wire format; double-replay equality extends
to EvidenceView (ADR-0012's rule); a truncated-output case asserts the flag
and byte cap; capture-failure injection asserts CaptureFailed records appear;
linker table tests cover linked/unlinked/ambiguous including the cross-turn
non-link; sup-020 integration closes the loop.

**No new dependencies**: serde structs over the existing journal/fold
machinery; every cited mechanism (turn ids, tool-call ids, ExecOutcome,
fingerprints, replay) already exists or has a ledger owner.

## Sources

- Repo inspection (2026-08-23): `z desktop/crates/z-core/src/journal.rs` —
  JournalKind + Other escape hatch (:36-48), lossy deserialization (:63-72),
  Record envelope (:90-103); `reducer.rs` — fold API (:15-27), TasksView
  fail-loud fold (:110-118), TaskStore segment convention (:198-205);
  `runtime.rs` — StoredToolCall ids (:23-29), turn_id minting (:408),
  StepStarted/StepFinished (:633, :720); `tools.rs` — ToolOutput (:13-17),
  Write classification (:49), execute funnel (:277-281), terminal_exec
  (:545-579); `sandbox.rs` — ExecOutcome (:30-34, :104).
- Z-DESKTOP-MASTER-SPEC.md: §2.1 principle 6 (:238-240), §2.2 (:314), §8.4
  (:462-481), §8.5 (:485-502), §29.2 events + ids-not-blobs (:1428-1432),
  §30.3 kinds (:1466-1470), §30.4 task record (:1474-1479), §35.1 playbook
  (:1608-1626), M6 (:1504), §74.H flow (:3161-3167).
- docs/Z-DESKTOP-TASKS.md (retrieved 2026-08-23): sup-001..006 (:541-557),
  sup-007/008 (:559-564), sup-009..013 (:565-578), sup-020 (:598-600),
  sup-021/022 (:601-605), sup-023 (:607-608), jour-023 (:224-225),
  edit-019 (:426).
- ADR-0012 (journal-only task store precedent, :164-170; artifact evidence
  refs and parent verification, :180-188; L3/sup-002 wiring, :136;
  double-replay testing rule, :235), ADR-0010 (fingerprint journaling the
  Diff hook builds on, :13-16, :209), ADR-0003 (sandbox threat posture).
