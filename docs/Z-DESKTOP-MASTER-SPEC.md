# Z Desktop Personal — Canonical Master Specification

> **Document class**: Canonical product and architecture specification.
> **Status**: Living document. Implementation is the source of truth for
> what exists; this document is the source of truth for what Z Desktop IS
> and what it intends to become.
> **Audience**: Future engineering agents and human contributors who must
> understand the project without tribal knowledge.
> **Maintenance rule**: When reality diverges from this spec, either fix the
> code or fix the spec — never let them drift silently. Record the
> reconciliation in `docs/DEVELOPMENT-STATE.md`.

---

## Table of Contents

1. [Product Identity](#1-product-identity)
2. [Product Philosophy](#2-product-philosophy)
3. [Ultimate Goal](#3-ultimate-goal)
4. [Core Design Principles](#4-core-design-principles)
5. [Scale Vision](#5-scale-vision)
6. [Canonical Source Hierarchy](#6-canonical-source-hierarchy)
7. [Current Implementation Status](#7-current-implementation-status)
8. [Architecture Domains](#8-architecture-domains)
   - 8.1 Agent Kernel
   - 8.2 Agent Orchestration
   - 8.3 Sub-Agents
   - 8.4 Agent Supervision
   - 8.5 Evidence Engine
   - 8.6 Tools System
   - 8.7 Skills System
   - 8.8 MCP Integration
   - 8.9 Plugins & Extensions
   - 8.10 Repository Intelligence
   - 8.11 Safe Editing Engine
   - 8.12 Git & Worktrees
   - 8.13 Context Engine
   - 8.14 Token Economy
   - 8.15 Cache Architecture
   - 8.16 Memory Architecture
   - 8.17 Model Router
   - 8.18 Provider Layer
   - 8.19 Local Models
   - 8.20 Hardware Intelligence
   - 8.21 Resource Scheduler
   - 8.22 Terminal
   - 8.23 IDE Surface
   - 8.24 Internal Browser
   - 8.25 Search & Research
   - 8.26 Live Preview
   - 8.27 3D Workspace
   - 8.28 Game Development Support
   - 8.29 Diagram Engine
   - 8.30 Database Workspace
   - 8.31 API Workbench
   - 8.32 Data Workspace
   - 8.33 Automation Engine
   - 8.34 Workflow Engine
   - 8.35 Artifact Platform
9. [UI Platform Specification](#9-ui-platform-specification)
10. [Theme System](#10-theme-system)
11. [Dock & Layout System](#11-dock--layout-system)
12. [Settings: User Mode & Developer Mode](#12-settings-user-mode--developer-mode)
13. [Customization Depth](#13-customization-depth)
14. [Accessibility](#14-accessibility)
15. [Diagnostics, Observability & Tracing](#15-diagnostics-observability--tracing)
16. [Security Model](#16-security-model)
17. [Failure & Recovery](#17-failure--recovery)
18. [Persistence Architecture](#18-persistence-architecture)
19. [Updates & Distribution](#19-updates--distribution)
20. [Testing Strategy](#20-testing-strategy)
21. [Benchmarking Strategy](#21-benchmarking-strategy)
22. [Cross-Platform Strategy](#22-cross-platform-strategy)
23. [Performance Philosophy](#23-performance-philosophy)
24. [Competitive Landscape & Lessons](#24-competitive-landscape--lessons)
25. [Differentiating Capability Families](#25-differentiating-capability-families)
26. [Glossary](#26-glossary)

---

## 1. Product Identity

**Z Desktop Personal** is a local-first, desktop-native AI engineering
workspace: a single application in which a user — starting with its author —
codes, debugs, tests, researches, runs agents, understands repositories,
manages Git, uses terminals, and increasingly performs every other technical
workflow, without leaving the window.

### 1.1 What Z Desktop is NOT

Z Desktop is explicitly not any of the following, even though it contains
capabilities that overlap with each:

- **Not an AI chat app.** Conversation is one surface among many; chat
  history is not project state.
- **Not a coding copilot plugin.** It does not live inside an editor; it IS
  the workspace.
- **Not an IDE fork or Electron shell.** The UI stack is custom GPU-rendered
  Rust (ZeroGPUI), built for native feel and long-session efficiency.
- **Not a terminal wrapper.** The terminal is one panel in a larger system.
- **Not a thin model frontend.** Providers are interchangeable plumbing;
  the value is the surrounding engineering system.

### 1.2 Personal-first mandate

The current phase targets exactly one user class: **the project owner**.
This is a deliberate constraint, not a limitation:

- No multi-user accounts, teams, billing, or cloud sync are implemented,
  planned for near-term milestones, or allowed to complicate the core.
- "Team features" appearing anywhere in code or docs is a defect against
  this mandate (recorded in DEVELOPMENT-STATE "Do Not Redo").
- Personal-first buys speed: no auth layer, no tenant isolation, no
  server-side anything unless a capability genuinely requires it.
- The architecture must nonetheless keep the door open: personal data lives
  in well-defined local stores that could later sync or share without
  redesign.

### 1.3 Product name and versioning

- Product: **Z Desktop** (Personal edition during this phase).
- Workspace version: `0.1.0` (see `z desktop/Cargo.toml`).
- Versioning intent: semver for crates once external consumers exist;
  until then, the protocol crate (`z-protocol`) carries compatibility
  discipline internally (additive changes only).

### 1.4 One-sentence definition

> Z Desktop Personal is a calm, fast, local-first agentic desktop where a
> competent AI engineering team — supervised by one user — works on real
> projects inside a single native application.

---

## 2. Product Philosophy

### 2.1 The category: Personal AI Operating Workspace

The industry has produced chatbots, copilots, and agent CLIs. Z Desktop's
thesis is that the next unit of software is the **agentic workspace**: an
environment where autonomous and semi-autonomous agents are first-class
workers with tools, memory, supervision, evidence requirements, and durable
state — embedded in a desktop environment that itself provides the surfaces
those workers need (editor, terminal, browser, diagrams, databases).

Design consequences:

1. **Agents are workers, not features.** Every subsystem asks: how does this
   help an agent do real work reliably?
2. **The human is the supervisor.** Approval gates, evidence requirements,
   and observability exist because delegation without verification is how
   agents destroy trust and codebases.
3. **State outlives sessions.** Tasks, journals, memories, and caches
   survive restarts; a crashed session resumes instead of restarting.
4. **Everything is inspectable.** If the agent read it, decided it, wrote
   it, or claimed it, the user can see it.

### 2.2 Calm engineering instrument

The aesthetic and interaction philosophy: a precision instrument, quiet by
default, loud only when attention is required. Concretely:

- No gratuitous animation, color, or notification.
- Progress shown is real progress (streaming deltas, actual tool output).
- Errors explain recovery paths, not just failure.
- Density is respected: professionals prefer information over decoration.

### 2.3 Local-first where useful

- Source code, indexes, journals, memories, and caches live on disk under
  the user's control (`data/` directories, gitignored).
- BYOK (bring your own key): provider credentials stay local; nothing
  proxies through a Z Desktop server (there is no Z Desktop server).
- Cloud services may be *consumed* (model APIs), but none are *required*
  for the app to function offline except model calls themselves.

### 2.4 Honesty as an engineering requirement

- No fake progress, fake tests, fake completion, feature theater, LOC
  theater. These are recorded as explicit prohibitions in DEVELOPMENT-STATE
  and enforced culturally and (increasingly) programmatically via the
  Evidence Engine (§8.5).
- Documentation distinguishes IMPLEMENTED / PARTIAL / PLANNED / RESEARCH at
  all times (§7).

---

## 3. Ultimate Goal

A user should increasingly be able to remain inside Z Desktop for the full
span of technical work:

| Domain | Trajectory |
|---|---|
| Coding | agent-assisted editing with repository understanding |
| Debugging | integrated run/test/log loops with agent diagnosis |
| Testing | execution, selection, coverage awareness |
| Research | internal browser + search + source analysis |
| Browsing | embedded browser with agent access |
| Git | full VCS surface incl. worktrees and merge assistance |
| Terminal | first-class PTY panels agents can use |
| Project understanding | symbol/reference/dependency graphs |
| Architecture | diagramming from code and intent |
| APIs | request workbench with environment management |
| Databases | connection, schema browsing, query workbench |
| Diagrams | render + generate mermaid-class formats |
| UI development | live preview of web/UI artifacts |
| 3D | viewport, asset inspection, scene composition |
| Game development | project templates, asset pipelines, runtime preview |
| Automation | scheduled and event-triggered agent workflows |
| Documents | reading/writing structured documents |
| Data | tabular inspection, transformation, plotting |
| Agent workflows | multi-agent orchestration with budgets |

None of these are bolted-on tabs. Each becomes a domain module speaking the
same protocol, sharing the same context/memory/security infrastructure, and
appearing in the same shell. The measure of success: **the user stops
switching applications**, not "the feature list grows".

---

## 4. Core Design Principles

These principles constrain every design decision. Violations require an ADR
that explicitly supersedes the principle.

1. **Personal-first.** One user, one machine, zero ceremony. Team/multi-user
   concerns are out of scope and must not shape core abstractions.
2. **Local-first where useful.** User data on disk, portable, inspectable.
   No required cloud component.
3. **Model-agnostic, provider-agnostic.** OpenAI-compatible and Anthropic-
   compatible protocols today; router abstraction so capabilities degrade
   gracefully across models rather than hard-requiring one vendor.
4. **Desktop-first.** Native windowing, keyboard-driven, multi-panel. No
   WebView-shell aesthetics or constraints.
5. **Performance-first.** Latency budgets are design inputs, not afterthoughts
   (§23). A slow tool is a broken tool.
6. **Evidence-based completion.** Agents claim done only with verifiable
   evidence (build exit codes, executed tests, diffs). Claims without
   evidence are treated as failures by supervision.
7. **Minimal token waste.** Every token costs money and latency; the Token
   Economy track treats waste as a bug class (§8.14).
8. **Accuracy before token savings.** Optimization never trades correctness
   for cost. When in doubt, include exact source.
9. **Modular expansion.** New capability = new module speaking existing
   contracts, not growth of a god-object core.
10. **Plugin-first when appropriate.** If it doesn't need core privileges,
    it's an extension (§8.9).
11. **Cache is not source of truth.** Indexes, ASTs, summaries, and memories
    accelerate; files and journals decide. Edits rehydrate exact source.
12. **Chat is not project state.** Conversations reference projects; they
    are not where project knowledge lives.
13. **No fake progress / tests / completion / theater.** Enforced by review
    and, over time, by supervision checks.
14. **Maximum useful capability per necessary line.** LOC is a cost metric,
    never a goal (§5).
15. **Security boundaries are load-bearing.** Scope checks, sandboxing, and
    approval gates are not removable conveniences (§16).
16. **Recovery is a feature.** Crash, timeout, and corruption paths are
    designed and tested, not discovered in production (§17).

---

## 5. Scale Vision

### 5.1 Codebase capacity

The architecture is designed so that, if justified by real capabilities, the
codebase can grow toward **1M–10M+ real lines** over its lifetime:

- Crate boundaries prevent monolith accretion; new domains become new
  crates/modules speaking stable contracts.
- The extension platform moves third-party-scale surface out of core.
- Build times are protected by workspace profile tuning and crate hygiene.
- Repository Intelligence is designed for million-line *external* repos,
  which keeps our own tooling honest at any internal scale.

### 5.2 The metric that matters

> **Maximum Useful Capability per Necessary Line.**

LOC is tracked (production/test/generated/vendored reported separately) as
an informational cost metric. Actions that inflate LOC without capability —
boilerplate generators, wrapper spam, test spam, duplication — are
prohibited (DEVELOPMENT-STATE "Do Not Redo").

### 5.3 Data scale

- Journals and threads: designed for years of daily use (append-only +
  compaction strategy, §18).
- Indexes: million-file repos via compact structures and on-disk spill
  (§8.10).
- Memories: bounded retrieval into context; unbounded storage on disk is
  acceptable, unbounded context injection is not.

---

## 6. Canonical Source Hierarchy

When sources disagree, resolve in this order — and record the resolution:

| Rank | Source | Authority |
|---|---|---|
| 1 | Current implementation | what actually exists |
| 2 | `docs/Z-DESKTOP-MASTER-SPEC.md` | product/architecture intent |
| 3 | `docs/Z-DESKTOP-TASKS.md` | backlog and per-task status |
| 4 | `docs/DEVELOPMENT-STATE.md` | session/resume state |
| 5 | `docs/adr/*.md` | binding engineering decisions |
| 6 | `skills/*/SKILL.md` | operational instructions |

Rules:

- Implementation wins for "what is"; spec wins for "what should be".
- Task ledger statuses change only with evidence (files, tests, benchmarks).
- DEVELOPMENT-STATE is updated atomically after meaningful slices so any
  session can resume without prior conversation context.
- Skills encode operational rules so future sessions don't rediscover
  intent; they defer to ADRs when decisions conflict.

---

## 7. Current Implementation Status

Status vocabulary (mandatory across all Z Desktop documentation):

| Label | Meaning |
|---|---|
| IMPLEMENTED | Working, integrated, covered by tests in the current tree |
| PARTIAL | Real code exists; known gaps remain before "done" |
| PLANNED | Designed/intended; no or negligible code yet |
| EXPERIMENTAL | Code exists but is not trusted for daily use |
| RESEARCH | Analysis/notes exist; no implementation decision made |

### 7.1 Implemented (verified against source, 2026-08)

| Capability | Location | Evidence |
|---|---|---|
| Protocol contracts (Command/Event, ProviderConfig, Risk, Id) | `z desktop/crates/z-protocol` | serde round-trip tests |
| Agent runtime: turn loop, tool rounds, approval gate | `z-core/src/runtime.rs` | unit + budget tests |
| Tool runtime with risk classification & scope checks | `z-core/src/tools.rs` | traversal/rejection tests |
| Built-in tools: fs_read/list/search/write, terminal_exec | `z-core/src/tools.rs` | round-trip tests |
| Process sandbox: Job Objects / process groups, tree kill | `z-core/src/sandbox.rs` | grandchild-survival regressions, timeout tests |
| Suspended-spawn job assignment (no escape window) | `z-core/src/sandbox.rs` | CREATE_SUSPENDED → assign → resume path |
| Output bounding (8 MiB stdout / 2 MiB stderr caps) | `z-core/src/sandbox.rs` | oversized-stdout test |
| Secret redaction (provider tokens, bearer, assignments) | `z-core/src/redact.rs` | pattern tests incl. edge cases |
| Local token estimation (CJK-aware heuristic) | `z-core/src/tokens.rs` | estimator tests, perf bound |
| Context budgeting + clean-boundary history trimming | `z-core/src/runtime.rs` | trim_history regression tests |
| Repository index v0 (walk, symbols, map text) | `z-core/src/repo.rs` | index tests |
| Provider layer: OpenAI-compatible + Anthropic SSE streaming | `z-core/src/provider.rs` | boundary rejection test |
| BYOK config persistence | `z-core/src/runtime.rs` | config.json write path |
| Thread persistence + corrupt-file-tolerant restore | `z-core/src/runtime.rs` | restore logic |
| ZeroGPUI: window, renderer, scene, a11y, timing | `z desktop/crates/z-gpui` | 109 crate tests |
| Shell workspace model (regions, panels, presets) | `z desktop/crates/z-shell` | 48 crate tests |
| Design tokens (color, spacing, typography, theme) | `z desktop/crates/z-tokens` | 24 crate tests |
| App composition layer + headless `--check` / `--shot` | `z desktop/crates/z-app` | 53 crate tests |

Workspace verification command:

```
cargo test --manifest-path "z desktop/Cargo.toml" --workspace
```

Current suite size at time of writing: **280 tests, 0 failed**. The live
count belongs to DEVELOPMENT-STATE; this section records capability truth,
not a number that rots.

### 7.2 Partially implemented

| Capability | Present | Missing |
|---|---|---|
| Sandbox cancellation | kill-on-timeout complete | mid-run cancel flag inside wait loop |
| Redaction coverage | tool output | runtime logs, journal events |
| Provider layer | two wire protocols, streaming | retries/backoff policy, usage accounting surfaced |
| Repo index | walk + light symbols | tree-sitter ASTs, references, incremental updates |
| Persistence | thread JSON snapshots | append-only journal, crash-recovery ordering |
| UI shell | panels/presets model | docking, layout profiles, full settings surface |
| Secrets storage | plaintext local config.json | OS keychain integration |

### 7.3 Planned (designed, not built)

Agent steering queue · durable task journal · sub-agent orchestration ·
worktree isolation · Best-of-N evaluation · supervision/evidence engine ·
tree-sitter index actor · semantic retrieval · memory layers · model router
· local-model support · hardware intelligence · resource scheduler · PTY
terminal · IDE editor surface · internal browser · diagram engine · database
workspace · API workbench · data workspace · automation/workflow engine ·
artifact platform · schema-driven settings · plugin SDK.

### 7.4 Research

Differentiating-capability research (170-item backlog), grok-build
dissection, capability matrix — see `docs/research/`. These inform priority;
none constitute implementation claims.

---

## 8. Architecture Domains

Each domain below states: purpose, architecture, invariants, current status.
Cross-cutting rules appear once and are referenced elsewhere.

### 8.1 Agent Kernel

**Purpose**: execute one user-visible unit of agentic work (a *turn*) safely
and observably.

**Architecture** (implemented):

```
UI ──Command──▶ Runtime.serve() ──spawn──▶ run_turn worker
                                              │
              Event channel ◀─────────────────┘
                    │
             event pump thread ──▶ EventQueue ──▶ frame drain ──▶ scene
```

- Turn = stream response → parse tool calls → per-call approval → execute →
  feed results back → repeat until plain-text completion or MAX_TOOL_ROUNDS.
- Approval gate: condvar-based; UI resolves via ResolveApproval command.
- Cancellation: cooperative flag checked between rounds and calls.
- Persistence: whole-thread snapshots at every mutation point.

**Invariants**:
1. User message persisted before first provider call.
2. Denied/timed-out approvals still produce tool results for the model.
3. Events are fire-and-forget; core survives dead UI.
4. History trimming preserves request structural validity.

**Status**: IMPLEMENTED (core loop); steering/pause/resume/checkpoints PLANNED.

### 8.2 Agent Orchestration

**Purpose**: coordinate multiple turns/tasks into goal-directed work beyond
a single request/response cycle.

**Architecture (planned)**:
- Task graph: tasks with dependencies, states (created/running/paused/
  failed/completed), and evidence attachments.
- Orchestrator consumes task graph, spawns agent runs, enforces budgets,
  merges results.
- Durable journal underlies all state so orchestration survives restarts.

**Invariants**: no task runs without a journal record; state transitions are
journal events; recovery replays the journal.

**Status**: PLANNED. Depends on: task journal (§18), supervision (§8.4).

### 8.3 Sub-Agents

**Purpose**: delegate scoped work to isolated agent contexts.

**Architecture (planned)**: see skills/z-subagents for the design contract —
isolation ladder (context-only → scope-granted → worktree → sandboxed),
exclusive write grants, reference-passed shared context, parent-evaluated
results, budget-aware concurrency caps.

**Invariants**: child failure never corrupts parent state; overlapping write
grants are an orchestration bug; results without evidence are rejected.

**Status**: PLANNED. Depends on: journal, worktrees, safe editing.

### 8.4 Agent Supervision

**Purpose**: detect and prevent dishonest or pathological agent behavior.

**Detection targets**:
- fake completion (claims done without evidence events)
- tests/builds claimed but never executed
- skipped requirements (task statement vs delivered diff mismatch)
- ignored failures (error output followed by success claims)
- repeated useless tool calls / doom loops (identical failing call ≥ N)
- premature stopping, placeholder implementations, hidden TODOs,
  commented-out implementations, mocks left in production paths

**Mechanism (planned)**: supervision pass over the turn's evidence log
before completion is accepted; circuit breakers on repeated failures;
requirement checklist derived from the task statement and checked against
artifacts.

**Invariants**: supervision can fail a turn but never silently edit its
output; supervision findings are user-visible.

**Status**: PLANNED.

### 8.5 Evidence Engine

**Purpose**: make important claims verifiable by construction.

**Evidence types**:
- Build passed → command + exit code + duration recorded.
- Tests passed → executed test run output recorded (not summarized claims).
- File edited → real diff recorded.
- Performance improved → before/after benchmark records.
- Bug fixed → regression test id recorded.

**Architecture (planned)**: evidence records attach to journal events; UI
surfaces them inline next to claims; supervision consumes them.

**Invariant**: evidence is captured at execution time by the system, not
narrated by the model.

**Status**: PLANNED (foundations: StepStarted/StepFinished events exist).

### 8.6 Tools System

**Purpose**: the agent's hands — safe, declared, auditable capabilities.

**Architecture (implemented)**:
- ToolDef registry with JSON-schema parameters advertised to providers.
- Risk classification: ReadOnly (auto-allowed in-scope) / Write / Execute
  (approval-gated). Unknown tools fail closed as Execute.
- Execution funnel: `tools::execute` is the ONLY place core touches the
  filesystem or spawns processes.
- Current tools: fs_read, fs_list, fs_search, fs_write, terminal_exec
  (sandboxed, optional timeout_ms).

**Planned tools**: git_* family, patch/edit with fingerprints, browser_*,
diagram render, db query, api request, workflow control, artifact ops.

**Invariants**: every spawn goes through the sandbox; every path through
scoped(); every output through redaction + bounding.

**Status**: IMPLEMENTED (core five); expansion PLANNED via extension API.

### 8.7 Skills System

**Purpose**: package operational knowledge for agents (and Hermes sessions)
as loadable instruction units.

**Design**: skill = directory with SKILL.md (frontmatter: name,
description; body: when-it-applies, files/modules, architecture, invariants,
failure cases, testing expectations, DoD). Skills are documentation-grade
contracts, not executable plugins.

**Status**: IMPLEMENTED for engineering sessions (this repo's `skills/`);
in-app agent skill loading PLANNED.

### 8.8 MCP Integration

**Purpose**: speak the Model Context Protocol so external tool/resource
servers plug into the agent.

**Architecture (planned)**: MCP client in core behind the same Risk/
approval machinery as native tools; server processes sandboxed; permissions
deny-by-default per server; resources mapped into context engine with
freshness metadata.

**Invariants**: an MCP tool is indistinguishable from a native tool to the
model but carries provenance ("via mcp:<server>") for audit.

**Status**: PLANNED.

### 8.9 Plugins & Extensions

See skills/z-extensions. Extension kinds: tool, provider, parser, panel,
renderer, command. Deny-by-default permissions; versioned contracts;
crash containment. In-process strict-trait v1, out-of-process later.

**Status**: PLANNED (design settled; SDK unbuilt).

### 8.10 Repository Intelligence

See skills/z-repository-intelligence. Index actor owns parsing/storage;
tree-sitter grammars produce symbol/reference/import graphs; content
fingerprints drive incremental re-indexing; retrieval layers: lexical →
structural → semantic (supplement only). Scale targets: 100k-file initial
index in minutes; single-file incremental < 50 ms; lookup < 10 ms.

**Invariants**: index is cache; edits rehydrate exact source; indexing never
blocks turn/UI threads; bounded memory with spill path.

**Status**: PARTIAL (v0 walk+symbols implemented; actor/AST/incremental PLANNED).

### 8.11 Safe Editing Engine

See skills/z-safe-editing. Fingerprint-before-write, atomic temp+rename
writes, scoped() on every path, staged multi-file operations with rollback,
user-edit preservation, exclusive write grants across agents.

**Status**: PARTIAL (scope checks + basic writes implemented; fingerprints,
atomicity, rollback PLANNED).

### 8.12 Git & Worktrees

**Purpose**: full VCS surface for humans and agents.

**Architecture (planned)**: git2/rust binding layer exposing read ops
(status/diff/log — auto-allowed) and write ops (stage/commit/branch/
worktree — approved). Worktree orchestration for sub-agents and Best-of-N.
Merge assistance: conflict analysis, hunks-level explanation, cherry-pick.

**Invariants**: agents never rewrite history or force-push; worktrees are
tracked resources with cleanup guarantees.

**Status**: PLANNED.

### 8.13 Context Engine

See skills/z-context-engine. Layered context (stable prefix / session /
turn / ephemeral), priority allocation, compaction with pinned facts,
freshness metadata, exact-source rehydration before edits.

**Status**: PARTIAL (budgeting + trimming implemented; compaction,
priority allocation, freshness plumbing PLANNED).

### 8.14 Token Economy

See skills/z-token-economy. Optimization ladder: don't-send → cache-hit →
send-less → compress-representation. Provider prompt-cache discipline
(byte-stable prefixes). Every optimization measured.

**Status**: PARTIAL (estimator + budgeting implemented; caching layers,
lazy tools, structured outputs PLANNED).

### 8.15 Cache Architecture

**Principle**: caches accelerate; they never decide. Every cache entry
carries a fingerprint of its inputs; mismatch invalidates. Cache classes:

| Cache | Key | Invalidation |
|---|---|---|
| AST/symbol | file content hash | file change |
| Retrieval results | query + corpus fingerprint | corpus change |
| Tool results | tool + args + input fingerprints | any input change |
| Prompt prefix | byte-exact prefix | any prefix edit (expensive!) |
| Rendered artifacts | source hash + renderer version | either changes |

**Status**: PLANNED as unified infrastructure (ad-hoc caching exists in
cosmic-text shaping etc.).

### 8.16 Memory Architecture

See skills/z-memory. Five layers (working/session/project/semantic/
episodic); provenance + confidence + superseding mandatory; consolidation
explicit; user-correctable; journal-backed replay.

**Status**: PLANNED.

### 8.17 Model Router

**Purpose**: choose the right model per sub-task; degrade gracefully.

**Design (planned)**: capability-tagged model registry (context window,
vision, tool-use quality tiers, cost, latency class); routing policies
("cheap for search, strong for synthesis"); fallback chains on provider
failure; per-task overrides from orchestrator/sub-agent budgets.

**Invariants**: router decisions are logged with reasons; fallback never
silently downgrades a task that declared hard model requirements.

**Status**: PLANNED.

### 8.18 Provider Layer

**Implemented**: OpenAI-compatible `/chat/completions` and Anthropic
`/v1/messages`, both SSE-streaming, blocking ureq client (rustls), BYOK
config persisted locally.

**Planned**: retry/backoff policy, usage accounting surfaced as events,
per-provider prompt-cache awareness, connection reuse tuning, provider
health checks.

**Invariants**: provider failures never lose the user message; streaming
deltas forward immediately; secrets never enter logs.

### 8.19 Local Models

**Purpose**: run inference locally (llama.cpp-class backends) for offline
use, privacy, and cost control.

**Design (planned)**: backend abstraction behind the same Provider trait;
hardware intelligence picks quantization/context size; resource scheduler
prevents local inference from starving UI/GPU; model downloads managed with
integrity checks.

**Status**: PLANNED. Depends on: hardware intelligence, resource scheduler.

### 8.20 Hardware Intelligence

**Purpose**: know the machine — CPU cores, RAM, GPU/VRAM, thermals — and
feed scheduling decisions (local model sizing, indexing parallelism,
rendering quality tiers).

**Status**: PLANNED.

### 8.21 Resource Scheduler

**Purpose**: arbitrate contention between UI, indexing, local inference,
sub-agents, and background jobs.

**Design (planned)**: priority classes (interactive > agent-turn >
background-index > batch); token/time budgets per class; admission control
for expensive operations.

**Invariant**: interactive latency never sacrifices to background work.

**Status**: PLANNED.

### 8.22 Terminal

**Purpose**: first-class PTY terminals as panels AND as agent tools.

**Design (planned)**: portable PTY layer (ConPTY/winpty, unix pty);
scrollback virtualization; agent access through the sandboxed exec path
(non-interactive) plus supervised interactive sessions; shell integration
markers for command/output attribution.

**Status**: PLANNED (agent-side non-interactive execution implemented via
sandbox; PTY panels not started).

### 8.23 IDE Surface

**Purpose**: editor-grade code interaction inside Z Desktop.

**Design (planned)**: buffer model with rope/text kernels, syntax
highlighting via tree-sitter queries, LSP client integration, go-to-
definition/references backed by repository intelligence, inline diff
review of agent edits.

**Status**: PLANNED. Depends on: repository intelligence, safe editing.

### 8.24 Internal Browser

**Purpose**: embedded browsing for research and web-app preview, with
controlled agent access.

**Design (planned)**: WebView surface isolated from UI process; agent gets
structured page access (DOM/readability extraction) through approved tools;
profile separation (user vs agent cookies); download safety.

**Status**: PLANNED.

### 8.25 Search & Research

**Purpose**: answer questions with sources — local corpus first, then web.

**Design (planned)**: local search over indexed repos/documents; web search
providers behind an abstraction; result provenance always attached;
research sessions produce cited artifacts.

**Status**: PLANNED.

### 8.26 Live Preview

**Purpose**: instant visual feedback for web/UI development.

**Design (planned)**: static server + HMR bridge for web projects; artifact
preview pane wired to the artifact platform; screenshot capture for agent
self-review of UI work.

**Status**: PLANNED.

### 8.27 3D Workspace

**Purpose**: viewport, asset inspection, and scene composition for 3D and
game workflows.

**Design (planned)**: wgpu-native renderer sharing the ZeroGPUI device;
glTF import; gizmos; scene graph; agent tools for asset queries and scene
manipulation.

**Status**: PLANNED (research stage).

### 8.28 Game Development Support

**Purpose**: project templates, asset pipelines, runtime preview, and agent
assistance tuned for game engines/projects.

**Status**: RESEARCH (follows 3D workspace).

### 8.29 Diagram Engine

**Purpose**: render and generate diagrams (mermaid-class, architecture
graphs, dependency graphs from repository intelligence).

**Status**: PLANNED.

### 8.30 Database Workspace

**Purpose**: connections, schema browsing, query workbench, data inspection
with agent assistance (schema-aware SQL generation, result analysis).

**Safety**: read-only by default; write queries require approval; credentials
in secret storage.

**Status**: PLANNED.

### 8.31 API Workbench

**Purpose**: HTTP request composition, environments, collections, response
inspection — with agent assistance for authoring and debugging requests.

**Status**: PLANNED.

### 8.32 Data Workspace

**Purpose**: tabular data inspection, transformation recipes, plotting.

**Status**: PLANNED.

### 8.33 Automation Engine

**Purpose**: scheduled and event-triggered agent runs (nightly maintenance,
watch-triggered fixes, CI-style local pipelines).

**Design (planned)**: triggers (cron, file-watch, webhook-local, manual) →
workflow invocation; runs are journaled tasks with budgets; missed schedules
resolve on wake.

**Status**: PLANNED. Depends on: workflow engine, journal.

### 8.34 Workflow Engine

**Purpose**: declarative multi-step agent workflows (templates combining
tasks, approvals, checkpoints, evaluation gates).

**Status**: PLANNED. Depends on: orchestration, journal.

### 8.35 Artifact Platform

**Purpose**: first-class outputs of agent work — documents, diagrams,
reports, patches, screenshots — versioned, previewable, shareable.

**Status**: PLANNED.

---

## 9. UI Platform Specification

**Purpose**: the window, scene, and interaction substrate on which every
surface renders.

### 9.1 Stack

| Layer | Crate | Responsibility |
|---|---|---|
| Windowing | winit | platform windows, input events, DPI |
| Rendering | wgpu | GPU abstraction (D3D12/Vulkan/Metal) |
| Scene | z-gpui scene | retained-per-frame display list |
| Text | cosmic-text | shaping, layout, font fallback |
| A11y | accesskit + accesskit_winit | platform accessibility trees |
| Layout | taffy | flexbox-class layout engine |
| Model | z-shell | regions, panels, presets, view state |
| Tokens | z-tokens | color/spacing/type primitives |
| Composition | z-app | shell state → scenes; runtime wiring |

### 9.2 Frame lifecycle

1. Drain EventQueue (runtime events, input, timers).
2. Update shell model (pure state transitions).
3. Build/diff scene from model (pure function where possible).
4. Submit to renderer; present.
5. Timing module records per-phase durations for diagnostics.

**Invariant**: no event mutates the scene mid-draw; the scene is a value.

### 9.3 Interaction model

- Keyboard-first: every action reachable by keybinding; bindings are
  user-mappable (§13).
- Focus is explicit and visible; focus rings are never suppressed.
- Streaming text renders incrementally without layout thrash (shaped-run
  caching).
- Long content virtualizes (chat history, diffs, logs, file trees).

### 9.4 Surfaces (current + planned)

Current: conversation view (streaming deltas, tool steps, approvals),
provider status, project indexing status, headless check/screenshot modes.

Planned: terminal panels, diff viewer, file tree, editor, browser pane,
diagram canvas, database grid, API workbench, artifact gallery, context
inspector, memory inspector, token usage dashboard, diagnostics console,
settings pages.

### 9.5 Performance contracts

- Scene build < 2 ms typical; input echo < 16 ms; panel switch < 50 ms.
- No per-frame heap churn in steady state (measure with timing module).
- Virtualized lists: O(visible) work regardless of content size.

---

## 10. Theme System

**Purpose**: every visual property flows from tokens; users retheme without
code changes.

### 10.1 Token architecture (three layers)

1. **Primitives**: raw values (hex colors, px sizes, font files).
2. **Semantic tokens**: role-based references (surface, surface-raised,
   text-primary, text-muted, accent, danger, success, border-subtle...).
   Views consume ONLY semantic tokens.
3. **Component tokens**: per-component overrides (button-radius, panel-gap)
   derived from semantic tokens.

### 10.2 Rules

- Hardcoding a color/size in a view is a defect; route through tokens.
- Dark/light/high-contrast themes are token-set swaps, not code paths.
- Syntax and terminal palettes are token families with their own semantic
  roles (keyword, string, comment, ansi-0..15).
- Theme files are versioned; unknown tokens warn, known-but-unused tokens
  are prunable.

### 10.3 Customization surface

User-editable theme files + in-app editor (Developer Mode) with live
preview. Import/export as single files. Community themes are files, not
plugins (no code execution for theming).

---

## 11. Dock & Layout System

**Purpose**: panels are first-class, user-arrangeable workspace citizens.

### 11.1 Model (z-shell)

- Layout = tree of regions (split horizontal/vertical) with leaf panels.
- Panel states: visible, hidden, focused, maximized, floating (later).
- Presets: named layouts ("Agent focus", "Review", "Research") switchable
  instantly; per-workspace persistence.

### 11.2 Behaviors

- Drag-to-dock with clear drop indicators; Escape cancels a drag.
- Resizable splitters with keyboard adjustment (accessibility).
- Panel visibility toggles are single keystrokes; layout never traps focus.
- Minimum sizes protect usability; overflow scrolls, never clips.

### 11.3 Persistence

Layout profiles serialize to workspace data; version migrations follow the
settings migration rules (§12).

---

## 12. Settings: User Mode & Developer Mode

See skills/z-settings for operational rules. Summary:

- **User Mode**: provider/model, project folder, theme, core agent
  behaviors. Under 20 options at launch. Everything else hidden.
- **Developer Mode**: full schema exposure — context/token policy, cache
  behavior, tool grants, MCP permissions, panel internals, animation,
  resource limits, diagnostics verbosity, experimental features.
- Schema-driven rendering, searchable, presets, versioned migrations,
  secrets excluded (credential storage instead).

---

## 13. Customization Depth

Long-term customization surface (Developer Mode):

| Domain | Customizable |
|---|---|
| Theme | all token layers, backgrounds, syntax/terminal palettes |
| Typography | font family/size/weight per role, line height, ligatures |
| Space | spacing scale, radius scale, panel gaps, density presets |
| Layout | panel visibility/position/size/docking, layout profiles |
| Keybindings | full remap, chord support, per-context scopes |
| Animation | durations, easing, reduced-motion override |
| Agent | tool grants, approval policy, budgets, model routing |
| Context | budget sizes, priority weights, compaction triggers |
| Cache | sizes, TTLs, invalidation strictness |
| Diagnostics | log levels, trace sinks, metrics export |

Constraint: default UX stays simple. Depth is opt-in, discoverable via
search, never required for basic operation.

---

## 14. Accessibility

- accesskit integration on every interactive node (label, role, state).
- Full keyboard operability; no pointer-only flows.
- Screen-reader announcements for streaming status (throttled, meaningful).
- Contrast ratios meet WCAG AA in all shipped themes; high-contrast theme
  exceeds AAA for text.
- Reduced-motion preference honored globally.
- Focus order follows visual order; modals trap and restore focus.

**Status**: foundations implemented (accesskit wired in z-gpui); full audit
PLANNED per-surface as surfaces ship.

---

## 15. Diagnostics, Observability & Tracing

### 15.1 Logging

- `log` crate facade; env_logger for dev; rotating file sink (planned) with
  redaction applied to any sink that persists (debt recorded).
- Levels: error (user-visible failures), warn (degraded paths), info
  (lifecycle), debug/trace (dev only).

### 15.2 Metrics (planned)

Counters/histograms for: turn duration, tool latency, token usage per
request, cache hit rates, frame times, index throughput. Export: local
JSONL + in-app dashboard (Developer Mode).

### 15.3 Tracing (planned)

Turn-scoped spans: provider call → tool call → sandbox exec. Correlated by
turn_id/call_id already present in events. Export to local file; no remote
telemetry — ever (personal-first).

### 15.4 Crash reporting

Local crash dumps + last-journal-tail capture. No automatic upload; user
initiates any sharing.

---

## 16. Security Model

See skills/z-security for operational rules. Model summary:

### 16.1 Trust boundaries

```
User ──full trust──▶ Z Desktop core
Core ──sandboxed──▶ agent-spawned processes
Core ──scoped──────▶ filesystem (project root)
Core ──approved────▶ write/execute tool calls
Extensions/MCP ──deny-by-default──▶ capabilities
```

### 16.2 Mechanisms (implemented)

- Process-tree isolation with suspended-spawn job assignment (no escape
  window), tree kill on timeout, orphan reaping on crash.
- Filesystem scope: lexical normalization, traversal rejection, symlink
  awareness at canonicalization.
- Secret redaction on tool output; risk classification with fail-closed
  unknowns; approval gate for write/execute.

### 16.3 Mechanisms (planned)

- OS keychain credential storage (DPAPI/keyring).
- Fingerprint-before-write stale-edit prevention.
- Plugin/MCP permission system with revocation UI.
- Network egress policy for extensions.
- Destructive-operation confirmation (delete, force git ops).

### 16.4 Threat posture

Primary threats: prompt-injected agents exfiltrating secrets or destroying
files; supply-chain via dependencies; malicious extensions. Mitigations:
redaction, scope, approval, sandbox, minimal dependency policy, deny-by-
default permissions. Personal-first means the user is also the admin —
but the system must protect the user from the AGENT, not just from outsiders.

---

## 17. Failure & Recovery

Every failure class has a designed behavior:

| Failure | Behavior |
|---|---|
| App crash mid-turn | thread snapshot + (planned) journal replay resume |
| Provider timeout/error | one retry (round 0), then turn fails; message persisted |
| Command timeout | sandbox tree-kill, partial output preserved |
| Tool failure | structured failure result reaches model; turn continues |
| Approval timeout | denial recorded; remaining calls proceed |
| Corrupt thread file | skipped on restore with warning; others load |
| Corrupt cache/index | invalidated and rebuilt; never blocks work |
| DB/journal failure | read-only degradation; repair tooling (planned) |
| Local model failure | router falls back or surfaces clear error |
| Plugin crash | contained (panic hook v1, process boundary later) |
| Malformed model output | tolerant parsing; stricter validation per tool |
| Context overflow | trim → compact → refuse with explanation (in order) |
| Partial edit | atomic writes prevent; rollback recovers (planned) |

**Recovery principle**: the user's data (messages, files, journal) is never
the thing that gets sacrificed to recover.

---

## 18. Persistence Architecture

### 18.1 Current

- Threads: `data/threads/<id>.json`, whole-snapshot writes at mutation
  points, corrupt-tolerant restore.
- Provider config: `data/config.json` (plaintext BYOK — keychain planned).

### 18.2 Planned: append-only journal

- `data/journal/` JSONL segments: command/event lifecycle, task states,
  checkpoints, evidence records.
- Replay rebuilds: threads, task graph, memory views, caches.
- Segment rotation + compaction; integrity via trailing checksums.
- Upgrade path to SQLite documented; JSONL first to avoid premature
  dependency (dependency discipline rule).

### 18.3 Invariants

1. Journal is truth; materialized views are caches.
2. Writes are append-only; corrections are new events, not edits.
3. Replay is deterministic (same journal → same state).
4. Crash between events loses nothing already appended.

---

## 19. Updates & Distribution

- Distribution: GitHub Releases with per-platform archives; installer
  packaging (MSIX/NSIS, AppImage, dmg) later.
- Update check: manual-first (personal-first; no phoning home). Explicit
  "check for updates" action; release notes rendered in-app.
- No auto-update without user opt-in.
- Build reproducibility: lockfiles committed; release builds from tagged
  commits with recorded toolchain (rust-toolchain file planned).

---

## 20. Testing Strategy

See skills/z-testing. Summary: inline unit tests per module; real-process
integration for sandbox/tools; headless app checks; screenshot captures;
failure injection as first-class; fuzzing for untrusted-input parsers
(planned); soak tests for services (planned); provider replay harness for
E2E without live keys (planned).

**Rule**: tests are never weakened to pass broken code; requirement changes
must be stated explicitly.

---

## 21. Benchmarking Strategy

- Fixtures: synthetic repos at small/medium/large/very-large scales.
- Harnesses: criterion where hot-path; hand-rolled timing for app-level.
- Records: docs/benchmarks/<topic>.md with date, machine class, numbers.
- Gates: hot-path regressions > 20% fail CI (when CI lands).
- Never invent numbers; targets and measurements live separately.

---

## 22. Cross-Platform Strategy

See skills/z-cross-platform. Platform code isolated behind cfg-gated
modules with identical trait surfaces. Windows is the development-primary
platform today; Linux/macOS parity is tracked per-feature in the task
ledger. ARM64 first-class. GPU backends: D3D12/Vulkan/Metal via wgpu.

---

## 23. Performance Philosophy

- Latency budgets are design inputs (§9.5, skills/z-performance).
- Idle must be free: no polling loops; event-driven only.
- Memory discipline: bounded caches, leak checks for long sessions.
- Measure → optimize → record → gate. No speculative optimization without
  a measured hot path.

---

## 24. Competitive Landscape & Lessons

Classification per finding: INSPIRE / ADAPT / REIMPLEMENT / REUSE / REJECT.
Full ledger: docs/THIRD_PARTY_RESEARCH_AND_REUSE.md; dissection:
docs/research/grok-build-dissection.md.

| Source | Key lessons taken | Classification |
|---|---|---|
| xai grok-build | agent loop structure, steering, compaction, codebase graph, task durability, worktrees, sandbox posture | INSPIRE/ADAPT |
| OpenAI Codex | Rust agent architecture, sandbox/approval UX | INSPIRE |
| Cline | task lifecycle, checkpoints, approval UX, MCP breadth | INSPIRE |
| OpenHands | event model, runtime isolation, evaluation harnesses | INSPIRE |
| OpenCode | provider abstraction, client/server split | INSPIRE |
| Aider | repo maps, edit formats, lint/test loops | ADAPT (repo map concept) |
| Gemini CLI | checkpointing, extensions, non-interactive runs | INSPIRE |
| Zed | GPUI-class rendering, editor craft, project model | INSPIRE (license: architecture reference only) |
| Claude Code / Cursor / Windsurf | closed-source: docs/release-notes study only | INSPIRE (no cloning) |

**Rejected approaches** (recorded to prevent relitigating): Electron-class
WebView shells; cloud-mandated architectures; team-first data models;
unbounded context stuffing as a strategy.

---

## 25. Differentiating Capability Families

From docs/research/differentiating-capabilities.md (170-item backlog),
organized into families that compose into the platform:

1. **Trust & Evidence** — supervision, evidence engine, approval UX,
   auditability. Agents that must prove their work.
2. **Token Economy** — estimation, budgeting, caching, lazy tools. The
   cheapest competent agent.
3. **Repository Intelligence** — AST graphs, incremental indexing,
   affected-analysis. An agent that already knows the codebase.
4. **Durable Execution** — journal, recovery, steering, checkpoints.
   Work that survives crashes and interruptions.
5. **Orchestration** — sub-agents, worktrees, Best-of-N, budgets. A team,
   not a chatbot.
6. **Workspace Breadth** — terminal, browser, diagrams, DB, API, data, 3D.
   The user stops switching apps.
7. **Local-First Control** — BYOK, local models, hardware intelligence,
   zero telemetry. The user owns the stack.
8. **Calm Native UX** — GPU-rendered, keyboard-driven, themeable. A tool
   that respects attention.

Each family has dependency-ordered tasks in the ledger; families compose
(e.g., Orchestration requires Durable Execution + Trust & Evidence).

---

## 26. Glossary

| Term | Definition |
|---|---|
| Turn | one user-visible unit of agentic work (request → tools → completion) |
| Thread | persisted conversation with tool-call history |
| Clean boundary | history cut point at a real user message (no tool results) |
| Sandbox | process-tree isolation for agent-spawned commands |
| Scope check | lexical path validation against canonical project root |
| Redaction | fingerprinted secret masking on output surfaces |
| Evidence | system-captured proof (exit codes, test output, diffs) |
| Journal | append-only event log; source of truth for durable state |
| Clean boundary (cache) | fingerprint match permitting cache reuse |
| Steering | user guidance injected into a running turn between tool rounds |
| Worktree | isolated git working tree for parallel agent work |
| Best-of-N | N independent solution attempts with evaluation and selection |
| ZeroGPUI | Z Desktop's custom GPU UI runtime (z-gpui crate) |
| BYOK | bring-your-own-key provider configuration |
| Doom loop | agent repeating identical failing calls; circuit-breaker target |

---

## 27. Subsystem Engineering Specifications

Deep specifications per crate. These are the contracts engineers code
against; where code and spec disagree, reconcile per §6.

### 27.1 z-protocol — the wire contract

**Role**: the only shared vocabulary between core and UI. If a type appears
on both sides of the command/event channels, it lives here.

**Owned types**:

| Type | Purpose | Stability rule |
|---|---|---|
| `Command` | UI → core requests (SendMessage, CancelTurn, ResolveApproval, ConfigureProvider, OpenProject, + planned EnqueueMessage, PauseTurn, ResumeTurn, OpenThread, DeleteThread, SetSetting...) | additive only; new variants get serde defaults where sensible |
| `Event` | core → UI notifications (Accepted, TurnStarted, TextDelta, TextDone, TurnFinished, ApprovalRequested, StepStarted, StepFinished, ProviderStatus, ProjectIndexed, + planned SteeringQueued, CheckpointCreated, EvidenceRecorded, UsageReported...) | additive only |
| `ProviderConfig` | BYOK descriptor (kind, base_url, model, key) | key field must never be logged/serialized to non-secret stores |
| `Risk` | ReadOnly / Write / Execute classification | closed set; new variants are security decisions |
| `Id` | opaque identifier string | format: `<prefix>-<hex-ms>-<hex-counter>` |

**Invariants**:
1. Zero internal dependencies; serde + std only.
2. Every enum variant round-trips through JSON (test per variant family).
3. No behavior — data only. Logic belongs to consumers.

**Failure modes**: unknown variant on deserialize → error at boundary, not
panic; UI must tolerate missing optional fields (serde defaults).

### 27.2 z-core — the agent core

**Module map**:

| Module | Responsibility | Key types |
|---|---|---|
| `runtime` | command loop, turn workers, approval gate, persistence, budgeting | Runtime, Thread, StoredMessage, trim_history |
| `provider` | provider trait + OpenAI/Anthropic SSE clients | Provider, ChatRequest, StreamItem, StreamOutcome |
| `tools` | tool registry, risk classify, scoped FS ops, execution funnel | ToolDef, ToolInvocation, ToolOutput, scoped() |
| `sandbox` | process-tree isolation, bounded exec | run(), ExecOutcome, Guard, JobHandle |
| `redact` | secret pattern redaction | redact(), Rule |
| `tokens` | local token estimation, budget classification | estimate(), check_budget(), Budget |
| `repo` | repository index v0 | RepoIndex |

**Threading model**: one command-loop thread; one worker per turn; reader
threads per sandboxed child; (planned) index actor thread. Shared state via
`Arc<Shared>` with narrow mutex scopes.

**Provider trait contract**:

```
fn stream(&self, req: &ChatRequest, on_item: &mut FnMut(StreamItem))
    -> Result<StreamOutcome, String>
```

- Must forward TextDelta items synchronously as they arrive.
- Must return complete text + tool_calls in StreamOutcome.
- Must not retry internally (retry policy lives in runtime).
- Must never include credentials in error strings.

**Sandbox contract** (see §16.2): spawn-suspended → job-assign → resume →
drain pipes concurrently → enforce deadline → tree-kill → reap → bounded
output. Any pre-resume failure terminates the child.

**Error philosophy**: `Result<_, String>` with actionable messages; typed
errors deferred per-domain (see skills/z-rust-engineering).

### 27.3 z-shell — workspace model

**Role**: pure state describing the workspace; no I/O, no threads.

**Owned concepts**: layout tree (regions/splits), panel registry and states,
presets, view state (active thread, streaming buffers, pending approvals
mirror), focus model.

**Invariant**: shell state transitions are pure functions of (state, event);
this makes replay and testing trivial.

### 27.4 z-gpui — rendering runtime

**Role**: turn shell state into GPU frames; own winit/wgpu/accesskit.

**Key subsystems**: window management, renderer (pipeline, batches),
scene graph, text shaping cache, timing instrumentation, a11y tree sync,
screenshot path (`--shot`), headless validation (`--check`).

**Invariant**: scene is a per-frame value; event handling never mutates it
mid-draw (§9.2).

### 27.5 z-tokens — design tokens

**Role**: three-layer token system (primitive → semantic → component) as
data. Themes are token-set files. Views reference semantic tokens only.

### 27.6 z-app — composition

**Role**: wire runtime ↔ shell ↔ gpui; CLI arg handling (--check, --shot);
event pump thread. Contains no business logic; if logic appears here, move
it down (core) or sideways (shell).

---

## 28. Tool Catalog (canonical specifications)

Each tool: purpose, schema, risk, invariants, failure modes, tests.

### 28.1 fs_read — IMPLEMENTED

- Schema: `{ path: string }`.
- Risk: ReadOnly. Scope-checked; 512 KiB size cap with pointer to
  terminal_exec for larger files.
- Failure modes: outside scope → explicit rejection; too large → guidance.
- Tests: round-trip, scope rejection.

### 28.2 fs_list — IMPLEMENTED

- Schema: `{ path: string, default "." }`. Returns `kind name (size)` lines,
  sorted. Scope-checked.

### 28.3 fs_search — IMPLEMENTED

- Schema: `{ query: string, path?: string }`. Literal substring match over
  text files; returns `path:line: text` (forward slashes), capped at 200
  hits; skips dependency/build dirs.
- Failure modes: empty query rejected; binary files skipped silently.
- Planned upgrade: regex mode, glob filters, result pagination.

### 28.4 fs_write — IMPLEMENTED (basic)

- Schema: `{ path: string, content: string }`. Creates parent dirs.
- Risk: Write (approval-gated).
- Planned: fingerprint-before-write, atomic temp+rename, structured patches.

### 28.5 terminal_exec — IMPLEMENTED

- Schema: `{ command: string, timeout_ms?: integer }`.
- Risk: Execute (approval-gated). Routes through sandbox (§16.2).
- Output: stdout, `[stderr]` block, optional `[killed: ...]` marker,
  `[exit code: N]`. Redacted + bounded (12k chars to model).
- Failure modes: empty command rejected; timeout kills tree with partial
  output preserved; spawn failure reported.

### 28.6 Planned tool families (specification sketches)

| Family | Tools | Risk | Notes |
|---|---|---|---|
| git read | git_status, git_diff, git_log | ReadOnly | auto-allowed |
| git write | git_add, git_commit, git_branch, git_worktree | Write/Execute | approval; never history rewrite |
| edit | edit_patch (search/replace blocks with fingerprints) | Write | stale-edit refusal |
| browser | browser_open, browser_read, browser_act | ReadOnly/Execute | profile-isolated |
| diagram | diagram_render (mermaid-class) | ReadOnly | artifact output |
| db | db_schema, db_query (read), db_execute (write) | ReadOnly/Write | credentials from secret store |
| api | http_request | Execute | egress policy applies |
| artifacts | artifact_save, artifact_list | Write | versioned store |
| workflow | workflow_start, workflow_status | Execute | journal-backed |

Every new tool MUST: declare Risk, scope-check inputs, redact outputs,
bound outputs, document failure modes, ship tests, and appear in this
catalog.

---

## 29. Command & Event Catalog

### 29.1 Commands (UI → core)

| Command | Payload | Status |
|---|---|---|
| ConfigureProvider | ProviderConfig | IMPLEMENTED |
| OpenProject | path | IMPLEMENTED |
| SendMessage | thread_id, text | IMPLEMENTED |
| CancelTurn | thread_id | IMPLEMENTED |
| ResolveApproval | call_id, approved | IMPLEMENTED |
| EnqueueMessage | thread_id, text | PLANNED (steering) |
| PauseTurn / ResumeTurn | thread_id | PLANNED |
| OpenThread / DeleteThread / ListThreads | ids | PLANNED |
| SetSetting | key, value | PLANNED |
| GrantPermission / RevokePermission | scope, subject | PLANNED |

### 29.2 Events (core → UI)

| Event | Payload | Status |
|---|---|---|
| Accepted | command_id | IMPLEMENTED |
| TurnStarted / TurnFinished | thread_id, turn_id, ok, error | IMPLEMENTED |
| TextDelta / TextDone | thread_id, turn_id, delta | IMPLEMENTED |
| ApprovalRequested | thread_id, call_id, tool, detail, risk | IMPLEMENTED |
| StepStarted / StepFinished | thread/turn/call ids, tool, detail, ok, summary | IMPLEMENTED |
| ProviderStatus | ok, message | IMPLEMENTED |
| ProjectIndexed | path, files, symbols | IMPLEMENTED |
| SteeringQueued / SteeringApplied | thread_id, depth | PLANNED |
| CheckpointCreated | thread_id, checkpoint_id | PLANNED |
| EvidenceRecorded | turn_id, evidence | PLANNED |
| UsageReported | turn_id, tokens_in/out, cache_hits | PLANNED |
| TaskStateChanged | task_id, from, to | PLANNED |

**Rule**: events carry ids, not blobs. Large content (diffs, outputs) is
referenced by id and fetched on demand — keeps the event channel light.

---

## 30. Data Schemas

### 30.1 Thread file (`data/threads/<id>.json`) — IMPLEMENTED

```json
{
  "id": "thread-<hex>-<hex>",
  "title": "first user message (≤48 chars)",
  "messages": [
    { "role": "user|agent", "text": "...", "tool_calls": [
        { "id": "...", "name": "...", "arguments_json": "...",
          "ok": true, "summary": "first line ≤120 chars" } ] }
  ]
}
```

Rules: whole-snapshot writes; unknown fields tolerated on read (forward
compat); corrupt file → skipped with warning.

### 30.2 Provider config (`data/config.json`) — IMPLEMENTED

ProviderConfig serialization. Contains the API key — must move to OS
keychain (planned); until then it is gitignored and never logged.

### 30.3 Journal record (planned)

```json
{ "seq": 1234, "ts": "2026-08-23T00:00:00Z", "kind": "tool_executed",
  "thread_id": "...", "turn_id": "...", "payload": { ... } }
```

Kinds: command_received, turn_started, message_persisted, provider_called,
tool_approved, tool_executed, checkpoint_created, task_state_changed,
evidence_recorded, turn_finished, error_observed. Segment files rotate at
~16 MiB; each segment ends with a checksum line.

### 30.4 Task record (planned)

```json
{ "id": "task-...", "title": "...", "state": "running",
  "depends_on": ["task-..."], "budget": { "tokens": 200000, "minutes": 30 },
  "evidence": ["evidence-..."], "parent": null, "created_seq": 1200 }
```

### 30.5 Memory record (planned)

```json
{ "id": "mem-...", "layer": "project|semantic|episodic",
  "content": "...", "provenance": { "kind": "message|tool|user", "ref": "..." },
  "confidence": 0.8, "superseded_by": null,
  "anchors": [{ "kind": "file", "fingerprint": "..." }] }
```

---

## 31. Milestone Roadmap

Milestones are capability-based, not date-based. Exit criteria are testable.

| Milestone | Capability exit criteria |
|---|---|
| M0 — Published baseline | repo public; workspace tests green; canonical docs in place |
| M1 — Trustworthy core | steering queue; JSONL journal + replay; mid-run cancel; redaction on all sinks |
| M2 — Knowing agent | tree-sitter index actor; incremental updates; symbol/reference lookup; repo-map v2 |
| M3 — Safe hands | fingerprinted edits; atomic writes; patch tool; rollback; git read tools |
| M4 — Durable work | task graph + orchestrator v1; checkpoints; crash-recovery E2E test |
| M5 — Team of agents | sub-agent spawn policy; worktree isolation; result evaluation; budgets enforced |
| M6 — Honest completion | supervision pass; evidence engine; doom-loop breaker |
| M7 — Frugal intelligence | provider cache awareness; tool-result cache; lazy tools; usage dashboard |
| M8 — Full workspace | PTY terminal; diff viewer; editor v1; settings system; theme editor |
| M9 — Breadth | browser; diagrams; DB workspace; API workbench; artifacts |
| M10 — Autonomy | workflows; automation triggers; local models; router |

Each milestone's tasks are marked in the ledger; a milestone is done when
its exit criteria have named, passing tests — not when its tasks are
nominally checked.

---

## 32. Engineering Workflows (how to do common things)

### 32.1 Add a tool

1. Define ToolDef (name, description, JSON schema) in `tools::definitions`.
2. Implement handler in `tools.rs`; classify risk; scope-check inputs.
3. Route any spawn through sandbox; any output through redact + bound.
4. Add unit tests (happy path + failure modes + scope rejection).
5. Update §28 catalog + task ledger + DEVELOPMENT-STATE if behavior is
   user-visible.

### 32.2 Add a provider

1. Implement `provider::Provider` for the wire protocol (SSE streaming).
2. Register in `provider::from_config` match.
3. Never log credentials; forward deltas synchronously.
4. Test: boundary rejection for unknown kind; (replay harness later).

### 32.3 Add a panel

1. Model the panel state in z-shell (pure data + transitions).
2. Render it in z-app composition; tokens only for styling.
3. A11y nodes + keyboard navigation from day one.
4. Screenshot via `--shot`; add to visual review set.

### 32.4 Add a setting

1. Declare in settings schema (id, type, default, category, mode).
2. Consume via typed accessor; never read the file directly.
3. Migration entry if shape changes; search indexing automatic via schema.

### 32.5 Add a skill

1. `skills/<name>/SKILL.md` with frontmatter (name, description).
2. Sections: when-it-applies, files/modules, architecture, invariants,
   failure cases, testing expectations, DoD.
3. Real operational content only; no generic advice.

### 32.6 Make a decision

1. Draft `docs/adr/NNNN-title.md` (context → decision → consequences).
2. Link from DEVELOPMENT-STATE; supersede (never edit) prior ADRs.

### 32.7 Fix a bug

1. Reproduce with a failing test first (regression test).
2. Fix implementation; keep the test.
3. If the test itself was wrong, state why in the commit body.

---

## 33. Risk Register & Open Questions

| Risk | Impact | Mitigation |
|---|---|---|
| Provider API drift breaks streaming | turns fail | replay harness; contract tests per provider |
| Job Object behavior varies across Windows versions | sandbox escape | regression tests on CI matrix; suspended-spawn invariant |
| Token estimator drift vs real tokenizers | budget misjudgment | calibrate from provider usage reports |
| Journal growth unbounded | disk bloat | segment rotation + compaction (designed) |
| Single-maintainer bus factor | project stall | canonical docs (this spec, ledger, skills) as institutional memory |
| Scope creep into "everything app" | core dilution | plugin-first rule; domain modules behind contracts |
| GPL contamination via references | legal exposure | references git-ignored; reuse ledger; license review gate |

Open questions (tracked, decided via ADR when answered):
- SQLite vs refined JSONL for journal v2 (decide at M4 with real load data).
- In-process vs out-of-process extension boundary v1 (decide at SDK start).
- Embedding store choice for semantic retrieval (defer until lexical/
  structural proven insufficient).

---

## 34. Documentation Index

| Document | Role |
|---|---|
| docs/Z-DESKTOP-MASTER-SPEC.md | this document — product/architecture intent |
| docs/Z-DESKTOP-TASKS.md | 2500-task engineering ledger (living) |
| docs/Z-DESKTOP-REFERENCE-RESEARCH.md | research & clone playbook |
| docs/DEVELOPMENT-STATE.md | session resume state |
| docs/THIRD_PARTY_RESEARCH_AND_REUSE.md | reuse ledger |
| docs/research/*.md | dissections, capability matrix, differentiators |
| docs/adr/*.md | architecture decision records |
| skills/*/SKILL.md | operational engineering skills |
| tools/*.py | developer utilities (validators, scanners) |

---

## 35. Capability Family Playbooks

Each family: goal, component breakdown, dependency order, failure modes,
and completion criteria. These expand §25 into engineering-ready detail.

### 35.1 Trust & Evidence

**Goal**: no agent claim is accepted without system-captured proof.

Components:
1. Evidence record schema (§30.3 kinds) + journal attachment.
2. Capture hooks at execution points: sandbox exit codes, test-runner
   output parsing, diff generation on fs writes.
3. Claim-to-evidence linker: model text is scanned for claims ("tests
   pass", "build succeeds") and matched against captured evidence.
4. Supervision verdict: pass / fail / needs-review attached to turn end.
5. UI: evidence badges inline next to agent messages; drill-down to raw
   output.

Dependency order: journal → capture hooks → linker → supervision verdict →
UI.

Failure modes: evidence capture itself fails (record the gap — absence of
evidence is itself evidence); ambiguous claims (mark needs-review, never
auto-pass).

Completion criteria: a scripted agent that claims success without running
tests receives a failed verdict in an integration test.

### 35.2 Token Economy

**Goal**: minimum tokens per unit of competent work; measured, not assumed.

Components:
1. Estimator calibration loop (provider usage reports → adjust constants).
2. Prompt-prefix stability guard (test asserts byte-identical prefixes).
3. Tool-result cache with fingerprint keys.
4. Lazy tool definitions (advertise categories; full schema on demand).
5. Structured large-output summaries with exact-source pointers.
6. Context delta protocol (send only what changed when provider supports).
7. Usage dashboard (tokens/request, cache hit rate, cost estimate).

Dependency order: estimator (done) → budgeting (done) → prefix guard →
tool cache → lazy tools → delta protocol → dashboard.

Failure modes: stale cache serving wrong content (fingerprint discipline);
summary losing decision-critical detail (pinned-facts rule).

### 35.3 Repository Intelligence

**Goal**: the agent opens a task already knowing the codebase.

Components:
1. Index actor (owning thread, channel API, snapshot reads).
2. Tree-sitter grammar registry (Rust, TS/JS, Python first).
3. Symbol/reference/import extraction per language.
4. Fingerprint store + incremental re-parse.
5. Affected-analysis (change file X → impacted symbols/tests).
6. Lexical search index (trigram) + structural query API.
7. Repo-map v2 generator (token-budgeted, relevance-ranked).

Dependency order: actor → grammars → extraction → fingerprints →
incremental → affected → search → map v2.

Failure modes: parser panics on malformed source (isolate per-file, skip +
log); incremental drift vs full rebuild (parity tests); memory blowup on
huge repos (spill design from day one).

### 35.4 Durable Execution

**Goal**: work survives crashes, restarts, and interruptions.

Components:
1. JSONL journal with segment rotation + checksums.
2. Deterministic replay engine for threads/tasks.
3. Checkpoints (turn-level state snapshots referenced by journal seq).
4. Steering queue (drain between tool rounds; combine gate).
5. Mid-run cancellation flag inside sandbox wait loop.
6. Pause/resume of long turns.
7. Crash-recovery E2E test harness (kill -9 mid-turn, verify resume).

Dependency order: journal → replay → checkpoints → steering → cancel →
pause → recovery harness.

### 35.5 Orchestration

**Goal**: a supervised team of agents, not one chat.

Components: task graph store; orchestrator loop; sub-agent spawn policy;
worktree manager; result evaluator; budget enforcer; Best-of-N runner.

Dependency order: durable execution → safe editing + git → sub-agents →
worktrees → evaluation → Best-of-N.

### 35.6 Workspace Breadth

Terminal (PTY layer, virtualized scrollback, shell integration markers) →
diff viewer → editor v1 (buffer, highlight, LSP client) → browser →
diagrams → DB workspace → API workbench → data workspace → artifacts.

Each surface: shell model state → scene renderer → a11y → keyboard map →
screenshot test. No surface ships pointer-only or without virtualization
if content can be large.

### 35.7 Local-First Control

BYOK hardening (keychain), local model backend behind Provider trait,
hardware probe service, download manager with integrity checks, zero-
telemetry guarantee documented and tested (no outbound calls except
configured providers).

### 35.8 Calm Native UX

Theme editor, layout profiles, keybinding remap, command palette,
notification center (quiet), reduced-motion support, DPI correctness,
focus management audit.

---

## 36. UI Surface Specifications

Detailed specs for each planned surface. Format: purpose, state, actions,
empty/error states, performance notes.

### 36.1 Conversation view (exists; upgrades planned)

- State: messages (user/agent), streaming buffer, tool steps with status,
  pending approvals inline, steering queue indicator.
- Actions: send, stop, steer (queue), approve/deny, copy, retry.
- Empty state: explains how to start (open project, configure provider).
- Error state: turn failure shows cause + retry action.
- Perf: virtualized message list; shaped-run caching during stream.

### 36.2 Terminal panel (planned)

- PTY-backed; multiple instances; split support.
- Scrollback virtualized (ring buffer, search over scrollback).
- Shell integration markers colorize command/output boundaries.
- Agent-launched runs render with provenance badge ("agent, approved").

### 36.3 Diff viewer (planned)

- Unified/split toggle; word-wrap off by default; syntax-aware.
- Hunk-level accept/reject for agent-proposed patches.
- Virtualized for huge diffs; binary/image diffs handled gracefully.

### 36.4 File tree + editor (planned)

- Tree: lazy-load directories, git status coloring, fuzzy filter.
- Editor v1: buffer with undo, tree-sitter highlighting, go-to-def via
  repository intelligence, minimap later. Large files (>5 MB) open in
  read-only viewer.

### 36.5 Browser pane (planned)

- WebView instance pool; profile separation; devtools toggle (Developer
  Mode). Agent access only through approved structured-extraction tools.

### 36.6 Diagram canvas (planned)

- Renders mermaid-class sources; zoom/pan; export PNG/SVG.
- Generated diagrams come from repository intelligence queries.

### 36.7 Database workspace (planned)

- Connection manager (credentials in secret store), schema tree, query
  editor with schema-aware completion, results grid (virtualized),
  read-only default with explicit write mode.

### 36.8 API workbench (planned)

- Request composer (method/url/headers/body), environment variables,
  response inspector (pretty/raw/headers), history. Agent can draft and
  debug requests with approval for actual sends.

### 36.9 Data workspace (planned)

- CSV/Parquet/JSONL table view (virtualized), column stats, transform
  recipes as journaled steps, simple plots.

### 36.10 Context inspector (planned)

- Shows exactly what the last request contained: prefix, history, tool
  results, token estimates per section. Essential for token economy trust.

### 36.11 Memory inspector (planned)

- Browse memory layers, provenance, confidence; correct/delete entries.

### 36.12 Token usage dashboard (planned)

- Per-turn/per-day usage, cache hit rates, cost estimates from provider
  pricing tables (user-editable).

### 36.13 Diagnostics console (planned)

- Log stream with level filters, trace spans, metrics sparklines.

### 36.14 Settings pages (planned)

- Schema-rendered forms per §12; search bar; mode switch.

### 36.15 Command palette (planned)

- Fuzzy command/action search; the keyboard-first entry point to
  everything.

---

## 37. Protocol Evolution & Compatibility Policy

1. Additive-only while any persisted data or external consumer depends on
   a version: new Command/Event variants, new optional fields.
2. Removals require a deprecation window: variant marked deprecated in
   docs, logged on receipt, removed at a major boundary.
3. Persisted formats (threads, journal, settings) carry versions; readers
   accept N and N+1; writers emit current.
4. The ledger records every protocol change with its task ID.
5. Compatibility tests: golden JSON fixtures per variant family; a change
   that breaks a fixture requires an explicit migration note.

---

## 38. Quality Gates & Review Checklists

### 38.1 Per-change checklist

- [ ] Tests added/updated; suite green
- [ ] Security checklist if security-relevant (skills/z-security)
- [ ] Tokens-only styling if UI
- [ ] A11y nodes if new interactive element
- [ ] Docs updated (catalog/spec/ledger/state as applicable)
- [ ] No new unwraps outside tested invariants
- [ ] Performance-sensitive? benchmark recorded

### 38.2 Pre-push gate

- [ ] Full workspace tests green
- [ ] tools/security_scan.py clean (or findings triaged + documented)
- [ ] .gitignore accurate; no target/data/references staged
- [ ] DEVELOPMENT-STATE updated atomically
- [ ] Ledger statuses match evidence

### 38.3 Milestone gate

- [ ] Exit criteria have named passing tests
- [ ] Benchmarks recorded where relevant
- [ ] Known limitations documented in DEVELOPMENT-STATE
- [ ] No "temporary" code without a tracking task ID

---

## 39. Operational Runbooks

### 39.1 Session start (any future agent/human)

1. Read skills index (skills/*/SKILL.md frontmatter).
2. Read docs/DEVELOPMENT-STATE.md fully.
3. Skim ledger section headers for current milestone.
4. Verify workspace builds: cargo test --workspace.
5. Select dependency-unblocked tasks; confirm with ledger statuses.

### 39.2 Session end

1. Update ledger statuses with evidence.
2. Update DEVELOPMENT-STATE (atomic, complete).
3. Run pre-push gate; commit; push if verified.

### 39.3 Incident: secret found in history

1. Rotate the credential immediately (out-of-band).
2. Purge via history rewrite ONLY with owner approval (rule exception).
3. Record incident + rotation in DEVELOPMENT-STATE (no secret values).

### 39.4 Incident: sandbox escape suspected

1. Freeze: stop agent turns.
2. Reproduce with minimal case; capture process tree evidence.
3. Fix + regression test reproducing the escape.
4. Audit all spawn paths for the same class.

### 39.5 Recovery: corrupt data directory

1. Stop app. Back up data/ before touching anything.
2. Threads: salvage parseable files; quarantine corrupt ones.
3. Journal: replay up to last valid checksum; truncate tail.
4. Rebuild caches/indexes from scratch (they are disposable).

---

## 40. Decision Log Summary

Binding decisions live in docs/adr/*.md; this table indexes them.

| ID | Decision | Status |
|---|---|---|
| ADR-0001 | Blocking threads + channels over async runtime in core | Accepted |
| ADR-0002 | Custom GPU UI stack (ZeroGPUI) over WebView | Accepted |
| ADR-0003 | Suspended-spawn job assignment for sandbox | Accepted |
| ADR-0004 | JSONL journal before SQLite | Accepted (revisit M4) |
| ADR-0005 | Personal-first: no team/multi-user abstractions in core | Accepted |
| ADR-0006 | BYOK plaintext config interim; keychain planned | Accepted |

(ADR files are created as decisions are formalized; this table may lead the
files briefly during canonicalization.)

---

## 41. Crate-Level Engineering Detail

Function-and-structure level documentation of each crate as it exists and
as it will grow. This section is the bridge between architecture prose and
the actual source tree.

### 41.1 z-protocol detail

**lib.rs structure**:

```
pub enum Command { ConfigureProvider{config}, OpenProject{path},
                   SendMessage{thread_id,text}, CancelTurn{thread_id},
                   ResolveApproval{call_id,approved} }        // + planned
pub enum Event   { Accepted{command_id}, TurnStarted{..}, TextDelta{..},
                   TextDone{..}, TurnFinished{ok,error},
                   ApprovalRequested{..}, StepStarted{..}, StepFinished{..},
                   ProviderStatus{ok,message}, ProjectIndexed{path,files,symbols} }
pub struct ProviderConfig { kind, base_url, model, key }
pub enum ProviderKind { OpenAICompatible, Anthropic }            // extensible
pub enum Risk { ReadOnly, Write, Execute }
pub type Id = String;                                            // prefixed ids
```

**Growth path**: EnqueueMessage/Pause/Resume commands; SteeringQueued/
CheckpointCreated/EvidenceRecorded/UsageReported events; Permission types;
SettingsValue envelope; TaskState enum for orchestration.

**Testing**: serde round-trip per variant; unknown-field tolerance; Id
format validation helper.

### 41.2 z-core/runtime.rs detail

**Structures**:

| Item | Notes |
|---|---|
| `Runtime` | owns cmd_rx, event_tx, threads map, shared state, data_dir |
| `Shared` | provider slot, label, project_root, index, approval gate, cancelled set |
| `ApprovalGate` | HashMap<call_id, bool> + Condvar; wait with deadline |
| `Thread/StoredMessage/StoredToolCall` | serde-persisted conversation model |
| `run_turn` | the turn loop; MAX_TOOL_ROUNDS=24; retry-once on round-0 stream failure |
| `build_request` | system prompt + repo map + budgeted history |
| `trim_history` | clean-boundary-only front trimming (tested) |
| `stored_message_tokens` | estimator over text + tool calls |

**Behavioral contract**:
- start_turn: create/reuse thread → title from first message → persist →
  spawn worker.
- Worker loop: cancel check → build request → stream (forward deltas) →
  handle tool calls (approval → execute → record) → repeat.
- Every exit path persists the thread and emits TurnFinished.

**Planned additions**: steering queue drain between rounds; pause/resume
via checkpoint; journal writes alongside snapshots; usage accounting from
provider responses feeding tokens calibration.

### 41.3 z-core/tools.rs detail

**Public surface**: definitions() → Vec<ToolDef>; classify(name,args) →
Risk; describe(name,args) → human string; execute(ToolInvocation) →
ToolOutput; walk_files(dir, visit).

**scoped() algorithm** (do not modify without tests):
1. Canonicalize root; strip Windows verbatim prefix via char codes.
2. Absolute or root-relative candidate.
3. Lexical normalization resolving `.`/`..`; ParentDir popping past root
   start = scope escape error.
4. Final starts_with(root) check.

**bound()**: char-count cap at 12k with `[output truncated]` marker —
preserves head, never silently drops.

### 41.4 z-core/sandbox.rs detail

**run(command, cwd, timeout)** pipeline (see §16.2). Constants:
DEFAULT_TIMEOUT=120s, MAX_TIMEOUT=600s. Output caps: stdout 8 MiB,
stderr 2 MiB (read_capped keeps draining past cap without growing).

**Windows specifics**: CREATE_SUSPENDED=0x4; JobHandle with
KILL_ON_JOB_CLOSE; resume_main_thread via Toolhelp32 snapshot +
OpenThread(THREAD_SUSPEND_RESUME)+ResumeThread; INVALID_HANDLE_VALUE
compared with `as _` cast (windows-sys HANDLE is a pointer here).

**Unix specifics**: process_group(0) at spawn; kill(-pgid, SIGKILL).

**Test inventory** (all must stay green): normal completion, nonzero exit,
timeout kill w/ partial output, ceiling clamp, grandchild death on timeout,
grandchild death on normal exit, oversized stdout capped.

### 41.5 z-core/redact.rs detail

Rule list order matters (specific before generic): sk-ant → sk → xai → gh*
→ AKIA → AIza → bearer(prefix group) → assignment(prefix group).
Fingerprint = first2+last2 chars (`[redacted:<label>…xy12]`). Assignment
rules keep the key visible, mask only the value.

### 41.6 z-core/tokens.rs detail

estimate(): single pass counting CJK vs other chars; base=other/4 ceil;
CJK≈1 token each; symbol-density correction when >25% symbols in >64-char
latin runs. check_budget(): Ok / Trim / Compact classification against
soft target + hard limit.

### 41.7 z-core/repo.rs detail (v0)

RepoIndex::open(root): walks (skipping .git/node_modules/target/dist/build/
__pycache__/.next/venv), extracts lightweight symbols, builds map_text(n)
for the system prompt, exposes file_count/symbol_count. Known limitation:
full rescan on open; incremental actor planned (§35.3).

### 41.8 z-core/provider.rs detail

from_config(kind,...) → boxed Provider. OpenAI client: POST
{base}/chat/completions, SSE parse `data:` lines, tool_calls accumulated
across deltas by index. Anthropic client: POST {base}/v1/messages, system
extracted to top-level param, tool results sent as user-turn blocks.
StreamOutcome{text, tool_calls}. Unknown kind rejected at boundary (test).

### 41.9 z-shell detail

Layout model: region tree (H/V splits) with panel leaves; panel registry
with visibility/focus; presets as named layouts; view state holds active
thread id, streaming buffers keyed by turn, pending approvals mirror.
All transitions pure: (state, event) → state. Serialization versioned.

### 41.10 z-gpui detail

Subsystems: window (winit lifecycle, DPI), renderer (wgpu pipelines,
batching by material), scene (display-list value), text (cosmic-text cache),
timing (phase instrumentation), a11y (accesskit tree sync), screenshot
(offscreen capture to PNG via png crate), headless check mode.

### 41.11 z-tokens detail

Token structs for color/spacing/typography; theme assembly from primitive
maps; semantic resolution; validation (unknown refs error). 24 tests cover
resolution and defaults.

### 41.12 z-app detail

main.rs: arg parsing (--check exits after init validation; --shot <dir>
captures screenshots); wires channels Runtime↔pump↔shell; frame loop drains
EventQueue then rebuilds scene. content.rs/view.rs compose scenes from
shell state.

---

## 42. Test Plan Catalog

Per-subsystem test matrices. "✓" exists today; "○" planned.

| Subsystem | Happy path | Failure modes | Security | Perf |
|---|---|---|---|---|
| protocol serde | ✓ round-trip | ○ unknown fields | n/a | n/a |
| runtime turns | ✓ budget trim | ○ provider fail/retry, corrupt restore | ✓ deny→result | ○ soak |
| tools fs | ✓ read/write/list/search | ✓ empty query, oversize | ✓ traversal | ○ large trees |
| sandbox | ✓ 7 process tests | ✓ timeout/partial | ✓ grandchild×2, suspended-spawn | ✓ output cap |
| redaction | ✓ patterns+edges | ✓ short inputs | ✓ itself | ○ throughput |
| tokens | ✓ accuracy classes | ✓ tiny inputs | n/a | ✓ 1MiB speed |
| provider | ✓ kind rejection | ○ malformed SSE, timeouts | ✓ no creds in errors | ○ replay |
| shell model | ✓ transitions | ○ corrupt layout | n/a | ○ huge trees |
| gpui render | ✓ unit suite | ○ device loss | n/a | ○ frame bench |
| app wiring | ✓ --check/--shot | ○ pump backpressure | n/a | n/a |

Rule: every ○ becomes ✓ before its milestone exits (§31).

---

## 43. Threat Model (STRIDE per boundary)

### 43.1 Boundary: UI ↔ Core (channels)

- Spoofing: only local processes can reach channels; low risk personal-first.
- Tampering: events are trusted but validated on deserialize.
- Repudiation: journal records command/event lifecycle (planned closes gap).
- DoS: event flood — bounded queues + coalescing deltas (planned guard).

### 43.2 Boundary: Core ↔ Provider APIs

- Credential exposure: keys never logged; errors sanitized (invariant).
- Malicious/misbehaving provider: response size caps; SSE line limits
  (planned hardening).

### 43.3 Boundary: Core ↔ Filesystem

- Path traversal: scoped() lexical rejection (tested).
- Symlink escape: canonicalize-based checks; TOCTOU window documented —
  fingerprint-before-write (planned) closes write-side risk.
- Destructive ops: approval gate; delete-class tools require explicit
  confirmation design (planned).

### 43.4 Boundary: Core ↔ Spawned processes

- Escape via early grandchild: eliminated (suspended-spawn invariant).
- Escape via breakaway: no breakaway flag; tested.
- Resource exhaustion: output caps, wall-clock ceiling, stdin null.
- Orphaned trees on crash: KILL_ON_JOB_CLOSE / group kill on drop.

### 43.5 Boundary: Core ↔ Extensions/MCP (planned)

- Capability escalation: deny-by-default grants; declared-vs-used check.
- Supply chain: extension signing later; provenance recorded now.

### 43.6 Information disclosure

- Secrets in outputs: redaction funnel (implemented) extended to all sinks.
- Telemetry: none; verified by egress audit task (ledger).

---

## 44. Performance Budget Tables

Budgets are targets until measured (§21 discipline).

| Path | Budget | Measured |
|---|---|---|
| Command → TurnStarted event | < 50 ms | pending |
| First TextDelta after send | provider-bound | n/a |
| Tool exec overhead (non-process) | < 5 ms | pending |
| Sandbox spawn→resume | < 30 ms | pending |
| Redaction 1 MB text | < 20 ms | pending |
| Token estimate 1 MiB | < 100 ms | ✓ (test-bounded) |
| build_request (1k-msg history) | < 50 ms | pending |
| Scene build typical frame | < 2 ms | pending |
| EventQueue drain 1k events | < 5 ms | pending |
| Index v0 open (1k files) | < 2 s | pending |

Each pending row gets a benchmark task in the ledger; measured values
replace targets in docs/benchmarks.

---

## 45. End-to-End Data Flow Walkthroughs

### 45.1 Simple question (no tools)

1. User types → SendMessage command → Accepted event.
2. start_turn: thread created/titled, message persisted, TurnStarted.
3. Worker: build_request (system+map+budgeted history) → provider.stream.
4. TextDelta events stream to UI; TextDone; agent message persisted.
5. TurnFinished(ok). Thread saved.

### 45.2 Tool-using turn (read-only)

Steps 1–3 as above; response contains tool_call fs_read.
4. classify → ReadOnly → no approval → StepStarted → execute (scope check,
   read, bound) → StepFinished → result carrier persisted.
5. Next round: request includes assistant tool_calls + tool result.
6. Model answers in text → TextDone → finish.

### 45.3 Write-requiring turn

Same as 45.2 until classify → Write.
4. ApprovalRequested → UI prompt → ResolveApproval(true) → execute →
   result. Denial path: result "denied by user" reaches model instead.

### 45.4 Runaway command

terminal_exec with hanging child → wait loop hits deadline → guard.kill_tree()
(Job terminate) → reaped within 1 s → partial stdout preserved → output
includes [killed: ...] marker → model sees truth.

### 45.5 Crash mid-turn (target behavior)

Worker dies between tool rounds → thread snapshot already on disk contains
user msg + prior steps → restart restores thread → (M4) journal replay
offers resume; today the user re-sends with context intact.

---

## 46. Configuration Reference

### 46.1 Environment variables

| Var | Effect |
|---|---|
| Z_DESKTOP_DATA | override data directory (default ./data) |
| RUST_LOG | log filter for env_logger (dev) |

### 46.2 Data directory layout

```
data/
  config.json          BYOK provider config (secret! gitignored)
  threads/<id>.json    persisted conversations
  journal/             (planned) JSONL segments
  cache/               (planned) fingerprints, AST, retrieval caches
  memory/              (planned) memory layer stores
```

### 46.3 CLI arguments (zero-app)

| Arg | Behavior |
|---|---|
| --check | initialize stack headlessly, validate, exit 0/nonzero |
| --shot <dir> | capture screenshots to directory, exit |

### 46.4 Workspace profiles (Cargo.toml)

dev opt-level 1 (+deps 3) for fast test iteration; release thin-LTO,
codegen-units=1, panic=abort; profiling profile inherits release with
debug info and unwind.

---

## 47. Contribution & Review Guide

1. Branch from main; small focused changes; conventional commit prefixes.
2. Self-review against §38.1 checklist before requesting review.
3. Reviewer verifies: tests meaningful (not assertion-free), security
   checklist if relevant, docs updated, no debug leftovers.
4. Disagreements resolve toward spec principles (§4); if principle conflicts
   with reality, propose an ADR.
5. Never merge red CI; never merge with untriaged scan findings.

---

## 48. Anti-Pattern Encyclopedia (project-specific)

| Anti-pattern | Why it's banned | Correct approach |
|---|---|---|
| Raw `
`/`\\?\` literals in tool-written files | save pipeline mangles escapes | char::from(10)/char::from(92) |
| Bypassing scoped() for "internal" paths | one bypass normalizes ten | route through tools::execute |
| Spawning processes outside sandbox | breaks tree-kill guarantee | sandbox::run only |
| Logging ProviderConfig/key | secret leak | redact or omit |
| Unbounded Vec growth in services | long-session OOM | caps + eviction |
| Polling loops for state | idle CPU burn | event-driven |
| Tests changed to fit broken code | hides real bugs | fix code; document requirement change |
| Feature flags without removal plans | flag debt | time-boxed flags with tracking tasks |
| Copying reference code without ledger entry | license/legal risk | reuse ledger first |
| "Temporary" direct DB/file access in views | layering rot | core service + protocol |

---

## 49. Planned Domain Module Designs

Detailed internal designs for major planned domains. Each is a future
crate or core module speaking the protocol; none may bypass §4 principles.

### 49.1 Terminal domain

**Goal**: production-grade PTY terminals as panels and agent surfaces.

**Design**:
- `z-terminal` crate. Backend trait `Pty { spawn, resize, read, write,
  kill }` with ConPTY (Windows), openpty+fork (unix) implementations.
- Event model: PtyOutput chunks → shell ring buffer (fixed-capacity,
  e.g., 10 MB per instance) → virtualized renderer.
- Shell integration: emit OSC 133-style markers via wrapper script when
  supported; fallback heuristic parsing otherwise.
- Agent integration: non-interactive runs use sandbox::run (existing);
  interactive supervised sessions attach to a PTY with approval-gated
  input forwarding.
- Resize handling: reflow only the viewport; scrollback preserves raw
  bytes for search fidelity.
- Search: incremental scan over ring buffer; highlight + jump list.

**Invariants**: kill on panel close kills process group; no unbounded
memory regardless of output volume; agent input never bypasses approval.

### 49.2 IDE domain

**Goal**: editor-grade code surface backed by repository intelligence.

**Design**:
- Buffer layer: rope (ropey-class) per open file; edit transactions;
  fingerprint tracking for safe-editing integration.
- Highlighting: tree-sitter query sets per language; theme token mapping.
- LSP client: JSON-RPC over stdio per server; capability negotiation;
  requests routed through resource scheduler (priority: interactive).
- Navigation: go-to-def/references answered by repo index first (fast),
  LSP second (authoritative); results merged with provenance badges.
- Agent-edit review: proposed diffs render inline with accept/reject per
  hunk; acceptance routes through safe editing.

**Invariants**: buffer state is authoritative while open; external changes
detected via watcher + fingerprint mismatch → reload prompt; large-file
guard (>5 MB read-only).

### 49.3 Browser domain

**Goal**: embedded browsing for research/preview with controlled agent
access.

**Design**:
- WebView2 (Windows) / WebKitGTK (Linux) / WKWebView (macOS) behind a
  unified surface trait; instances pooled; one process per profile class.
- Profiles: user (persistent cookies) vs agent (ephemeral, isolated).
- Agent tools: browser_open(url), browser_read() → readability-extracted
  text + metadata, browser_act(...) limited interaction set — all
  approval-gated per risk classification.
- Download safety: quarantine directory + scan hook before opening.

**Invariants**: agent never sees user-profile cookies; page scripts cannot
reach Z Desktop internals (standard WebView isolation + no privileged
bridge exposed).

### 49.4 Diagram domain

**Goal**: render and generate diagrams natively.

**Design**:
- Renderer v1: mermaid-class subset rendered by our own layout engine
  (dagre-class layered layout) into the scene graph — no JS runtime.
- Sources: hand-written, generated from repo intelligence (dependency
  graphs, call graphs, architecture views), or from agent output.
- Export: PNG/SVG via offscreen capture; artifacts platform storage.
- Interaction: zoom/pan, node selection → linked navigation (jump to
  symbol in IDE surface).

### 49.5 Database workspace

**Design**:
- Connection registry (kind, DSN ref → secret store, read-only flag).
- Drivers: rusqlite (SQLite first), then postgres/mysql via pure-Rust
  clients where available.
- Schema tree cached with fingerprint invalidation; query editor with
  schema-aware completion from cached schema.
- Results grid virtualized; export CSV/JSONL; query history journaled.
- Safety: default connection mode read-only; writes require per-session
  explicit enablement + approval per statement class (DDL always).

### 49.6 API workbench

**Design**:
- Request model: method/url/headers/body/auth refs; environments as
  variable scopes; collections as files in workspace.
- Execution via shared HTTP client stack (ureq today; streaming bodies
  later); response inspector with size caps and binary detection.
- Agent assistance: draft requests from OpenAPI specs; debug failing
  requests using captured history; actual sends approval-gated.

### 49.7 Data workspace

**Design**:
- Readers: CSV (streaming, dialect sniffing), JSONL, Parquet (later).
- Table view virtualized over row cursor; column profiling (type inference,
  nulls, cardinality) computed lazily.
- Transforms: declarative recipe steps (filter/project/join/aggregate)
  recorded in journal; reproducible; exportable as scripts.

### 49.8 Automation engine

**Design**:
- Trigger types: cron (local scheduler), file-watch (debounced), manual,
  app-event (e.g., "tests failed").
- Runs are tasks in the orchestration graph with budgets; missed cron
  fires resolve once on wake (no stampede).
- Concurrency cap global + per-trigger; quiet hours respected.

### 49.9 Workflow engine

**Design**:
- Workflow = ordered steps referencing prompts/tools/approvals/gates;
  stored as versioned files; runs instantiate task records.
- Gates: human approval, evidence check, budget check, evaluator verdict.
- Failure policy per step: retry(n, backoff) / skip / abort / ask.

### 49.10 Artifact platform

**Design**:
- Artifact = content + type + provenance (turn/task) + version chain.
- Storage: content-addressed blobs + manifest index under data/artifacts.
- Surfaces: gallery panel, preview renderers per type (markdown, diagram,
  image, patch), share = export file (personal-first: no upload service).

### 49.11 Memory subsystem

**Design**:
- Store per layer under data/memory/<layer>/ as journal-derived views.
- Write paths: explicit user save, consolidation pass, supervised
  extraction from turns (candidate memories require confirmation).
- Retrieval API: query → ranked candidates with provenance + confidence;
  context injection respects context-engine budgets.
- Correction: user edit creates superseding record; dependents flagged.

### 49.12 Model router

**Design**:
- Registry entries: provider config ref, model id, capabilities (context
  window, vision, tools tier, cost per Mtok, latency class).
- Policy DSL: rules like "task.kind==search → cheapest-tools-tier".
- Fallback chains evaluated on failure classes (auth/rate/capacity).
- Decisions logged (task id, chosen model, reason) for audit + tuning.

### 49.13 Local models

**Design**:
- Backend abstraction: llama.cpp server process managed by us (first),
  others later — all behind Provider trait.
- Model catalog: local files with manifest (size, quant, ctx, license);
  download manager verifies hashes; disk-space checks.
- Scheduler integration: inference jobs declare VRAM/RAM needs; admission
  control defers during interactive spikes.

### 49.14 Hardware intelligence

**Design**:
- Probe service: CPU topology, RAM, GPU adapter info + VRAM (via wgpu
  adapter + OS APIs), thermals where available.
- Exposes snapshot + change events; consumers: router, scheduler,
  settings defaults ("recommended local model size").

### 49.15 Resource scheduler

**Design**:
- Priority classes: Interactive(0) > AgentTurn(1) > BackgroundIndex(2) >
  Batch(3). Admission control for class ≥2 when interactive latency
  budget at risk.
- Token/time budgets per class enforced at orchestrator level.
- Metrics exported for dashboard verification of fairness.

---

## 50. Keyboard & Focus Specifications

Global rules:
- Ctrl/Cmd+K palette; Ctrl/Cmd+P quick-open; Ctrl+` terminal toggle;
  Ctrl+B sidebar; Escape cancels transient states (drag, pending approval
  focus, search).
- Tab order follows visual order; Shift+Tab reverses; focus visible at
  all times; no focus traps outside modals.
- Per-surface maps defined in z-shell keymap tables (data, not code);
  conflicts resolved by scope specificity then recency.

Streaming behavior: incoming deltas never steal focus; approvals DO raise
attention (badge + optional sound setting, default subtle).

---

## 51. Error Message Catalog & UX Writing Guide

Rules:
1. Say what happened, why, and what to do next — in that order.
2. Never expose internals (paths of secrets, raw panics, credentials).
3. One sentence where possible; details behind expanders.
4. Examples (canonical tone):
   - "No provider configured — add your API key in Settings to start."
   - "That path is outside the project folder, so it was blocked."
   - "The command ran too long and was stopped. Partial output is shown."
   - "This file changed since it was read. Re-read it before editing."
5. Error codes (planned): stable ids (ZD-E-xxxx) for supportability,
   mapped to docs.

---

## 52. Dependency Policy & Approved Crates

Rules (skills/z-rust-engineering + dependency discipline):
- Every new dependency: necessity, maintenance health, license (MIT/Apache-
  2.0 preferred), platform support, size/build-time impact, security
  posture. Record decision in ADR if nontrivial.
- Approved core stack: serde/serde_json, regex, ureq (+rustls), log/
  env_logger, windows-sys, libc, winit, wgpu, cosmic-text, taffy,
  accesskit(+winit), png, bytemuck, pollster, raw-window-handle.
- Under evaluation: ropey (editor), tree-sitter (+grammars), git2 (or
  shelling out initially), notify (watchers), keyring (secrets).
- Banned patterns: heavyweight frameworks duplicating existing layers;
  crates requiring tokio in core; unmaintained (<1 year) without fork plan.

---

## 53. Release Engineering

- Versioning: workspace 0.x until public API stability; protocol crate
  carries its own minor for compatibility tracking.
- Tagging: vX.Y.Z on main; release builds from tags only.
- Changelog: Keep-a-Changelog format, generated from ledger milestone
  completions + curated highlights.
- Artifacts: per-platform archives (zip/tar.gz) + checksums; installer
  packaging later (§19).
- Release checklist: tests green on CI matrix, scan clean, changelog,
  tag push, artifact upload, verify download + smoke run.

---

## 54. Architecture Evolution Scenarios

Pre-committed responses to foreseeable scale events:

| Scenario | Response |
|---|---|
| Core exceeds ~40k LOC | split domains into crates (terminal, ide, ...) speaking protocol |
| >5 providers | extract provider registry crate; config UI |
| Sub-agents at scale | dedicated orchestrator thread pool; journal-backed queues |
| Million-line indexed repos | spill-to-disk index shards; mmap structures |
| Plugin ecosystem demand | out-of-process extension host; SDK stabilization freeze |
| Multi-window | shell model becomes per-window; shared runtime unchanged |
| Remote development | project root abstraction gains transport; sandbox stays local-first |

Each scenario has trigger metrics; revisit at architecture audits
(every 15–20 slices per DEVELOPMENT-STATE cadence).

---

## 55. Milestone Work Breakdown

Named work items per milestone with acceptance criteria. Ledger IDs are
assigned in docs/Z-DESKTOP-TASKS.md; this section defines WHAT DONE MEANS.

### 55.1 M1 — Trustworthy core

| Item | Acceptance criteria |
|---|---|
| Steering queue protocol | EnqueueMessage round-trips; queued depth event emitted |
| Steering drain | queued text applied between tool rounds; combine gate merges consecutive plain texts; test proves mid-turn injection |
| JSONL journal writer | append + fsync policy documented; segment rotation at 16 MiB |
| Journal replay | replay of fixture journal reproduces thread state exactly |
| Mid-run cancel flag | cancel during 30 s sleep kills within 500 ms; partial output kept |
| Redaction on log sink | file logger output contains zero secret patterns (scan-based test) |
| Redaction on journal | journal records redacted by construction |

### 55.2 M2 — Knowing agent

| Item | Acceptance criteria |
|---|---|
| Index actor | channel API; no shared mutable state; snapshot reads |
| Rust grammar extraction | symbols/refs/imports on fixture repo match hand count |
| TS/JS + Python grammars | same parity standard |
| Fingerprint store | unchanged file → zero reparse (asserted) |
| Incremental update | change 1 file → only its symbols diff (parity vs full rebuild) |
| Affected analysis | edit fn X → impacted callers listed correctly on fixture |
| Lexical search | trigram index; p95 < 200 ms on medium fixture |
| Repo-map v2 | token-budgeted; relevance-ranked; stable across no-change builds |

### 55.3 M3 — Safe hands

| Item | Acceptance criteria |
|---|---|
| Read fingerprints | fs_read records hash; stored per thread |
| Stale-write refusal | modify-between-read-write → refused with re-read guidance |
| Atomic writes | temp+rename; crash simulation leaves old or new, never partial |
| edit_patch tool | search/replace blocks with anchors; missing anchor fails cleanly |
| Rollback staging | multi-file op failure → zero partial state |
| git read tools | status/diff/log via library; read-only risk class |

### 55.4 M4 — Durable work

| Item | Acceptance criteria |
|---|---|
| Task record store | create/transition/query; journal-backed |
| Orchestrator v1 | runs ready tasks respecting dependencies; budgets enforced |
| Checkpoints | turn-level snapshot referenced by seq; restore works |
| Crash-recovery E2E | kill -9 mid-turn → restart → resume completes task |
| SQLite decision ADR | written with load data; either migrate or close question |

### 55.5 M5 — Team of agents

| Item | Acceptance criteria |
|---|---|
| Spawn policy | role/scope/budget/model parameters validated |
| Write grants | exclusive per file; overlap rejected at grant time |
| Worktree manager | create/list/cleanup; orphan detection |
| Result evaluation | parent verifies evidence before accepting child result |
| Best-of-N runner | N attempts, evaluator selection, budget cap honored |

### 55.6 M6 — Honest completion

| Item | Acceptance criteria |
|---|---|
| Evidence capture hooks | exit codes, test output, diffs captured automatically |
| Claim linker | "tests pass" claim without evidence → flagged |
| Supervision verdict | pass/fail/needs-review attached to TurnFinished |
| Doom-loop breaker | identical failing call ×N aborts turn with diagnosis |
| Placeholder detector | TODO-only implementations flagged in review surface |

### 55.7 M7 — Frugal intelligence

| Item | Acceptance criteria |
|---|---|
| Prefix stability guard | byte-identical prefix test in CI |
| Usage accounting | provider usage parsed → UsageReported events |
| Estimator calibration | constants adjust from usage deltas; accuracy report |
| Tool-result cache | fingerprint-keyed; hit path verified end-to-end |
| Lazy tools | category advertisement; schema fetched on demand |
| Usage dashboard | tokens/request, cache rate, cost estimate rendered |

### 55.8 M8 — Full workspace

| Item | Acceptance criteria |
|---|---|
| PTY layer | ConPTY + unix pty behind trait; kill-on-close proven |
| Terminal panel | virtualized scrollback; resize; search |
| Diff viewer | unified/split; hunk accept/reject wired to safe editing |
| Editor v1 | buffer+undo+highlight+go-to-def on fixtures |
| Settings system | schema-driven; searchable; migrations tested |
| Theme editor | token editing with live preview |

### 55.9 M9 — Breadth

Browser pane · diagram engine · DB workspace · API workbench · data
workspace · artifact gallery — each with its §49 design's invariants
tested and screenshot-reviewed.

### 55.10 M10 — Autonomy

Workflow engine · automation triggers · local model backend · model router
— each journaled, budgeted, and supervised like everything else.

---

## 56. Protocol Message Reference (field-level)

### Commands

**ConfigureProvider** `{ config: ProviderConfig }`
- ProviderConfig { kind: OpenAICompatible|Anthropic, base_url: string,
  model: string, key: string }.
- Effects: provider slot replaced; ProviderStatus emitted; config.json
  persisted (secret).

**OpenProject** `{ path: string }`
- Validates directory; indexes (v0); ProjectIndexed{path, files, symbols}.

**SendMessage** `{ thread_id: Id, text: string }`
- Creates/reuses thread; spawns turn worker. Non-blocking.

**CancelTurn** `{ thread_id: Id }` — cooperative cancel flag insert.

**ResolveApproval** `{ call_id: Id, approved: bool }` — gate resolution;
unknown call_id ignored safely.

*(Planned messages carry analogous field tables in their design sections.)*

### Events

**Accepted** `{ command_id: u64 }` — first response to any command.
**TurnStarted** `{ thread_id, turn_id }`.
**TextDelta** `{ thread_id, turn_id, delta: string }` — ordered per turn.
**TextDone** `{ thread_id, turn_id }`.
**TurnFinished** `{ thread_id, turn_id, ok: bool, error?: string }`.
**ApprovalRequested** `{ thread_id, call_id, tool, detail, risk }`.
**StepStarted** `{ thread_id, turn_id, call_id, tool, detail }`.
**StepFinished** `{ ..., ok: bool, summary: string }`.
**ProviderStatus** `{ ok: bool, message: string }`.
**ProjectIndexed** `{ path, files: u64, symbols: u64 }`.

Ordering guarantee: per-thread event order matches causal order; cross-
thread interleaving is unspecified.

---

## 57. Sandbox Deep Specification

### 57.1 State machine

```
SpawnSuspended ──assign job──▶ AssignedSuspended ──resume──▶ Running
     │                              │                          │
     │ fail                         │ resume-fail              │ deadline / natural exit
     ▼                              ▼                          ▼
  Kill+Wait ◀───────────────────── Kill+Wait            ReapLoop(≤1s) ──▶ Done
```

Any transition into Kill+Wait terminates the tree and waits; no path leaves
a live unmanaged process.

### 57.2 Timing guarantees

- Deadline check granularity: 25 ms poll loop.
- Post-kill reap window: 50 × 20 ms polls, then unconditional wait().
- Reader threads: joined after status resolution; EOF guaranteed by tree
  death (all pipe writers dead).

### 57.3 Edge cases handled

| Case | Behavior |
|---|---|
| Child exits before resume | OpenThread fails → scan continues → error if none; suspended zombie reaped |
| Grandchild holds pipes | pre-start redirect pattern in tests; production commands should redirect; cap prevents OOM regardless |
| Timeout=0 requested | clamped to minimum meaningful slice; effectively immediate kill after start |
| spawn succeeds, attach fails | explicit kill+wait (orphan fix) |
| Job assignment on already-dead pid | AssignProcessToJobObject fails → clean error path |

### 57.4 Unix parity notes

Process group set at spawn (no race equivalent needed — group membership is
atomic with exec). SIGKILL to -pgid. Drop-time best-effort kill documented
as weaker than Windows KILL_ON_JOB_CLOSE; acceptable for personal use,
revisit if daemon mode emerges.

---

## 58. Context Assembly Algorithm (normative)

```
assemble(thread, shared):
  fixed   = identity_prompt + repo_map(budgeted) + tool_schemas
  fixed_t = estimate(fixed)
  soft    = HARD_LIMIT - COMPLETION_RESERVE          # 128k - 12k
  hist_budget = soft - fixed_t - SAFETY(16)
  history = trim_history(thread.messages, hist_budget)
  request = [System(fixed)] ++ render(history)
  assert structural_validity(request)                # tool_call/result pairing
  return request
```

trim_history:
1. costs[i] = estimate(message i incl. tool calls).
2. If total ≤ budget → return all.
3. Walk boundaries oldest→newest; suffix(i)=Σcosts[i..]; first clean
   boundary (real user msg) with suffix ≤ budget wins (max history kept).
4. None fits → last clean boundary (max valid trim).
5. No clean boundary → send untrimmed (validity over budget).

Structural validity: every assistant tool_call has a following result
carrier; request starts with System; alternation constraints per provider
handled in provider rendering layer.

---

## 59. Journal Format Deep Spec

### 59.1 File layout

```
data/journal/seg-000001.jsonl
data/journal/seg-000002.jsonl
...
```

Each line: one JSON record (§30.3). Segment closes with:
`{"kind":"segment_end","seq":N,"checksum":"fnv1a64-of-bytes"}`.

### 59.2 Writer rules

- Append-only; O_APPEND writes; fsync at checkpoint boundaries and every
  N records (configurable; default 64).
- Rotation when size > 16 MiB → new segment; active segment name in
  `data/journal/CURRENT`.

### 59.3 Reader/replay rules

- Read segments in order; stop at first invalid line; truncate tail beyond
  last valid record on repair.
- Replay applies records to views deterministically; view builders must be
  pure functions of (state, record).
- Seq gaps indicate lost records → warn + continue (personal-first: never
  block startup on journal issues).

### 59.4 Compaction

Background compaction folds segments older than K days into a snapshot
record + fresh segment; snapshots carry full state hash for verification.

---

## 60. Testing Cookbook (repo-specific recipes)

### 60.1 Process-behavior test

Use sandbox::run directly with real commands; serialize global-process
probes (PING_LOCK pattern); poll external state with bounded retries;
never assume timing below 100 ms.

### 60.2 Pure-logic test

Inline mod tests; descriptive behavior names; table-driven where inputs
vary; assert on observable outputs only.

### 60.3 Protocol test

Round-trip serde for each variant; golden JSON fixtures under
crates/z-protocol/tests/fixtures (planned); unknown-field tolerance test.

### 60.4 UI scene test

Build shell state → compose scene → assert scene contents (pure); visual
checks via --shot captures reviewed manually + diffed in CI later.

### 60.5 Failure-injection test

Wrap the failing dependency behind a trait in tests; inject timeout/error/
corruption; assert graceful degradation paths emit correct events/results.

### 60.6 Soak test (planned)

Run service under synthetic load for N minutes; assert memory delta <
threshold and no thread/handle growth (counted via diagnostics).

---

## 61. Acronyms & Expanded Glossary

| Term | Expansion / detail |
|---|---|
| PTY | pseudo-terminal; ConPTY on Windows, openpty on unix |
| SSE | server-sent events; provider streaming wire format |
| LSP | Language Server Protocol |
| OSC 133 | shell integration escape sequence marking command/output |
| BYOK | bring your own key |
| DoD | definition of done |
| STRIDE | Spoofing/Tampering/Repudiation/Info-disclosure/DoS/Elevation |
| FNV-1a | fast non-cryptographic hash used for segment checksums |
| TOCTOU | time-of-check-to-time-of-use race |
| HMR | hot module reload (live preview) |
| DDL/DML | data definition/manipulation language (DB safety classes) |
| Trigram index | 3-char substring inverted index for lexical search |

---

## 62. Engineering FAQ

**Q: Why not tokio?** Core concurrency is thread-per-turn with blocking
I/O; an async runtime adds complexity without benefit here (ADR-0001).

**Q: Why a custom UI stack?** Native feel, GPU efficiency, full control,
no WebView overhead (ADR-0002); Zed demonstrates viability at scale.

**Q: Why JSONL before SQLite?** Zero-dependency start, trivially debuggable,
adequate at personal scale; migration path documented (ADR-0004).

**Q: Can agents bypass approvals?** No. Risk classification is code-level;
unknown tools fail closed; there is no trusted-mode bypass by design.

**Q: What happens to my data?** It stays in ./data on your disk. No
telemetry exists; egress limited to configured providers.

**Q: How do I know an agent didn't fake its work?** Evidence Engine (M6)
attaches system-captured proof to claims; until then, StepFinished events
carry real summaries and the terminal shows real output.

---

## 63. Public API Reference Tables

Signatures and contracts of the current public surface (verify against
source; this table is documentation, not generated).

### z-core

| Item | Signature | Contract |
|---|---|---|
| Runtime::new | (event_tx: Sender<Event>, cmd_rx: Receiver<(u64, Command)>) -> Self | creates data dirs; restores threads |
| Runtime::serve | (self) | blocks on command loop until channel closes |
| data_dir | () -> PathBuf | Z_DESKTOP_DATA override or ./data |
| sandbox::run | (&str, &Path, Option<Duration>) -> Result<ExecOutcome, String> | tree-isolated bounded exec |
| sandbox::DEFAULT_TIMEOUT / MAX_TIMEOUT | Duration consts | 120 s / 600 s |
| redact::redact | (&str) -> String | fingerprinted masking |
| tokens::estimate | (&str) -> usize | heuristic count |
| tokens::estimate_messages | (&[Message]) -> usize | + per-message overhead |
| tokens::check_budget | (usize, usize, usize) -> Budget | Ok/Trim/Compact |
| tools::definitions | () -> Vec<ToolDef> | advertised schemas |
| tools::classify | (&str, &Value) -> Risk | unknown → Execute |
| tools::execute | (ToolInvocation) -> ToolOutput | the only FS/process funnel |
| tools::walk_files | (&Path, &mut FnMut(&Path)) | skips dependency dirs |
| repo::RepoIndex::open | (&Path) -> RepoIndex | v0 walk+symbols |
| provider::from_config | (ProviderConfig) -> Result<Box<dyn Provider>, String> | kind dispatch |

### z-protocol

All Command/Event variants (see §56), ProviderConfig, Risk, Id — serde
Serialize+Deserialize+Clone+Debug throughout.

### z-shell / z-tokens / z-gpui / z-app

Internal APIs documented in crate docs; stability expectations: shell model
types are serde-stable (persisted layouts); gpui internals may churn until
M8; app composition is glue.

---

## 64. UI Component Specifications

Shared components every surface composes from. Tokens-only styling; a11y
mandatory; keyboard operable.

| Component | Spec highlights |
|---|---|
| Button | variants primary/secondary/danger/ghost; focus ring token; disabled state explains why via tooltip when non-obvious |
| Text input | label always visible (not placeholder-only); validation inline; paste-safe |
| Multiline editor | auto-grow up to cap; scroll after; undo integrated |
| List (virtualized) | O(visible); keyboard nav (arrows/home/end/type-ahead); selection model single/multi per context |
| Tabs | overflow menu; close buttons with confirm-on-dirty; drag reorder later |
| Dialog | modal focus trap; Escape = cancel; destructive actions require typed confirmation when irreversible |
| Tooltip | delayed 400 ms; never blocks pointer; dismiss on scroll |
| Badge/status dot | semantic colors from tokens; counts capped "99+" |
| Empty state | icon + one-line explanation + primary action button |
| Skeleton | only where real progress cannot be computed; never fake duration |
| Toast/notification | quiet by default; stack max 3; errors persist until dismissed |
| Splitter | draggable + double-click reset; keyboard adjustable; min sizes enforced |

Component tests: scene-level assertions for structure; screenshot review
for visuals; a11y assertions for labels/roles.

---

## 65. Agent Prompt Engineering Standards

**System prompt** (runtime build_request):
1. Identity + behavioral rules (precise, honest, economical).
2. Project root + repository map (budgeted slice).
3. Active model label.
4. Rules are short, testable statements; no essays.

**Tool descriptions**: one paragraph each; state purpose + constraints +
failure semantics ("returns [killed] marker on timeout"). Descriptions are
part of the stable prefix — changes invalidate provider caches; batch them.

**Few-shot policy**: none in system prompt today; examples belong in skill
docs or task templates, not the fixed prefix.

**Steering text**: queued user messages combine into a single appended
user turn between tool rounds; format: original task context preserved,
steering prefixed with "User steering:" marker.

---

## 66. Internationalization Posture

- Personal-first: English-only UI strings today; all user-facing strings
  centralized (error catalog §51) so extraction is mechanical later.
- No string concatenation for sentences; message templates with parameters
  from day one.
- CJK-aware text handling already required (tokens estimator, shaping via
  cosmic-text); RTL support deferred but layout must not hardcode LTR
  assumptions in shared components.

---

## 67. Legal & Licensing Posture

- **No project license chosen yet** — recorded explicitly; do not fabricate
  one. Decision tracked as an open question for the owner.
- Third-party code reuse requires ledger entry + license compatibility
  review (skills/z-research).
- references/external clones are git-ignored and never distributed.
- Dependency licenses audited at release time (cargo-license/cargo-deny
  planned gate).
- Trademark care: no bundled third-party logos without permission.

---

## 68. Metric Definitions (normative)

Precise definitions so dashboards mean what they say:

| Metric | Definition |
|---|---|
| turn_duration_ms | TurnStarted → TurnFinished wall time |
| tool_latency_ms | StepStarted → StepFinished per call |
| tokens_in/out | provider-reported usage per request/response |
| cache_hit_rate | cached-prefix hits ÷ requests (provider-reported when available) |
| frame_time_ms | scene build + render submit per frame (timing module) |
| input_echo_ms | key event → first paint of its effect |
| index_files_per_min | files parsed ÷ elapsed during initial index |
| incremental_reindex_ms | file-change event → index updated |
| search_p95_ms | 95th percentile query latency over rolling window |
| journal_lag_records | records written − records fsynced |
| memory_rss_mb | process RSS sampled every 30 s |
| thread_count | live OS threads attributed to subsystems |

---

## 69. Data Retention & Privacy

- Everything local; nothing leaves the machine except provider calls with
  user-configured keys.
- Retention defaults: threads kept indefinitely (user-deletable); journal
  segments compacted after 90 days (configurable); caches evictable freely;
  memories retained until corrected/deleted.
- Deletion is real deletion (files removed), not soft-delete flags.
- Export: threads/settings exportable as JSON for portability (planned).

---

## 70. Onboarding Guide (new contributor or agent session)

Day-one path:
1. Read skills frontmatter index → pick relevant skills for your task.
2. Read DEVELOPMENT-STATE completely (10 minutes; saves hours).
3. Skim Master Spec §1–§8 for identity/architecture; deep-read sections
   relevant to your task.
4. Build + test: `cargo test --manifest-path "z desktop/Cargo.toml"
   --workspace` (expect green; if not, stop and diagnose before anything).
5. Run the app headless: `cargo run -p zero-app -- --check`.
6. Pick a ledger task marked `[ ] [ ]` whose dependencies are `[x] [ ]`;
   read its notes; implement per §32 workflow; update statuses with
   evidence; commit; push.

Rules that will get your PR rejected instantly: weakening tests, bypassing
scope/sandbox/redaction, hardcoded colors, untracked "temporary" code,
secret in commit, history rewrite.

---

## 71. Subsystem Interaction Matrix

Who calls whom, with what. Rows call columns.

| From \ To | protocol | runtime | provider | tools | sandbox | redact | tokens | repo | shell | gpui | app |
|---|---|---|---|---|---|---|---|---|---|---|---|
| z-app | Command/Event | spawn+channels | — | — | — | — | — | — | state updates | scene submit | self |
| runtime | types | self | stream() | execute() | via tools | via tools | estimate/budget | map_text | — | — | events out |
| provider | ChatRequest types | — | self | — | — | — | — | — | — | — | — |
| tools | Risk | — | — | self | run() | redact() | — | — | — | — | — |
| sandbox | — | — | — | — | self | — | — | — | — | — | — |
| gpui | Event (via queue) | — | — | — | — | — | tokens | — | layout model | self | — |
| shell | Event shapes | — | — | — | — | — | — | — | self | — | — |

Rule: no arrow may skip a layer downward (app never calls provider; gpui
never calls runtime directly).

---

## 72. Milestone Dependency Graphs

Text DAGs; arrows mean "blocks".

```
M1: journal ──▶ replay ──▶ checkpoints          steering ◀── protocol-ext
      └────────▶ recovery-harness               midrun-cancel ◀── sandbox-ext

M2: index-actor ──▶ grammars ──▶ extraction ──▶ fingerprints
        ──▶ incremental ──▶ affected ──▶ search ──▶ repo-map-v2

M3: fingerprints(read) ──▶ stale-refusal ──▶ atomic-writes
        ──▶ edit-patch ──▶ rollback ──▶ git-read-tools

M4: journal(M1) ──▶ task-store ──▶ orchestrator ──▶ crash-e2e
M5: M4 + M3 ──▶ subagents ──▶ worktrees ──▶ evaluation ──▶ best-of-n
M6: M1(journal) ──▶ evidence-hooks ──▶ claim-linker ──▶ verdicts
M7: usage-events ──▶ calibration ──▶ tool-cache ──▶ lazy-tools ──▶ dashboard
M8: pty ──▶ terminal-panel; diff-viewer ◀── safe-editing(M3)
      editor-v1 ◀── M2(index); settings ◀── schema; theme-editor ◀── tokens
M9: browser/diagram/db/api/data/artifacts (each independent once M8 lands)
M10: workflows ◀── M4; automation ◀── workflows; local-models ◀── hw+sched;
       router ◀── registry
```

---

## 73. Tool Schema Examples (canonical JSON)

### fs_read (implemented)

```json
{"name":"fs_read","description":"Read a text file inside the project.",
 "parameters":{"type":"object",
   "properties":{"path":{"type":"string","description":"Project-relative or absolute path"}},
   "required":["path"]}}
```

### terminal_exec (implemented)

```json
{"name":"terminal_exec",
 "description":"Run a shell command in the project directory. Returns stdout/stderr and the exit code. The process tree is killed automatically if it exceeds the timeout.",
 "parameters":{"type":"object",
   "properties":{
     "command":{"type":"string","description":"Shell command line"},
     "timeout_ms":{"type":"integer","description":"Optional wall-clock budget in milliseconds; default 120000, hard maximum 600000"}},
   "required":["command"]}}
```

### edit_patch (planned)

```json
{"name":"edit_patch",
 "description":"Apply search/replace blocks to a file previously read this session. Fails if any anchor text is absent (file drifted).",
 "parameters":{"type":"object",
   "properties":{
     "path":{"type":"string"},
     "read_fingerprint":{"type":"string","description":"Fingerprint from your last fs_read of this file"},
     "edits":{"type":"array","items":{"type":"object","properties":{
        "search":{"type":"string"},"replace":{"type":"string"}},"required":["search","replace"]}}},
   "required":["path","read_fingerprint","edits"]}}
```

### git_status (planned)

```json
{"name":"git_status","description":"Show working tree status (read-only).",
 "parameters":{"type":"object","properties":{},"required":[]}}
```

Schema rules: flat objects; explicit required arrays; descriptions on every
property; enums as string lists; no nested depth > 2 (model reliability).

---

## 74. Event Sequence Diagrams (text)

### 74.A Approval flow

```
UI                    Runtime worker              Gate
 │ SendMessage ───────▶│                            │
 │◀─ Accepted/TurnStarted                           │
 │                     │ stream → tool_call(write)  │
 │◀─ ApprovalRequested ────────────────────────────▶│ wait(call_id)
 │ ResolveApproval ───▶│───────────────────────────▶│ resolve
 │                     │◀── Some(true) ─────────────│
 │◀─ StepStarted/Finished                           │
 │◀─ ... rounds ...                                 │
 │◀─ TurnFinished                                  │
```

### 74.B Timeout flow

```
worker: deadline hit → guard.kill_tree()
      → reap loop ≤1s → partial output captured
      → result "[killed: ...]" persisted → next round sees truth
```

### 74.C Steering flow (planned)

```
UI: EnqueueMessage ──▶ queue(depth=1) ──▶ SteeringQueued event
worker: between rounds → drain queue → combine texts
      → SteeringApplied → appended user turn in next request
```

### 74.D Crash-recovery flow (planned, M4)

```
crash mid-turn → restart → load threads + replay journal to last seq
      → task marked interrupted → orchestrator offers resume
      → resume from checkpoint → completes → evidence recorded
```

### 74.E Index update flow (planned, M2)

```
watcher: file changed ──▶ index actor
actor: fingerprint differs? ──yes──▶ reparse file ──▶ diff symbols
      ──▶ update reverse edges ──▶ publish snapshot version
agent: query ──▶ latest snapshot (never blocks on parse)
```

### 74.F Best-of-N flow (planned, M5)

```
orchestrator: create N tasks in N worktrees (budget-capped)
      → run attempts concurrently → collect evidence per attempt
      → evaluator scores (tests, diff quality, budget used)
      → winner merged via safe-editing; losers cleaned up
```

### 74.G Provider failover flow (planned, router)

```
provider A fails (rate limit) → router classifies failure
      → fallback chain: B (same tier) → C (lower tier, flagged)
      → decision logged with reason → turn continues transparently
```

### 74.H Evidence verification flow (planned, M6)

```
agent text claims "tests pass" → claim linker scans text
      → finds no test-run evidence in turn log → verdict needs-review
      → UI shows unverified badge; supervision blocks auto-complete
```

---

## 75. Initial Settings Schema Draft

| id | type | default | mode | notes |
|---|---|---|---|---|
| provider.kind | enum | openai_compatible | user | openai_compatible \| anthropic |
| provider.base_url | string | https://api.openai.com/v1 | user | |
| provider.model | string | gpt-4o-mini | user | free text |
| project.last_path | path | "" | user | remembered |
| theme.active | string | "z-dark" | user | token-set name |
| agent.max_tool_rounds | int | 24 | dev | doom-loop ceiling |
| agent.approval_timeout_s | int | 300 | dev | gate deadline |
| context.hard_limit_tokens | int | 128000 | dev | window size |
| context.completion_reserve | int | 12000 | dev | |
| exec.default_timeout_ms | int | 120000 | dev | sandbox |
| exec.max_timeout_ms | int | 600000 | dev | hard ceiling |
| exec.output_cap_stdout_mb | int | 8 | dev | |
| exec.output_cap_stderr_mb | int | 2 | dev | |
| ui.reduced_motion | bool | system | user | system\|on\|off |
| diagnostics.log_level | enum | info | dev | error..trace |
| experimental.* | various | off | dev | time-boxed flags |

Rules: secrets NEVER here; every entry searchable; migrations versioned.

---

## 76. Theme Token Catalog Draft (semantic layer)

| Token | Role |
|---|---|
| surface / surface.raised / surface.sunken | background hierarchy |
| text.primary / text.muted / text.disabled | text roles |
| accent / accent.hover / accent.subtle | interactive emphasis |
| danger / warning / success / info | status family (+ .bg/.fg variants) |
| border.subtle / border.strong | separators and outlines |
| focus.ring | keyboard focus indicator |
| syntax.keyword/string/comment/type/function/number | editor palette |
| terminal.ansi0..ansi15 | terminal palette |
| chart.categorical1..8 | diagram/chart series |
| motion.fast/base/slow | duration tokens |
| space.xs/sm/md/lg/xl | spacing scale refs |
| radius.sm/md/lg/pill | corner scale refs |

Dark theme ships first ("z-dark"); light ("z-light") and high-contrast
follow; all three must pass contrast gates (§14).

---

## 77. Code Style Exemplars (from this codebase)

**Doc comment stating WHY + invariant**:

```rust
/// Trim history to `budget` tokens by dropping whole turns from the FRONT.
///
/// Safety rules:
/// - A cut may only happen at a "clean boundary"...
fn trim_history(msgs: &[StoredMessage], budget: usize) -> Vec<StoredMessage>
```

**Behavior-named test**:

```rust
#[test]
fn trimming_never_orphans_a_tool_result_carrier() { ... }
```

**Escape-safe literal construction** (pipeline-proof):

```rust
let prefix: String =
    [char::from(92), char::from(92), '?', char::from(92)].iter().collect();
```

**Fail-closed classification**:

```rust
_ => { let _ = args; Risk::Execute }
```

**Fire-and-forget events**:

```rust
let _ = event_tx.send(Event::TurnStarted { .. });
```

New code should read like these excerpts; when it cannot, add a comment
explaining the deviation.

---

## 78. Honest Limitations Appendix (as of publication)

Stated plainly, so nobody mistakes ambition for reality:

1. Single conversation thread surfaced in UI; multi-thread management not
   wired.
2. No settings UI yet; BYOK via data/config.json only.
3. Repo index is v0 (walk + light symbols); no AST/references yet.
4. No steering, pause/resume, checkpoints, or journal yet.
5. Redaction covers tool output only.
6. Mid-run cancellation of an in-flight tool is not possible yet.
7. Windows is the only tested platform today.
8. No CI pipeline yet; tests run locally.
9. No license chosen yet (§67).
10. The agent loop has not been exercised against live providers in CI;
    manual testing only so far.

These map to ledger tasks; each limitation's removal is a checkable event,
not a vibe.

---

## 79. Function Inventory (current implementation)

Complete inventory of implemented functions with one-line contracts.
Maintained manually; drift is a review finding.

### z-core/src/runtime.rs

| Function | Contract |
|---|---|
| Runtime::new | init data dir, restore threads, wire channels |
| Runtime::serve | command loop: dispatch each Command variant |
| configure_provider | build provider, update label/slot, emit status, persist config |
| open_project | validate dir, build RepoIndex, store, emit ProjectIndexed |
| start_turn | create/reuse thread, title, persist, spawn worker |
| persist | whole-thread JSON snapshot write |
| run_turn | turn loop: rounds of stream→tools; all exits persist+finish |
| record_result | attach ok/summary to stored call; emit StepFinished |
| is_cancelled | cancelled-set membership check |
| save_thread | snapshot write helper (worker-side) |
| stored_message_tokens | estimator over message incl. tool calls |
| trim_history | clean-boundary front trim to budget (tested) |
| build_request | assemble system+map+budgeted history into ChatRequest |

### z-core/src/tools.rs

| Function | Contract |
|---|---|
| bound | char-cap output at 12k with truncation marker |
| classify | name→Risk; unknown fail-closed Execute |
| describe | human-readable detail for events/approvals |
| fmt_arg | string arg extraction with "?" fallback |
| definitions | ToolDef list advertised to providers |
| scoped | lexical path normalization + scope enforcement |
| strip_verbatim | remove Windows \\?\ prefix via char codes |
| execute | dispatch to handlers; only FS/process funnel |
| fs_read/fs_list/fs_search/fs_write | file operations per §28 specs |
| terminal_exec | sandbox-backed exec w/ optional timeout |
| walk_files | recursive walker skipping dependency dirs |

### z-core/src/sandbox.rs

| Function | Contract |
|---|---|
| run | full pipeline: suspended spawn → job → resume → drain → deadline → kill → reap |
| read_capped | drain pipe to EOF, buffer capped |
| spawn | platform shell construction (+CREATE_SUSPENDED on win) |
| Guard::attach | Job (win) / Group (unix) creation for child |
| Guard::kill_tree | TerminateJobObject / kill(-pgid) |
| JobHandle::for_child | create KILL_ON_JOB_CLOSE job, assign process |
| JobHandle::terminate | TerminateJobObject wrapper |
| winjob::resume_main_thread | Toolhelp32 find thread, ResumeThread |

### z-core/src/redact.rs

| Function | Contract |
|---|---|
| rules | lazy static rule list (order matters) |
| redact | apply all rules with fingerprinted replacement |
| fingerprint | first2+last2 chars or asterisks for short |

### z-core/src/tokens.rs

| Function | Contract |
|---|---|
| estimate | CJK-aware heuristic token count |
| estimate_messages | sum over messages + overhead |
| estimate_tool_def | name+desc+schema estimate |
| check_budget | Ok/Trim/Compact classification |

### z-core/src/repo.rs (v0)

| Function | Contract |
|---|---|
| RepoIndex::open | walk root, extract symbols, build index |
| map_text | bounded repo map text for system prompt |
| file_count / symbol_count | counters |

### z-core/src/provider.rs

| Function | Contract |
|---|---|
| from_config | kind → boxed provider; unknown rejected |
| OpenAIProvider::stream | SSE chat completions w/ tool-call accumulation |
| AnthropicProvider::stream | SSE messages w/ system extraction |
| StreamOutcome::push | internal accumulator append |

### UI crates

z-shell/z-tokens/z-gpui/z-app expose model transitions, token resolution,
scene composition, and wiring respectively — see crate docs; their function
inventories live in source and are not duplicated here to avoid rot.

---

## 80. Test Fixture Plan

| Fixture | Contents | Used by |
|---|---|---|
| fixtures/tiny-repo | 5 files, 2 languages, known symbol counts | index tests |
| fixtures/medium-repo | ~200 files synthetic Rust/TS/Python | search/index perf |
| fixtures/large-repo | ~5k files generated | scale benchmarks |
| fixtures/xl-repo | ~50k files generated on demand | very-large benchmarks |
| fixtures/journal-seq | scripted journal segments incl. corrupt tail | replay tests |
| fixtures/provider-sse | recorded SSE streams (openai/anthropic shapes) | provider replay tests |
| fixtures/layouts | shell layout JSON versions v1..vN | settings/layout migrations |

Generation scripts live in tools/; fixtures themselves are gitignored when
generated, committed when small and deterministic.

---

## 81. Keyboard Map Draft (default bindings)

| Scope | Key | Action |
|---|---|---|
| global | Ctrl/Cmd+K | command palette |
| global | Ctrl/Cmd+P | quick open file/thread |
| global | Ctrl+B | toggle sidebar |
| global | Ctrl+` | toggle terminal panel |
| global | Escape | cancel transient state |
| chat | Enter | send message |
| chat | Shift+Enter | newline |
| chat | Ctrl+C (streaming) | cancel turn |
| approvals | Y / N | approve / deny focused request |
| lists | ↑ ↓ Home End | navigate |
| tabs | Ctrl+W | close tab |
| editor | Ctrl+S | save (routes through safe editing) |
| editor | F12 / Shift+F12 | go-to-def / references |

All remappable (§13); conflicts resolved by scope specificity.

---

## 82. Error Code Registry (planned ZD-E-xxxx)

| Code | Meaning |
|---|---|
| ZD-E-0001 | no provider configured |
| ZD-E-0002 | provider auth failed (key rejected) |
| ZD-E-0003 | provider unreachable/network error |
| ZD-E-0004 | provider stream ended unexpectedly |
| ZD-E-0005 | malformed provider response |
| ZD-E-0010 | no project open |
| ZD-E-0011 | project path invalid/not a directory |
| ZD-E-0020 | path outside project scope |
| ZD-E-0021 | file too large to read directly |
| ZD-E-0022 | empty query/command argument |
| ZD-E-0030 | command timed out (tree killed) |
| ZD-E-0031 | spawn failed |
| ZD-E-0040 | approval denied by user |
| ZD-E-0041 | approval timed out |
| ZD-E-0050 | context overflow after trim+compact |
| ZD-E-0060 | stale edit refused (file changed since read) |
| ZD-E-0061 | patch anchor missing |
| ZD-E-0070 | journal corrupt tail truncated |
| ZD-E-0080 | extension capability denied |
| ZD-E-0090 | doom-loop breaker tripped |

Registry grows additively; codes are stable once assigned.

---

## 83. Data Migration Playbook

For each persisted format (threads, layouts, settings, journal):

1. Bump format version constant.
2. Write migrator fn: old_version_json → new_version_json (pure).
3. Reader accepts [current-1, current]; writer emits current.
4. Migration test: fixture files per historical version load correctly.
5. Unknown-version behavior: refuse with actionable error (never guess).
6. Deprecations logged at read time until removal window closes.

---

## 84. Decision Index by Domain

Quick lookup of which ADR governs which area:

| Domain | Governing decisions |
|---|---|
| Concurrency | ADR-0001 (threads/channels) |
| UI stack | ADR-0002 (ZeroGPUI) |
| Sandbox | ADR-0003 (suspended-spawn), security skill rules |
| Persistence | ADR-0004 (JSONL-first) |
| Product scope | ADR-0005 (personal-first) |
| Secrets | ADR-0006 (interim plaintext, keychain planned) |
| Protocol | additive-only policy (§37) |
| Dependencies | policy §52; per-dep ADRs as needed |

---

## 85. Documentation Maintenance Rules

1. Every doc names its owner role (Manager maintains spec/ledger/state;
   skills maintained alongside the code they govern).
2. Statuses use the §7 vocabulary; no invented labels.
3. Numbers that rot (test counts, LOC) live in DEVELOPMENT-STATE only.
4. Contradictions are resolved within the session that discovers them,
   or recorded as an explicit open question.
5. Docs are updated in the same commit as the change they describe when
   practical; otherwise the follow-up commit must reference the task ID.

---

## 86. Steering Queue — Detailed Design

### 86.1 State

```
Shared.steering: Mutex<HashMap<thread_id, VecDeque<String>>>
```

Commands: EnqueueMessage{thread_id, text} → push + emit SteeringQueued
{depth}. Queue is unbounded but depth surfaces in UI; >10 queued triggers
a gentle warning event (user decides).

### 86.2 Drain algorithm (in run_turn, between tool rounds)

```
drain_steering(thread_id):
  msgs = take all queued texts
  if empty → None
  combined = combine(msgs)
  return UserMessage(combined)
combine(msgs):
  if all plain text → join with "

" prefixed by "User steering:"
  else → keep as separate messages (rare; only when future typed queues exist)
```

### 86.3 Edge cases

| Case | Behavior |
|---|---|
| Enqueue while no turn running | becomes a normal SendMessage (runtime checks active turn) |
| Cancel while queued | queue cleared on turn end |
| Steering arrives during final round | applied next turn if turn already finishing (documented, not an error) |
| Duplicate rapid enqueues | combined into one message (token economy) |

Tests: mid-turn injection lands before next provider call; combine gate;
cancel clears queue; depth events correct.

---

## 87. Journal Replay Engine — Detailed Design

### 87.1 View builders

Each consumer registers a reducer: `fn(state, record) -> state`. Reducers
must be total (handle every kind), pure, and fast. Initial consumers:
threads view, task view, usage stats.

### 87.2 Replay procedure

1. List segments in order; read CURRENT pointer.
2. For each record: validate seq continuity (warn on gap); apply to each
   registered reducer.
3. On invalid line: stop; report last good seq; repair = truncate.
4. Emit ReplaySummary {records, gaps, duration} as a log event.

### 87.3 Determinism requirements

- No wall-clock reads inside reducers (timestamps are data, not behavior).
- No external I/O in reducers.
- HashMap iteration order never affects output (sort where order matters).

### 87.4 Tests

Fixture journal (§80) covering: clean replay, corrupt tail repair, seq gap
tolerance, deterministic double-replay equality (hash compare).

---

## 88. Tree-sitter Integration — Detailed Design

### 88.1 Grammar registry

```rust
struct LanguagePack {
    id: &'static str,            // "rust", "typescript", "python"
    parser: tree_sitter::Language,
    symbol_query: Query,          // captures definitions
    reference_query: Query,       // captures identifier usages
    import_query: Query,          // module/import edges
}
```

Feature-gated per language so builds stay lean; registry built at startup
from enabled packs.

### 88.2 Extraction pipeline

Parse file → walk query captures → produce:
- Symbol { id, kind (fn/class/method/struct/...), name, range, signature }
- Reference { symbol_name, range, context_kind }
- ImportEdge { from_file, to_module }

Symbol ids are stable within a file version: `file_hash:name:kind:index`.

### 88.3 Incremental update

On change: reparse file → new symbol set → diff vs old (by id) →
added/removed/changed symbols → update reverse-reference index entries that
pointed at changed symbols (re-resolve their files lazily on next query,
not eagerly — bounded work per edit).

### 88.4 Failure containment

Parser panics caught via catch_unwind per file (tree-sitter is C; a crash
must not kill the actor). Failed files marked errored with reason; retried
on grammar upgrade.

---

## 89. Safe-Editing Fingerprint System — Detailed Design

### 89.1 Fingerprint definition

`fingerprint = fnv1a64(file_bytes)` recorded at read time alongside path +
size. Stored per thread in memory and journaled on write attempts.

### 89.2 Write-time check

```
write(path, content, expected_fingerprint?):
  scoped(path)?
  if expected_fingerprint.is_some():
      current = hash(read bytes)
      if current != expected → Err(ZD-E-0060 stale)
  atomic_write(temp+rename)
  journal(tool_executed with old/new fingerprints)
```

fs_write without prior read: allowed but flagged in result ("no read
fingerprint — verify content"). edit_patch REQUIRES the fingerprint.

### 89.3 Rollback staging

Multi-file ops collect (path, old_bytes) before writing; on failure or
explicit rollback, restore originals atomically in reverse order. Staging
memory capped (large ops spill staged originals to temp dir).

---

## 90. Layout Region Model — Detailed Spec

### 90.1 Types

```rust
enum Region { Split { axis, ratio, children: Vec<Region> },
              Panel { id: PanelId } }
struct PanelState { visible, focused, size_hint }
struct Layout { root: Region, panels: HashMap<PanelId, PanelState>,
                version: u32 }
```

### 90.2 Operations (pure)

toggle_panel(id), focus(id), split(id, axis, ratio), close(id),
set_ratio(node, r), apply_preset(name). Each returns new Layout; shell
keeps undo stack (depth 50) for layout changes.

### 90.3 Persistence & migration

Serialized JSON with version; migrators per §83. Unknown panel ids on load
→ dropped with warning (panels may not exist in older layouts).

### 90.4 Constraints enforced at transition time

- Minimum panel size 120 px equivalent; splits rebalance to satisfy.
- At least one visible panel always (closing last focuses empty state).
- Ratio children sum ≈ 1.0 (normalized after edits).

---

## 91. Worked Task Examples (fully specified)

Examples of the specification depth expected for major ledger tasks.

### 91.1 Example: "Implement steering queue drain"

- **Task**: drain queued steering messages between tool rounds.
- **Depends on**: EnqueueMessage command; SteeringQueued event.
- **Implementation**: Shared.steering map; drain_steering() at top of each
  round after cancel check; combined text appended as user message.
- **Tests**: (1) enqueue during streaming → applied before round 2 request;
  (2) two rapid enqueues → single combined message; (3) cancel clears.
- **Evidence**: test names + request-capture showing injected text.
- **DoD**: all tests green; §86 edge cases covered; ledger note updated.

### 91.2 Example: "Atomic file writes"

- **Task**: fs_write uses temp+rename.
- **Depends on**: none.
- **Implementation**: write to `<path>.ztmp-<pid>` in same dir → fsync →
  rename over target. Same-volume rename is atomic on all platforms.
- **Tests**: (1) normal write round-trip; (2) injected failure between
  temp-write and rename → original intact; (3) concurrent reader sees old
  or new, never partial.
- **Evidence**: test output; before/after of crash-simulation script.
- **DoD**: fs_write + edit_patch both route through the atomic helper.

### 91.3 Example: "Doom-loop breaker"

- **Task**: abort turn after N identical failing tool calls.
- **Depends on**: StepFinished events carrying tool+args hash.
- **Implementation**: per-turn counter keyed by (tool, args_hash, ok=false);
  threshold 3 → abort with diagnostic summary listing the repeated call.
- **Tests**: scripted 5 identical failures → turn ends at 3 with breaker
  message; different args reset counter.
- **Evidence**: TurnFinished error text contains breaker diagnosis.
- **DoD**: threshold configurable via settings (dev mode).

---

## 92. Supervision Detection Rules — Detailed Design

| Detector | Trigger | Threshold | Response |
|---|---|---|---|
| fake-completion | success-claim regex ∧ no matching evidence record | any | verdict=needs-review |
| unexecuted-tests | "tests pass" ∧ no test-runner evidence | any | verdict=fail |
| unexecuted-build | "build succeeds" ∧ no build evidence | any | verdict=fail |
| ignored-failure | tool ok=false followed by success claim citing it | any | verdict=fail |
| doom-loop | identical (tool,args-hash,fail) repeats | 3 | abort turn w/ diagnosis |
| premature-stop | task checklist items unaddressed ∧ turn ended ok | any | verdict=needs-review |
| placeholder-code | diff contains TODO-only/stub bodies in claimed-complete files | any | flag in review surface |
| mock-in-prod | diff adds mock/fake/stub to non-test paths | any | flag |
| requirement-skew | delivered diff touches none of task's target paths | any | verdict=needs-review |

Claim regexes are conservative (explicit phrases); false positives land in
needs-review, never auto-fail. All verdicts are user-visible and
appealable (user override recorded in journal).

---

## 93. Model Router Policy Examples

```yaml
policies:
  - name: cheap-research
    when: { task_kind: research, budget: low }
    prefer: [tier: fast-cheap, tools: basic]
    fallback: [tier: mid]
  - name: strong-coding
    when: { task_kind: implementation }
    prefer: [tier: frontier-tools]
    fallback: [tier: mid-tools, note: "downgrade flagged to user"]
  - name: local-offline
    when: { connectivity: none }
    prefer: [backend: local]
    fallback: [error: "no local model installed"]
```

Router output: (provider_ref, model, reason) logged per decision. Hard
requirements (declared by task) bypass downgrade fallbacks.

---

## 94. PTY Layer — Detailed Design

### 94.1 Trait

```rust
trait Pty {
    fn spawn(&mut self, cmd: &str, cwd: &Path, size: (u16, u16)) -> Result<PtyId>;
    fn resize(&mut self, id: PtyId, size: (u16, u16)) -> Result<()>;
    fn write(&mut self, id: PtyId, bytes: &[u8]) -> Result<()>;
    fn kill(&mut self, id: PtyId) -> Result<()>;
    fn poll_output(&mut self, id: PtyId) -> Option<Vec<u8>>;
}
```

### 94.2 Backends

- Windows: ConPTY via CreatePseudoConsole; process in Job Object (reuse
  sandbox guard semantics for kill-on-close).
- Unix: openpty + fork/exec with setsid; process group kill on close.

### 94.3 Output handling

Reader thread per PTY → chunk queue → shell ring buffer (10 MB cap, oldest
evicted) → renderer parses VT sequences (vte-class parser) into styled
cells. Search operates on raw bytes (fidelity) with cell-level highlight
mapping.

### 94.4 Agent interactive sessions

Attach flow: approval → PTY spawn → agent writes gated per-line (user sees
pending input) → output streams to both panel and turn log. Detach leaves
process running under job guard; reattach supported.

---

## 95. Diff & Patch Pipeline — Detailed Design

### 95.1 Diff generation

Unified diff computed via histogram-style algorithm (patience fallback for
noisy files); context 3 lines default; binary files → "binary differs"
placeholder + size delta.

### 95.2 Patch application

edit_patch blocks: exact-match search (whitespace-normalized fallback with
warning); replace; multi-block sequential application with per-block
fingerprints; any failure aborts whole patch (no partial application).

### 95.3 Review flow

Agent-proposed patch → diff viewer renders → per-hunk accept/reject →
accepted hunks applied via safe-editing (fingerprint re-verified at apply
time) → result journaled with final diff.

### 95.4 Conflict handling

If target changed since proposal (fingerprint mismatch): recompute diff
against current content; hunks that no longer anchor are surfaced as
conflicts for manual resolution; never auto-merge.

---

## 96. Glossary Part 3 & Index

| Term | Definition |
|---|---|
| Ring buffer | fixed-capacity circular buffer (terminal scrollback) |
| Histogram diff | diff algorithm minimizing block moves |
| Patience diff | diff variant anchoring on unique lines |
| Reducer | pure (state, record) → state function for journal replay |
| Language pack | tree-sitter grammar + query set for one language |
| Write grant | exclusive permission for one agent to modify a file |
| Verdict | supervision outcome: pass / fail / needs-review |
| Claim linker | matches model claims to captured evidence |
| Admission control | scheduler refusing work that would break latency budgets |
| Segment | one journal JSONL file with checksum trailer |
| CURRENT | pointer file naming the active journal segment |
| Hunk | contiguous diff block |

Index of normative sections: §4 principles · §7 status vocabulary · §16
security · §17 recovery · §28 tools · §29 protocol · §31 milestones · §37
compatibility · §38 gates · §44 budgets · §58 context algorithm · §59
journal · §68 metrics · §75 settings · §76 tokens.

---

## 97. Evidence Records — Types & Capture Points

### 97.1 Record types

```rust
enum Evidence {
    Build { command: String, exit_code: i32, duration_ms: u64 },
    Tests { runner: String, passed: u32, failed: u32, output_ref: Id },
    Diff { path: String, before_fingerprint: String,
           after_fingerprint: String, diff_ref: Id },
    Benchmark { name: String, before: f64, after: f64, unit: String },
    RegressionTest { test_id: String, reproduces_bug: bool },
}
```

### 97.2 Capture points

| Point | Captured |
|---|---|
| sandbox::run completion | exit code + duration → Build/Tests when command matches known runners |
| fs_write / edit_patch success | before/after fingerprints + computed diff |
| benchmark harness run | structured numbers |
| regression test creation | test id linked to bug report/task |

### 97.3 Storage

Evidence records are journal events; large outputs stored as content-
addressed blobs with the record holding a reference. UI fetches on demand
(§29 rule: ids not blobs).

---

## 98. Worktree Manager — Detailed Design

### 98.1 Operations

create(task_id, base_branch) → registers worktree under data/worktrees/
<task_id>/; list(); merge(task_id) → safe-editing pipeline over diff;
cleanup(task_id) → remove dir + prune registration; orphan_scan() at
startup detects unregistered dirs.

### 98.2 Invariants

- Every worktree has an owning task id; no anonymous worktrees.
- Merge requires: tests green in worktree, fingerprint checks pass,
  parent approval for non-trivial diffs.
- Cleanup is guaranteed by orchestrator finally-blocks AND startup orphan
  scan (belt and suspenders).

### 98.3 Failure modes

Dirty worktree at cleanup → quarantine (rename with timestamp) instead of
deletion; user decides later. Merge conflict → conflict surface, never
auto-resolve.

---

## 99. Best-of-N Evaluator Scoring Rubric

Score components (weighted, configurable):

| Component | Weight | Measure |
|---|---|---|
| Correctness | 0.4 | tests pass in attempt's worktree |
| Completeness | 0.25 | task checklist coverage from supervision |
| Diff quality | 0.15 | size vs task complexity heuristic; no unrelated changes |
| Budget efficiency | 0.1 | tokens+time used vs cap |
| Risk posture | 0.1 | security checklist clean; no scope violations |

Ties broken by earlier completion. Loser attempts archived as artifacts
(learning data), never silently deleted.

---

## 100. Artifact Platform Storage Format

```
data/artifacts/
  blobs/<sha256>            content-addressed content
  index.jsonl               append-only manifest records
```

Record: `{ id, type, title, created_seq, provenance:{turn_id?,task_id?},
blob: sha256, mime, version_of?: id }`. Version chains via version_of.
Preview renderers registered per type (markdown, diagram, image, patch,
text). Export = copy blob + metadata to chosen location.

---

## 101. Settings Engine Internals

### 101.1 Schema representation

```rust
struct SettingDef { id, kind: SettingKind, default: Value, category,
                    mode: User|Developer, restart_required: bool,
                    description, constraints: Vec<Constraint> }
enum Constraint { Min(f64), Max(f64), OneOf(Vec<String>), Pattern(String) }
```

### 101.2 Runtime flow

Load file → migrate to current version → validate against schema (unknown
keys kept+warned; constraint violations reset to default+warned) → typed
accessor cache → SetSetting commands update + persist + emit change event.

### 101.3 Search index

Built from schema (id + description tokens); palette integration free.

---

## 102. Workspace Manifest Documentation

`z desktop/Cargo.toml` facts (verify against source):

- Members: z-protocol, z-core, z-shell, z-gpui, z-tokens, zero-app.
- Shared deps pinned at workspace level where versions must agree.
- Profiles per §46.4.
- Rule: dependency additions happen at workspace level first; crate-level
  only for genuinely local needs.

Crate dependency graph (must remain acyclic in this direction):

```
z-protocol ◀── z-core ◀────────────┐
z-shell   ◀── z-gpui ◀── z-app ────┘ (composition)
z-tokens  ◀── z-gpui, z-shell
```

---

## 103. Context Compaction Algorithm — Detailed Design

### 103.1 Trigger

check_budget returns Trim → trim_history first; Compact only when trimming
cannot fit even a single clean boundary's suffix.

### 103.2 Compaction procedure

1. Identify compactable span: completed tool rounds older than the last
   user message.
2. Extract pinned facts: file paths touched, decisions stated, errors hit,
   task requirements quoted verbatim.
3. Summarize each tool round to ≤ 40 tokens: tool, key result line, error
   if any.
4. Replace span with: "Earlier work summary:" + pinned facts + round
   summaries. Structural validity preserved (summary is a user-role block).
5. Journal the compaction event with before/after token counts.

### 103.3 Invariants

- Pinned facts are never paraphrased beyond meaning-preserving compression;
  exact paths/commands survive verbatim.
- Compaction is idempotent per span (re-compaction of already-compacted
  content is a no-op).
- The most recent user message is never compacted.

---

## 104. Memory Consolidation Pass — Detailed Design

### 104.1 When

After turn completion, batched (not per-turn synchronous); nightly full
pass over new journal records.

### 104.2 Procedure

1. Candidate extraction: recurring entities, repeated decisions, stable
   project facts from journaled turns.
2. Deduplication against existing memories by anchor + content similarity.
3. Confidence scoring: source count × recency decay × user-affirmation.
4. Write candidates as provisional; promote after N independent sources or
   explicit user confirmation.

### 104.3 Bounds

Consolidation runs under Batch priority class; caps: ≤ 100 candidates per
pass; ≤ 10 MB new memory writes per day.

---

## 105. Lexical Search (Trigram Index) — Detailed Design

### 105.1 Structure

Per-file trigram sets stored in an on-disk inverted index:
trigram → {file_id}. Query = intersect candidate files for all query
trigrams, then verify with substring scan (no false positives).

### 105.2 Update

File change → recompute its trigram set → diff vs old set → update postings
incrementally. Deletion removes postings.

### 105.3 Performance envelope

Medium fixture (~200 files): build < 2 s; query p95 < 200 ms (§55.2).
XL fixture: spill postings to disk in sorted runs; memory cap 256 MB.

### 105.4 Ranking

No relevance ranking at this layer (lexical = filter); ranking happens at
the retrieval layer above (recency + path proximity + symbol hits).

---

## 106. Repo-Map v2 Generator — Detailed Design

### 106.1 Inputs

Symbol graph, git recency signals, open-file set, current task text.

### 106.2 Selection algorithm

1. Score symbols: structural centrality (fan-in/out) × recency × task-term
   overlap.
2. Greedy pack top-scoring entries into token budget (default 4k):
   path:symbol(signature) lines, grouped by directory.
3. Always include: entry points, recently edited files' symbols, symbols
   named in the task text.

### 106.3 Stability requirement

Identical inputs → byte-identical map (test asserted). Map is part of the
stable prompt prefix; regeneration only when inputs actually change.

---

## 107. Final Appendix — Normative Section Index

| Topic | Section |
|---|---|
| Identity & philosophy | §1–§3 |
| Principles | §4 |
| Status vocabulary | §7 |
| Domain designs | §8, §49, §86–§106 |
| UI specs | §9–§14, §36, §50, §64, §81 |
| Security | §16, §43 |
| Recovery | §17, §39 |
| Persistence | §18, §30, §59, §83, §87 |
| Tools | §28, §73 |
| Protocol | §29, §37, §56 |
| Milestones | §31, §55, §72, §91 |
| Quality | §38, §42, §60 |
| Performance | §23, §44 |
| Metrics | §68 |
| Settings | §12, §75, §101 |
| Theming | §10, §76 |

---

## 108. Provider Retry & Backoff Policy — Detailed Design

### 108.1 Classification

| Failure class | Retry? | Policy |
|---|---|---|
| Network error / timeout | yes | 1 retry, 2 s backoff (round 0 only) |
| HTTP 429 rate limit | yes | honor Retry-After if present, else 5 s |
| HTTP 5xx | yes | 1 retry, 3 s backoff |
| HTTP 401/403 auth | no | fail with ZD-E-0002 (user action needed) |
| Malformed stream mid-way | no | fail with partial text preserved in error |

### 108.2 Rules

- Retries happen at the runtime layer, never inside providers.
- A retried request re-sends byte-identical payload (cache-friendly).
- Retry attempts are journaled (provider_called with attempt field).
- After final failure: TurnFinished(ok=false) with classified message;
  user message already persisted — nothing lost.

---

## 109. Usage Accounting — Detailed Design

### 109.1 Extraction

OpenAI: `usage` object on final chunk (stream_options include_usage).
Anthropic: `usage` fields on message_start/message_delta events.

### 109.2 Records

UsageReported { turn_id, tokens_in, tokens_out, cache_read?, cache_write? }
→ journal + rolling aggregates (per thread/day/model) for dashboard.

### 109.3 Calibration loop

estimator_accuracy = estimate(prompt) / tokens_in over rolling window;
constants adjusted when systematic drift > 15% persists across ≥ 50
requests. Adjustment is an explicit, reviewed change (not auto-tuning).

---

## 110. Keychain Integration — Detailed Design

### 110.1 Backends

Windows: DPAPI via windows-sys CryptProtectData (file-backed secret store,
no extra dependency). Later: generic keyring crate evaluation.

### 110.2 Store layout

data/secrets/<name>.dpapi — DPAPI-protected blobs, current-user scope.
config.json keeps non-secret fields only after migration; migration moves
key → secrets store on first run of new version (one-way, logged).

### 110.3 Invariants

- Secret blobs never appear in logs/journals/backups (scan-gated).
- Export functions exclude secrets by default; explicit opt-in required.
- Corrupt blob → clear error prompting re-entry (never silent fallback to
  plaintext).

---

## 111. Watcher Service — Detailed Design

### 111.1 Responsibilities

File-change notifications feeding: index actor, live preview, automation
triggers, editor external-change detection.

### 111.2 Discipline

- Debounce window 300 ms per path; coalesce bursts into one event batch.
- Ignore patterns shared with walk_files (dependency dirs).
- Watchers are refcounted: last consumer unsubscribes → OS watch removed.
- Event batches carry monotonic sequence numbers for consumer dedup.

### 111.3 Failure modes

Watch overflow (OS buffer full) → full rescan of affected subtree
(debounced); never silently drop changes. Watcher thread death → restart
with backoff + diagnostics event.

---

## 112. Glossary Part 4 & Closing Index

| Term | Definition |
|---|---|
| DPAPI | Windows Data Protection API (per-user secret encryption) |
| Retry-After | HTTP header guiding backoff timing |
| Refcounted watcher | file-watch removed when last subscriber leaves |
| Provisional memory | unconfirmed candidate awaiting promotion |
| Pinned fact | compaction-protected verbatim detail |
| Attempt | one provider call within a turn (attempt 0 = first) |
| Rolling aggregate | windowed statistic maintained incrementally |

Closing normative index: principles §4 · statuses §7 · domains §8/§49/
§86–§106 · UI §9–§14/§36/§50/§64/§81 · security §16/§43 · recovery §17/§39
· persistence §18/§30/§59/§83/§87 · tools §28/§73 · protocol §29/§37/§56
· milestones §31/§55/§72/§91 · quality §38/§42/§60 · performance §23/§44
· metrics §68 · settings §12/§75/§101 · theming §10/§76 · providers §108–§109
· secrets §110 · watchers §111.

---

## 113. Live Preview Server — Detailed Design

### 113.1 Architecture

Embedded static file server (tiny_http-class, std threads) bound to
127.0.0.1 with an ephemeral port; serves the project's preview root.
Change events from the watcher service trigger client reload via injected
EventSource script (no WebSocket dependency initially).

### 113.2 Rules

- Binds loopback only; never exposes to network without explicit setting.
- Serves only under the configured preview root (path traversal guarded).
- HMR bridge v1 = full reload; module-level HMR deferred until a real
  bundler integration exists.
- Preview state (scroll position) preserved across reloads where possible.

### 113.3 Agent self-review loop

`--shot` capture of the preview URL gives agents visual feedback for UI
work; screenshots stored as artifacts and attached as evidence.

---

## 114. Diagram Layout Engine — Detailed Design

### 114.1 Pipeline

Parse mermaid-class source → graph model (nodes/edges/subgraphs) → layered
layout (Sugiyama-style: cycle removal → layering → ordering → coordinate
assignment) → scene primitives.

### 114.2 Scope discipline

v1 supports: flowchart (TD/LR), sequence diagrams, class diagrams (basic).
Everything else renders as "unsupported construct" node rather than failing
silently — honest degradation.

### 114.3 Interaction contract

Pan/zoom transform is view state; selection maps node → symbol reference
when generated from repo intelligence; export at current zoom or fit-all.

---

## 115. Database Connection & Query Safety — Detailed Design

### 115.1 Connection management

Pool per connection profile (min 0, max 4); health check on acquire;
stale connections recycled; all credentials resolved from secret store at
connect time — never cached in memory beyond session.

### 115.2 Statement classification

Lexer-based classification before execution:
- SELECT/EXPLAIN/SHOW → read path (auto-allowed on read-only sessions)
- INSERT/UPDATE/DELETE → write path (approval per statement)
- CREATE/ALTER/DROP/TRUNCATE → DDL path (always approval + typed confirm)

### 115.3 Result handling

Row cursor streaming into virtualized grid; large cell values truncated
with fetch-on-demand; query cancel maps to driver cancel or connection
kill (documented per driver).

---

## 116. Glossary Part 5 & Final Index

| Term | Definition |
|---|---|
| ConPTY | Windows pseudo-console API |
| Sugiyama layout | layered DAG drawing algorithm family |
| EventSource | SSE-based browser reload channel |
| Loopback binding | server listening on 127.0.0.1 only |
| Statement class | read/write/DDL safety classification of SQL |
| Row cursor | streaming iterator over query results |

Final normative index: principles §4 · statuses §7 · domains §8/§49/
§86–§106/§113–§115 · UI §9–§14/§36/§50/§64/§81 · security §16/§43 ·
recovery §17/§39 · persistence §18/§30/§59/§83/§87 · tools §28/§73 ·
protocol §29/§37/§56 · milestones §31/§55/§72/§91 · quality §38/§42/§60 ·
performance §23/§44 · metrics §68 · settings §12/§75/§101 · theming §10/§76
· providers §108–§109 · secrets §110 · watchers §111 · preview §113 ·
diagrams §114 · databases §115.

---

## 117. Editor Buffer & Undo Model — Detailed Design

### 117.1 Buffer representation

Rope per open file (ropey-class); line index maintained incrementally;
fingerprint recomputed on transaction commit. Edits are transactions:
{range, inserted_text, inverse} — inverse enables undo without snapshots.

### 117.2 Undo semantics

- Grouping: consecutive single-char edits within 500 ms coalesce; explicit
  boundaries at cursor jumps, paste, and agent edits.
- Agent edits are their own undo group labeled "agent: <task>" — one
  Ctrl+Z reverts an entire agent change, not fragments.
- Undo depth 1000 per buffer; redo symmetric.

### 117.3 External change handling

Watcher event + fingerprint mismatch while buffer dirty → conflict banner
(buffer vs disk) with three actions: keep buffer / load disk / diff view.
Never auto-overwrite either side.

### 117.4 Save path

Ctrl+S routes through safe-editing (fingerprint check + atomic write);
save failure leaves buffer dirty with error surfaced — data never lost.

---

## 118. LSP Client — Detailed Design

### 118.1 Lifecycle

Server spawn per language per workspace root (config-driven); initialize
handshake with capability negotiation; requests multiplexed by id over
JSON-RPC stdio; shutdown on workspace close.

### 118.2 Scheduling

LSP requests run at Interactive priority; hover/completion results cached
per (position, version) to avoid redundant calls; server crash → restart
with backoff, max 3, then degraded mode (index-only navigation).

### 118.3 Integration points

- Completion: LSP primary, index fallback.
- Diagnostics: LSP publish → editor gutter + problems panel.
- Definition/references: index first (fast), LSP authoritative merge.

---

## 119. Notification Center — Detailed Design

### 119.1 Model

Notifications are shell state items: {id, severity (info/success/warn/
error), title, body, actions[], created_at, read}. Cap 50; oldest pruned.

### 119.2 Behavior

- Quiet by default: no sounds, no focus stealing; badge count only.
- Errors persist until dismissed; others auto-expire after 10 min.
- Actions are commands (e.g., "Retry turn", "Open diff") — never raw URLs.
- Approval requests are NOT notifications; they are inline blocking UI.

---

## 120. Glossary Part 6 & Final Index

| Term | Definition |
|---|---|
| Rope | efficient editable text sequence structure |
| Transaction | atomic buffer edit with inverse for undo |
| Undo group | coalesced set of edits reverted together |
| Capability negotiation | LSP initialize exchange of supported features |
| Degraded mode | reduced functionality after dependency failure |
| Publish diagnostics | LSP server-pushed error/warning updates |

Final normative index: principles §4 · statuses §7 · domains §8/§49/
§86–§106/§113–§119 · UI §9–§14/§36/§50/§64/§81 · security §16/§43 ·
recovery §17/§39 · persistence §18/§30/§59/§83/§87 · tools §28/§73 ·
protocol §29/§37/§56 · milestones §31/§55/§72/§91 · quality §38/§42/§60 ·
performance §23/§44 · metrics §68 · settings §12/§75/§101 · theming §10/§76
· providers §108–§109 · secrets §110 · watchers §111 · preview §113 ·
diagrams §114 · databases §115 · editor §117 · LSP §118 · notifications §119.

---

## 121. Command Palette — Detailed Design

### 121.1 Model

Palette = fuzzy matcher over a registry of actions: {id, title, keywords,
scope, handler}. Actions registered by every surface at composition time;
settings and keybindings are automatically indexed (§101.3).

### 121.2 Matching

Subsequence match with contiguous-run bonus; recency boost for last-used;
scope filter (current surface first). Results capped 20; Enter runs top,
arrow keys navigate, Tab previews (where previewable).

### 121.3 Extensibility

Extensions register actions through the same registry — no special path.
Unknown action ids from persisted state are dropped silently.

---

## 122. File Tree Panel — Detailed Design

### 122.1 Model

Lazy directory nodes: {path, expanded, children_loaded}; git status overlay
(clean/modified/untracked) fetched in batch per visible page. Root follows
OpenProject.

### 122.2 Behavior

- Type-ahead filter builds a flat filtered view (fuzzy on path segments).
- Keyboard: arrows navigate, Right/Left expand/collapse, Enter opens in
  editor, F2 rename (approval-gated write), Delete → confirm dialog.
- Large directories (>1000 entries) paginate with "load more" instead of
  rendering all.

### 122.3 Performance

Visible-page-only stat calls; status overlay computed incrementally from
watcher events rather than full `git status` per render.

---

## 123. Problems & Diagnostics Panel Integration

- Sources: LSP publishDiagnostics, build/test output parsers, supervision
  flags, sandbox failures.
- Unified item model: {source, severity, path?, range?, message, code?,
  action?}.
- Click navigates to location or evidence detail; filters by source and
  severity; counts shown as quiet badges (never alarm colors unless error).

---

## 124. Glossary Part 7 & Final Index

| Term | Definition |
|---|---|
| Action registry | central list of palette-invokable operations |
| Subsequence match | fuzzy match allowing gaps between characters |
| Lazy node | tree node whose children load on demand |
| Status overlay | VCS state decoration on file listings |
| Problems item | unified diagnostic entry from any source |
| Quiet badge | count indicator without alarm styling |

Final normative index: principles §4 · statuses §7 · domains §8/§49/
§86–§106/§113–§119 · UI §9–§14/§36/§50/§64/§81/§121–§123 · security §16/§43
· recovery §17/§39 · persistence §18/§30/§59/§83/§87 · tools §28/§73 ·
protocol §29/§37/§56 · milestones §31/§55/§72/§91 · quality §38/§42/§60 ·
performance §23/§44 · metrics §68 · settings §12/§75/§101 · theming §10/§76
· providers §108–§109 · secrets §110 · watchers §111 · preview §113 ·
diagrams §114 · databases §115 · editor §117 · LSP §118 · notifications §119.

---

## 125. Quick Open — Detailed Design

### 125.1 Model

Ctrl+P opens a fuzzy file/thread/symbol switcher over: indexed files
(path match), threads (title match), symbols (repo index, when available).
Result type badges distinguish the three.

### 125.2 Behavior

- Query empty → recent files/threads first.
- Path matching scores directory-boundary hits higher than mid-segment.
- Enter opens; Ctrl+Enter opens in split; Esc closes without side effects.

---

## 126. Search & Replace Across Files — Detailed Design

### 126.1 Model

Search panel: query (+ optional regex mode), include/exclude globs,
scope (whole project / current dir). Results grouped by file with per-file
expand and inline context lines.

### 126.2 Replace flow

Replace is a staged operation: preview diff per file → apply-all routes
through safe-editing (fingerprints verified at apply time) → failures
listed individually, never silently skipped. Undo = one journal-linked
group (§117.2 agent-group semantics apply to user bulk replaces too).

### 126.3 Performance

Backed by trigram index candidate narrowing (§105) once M2 lands; before
that, bounded parallel scan with cancellation on new queries.

---

## 127. Glossary Part 8 & Final Index

| Term | Definition |
|---|---|
| Quick open | fuzzy switcher for files/threads/symbols |
| Staged replace | preview-then-apply bulk edit flow |
| Glob filter | include/exclude path pattern |
| Candidate narrowing | index-driven reduction of files to scan |

Final normative index: principles §4 · statuses §7 · domains §8/§49/
§86–§106/§113–§119 · UI §9–§14/§36/§50/§64/§81/§121–§123/§125–§126 ·
security §16/§43 · recovery §17/§39 · persistence §18/§30/§59/§83/§87 ·
tools §28/§73 · protocol §29/§37/§56 · milestones §31/§55/§72/§91 ·
quality §38/§42/§60 · performance §23/§44 · metrics §68 · settings
§12/§75/§101 · theming §10/§76 · providers §108–§109 · secrets §110 ·
watchers §111 · preview §113 · diagrams §114 · databases §115 · editor
§117 · LSP §118 · notifications §119 · palette §121 · tree §122 · problems
§123 · quick-open §125 · search-replace §126.

---

## 128. Thread Management UI — Detailed Design

### 128.1 Model

Thread list panel: {id, title, last_activity, message_count, active?}.
Sorted by recency; search filters by title/content prefix.

### 128.2 Operations

New thread (Ctrl+N), open (Enter), rename (F2), delete (confirm dialog;
removes file + journal tombstone record), duplicate (copy with new id).
Active turn's thread cannot be deleted (command rejected with reason).

### 128.3 Restore behavior

On startup the most recent thread opens automatically; list reflects
corrupt-file skips (§17) so nothing silently disappears.

---

## 129. Export & Import — Detailed Design

### 129.1 Export

Settings → JSON file (secrets excluded by default, explicit opt-in for
key export with typed confirmation). Threads → single JSON bundle or
per-thread files. Layouts/themes → their native files. All exports carry
a format version header.

### 129.2 Import

Validates version ≤ current; migrates per §83; conflicts resolved
explicitly (skip/overwrite/rename) — never silent merge. Import of secrets
requires typed confirmation and is journaled as a security-relevant event.

---

## 130. Glossary Part 9 & Final Index

| Term | Definition |
|---|---|
| Tombstone | deletion marker record in append-only storage |
| Bundle | versioned multi-item export archive |
| Conflict banner | explicit buffer-vs-disk resolution UI |
| Format header | version metadata prepended to exports |

Final normative index: principles §4 · statuses §7 · domains §8/§49/
§86–§106/§113–§119 · UI §9–§14/§36/§50/§64/§81/§121–§123/§125–§126/§128 ·
security §16/§43 · recovery §17/§39 · persistence §18/§30/§59/§83/§87 ·
tools §28/§73 · protocol §29/§37/§56 · milestones §31/§55/§72/§91 ·
quality §38/§42/§60 · performance §23/§44 · metrics §68 · settings
§12/§75/§101 · theming §10/§76 · providers §108–§109 · secrets §110 ·
watchers §111 · preview §113 · diagrams §114 · databases §115 · editor
§117 · LSP §118 · notifications §119 · palette §121 · tree §122 · problems
§123 · quick-open §125 · search-replace §126 · threads §128 · export §129.

---

## 131. Workspace Profiles & Per-Project State — Detailed Design

### 131.1 Model

Per-project state lives in `<project>/.zdesktop/` (gitignored by default,
committable if a team later wants shared config): layout profile, recent
files, per-project settings overrides, index cache pointer.

### 131.2 Precedence

Per-project override > global setting > schema default. Overrides are
explicitly listed (not wholesale copies) so global changes still apply.

### 131.3 Rules

- .zdesktop/ contents are versioned-format files with migrations.
- Deleting .zdesktop/ resets project-local state; never touches user data.
- Opening a project without .zdesktop/ creates it lazily on first write.

---

## 132. Update Check Flow — Detailed Design

### 132.1 Flow

Manual action "Check for updates" → fetch latest release metadata from
GitHub API → compare versions → show release notes + download link.
No background checks, no auto-download, no telemetry.

### 132.2 Rules

- Network failure → quiet message, no retry loop.
- Version comparison is semver-aware; pre-releases only shown when the
  current build is itself a pre-release.
- The check records nothing beyond the session.

---

## 133. Glossary Part 10 & Final Index

| Term | Definition |
|---|---|
| Workspace profile | per-project UI/state configuration set |
| Override precedence | project > global > default resolution order |
| Lazy creation | directory/file created on first write need |
| Semver-aware compare | pre-release ordering rules applied to versions |

Final normative index: principles §4 · statuses §7 · domains §8/§49/
§86–§106/§113–§119 · UI §9–§14/§36/§50/§64/§81/§121–§123/§125–§126/§128 ·
security §16/§43 · recovery §17/§39 · persistence §18/§30/§59/§83/§87 ·
tools §28/§73 · protocol §29/§37/§56 · milestones §31/§55/§72/§91 ·
quality §38/§42/§60 · performance §23/§44 · metrics §68 · settings
§12/§75/§101 · theming §10/§76 · providers §108–§109 · secrets §110 ·
watchers §111 · preview §113 · diagrams §114 · databases §115 · editor
§117 · LSP §118 · notifications §119 · palette §121 · tree §122 · problems
§123 · quick-open §125 · search-replace §126 · threads §128 · export §129 ·
profiles §131 · updates §132.

---

## 134. Agent Identity Prompt — Canonical Text

The system prompt's behavioral core (kept short, testable, honest):

```
You are Z, an engineering agent working inside Z Desktop on the user's
project.

Rules:
1. Be precise and honest. Never claim work you did not do.
2. Prefer reading before writing. Verify assumptions with tools.
3. Keep responses tight. No filler, no restating the task.
4. When a command fails, report the real error — never invent success.
5. Respect scope: operate only within the project directory.
6. Large outputs: summarize with exact pointers, don't truncate silently.
7. Finish turns with either completed work or a clear blocker.
```

Changes to this text are protocol-adjacent (stable prefix) — batch them,
test them against fixture conversations, and record in the ledger.

---

## 135. Tool Result Formatting Standards

How results are rendered back to the model (and mirrored to UI):

| Tool | Format |
|---|---|
| fs_read | raw file content verbatim |
| fs_list | `kind name (size)` lines, sorted |
| fs_search | `path:line: text` per hit, forward slashes |
| fs_write | confirmation + byte count |
| terminal_exec | stdout block, `[stderr]` block if any, `[killed: ...]` if timed out, `[exit code: N]` always |

Rules: deterministic ordering; no timestamps in results (nondeterminism
breaks cache); errors are plain actionable sentences; all output passes
redact() then bound().

---

## 136. Glossary Part 11 & Final Index

| Term | Definition |
|---|---|
| Behavioral core | model-independent rules section of the system prompt |
| Result format | canonical rendering of tool output for the model |
| Deterministic ordering | stable sort independent of filesystem timing |
| Cache-safe formatting | output style that avoids nondeterministic fields |

Final normative index: principles §4 · statuses §7 · domains §8/§49/
§86–§106/§113–§119 · UI §9–§14/§36/§50/§64/§81/§121–§123/§125–§126/§128 ·
security §16/§43 · recovery §17/§39 · persistence §18/§30/§59/§83/§87 ·
tools §28/§73/§135 · protocol §29/§37/§56 · milestones §31/§55/§72/§91 ·
quality §38/§42/§60 · performance §23/§44 · metrics §68 · settings
§12/§75/§101 · theming §10/§76 · providers §108–§109 · secrets §110 ·
watchers §111 · preview §113 · diagrams §114 · databases §115 · editor
§117 · LSP §118 · notifications §119 · palette §121 · tree §122 · problems
§123 · quick-open §125 · search-replace §126 · threads §128 · export §129 ·
profiles §131 · updates §132 · prompts §65/§134.

---

## 137. In-App Skill Loading — Detailed Design

### 137.1 Model

Skills live as directories under a configurable skills root (default:
`<project>/.zdesktop/skills/`). The runtime scans frontmatter at startup
and exposes a `skills_list` read-only tool plus per-skill content loading
via `skills_read(name)`.

### 137.2 Rules

- Skill content enters context on demand (token economy), never wholesale.
- Frontmatter parse failure → skill listed as errored with reason, not
  silently skipped.
- Skills are data (markdown), never executable — no code execution from
  skill files, ever.

---

## 138. Glossary Part 12 & Final Index

| Term | Definition |
|---|---|
| Skills root | configured directory scanned for SKILL.md files |
| On-demand loading | skill content fetched into context only when used |
| Errored skill | frontmatter-invalid skill surfaced with reason |
| Data-only extension | non-executable knowledge artifact |

Final normative index: principles §4 · statuses §7 · domains §8/§49/
§86–§106/§113–§119 · UI §9–§14/§36/§50/§64/§81/§121–§123/§125–§126/§128 ·
security §16/§43 · recovery §17/§39 · persistence §18/§30/§59/§83/§87 ·
tools §28/§73/§135 · protocol §29/§37/§56 · milestones §31/§55/§72/§91 ·
quality §38/§42/§60 · performance §23/§44 · metrics §68 · settings
§12/§75/§101 · theming §10/§76 · providers §108–§109 · secrets §110 ·
watchers §111 · preview §113 · diagrams §114 · databases §115 · editor
§117 · LSP §118 · notifications §119 · palette §121 · tree §122 · problems
§123 · quick-open §125 · search-replace §126 · threads §128 · export §129 ·
profiles §131 · updates §132 · prompts §65/§134 · skills §137.

---

## 139. In-App Skill Loading — Detailed Design

### 139.1 Model

Skills live as directories under a configurable skills root (default:
`<project>/.zdesktop/skills/`). The runtime scans frontmatter at startup
and exposes a `skills_list` read-only tool plus per-skill content loading
via `skills_read(name)`.

### 139.2 Rules

- Skill content enters context on demand (token economy), never wholesale.
- Frontmatter parse failure → skill listed as errored with reason, not
  silently skipped.
- Skills are data (markdown), never executable — no code execution from
  skill files, ever.

---

## 140. Glossary Part 13 & Final Index

| Term | Definition |
|---|---|
| Skills root | configured directory scanned for SKILL.md files |
| On-demand loading | skill content fetched into context only when used |
| Errored skill | frontmatter-invalid skill surfaced with reason |
| Data-only extension | non-executable knowledge artifact |

Final normative index: principles §4 · statuses §7 · domains §8/§49/
§86–§106/§113–§119 · UI §9–§14/§36/§50/§64/§81/§121–§123/§125–§126/§128 ·
security §16/§43 · recovery §17/§39 · persistence §18/§30/§59/§83/§87 ·
tools §28/§73/§135 · protocol §29/§37/§56 · milestones §31/§55/§72/§91 ·
quality §38/§42/§60 · performance §23/§44 · metrics §68 · settings
§12/§75/§101 · theming §10/§76 · providers §108–§109 · secrets §110 ·
watchers §111 · preview §113 · diagrams §114 · databases §115 · editor
§117 · LSP §118 · notifications §119 · palette §121 · tree §122 · problems
§123 · quick-open §125 · search-replace §126 · threads §128 · export §129 ·
profiles §131 · updates §132 · prompts §65/§134 · skills §137.

---

## 141. In-App Skill Loading — Detailed Design

### 141.1 Model

Skills live as directories under a configurable skills root (default:
`<project>/.zdesktop/skills/`). The runtime scans frontmatter at startup
and exposes a `skills_list` read-only tool plus per-skill content loading
via `skills_read(name)`.

### 141.2 Rules

- Skill content enters context on demand (token economy), never wholesale.
- Frontmatter parse failure → skill listed as errored with reason, not
  silently skipped.
- Skills are data (markdown), never executable — no code execution from
  skill files, ever.

---

## 142. Glossary Part 14 & Final Index

| Term | Definition |
|---|---|
| Skills root | configured directory scanned for SKILL.md files |
| On-demand loading | skill content fetched into context only when used |
| Errored skill | frontmatter-invalid skill surfaced with reason |
| Data-only extension | non-executable knowledge artifact |

Final normative index: principles §4 · statuses §7 · domains §8/§49/
§86–§106/§113–§119 · UI §9–§14/§36/§50/§64/§81/§121–§123/§125–§126/§128 ·
security §16/§43 · recovery §17/§39 · persistence §18/§30/§59/§83/§87 ·
tools §28/§73/§135 · protocol §29/§37/§56 · milestones §31/§55/§72/§91 ·
quality §38/§42/§60 · performance §23/§44 · metrics §68 · settings
§12/§75/§101 · theming §10/§76 · providers §108–§109 · secrets §110 ·
watchers §111 · preview §113 · diagrams §114 · databases §115 · editor
§117 · LSP §118 · notifications §119 · palette §121 · tree §122 · problems
§123 · quick-open §125 · search-replace §126 · threads §128 · export §129 ·
profiles §131 · updates §132 · prompts §65/§134 · skills §137.

---

*End of Master Specification. Maintain via the reconciliation rule in the
header; record material changes in DEVELOPMENT-STATE.*
