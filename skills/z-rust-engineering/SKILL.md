---
name: z-rust-engineering
description: Z Desktop Rust conventions — error handling, ownership, concurrency model (threads over async), crate boundaries, testing patterns, unsafe policy, performance discipline. Use when writing or reviewing Rust code in the workspace.
---

# Z Rust Engineering

## When this skill applies
Writing/reviewing any Rust code under `z desktop/crates/`.

## Concurrency model (deliberate)

- Blocking I/O + dedicated threads. NO tokio/async in core. The agent loop
  already runs on worker threads; channels (std::sync::mpsc) carry commands
  and events; condvars handle approval parking.
- Shared state uses Mutex/Arc with narrow critical sections. Lock ordering:
  never hold two Shared locks at once unless documented.
- Long-running work spawns named threads (`std::thread::Builder::new().name(...)`).

## Error handling

- Internal boundaries: `Result<T, String>` today. Do NOT mass-migrate to
  custom error enums preemptively; introduce typed errors per-domain when
  the domain stabilizes and callers need to match on variants.
- User-facing failures must be actionable strings ("no provider configured —
  set one in Settings"), never bare debug dumps.
- `unwrap()/expect()` allowed ONLY where the invariant is proven by tests or
  truly impossible to violate (e.g., pipes we just configured). Prefer
  expect() with a message explaining the invariant.

## Ownership & API design

- Prefer passing `&Path`, `&str`, slices; clone only at boundaries.
- Actor pattern for stateful subsystems (index, future services): one owner
  thread + channel; snapshots out. Avoid shared mutable graphs.
- Public crate APIs are minimal; keep types private until needed.

## unsafe policy

- unsafe appears only in platform interop (windows-sys, libc) and is
  confined to small functions with doc comments stating the safety contract.
- Every unsafe block gets a comment explaining why it cannot be safe code.
- No unsafe in z-protocol, z-shell, z-tokens, z-app.

## Testing patterns

- Unit tests inline (`#[cfg(test)] mod tests`) with descriptive names that
  state behavior ("runaway_process_is_killed_at_the_timeout_with_partial_output").
- Real-process integration tests for sandbox/tools (they take ~30s; that's
  acceptable).
- Protocol changes need serde round-trip tests.

## Performance discipline

- No allocation in hot loops without justification (rendering, indexing).
- Measure before optimizing; record benchmarks in docs/benchmarks.
- `[profile.dev] opt-level = 1` + deps at 3 exists for test speed — preserve.

## Definition of Done

- `cargo test --workspace` green; clippy-clean for touched crates when
  feasible; no new unwraps outside tested invariants.