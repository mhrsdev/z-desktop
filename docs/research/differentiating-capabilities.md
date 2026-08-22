# Z Desktop — Differentiating Capabilities (v1)

≥150 capabilities extracted from competitive research + real developer pain
points. Each has: problem → approach. Priorities: **C**=Core, **H**=High Value,
**A**=Advanced, **E**=Experimental, **L**=Long-term. This list drives the
implementation map; nothing here is filler — every item must earn its line.

## Agent Kernel (1–18)

1. **C** Evidence-gated completion — "done" claims require runner exit codes,
   build artifacts, or visual proof; runtime refuses unevidenced completion.
2. **C** Tool-pair-safe compaction — history shrinks without orphaning tool calls.
3. **C** Steering queue with combine gates — user input mid-run merges or queues
   between tool rounds, never mid-call.
4. **C** Durable task journal (SQLite event sourcing) — every turn/tool/event is
   replayable after crash.
5. **C** Checkpoints + rewind — restore conversation AND workspace state to any
   checkpoint boundary.
6. **C** Cancellation semantics per layer — Escape cancels the turn, not the app;
   tool-level timeouts with partial-result capture.
7. **C** Doom-loop detector — repeated identical failing tool calls trigger
   escalation to the user instead of burning tokens.
8. **C** Honest progress projection — UI progress derives from step states only;
   impossible for UI to claim work the runtime didn't do.
9. **H** Plan mutation protocol — agent may revise its plan mid-task; revisions
   are versioned events, visible in Trace Viewer.
10. **H** Self-validation passes — before completion, agent re-reads changed
    files and re-runs targeted tests; results attached as evidence.
11. **H** Task decomposition with dependency graph — subtasks carry explicit
    deps; parallelizable leaves run concurrently.
12. **H** Long-horizon mode — budgeted multi-hour tasks with periodic
    checkpoints, context compaction, and resumable state.
13. **A** Interrupt-with-context — user interjections inject as system-reminder
    frames, not chat noise.
14. **A** Retry taxonomy — provider vs tool vs model errors get different retry
    policies (circuit breaker for providers).
15. **A** Idle-time useful work — while waiting on network/model: incremental
    indexing, cache warmup, validation (never fake work).
16. **A** Predictive context prefetch — precompute likely-next context locally,
    zero extra provider tokens.
17. **E** Speculative tool dry-runs — read-only preview of a write's effect
    shown before approval.
18. **E** Agent self-review gate — second cheap-model pass critiques diffs
    before they reach the user.

## Multi-Agent (19–30)

19. **H** Sub-agent spawn policy — default anti-spam: one specialist at a time
    unless task graph proves parallelism helps.
20. **H** Worktree isolation per sub-agent — parallel agents never share a dirty tree.
21. **H** Result merging with conflict detection — overlapping file edits are
    surfaced, never silently last-write-wins.
22. **H** Agent supervision — parent agent reviews child evidence before merge.
23. **A** Specialist registry — debugger/tester/reviewer personas with scoped tools.
24. **A** Transient vs persistent agents — ephemeral workers die with their task;
    persistent ones keep memory.
25. **A** Best-of-N with evaluator — N approaches in isolated worktrees, one judge.
26. **A** Shared blackboard — typed facts (not transcript) shared across agents.
27. **E** Agent-vs-agent code review duels on critical paths.
28. **E** Human-in-the-loop delegation — user assigns a subtask to a specific persona.
29. **L** Agent teams with budgets — token/time caps per team enforced by runtime.
30. **L** Cross-project agent reuse — a tuned debugging agent reusable across repos.

## Quality Control (31–40)

31. **C** Fake-test detector — tests that cannot fail are flagged in review.
32. **C** Hallucinated-file guard — edits to files that don't exist without an
    explicit create intent are rejected.
33. **C** Unverified-claim tagging — model statements about build/test status
    link to the actual evidence artifact.
34. **H** Regression dataset from failures — every real failure becomes a replayable eval case.
35. **H** Failure root-cause classifier — model/tool/context/retrieval/orchestration buckets drive fixes.
36. **H** TODO-completion audit — "done" with remaining TODOs in touched files fails the gate.
37. **A** Diff quality score — heuristic + model review of every diff before presentation.
38. **A** Test-coverage delta report per change.
39. **E** Property-based fuzzing suggestions for new parsers/handlers.
40. **E** Benchmark-before/after requirement for perf claims.

## Repository Intelligence (41–52)

41. **C** Tree-sitter symbol index (defs/refs) behind an actor — replaces regex heuristics.
42. **C** Incremental reindex from FS events — no full rescans.
43. **C** Binary/generated-code detection — excluded from maps and search noise.
44. **C** Bounded repo map grouped by directory — already implemented; keep budgeted.
45. **H** Import/dependency graph — "what breaks if I change X".
46. **H** Test-relation mapping — which tests cover this file/symbol.
47. **H** Change intelligence — edit → affected symbols/tests/caches/memory invalidation set.
48. **A** Call-graph queries via tools exposed to the agent.
49. **A** Ownership hints from git blame density.
50. **A** Large-repo mode — partitioned lazy indexing for monorepos.
51. **E** Semantic search over embeddings alongside lexical.
52. **E** Architecture-drift detection — module dependency rules enforced against reality.

## Editing & Files (53–62)

53. **C** Hash-guarded writes — stale-file detection prevents clobbering user edits.
54. **C** Atomic writes with rollback journal.
55. **C** Patch preview before apply — unified diff in approval card.
56. **H** AST-aware structured edits for supported languages.
57. **H** Multi-edit transactions — all-or-nothing across files.
58. **H** Undo beyond text — agent operations reversible from a command log.
59. **A** Conflict detection when user edits during agent run (hunk-level).
60. **A** Format-on-write respecting project formatters.
61. **E** Semantic rename through the index (project-wide).
62. **E** Edit-intent recording — why each hunk was made, stored as provenance.

## Context & Token Economy (63–74)

63. **C** Local token estimation before send — no surprises at the API.
64. **C** Context categories with lifecycle/priority/freshness (system/project/task/agent/tool/repo/memory/conversation/temp/evidence/prefs).
65. **C** Stable deterministic prefixes — maximize provider prompt-cache hits.
66. **C** Tool-output bounding with pagination + retrieval instead of dumping.
67. **H** Context deduplication — identical blocks sent once per request.
68. **H** Dynamic context budgeting per model window.
69. **H** Structured summaries as navigation, source-of-truth on demand ("accuracy-first caching").
70. **A** Model-specific context strategies (per-provider serialization quirks).
71. **A** Context Inspector panel — see exactly what entered the request.
72. **A** Token Inspector — input/output/cached/cost per turn.
73. **E** Semantic compression of repetitive logs.
74. **E** Cross-tool context transfer without LLM round-trips.

## Memory (75–82)

75. **H** Layered memory (working/session/project/semantic) with provenance + confidence + TTL.
76. **H** Decision memory — architectural decisions recorded and retrievable.
77. **H** Failure memory — what broke before, surfaced when similar context recurs.
78. **A** Consolidation pass during idle time ("dream" mode, locked).
79. **A** Superseding/conflict detection between memories.
80. **A** Memory Inspector — view/edit/pin/invalidate.
81. **E** User preference learning with explicit confirmation.
82. **E** Forgetting curve — low-value memories decay unless pinned.

## Models & Providers (83–94)

83. **C** Multi-provider BYOK (OpenAI/Anthropic/Google/xAI/Z.ai/DeepSeek/OpenRouter/OAI-compatible/custom/local).
84. **C** Provider circuit breaker + principled retries.
85. **C** Streaming SSE robustness (already tested; keep hardening).
86. **H** Model capability registry (coding/vision/context/cost/latency/reliability).
87. **H** Adaptive routing — explainable, overridable; fast/deep/vision/local roles configurable.
88. **H** Dedicated compaction model support.
89. **A** Local inference first-class (llama.cpp backend abstraction).
90. **A** Hardware intelligence — CPU/RAM/GPU/VRAM detection drives defaults.
91. **A** GPU pressure handling — inference throttling protects UI frame rate.
92. **E** Speculative decoding / KV-cache awareness where backends allow.
93. **E** Model A/B shadowing — same prompt to two models, compare offline.
94. **L** On-device embedding models for local semantic memory.

## Desktop Platform (95–110)

95. **C** Native GPU rendering with damage tracking (done in z-gpui — maintain).
96. **C** Virtualized everything — chat, lists, terminal output, diffs (done; extend).
97. **C** Full keyboard navigation + visible focus (done; keep as invariant).
98. **C** Accessibility tree as first-class citizen (done; platform actions wired).
99. **C** RTL/bidi correctness incl. isolated code/path runs (done; extend to all surfaces).
100. **H** Layout presets + per-project profiles + docking/resizing.
101. **H** Command palette covering every action.
102. **H** Minimal Mode (chat-only) ↔ Power Mode (dense diagnostics).
103. **H** Startup under budget — staged/lazy subsystem init, idle CPU ≈ 0.
104. **A** Theme engine with token system + WCAG contrast guardrails + custom accents.
105. **A** Shortcut engine — everything rebindable.
106. **A** Drag & drop between panels where meaningful.
107. **A** Crash recovery — sessions/layout/unsaved state restored safely.
108. **E** Glass/transient materials restricted to nav/overlays (restraint rule).
109. **E** Per-monitor DPI perfection + multi-window.
110. **L** Plugin-provided panels/tabs/commands via versioned Extension API.

## IDE & Code Surfaces (111–120)

111. **H** Editor surface with syntax highlighting (syntect-class), tabs, splits.
112. **H** Diagnostics integration (LSP client architecture ready).
113. **H** Go-to-definition/references powered by our own index (#41).
114. **H** Search Everywhere — files/symbols/commands/settings/chats/tasks/memory.
115. **A** Outline/breadcrumbs per buffer.
116. **A** Inline diff review with hunk accept/reject.
117. **A** Breakpoint-style debug adapter abstraction (DAP).
118. **E** Minimap (optional, off by default).
119. **E** Multi-cursor editing.
120. **L** Refactoring toolbox built on the index.

## Terminal & Execution (121–128)

121. **C** Real PTY terminal sessions with tabs/splits (alacritty-terminal-class).
122. **C** Command risk classification + permission rules (no fatigue for safe reads).
123. **C** Job Object sandboxing + timeouts on agent-spawned processes (Windows-first).
124. **H** Large-output virtualization + searchable scrollback.
125. **H** Process tree + exit-state visibility.
126. **A** Session restore across restarts.
127. **A** Task-associated terminals (a terminal belongs to a task/journal entry).
128. **E** Shell detection + smart completions.

## Git & Parallel Work (129–136)

129. **H** Git status/staging/diff/blame via libgit2-or-gix class library.
130. **H** Worktree management UI for parallel agents (#20/#25).
131. **H** Agent-changes vs user-changes attribution in diffs.
132. **A** Partial staging interaction.
133. **A** Cherry-pick/revert flows with safety prompts.
134. **A** Commit message generation grounded in the actual diff.
135. **E** Merge-conflict resolution assistant with three-way view.
136. **L** History time-travel browsing tied to checkpoints.

## Research/Browser/Data (137–146)

137. **H** Built-in browser with tabs/history/bookmarks + agent navigation.
138. **H** Page snapshot → agent-readable markdown (reader mode).
139. **H** Research Engine — multi-source search, ranking, contradiction flags, citation storage.
140. **A** DOM inspection bridge for Live UI Development (#141).
141. **A** Live Preview panel with element→source mapping.
142. **A** Visual regression checks on UI changes.
143. **E** Network/console views where webview exposes them.
144. **E** Data workspace (CSV/JSON/Parquet tables + charts).
145. **E** Database workspace (SQLite first) with schema/explain views.
146. **L** API Workbench (REST/GraphQL/SSE collections).

## Extensibility & Ops (147–160)

147. **C** Event bus with typed events and clear ownership (exists; enforce discipline).
148. **H** Observability center — agent events, tokens, latency, cache hits, timings.
149. **H** Trace Viewer — decisions + tool evidence without private CoT storage.
150. **H** Diagnostics Center — CPU/RAM/GPU/disk/provider/plugin/cache health.
151. **H** Performance benchmarks suite + regression gates in CI.
152. **H** Feature flags for experimental capabilities.
153. **A** Hooks system (before/after tool/task/edit/command/commit/response).
154. **A** MCP client with permission/health/timeout/isolation per server.
155. **A** Settings dual-mode (User simple / Developer deep) with progressive disclosure.
156. **A** Import/export of settings/themes/keybindings/workflows.
157. **E** User-created tools via agent scaffolding into plugin skeleton.
158. **E** Self-extending developer mode (agent builds extensions, core untouched).
159. **L** Versioned Extension API + SDK + example extensions.
160. **L** Update channels (stable/beta/nightly) with secure updates.

## Personal Knowledge & Automation (161–170)

161. **H** Project knowledge base — notes/decisions/references beside the repo.
162. **H** Automation triggers — file/git/test/command events fire workflows.
163. **A** Workflow builder — Trigger→Agent→Tool→Condition→Action, no heavy hardcode.
164. **A** Scheduled agent runs (nightly dependency check, flaky-test hunt).
165. **A** Proactive alerts — broken tests after edit, stale deps, error spikes.
166. **E** Artifact generation (docs/reports/diagrams) as editable sources.
167. **E** Diagram engine — flowchart/architecture/sequence as editable graphs.
168. **E** Personal scratchpad with agent access.
169. **L** Cross-project knowledge sync (opt-in).
170. **L** Voice input for steering (accessibility + convenience).

## Count & coverage

170 items ≥ 150 required. Priority mix: ~40 Core, ~55 High, ~45 Advanced,
~25 Experimental, ~15 Long-term. Each maps to at least one mission section and
one competitor observation or novel gap. The capability matrix
(`capability-matrix.md`) tracks implementation status per item family.