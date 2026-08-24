# Z Desktop Personal — Development State

> Session-resume protocol file. Every session MUST read this before asking the
> user anything, and update it before ending work. Information that lives only
> in a conversation is lost information.

Last updated: 2026-08-24

## Wave 18 (jour-019 + mem-019 layer export + ctx-016) — 2026-08-24 COMPLETE

1. ✅ **jour-019**: replay_perf_smoke + 10k-record perf test (< 2s gate).
2. ✅ **mem-019 ext**: export_layer per-layer pretty JSON.
3. ✅ **ctx-016**: compact_once idempotent compaction (compacted items
   survive tighter budgets; double-run identical). ctx-005 flipped.

Verification: full workspace suite green — **682 tests, 0 failed**
(678 → 682). Ledger: 150 IMPLEMENTED.

Next work continues → ui-050 usage dashboard panel data, mcp-001 MCP
study, or ext-001 plugin manifest research.

## Wave 17 (jour-017 + set-008 export + regression history) — 2026-08-24 COMPLETE

1. ✅ **jour-017**: write_fixture deterministic journal generator
   (byte-identical for identical params).
2. ✅ **set-008 ext**: export_schema_json pretty schema dump.
3. ✅ **sup-021 ext**: regression_history parser over Regression-kind
   evidence.

Verification: full workspace suite green — **678 tests, 0 failed**
(670 → 678). Ledger: 148 IMPLEMENTED.

Next work continues → ui-050 usage dashboard panel data, mcp-001 MCP
study, or ext-001 plugin manifest research.

## Wave 16 (ctx-006 + set-009 + sup-019) — 2026-08-24 COMPLETE

1. ✅ **ctx-006**: oldest_message_age_ms freshness helper over the
   ThreadsView.
2. ✅ **set-009**: remap_check validated-clamp preview for the remap UI.
3. ✅ **sup-019**: evidence_summary per-kind totals/oks aggregation
   (drill-down data; sup-019 marked PARTIAL — UI viewer itself pending).

Verification: full workspace suite green — **670 tests, 0 failed**
(662 → 670). Ledger: 147 IMPLEMENTED.

Next work continues → ui-050 usage dashboard panel data, mcp-001 MCP
study, or ext-001 plugin manifest research.

## Wave 15 (ctx-013 + set-008 + sup-024) — 2026-08-24 COMPLETE

1. ✅ **ctx-013**: never_compact_latest_user_invariant runtime check —
   the latest-user-message invariant as an executable assertion.
2. ✅ **set-008**: search_defs() case-insensitive fragment search over
   the schema (future settings UI data).
3. ✅ **sup-024**: verdict_effective() pure helper over the override
   list.

Verification: full workspace suite green — **662 tests, 0 failed**
(655 → 662). Ledger: 146 IMPLEMENTED.

Next work continues → ui-050 usage dashboard panel data, mcp-001 MCP
study, or ext-001 plugin manifest research.

## Wave 14 (ctx-015 + mem-020 + tok-012) — 2026-08-24 COMPLETE

1. ✅ **ctx-015**: save/load_session_layer — Session-layer JSONL
   persistence (round-trip incl. pinned flag).
2. ✅ **mem-020**: consolidation bounds dry-run (would_promote vs total
   provisional) for pre-flight cap checks.
3. ✅ **tok-012**: CacheMetrics atomic counters wired into the fs_read
   cache hit/miss points.

Verification: full workspace suite green — **655 tests, 0 failed**
(648 → 655). Ledger: 144 IMPLEMENTED.

Next work continues → ui-050 usage dashboard panel data, mcp-001 MCP
study, or ext-001 plugin manifest research.

## Wave 13 (ctx-012 + set-005 + idx-019) — 2026-08-24 COMPLETE

1. ✅ **ctx-012**: fold_twice_equal runtime determinism check +
   task_counts() by status.
2. ✅ **set-005**: constraint_error pretty messages + reset_to_default
   per-key restore.
3. ✅ **idx-019**: incremental_add/remove with doc→trigram reverse map;
   removed ids stay reserved (slot reuse deferred).

Verification: full workspace suite green — **648 tests, 0 failed**
(639 → 648). Ledger: 142 IMPLEMENTED.

Next work continues → ui-050 usage dashboard panel data, mcp-001 MCP
study, or ext-001 plugin manifest research.

## Wave 12 (sup-023 verify + prov-002 + edit-005 backup) — 2026-08-24 COMPLETE

1. ✅ **sup-023**: verified EvidenceRecorded wiring complete; added
   evidence_for_turn(view, turn_id) filtered helper.
2. ✅ **prov-002**: HealthProbe types + probe_verdict latency bands
   (fast/normal/slow); network probing stays runtime-side.
3. ✅ **edit-005 hardening**: write_with_backup — rolling .bak before
   overwrite, skip-identical semantics preserved.

Verification: full workspace suite green — **639 tests, 0 failed**
(633 → 639). Ledger: 140 IMPLEMENTED.

Next work continues → ui-050 usage dashboard panel data, mcp-001 MCP
study, or ext-001 plugin manifest research.

## Wave 11 (set-007 + tok-022 + term-017) — 2026-08-24 COMPLETE

1. ✅ **set-007**: SETTINGS_VERSION + migrate() — passthrough/upgrade/
   newer-version-refusal, chained-migration seam documented.
2. ✅ **tok-022**: estimator_error MAPE harness + worst_sample().
3. ✅ **term-017**: throughput benchmark (measured MB/s printed,
   scrollback cap asserted under load).

Verification: full workspace suite green — **633 tests, 0 failed**
(623 → 633). Ledger: 139 IMPLEMENTED.

Next work continues → ext-001 plugin manifest research, ui-050 usage
dashboard panel data, or mcp-001 MCP integration study.

## Wave 10 (ctx-009 weights + jour-016 lag metric) — 2026-08-24 COMPLETE

1. ✅ **ctx-009**: PriorityWeights (per-layer f32, default 1.0) +
   weighted_tokens() with ceil scaling — settings-backed allocation
   groundwork.
2. ✅ **jour-016**: LagStats {records, last_seq, gaps} + lag_stats() —
   total missing-seq accounting across all gap points (reuses
   first_seq_gap).

Note: ctx-009's mid-run "2 failed" report was the sibling's in-flight
journal.rs state; final tree is green.

Verification: full workspace suite green — **623 tests, 0 failed**
(616 → 623). Ledger: 136 IMPLEMENTED.

Next work continues → ui-050 usage dashboard panel, term-006 VT parser
study (clone vte-rs), or ext-001 plugin manifest format research.

## Wave 9 (jour-016 + mem-018 + theme-010) — 2026-08-24 COMPLETE

1. ✅ **jour-016**: redacted_summary — counts records with secret-shaped
   substrings already masked; security-scan allowlist gained the new
   alphabet fake fixture (real value extracted from source, not guessed).
2. ✅ **mem-018**: AnchoredFact + anchored_facts()/stale_anchors() —
   file-anchored memory records flagged stale on fingerprint change.
3. ✅ **theme-010**: LIGHT tokens (calm light palette, same accent hue
   family; luminance-tested).

Verification: full workspace suite green — **616 tests, 0 failed**
(609 → 616). Ledger: 134 IMPLEMENTED.

Next work continues → ui-050 usage dashboard, ctx-009 priority weights,
or term-006 VT parser study.

## Wave 8 (set-006 verify + ctx-004 pins + diff-020 patience) — 2026-08-24 COMPLETE

1. ✅ **set-006**: verified prior attempt landed (known_keys/defaults_map +
   unknown-key rejection test) — no rewrite needed.
2. ✅ **ctx-004**: ContextItem.pinned field + set_pinned(); assemble()
   never drops pinned Sessions (tested under tight budgets). ctx-004
   marked PARTIAL (round summaries still future).
3. ✅ **diff-020**: unified_patience — unique-line anchors + LIS, LCS
   fallback between anchors; Strategy enum; fewer changed lines than LCS
   on noisy replacements (tested).

Verification: full workspace suite green — **609 tests, 0 failed**
(605 → 609). Ledger: 132 IMPLEMENTED + 5 PARTIAL.

Next work continues → ui-050 usage dashboard, term-006 VT parser study,
or ctx-009 priority weight settings.

## Wave 7 (prov-010 verify + usage-by-day + mem export + bench trend) — 2026-08-24 COMPLETE

1. ✅ **prov-010**: verified already landed in wave 5 (MAX_SSE_LINE cap
   present, committed 40d14c0) — ledger flipped to IMPLEMENTED.
2. ✅ **jour-009 ext**: usage_by_day(path) — UTC-day turn buckets.
3. ✅ **mem-019**: export_json/import_records round-trip via journal.
4. ✅ **sup-021 ext**: BenchPoint/bench_history/trend — benchmark
   regression tracking over evidence.

Verification: full workspace suite green — **596 tests, 0 failed**
(587 → 596). Ledger: 131 IMPLEMENTED.

Next work continues → ui-050 usage dashboard panel data, term-006 VT
parser research (clone vte-rs reference), or ctx context-delta research.

## Wave 6 (retries + lazy manifests + symbol table) — 2026-08-24 COMPLETE

1. ✅ **red-002** (retry OK): RedactionReport/RedactionStats counters —
   per-kind hits, last-hit timestamp, snapshot isolation; funnel wiring
   deferred (red-002 marked PARTIAL).
2. ✅ **tok-017** (retry OK): cache_hit_rate + CacheStats.
3. ✅ **tok-021**: should_list/filter_manifest — grok-build's per-turn
   manifest filtering adapted sync (token economy).
4. ✅ **idx-005**: SymbolTable cross-file index — add_file/lookup_name/
   total over symbol ids.

Verification: full workspace suite green — **587 tests, 0 failed**
(573 → 587). Ledger: 130 IMPLEMENTED.

Next work continues → prov-010 SSE cap retry, ui-050 usage dashboard, or
term-006 VT parser research.

## 8-Agent Wave 5 (inspectors + tokens + schema + ADR-0022) — 2026-08-24 COMPLETE

First 8-sibling fan-out. Landed:

1. ✅ **jour-014**: journal compact(path, keep_last) — atomic rewrite,
   seq renumbering, no-op when count <= keep_last.
2. ✅ **idx-021**: repo_map(max_chars) with FNV fingerprint cache keyed
   on (path, stamp) pairs — budget-respecting, truncation marker.
3. ✅ **ADR-0022** (docs/adr/0022-ui-maturation.md): honest UI audit
   (hardcoded px leaks line-cited), Z Desktop design language, phased
   plan A-D (tokens → components → motion → theming).
4. ✅ **set-002**: SettingDef/DefKind/SettingDefault + schema_defs() +
   validate() reusing load()/apply() range consts.
5. ✅ **ctx-008**: ContextStats + stats()/preview() inspector APIs.
6. ✅ **mem-012**: MemoryStats + view_stats().
7. ✅ **edit-018**: blind_write_check() — no-recorded-fingerprint refusal.
8. ✅ **theme-001**: z-shell theme.rs Tokens::DARK mirroring the approved
   zero_dark palette via z_tokens::Rgba (luminance-tested).
9. ✅ **edit-004 completion**: atomic_write write_if_changed dedup.

Failed (empty model replies, retry next wave): red-002 redaction
counters, tok-017 cache hit-rate, prov-010 SSE line cap.

Verification: full workspace suite green — **573 tests, 0 failed**
(545 → 573). Ledger: 128 IMPLEMENTED.

## Parallel Wave 4 (sup-022 + idx-020 + kb nav) — 2026-08-24 COMPLETE

1. ✅ **sup-022**: regression() + regression_batch() evidence helpers —
   per-test PASS/FAIL linkage records.
2. ✅ **idx-020**: ranked_search (trigram coverage scoring) +
   repo.build_search_index()/search() mapping doc ids to rel paths
   (rebuild-on-demand; incremental updates deferred).
3. ✅ **kb nav**: thread rows are focusable semantic buttons — ↑/↓ roam
   (wrapping), Enter switches via the existing ViewCommand path, mouse
   clicks work free. kb-001 marked PARTIAL (app-level nav exists;
   z-shell keymap tables as data still pending).

Verification: full workspace suite green — **545 tests, 0 failed**
(532 → 545). Ledger: 124 IMPLEMENTED + 4 PARTIAL.

Next work continues → idx-021 repo-map cache, sup-023 EvidenceRecorded
journal kind, or ui-050 usage dashboard.

## Codex Reference Wave (ADR-0021 + term-004 + thread selection) — 2026-08-24 COMPLETE

1. ✅ **ADR-0021** (docs/adr/0021-sandbox-and-exec.md): evidence-based
   survey of codex-rs exec/ (process_group(0), kill_on_drop, Windows Job
   Objects) vs our sandbox.rs; phased decision — Phase 1 kill-on-close
   guards + timeout + caps (shipped now), Phase 2 platform sandboxes
   (Landlock/Seatbelt/Job objects) behind a trait seam with graceful
   fallback, Phase 3 per-tool capability grants integrated with Risk::
   Write machinery. Containers-by-default rejected for personal desktop.
2. ✅ **term-004**: ChildGuard kill-on-drop in sandbox.rs wired to the
   timeout path — semantics match codex's kill_on_drop; process-group
   kill deferred to Phase 2 (needs libc dep).
3. ✅ **Thread selection UI**: active-thread highlight in the sidebar
   (primary vs secondary text) completing the SwitchThread visual loop.

Verification: full workspace suite green — **532 tests, 0 failed**
(526 → 532). Ledger: 120 IMPLEMENTED.

Next work continues → sup-022 regression recorder, ui keyboard nav
completion, or idx-020 lexical search query path.

## Reference Audit + Parallel Wave 2 — 2026-08-24 COMPLETE

Mission-recall audit performed (no rebuilds, no redesign):

1. ✅ **Reference audit**: grok-build verified in place (HEAD 07b2f71,
   Apache-2.0, 7-subsystem dissection already feeding ADR-0007/0008/0014,
   steering queue, tool design). REFERENCE-IMPLEMENTATION-MAP.md created
   with per-subsystem evidence rows; codex/zed/aider clone points tied to
   their upcoming slices per §7 no-speculative-clones rule.
2. ✅ **idx-010**: symbol_id(file_hash, name, kind, index) +
   extract_with_ids.
3. ✅ **diff-019**: z-core/src/diff.rs — LCS unified line diff + stats.
4. ✅ **idx-018**: z-core/src/search_index.rs — trigram index with
   candidates/verify/search + short-query linear fallback.

Verification: full workspace suite green — **526 tests, 0 failed**
(510 → 526). Ledger: 119 IMPLEMENTED.

Next work continues → term sandbox hardening (clone codex first), ui
thread selection, or sup-022 regression recorder.

## Parallel Fan-Out Wave (sup-021 + layout-002 + term-005/018 + tok-019) — 2026-08-24 COMPLETE

First wave dispatched via the user-provided 3-key OpenRouter credential
pool (keys stored in /root/.secrets/, never echoed):

1. ✅ **sup-021**: Evidence::bench + first-round provider timing captured
   once per turn. Found+fixed a real bug: always-ok Bench records were
   whitewashing the sup-007 gate — Bench excluded from the ok-count.
2. ✅ **layout-002**: dock_indicators.rs — DropZone/DropIndicator +
   compute_drop_indicator over ShellFrame rects (pure geometry).
3. ✅ **term-005/018**: Scrollback ring buffer with 10 MiB cap + 64 MiB
   push safety test proving bounded memory.
4. ✅ **tok-019**: usage_stats.rs TokenLedger — per-task charge/total/
   top_n + ORCH_TOKEN_BUDGET_PER_TASK enforcement helper.

Verification: full workspace suite green — **510 tests, 0 failed**
(492 → 510). Ledger: 115 IMPLEMENTED.

Next work continues → ui thread selection, sup-022 regression recorder,
or term-006 VT parser.

## core-024 + SwitchThread — 2026-08-23 COMPLETE

1. ✅ **core-024**: Shared.active_turns set in start_turn, cleared in
   run_turn's finish; DeleteThread on an active turn emits the rejection
   message instead of deleting (tested both paths).
2. ✅ **SwitchThread**: additive command validates existence and echoes
   ThreadSwitched; z-app mirrors active_thread_id.

Verification: full workspace suite green — **492 tests, 0 failed**
(487 → 492). Ledger: 110 IMPLEMENTED.

Next work continues → ui thread selection click-through, sup-021
benchmark recorder, or core-028 latency benchmark.

## prov-007 Fallback Chain — 2026-08-23 COMPLETE

1. ✅ **prov-007**: fallback_chain(registry, requested, available) —
   exact match first, then capability supersets (ctx >= and tools >=),
   then the rest, stable within buckets; unavailable requested never
   appears. Runtime wiring awaits multi-provider config.

Verification: full workspace suite green — **487 tests, 0 failed**
(483 → 487). Ledger: 109 IMPLEMENTED.

Next work continues → ui thread selection, sup-021 benchmark recorder,
or core-028 latency benchmark.

## sup-020 Dishonest-Agent E2E — 2026-08-23 COMPLETE

1. ✅ **sup-020**: two scripted end-to-end scenarios — dishonest agent
   (claims tests pass with zero Tests evidence => sup-009 fires, verdict
   reason carries it) vs honest agent (cargo test exit-0 evidence lands =>
   clean success). Proves the whole supervision pipeline works live.

Verification: full workspace suite green — **483 tests, 0 failed**
(481 → 483). Ledger: 108 IMPLEMENTED.

Next work continues → prov-007 fallback chain, ui thread selection, or
sup-021 benchmark evidence recorder.

## jour-009 Usage Stats Reducer — 2026-08-23 COMPLETE

1. ✅ **jour-009**: UsageView {turns_started, commands_total,
   messages_persisted, provider_errors} + usage_fold over the generic
   fold; unknown kinds ignored.

Verification: full workspace suite green — **481 tests, 0 failed**
(478 → 481). Ledger: 107 IMPLEMENTED.

Next work continues → sup-020 scripted dishonest-agent integration test,
prov-007 fallback chain, or ui thread selection.

## sup-016 Requirement-Skew Detector — 2026-08-23 COMPLETE

1. ✅ **sup-016**: detect_requirement_skew — extracts quoted strings and
   ALL-CAPS tokens (>=4 chars) from the request; fires warn-only when any
   demanded token is absent from the delivered text. run_turn compares
   final user request vs agent response.

Verification: full workspace suite green — **478 tests, 0 failed**
(474 → 478). Ledger: 106 IMPLEMENTED.

Next work continues → prov-007 fallback chain, sup-020 scripted dishonest-
agent integration test, or jour-009 usage-stats reducer.

## jour-012 Deterministic Replay Proof — 2026-08-23 COMPLETE

1. ✅ **jour-012**: double-replay equality tests for ThreadsView,
   TasksView, and the deps variant; plus reverse-construction-order fold
   proving identical views (ADR-0004 deterministic replay posture).

Verification: full workspace suite green — **474 tests, 0 failed**
(470 → 474). Ledger: 105 IMPLEMENTED.

Next work continues → sup-016 requirement-skew detector, prov-007
fallback chain, or jour-009 usage-stats reducer.

## core-027 Perf Benchmark — 2026-08-23 COMPLETE

1. ✅ **core-027**: 1k-message perf test — build_request median of 5 runs
   = **3.41ms** (< 50ms, ~15× headroom); enforce_budget full-compaction
   path = **3.15ms** (< 20ms). Measurements print on every run so
   regressions are visible. No thresholds loosened.

Verification: full workspace suite green — **470 tests, 0 failed**
(469 → 470). Ledger: 104 IMPLEMENTED.

Next work continues → sup-016 requirement-skew detector, prov-007
fallback chain, or jour-012 double-replay equality.

## mem-015 Replay + View Rebuild — 2026-08-23 COMPLETE

1. ✅ **mem-015**: replay_summary (live/superseded/provisional partition,
   invariant-tested) + MemoryStore::rebuild_views — THE recovery path:
   folds the journal, rewrites all three layer views atomically, heals
   corrupted view files (tested), journal untouched.

Verification: full workspace suite green — **469 tests, 0 failed**
(466 → 469). Ledger: 103 IMPLEMENTED.

Next work continues → core-027 build_request perf benchmark, sup-016
requirement-skew detector, or prov-007 fallback chain.

## mem-014 Daily Write Cap — 2026-08-23 COMPLETE

1. ✅ **mem-014**: DAILY_RECORD_CAP=200 (record-count proxy for the
   ADR-0014 ≤10 MB/day budget); count_today folds by UTC day index;
   promote_candidates stops mid-pass when today's quota is exhausted —
   partial success, no error. Corrupt journal counts as full (fail-safe).

Verification: full workspace suite green — **466 tests, 0 failed**
(463 → 466). Ledger: 102 IMPLEMENTED.

Next work continues → sup-016 requirement-skew detector, mem-015 replay
from journal, or core-027 perf benchmark.

## mem-013 Recency Decay — 2026-08-23 COMPLETE

1. ✅ **mem-013**: decay_confidence (c × 0.5^(age/half_life), clamped,
   degenerate half-life safe) + retrieve_with_decay re-ranking by
   provenance.ts_ms age; fresh beats old on equal matches (tested with
   exact score math).

Verification: full workspace suite green — **463 tests, 0 failed**
(460 → 463). Ledger: 101 IMPLEMENTED.

Next work continues → core-027 build_request perf benchmark, sup-016
requirement-skew detector, or mem-014 daily write cap.

## prov-006 Router Decision Logging — 2026-08-23 COMPLETE

1. ✅ **prov-006**: Decision {model, caps, reason} + decide() wrapping
   lookup with "family match 'x'" / "fallback" reasons; run_turn logs one
   info line per turn with tools/ctx/reason.

Verification: full workspace suite green — **460 tests, 0 failed**
(458 → 460). Ledger: 100 IMPLEMENTED — the century mark.

Next work continues → mem-013 recency decay, core-027 build_request perf
benchmark, or sup-016 requirement-skew detector.

## sup-013 Premature-Stop Detector — 2026-08-23 COMPLETE

1. ✅ **sup-013**: ChecklistExpectation + detect_premature_stop — pure
   coverage check of same-kind evidence counts against an explicit
   checklist; TODO marks the run_turn slot for orch-001 checklist
   integration.

Verification: full workspace suite green — **458 tests, 0 failed**
(457 → 458). Ledger: 99 IMPLEMENTED.

Next work continues → prov-006 decision logging, mem-013 recency decay,
or core-027 build_request perf benchmark.

## set-004 Live Settings — 2026-08-23 COMPLETE

1. ✅ **set-004**: additive Command::SetSetting → Event::SettingChanged;
   settings::apply() is the single validation point (same range consts as
   load()); handler persists via store + swaps the Shared snapshot Arc
   (ADR-0011 swap-on-write). Test proves max_tool_rounds=2 takes effect on
   the next turn in the SAME session — no restart.

Verification: full workspace suite green — **457 tests, 0 failed**
(453 → 457). Ledger: 98 IMPLEMENTED.

Next work continues → set-002 full schema draft, sup-013 premature-stop
detector, or prov-006 decision logging.

## sup-017/024 Verdict Appeals — 2026-08-23 COMPLETE

1. ✅ **sup-017**: additive Command::AppealVerdict → Event::
   VerdictOverridden; JournalKind::VerdictOverridden records {turn_id,
   thread_id}; Shared.overridden_turns skip-set consulted by the run_turn
   gate (overridden turns go warn-only).
2. ✅ **sup-024 (partial)**: load_overridden_turns() folds journaled
   overrides into the set at startup — persistence across restarts.

Verification: full workspace suite green — **453 tests, 0 failed**
(450 → 453). Ledger: 97 IMPLEMENTED.

Next work continues → sup-013 premature-stop detector, prov-006 decision
logging, or mem-013 recency decay.

## sup-012 Fake-Completion Detector — 2026-08-23 COMPLETE

1. ✅ **sup-012**: detect_fake_completion — fires when claims exist, ALL
   are unlinked (even unrelated ok evidence doesn't suppress), and the
   final text has a whole-word completion marker. Warn-only alongside the
   sup-009/010/011 detectors; gating escalation deferred.

Verification: full workspace suite green — **450 tests, 0 failed**
(449 → 450). Ledger: 95 IMPLEMENTED.

Next work continues → sup-013 premature-stop detector, sup-017 appeal
flow, or prov-006 decision logging.

## jour-011 Corrupt-Tail Repair — 2026-08-23 COMPLETE

1. ✅ **jour-011**: truncate_corrupt_tail(path) removes a torn final write
   (crash mid-append per ADR-0004); Journal::replay auto-repairs once when
   the malformed line is the LAST line, then retries; middle corruption
   stays fail-loud. Idempotent second replay verified.

Verification: full workspace suite green — **449 tests, 0 failed**
(445 → 449). Ledger: 94 IMPLEMENTED.

Next work continues → sup-012 fake-completion detector, prov-006 decision
logging, or mem-013 recency decay.

## core-017/018/019 Retry Hardening — 2026-08-23 COMPLETE

1. ✅ **core-017**: parse_retry_after ("retry after N" / "retry-after: N")
   with min(N, 30) sleep for RateLimited; default 1s otherwise.
2. ✅ **core-018**: pending_retry slot replays the SAME ChatRequest object
   on retry (steering drain skipped on the replay round); capturing
   provider test asserts byte-for-byte request identity.
3. ✅ **core-019**: additive JournalKind::ProviderError breadcrumbs
   {attempt, class} per failed attempt.

Verification: full workspace suite green — **445 tests, 0 failed**
(442 → 445). Ledger: 93 IMPLEMENTED.

Next work continues → sup-012 fake-completion detector, prov-006 decision
logging, or jour-011 corrupt-tail repair.

## jour-010 Seq-Gap Tolerance — 2026-08-23 COMPLETE

1. ✅ **jour-010**: reducer::fold calls first_seq_gap after replay and
   warns ("gap at record N, expected seq X") but continues — views
   tolerate gaps; fail-loud remains only in replay's malformed-line path.
   Reused the committed (index, expected_seq) helper shape; 1-based warn.
   Test asserts the warning via a captured log::Log and that the view
   still folds.

Verification: full workspace suite green — **442 tests, 0 failed**
(441 → 442). Ledger: 90 IMPLEMENTED.

Next work continues → sup-012 fake-completion detector, sup-017 appeal
flow, or prov-006 decision logging.

## sup-014/015 Write Detectors — 2026-08-23 COMPLETE

1. ✅ **sup-014**: detect_placeholder_code — 7 marker classes (todo:
   implement, fixme, not implemented, unimplemented!(), todo!(), <insert,
   ...rest of), any-single-marker policy documented.
2. ✅ **sup-015**: detect_mock_in_prod — identifier markers
   mock_response/dummy_data/stub_impl/fake_ with word-boundary awareness.
3. Capture hook runs both on successful fs_write/edit_patch content,
   warn-only, naming detector + path.

Verification: full workspace suite green — **441 tests, 0 failed**
(438 → 441). Ledger: 89 IMPLEMENTED.

Next work continues → sup-012 fake-completion detector, sup-017 appeal
flow, or jour-010 seq-gap UI.

## core-023 Delete Tombstones — 2026-08-23 COMPLETE

1. ✅ **core-023**: additive JournalKind::ThreadDeleted; delete_thread
   appends a shape-only {thread_id} tombstone best-effort after map+disk
   removal; delete without journal still works. Reducer-side exclusion of
   deleted ids deferred until ThreadsView consumes the journal.

Verification: full workspace suite green — **438 tests, 0 failed**
(436 → 438). Ledger: 87 IMPLEMENTED.

Next work continues → sup-012..015 detectors, prov-006 decision logging,
or jour-010 seq-gap detection UI.

## prov-004/005 Capability Registry — 2026-08-23 COMPLETE

1. ✅ **prov-004**: `z-core/src/router.rs` — Capabilities {context_window,
   supports_tools} + Registry with seeded families (gpt-/claude-/o3/
   o4-mini/llama) and conservative fallback; longest case-insensitive
   prefix lookup. Provider trait gains additive model() accessor.
2. ✅ **prov-005 (partial hook)**: build_request gates tool attachment on
   capabilities — tool-less models get an empty tools list and a
   warn-once-per-turn notice; token budget shrinks consistently.

Verification: full workspace suite green — **436 tests, 0 failed**
(429 → 436). Ledger: 86 IMPLEMENTED.

Next work continues → prov-006 decision logging, core-023 delete
tombstones, or sup-012..015 detectors.

## ui-040 Evidence Badges — 2026-08-23 COMPLETE

1. ✅ **ui-040**: additive Command::GetEvidence {turn_id} → Event::
   EvidenceSummary {items: Vec<EvidenceInfo>} (cap 50, best-effort fold);
   z-app mirrors deduped (kind, ok) badges; Chat header draws
   "[Build ok]"/"[Tests FAIL]" style badges, hidden when empty. Visual
   suite green.

Verification: full workspace suite green — **429 tests, 0 failed**
(424 → 429). Ledger: 85 IMPLEMENTED.

Next work continues → sup-012..015 detectors, prov-005 capability
registry, or core-023 delete tombstones.

## core-026 Corrupt-Thread Ghosts — 2026-08-23 COMPLETE

1. ✅ **core-026**: restore_threads reports unreadable .json files
   ("{filename}: {error}"); Runtime.corrupt_threads accessor; ThreadList
   appends read-only ghost rows ("[corrupt] {filename}", 0 messages,
   sorted last) so gaps stay visible per ADR-0016 CaptureFailed
   philosophy. DeleteThread removes the file and drops the ghost.

Verification: full workspace suite green — **424 tests, 0 failed**
(422 → 424). Ledger: 84 IMPLEMENTED.

Next work continues → ui-040 evidence badges, prov-005 capability
registry, or core-023 delete tombstones in journal.

## mem-010/011 Corrections — 2026-08-23 COMPLETE

1. ✅ **mem-010**: correct(journal, original_id, …) — validates the live
   target (unknown/double-correct error), appends supersede event + a
   Promoted replacement ({id}-c{ts}, clamped confidence); replacement is
   built before any append so failures can't orphan the chain.
2. ✅ **mem-011**: dependents_of walks superseded_by chains iteratively
   (nearest first, cycle-safe).

Verification: full workspace suite green — **422 tests, 0 failed**
(419 → 422). Ledger: 83 IMPLEMENTED.

Next work continues → prov-005 capability registry, ui-040 evidence
badges, or core-026 corrupt-thread surfacing.

## mem-008/009 Retrieval + Injection — 2026-08-23 COMPLETE

1. ✅ **mem-008**: retrieve(view, terms, cap) — scores live() records as
   0.3×confidence + case-insensitive term-overlap ratio; sorts desc,
   truncates. Substring ranking noted as the pre-embeddings ceiling.
2. ✅ **mem-009**: inject_memories appends Turn-layer "[memory] …" items
   while cumulative est_tokens fits the budget; pure function with unit
   tests — runtime wiring awaits a settings knob.

Verification: full workspace suite green — **419 tests, 0 failed**
(415 → 419). Ledger: 81 IMPLEMENTED.

Next work continues → mem-010 correction flow, sup-012..015 detectors,
or prov-005 capability registry.

## mem-006/007 Consolidation — 2026-08-23 COMPLETE

1. ✅ **mem-006**: consolidate(journal, store) — promotes Provisional
   records with confidence >= 0.75 via same-id Promoted follow-up events
   (last-line-wins fold), dedups by (layer, normalized content) keeping
   highest confidence and superseding the rest, caps at 100 promotions per
   pass, then rebuilds all three layer views from journal truth.
2. ✅ **mem-007**: Provisional→Promoted lifecycle exercised through the
   same pass (status transitions are journal events, never in-place edits).

Verification: full workspace suite green — **415 tests, 0 failed**
(412 → 415). Ledger: 79 IMPLEMENTED.

Next work continues → mem-008 retrieval API, sup-012..015 detectors, or
prov-005 capability registry.

## sup-009/010/011 Detectors — 2026-08-23 COMPLETE

1. ✅ **sup-009/010**: detect_unexecuted_tests/build — a success claim of
   that kind with zero same-turn evidence of the kind (failed evidence
   still counts as executed).
2. ✅ **sup-011**: detect_ignored_failures — failing Tests evidence plus a
   success phrase in the final text.
3. run_turn supervision block warns naming fired detectors; unexecuted
   kinds extend the sup-007 blocked reason ([detectors: …]); gating
   behavior otherwise unchanged.

Verification: full workspace suite green — **412 tests, 0 failed**
(409 → 412). Ledger: 77 IMPLEMENTED.

Next work continues → sup-012..015 remaining detectors, mem-006
consolidation, or prov-005 capability registry.

## orch-024 Crash Recovery — 2026-08-23 COMPLETE

1. ✅ **orch-024**: recover_orphans(tasks_dir) folds the task journal;
   orphaned Running tasks transition to Failed ("orphaned by restart"),
   pending-ready count returned for external re-submission (bodies are
   closures and die with the process — honest recovery). Wired into
   Orchestrator::spawn best-effort; missing journal = (0,0).

Verification: full workspace suite green — **409 tests, 0 failed**
(406 → 409). Ledger: 74 IMPLEMENTED.

Next work continues → sup-009..015 detectors, mem-006 consolidation, or
prov-005 capability registry.

## sup-008 Verdict on TurnFinished — 2026-08-23 COMPLETE

1. ✅ **sup-008**: z-protocol gains SupervisionVerdictInfo; TurnFinished
   carries verdict: Option<...> with #[serde(default)] (legacy payloads
   deserialize to None — tested). run_turn stores the last evaluation in
   turn-local state and the shared finish closure attaches it on every
   exit path; turns that never evaluate keep None.

Verification: full workspace suite green — **406 tests, 0 failed**
(403 → 406). Ledger: 73 IMPLEMENTED.

Next work continues → prov-005 capability registry, mem-006 consolidation,
or ui-040 evidence badges.

## sup-007 Supervision Verdict — 2026-08-23 COMPLETE

1. ✅ **sup-007**: SupervisionVerdict + evaluate_claims — blocks only when
   claims exist, ALL are unlinked, zero ok evidence in the turn, journal
   folded fine, and the turn had tool calls (evidence should have existed).
   Otherwise warn-only. run_turn converts a blocked verdict into
   finish(false) with the reason.

Verification: full workspace suite green — **403 tests, 0 failed**
(402 → 403). Ledger: 72 IMPLEMENTED.

Next work continues → sup-008 verdict on TurnFinished event, mem-006
consolidation, or prov-005 provider capability registry.

## ctx-007 Stale Rehydration — 2026-08-23 COMPLETE

1. ✅ **ctx-007**: fingerprint.rs gains stale_reads(thread_id, root) —
   diffs the registry against current disk, returning changed raw paths
   (missing files skipped). context.rs: ContextItem.stale additive flag +
   demote_if_stale(); assemble() now drops stale Ephemeral items FIRST.
   runtime computes the stale set at turn start and marks items.

Verification: full workspace suite green — **402 tests, 0 failed**
(398 → 402). Ledger: 71 IMPLEMENTED.

Next work continues → sup-007 fake-completion gating, mem-006 consolidation,
or ui-040 evidence badges.

## orch-004/012 Budget Deadlines — 2026-08-23 COMPLETE

1. ✅ **orch-004**: OrchCommand::EnqueueTask carries optional absolute
   deadline_ms; the orchestrator's 1s sweep now really fires: past-deadline
   tasks still in `running` transition to Failed ("budget exceeded") with
   journal evidence. Worker removes itself from `running` before its final
   Done transition, so Done-after-sweep races are structurally excluded
   (ponytail note: tiny window documented, fine at personal scale).
2. ✅ **orch-012**: ORCH_CEILING=4 constant added beside
   ORCH_MAX_CONCURRENT=2; enqueue asserts both bounds (per-parent caps are
   the later slice).

Verification: full workspace suite green — **398 tests, 0 failed**
(395 → 398). Ledger: 70 IMPLEMENTED.

Next work continues → orch-005 spawn-policy validation, sup-007 gating,
or ctx-007 stale rehydration markers.

## Thread List UI Wiring — 2026-08-23 COMPLETE

1. ✅ **core-021 UI**: WorkspaceView.threads mirrors Event::ThreadList;
   sidebar draws thread rows (title + message count) above the project
   index rows via a shared two-line row helper; apply_event populates the
   mirror and triggers re-render. Display-only; selection is later work.

Verification: full workspace suite green — **395 tests, 0 failed**
(392 → 395). Ledger: 69 IMPLEMENTED.

Next work continues → thread selection/switching, sup-007 fake-completion
gating, or mem-006 consolidation pass.

## mem-005 Candidate Extraction — 2026-08-23 COMPLETE

1. ✅ **mem-005**: extract_candidates — regex-free sentence heuristics;
   marker phrases ("remember that", "the user prefers", "always/never
   use") → Project candidates (0.6), definitional "X means/is a Y" →
   Semantic (0.5); dedup + 20/pass cap. promote_candidates writes
   Provisional MemoryRecorded events best-effort; run_turn success path
   extracts from final text (never fails the turn).

Verification: full workspace suite green — **392 tests, 0 failed**
(387 → 392). Ledger: 68 IMPLEMENTED.

Next work continues → mem-006 consolidation/promotion pass, sup-007
fake-completion gating, or ui thread list panel.

## sup-005/006 Claim Linking — 2026-08-23 COMPLETE

1. ✅ **sup-005**: ClaimSpan + extract_claims — regex-free sentence
   scanner with 8 conservative success-phrase classes mapping to
   Tests/Build/Bench/Regression kinds.
2. ✅ **sup-006**: link_claims(claims, evidence) → LinkReport; a claim
   links only when same-kind ok evidence exists. run_turn's success path
   now extracts+links claims on the final text; unlinked spans log a
   warn (observability only — gating is sup-007+).

Verification: full workspace suite green — **387 tests, 0 failed**
(384 → 387). Ledger: 67 IMPLEMENTED.

Next work continues → sup-007 fake-completion gating, mem-005 extraction,
or the ui thread list panel.

## core-021/022/025 Thread Management — 2026-08-23 COMPLETE

1. ✅ **core-021**: additive Command::ListThreads → Event::ThreadList with
   z-protocol ThreadInfo {id, title, message_count, updated_ms}, sorted
   most-recent-first.
2. ✅ **core-022**: RenameThread (120-char clamp), DeleteThread (map +
   data_dir/threads/<id>.json removal), DuplicateThread (deep clone under
   new id + persist); each emits a refreshed ThreadList. Active-turn
   delete is rejected (core-024 satisfied structurally).
3. ✅ **core-025 (partial)**: Runtime tracks most_recent_restored at
   startup; `most_recent_thread()` accessor ready for app wiring.
   core-023 tombstones deferred to a journal-backed pass.

Verification: full workspace suite green — **384 tests, 0 failed**
(379 → 384). Ledger: 65 IMPLEMENTED.

Next work continues → ui thread list panel on ThreadList events, mem-005
extraction, or sup-006 claim linking.

## mem-001..004 Memory Architecture — 2026-08-23 COMPLETE

1. ✅ **mem-001/003**: `z-core/src/memory.rs` — MemoryRecord
   {id, layer, content, provenance{kind, ref, thread_id, turn_id, ts_ms},
   confidence, status, superseded_by}; constructor enforces non-empty
   provenance and 0..=1 confidence (Result, never panic). record() appends
   additive JournalKind::MemoryRecorded events best-effort.
2. ✅ **mem-002**: MemoryStore writes per-layer last-line-wins JSONL views
   under data/memory/<layer>.jsonl; journal remains the fail-loud truth.
3. ✅ **mem-004**: MemoryView::fold over the journal; live() returns
   Promoted, non-superseded tips only.

Verification: full workspace suite green — **379 tests, 0 failed**
(373 → 379). Ledger: 64 IMPLEMENTED.

Next work continues → mem-005 candidate extraction from journaled turns,
mem-006 consolidation pass, or core-021/022 thread management.

## ui-030 Sidebar Scaffold — 2026-08-23 COMPLETE

1. ✅ **ui-030**: WorkspaceView gains `sidebar_items: Vec<(label, hint)>`
   mirrored from ProjectIndexed (view invents nothing); the ADR-0019
   dispatch arm draws nav rows plus index summary rows (label in BASE,
   hint in LABEL typography) inside frame.rect(PanelId::Sidebar).
   Visual suite stays green.

Verification: full workspace suite green — **373 tests, 0 failed**
(371 → 373). Ledger: 60 IMPLEMENTED.

Next work continues → ui-040 evidence badges, mem-006..009 memory stores
per ADR-0014, or core-021/022 thread management commands.

## idx-004 Tree-Sitter Rust Grammar — 2026-08-23 COMPLETE

1. ✅ **idx-004**: first authorized dependency add per ADR-0007 —
   tree-sitter 0.26.13 + tree-sitter-rust 0.24.2 (+ transitives
   streaming-iterator, tree-sitter-language), no wasm feature. New
   `z-core/src/symbols.rs`: extract_rust_symbols over six node kinds with
   catch_unwind panic containment; impl items named by trait/type operand;
   dedup is (name, kind) so a trait and its impl coexist. repo.rs routes
   .rs files through tree-sitter with the regex scan kept as fallback.
5 new tests; malformed input never panics (tested).

Verification: full workspace suite green — **371 tests, 0 failed**
(367 → 371). Ledger: 59 IMPLEMENTED.

Next work continues → idx-005/006 TS/JS + Python grammar packs, or ui-030
sidebar scaffold on the new panel seam.

## ctx-003 Budget Gate + ADR-0020 — 2026-08-23 COMPLETE

1. ✅ **ctx-003**: `enforce_budget(msgs, budget)` in runtime.rs — second
   compaction gate at the end of build_request: byte-identical passthrough
   when under budget; otherwise maps messages to context layers (tool
   bodies → Ephemeral, final user → pinned Session, system → Prefix) and
   runs context::assemble, then maps back. trim_history stays primary.
2. ✅ **ADR-0020** (`docs/adr/0020-local-model-support.md`): local model
   runtime decision for prov-020..025.

Verification: full workspace suite green — **366 tests, 0 failed**
(362 → 366). Ledger: 58 IMPLEMENTED.

Next work continues → idx-004 tree-sitter Rust grammar pack (first dep add
per ADR-0007), or ui-001 ShellFrame/render_panel seam per ADR-0019, or
sup-006 claim linking.

## Context Engine Core + ADR-0019 — 2026-08-23 COMPLETE

1. ✅ **ctx-001/002**: `z-core/src/context.rs` — Layer enum
   (Prefix/Session/Turn/Ephemeral), ContextItem {layer, text, est_tokens},
   and the pure `assemble(items, budget)` allocator per ADR-0013: drop
   order Ephemeral → oldest Turn → oldest non-pinned Session; Prefix and
   the last Session message always survive; under-budget passes through
   unchanged. build_request rewiring deliberately deferred to a follow-up
   slice (regression risk isolation).
2. ✅ **ADR-0019** (`docs/adr/0019-ui-shell-architecture.md`): UI state
   flow + panel seam decision for ui-001..020.

Verification: full workspace suite green — **362 tests, 0 failed**
(355 → 362). Ledger: 57 IMPLEMENTED.

Next work continues → ctx-003 compaction trigger wiring into build_request,
or idx-004 tree-sitter Rust grammar pack (needs dep add), or ui-001 panel
seam per ADR-0019.

## Diff/Tests Evidence Hooks + ADR-0018 — 2026-08-23 COMPLETE

1. ✅ **sup-004**: capture hook records Diff evidence for fs_write and
   edit_patch calls (ok + first summary line).
2. ✅ **sup-003 (partial)**: classify_command in evidence.rs maps
   cargo test/npm test/pytest/go test commands to Tests kind (recorded
   instead of Build for those terminal_exec calls).
3. ✅ **ADR-0018** (`docs/adr/0018-protocol-versioning.md`): protocol
   evolution policy — additive-only discipline codified (append variant +
   round-trip test, never rename/reorder), strict enums stay until an
   external IPC boundary exists, git-history-as-schema for Personal scale.

Verification: full workspace suite green — **355 tests, 0 failed**
(353 → 355). Ledger: 55 IMPLEMENTED.

Next work continues → sup-006 claim linking, ctx-001 ContextItem assembly
per ADR-0013, or idx-004 tree-sitter Rust grammar pack.

## Doom-Loop Breaker + Retry Classification (core-013/014/016) — COMPLETE

1. ✅ **core-014**: per-turn HashMap<u64, usize> counter in run_turn keyed
   by fnv1a64(tool_name ++ raw arguments_json) (raw args fine in practice —
   ponytail note: canonicalize if a provider ever reorders keys).
2. ✅ **core-013**: escalation ladder per ADR-0017 — at N identical calls
   (doom_threshold setting, default 3) inject one steering StoredMessage
   ("Change approach or explain what you are waiting for."); at ≥2N fail
   the turn with the loop-detected message; both paths persist.
3. ✅ **core-016 (partial)**: classify_provider_error maps error strings to
   Network/RateLimited/ServerError/Auth/Other; Network/RateLimited/Server
   retry once (round==0) with 1s pre-sleep for RateLimited/ServerError;
   Auth/Other fail fast. Replaces the crude "stream read failed" check.

Verification: full workspace suite green — **353 tests, 0 failed**
(350 → 353). Ledger: 53 IMPLEMENTED.

Next work continues → sup-003/004 capture hooks, core-017..019 retry
journaling, or ctx-001 ContextItem assembly per ADR-0013.

## Evidence Records (sup-001/002) — 2026-08-23 COMPLETE

1. ✅ **sup-001**: `z-core/src/evidence.rs` — Evidence envelope
   {id, kind, thread_id, turn_id, ok, summary} with five kinds
   (Build/Tests/Diff/Bench/Regression); record() appends EvidenceRecorded
   journal events best-effort; EvidenceView folds payloads (malformed fails
   loud). Helper constructors encode pass semantics: build() => exit==0,
   tests() => failed==0.
2. ✅ **sup-002 (partial)**: terminal_exec tool calls now also record Build
   evidence through the journal already threaded into run_turn; exit code
   parsed from the output's "[exit code: N]" marker.

Verification: full workspace suite green — **350 tests, 0 failed**
(345 → 350). Ledger: 50 IMPLEMENTED.

Next work continues → sup-003 (test-runner parse) + sup-004 (diff capture)
hooks, then core-013..019 doom-loop/retry per ADR-0017.

## Token Cache + ADR-0016 (2026-08-23) — COMPLETE

1. ✅ **tok-003/004/005**: fs_read result cache — static map keyed by
   ("fs_read", root+NUL+raw-path, fingerprint), 128-entry cap, outputs
   bound to ≤12k chars; hits re-fingerprint at call time so changed bytes
   can never serve stale content; only fs_read cached (multi-file
   invalidation for search/list deferred with a ponytail note).
2. ✅ **tok-020**: peek_fingerprint added to the registry; unchanged
   re-reads append "(duplicate read of unchanged file)" so the model sees
   it is wasting tokens.
3. ✅ **ADR-0016** (`docs/adr/0016-supervision-evidence.md`): five evidence
   record types (Build/Tests/Diff/Bench/Regression) sharing one envelope
   drawn from existing capture points, appended as EvidenceRecorded journal
   events and folded into an EvidenceView reducer per ADR-0012's
   journal-is-truth posture.

Verification: full workspace suite green — **345 tests, 0 failed**
(341 → 345). Ledger: 48 IMPLEMENTED.

Next work continues → sup-002..004 capture hooks wiring evidence into
sandbox/test-runner/write paths per ADR-0016.

## Orchestrator Skeleton + ADR-0015 (2026-08-23) — COMPLETE

1. ✅ **orch-002**: TasksView::ready_set — Pending tasks whose deps are all
   Done (deps field added additively to TaskRecord; unknown deps block
   forever, safe default). Chain tests: A→B gating, Failed dep blocks.
2. ✅ **orch-003 (skeleton)**: Orchestrator thread with mpsc inbox +
   1s recv_timeout deadline-sweep placeholder per ADR-0012; EnqueueTask{id,
   body} runs nested task bodies on named "z-subagent" workers under the
   global cap of 2 concurrent (AtomicUsize test asserts the cap held);
   body Ok → Done, Err → Failed via TaskStore.
3. ✅ **ADR-0015** (`docs/adr/0015-token-economy.md`): stable prefix defined
   byte-exactly as system string + serialized tools array with a guard
   test; tool-output cache keyed by file fingerprints with invalidation on
   mismatch; redundant-read detection rides the existing registry.

Verification: full workspace suite green — **341 tests, 0 failed**
(337 → 341). Ledger: 44 IMPLEMENTED.

Next work continues → tok-003..005 cache implementation per ADR-0015, or
orch-004 budget enforcer, or core-013/014 doom-loop breaker.

## Reducer API + Task Store + ADR-0014 (2026-08-23) — COMPLETE

1. ✅ **jour-006**: `z-core/src/reducer.rs` — free `fold(path, init, f)`
   replaying a journal in order (no Reducer trait; two concrete views are
   the smaller design).
2. ✅ **jour-007**: ThreadsView — per-thread summary (title from first
   message, message count, last kind) folded from MessagePersisted/
   TurnStarted records.
3. ✅ **jour-008 + orch-001**: TasksView over additive JournalKind::
   TaskStateChanged records; TaskStore appends create/transition events
   with seq continuity; unknown kinds (Other) never break folds.

Verification: full workspace suite green — **337 tests, 0 failed**
(332 → 337). Ledger: 42 IMPLEMENTED.

Also this session:
- ✅ **ADR-0014** (`docs/adr/0014-memory-architecture.md`): memory records
  {id, layer, content, provenance, confidence, ttl, superseded_by} in
  JSONL layer stores under data/memory/ (journal pattern reused),
  provenance enforced at write time, linear supersede chains with
  retrieval picking live records; embeddings stay RESEARCH.

Next work continues → mem-006..009 on the ADR-0014 stores, or orch-002
ready-set computation on TasksView.

## Rollback + Write Grants + ADR-0013 (2026-08-23) — COMPLETE

1. ✅ **edit-014**: checked_write stages the target's current bytes to a
   sibling `.{name}.{pid}.rollback.tmp` (written via atomic_write itself)
   BEFORE the rename; one generation kept; `rollback_last(path)` restores
   it exactly and clears it.
2. ✅ **edit-016/017**: per-file write grants in Shared keyed by canonical
   path with owner thread_id — overlap rejection at grant time ("File is
   being edited by another task."), reentrant for the same thread,
   acquired before every Write-risk tool call and released after.
3. ✅ **ADR-0013** (`docs/adr/0013-context-engine.md`): context becomes a
   typed ContextItem stream (Prefix/Session/Turn/Ephemeral layers with
   priority + fnv1a64 freshness) assembled by one pure function that
   build_request calls, allocated by strict priority order with caps.

Verification: full workspace suite green — **332 tests, 0 failed**
(327 → 332). Ledger: 38 IMPLEMENTED.

Next work continues → jour-006..008 reducer API (feeds orch-001), then
edit-018 blind-write flagging.

## Settings Module + core-011/012 + ADR-0012 (2026-08-23) — COMPLETE

1. ✅ **set-001/003**: `z-core/src/settings.rs` — `Settings{max_tool_rounds,
   approval_timeout_secs}` defaulting to the former consts (24 / 300);
   `load()` tolerates missing/malformed/out-of-range values per-field with
   warnings (never fails); `store()` writes via atomic_write; snapshot
   cache (`Mutex<Arc>`) cloned once per turn start per ADR-0011.
2. ✅ **core-011/core-012**: run_turn and the approval gate now read the
   turn-start snapshot instead of hardcoded consts. Test proves a
   settings.json with max_tool_rounds=2 actually stops a tool-looping
   provider at 2 rounds ("stopped after 2 tool rounds").
3. ✅ **ADR-0012** (`docs/adr/0012-subagent-orchestration.md`): task records
   exist only as journal events folded by jour-008's reducer (no tasks.json);
   orchestrator = dedicated thread on the ADR-0009 actor pattern; sub-agent =
   nested run_turn with restricted grants + budget caps, not a second Runtime;
   L0..L3 isolation ladder mapped to concrete ledger features.

Verification: full workspace suite green — **327 tests, 0 failed**
(322 → 327). Ledger: 35 IMPLEMENTED.

Next work continues → edit-014..018 (rollback staging + write grants) or
jour-006..008 reducer API, then orch-001 task store on the journal.

## edit_patch Tool + ADR-0011 (2026-08-23) — COMPLETE

1. ✅ **edit-008..013**: `edit_patch` tool — multi-block sequential patching
   against an in-memory copy: exact substring match per block, then a narrow
   whitespace-normalized fallback (summary notes the normalization); missing
   anchor aborts the WHOLE patch before disk contact with the ZD-E-0061
   message; safety path shared with fs_write via one extracted
   `checked_write` (scope → fingerprint stale-check → parent dirs → atomic
   write → re-arm). Wired into definitions/classify(Write)/describe.
2. ✅ **ADR-0011** (`docs/adr/0011-settings-and-provider-router.md`):
   settings in versioned `data/settings.json` ({version, values}) keyed by
   spec ids, schema-owned defaults, snapshot cache access (Mutex<Arc> cloned
   once per turn, mirroring ADR-0009 semantics); provider router = registry +
   failover hook seams behind the existing single-active ConfigureProvider,
   keeping protocol additive. Unblocks set-002/003 → core-011/012/015 and
   prov-004..008.

Verification: full workspace suite green — **322 tests, 0 failed**
(316 → 322). Ledger: 31 IMPLEMENTED.

Next work continues → set-002/003 + core-011/012 wiring (settings now have
a contract), then edit-014..018 (rollback staging, write grants).

## Atomic Writes + Git Read Tools (2026-08-23) — COMPLETE

1. ✅ **edit-004/005**: `z-core/src/atomic_write.rs` — same-dir temp
   (`.{name}.{pid}.{n}.tmp`) → write → `sync_all` → rename; Windows rename
   retried 5×50 ms for sharing violations; Unix best-effort parent-dir
   sync after rename; temp removed on every error path. `fs_write` routes
   through it (fingerprint stale-check/re-arm untouched). Race test: 8
   readers × 50 writes — every observed read is old-or-new, never partial.
2. ✅ **edit-022..024**: git_status / git_diff / git_log read tools behind
   a single `run_git` facade per ADR-0008 — direct argv only, LC_ALL=C,
   GIT_OPTIONAL_LOCKS=0 on reads, exit code authoritative, stderr carried
   into failure messages. porcelain=v2 -z status, numstat -z diff,
   %H%x00-separated log with clamped limits. classify() = ReadOnly.
   Tests use real temp repos (git init + commit) and skip cleanly if git
   is absent.

Verification: full workspace suite green — **316 tests, 0 failed**
(307 → 316). Ledger: 25 IMPLEMENTED.

Next work continues → edit-006/007 crash-simulation tests for the atomic
path, then edit-008+ patch tool (edit_patch) per ADR-0010.

## Safe-Editing Foundation + ADR-0010 (2026-08-23) — COMPLETE

1. ✅ **edit-001**: `z-core/src/fingerprint.rs` — hand-rolled FNV-1a 64-bit
   (spec vectors tested: empty/a/foobar), `file_fingerprint` streams 8 KiB
   chunks (no whole-file loads), plus a per-(thread,path) fingerprint
   registry (`record_fingerprint` / take-on-read `take_fingerprint`;
   unbounded map noted as fine at personal scale).
2. ✅ **edit-002**: `ToolInvocation` gained `thread_id`; runtime passes the
   real thread id; fs_read records the file's fingerprint after a successful
   read. Empty thread_id (tests) never records.
3. ✅ **edit-003** (ZD-E-0060): fs_write refuses when the recorded
   fingerprint differs from current on-disk content — error text is the §51
   canonical sentence. Never-read files stay writable (blind writes; edit-018
   flags later). Successful writes re-arm the fingerprint so consecutive
   agent edits work.

Verification: full workspace suite green — **307 tests, 0 failed**
(302 → 307; fingerprint vectors, registry semantics, fs_read recording,
stale-write refusal incl. user-edit preservation and write re-arming).

Also this session:
- ✅ **ADR-0010** (`docs/adr/0010-safe-editing-pipeline.md`): all writes go
  through one atomic helper — same-dir temp → fsync → rename → dir-sync,
  Windows via MoveFileExW semantics with sharing-violation retries; patches
  apply in memory (exact match, whitespace-normalized fallback, ZD-E-0061 on
  absent anchor, abort-before-disk); rollback = captured old bytes (git not
  assumed); write grants live in Shared keyed by canonical path;
  multi-file txns are validate-all → stage-all → apply.

Next work continues → edit-004 atomic write helper behind the existing
fs_write path, then edit-008+ patch tools per ADR-0010.

## Journal Wiring + ADR-0009 (2026-08-23) — COMPLETE

1. ✅ **jour-024**: Runtime owns `journal: Mutex<Journal>` at
   `data_dir/journal/runtime.jsonl`, opened with `open_resuming` from the
   replayed max seq (restart-safe, no seq reuse). Every command received is
   journalled as CommandReceived with a SHAPE-only payload — message text,
   provider config values and API keys never enter the journal (enforced by
   a dedicated test). TurnStarted records on SendMessage.
2. ✅ **jour-029**: MessagePersisted records on every thread persist point
   (success, cancel, and error paths) with new-message count + last role.
   Journal append failures log a warning and never fail the turn.

Verification: full workspace suite green — **302 tests, 0 failed**
(298 → 302; journal wiring tests incl. restart seq continuity and
secret-shape assertions). Security scan clean after allowlisting the
journal test's synthetic placeholder (the test itself asserts the fake
key never persists).

Also this session:
- ✅ **idx-001/002** (ADR): `docs/adr/0009-repository-index-actor.md` —
  single owner thread (`z-index`) holding ALL mutable index state, fed via
  std::sync::mpsc IndexCommand inbox; readers get immutable snapshots
  through a Mutex<Arc<IndexSnapshot>> swap; crossbeam-channel evaluated
  and declined (no new deps); watcher (notify) stays deferred to its own
  evaluation; initial indexing moves off the command loop. Unblocks
  idx-012/013, idx-026..029, idx-035, repo-map v2, go-to-def tools.
- Ledger statuses updated through `tools/gen_tasks.py`: 17 IMPLEMENTED.

Next work continues → idx-004 Rust grammar pack behind the idx-001 actor,
or edit-001 fingerprint utilities (safe-editing foundation), in dependency
order per the ledger.

## Journal Slice + ADR-0008 (2026-08-23) — COMPLETE

Vertical slice "Exact Next Tasks #2" (JSONL task journal) shipped and verified:

1. ✅ **jour-001**: `z-core/src/journal.rs` — `Record { seq, ts_ms, kind,
   thread_id, payload }`, `JournalKind` enum (snake_case kinds + `Other(String)`
   escape hatch so additive evolution never breaks replay), `Journal::open`.
   Verified empirically that pinned serde_json 1.0.151 round-trips u128.
2. ✅ **jour-002**: O_APPEND writer (`OpenOptions::append(true)`); flush per
   record; `sync_all()` every N records (configurable, default 32) + explicit
   `flush_and_sync()`; crash window bounded by N; `records_since_sync`
   observable; `open_resuming(dir, name, last_seq)` for reopen continuity.
3. ✅ **jour-005**: `Journal::replay(path)` — ordered line parse, empty-line
   skip, malformed line fails loud with line number (repair = jour-011 later);
   `first_seq_gap(records)` helper ready for jour-010.

Verification: full workspace suite green — **298 tests, 0 failed**
(288 → 298; z-core now 50 with 10 journal tests incl. 500-record burst
round-trip and fsync-policy observability). No new dependencies.

Also this session:
- ✅ **edit-025** (ADR): `docs/adr/0008-git-access.md` — git access via a
  single internal facade over the user's installed git CLI: direct argv
  (never shell strings), one serialized worker thread (avoids index.lock
  contention), machine-readable output only (`--porcelain=v2 --branch -z`,
  `-z` numstat/raw, `%x00` log format), `GIT_OPTIONAL_LOCKS=0` for reads,
  approved writes run without identity overrides (they are the user's
  writes), version gate ≥2.20 at project open. Rejected: git2-rs now
  (C dep chain + libssh2 CVE-2026-5917 CVSS 9.6 in ssh-backend builds),
  gix now (maturity revisit trigger). Unblocks edit-026..028, orch-007..009.
- Ledger statuses updated through `tools/gen_tasks.py`: 13 IMPLEMENTED.

Next work continues per "Exact Next Tasks" below → item 3: wire journal into
runtime lifecycle (command_received/turn_started/turn_finished/message_
persisted records — jour-024/029 family), then idx-001 index actor.

## Steering Slice (2026-08-23) — COMPLETE

Vertical slice "Exact Next Tasks #1" (steering queue) shipped and verified:

1. ✅ **core-020**: `Command::EnqueueMessage { thread_id, text }` +
   `Event::SteeringQueued { thread_id, depth }` in z-protocol (additive;
   serde round-trip + snake_case tag tests).
2. ✅ **core-004**: per-thread steering queue (`VecDeque<String>`) in
   `Shared`, capped at `STEERING_QUEUE_CAP = 16` (oldest dropped under
   pressure — newest intent wins).
3. ✅ **core-005**: `enqueue_message` on the command loop; empty/whitespace
   text ignored; depth event emitted per enqueue.
4. ✅ **core-006**: turn worker drains the queue at the top of every tool
   round after round 0, before building the next provider request; injected
   as one combined user message; persisted with the thread.
5. ✅ **core-007**: combine gate — all texts drained in one pass merge into
   a single `User steering:\n…` history entry (one marker, N lines).
6. ✅ **core-008**: `CancelTurn` clears that thread's pending steering so
   stale guidance never leaks into a later turn.
7. ✅ App layer: composer routes through `EnqueueMessage` while
   `streaming == true` (SendMessage otherwise); `SteeringQueued` drives a
   `steering_depth` view field + status line ("steering queued (N pending)").

Verification: full workspace suite green — **288 tests, 0 failed**
(baseline was 278; +10: z-core steering tests ×7 incl. scripted-provider
mid-turn injection proof, z-protocol serde ×2, zero-app composer routing +
depth indicator ×2... net +10 across crates). Clippy warnings unchanged vs
baseline (8 pre-existing; none introduced).

Also this session:
- ✅ **idx-003** (ADR): `docs/adr/0007-tree-sitter-indexing.md` — tree-sitter
  0.26.x accepted for M2 (MSRV 1.77 < workspace floor 1.85), grammar packs
  incremental (Rust first per idx-004), wasm feature banned, per-file
  catch_unwind mandated, TS/TSX gated on upstream staleness (>21 months at
  evaluation time). Ledger statuses updated via `tools/gen_tasks.py` (the
  generator, not the generated file).
- Reference sync: grok-build re-cloned to `references/external/grok-build`
  (blob-filtered), HEAD `07b2f71` recorded in THIRD_PARTY ledger. Its
  `xai-interjection-core` confirms our shape: capped queue + drain-at-hook
  + single framing note per injected batch. No code copied.

Next work continues per "Exact Next Tasks" below → item 2: JSONL task
journal (jour-001..005), then idx-001 index actor.

## Canonicalization Slice (2026-08-23) — COMPLETE

This session published the repository's canonical documentation layer:

1. ✅ `docs/Z-DESKTOP-MASTER-SPEC.md` — full canonical specification
   (~4,800 lines, §1–§142): identity, principles, architecture domains,
   subsystem specs, tool/protocol catalogs, milestones M0–M10 with
   acceptance criteria, security/threat model, performance budgets,
   detailed designs for every planned domain, glossaries, normative
   indexes.
2. ✅ `docs/Z-DESKTOP-TASKS.md` — engineering task ledger: **737 tasks**
   across 40 domains, each with id/status/deps; dependency graph validated
   programmatically by `tools/gen_tasks.py` (deterministic generator;
   edit the generator, not the file).
3. ✅ `docs/Z-DESKTOP-REFERENCE-RESEARCH.md` — research & clone playbook
   (rules of engagement, license policy, study targets).
4. ✅ `README.md` — public landing page (honest status, build/test
   commands, layout, no license claim).
5. ✅ `.gitignore` hardened + `tools/security_scan.py` secret scanner.
6. ✅ 19 operational skills under `skills/<name>/SKILL.md`.

Publication: git repo initialized at workspace root; pushed to
github.com/mhrsdev/z-desktop (branch main). Baseline commit
`45935d6f84a5e6f87724bf69599bb0ffa9b37314` (75 files, 30,080 lines);
publication verified via `git ls-remote` — remote SHA matches local HEAD.

Next work continues per "Exact Next Tasks" below (steering queue →
journal → index actor), now tracked as ledger IDs core-005..core-008,
jour-001..jour-005, idx-001..idx-003.

## Current Phase

Phase 2 — vertical slices in progress (continuous execution mode).

## Current Task

Completed slices (all verified by the full workspace suite):
1. ✅ Research foundation (grok-build clone, dissection, capability matrix,
   170-capability backlog, reuse ledger) — see docs/research/.
2. ✅ **Sandbox slice**: `z-core/src/sandbox.rs` — cross-platform process-tree
   guard. Windows: Job Objects (KILL_ON_JOB_CLOSE + TerminateJobObject);
   unix: own process group + group SIGKILL. Reader threads prevent pipe
   deadlock; partial output captured on timeout; output capped (8 MiB stdout /
   2 MiB stderr); timeout default 120 s, hard ceiling 600 s. `terminal_exec`
   now routes through it and accepts optional `timeout_ms`.
3. ✅ **Redaction slice**: `z-core/src/redact.rs` — fingerprinted secret
   redaction (`[redacted:label…xy12]`) for provider tokens (sk-/sk-ant-/xai-/
   gh*_/AKIA/AIza), bearer headers, and api_key/secret/token/password
   assignments. Wired into `terminal_exec` output (nothing leaves the tool
   boundary unredacted).
5. ✅ **Token estimation + context budgeting** (matrix #11, cap #63):
   - `z-core/src/tokens.rs`: single-pass heuristic estimator — chars/4
     baseline, CJK ≈ 1 token/char, symbol-density correction for code,
     per-message structural overhead, tool-def estimator. 8 unit tests
     including a <100 ms check on ~1 MiB input.
   - Runtime budget gate in `build_request`: fixed cost (system prompt +
     repo map + tool schemas) is estimated first; history gets the remainder
     of (128k − 12k completion reserve). Over-budget history is trimmed at
     CLEAN turn boundaries only (real user messages), so assistant
     tool_calls can never be separated from their result carriers.
   - `trim_history` had two real bugs caught by its own tests during
     development: suffix accumulation ran in the wrong direction, and the
     boundary search kept the MINIMAL instead of MAXIMAL fitting history.
     Both fixed; regression tests retained.

Verification: full workspace suite green — **280 tests, 0 failed**
(z-core 36, z-gpui 109, z-protocol 2, z-shell 48, z-tokens 24, zero-app 53,
integration 8).

Next slices in dependency order:
6. Steering queue: `Command::EnqueueMessage` + combine gates in z-protocol +
   runtime drain between tool rounds (cap #3).
7. SQLite task journal (event sourcing) for durable sessions (cap #4).
8. Tree-sitter repo index actor (matrix #3, cap #41).
   - Job Object assignment race ELIMINATED: children now spawn with
     CREATE_SUSPENDED, are assigned to the kill-on-close job, and only then
     resumed via Toolhelp32 thread enumeration + ResumeThread. The child's
     first instruction already executes inside the job — no escape window.
     Any failure before resume terminates the suspended child.
   - Orphan leak fixed: attach failure now explicitly kills+waits the child
     (dropping `Child` does not kill on Windows).
   - Regression tests added: detached grandchild (`start /b`) dies on both
     timeout AND normal parent exit (verified via tasklist polling);
     ~64 MiB stdout collapses into the 8 MiB cap without unbounded growth.
   - Verified: no breakaway flag on the job → shell wrappers cannot detach;
     reader threads always join at EOF; handles closed on every error path.

Verification: full workspace suite green — **268 tests, 0 failed**
(z-core 24, z-gpui 109, z-protocol 2, z-shell 48, z-tokens 24, zero-app 53,
integration 8).


## Last Completed Work

- z-core tool runtime fixed and hardened:
  - `scoped()` rewritten as lexical normalisation against the canonicalised,
    verbatim-stripped project root. Rejects `..` traversal, accepts
    not-yet-existing write targets, immune to Windows `\\?\` prefix issues.
  - `strip_verbatim()` builds its prefix from char codes so escape handling in
    editors/tooling can never corrupt it.
  - `fs_search` returns forward-slash relative paths.
- Repo index bug fixed: `rel_path` was reset to empty on every rescan, forcing
  full reparse each time and producing broken map text.
- App layer: Escape now optimistically leaves streaming state on cancel;
  `TextDone` event handled; u64→u32 casts for context entries; imports fixed.
- Full workspace test suite green:
  - z-core 11, z-gpui 109, z-protocol 2, z-shell 48, z-tokens 24, zero-app 53,
    plus 8 doc/integration tests = **255 passed, 0 failed**.

## Architecture Summary

Workspace root: `z desktop/` (Rust workspace).

| Crate      | Role                                                        |
|------------|-------------------------------------------------------------|
| z-protocol | Contracts: Command/Event enums, ProviderConfig, Risk, Id    |
| z-core     | Agent Runtime: threads, turns, tools, providers, repo index |
| z-shell    | Workspace model: layout regions, panels, presets, view state|
| z-gpui     | ZeroGPUI runtime: window, renderer, scene, a11y, timing     |
| z-tokens   | Design tokens: color, spacing, typography, theme            |
| z-app      | View layer: turns shell model into scenes, wires runtime    |

Data flow: UI → Command channel → Runtime thread → Event channel → event pump
thread → EventQueue → drained at frame start → scene rebuild.

## Run Commands

```text
cargo check --manifest-path "z desktop/Cargo.toml" --workspace
cargo test  --manifest-path "z desktop/Cargo.toml" --workspace
cargo run -p zero-app --manifest-path "z desktop/Cargo.toml" -- --check
cargo run -p zero-app --manifest-path "z desktop/Cargo.toml" -- --shot <dir>
```

## Known Issues / Debt

- Compiler warnings (dead code): `StreamOutcome::push`, `SKIP_DIRS`,
  `Conversation::from_thread`, `PendingApproval` fields, unused `provider`
  param in `build_request`. Clean up or wire up deliberately.
- Provider config has no Settings UI yet (BYOK via data/config.json only).
- Single conversation thread; multi-thread UI not wired.
- deepseek-harness cloned at workspace root (user request); treat as research
  reference, not part of Z Desktop source tree.
- Redaction covers tool output; extend to runtime logs + persisted events when
  the journal lands.
- Sandbox: mid-run cancellation of an in-flight tool call is not possible yet
  (tool runs synchronously on the runtime thread); needs a cancel flag checked
  in the wait loop when steering/cancel lands.
- `\\?\` verbatim-prefix and `\n` escape literals must be built via char codes
  (char::from(92), char::from(10)) — the file-saving pipeline mangles raw
  backslash escapes in tool-written content.

## Do Not Redo

- Do NOT rewrite `scoped()` again — it is correct and tested.
- Do NOT "fix" the char-code prefix in `strip_verbatim` back to string
  literals; the file-saving pipeline mangles backslash escapes.
- Do NOT add Team features (Personal-only mandate).
- Do NOT grow LOC artificially (no boilerplate/placeholder modules).

## Exact Next Tasks

1. ~~Steering queue~~ — COMPLETE (see Steering Slice above; core-020, 004–008).
2. Task journal: append-only JSONL event log under data/journal/ first (no
   new dep); record command/event lifecycle; replay on startup; crash-
   recovery ordering tests. Upgrade path to SQLite documented (jour-001..005).
3. Tree-sitter repo index actor (matrix #3, cap #41) after journal lands;
   ADR-0007 already fixes the dependency decision (idx-004 Rust pack first).

Resume command: cargo test --manifest-path "z desktop/Cargo.toml" --workspace
