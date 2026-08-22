---
name: z-performance
description: Z Desktop performance engineering — CPU/RAM/GPU budgets, startup, idle behavior, indexing, UI latency, benchmarks, regression gates. Use when optimizing or setting performance targets.
---

# Z Performance

## When this skill applies
Optimization work, performance-sensitive design reviews, benchmark creation,
regression investigation.

## Philosophy

Measure first. Targets are commitments only after measurement exists. Never
report invented numbers. Every optimization records before/after in
docs/benchmarks/<topic>.md.

## Budget targets (aspirational until measured)

| Metric | Target |
|---|---|
| Cold start to interactive | < 1.5 s |
| Idle CPU | ~0% (no polling loops) |
| Idle RAM | < 150 MB |
| Frame time p95 | < 8 ms typical UI |
| Input echo | < 16 ms |
| Indexing throughput | > 10k files/min initial |
| Single-file incremental reindex | < 50 ms |
| Search p95 (medium repo) | < 200 ms |
| Terminal throughput | line-rate bound, not UI-bound |

## Resource discipline rules

1. No constant polling; event-driven or long-interval backoff only.
2. Watchers justify themselves: debounce, coalesce, and stop when unused.
3. Eager loading is guilty until proven cheap; lazy-load panels/providers.
4. Duplicate caches require an invalidation story or they don't ship.
5. Long-running sessions need leak checks (threads, handles, subscriptions).

## Benchmark practice

- Synthetic fixtures for repo scale (small/medium/large/very large).
- Criterion or hand-rolled timing harnesses under benches/ where useful.
- Regression gates on hot paths: CI fails if p95 regresses > 20%.

## Definition of Done

Performance work is done when measured improvement is recorded AND no
correctness test was weakened to achieve it.