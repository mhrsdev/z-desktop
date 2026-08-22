# Competitive Capability Matrix

Living document. Update whenever a subsystem is researched or a new competitor
signal appears. Status values: `Implement` / `Future` / `Reject`.

Legend for relevance: ★★★ core to Z Desktop · ★★ high value · ★ niche.

## Products tracked

| Product | Type | Source availability | Notes |
|---|---|---|---|
| Grok Build | Terminal agent (TUI) | Open source (Apache-2.0), ~90 Rust crates | Dissected in `grok-build-dissection.md` |
| Codex (OpenAI) | Terminal/cloud agent | Partially open (ported inside grok-build tree) | Tool designs portable under Apache |
| Claude Code | Terminal agent | Closed; skills/plugins documented publicly | Skills + hooks model is strong |
| OpenCode | Terminal agent | Open source | Ported into grok-build tree |
| OpenHands | Autonomous dev agent | Open source | Event-sourced runtime, browser use |
| Cline / Roo Code | IDE extension agents | Open source | Approval UX, plan/act modes |
| Continue | IDE assistant | Open source | Config-driven providers |
| Aider | CLI pair programmer | Open source | Repo-map, edit formats, git discipline |
| Zed | Editor | Open source | Native perf bar, collab later |
| Cursor / Windsurf | AI IDEs | Closed | Tab-completion UX, background agents |
| Gemini CLI | Terminal agent | Open source | Free tier, large context |
| Copilot Agent | Cloud PR agent | Closed | Async task model |
| DeepSeek Herons | Web agent UI | Reference only | Activity visualization inspiration |
| Hermes Agent / Zero | Local references | In workspace (`hermes-agent/`, `zero final/`) | To be audited next |

## Capability rows (seeded from Grok Build dissection + mission §10–§151)

| # | Capability | Seen in | Problem solved | Strength of best impl | Weakness / gap | Relevance | Z improvement opportunity | Status |
|---|---|---|---|---|---|---|---|---|
| 1 | Streaming tool protocol (Progress*→Terminal) | Grok Build | long tools stay responsive | uniform trait seam | async-only | ★★★ | sync progress channel over event bus | Implement |
| 2 | Dynamic tool manifest per turn (`should_list`) | Grok Build | token economy | built into trait | needs ctx design | ★★★ | context-aware manifest + lazy discovery (#147) | Implement |
| 3 | Tree-sitter repo index actor | Grok Build | fast defs/refs w/o LSP | incremental, mmap cache | no semantic layer | ★★★ | replace regex extractor; keep budget map_text | Implement |
| 4 | Full-replace compaction w/ validation | Grok Build | context overflow | tool-pair-safe, failure taxonomy | heavy | ★★★ | cheap-model summarizer + degenerate retry | Implement |
| 5 | Prompt queue combine gates | Grok Build | mid-run user input | merge saves tokens | product-coupled | ★★★ | EnqueueMessage command + drain between rounds | Implement |
| 6 | Memory w/ MMR + query expansion + dream consolidation | Grok Build | durable knowledge | retrieval quality | no inspection UI | ★★ | layered memory + Memory Inspector (#87) | Future |
| 7 | Worktree-per-agent parallelism | Grok Build | safe parallel agents | true isolation | daemon complexity | ★★ | in-process first, service boundary kept | Future |
| 8 | OS sandbox (JobObjects) around exec | Grok Build | blast-radius control | real isolation | Windows best-effort upstream | ★★★ | we are Windows-first: JobObjects + timeout on terminal_exec | Implement |
| 9 | Secret store + redaction | Grok Build | key hygiene | DPAPI-class storage | — | ★★★ | redaction filter on ALL outputs/logs | Implement |
| 10 | Circuit breaker for providers | Grok Build | flaky API resilience | principled backoff | — | ★★ | replace single-retry logic | Implement |
| 11 | Local token estimation | Grok Build | pre-send budgeting | offline counting | — | ★★★ | context engine dependency | Implement |
| 12 | SQLite event journal + session resume/search | Grok Build | crash recovery, history search | durable event sourcing | TUI-only surfacing | ★★★ | Task Journal + Trace Viewer foundation | Implement |
| 13 | Hooks before/after tool/task/edit | Grok Build, Claude Code | extensibility | typed hook points | trust model needed | ★★ | permission-scoped hooks registry | Future |
| 14 | Plugin marketplace + trust levels | Grok Build | ecosystem | install/trust flow | security surface | ★ | plugin-first expansion later | Future |
| 15 | ACP embedding for editors | Grok Build | external clients | protocol adapter | extra surface | ★ | adapter after core stable | Future |
| 16 | Evidence-gated completion | Mission §11 | fake "done" claims | — (novel) | — | ★★★ | completion requires runner exit codes/artifacts | Implement |
| 17 | Change intelligence (affected tests/symbols) | Mission §13 | no full re-ingest | — | — | ★★★ | diff → symbol graph → invalidation set | Implement |
| 18 | Token Inspector / Context Inspector | Mission §85–86 | cost transparency | — | — | ★★ | developer-mode panels fed by runtime metrics | Implement |
| 19 | Best-of-N evaluation runs | Mission §33 | quality on hard tasks | worktree isolation upstream | token cost | ★ | opt-in policy, evaluator agent | Future |
| 20 | Live UI preview + element→source mapping | Mission §37–38 | frontend iteration loop | — | — | ★★ | preview panel + DOM/source bridge | Future |

(Extend this table — do not let it stagnate. New research appends rows.)

## Gap analysis vs. best-in-class (summary)

- **Ahead:** native GPU UI with damage tracking + virtualization (z-gpui),
  accessibility tree as first-class, RTL/bidi text handling, Windows-first
  platform ownership.
- **Parity:** basic tool set, approval gate, provider BYOK config.
- **Behind (priority order):** repo intelligence depth (tree-sitter), compaction,
  durable session journal/resume, sandboxing, secrets/redaction, memory,
  steering queue, evidence-based completion, diagnostics center.

## Research method notes

- Source priority per mission §161: code > official docs > engineering posts >
  release notes > issues > community.
- Every row must cite where the capability was observed (repo path or doc URL)
  when added from a specific product.