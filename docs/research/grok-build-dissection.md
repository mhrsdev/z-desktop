# Grok Build (xai-org/grok-build) — Engineering Dissection

Research reference only. This repository is NOT part of the Z Desktop source
tree and no code is copied from it without a license check recorded in
`docs/THIRD_PARTY_RESEARCH_AND_REUSE.md`.

- Cloned at: `references/external/grok-build`
- Upstream monorepo revision: `SOURCE_REV` = `7d67deacbeb1c1093fdb4f9bcbfab2630e18a6aa`
- License: first-party code **Apache-2.0**; vendored third-party code keeps its
  own licenses (`THIRD-PARTY-NOTICES`, per-crate notices).
- Notable: the tree contains **in-tree source ports of openai/codex and
  sst/opencode tool implementations** — i.e. Grok Build itself reuses competitor
  tool designs under Apache terms. That validates our own "dissect, adapt,
  attribute" approach.
- Shape: ~90-crate Rust workspace. TUI product (`xai-grok-pager`), agent runtime
  (`xai-grok-shell`), tools (`xai-grok-tools`), workspace layer
  (`xai-grok-workspace` + daemon), plus many small leaf crates.

## Subsystem dissections

Format per subsystem: Problem → Architecture → Strength → Weakness → Z Desktop
Adaptation → Decision (Reuse / Adapt / Rewrite / Reject / Improve / Replace).

### 1. Tool abstraction (`xai-tool-runtime`, `xai-tool-protocol`, `xai-tool-types`)

**Problem:** one uniform way to define, describe, dispatch and stream every
tool across blocking and streaming execution.

**Architecture:** a single `Tool` trait with typed `Args` (JSON-schema via
schemars) and typed `Output`. `execute` is the canonical streaming entry point
(`ToolStream = [Progress*, Terminal]`); default impl wraps a simpler blocking
`run` into a one-item stream so simple tools ignore streaming entirely.
Type erasure goes through `ToolDyn`; the trait itself is not dyn-compatible
(RPITIT with Send bound, zero boxing on the hot path). Context objects
(`ListToolsContext`, `ToolCallContext`) let descriptions and listing be
per-turn dynamic (`should_list`) — enabling lazy/dynamic tool manifests.

**Strength:** typed args+outputs end-to-end; streaming is opt-in but uniform;
dynamic listing is built into the trait seam (token economy by design);
trait-object safety is explicitly tested.

**Weakness:** async-everywhere; our desktop core is currently thread-based and
sync, and an async runtime would buy complexity we do not need yet.

**Adaptation:** keep Z Desktop's sync `execute(ToolInvocation) -> ToolOutput`,
but adopt three ideas: (a) typed args via serde + JSON schema from a derive,
(b) a `Progress` channel so long tools can stream status before finishing,
(c) `should_list(ctx)` per-turn manifest filtering for token economy.

**Decision: Adapt** (ideas, not code).

### 2. Repository intelligence (`xai-codebase-graph`)

**Problem:** fast go-to-definition/references and incremental repo indexing
without an LSP server.

**Architecture:** tree-sitter queries per language (rust/ts/js/python/go),
rayon parallel initial index, memory-mapped IO, binary-content detection,
on-disk cache with workspace lock, and an `IndexManager` actor: file events in
(`FileEvent::modified/...`), query commands answered in-place without cloning
the full index; scope graph with symbols/references/local imports; memory
regression tests (`incremental_memory.rs`, `memory_integration.rs`).

**Strength:** actor-based incremental manager is exactly right for a desktop
app that must stay responsive while indexing; cache + lock discipline;
memory tests as first-class citizens.

**Weakness:** TUI-only consumers; no semantic/embedding layer here (that lives
in xai-grok-memory).

**Adaptation:** replace Z Desktop's regex-heuristic `repo.rs` symbol extraction
with a tree-sitter-based indexer behind the same actor pattern. Our existing
`(mtime,size)` stamping and map_text budget stay; the extractor upgrades.

**Decision: Adapt** (architecture) / **Rewrite** (implementation, licensed
Apache-2.0 so reuse is possible later with attribution if we choose).

### 3. Compaction engine (`xai-grok-compaction`)

**Problem:** conversations exceed context; history must shrink without losing
the active task or breaking tool-call pairing.

**Architecture:** transport-agnostic core with trait seams
(`CompactionItem`, `ItemTokenCounter`, `CompactionSampler`, observers). Three
styles: `code_compaction` (whole-session full-replace summary),
`intra_compaction` (tail-keep per-step), `inter_compaction` (chunked
between-turn). Tool-pair-safe tail-keep selection shared by styles; dedicated
compaction model override chain; degenerate-summary and context-length-error
classification; user-query preservation and validation.

**Strength:** policy/prompt/selection separated from host wiring; explicit
failure taxonomy; validation of compacted history.

**Weakness:** heavy crate surface; some styles are chat-product-specific.

**Adaptation:** Z Desktop needs compaction before long-horizon tasks. Adopt:
full-replace summarization with a cheap model, tool-pair-safe selection,
degenerate-summary retry, and a `<system-reminder>` injection format.

**Decision: Adapt.**

### 4. Steering / interjection (`xai-interjection-core`, `xai-prompt-queue`)

**Problem:** user types while the agent works; messages must not be lost or
naively appended mid-tool-call.

**Architecture:** an interjection buffer + event queue formats queued user
input as interrupt notes / follow-up turns with framing rules; prompt queue has
explicit combine gates (`can_merge_front/follower`) so consecutive queued
prompts merge into one turn when safe, with combined-display metadata.

**Strength:** treats "user talks mid-run" as a first-class protocol, not a UI
afterthought; merging saves tokens and round-trips.

**Weakness:** wire-format coupling to their chat product.

**Adaptation:** add a `Command::EnqueueMessage` + queue-combine gate to
z-protocol; runtime drains the queue between tool rounds (never mid-call).

**Decision: Adapt.**

### 5. Memory (`xai-grok-memory`)

**Problem:** durable project knowledge beyond the conversation.

**Architecture:** observation capture → chunker → embeddings → indexed storage
with MMR re-ranking, query expansion, archive/flush lifecycle, watcher-driven
updates, and a `dream` module (background consolidation pass, with locking).

**Strength:** MMR + query expansion are cheap wins for retrieval quality;
"dream" consolidation matches our idle-time mandate (#143).

**Weakness:** embedding backend assumptions; no user inspection surface here.

**Adaptation:** design z-core memory as layers (working/session/project/
semantic) with provenance + confidence + TTL per mission §18; borrow MMR and
idle-time consolidation concepts.

**Decision: Adapt concepts; implement natively.**

### 6. Workspace layer (`xai-grok-workspace*`, `xai-fast-worktree`, `xai-gix-status`, `xai-hunk-tracker`)

**Problem:** host FS, VCS state, execution, checkpoints, and parallel isolated
worktrees.

**Architecture:** separate daemon/client/types crates (workspace service out-
of-process), gix-based git status, fast worktree creation for parallel agents,
hunk tracking for edit attribution.

**Strength:** worktree-per-agent enables true parallel Best-of-N; hunk tracker
supports human+agent co-editing conflict detection.

**Weakness:** daemon adds operational complexity; Windows support is
best-effort upstream.

**Adaptation:** Z Desktop Personal should start in-process, but keep the
service boundary so a daemon split is possible later. Worktrees power our
parallel-agent slice (#32/#33).

**Decision: Adapt architecture; implement in-process first.**

### 7. Safety & ops crates

- `xai-grok-sandbox` — OS-level execution isolation (JobObjects on Windows).
  **Adapt**: wrap our terminal_exec with Job Object limits + timeout.
- `xai-grok-secrets` — secret storage/redaction. **Adapt**: DPAPI-backed store,
  redaction filter on all tool output and logs.
- `xai-circuit-breaker` — provider failure backoff. **Adopt** concept for
  provider retries (we already have one retry; make it principled).
- `xai-token-estimation` — local token counting. **Adopt** for budgeting
  before sending (context engine dependency).
- `xai-sqlite-journal`, `xai-session-events`, `xai-session-search` — durable
  event sourcing + resume + search over past sessions. **Adopt**: this is our
  Task Journal / checkpoint foundation (#93/#94/#129).
- `xai-agent-lifecycle`, `xai-subagent-resolution` — sub-agent spawn/resolution
  rules. **Adapt** for multi-agent phase with anti-spawn-spam defaults.
- `xai-system-power` — power/battery awareness. **Adopt** for adaptive
  performance on laptops (#25).
- `xai-grok-hooks` + `xai-hooks-plugins-types` + plugin marketplace/trust —
  hooks and plugin trust model. **Adapt**: permission-scoped plugin registry
  with trust levels before any marketplace.

## Cross-cutting observations

1. **Small crates, hard seams.** Nearly every subsystem is a leaf crate with a
   documented trait seam. This is the model for our modular monolith: clear API,
   clear ownership, testable in isolation.
2. **Docs live with crates.** User guide ships inside the pager crate.
3. **They ported competitors' tools openly** (codex, opencode) under Apache —
   precedent for disciplined reuse with attribution.
4. **Windows is second-class upstream.** Z Desktop is Windows-first; our
   platform layer must own the differences they skip.
5. **Generated root Cargo.toml** — they treat workspace config as build output.

## Reuse ledger

Nothing reused yet. Candidate future reuse (all Apache-2.0, requires NOTICE +
change log): none decided. Concepts adapted are listed per-subsystem above.