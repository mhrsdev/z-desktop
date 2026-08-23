# ADR-0012: Sub-agent orchestration (task store, orchestrator loop, spawn policy)

Ledger: orch-001..006 directly (task store, ready-set, orchestrator thread,
budget enforcer, spawn policy validation, isolation ladder); fixes cap
numbers orch-012 will implement and the enforcement points orch-020/021
consume. Unblocks: orch-010/011 (evidence verification, child failure
isolation ride the spawn contract), orch-013..017 (Best-of-N and merge need
spawn + worktree levels), orch-018/019 (UI consumes the journal-backed task
view), orch-024 (crash recovery replays the same records).

## Status

Accepted (2026-08-23). Justification: the direction is binding — §8.2
declares "task graph … orchestrator consumes task graph, spawns agent runs,
enforces budgets" with the invariants "no task runs without a journal record;
state transitions are journal events; recovery replays the journal"
(:443-444); §8.3 delegates the sub-agent contract to skills/z-subagents
(isolation ladder, exclusive write grants, reference-passed context,
parent-evaluated results, budget-aware caps, :452-458); §74.F sketches the
Best-of-N flow this must be able to express (:3144-3151); §55.4 makes "Task
record store | create/transition/query; journal-backed" and "Orchestrator v1 |
runs ready tasks respecting dependencies; budgets enforced" M4 acceptance rows
(:2553-2554), and §55.5 makes spawn policy, grants, worktrees, result
evaluation M5 rows (:2563-2567). This ADR decides what those commitments leave
open: WHERE task state lives given jour-008 exists as a planned reducer, WHICH
thread runs the loop, WHAT a sub-agent concretely is at personal scale, which
ledger feature enforces each ladder level, and where budgets bite. Adds no
dependency: §52's list is untouched.

## Context

What exists today (repo inspection 2026-08-23): one command-loop thread
(`Runtime::serve`, runtime.rs:213-214) dispatching `Command`s; each turn
spawns a named `z-turn` worker running `run_turn` to completion
(runtime.rs:408-423); `run_turn` already loops `for round in 0..
max_tool_rounds` with a cancellation flag checked at the round top and before
every tool call (runtime.rs:498-505, :586-590) and drains steering between
rounds (:507-512); the JSONL journal is append+replay with seq discipline
(journal.rs:188, :252; ADR-0004 posture per §84 :1902); local token estimation
and budget classification exist (`tokens::estimate`, `check_budget`,
tokens.rs:15, :85); sandboxed exec is a process-tree-guarded `sandbox::run`
returning `ExecOutcome` (sandbox.rs:45, Guard :159-164; ADR-0003
suspended-spawn). The journal currently carries lifecycle records only
(jour-001/005/024/029 IMPLEMENTED); jour-006 views, jour-008's task view
reducer, and TaskStateChanged/EvidenceRecorded events are PLANNED with the
dependencies already wired in the ledger (tasks :179-180, :221-225).

Scale honesty (personal-first, per ADR-0009/0010 precedent): one user, BYOK
API keys whose cost multiplies with concurrency, a desktop process sharing the
machine with the user's own work, repos where git may not even exist until
ADR-0008's advisory gate. Sub-agents are our own turns against our own
provider config — not fleet scheduling. Non-blocking guarantees, honest
accounting, and recovery matter more than throughput.

## Considered options

**(1) Task record store home (orch-001).**

*(a)* Journal-backed view: task state exists only as journal events
(TaskCreated/TaskStateChanged/EvidenceRecorded shapes); jour-008's reducer
folds them into an in-memory query view, exactly as threads views fold
lifecycle records. Matches §8.2's invariants verbatim, gives orch-024 crash
recovery for free (replay = state), feeds jour-022/023 and orch-018 UI events
from one source. Chosen.

*(b)* Separate `data/tasks.json` mutated alongside the journal. Two writers of
the same truth; every invariant becomes "keep these in sync" — precisely the
shared-mutable-state shape ADR-0009 removed for the index. Recovery needs a
merge rule instead of a replay. Rejected.

*(c)* SQLite now. Contradicts ADR-0004's JSONL-first posture (§84 :1902);
§55.4 already schedules the SQLite decision ADR separately with load data
(:2557). Rejected for M4.

**(2) Orchestrator loop placement (orch-003).**

*(a)* Driven from the command loop: ready-set recomputation and child-wait
would block command dispatch exactly like inline indexing did pre-ADR-0009;
waiting on children means minutes-long blocking or a poll timer on the thread
that must stay responsive. Rejected.

*(b)* Dedicated `z-orchestrator` thread with its own `std::sync::mpsc`
inbox and per-command reply senders — the ADR-0009 topology. ADR-0010
(5b) warned against actor cargo-culting, but the profile test passes here:
the orchestrator's natural state is BLOCKED waiting on child completions and
deadlines (`recv_timeout`), which is unshareable with any other duty, unlike
grant checks (nanosecond mutex ops that did not warrant an actor). Same std-only
channel shape, no select needed: completions arrive from turn workers,
commands from parent turns/UI. Chosen.

*(c)* No orchestrator; each parent turn spawns and supervises its own children
inline. Makes Best-of-N (which outlives no single parent turn and needs
post-parent evaluation) and crash recovery incoherent, and scatters cap
enforcement across call sites. Rejected.

**(3) Spawn shape (orch-005): what IS a sub-agent here?**

*(a)* Full child `Runtime` instance per sub-agent. Each Runtime owns a command
loop, event channel, Shared, and provider slot — N command loops serving
nobody (children take no interactive commands), duplicate provider/config
state, and cross-runtime approval wiring for the grant model we already have.
Fleet-scale machinery at desktop scale. Rejected.

*(b)* A nested `run_turn` worker with restricted inputs: fresh `Thread`
(own context window — skill principle 1), a restricted tool set derived from
grants, a `ChildSpec` carrying role prompt/task id/budget/model, spawned as a
named `z-turn`-style worker exactly like start_turn does today
(runtime.rs:417-422). Cancellation rides the existing `Shared.cancelled`
flag; approvals are settled at SPAWN time (the grant set IS the approval —
anything outside it fails classification rather than prompting, since there
is no user watching a child). Results return as a structured artifact record,
never merged into parent history unverified (skill principle 5). Chosen:
it reuses every mechanism above and adds only restriction, not machinery.

*(c*) Same-context role prompting ("you are now a researcher") with no context
or tool separation. Not a sub-agent by the skill's own definition (skill
:14-16); nothing isolated, budgets meaningless. Rejected.

Concurrency caps (orch-012), fixed numbers: global default **2** concurrent
children, **1** per parent, hard ceiling **4**; Best-of-N attempts default
N=3 but queue under the global cap. Rationale: BYOK rate limits and token
cost multiply linearly; two-way parallelism already covers genuine fan-out at
personal scale, and the skill demands budget-aware caps (skill :42-43).
Constants, not config, until orch-021 wires dev-mode settings — same stance
as ADR-0010's staging constants.

**(4) Isolation ladder enforcement map (orch-006).** The ladder itself is
fixed by the skill (:25-31); what was open is WHICH mechanism enforces each
rung:

| Level | Meaning | Enforcement site |
|---|---|---|
| L0 context-only | read-only research | tool set built WITHOUT write-class tools (classification per tools.rs `classify`); fresh Thread |
| L1 scope-granted | writes inside granted paths | full tool set; writes acquire ADR-0010 grants (`Shared` grant map, canonical paths, overlap rejected naming holder); orchestrator partitions scope up front — overlap = orchestration bug (skill :34-36) |
| L2 worktree | independent attempts | requires orch-007 over ADR-0008's CLI facade (`git worktree add`); registered under `data/worktrees/<task_id>/` per §98.1; owning task id mandatory, merge via safe-editing (§98.2), dirty → quarantine (§98.3) |
| L3 sandbox | untrusted evaluation | `z-core::sandbox::run` (suspended-spawn guard tree, ExecOutcome); sup-002 hooks exit codes into Build evidence |

Selection rule stays "lowest sufficient level" (skill :25); L2/L3 compose
(L3 inside an L2 worktree is legal and expected for risky experiments).

**(5) Budget enforcement point (orch-004).**

*(a)* Post-hoc accounting only: overspend discovered after the fact is not
enforcement. Rejected as sole mechanism.

*(b)* Provider-stream interception (kill mid-stream on token estimate):
mid-stream abort wastes the partial completion and estimates mid-flight are
noisier than pre-flight ones. Declined.

*(c)* Round-boundary enforcement inside the child's own turn loop, plus an
orchestrator-side wall-clock sweep. The `for round in 0..max_tool_rounds`
loop is the chokepoint every provider call already passes through and where
cancel + steering checks already live (runtime.rs:498-512): per-round the
child checks cumulative `tokens::estimate` against its budget, its remaining
tool rounds, and its wall-clock deadline — exceeding any converts to a clean
turn end flagged `budget_exhausted`, never a panic. The orchestrator's
`recv_timeout(next_deadline)` sweep is the backstop for hung children: past
deadline it sets `cancelled` (same path as CancelTurn, runtime.rs:228-229)
and waits for the worker to exit. Pause-vs-fail policy on exhaustion is
orch-020's decision, deliberately not fixed here. Chosen.

## Decision

1. **Task store = journal events + jour-008 view** (orch-001): new journal
   kinds TaskCreated {task_id, parent, role, grants, budget, level} /
   TaskStateChanged {task_id, from→to} / EvidenceRecorded (jour-023's shape,
   sup-001 types). The store API (create/transition/query) is a fold over the
   jour-008 view held by the orchestrator; replay rebuilds it; no second
   persistence artifact. States follow §8.2: created/running/paused/failed/
   completed.
2. **Orchestrator topology** (orch-003): one named `z-orchestrator` thread,
   `mpsc::channel::<OrchCommand>()` with per-command reply senders; commands:
   SubmitTask, CancelTask, Snapshot{reply}, SetCap (dev), Shutdown. Ready-set
   recomputation (orch-002) runs when TaskCreated/StateChanged are folded;
   children spawn through decision 3; completions arrive as messages from
   turn workers. Loop blocks in `recv_timeout(next_deadline)` so deadlines
   fire without a timer thread. Shutdown ordering mirrors ADR-0009: stop
   admitting → cancel outstanding children → join workers → exit; journal
   records survive for orch-024 resume.
3. **Spawn shape** (orch-005): a sub-agent is a ChildSpec {task_id, role
   prompt, task statement, grant list, budget {tokens, wall-clock, tool
   rounds}, model selection, ladder level} executed as ONE nested `run_turn`
   on a dedicated worker with a fresh Thread and the tool set restricted per
   level. No child Runtime, no child command loop, no interactive approvals —
   the grant set is the pre-settled approval surface. Results return as
   structured artifacts (patch bytes or report + evidence refs) journaled on
   the child's task; the parent verifies evidence before applying anything
   (skill :46-51; orch-010).
4. **Caps** (orch-012): global 2 / per-parent 1 / ceiling 4, Best-of-N N≤3
   queued under the global cap; admission control lives in the orchestrator
   inbox handler, so a spawn request that would exceed a cap queues as a
   paused task rather than being refused.
5. **Ladder enforcement** per the table in option (4); L0/L1 ship with
   orch-006, L2 activates behind orch-007/edit-026, L3 is already implemented
   and gets wired via sup-002.
6. **Budget enforcement** (orch-004): per-round check in the child turn loop
   (token estimate via tokens::estimate, rounds counter, elapsed time) +
   orchestrator recv_timeout sweep forcing `cancelled` past hard deadline.
   All three budget dimensions recorded per round for §99's Budget-efficiency
   scoring (0.1 weight, :3896).

## Consequences

**Non-blocking holds end to end**: parents, UI, and command loop never wait
on children; waiting is the orchestrator's own blocked state, which costs
nothing else.

**Crash recovery is replay, not repair** (orch-024): kill -9 anywhere loses
at most in-flight child turns; restart folds the journal, finds non-terminal
tasks, recomputes the ready set, resumes. Orphaned L2 worktrees are covered
by orch-008's startup scan (§98.2 "belt and suspenders"), not by the task
view.

**Child failure isolation is structural** (orch-011): a panicking child is a
panicking worker thread — contained at join like every z-turn today; it can
mark its task failed but cannot touch parent state, and L2 children corrupt
only their worktree. A leaked/killed child leaves at most a cancelled flag
unconsumed, swept by shutdown (skill DoD :60-62).

**Honest ceilings**: token counts are LOCAL ESTIMATES until M7 calibrates
against UsageReported (§55.7) — budgets are enforced against estimates and
say so in records. Caps are constants sized for BYOK reality; orch-021
relaxes them in dev mode only. L3 is process-tree containment (job objects /
process groups), not a security boundary against hostile code with kernel
ambitions — ADR-0003's threat posture governs.

**No new dependencies**: std threads + std mpsc; every mechanism cited
(round loop, cancelled flag, grants, sandbox, journal) already exists or has
a ledger owner.

**Testing obligations locked in**: spawn-policy validation rejects specs with
overlapping grants, empty role prompts, or levels above the project's git
capability; budget test drives a child past all three limits and asserts
clean budget_exhausted ends; cancellation propagation parent→child under the
cap queue; double-replay equality extends to task views (jour-012's rule);
leaked-child containment test kills a worker mid-write and asserts grant
release + parent integrity.

## Sources

- Repo inspection (2026-08-23): `z desktop/crates/z-core/src/runtime.rs` —
  `serve` command loop (:213-214), `Shared.cancelled` (:99) + CancelTurn
  handling (:228-232), turn worker spawn (:408-423), `run_turn` round loop
  with cancel checks (:498-505, :586-590), steering drain between rounds
  (:507-512), max-round stop (:660-662); `journal.rs` — append/replay/seq
  (:188, :252, :281); `tokens.rs` — estimate/check_budget (:15, :85);
  `sandbox.rs` — run/Guard/ExecOutcome (:45, :159-164).
- Z-DESKTOP-MASTER-SPEC.md: §8.2 (:431-446), §8.3 (:448-460), threading
  model (:1280-82), module table sandbox/tokens rows (:1275-1277), §55.4 M4
  rows (:2549-2557), §55.5 M5 rows (:2559-2567), §57 sandbox spec
  (:2655-2693), §59 journal format (:2724-2756), §74.F Best-of-N flow
  (:3144-3151), §84 decision index incl. ADR-0004 (:1902, :3467), §98
  worktree manager (:3862-3884), §99 evaluator rubric (:3887-3900).
- docs/Z-DESKTOP-TASKS.md (retrieved 2026-08-23): orch-001..025 ledger
  (:462-537), sup-001..003 (:539-548), jour-006/008 reducers (:176-180),
  jour-022/023 events (:221-225), jour status (001/002/005/024/029
  IMPLEMENTED).
- skills/z-subagents/SKILL.md: design principles 1-5 (:14-23), isolation
  ladder (:25-31), conflict avoidance (:33-38), parallelism rules (:40-44),
  result evaluation (:46-51), DoD (:59-62).
- ADR-0008 (git facade gating orch-007..009, :3-5), ADR-0009 (actor topology
  precedent and its limits), ADR-0010 (write-grant registry this ADR's L1
  rides; constants-not-config stance), ADR-0004 via §84 (JSONL-first).
