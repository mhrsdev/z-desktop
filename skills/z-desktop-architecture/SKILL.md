---
name: z-desktop-architecture
description: Z Desktop Personal architecture boundaries, crate map, dependency rules, Core-vs-Extension policy, conventions, ADR rules, and source-of-truth hierarchy. Use when adding modules, creating crates, moving code between layers, or reviewing architectural changes.
---

# Z Desktop Architecture

## When this skill applies
Any task that creates, moves, or reviews code; any decision about where a
capability lives; any new crate, module, or cross-crate dependency.

## Repository layout (source of truth: the filesystem)

```
z desktop/            Rust workspace (the product)
  crates/z-protocol   Contracts: Command/Event enums, ProviderConfig, Risk, Id
  crates/z-core       Agent Runtime: threads, turns, tools, providers, repo index
  crates/z-shell      Workspace model: layout regions, panels, presets, view state
  crates/z-gpui       ZeroGPUI runtime: window, renderer, scene, a11y, timing
  crates/z-tokens     Design tokens: color, spacing, typography, theme
  crates/z-app        View layer: turns shell model into scenes, wires runtime
docs/                 Canonical specs, task ledger, research, ADRs
skills/               Hermes skills (this directory)
tools/                Developer utilities (validators, scanners)
references/external/  Research clones — NEVER compiled, NEVER published
```

## Dependency rules (enforced by review, target: cargo-deny later)

- `z-protocol` depends on NOTHING internal. It is the wire contract.
- `z-core` depends only on `z-protocol` (+ external crates). It must never
  depend on a UI crate. The same core must be drivable from a CLI.
- `z-shell`, `z-tokens` are pure data/model crates: no I/O, no threads.
- `z-gpui` depends on `z-shell` + `z-tokens`, never on `z-core`.
- `z-app` is the ONLY crate allowed to depend on both `z-core` and `z-gpui`.
  It is composition, not logic.
- Data flow is one-directional: UI → Command channel → Runtime thread →
  Event channel → event pump → EventQueue → drained at frame start → scene.

## Core vs Extension policy

- Core owns: runtime loop, protocol, sandbox, tools registry, provider
  abstraction, persistence, security boundaries.
- Extensions own: additional tools, panels, providers, parsers, renderers —
  anything that could plausibly be third-party. When a feature does not need
  core privileges, build it against the extension API, not inside core.
- Built-in features should dogfood the extension API where practical.

## Conventions

- Crate/module docs start with `//!` explaining WHY the module exists.
- Every nontrivial function carries a doc comment stating its invariant.
- Errors are `Result<_, String>` at internal boundaries today; structured
  error types are introduced per-domain as they stabilize (do not mass-
  convert preemptively).
- Tests live in the same file (`#[cfg(test)] mod tests`) unless they need
  fixtures; integration tests go in `crates/<crate>/tests/`.
- No async runtime in core. Blocking + threads is a deliberate choice
  (see ADR); do not introduce tokio "because it's modern".

## ADR rules

- Major decisions (new dependency, new crate, protocol change, security
  boundary change) get a short ADR in `docs/adr/NNNN-title.md`:
  context → decision → consequences, under ~60 lines.
- ADRs are immutable once accepted; supersede, never edit.

## Source-of-truth hierarchy

1. Current implementation — what actually exists (verify, don't assume).
2. `docs/Z-DESKTOP-MASTER-SPEC.md` — product/architecture intent.
3. `docs/Z-DESKTOP-TASKS.md` — backlog and per-task status.
4. `docs/DEVELOPMENT-STATE.md` — session resume state.
5. ADRs — binding engineering decisions.
6. Skills — operational instructions.

On contradiction: implementation wins for "what is", spec wins for "what
should be", and the contradiction must be recorded and resolved.

## Failure cases to avoid

- Putting UI types into z-core (breaks headless testability).
- Adding a dependency to z-protocol (breaks the contract crate).
- Reaching from z-gpui into runtime internals instead of going through
  Command/Event.
- "Temporary" direct filesystem access from the view layer.

## Definition of Done for architectural changes

- Dependency direction verified (no new reverse edges).
- Workspace `cargo test` green.
- ADR written if the change was a decision, not just code.
- `docs/DEVELOPMENT-STATE.md` architecture summary still accurate.