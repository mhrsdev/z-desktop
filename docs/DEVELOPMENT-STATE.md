# Z Desktop Personal — Development State

> Session-resume protocol file. Every session MUST read this before asking the
> user anything, and update it before ending work. Information that lives only
> in a conversation is lost information.

Last updated: 2026-08-23

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
