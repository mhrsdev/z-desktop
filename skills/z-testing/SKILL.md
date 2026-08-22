---
name: z-testing
description: Z Desktop testing strategy — unit, integration, E2E, regression, performance, fuzzing, long-running and failure-injection tests. Use when writing tests or deciding test strategy for a feature.
---

# Z Testing

## When this skill applies
Writing tests for any feature; deciding what level of testing a change needs.

## Test pyramid in this repo

1. **Unit** (inline `#[cfg(test)]`): pure logic — budget math, redaction,
   scope checks, protocol serde. Fast, thousands eventually.
2. **Integration with real processes**: sandbox kill/timeout behavior,
   tool execution. ~30 s suite; acceptable.
3. **App-level**: `--check` headless startup; `--shot` screenshots.
4. **E2E/manual scripts**: full agent turns against recorded providers
   (planned: replay harness so E2E doesn't need live API keys).

## Rules

1. Tests are never modified to pass broken code (fix the code). Changing a
   test requires stating why the requirement itself changed.
2. Bug fixes add regression tests reproducing the bug first.
3. Failure injection is first-class: provider timeouts, malformed JSON,
   corrupt files, killed processes all have tests.
4. Long-running/soak tests exist for memory-leak verification of services
   (watchers, index actors) before they ship.
5. Fuzzing targets (cargo-fuzz later): path scoping, patch application,
   protocol parsing — anything consuming untrusted input.

## Naming & structure

- Test names state BEHAVIOR: "trimming_never_orphans_a_tool_result_carrier".
- Arrange-Act-Assert; fixtures via helper fns (temp_root pattern).
- Platform-specific tests cfg-gated; PING_LOCK-style serialization when
  tests probe global process state.

## Current evidence

Workspace suite: 280 tests green across 7 crates + integration (see
DEVELOPMENT-STATE for current count).

## Definition of Done

A feature is tested when its failure modes have named tests, not just its
happy path.