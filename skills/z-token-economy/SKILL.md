---
name: z-token-economy
description: Z Desktop token economy — estimation, budgets, provider prompt caching, stable prefixes, deduplication, structured outputs, tool-result caching, incremental context. Use when optimizing token usage anywhere in the pipeline.
---

# Z Token Economy

## When this skill applies
Any change affecting how many tokens a request/response consumes, or any
caching that avoids recomputation/retransmission.

## The absolute rule

**Accuracy is never sacrificed for token savings.** A cheaper wrong answer
is worse than an expensive right one. Every optimization must preserve or
provably not-harm answer quality.

## Implemented today (verify in source)

- Local estimator `z-core/src/tokens.rs` (chars/4 baseline, CJK-aware,
  code-density correction) — drives pre-send budget checks.
- Context budgeting in runtime `build_request`: fixed prefix estimated,
  history trimmed at clean boundaries under a 128k/12k-reserve budget.

## Optimization ladder (in order of preference)

1. **Don't send it**: lazy tool definitions, on-demand capability discovery,
   don't inject what the turn doesn't need.
2. **Cache hits**: provider prompt caching via byte-stable prefixes;
   local AST/index/tool-result caches to avoid re-reading.
3. **Send less**: dedup repeated content, structured summaries of large
   outputs (with exact-source pointers), incremental context deltas.
4. **Compress representation**: structured outputs instead of prose where
   the consumer is another tool.

## Provider prompt-cache rules

- Fixed prefix must be byte-identical across turns: no timestamps, no
  volatile counters, no reordering inside the prefix.
- Changing anything in the prefix invalidates the whole cached prefix —
  treat prefix edits as expensive operations.

## Cache discipline

- Caches are never source of truth. File edits always rehydrate exact
  source. Cache entries carry fingerprints; mismatch = invalidate.
- Tool-result cache keyed by (tool, args, relevant-input-fingerprints).

## Measurement

Every optimization lands with a before/after measurement (tokens/request,
cache hit rate) recorded in docs/benchmarks. No optimization ships on vibes.

## Definition of Done

- Optimization has a test proving behavior preservation.
- Measurement recorded; regression gate defined if it guards a hot path.