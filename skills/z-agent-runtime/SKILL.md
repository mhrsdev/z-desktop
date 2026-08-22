---
name: z-agent-runtime
description: Z Desktop agent kernel — turn lifecycle, tool rounds, approval gate, cancellation, steering, checkpoints, recovery, supervision, and evidence-based completion. Use when modifying runtime.rs, the turn loop, protocol commands/events, or agent reliability behavior.
---

# Z Agent Runtime

## When this skill applies
Work on `z desktop/crates/z-core/src/runtime.rs`, `z-protocol` Command/Event
variants, the tool loop, approvals, cancellation, or any "agent behaves
wrongly" bug.

## Current architecture (verify against source before changing)

- One `Runtime` owns a command channel (`Receiver<(u64, Command)>`) and an
  event channel (`Sender<Event>`). `serve()` is the command loop.
- Each `SendMessage` spawns a worker thread running `run_turn`:
  stream → (tool calls → approve → execute)×N → done.
  MAX_TOOL_ROUNDS = 24 is the doom-loop ceiling.
- Approvals: non-read-only tools emit `ApprovalRequested` and park on an
  `ApprovalGate` condvar until `ResolveApproval` or a 300 s timeout.
- Cancellation: `CancelTurn` inserts the thread id into a cancelled set;
  workers check it between rounds and between tool calls.
- Persistence: threads serialize to `data/threads/<id>.json` at every
  mutation point; corrupt files are skipped on restore, never fatal.
- Context budget: `build_request` estimates fixed cost + history with the
  local token estimator and trims history only at clean user-turn boundaries
  so assistant tool_calls never lose their result carriers.

## Invariants (never violate)

1. The user's message is persisted BEFORE the first provider call.
2. A turn never leaves the thread file unparseable — save whole snapshots.
3. Tool results always reach the model even when denied/timed out ("denied
   by user" is itself a result).
4. Trimming history must keep request validity: no orphaned tool results,
   no unanswered tool_calls. Cut only at real user messages.
5. Events are fire-and-forget (`let _ = send`); a dead UI must not kill the
   core.
6. No provider call without a configured provider AND an open project root.

## Steering / pause / resume (planned — see ZD ledger)

Design direction: queued messages drain BETWEEN tool rounds, never mid-call;
consecutive plain-text queued messages combine into one; queue depth surfaces
as an event. Mid-run tool cancellation requires a cancel flag checked inside
the sandbox wait loop (recorded debt).

## Supervision & evidence rules

- Completion claims require evidence: exit codes for builds, executed test
  output for tests, diffs for edits. A turn that claims success without
  evidence events is a supervision failure to be detected, not tolerated.
- Repeated identical failing tool calls should trip a circuit breaker
  (planned); do not let agents burn 24 rounds on the same error.

## Failure handling expectations

- Provider transient failure: one retry on round 0, then fail the turn with
  the error; the user message stays persisted either way.
- Malformed model JSON: tolerate via `unwrap_or(json!({}))` at tool args;
  stricter validation lands per-tool.
- Approval timeout: record result, continue remaining calls, finish turn.

## Testing expectations

- Runtime logic gets unit tests in `runtime.rs` (`mod budget_tests` pattern).
- Sandbox/tool behavior gets integration-style tests with real processes.
- Any new Command/Event variant needs a round-trip serde test in z-protocol.

## Definition of Done

- Workspace tests green; new invariants have tests.
- Protocol changes are additive (old clients still parse).
- DEVELOPMENT-STATE updated if the turn lifecycle changed.