# Reference Implementation Map

Per-subsystem record of what was studied in reference repositories, what was
learned, and what Z Desktop decided. Evidence-based: each entry names the
actual files/modules studied. Companion to `docs/THIRD_PARTY_RESEARCH_AND_REUSE.md`
(license/attribution ledger) and `docs/research/grok-build-dissection.md`
(full dissection notes).

Decisions use the vocabulary: INSPIRE / ADAPT / REIMPLEMENT / REUSE / REJECT.

---

## grok-build (xai-org/grok-build)

- Clone: `references/external/grok-build` (gitignored), HEAD `07b2f71`
  (2026-08-23), Apache-2.0 first-party.
- Full dissection: `docs/research/grok-build-dissection.md` (7 subsystems).

| # | Subsystem | Files studied | Lesson | Decision | Z Desktop module |
|---|-----------|---------------|--------|----------|------------------|
| 1 | Tool abstraction | `xai-tool-runtime`, `xai-tool-protocol`, `xai-tool-types` | typed args via schema derive; Progress streaming channel; `should_list(ctx)` per-turn manifest filtering (token economy by design) | ADAPT (sync, no async) — should_list idea feeds tok-021 lazy manifests | `z-core/src/tools.rs` |
| 2 | Repo intelligence | `xai-codebase-graph` | tree-sitter queries per lang, rayon initial index, mmap IO, IndexManager actor with file-event input, on-disk cache + workspace lock | INSPIRE — our ADR-0007/0009 mirror this shape (tree-sitter landed idx-004; actor ADR accepted) | `z-core/src/symbols.rs`, `repo.rs`, ADR-0009 |
| 3 | Compaction | `xai-grok-compaction` | compaction as explicit engine with budget targets and summary artifacts rather than ad-hoc truncation | INSPIRE — our ctx-003 enforce_budget is the seed; full summarizing compaction remains future work | `z-core/src/context.rs`, runtime budget gate |
| 4 | Steering/interjection | `xai-interjection-core`, `xai-prompt-queue` | capped event-queue buffer; "user sent a message while you were working" framing; drain at safe boundaries | ADAPT conceptually — implemented as our steering queue slice (core-020..008) | runtime steering queue |
| 5 | Memory | `xai-grok-memory` | memory as derived view over an append-only stream with consolidation passes | VALIDATES ADR-0014 — we arrived independently at journal-derived views | `z-core/src/memory.rs` |
| 6 | Workspace/worktrees | `xai-grok-workspace*`, `xai-fast-worktree`, `xai-gix-status`, `xai-hunk-tracker` | gix for read-only status avoids git2 CVE surface; hunk tracking enables partial-stage UX | INSPIRE — ADR-0008 chose shell-out git CLI now, gix named as successor trigger | ADR-0008, future edit-* tasks |
| 7 | Safety & ops | safety crates set | capability-style tool gating with per-call approval decisions | VALIDATES our write-grant design (Risk::Write grants before mutation) | runtime write grants |

Notable meta-fact: grok-build itself contains in-tree ports of codex/opencode
tool implementations under Apache terms — validating our own dissect-adapt-
attribute posture.

## hermes-agent (NousResearch)

- Not cloned into references/external: it IS the host agent platform running
  this development loop; its architecture (skills, delegation, cron,
  provider pool) is observed daily from the inside.
- Lessons feeding Z Desktop: sub-agent fan-out pattern with strict file
  ownership (our dispatch model), credential pooling with rotation
  (provider.rs config), skills-as-procedural-memory (future ext-* tasks).
- Decision: INSPIRE (architecture-level).

## codex (openai/codex)

- Not yet cloned. Needed when work reaches sandbox hardening (term-004
  kill-on-close), approval flows, or patch/edit reliability (edit-019+ diff
  generation). Priority reference per mission brief §15.
- Action queued: clone blob-filtered when the next sandbox or approvals
  slice starts.

## zed (zed-industries/zed)

- Not yet cloned. Required before any serious GPUI/UI-shell maturation work
  beyond the current seam (ADR-0019). Our UI is currently structural;
  Zed study targets pane system, command palette, text performance.
- Action queued: clone when ui-100+ (palette, panes) begins.

## vscode, cline, openhands, aider, gemini-cli, opencode, bevy, godot

- Not cloned. Each maps to later phases (extensions, agent UX, isolation,
  repo-map context, tools/MCP, 3D). Per §7 of the mission: clone only when
  needed for current work — no speculative clones.

## deepseek-harness

- Shallow clone exists at workspace root (`deepseek-harness/`), license
  review pending. Relevant to future UI color/hierarchy mining (§27).
- Action queued: license check before any visual-language study.

---

## Maintenance rule

Every new subsystem study appends a row here BEFORE implementation starts
referencing it. Every real code reuse also lands in
THIRD_PARTY_RESEARCH_AND_REUSE.md with SHA + license + attribution.
