# ADR-0014: Project memory architecture (record schema, layer stores, provenance, superseding)

Ledger: decides mem-001 (record schema), mem-002 (layer stores),
mem-003 (provenance/confidence enforcement point), mem-004 (superseding
chains), mem-005 (candidate extraction from journaled turns). Unblocks
directly: mem-006/007 (consolidation pass + promotion write into the stores
defined here), mem-008/009 (retrieval ranks live records; injection rides
ADR-0013's turn layer), mem-010/011 (correction = a superseding record on
the chain fixed in D4), mem-015/020 (replay and caps tests consume D2/D5),
mem-018 (anchor invalidation binds to edit-001 fingerprints per D1), and
mem-012 (inspector browses exactly the fields fixed in D1).

## Status

Accepted (2026-08-23). Justification: §8.16 makes the direction binding —
"Five layers (working/session/project/semantic/episodic); provenance +
confidence + superseding mandatory; consolidation explicit;
user-correctable; journal-backed replay" (:631-638); §30.5 already sketches
the memory record (:1481-1488); §49.11 fixes the storage posture ("Store
per layer under data/memory/<layer>/ as journal-derived views"; candidate
memories require confirmation; correction creates superseding records with
dependents flagged, :2379-2387); §104 fixes consolidation timing and bounds
(:3991-4010); the glossary defines provisional memory (:4177).
skills/z-memory prescribes the five rules every layer obeys (provenance,
supersede-with-retention, anchor invalidation, user control, explicit-only
consolidation) and the storage direction ("views are caches, the journal is
truth"). Everything those commitments leave open is mechanical: the exact
serde shape, where enforcement lives, how chains resolve deterministically,
and which file each layer occupies. Adds no dependency (§52 untouched):
JSONL + the existing journal/replay stack only.

## Context

What exists today (repo inspection 2026-08-23): the JSONL journal is real —
schema + writer (jour-001 IMPLEMENTED), O_APPEND+fsync policy (jour-002
IMPLEMENTED), ordered replay engine (jour-005 IMPLEMENTED); rotation and
checksum trailers remain jour-003/004. The safe-edit pipeline computes
fnv1a64 content fingerprints (edit-001 IMPLEMENTED) and records them per
thread at `fs_read` (edit-002 IMPLEMENTED). ADR-0013 defined the
`ContextItem` stream whose `Freshness::fingerprint` is an `Option<u64>`
using the same fnv1a64 convention, and reserved the turn layer for
`RetrievedSnippet`s that mem-009 will inject — "how they are ranked is a
memory-track decision", decided here as far as D4's live-record predicate.
The data layout reserves `memory/` under the data dir (:2199, planned);
threads persist as whole-snapshot JSON files with "unknown fields tolerated
on read; corrupt file → skipped with warning" (§30.1).

Scale honesty (personal-first, ADR-0005 posture; ADR-0009/0012 precedent):
one user, one open project; §104.3 caps the firehose at ≤ 100 candidates
per pass and ≤ 10 MB of new memory writes per day. This is a small,
append-heavy, read-ranked corpus — gigabytes over years at worst, and most
of it cold. Constraints inherited: blocking threads, no async in core
(ADR-0001); JSONL-before-SQLite posture (ADR-0004, "Accepted (revisit M4)",
:1902 — the revisit trigger is measured load, not aesthetics); interactive
latency never sacrificed to background work (§8.21 invariant, :695).

There is no memory code yet. The risk this ADR manages is not performance
but *truth drift*: memory that silently diverges from what was said, sources
that cannot be re-checked, corrections that lose history, or a store that
cannot be rebuilt after corruption.

## Considered options

**(1) Store home for the three persistent layers (mem-002).**

*(a)* Journal events are truth; one append-only JSONL view file per layer
under `data/memory/<layer>.jsonl`, rebuildable by replay (delete the files,
fold `memory_recorded` events through jour-005, byte-compare). Matches
§49.11's "journal-derived views" verbatim, §1097's replay promise ("Replay
rebuilds: threads, task graph, memory views, caches"), z-memory's "views
are caches, the journal is truth", and ADR-0004's posture. Crash recovery
and mem-015's replay feature are the same mechanism. Chosen.

*(b)* Standalone `data/memory.json` mutated alongside the journal. Two
writers of the same truth — the exact shape ADR-0012 option 1(b) rejected
for tasks; every invariant becomes a sync rule. Rejected.

*(c)* SQLite now. Contradicts ADR-0004 (revisit scheduled against load data
at M4, not before); SQL buys nothing for last-line-wins folds and id-keyed
chains at ≤10 MB/day. Rejected for now; the view files are throwaway caches,
so a later SQLite migration rewrites nothing canonical.

**(2) Where writes land (mem-005 path).**

*(a)* Every memory mutation is a journal event (`memory_recorded` carrying
the full record payload); the JSONL views are derived by fold. One source of
truth; mem-020-style tests assert view ≡ replay. Chosen.

*(b)* Write records straight into `<layer>.jsonl` and journal a pointer.
The view becomes truth by accident the first time someone skips the journal
call, and replay can no longer rebuild state — breaking §1097 and the
z-memory testing expectation "journal replay reproduces identical memory
state". Rejected.

On §29's "events carry ids, not blobs" (:1432): memory records ride as
payloads anyway because they are small bounded prose (the rule targets
diffs/tool outputs fetched on demand) and because replay must be
self-contained — an event that cannot reconstruct its record makes the
journal a lossy truth.

**(3) Superseding representation (mem-004).**

*(a)* Immutable records linked in a chain: a correction appends a new
record naming its predecessor (`supersedes`); the view backfills the old
record's `superseded_by`. Both sides retained for audit (z-memory rule 2),
chain walk is O(depth), deterministic tie-break possible. Spec shape keeps
both field names (:1486). Chosen.

*(b)* Tombstones: mark the old record dead, write replacement elsewhere.
A tombstone cannot carry the replacement content, so you still write a new
record — (a) minus the backward link, plus a second concept. Rejected.

*(c)* Mutable version list inside one record. In-place mutation destroys
the append-only property the journal enforces and makes "what did we believe
when" unanswerable. Rejected.

**(4) TTL field on the record.** Rejected outright. z-memory forbids
silent TTL expiry of user-corrected facts; aging is already owned by
confidence recency decay (§104.2 step 3; mem-013 implements it). A TTL adds
a second, silent, non-auditable death mechanism. The proposed
`ttl?` field is dropped; expiry-by-time does not exist in this design.

## Decision

### D1 — Record schema (mem-001): §30.5 amended at four points

Persistent memory layers are exactly project | semantic | episodic.
Working/session are not stored here: working dies with the turn, session
lives in threads + the context engine's session layer (ADR-0013 D1 table).
The spec names five layers but its own record enum lists three (:1484) —
this ADR resolves that in the enum's favor.

```rust
enum Layer { Project, Semantic, Episodic }
enum ProvKind { Message, Tool, User }
enum Status { Provisional, Promoted }

struct Provenance {
    kind: ProvKind,
    ref_: String,      // resolvable coordinate: message id / tool-call id /
                       // consolidation-pass id / "user"
    thread_id: String, // denormalized from the journal event so ranking and
    turn_id: String,   // the inspector never re-read the journal per candidate
    ts: String,        // capture time (RFC 3339)
}

struct Anchor {        // mem-018 binds here
    kind: File,
    path: String,      // amendment: fingerprint is unverifiable without it
    fingerprint: u64,  // edit-001 fnv1a64 — same u64 as Freshness (ADR-0013 D4)
}

struct MemoryRecord {
    id: String,                  // "mem-<hex>"
    layer: Layer,
    status: Status,              // amendment, see below
    content: String,             // bounded prose; cap enforced at write
    provenance: Provenance,      // required
    confidence: f32,             // required, clamped [0,1]
    supersedes: Option<String>,  // predecessor id
    superseded_by: Option<String>, // backfilled by the reducer, never set directly
    anchors: Vec<Anchor>,
}
```

Amendments to §30.5, each forced by another binding line:

1. **`status: Provisional|Promoted` added** — §49.11 "candidate memories
   require confirmation" (:2382-2383), §104.2 step 4 ("write candidates as
   provisional; promote after N independent sources or explicit user
   confirmation"), glossary :4177. Without a status field these lines have
   no representation.
2. **Provenance merged**: spec has `{kind, ref}`; the ledger proposal adds
   `{turn_id, thread_id, ts}`. Kept all five — `ref` stays the canonical
   coordinate; thread/turn/ts are denormalized reads of the originating
   journal event so mem-008 ranking gets recency without journal I/O.
3. **Anchor gains `path`** — a bare fingerprint claims nothing checkable;
   mem-018 compares against the current bytes of a named file.
4. **`ttl` rejected** (option 4); `superseded_by` retained but
   reducer-managed (D3).

### D2 — Store layout (mem-002): three JSONL views fed by one journal stream

Writes: one journal event kind `memory_recorded { record }`. Views:
`data/memory/project.jsonl`, `semantic.jsonl`, `episodic.jsonl` — plain
append-only JSONL, one line per record version, **last line per id wins**
on load (log-compaction semantics; no rewrite machinery). This spells
§49.11's `data/memory/<layer>/` as one file per layer: a directory earns
its existence when a layer grows sidecars (embedding vectors would do it —
that day is mem-016's problem, deliberately not today's). Corrupt line →
skipped with warning, matching the §30.1 thread-file precedent.

Reducer discipline mirrors jour-006..008: applying `memory_recorded`
appends to the target layer file; if the record carries `supersedes`, the
reducer also appends an updated line for the predecessor with
`superseded_by` set. Deleting any or all view files and replaying the
journal must reproduce equivalent state — that equivalence is mem-015's
test, stated once here as the acceptance bar.

### D3 — Provenance/confidence enforcement (mem-003): at the single write gate

Enforcement is structural, not conventional:

- `MemoryRecord` has no public mutable fields; construction goes through
  one constructor requiring provenance and confidence — there is no way to
  author a record without them, so no call site to forget.
- serde: both fields are non-Option → deserialization fails on absence.
- The reducer refuses `memory_recorded` events with empty provenance or
  out-of-range confidence (skip + warn, same discipline as corrupt lines).
- Exactly three writers exist, each fixing provenance semantics:
  **explicit user save** (ProvKind::User, Promoted, confidence 1.0);
  **supervised extraction** (mem-005, Message/Tool refs to the journaled
  turn, Provisional); **consolidation pass** (mem-006, Tool ref to the pass
  id, Provisional until promotion). Anything else is a bug, not an extension
  point.

### D4 — Superseding chains (mem-004): linear pointers, tip wins

Chains are per-topic linear sequences built from `supersedes` pointers;
`superseded_by` exists so retrieval never scans. Retrieval's live-record
predicate, fixed here for mem-008 to rank within: **a record is live iff
`superseded_by == None` and `status == Promoted`**; among competing tips
(two successors claiming one predecessor — possible from concurrent passes),
highest journal seq wins. That tie-break makes conflict resolution
deterministic, which is z-memory's first testing expectation. Corrections
(mem-010) and dependent-flagging (mem-011) are consumers of this chain, not
new machinery: a correction is a User-provenance record superseding the old
tip; dependents are found by walking `ref_`s that cite a superseded record.

### D5 — Candidate extraction (mem-005): batched post-turn, provisional only

Extraction runs after `turn_finished`, batched under Batch priority —
never inline in the turn (§104.1; §8.21 latency invariant :695). It reads
the journaled turn stream via jour-005's replay, detects recurring
entities / repeated decisions / stable project facts (§104.2 step 1), dedups
against existing memories by anchor + content similarity (step 2), and
emits `memory_recorded` events with Provisional status. It never promotes;
promotion (N-independent-sources or user confirmation) is mem-007 riding
the same event log. Bounds fixed here for mem-014/mem-020 to implement and
test: **≤ 100 candidates per pass, ≤ 10 MB new memory writes per day**
(§104.3 verbatim — cited, not invented).

### D6 — Out of scope

Embeddings and vector stores (mem-016 is its own RESEARCH ADR; mem-017
stays RESEARCH-until-justified — lexical/dedup similarity suffices at this
corpus size); cross-device sync (personal-first; portability is export via
exp-001, which mem-019 consumes); consolidation procedure internals and the
promotion threshold value (mem-006/007 decide those; only the caps and the
provisional landing zone are fixed here); ranking weights beyond the
live-record predicate (mem-008); memory UI (mem-012 browses D1's fields);
recency-decay formula (mem-013).

## Consequences

**Immediate**: mem-001 reduces to the struct above + constructor + serde;
mem-002 to one event kind, one reducer, three append-only files;
mem-003 to ~a dozen validation lines in one gate; mem-004 to two
`Option<String>` fields plus a seq tie-break; mem-005 to a post-turn batch
job emitting records. Nothing else moves; no new dependency.

**Audit is free, deletion is loud**: superseded records stay on disk
forever unless the user deletes them (mem-012 surface, z-memory rule 4) —
time never retires anything silently (no TTL). Anchored memories are
*invalidated lazily*: mem-018 compares stored anchor fingerprints
(edit-001 fnv1a64) against current file bytes at retrieval time and drops/
marks stale candidates; the record itself survives for audit, consistent
with ADR-0013's mark-don't-mutate freshness stance.

**Rebuildability is the safety net**: every failure mode (torn line, bad
migration, future format change) resolves to "delete `data/memory/*.jsonl`,
replay". The cost is journal growth — accepted; segment rotation (jour-003)
is shared infrastructure, and memory events are the smallest payloads on
the wire.

**Accepted debt**: last-line-wins means a long-lived, heavily-corrected
topic accumulates dead lines until a compaction pass exists (none planned —
file sizes are bounded by the 10 MB/day cap; add compaction when a view
file measurably dominates load time). Denormalized thread/turn/ts can
drift from the journal if the referenced event is ever pruned; acceptable
because segments rotate, not delete, and `ref_` remains the canonical
coordinate. Confidence values across the three writers are not yet
calibrated against retrieval quality — mem-013's decay tuning is the first
honest measurement point.

**Testing obligations locked in**: contradictory-tip determinism test
(z-memory expectation 1); anchor-invalidation test with a mutated fixture
file (expectation 2, feeds mem-018); delete-and-replay byte-equivalence
test (expectation 3, mem-015); provenance-less record rejected at
constructor AND reducer (mem-003); cap-enforcement tests (mem-020);
provisional-never-retrieved-as-live test (D4 predicate).

## Sources

- Repo inspection (2026-08-23): journal writer + replay implemented
  (docs/Z-DESKTOP-TASKS.md :158-171, jour-001/jour-002/jour-005);
  fnv1a64 fingerprint utility + per-thread fs_read records implemented
  (:372-376, edit-001/edit-002); memory ledger rows (:695-753).
- Z-DESKTOP-MASTER-SPEC.md (retrieved 2026-08-23): §8.16 Memory
  Architecture (:631-638); §8.21 resource-scheduler latency invariant
  (:686-697); replay-rebuilds promise (:1097); ids-not-blobs rule (:1432);
  §29 event kinds (:1425-1449); §30.1 thread-file tolerance rules
  (:1453-1461); §30.5 memory record sketch (:1481-1488); §40 decision
  index incl. ADR-0004 (:1902); §46.2 data dir with planned `memory/`
  (:2189-2200); §49.11 memory subsystem (:2379-2387); §104 consolidation
  pass design (:3991-4010); glossary "Provisional memory" (:4177).
- docs/adr/0012-subagent-orchestration.md (store-home reasoning template:
  journal-backed view vs sidecar file vs SQLite-now; personal-first scale
  honesty), 0013-context-engine.md (ContextItem/Freshness fingerprint
  convention D4; turn-layer injection point reserved for mem-009;
  mark-don't-mutate staleness stance).
- skills/z-memory/SKILL.md (five layer rules; anti-patterns including the
  silent-TTL ban and chat-is-not-project-memory; storage direction
  "views are caches, the journal is truth"; testing expectations).
