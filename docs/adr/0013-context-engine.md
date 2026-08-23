# ADR-0013: Context engine layering & budget allocation

Ledger: decides the concrete types behind ctx-001 (layered context model),
ctx-002 (priority allocation weights), ctx-003 (compaction trigger), and the
freshness/rehydration shape ctx-006/ctx-007 consume. Unblocks directly:
ctx-004/005 (compaction procedure + idempotence build on the trigger defined
here), ctx-008 (inspector export reads the same item stream), ctx-009
(weights become settings rows), ctx-010..015, mem-009 (memory injection
enters through the turn layer), and core-045 (trim→compact escalation is
Decision 3 verbatim).

## Status

Accepted (2026-08-23). Justification: §8.13 fixes the layer names ("stable
prefix / session / turn / ephemeral"), priority allocation, "compaction with
pinned facts", "freshness metadata, exact-source rehydration before edits",
and marks the engine PARTIAL — "budgeting + trimming implemented;
compaction, priority allocation, freshness plumbing PLANNED". §8.14 binds
the prefix to provider prompt-cache discipline ("byte-stable prefixes";
"changing anything in the prefix invalidates the whole cached prefix").
skills/z-context-engine prescribes the budget rule ("hard window − completion
reserve − fixed prefix"), the priority ladder, and the rehydration invariant.
This ADR makes those commitments implementable against today's
`build_request` without a second decision round. It adds no dependency
(§52 untouched): everything here is std types plus the existing estimator.

## Context

The entire context pipeline today lives in one function,
`build_request` (runtime.rs:775–865), and it already embodies three of the
four layers implicitly:

- **Prefix**: the system string — identity, rules, project root, rendered
  repo map, active-model label (runtime.rs:790–801) — plus the serialized
  tool definitions counted separately (runtime.rs:804–813). The repo map
  comes from the index snapshot per turn (`map_text(160)`,
  runtime.rs:782–788; snapshot discipline per ADR-0009).
- **Session**: `thread.messages` (`StoredMessage` vec), budget-checked and
  trimmed by `trim_history` (runtime.rs:738–769), which drops whole turns
  from the front, cutting only at clean user-message boundaries so no
  assistant `tool_call` ever loses its result carrier.
- **Turn**: does not exist as a distinct site. Tool results ride inside
  `StoredMessage.tool_calls` summaries; there is no place where this-turn
  retrieval (mem-009, MCP resources, future RAG) could inject without
  hacking the system string or forging history messages.
- **Ephemeral**: streaming deltas and scratch state — correctly absent from
  prompts today, but absent by omission, not by construction.

Budget math: `CONTEXT_HARD_LIMIT = 128_000`, `COMPLETION_RESERVE = 12_000`
(runtime.rs:713–714); `soft_target = hard − reserve`, `history_budget =
soft_target − fixed` where fixed = estimate(system) + Σ tool defs + 16
(runtime.rs:814–816). The estimator is local and dependency-free
(tokens.rs:15–47). tokens.rs already defines a three-way verdict —
`Budget::Ok | Trim | Compact` (tokens.rs:72–94) — but **nothing produces
compaction**: `Trim` is handled by `trim_history`; the `Compact` arm exists
only as an unused classifier. When trimming cannot fit (no clean boundary
fits, runtime.rs:763–767 fallback), v0 sends untrimmed rather than lose
request validity — i.e., the failure mode is "bet on the window" exactly
when the conversation is biggest. core-044 (overflow refusal) and core-045
(trim→compact escalation) exist to close this; both wait on this ADR.

There is no freshness metadata anywhere in the request path. The safe-edit
pipeline already fingerprints files (edit-001 fnv1a64; edit-002 records a
fingerprint per thread at `fs_read`; edit-003 refuses stale writes), so the
*pipeline* enforces exact-source rehydration mechanically — but the *model*
cannot see staleness: a snippet injected five turns ago looks as
authoritative as one read this turn until a write bounces off edit-003.

Constraints inherited: blocking threads, no async in core (ADR-0001);
snapshot reads over shared mutable state (ADR-0009/0011 discipline);
prompt-prefix byte stability is an economic requirement, not a niceness
(§8.14, tok-001/002); accuracy is never traded for tokens (z-token-economy
absolute rule); personal-first scale — one user, one open project, windows
in the 128k class. Scale honesty: threads live for years (§5.3), so the
session layer needs a bounded end-state (compaction), not just front-drop.

## Considered options

**(a) Formalize layers as a `ContextItem` stream assembled by one pure
function.** Every candidate piece of context becomes a typed item declaring
layer, source, priority, and freshness; one assembler sorts, budgets, and
renders `Vec<ChatMessage>`; `build_request` shrinks to a caller. Matches
§8.13's names one-to-one, gives ctx-008's inspector a single exportable
stream, and gives mem-009/MCP a legitimate injection point. Chosen.

**(b) Keep ad-hoc assembly; bolt on a second injection point for turn
context.** Two special cases now, four later; budget accounting smears
across call sites and the inspector has nothing coherent to export. This is
v0's trajectory generalized. Rejected.

**(c) Fixed-percentage budget split (e.g. prefix 10% / session 60% / turn
30%).** Deterministic and trivially testable, but wasteful in the common
case: most turns have near-empty turn context, so a reserved slice sits
idle while session history trims harder than it must — and percentages need
re-tuning per model class. Rejected in favor of priority order with caps
(Decision 2), which degenerates to "everything fits, nothing reserved" when
the thread is small.

**(d) Summarize-on-every-turn (rolling summary instead of trim).** Rejected:
a summary call per turn multiplies provider cost and latency for a problem
threads only actually hit late in life, and it breaks the "trimmed requests
stay byte-comparable turn-to-turn" property that makes tok-001 testable.
Compaction is an event, not a background hum.

**(e) Demote the repo map out of the prefix** (it changes on rescan, so
byte-stability tok-001 chases a moving target). Rejected: moving it into
session/turn doesn't reduce churn, it just relocates it while losing the
"always present, always first" guarantee; rescan publishes coalesce
(ADR-0009 Decision 5), so invalidations are rare bursts, not per-turn
drift. Instead the prefix is *defined* as versionless-except-at-explicit
boundaries and tok-002 documents the impact (Decision 5).

**(f) Freshness as timestamps only** (age-based staleness). Rejected: wall
clock says nothing about correctness — a file untouched for a week makes a
week-old snippet perfectly fresh, and a mtime-equal content change (edit-001
exists precisely for this) defeats age entirely. Fingerprint comparison
against the edit-002 records is the only honest signal; it costs one u64.

## Decision

### D1 — Layered model (ctx-001): a `ContextItem` stream, assembled once

One type, one assembler, four layers named exactly as §8.13 names them:

```rust
enum Layer { Prefix, Session, Turn, Ephemeral }

/// One candidate unit of model context. Assembled per send; nothing here
/// mutates the thread store — Session items are views over StoredMessage.
struct ContextItem {
    layer: Layer,
    kind: ItemKind,          // SystemPrompt | RepoMap | ToolDefs | History(MsgRef)
                             // | TurnUserMsg | RetrievedSnippet | CompactionSummary
                             // | PinnedFact
    priority: Priority,      // Pinned > Critical > High > Normal > Low
    est_tokens: usize,       // tokens::estimate at assembly time
    freshness: Freshness,
}

struct Freshness {
    /// fnv1a64 content fingerprint captured at injection/read time
    /// (edit-001 convention); None = not file-derived (never "stale").
    fingerprint: Option<u64>,
}
```

Mapping onto today's code, with intent to preserve behavior:

| Layer | Today | Becomes |
|---|---|---|
| Prefix | system string + tool defs (runtime.rs:790–813) | `SystemPrompt`, `RepoMap`, `ToolDefs` items |
| Session | `thread.messages` after `trim_history` | one `History` item per kept `StoredMessage` |
| Turn | *(absent)* | current user msg + `RetrievedSnippet`s (mem-009, MCP resources) |
| Ephemeral | *(absent)* | never assembled — type-level exclusion, not convention |

Assembly replaces the body of `build_request` (runtime.rs:781–858) with one
pure function: collect items → drop `Ephemeral` → sort by the Decision-2
order → walk the budget → render `Vec<ChatMessage>` exactly as the current
match arms do (runtime.rs:829–857 unchanged in effect). With zero Turn
items and a fresh thread, output is byte-identical to v0 — the migration
is provable by absence, same discipline as ADR-0011's settings cutover.

### D2 — Priority allocation (ctx-002): strict order + caps, no percentages

Allocation is a priority walk, not a split. Order (each tier takes what it
needs before the next starts):

1. **Prefix** — must fit; it is the request's skeleton. Already bounded in
   practice (`map_text(160)` caps the map at 160 lines; tool defs are a
   closed set). If prefix alone exceeds soft target, that is a config bug,
   surfaced loudly, not allocated around.
2. **Pinned items** — `PinnedFact`s and the latest user message. Never
   dropped, never compacted (ctx-011 codifies the latter as a test).
3. **Turn context** — `RetrievedSnippet`s, highest priority first, capped:
   `TURN_LAYER_CAP = CONTEXT_HARD_LIMIT / 8` (16k). Overflow within the
   layer drops lowest-priority items first (z-context-engine's rule).
4. **Session** — newest-clean-boundary-first via the existing
   `trim_history` walk (runtime.rs:748–762), getting whatever remains.

Rationale: tiers 1–3 are small and non-growing (≤ ~20% of window combined);
the session layer absorbs all long-run growth, which is exactly the shape
v0 already implements. Fixed weights would reserve idle slices (option c).
Weights become dev-mode settings only if measured pressure shows a need
(ctx-009 reads them from the ADR-0011 settings snapshot; defaults live in
this table until then — one source of truth).

### D3 — Compaction trigger (ctx-003): trim first, compact on trim exhaustion

Two-stage escalation, implementing core-045:

- **Stage 1 — trim (status quo)**: `Budget::Ok | Trim` resolves exactly as
  today via `trim_history`. Zero behavior change in steady state.
- **Stage 2 — compact**: triggered when trimming *exhausts* — concretely,
  when the best clean cut either doesn't fit (the runtime.rs:763–767
  fallback fires) or fits only by keeping less than
  `TRIM_FLOOR = history_budget / 2` of the budget (we're dropping most of
  the thread to scrape under). At that point the span that stage 1 would
  have discarded is summarized instead of deleted: one `CompactionSummary`
  item carrying pinned facts (user constraints, decisions, open questions)
  + a round-by-round digest, with provenance naming the folded message ids
  (z-context-engine compaction rules; journaled per ctx-010).
- **Exhaustion**: if prefix + pinned + turn + compacted session still
  exceed the hard limit, refuse with the core-044 overflow message rather
  than send a truncated lie. Never silently degrade accuracy
  (z-token-economy absolute rule).

Compaction runs synchronously on the turn thread before the provider call
(no async, ADR-0001); it costs one extra small provider call on a turn that
is already oversized — acceptable at personal scale. Idempotence per span
(ctx-005: a span folds once, keyed by message-id range) and the compaction
procedure itself (ctx-004) are downstream tasks building on this trigger;
this ADR fixes only *when*, not the summarization prompt.

### D4 — Freshness & exact-source rehydration (ctx-006/ctx-007)

- Items derived from file bytes carry the fnv1a64 fingerprint observed at
  capture (`Freshness::fingerprint`). At assembly time each is compared to
  the thread's recorded fingerprint state (edit-002's per-thread records):
  mismatch ⇒ the item is **stale this turn**.
- Stale rendering, decided now: a stale `RetrievedSnippet` renders with a
  one-line marker (`[stale: <path> changed since capture — re-read before
  relying on it]`) and demotes one priority tier. We mark; we do not
  auto-re-read at assembly (that would make request assembly do hidden
  I/O and break the snapshot-read discipline). Rehydration remains an
  agent action: **before any edit, `fs_read` the exact file** — already
  mechanically enforced by edit-003's stale-write refusal; the marker
  makes the model a willing participant instead of a surprised one, and
  feeds ctx-013's staleness warnings for free.
- Prefix items are exempt: the repo map is navigation, not authority
  (ADR-0009 Consequences: "correctness-critical paths re-read files; the
  index only shapes navigation"). It carries no fingerprint claim.

### D5 — Prefix stability contract (feeds tok-001/tok-002)

The prefix is byte-stable **within a project session**, changing only at
declared boundaries: project switch, provider switch (label line,
runtime.rs:799), completed reindex (new `RepoMap` text). Each such change
is a deliberate prompt-cache invalidation — accepted because rescans
coalesce (ADR-0009) and switches are user-initiated — and tok-001's guard
test asserts exactly this: two consecutive builds with no declared boundary
produce identical prefix bytes. Volatile data (timestamps, counters) is
barred from the prefix by construction: `ContextItem` has no field that
could carry one.

### D6 — Out of scope

Semantic embeddings and retrieval ranking (mem-* track — mem-009 will
inject ranked candidates as `RetrievedSnippet`s through the turn layer, but
how they are ranked is a memory-track decision); cross-session memory
persistence (ctx-014 handles session-layer persistence mechanically, not
memory semantics); provider-side prompt-cache APIs and pricing (tok-*);
tool-result caching keyed by fingerprints (tok-003..005 — adjacent, shares
the fnv1a64 convention, separate mechanism); the context inspector UI
(ctx-008 consumes the item stream decided here); compaction prompt wording
and summarization quality (ctx-004).

## Consequences

**Immediate**: ctx-001 reduces to introducing `ContextItem`/`Layer`/
`Freshness` and rewriting `build_request`'s middle as the assembler — a
mechanical refactor whose no-turn-items output is asserted identical to
today's. ctx-002 is constants plus a sort; ctx-003 wires the existing
unused `Budget::Compact` arm (tokens.rs:92) to a real procedure; ctx-006/
007 add one `Option<u64>` and one render branch. Nothing else moves.

**The overflow cliff becomes a staircase**: v0's worst case (untrimmed send
on fallback, runtime.rs:767) is replaced by trim → compact → refuse, in
that order, each stage observable in the journal (ctx-010 logs token counts
per stage). core-044's refusal message lands as the staircase's bottom step.

**Honest staleness**: the model sees `[stale]` markers instead of
discovering staleness via bounced writes. Cost: one fingerprint compare per
file-derived item at assembly — nanoseconds against the existing estimator
passes. The pipeline-side edit-003 enforcement is untouched; this is
visibility, not a second enforcement layer.

**Prefix invalidations stay rare and declared** (D5), keeping provider
prompt-cache economics intact (§8.14) and giving tok-001 a decidable spec.

**Accepted debt**: compaction quality is unproven until ctx-004 ships and
ctx-011 proves the never-compact-latest-user-message invariant; `TURN_LAYER_CAP`
and `TRIM_FLOOR` are reasoned constants, not measured ones — they are the
first candidates for ctx-009 settings exposure and should be revisited on
first real overflow telemetry, not before. No percentage-based allocation
exists to tune; if a future multi-model world needs per-window profiles,
that is a successor ADR amending D2's table.

**Testing obligations locked in**: byte-identical parity test (assembler vs
legacy `build_request`, no turn items); budget-allocation unit tests
(ctx-012) covering tier starvation order; trim-floor trigger test (compact
fires iff stage 1 exhausts/falls below floor); idempotent-compaction test
(ctx-005); never-compact-latest-user-message test (ctx-011); stale-marker
render test with a mutated fixture file; prefix-stability test across a
mock rescan boundary (tok-001).

## Sources

- Repo inspection (2026-08-23): `z desktop/crates/z-core/src/runtime.rs` —
  budget consts (:711–714), `stored_message_tokens` (:717–728),
  `trim_history` clean-boundary walk + no-fit fallback (:738–769),
  `build_request` prefix/system/map assembly (:781–801), fixed-cost +
  `history_budget` math (:803–817), `ChatMessage` rendering (:827–865);
  `crates/z-core/src/tokens.rs` — `estimate` (:15–47),
  `Budget::Ok|Trim|Compact` + `check_budget` (:70–94, Compact arm unused).
- Z-DESKTOP-MASTER-SPEC.md (retrieved 2026-08-23): §5.3 data scale
  (:287–290), §8.13 Context Engine (:597–604), §8.14 Token Economy
  (:606–612), §8.15 Cache Architecture fingerprint table (:618–627),
  §96 settings categories incl. "Context: budget sizes, priority weights,
  compaction triggers" (:967).
- docs/Z-DESKTOP-TASKS.md (retrieved 2026-08-23): ctx-001..015 ledger with
  dependency edges; edit-001..003 IMPLEMENTED (fnv1a64, per-thread
  fingerprint records, stale-write refusal); core-044/core-045; mem-009 ←
  ctx-001; tok-001..013.
- skills/z-context-engine/SKILL.md (layered model, budgeting rules,
  compaction rules, freshness & rehydration invariants, testing
  expectations); skills/z-token-economy/SKILL.md (accuracy-is-absolute
  rule, prompt-cache byte-stability rules, optimization ladder);
  skills/z-agent-runtime/SKILL.md (trim validity invariant 4, budget_tests
  pattern).
- docs/adr/0009 (index actor: snapshot reads, map_text consumption,
  coalesced rescans, "cache never authority" consequence), 0011 (settings
  snapshot-cache access pattern reused for ctx-009; additive-evolution and
  prove-by-absence disciplines).
