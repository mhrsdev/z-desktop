# Z Desktop Personal — Development State

> Session-resume protocol file. Every session MUST read this before asking the
> user anything, and update it before ending work. Information that lives only
> in a conversation is lost information.

Last updated: 2026-08-23

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
