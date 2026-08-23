# ADR-0010: Safe-editing pipeline (atomic writes, patch model, write grants)

Ledger: edit-004..007 (atomic write helper, routing, crash/reader tests),
edit-009..015 (patch application model, whole-patch abort, rollback staging,
spill), edit-016/017 (write grants), edit-029/030 (user-edit preservation
surface, multi-file transaction). Unblocks the M3 "Safe hands" rows that do
not already have an ADR (git access is ADR-0008) and gives orch-016
(merge-via-safe-editing) its substrate.

## Status

Accepted (2026-08-23). Justification: the direction is binding — §8.11 lists
"fingerprint-before-write, atomic temp+rename writes, scoped() on every path,
staged multi-file operations with rollback, user-edit preservation, exclusive
write grants" as the domain architecture; §89 fixes fingerprint mechanics
(fnv1a64 at read, ZD-E-0060 at write, old/new fingerprints journaled) and
sketches both atomic_write and rollback staging; §91.2 specifies the worked
example ("same-volume rename is atomic on all platforms"); §55.3 makes five of
these M3 acceptance criteria. This ADR decides what the spec leaves open: the
fsync/directory-sync ordering behind that sketch and its Windows behavior, the
whitespace-fallback policy for anchors, captured-bytes vs git-stash recovery,
where grants live now that ADR-0009 fixed snapshot reads for the index, the
transaction sequencing for edit-030, and the deliberate out-of-scope list.
Adds no dependency: §52's list is untouched.

## Context

What v0 does today: `fs_write` runs `std::fs::write` directly on the target
path (`z desktop/crates/z-core/src/tools.rs:272-287`, in-place write at :280)
after the `scoped()` boundary check (:133). In-place truncate-then-write means
a crash or kill mid-write leaves a truncated file — the exact failure §86's
edge table assigns to this pipeline ("Partial edit | atomic writes prevent;
rollback recovers"). There are no fingerprints yet (edit-001..003 land
separately); no grants; no patch tool. Risk classification is already right:
`fs_write` is Write-class (:44), so everything here sits behind approval.

What M3 must satisfy (§55.3 + skills/z-safe-editing): stale-write refusal with
re-read guidance; crash simulation leaves old-or-new, never partial; concurrent
readers see old-or-new; edit_patch with search/replace blocks where a missing
anchor fails cleanly (ZD-E-0061); multi-file op failure leaves zero partial
state; every failure mode produces an actionable message, never silent
corruption.

Scale honesty (personal-first, per ADR-0009's precedent): one user, local disk,
files bounded by fs_read's 512 KiB read cap (tools.rs:196) as the practical
working set, operations touching single-digit file counts, concurrent writers
are our own future sub-agents rather than arbitrary processes, and git is NOT
guaranteed present — ADR-0008 gates on an installed git ≥ 2.20 and any folder,
repo or not, can be opened as a project. Safety therefore cannot lean on git
for correctness; git is the user's history layer, not our transaction log.
Constraints carried in: blocking threads over async (ADR-0001), Result<T,String>
at internal boundaries, minimal-dependency discipline (§52) — nothing below
needs a crate; std::fs suffices.

## Considered options

**(1) Atomicity primitive.**

*(a)* Keep in-place writes (v0). One syscall fewer; a kill anywhere inside the
write window corrupts user data. Rejected — it is the bug being fixed.

*(b)* Temp file + rename over target, temp created in the target's own
directory. Same-directory placement makes cross-device rename (EXDEV)
impossible by construction and keeps the rename a same-volume metadata
operation, which is the atomic swap POSIX guarantees and NTFS provides via
MoveFileEx semantics. Ordering matters more than the primitives: fsync the temp
file BEFORE renaming (otherwise a power-loss window exists where the rename is
durable but the data behind it is not — the classic zero-length-file failure),
then sync the parent directory on POSIX so the rename itself survives power
loss. Chosen.

*(c)* `O_TMPFILE` + `linkat(AT_EMPTY_PATH)`. Linux-only, invisible-until-linked
elegance we do not need, and no Windows story. Rejected as platform-exotic for
identical guarantees.

*(d)* Our own write-ahead log / copy-on-write shadow copies as the durability
mechanism. Re-implements what rename gives for free, doubles write volume, and
confuses two layers: the journal (§59) records event metadata — old/new
fingerprints, never shadow bytes. Declined.

Crash-window accounting under (b): W1 death mid-temp-write → target untouched
(old). W2 after temp fsync, before rename → old. W3 after rename, before dir
sync → either old or new (rename may not be durable yet), which still satisfies
the invariant. The only way to violate old-or-new is writing the target in
place — option (a). Note we deliberately do NOT rely on filesystem-specific
heuristics (e.g., ext4's flush heuristics for rename-replace patterns); the
explicit fsync ordering is the contract.

Windows specifics: `std::fs::rename` corresponds to `MoveFileExW` with a
fallback to `SetFileInformationByHandle` (Rust std docs, retrieved 2026-08-23);
the documented flag set includes `MOVEFILE_REPLACE_EXISTING` and
`MOVEFILE_WRITE_THROUGH` (Win32 docs, retrieved 2026-08-23). std does not
expose WRITE_THROUGH and there is no directory-handle fsync equivalent, so
post-crash durability of the rename itself is best-effort there — but W3's
failure mode degrades to "old survived", which the invariant accepts. Real
Windows hazard instead: antivirus/indexer handles held open against the target
cause sharing violations at rename time → bounded retry (short backoff, ~3
attempts) then actionable error. Consequence accepted: the renamed-in file
carries the temp file's attributes/ACLs rather than preserving the original's —
standard save-by-rename behavior, editors do the same.

**(2) Patch application policy (edit-009..011).**

*(a)* Byte-exact match only. Strictest anchor discipline; but models routinely
emit blocks with drifted indentation, so legitimate edits fail spuriously and
train the model to fall back to full-content rewrites — the opposite of what
§30 intends. Rejected as sole policy.

*(b)* Whitespace-normalized matching silently always-on. Mutates user code
under indentation the model did not see, invisibly. Rejected.

*(c)* Exact match first; whitespace-normalized fallback second, surfacing a
warning in the tool result and journal (edit-010); anchor absent entirely →
clean failure ZD-E-0061 (edit-011), never a guess. Normalization is defined
narrowly: trailing-whitespace-insensitive per line, leading-indent compared
relative to the block's own first line, replacement text re-indented to the
matched region's actual indent. Chosen — strictness where it protects
(drift detection), flexibility only where it is provably cosmetic, and a
visible warning trail supervision (M6) can later consume.

Format alternative: line-anchored diffs instead of search/replace blocks.
Better for human review surfaces later, heavier for models to produce
correctly, and contradicts the tool shape §30 already declares for edit_patch.
Declined for M3; the in-memory application layer does not care where blocks
come from if review tooling ever wants another format.

**(3) Whole-patch abort (edit-013) vs best-effort partial apply.** Multi-block
patches apply sequentially against an IN-MEMORY buffer; any failed block aborts
the whole patch and nothing has touched disk — free atomicity, because
application is pure string work and disk contact happens exactly once, through
helper (1b), after every block resolved. Best-effort partials would couple the
patch layer to rollback for no user value. Trivially chosen.

**(4) Recovery mechanism: staged old bytes vs git-stash-style recovery.**

*Stash/recovery-by-git*: requires git (not guaranteed — ADR-0008 gate is
advisory), manipulates the user's ref and index space wholesale when z-git rule
5 declares uncommitted user work sacred and rule 2 forbids reset/checkout-class
operations without approval, restores MORE than the operation touched (any
unrelated dirty state gets swept into the stash dance), and stash-pop conflicts
are a recovery path that itself needs recovery. Byte staging restores exactly
the paths the op wrote to exactly their pre-op bytes — captured immediately
before writing (edit-014), spilled to a temp staging directory above a memory
cap (edit-015). If even byte restore fails, hard-stop with an error naming the
spill path — never silent. Chosen: staged bytes. Cost accepted: the rollback
window is the operation lifetime, not durable across restart; durable undo is
out of scope below and the journal's old/new fingerprints keep the audit trail.

**(5) Grant registry home (edit-016/017).**

*(a)* Static global registry (once_cell-style). Process-wide lifetime decoupled
from the runtime: leaks across tests, survives runtime teardown wrongly, hides
a dependency from every call site. Declined.

*(b)* Dedicated actor thread à la ADR-0009. The pattern fits work worth
isolating from other threads; grant checks are O(1) mutex-guarded map
mutations with zero I/O under the lock — the opposite profile from rescans. An
actor adds a channel round-trip to every write for no isolation win. Declined
as pattern cargo-culting; ADR-0009 fixed snapshot READS, not a house style
that everything must be an actor.

*(c)* Runtime-owned state in `Shared`: `Mutex<HashMap<PathBuf, Grant>>`, keyed
by scoped()-canonicalized path (canonicalization identical to fingerprints and
scope checks, else symlink aliases defeat exclusivity), valued by holding turn
id + acquired-at. Chosen. Lifetime matches turn lifetimes exactly — release at
turn end, cancellation, and runtime shutdown; overlap rejection happens at
grant-acquire time (edit-017) naming the holder; a liveness sweep reaps grants
whose turn no longer exists. Contention is irrelevant at personal scale: the
critical section is nanoseconds next to millisecond-scale writes.

Relationship to ADR-0009 kept explicit: grants serialize WRITERS; snapshots
serve READERS. The index remains a cache that rehydrates real bytes
(skills/z-repository-intelligence invariant 1) — grant state never touches it,
and index staleness after a write stays benign exactly as decided there.

**(6) Multi-file transaction shape (edit-030).**

*(a)* Persistent intent log + two-phase commit: survives process death
mid-commit, at the cost of a new durable artifact, a recovery pass, and state
machine — machinery sized for databases, not a desktop app whose ops touch
single-digit files. Deferred unless consequences force it (triggers below).

*(b)* Three-phase in-process sequencing over existing pieces: VALIDATE all
paths up front (scope + fingerprint + grant acquisition, all-or-nothing before
any staging) → STAGE old bytes per path (edit-014 machinery, spill included) →
APPLY each op through helper (1b) in deterministic order (paths sorted), →
journal commit record. Any validation failure aborts pre-stage (nothing
happened); any apply failure rolls back already-applied paths in REVERSE order
from staged bytes, then reports precisely which paths were restored. Chosen:
"zero partial state on failure" (§55.3) holds for every in-process failure,
using nothing new. Accepted debt: a hard process kill mid-apply can leave some
files new and some old; mitigation hooks are the spilled staging directory
(keyed by op id, swept at startup like any orphaned temp) plus the user's git.
This is stated honestly rather than papered over with a WAL nobody asked for.

## Decision

1. **One atomic write helper** (edit-004): temp `<name>.ztmp-<pid>-<n>` created
   in the TARGET's directory → write → `sync_all()` → `std::fs::rename` over
   target → parent-directory sync on POSIX. Temp removed best-effort on any
   failure. Every file-writing path routes through it: `fs_write` (edit-005),
   edit_patch application, transaction applies — no second write site (DoD,
   §91.2). Windows adds the sharing-violation retry and accepts the attribute
   note recorded above.
2. **Patch model** (edit-009..013): search/replace blocks applied sequentially
   in memory; exact match, else narrow whitespace-normalized fallback WITH
   warning; absent anchor → Err(ZD-E-0061); any block failure aborts the whole
   patch before disk contact. edit_patch additionally REQUIRES a session read
   fingerprint (§89.2) so the stale check rides edit-002/003 unchanged.
   fs_write without a prior read stays allowed-but-flagged per §89.2.
3. **Rollback = captured old bytes** (edit-014/015): stage `(path, old_bytes)`
   before each write; spill to temp staging beyond a 64 MiB aggregate cap;
   restore through helper #1 in reverse order; restore failure is a hard error
   naming the spill path. Git-stash-style recovery declined (rationale above);
   git remains the user's safety net, not ours.
4. **Write grants live in `Shared`** (edit-016/017):
   `Mutex<HashMap<PathBuf-canonicalized, Grant{turn_id, acquired_at}>>`;
   acquired before a turn's first write to a path, rejected at acquire time on
   overlap with an actionable message naming the holder, released on turn
   end/cancel/shutdown, reaped by liveness sweep. Not a static global, not an
   actor thread.
5. **Transactions are sequencing, not machinery** (edit-030):
   validate-all → stage-all → apply-sorted-via-helper → journal commit; failure
   ⇒ staged reverse-order rollback. Journal records BeginStagedOp /
   StagedOpCommitted / StagedOpRolledBack shapes under the repo's serde
   snake_case round-trip test rule.
6. **Deliberately out**: LSP-style buffers and ropey integration (editor v1,
   §35.6, owns those); undo stacks beyond single-op/in-process rollback; durable
   cross-restart undo; CRDT/merge machinery; file-watching triggers (wat-001);
   intra-file incremental tree editing (already deferred by ADR-0007).

## Consequences

**Crash safety**: old-or-new holds for every single-file write across kill,
crash, and power loss, independent of filesystem heuristics; the invariant is
enforced by construction because no production write path bypasses the helper
(edit-005 makes routing structural, and a guard test belongs beside edit-006).

**Cost**: one extra fsync + rename per write — milliseconds on local disks,
invisible at personal scale against network-bound model calls. Transactions pay
one validation pass total, not per-op.

**Model friction is a feature**: normalized-fallback warnings and clean
ZD-E-0061/ZD-E-0060 failures push re-read behavior and hand supervision (M6)
signal channels for free; silent-tolerance options were rejected partly for
this reason.

**Accepted debt** (revisit triggers, audit cadence per DEVELOPMENT-STATE):

- Rollback is not durable across restart; crash mid-multi-file-commit leaves
  mixed state recoverable only via spilled staging + user git. Trigger:
  real-world frequency of killed transactions → add the persistent intent log
  (option 6a) via superseding ADR; the journal already carries old/new
  fingerprints to build on.
- Windows rename durability is best-effort (old survives, never torn).
  Trigger: none foreseeable — the invariant cannot regress without an OS
  break of documented MoveFileEx semantics.
- Grants enforce intra-runtime exclusivity only; external editors are covered
  by fingerprints (edit-029), not grants. Trigger: contention patterns grants
  cannot see → consider advisory file locking, superseding ADR.
- Staging cap (64 MiB) and retry counts (3) are constants, not config.
  Trigger: measurement says otherwise; YAGNI until then.

**Testing obligations locked in** (mapping §55.3 + skills/z-safe-editing):
kill-mid-write crash simulation (edit-006); concurrent reader observes
old-or-new (edit-007); stale-write refusal with re-read guidance; missing-anchor
clean failure; whole-patch abort leaves file untouched; multi-file rollback
leaves zero partial state; grant overlap rejection names holder; canonical-
alias paths resolve to ONE grant key; helper-routing guard test (no `fs::write`
on targets outside the helper).

## Sources

- Repo inspection (2026-08-23): `z desktop/crates/z-core/src/tools.rs` —
  `classify` fs_write→Write (:44), `scoped()` canonicalization (:130-137),
  512 KiB fs_read cap (:196-198), `fs_write` in-place `std::fs::write`
  (:272-287, write at :280).
- Z-DESKTOP-MASTER-SPEC.md: §8.11 Safe Editing Engine (:574-581); tool family
  table incl. edit_patch risk (:1380-1385); M3 milestone (:1501); edge-case
  table "Partial edit" (:1078); §55.3 M3 acceptance rows (:2538-2547);
  edit_patch schema (:3068+); ZD-E code registry (:3431-3439); §32.6 ADR
  format (:1554-1557); §89 detailed design (:3599-3626); §91.2 worked example
  (:3675-3685).
- docs/Z-DESKTOP-TASKS.md edit-001..030 ledger (retrieved 2026-08-23).
- skills/z-safe-editing/SKILL.md (rules 1-5, patch formats, testing
  expectations, DoD); skills/z-git/SKILL.md (rules 1-6); skills/z-subagents/
  SKILL.md (exclusive grants, overlap = orchestration bug).
- ADR-0008 (git facade + version gate making git optional for us), ADR-0009
  (snapshot-read fix this decision complements; actor-pattern scope), ADR-0007
  (incremental tree editing deferral).
- Rust standard library documentation, `std::fs::rename` (doc.rust-lang.org,
  retrieved 2026-08-23): Windows implementation is MoveFileExW with fallback
  to SetFileInformationByHandle.
- Microsoft Win32 documentation, MoveFileExA (learn.microsoft.com, retrieved
  2026-08-23): MOVEFILE_REPLACE_EXISTING and MOVEFILE_WRITE_THROUGH flag
  semantics.
