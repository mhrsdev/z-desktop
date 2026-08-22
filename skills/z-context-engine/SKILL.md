---
name: z-context-engine
description: Z Desktop context engineering — context lifecycle, budgeting, priority allocation, retrieval, compaction, freshness metadata, exact-source rehydration. Use when designing how information enters the model context or how history is managed.
---

# Z Context Engine

## When this skill applies
Anything that decides WHAT goes into a provider request: system prompt
assembly, repo-map injection, tool-result inclusion, history trimming,
compaction, retrieval-augmented turns.

## Layered model

1. **Fixed prefix** (stable across turns): identity + rules + repo map.
   Must be byte-stable turn-to-turn so provider prompt caches hit.
2. **Session state**: conversation history. Trimmed at clean boundaries
   (see runtime trim_history); compacted (summarized) when trimming alone
   cannot fit.
3. **Turn context**: retrieved files/snippets/tool results for THIS task.
   Priority-ranked; lowest priority drops first.
4. **Ephemeral**: streaming deltas, scratch state. Never persisted into
   prompts.

## Budgeting rules

- Budget = hard window − completion reserve − fixed prefix. History gets
  what remains (implemented in runtime.rs).
- Allocation is priority-based: safety-critical (paths, user constraints)
  > current task evidence > recent history > old history > nice-to-have.
- The local estimator (tokens.rs) drives pre-send decisions; real usage
  numbers from provider responses calibrate it over time.

## Compaction rules

- Compaction REPLACES history with a summary + pinned facts; it never
  silently deletes user constraints or open questions.
- Summaries record provenance (which messages were folded) so nothing is
  unrecoverable from the journal.

## Freshness & rehydration

- Every injected snippet carries freshness metadata (file fingerprint at
  injection time).
- BEFORE any edit based on earlier-injected content, re-read the actual
  file. Cache/index/map data is never sufficient authority for a write.

## Accuracy rule (absolute)

Token savings must never sacrifice accuracy. When in doubt, include the
exact source. Deduplication and summarization are allowed only when loss is
acceptable for the decision at hand.

## Testing expectations

- Budget math unit-tested (see runtime::budget_tests pattern).
- Compaction preserves-pinned-facts tests.
- Prefix stability test: two consecutive builds with no relevant change must
  produce identical fixed-prefix bytes.

## Definition of Done

- No unbounded growth path (every injection site has a cap).
- Every new context source declares priority + cap + freshness strategy.