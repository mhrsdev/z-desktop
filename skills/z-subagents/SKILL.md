---
name: z-subagents
description: Sub-agent engineering for Z Desktop — delegation design, context isolation, shared references, worktrees, parallelism, conflict avoidance, result evaluation. Use when designing or implementing sub-agent spawn policies, parent/child task relations, or parallel task execution.
---

# Z Sub-Agents

## When this skill applies
Designing or implementing sub-agent spawning, delegation, worktree isolation,
parallel task execution, or result merging inside Z Desktop.

## Design principles

1. A sub-agent is a scoped agent run with its OWN context window, its own
   tool budget, and a single deliverable. It is not a smaller chat.
2. The parent passes: role prompt, task statement, explicit file/scope
   grants, resource budget (tokens, time, tool rounds), and model selection.
3. Shared context travels by REFERENCE (file paths, symbol names, journal
   ids), never by copying large content into both contexts.
4. Cancellation propagates parent→child immediately; child completion never
   cancels the parent.
5. Results return as structured artifacts (patch, report, evidence log),
   which the parent evaluates before applying anything.

## Isolation ladder (choose the lowest sufficient level)

- Level 0 — same thread, fresh context: read-only research sub-tasks.
- Level 1 — same repo, restricted tool grants: scoped refactors.
- Level 2 — git worktree: independent implementation attempts (Best-of-N),
  risky migrations, experiments. Merge via diff review, never auto-push.
- Level 3 — separate process/sandbox: untrusted code evaluation.

## Conflict avoidance

- Two sub-agents must never hold write grants on overlapping files. The
  orchestrator partitions scope up front; overlap = orchestration bug.
- Worktree tasks merge through the safe-editing pipeline (fingerprint check
  at merge time), not by blind checkout.

## Parallelism rules

- Parallelism is budget-aware: N concurrent sub-agents multiply token and
  API costs; the scheduler enforces a configurable concurrency cap.
- Fan-out only when subtasks are genuinely independent; otherwise sequence.

## Result evaluation

Every sub-agent result carries: task id, files touched, evidence (test
output, build status), unresolved issues. The parent MUST verify evidence
before marking delegated work complete — a sub-agent's self-report is a
claim, not a fact.

## Implementation status

PLANNED. Foundation dependencies: durable task journal (parent/child task
relations), git worktree support, safe-editing merge checks. See ZD ledger
section "Sub-Agents" for the ordered task list.

## Definition of Done (for sub-agent features)

- Spawn policy, budget enforcement, cancellation propagation, and result
  verification each have tests.
- A leaked/killed sub-agent cannot corrupt parent state or the worktree.