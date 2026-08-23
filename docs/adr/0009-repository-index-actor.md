# ADR-0009: Repository index actor architecture

Ledger: idx-001 (index actor thread + channel API), idx-002 (snapshot read
semantics). Unblocks: idx-012/013 (incremental reparse rides this protocol),
idx-026 (open benchmark depends idx-001), idx-035 (metrics depends idx-001),
and every snapshot consumer downstream: repo-map v2 (idx-023..025),
go-to-def/find-refs (idx-031/032), affected-analysis (idx-015).

## Status

Accepted (2026-08-23). Justification: the shape is already binding — §35.3
names "Index actor (owning thread, channel API, snapshot reads)" as component
1 of Repository Intelligence, §16's threading model reserves a "(planned)
index actor thread", §55.2 makes "channel API; no shared mutable state;
snapshot reads" an M2 acceptance row, §74.E sketches the update flow, and
ADR-0007 decision 3 fixed "single index actor thread (ADR-0001), snapshot-only
reads (idx-002)" with tree-sitter invoked synchronously from that thread.
This ADR fixes the concrete types and protocol those commitments imply. It
adds no dependency: §52's evaluation list is untouched (ropey/notify/keyring
remain open; crossbeam-channel is evaluated and declined below).

## Context

What M2 must satisfy (§55.2 + skills/z-repository-intelligence): channel API;
no shared mutable state; snapshot reads; unchanged file → zero reparse;
1k-file open <2 s (idx-026); one-file incremental <50 ms (idx-027); lookup
<10 ms (idx-028); 100k-file initial index in minutes (idx-029); indexing
never blocks the UI or turn threads; corrupt/missing index degrades to "no
map", never a crash. Readers today and planned: `build_request`'s repo-map
(`map_text(160)`), the ProjectIndexed event counts; then repo-map v2 scoring,
def/ref tools, affected-analysis — all read-shaped, none mutate.

Current v0 violates three of those properties by construction:

- `RepoIndex` lives in `Shared` as `Mutex<Option<RepoIndex>>` (runtime.rs:95):
  shared mutable state, and every `map_text()` read contends on the same lock
  a future writer would hold for a whole rescan.
- `set_project_root` builds the index inline on the command-loop thread
  (runtime.rs:272): opening a large repo freezes the UI and the approval
  pipeline for the length of the walk.
- Nothing rescans after open outside tests: agent-written files stay invisible
  until restart. The (mtime,size) stamp machinery (`file_stamp`, `rescan()`)
  is sound but has no driver.

Constraints carried in from prior decisions: no tokio/async in core
(ADR-0001); blocking I/O on dedicated threads, `std::sync::mpsc` channels,
named threads via `thread::Builder` (skills/z-rust-engineering); actor pattern
"one owner thread + channel; snapshots out"; tree-sitter is synchronous C
called from the actor thread (ADR-0007); per-file `catch_unwind` containment
mandatory (idx-016, §88.4). Personal-first scale honesty: one user, local
disk, repos to ~100k files (idx-029) — throughput needs are "minutes not
hours"; non-blocking guarantees and correctness matter more than parallel parse
throughput.

## Considered options

**(a) Single index actor thread; commands over `std::sync::mpsc`; publish
immutable snapshots via `Mutex<Arc<IndexSnapshot>>`.** All mutable index state
owned by one thread; readers clone an `Arc` and never touch actor internals.
Matches the skill's actor pattern, ADR-0007 decision 3, and every acceptance
row. Chosen.

**(b) Shard pool: N parser threads owning disjoint subtrees.** Cuts initial
wall-clock roughly N× and isolates a pathological file to one shard — but
imports ignore shard boundaries (cross-shard symbol resolution), every
snapshot needs a merge step, and N pending queues replace one coalescing
point. At ~1–3 ms/file single-thread parse, 100k files lands in ~2–5 min:
inside "minutes". Rejected for M2; kept as the pre-committed scale response
(Consequences) reachable without changing the channel contract — shards become
workers under the same inbox, with the actor retaining sole publish authority.

**(c) No actor: `RwLock<Index>` in `Shared`; writers hold the write guard
across rescans.** Satisfies neither "no shared mutable state" nor non-blocking
reads; coalescing/backpressure logic smears across call sites. This is v0's
design generalized. Rejected.

**(d) `crossbeam-channel` bounded inbox + select over receivers.** Adds MPMC
and select — capabilities this design does not use: commands arrive on one
inbox, replies go out on per-command `Sender`s, so there is nothing to select
over. Upstream is healthy (0.5.16, published 2026-07-06; crates.io retrieved
2026-08-23), but it is a dependency in search of a requirement. Declined;
revisit only if a second inbound source class appears that genuinely needs
select.

**(e) Update trigger: watcher-driven now (notify) vs polling fingerprint
rescan for M2.** notify is mid major-version transition at decision time —
max stable 8.2.0 with 9.0.0-rc.4 published 2026-05-02 (crates.io retrieved
2026-08-23) — and §52 still holds it under evaluation (wat-001 owns that).
Polling costs almost nothing to adopt: v0 already fingerprints (mtime,size),
the walk skips dependency dirs, and an unchanged tree does zero parse work.
Chosen: M2 ships poll-triggered rescans (Decision 4); wat-001 later adopts
notify behind the SAME `Reparse` command and idx-036 flips the trigger — no
API change downstream.

**(f) Snapshot delivery variants.** Versioned lease handles (readers pin a
generation; actor retires them) buy old-snapshot bookkeeping nobody needs at
desktop scale — readers drop their `Arc` when done, and `Arc` already keeps a
superseded snapshot alive safely. `arc_swap` removes the publish mutex but is
a dependency to elide a nanosecond-scale lock taken once per read. Both
declined; plain `Mutex<Arc<_>>` plus a u64 version field kept for metrics/
diagnostics only — it must never leak into map output, which keeps idx-025's
byte-stability test trivial.

## Decision

**1. Topology: one owner thread.** Spawned named (`z-index`) when a project
opens; owns ALL mutable index state — file table, fingerprints, symbol/ref/import
graphs, errored-file registry (idx-017), pending queue. No other thread ever
holds `&mut` to any of it. The runtime holds a clonable client handle
(command `Sender` + snapshot getter); turn threads, UI, tools, and the future
watcher talk to the actor only through it. Initial indexing moves off the
command loop: `set_project_root` sends `IndexRoot` and returns immediately;
ProjectIndexed fires when the walk completes; until then `build_request`
renders an empty map (the existing `unwrap_or_default` path).

**2. Channels: `std::sync::mpsc::channel::<IndexCommand>()` inbox;
per-command reply `Sender`s.** Many producers → one consumer is exactly std
mpsc's shape. The inbox is unbounded by design: Decision 5 bounds real memory
via coalescing inside the actor, and bounding the queue would only move the
pile up a layer while adding sync_channel complexity for no reader benefit.

```rust
enum IndexCommand {
    /// Open/supersede a project root: clears pending work, full walk+parse.
    IndexRoot { root: PathBuf },
    /// These paths may have changed; re-fingerprint, parse only diffs.
    /// Batch form absorbs save bursts and (later) watcher event floods.
    Reparse { paths: Vec<PathBuf> },
    /// Full-tree sweep; unchanged fingerprints cost one stat each.
    Rescan,
    /// Latest published snapshot, answered immediately — never queued
    /// behind pending parses.
    Snapshot { reply: mpsc::Sender<Arc<IndexSnapshot>> },
    /// Queue depth, parse/error counters (idx-035).
    Metrics { reply: mpsc::Sender<IndexMetrics> },
    /// Finish the current file only, then exit the thread.
    Shutdown,
}

pub struct IndexSnapshot {
    version: u64,                                // publish counter; diagnostics only
    built_at: SystemTime,
    files: HashMap<Arc<str>, Arc<FileRecord>>,   // rel-path keyed
    // FileRecord: stamp + content hash (idx-011), symbols/references/imports
    // as grammar packs land (idx-007..009). Reverse-reference postings
    // (idx-014) and trigram postings (idx-018..022) join immutably.
}
```

**3. Snapshot semantics (idx-002): publish-on-write, read-by-Arc.** The actor
mutates private working state; after each completed unit of work it publishes:
shallow-clone the map (keys and values are `Arc`, so a clone is pointer
copies, O(file-count) but allocation-free), replace changed entries with fresh
`Arc<FileRecord>`s, bump `version`, swap into `Mutex<Arc<IndexSnapshot>>`.
Readers: lock, clone the `Arc`, unlock — nanosecond-scale, never blocked by a
parse, and every holder sees ONE coherent generation for as long as it keeps
the `Arc` (RCU discipline; §74.E's "latest snapshot, never blocks on parse").
Lookup = one fetch + hash lookup, meeting idx-028's <10 ms with margin.
Shallow cloning keeps publish cost proportional to what changed, protecting
idx-027's <50 ms at 100k entries. The index stays a CACHE: snapshots serve
navigation/retrieval; edits always rehydrate real bytes
(skills/z-repository-intelligence invariant 1).

**4. Incremental update protocol for M2: poll-triggered, fingerprint-gated.**
The actor parses only when a fingerprint differs. Triggers, all funneling into
the commands above:

- `IndexRoot`: project open/switch — full walk, parse everything unknown.
- `Rescan`: issued by the runtime after each completed turn (agent writes just
  landed), on window re-focus, and from an explicit refresh action.
  Metadata walk only.
- `Reparse{paths}`: targeted — write-tool completions can name exact paths.

Actor-side gate regardless of trigger: re-stat each path, compare with the
stored stamp, equal → drop (this IS the "unchanged → zero reparse" assertion,
§55.2). idx-011 later adds a content hash (fnv1a64 convention, §89.1) beside
the cheap stamp pre-filter, closing the same-mtime-different-content case the
testing skill requires covered. wat-001/notify (idx-036) becomes a fourth
trigger emitting `Reparse` batches; consumers cannot tell the difference.

**5. Backpressure, coalescing, cancellation, shutdown.**

- Pending set: queued-but-unparsed paths live in an ordered set inside the
  actor. A duplicate `Reparse` for an already-queued or in-flight path
  collapses into the existing entry — a save storm over one file costs one
  parse. Memory bound = one key per repo file, not event count; this is why
  the unbounded inbox is safe.
- Superseding: `IndexRoot` clears pending work and starts fresh (project
  switch); stale entries for the old root are discarded by construction.
- Cancellation: no per-job cancellation in M2. Units are milliseconds; the
  meaningful cancel points are supersede (above) and shutdown. Turn
  cancellation never touches the index — it is a cache; staleness is benign.
- Shutdown ordering: runtime stops issuing commands → sends `Shutdown` →
  joins the thread at app exit. The actor finishes the CURRENT file only
  (≤ one parse latency), drains nothing else, exits. Published snapshots stay
  valid — they are `Arc`s that outlive the actor. No persistence at M2:
  cold start reindexes within idx-026's budget; persistence is deferred until
  open benchmarks say otherwise.

**6. Failure containment placement (idx-016): the catch lives where the parse
runs — inside the actor loop, around exactly one file.** Per file:
`catch_unwind(AssertUnwindSafe(|| parse_and_extract(path)))` → `Ok(records)`
feeds publish; `Err(_)` records the path in the errored registry with a reason
(idx-017) and the loop proceeds. Tree-sitter is foreign C: a panic must not
propagate past the loop and kill the queue (§88.4). If option (b)'s workers
ever take over parsing, the invariant travels with the execution site — each
worker catches per file before sending results over the channel. The rule:
containment wraps the smallest unit that invokes foreign C; it is not tied to
the actor as such.

## Consequences

**Non-blocking by construction**: UI/turn threads never wait on a parse —
worst case is a mutex held for an `Arc` clone plus whatever the latest
snapshot shows (possibly empty, possibly one generation old). The command-loop
freeze at project open disappears.

**Single-thread parse throughput is the accepted ceiling**: ~2–5 min for 100k
files at 1–3 ms/file, to be measured by idx-029, not assumed. Pre-committed
response if it breaches "minutes": spawn K parser workers under the same
inbox, shard by path, catch per file, ship finished `FileRecord`s to the
actor, which alone publishes snapshots. Channel API, snapshot type, and all
readers unchanged (mirrors §54's evolution discipline).

**Burst storms degrade gracefully**: watcher floods (idx-036) and rapid saves
coalesce to one pending entry per file. Metrics (idx-035) expose queue depth
and coalesce rate so the bound stays observable rather than assumed.

**Staleness window is honest**: between a write and the next trigger the map
is one generation old, by design. Correctness-critical paths re-read files
(invariant 1); the index only shapes navigation and retrieval.

**No new dependencies**: std threads + std mpsc + std Mutex/Arc only.
crossbeam-channel and arc-swap evaluated and declined above so the question
stays closed; notify remains §52-open until wat-001 evaluates it against the
9.0 release transition.

**Testing obligations locked in**: actor isolation proven by construction (no
shared `&mut` escapes the thread); incremental parity vs full rebuild
(idx-013); unchanged→zero-parse assertion (§55.2); snapshot coherence test (a
reader holding a snapshot across a publish sees one consistent generation);
shutdown-drain test; panic-containment fixture feeding a panicking grammar
path (idx-016).

## Sources

- Repo inspection (2026-08-23): `z desktop/crates/z-core/src/repo.rs` —
  (mtime,size) stamping (`file_stamp`), incremental `rescan()`,
  `extract_symbols()` heuristics retained as fallback, `map_text()`;
  `z desktop/crates/z-core/src/runtime.rs` — `Shared.index:
  Mutex<Option<RepoIndex>>` (:95), inline `RepoIndex::open` on the command
  loop (:272), `map_text(160)` consumed in `build_request` (:620).
- docs/Z-DESKTOP-TASKS.md (retrieved 2026-08-23): idx-001..040 ledger,
  dependencies cited above; wat-001 "notify-based watcher service" PLANNED.
- Z-DESKTOP-MASTER-SPEC.md: §8.10 (:561-572), §16 threading model (:1280-82),
  §35.3 (:1651-1669), §52 dependency policy (:2459-2471), §54 architecture
  evolution (:2489-2504), §55.2 M2 acceptance (:2525-2536), §74.E index
  update flow (:3135-3142), §88 detailed design incl. §88.4 containment
  (:3558-3596).
- skills/z-rust-engineering/SKILL.md (concurrency model, actor pattern,
  naming/testing discipline); skills/z-repository-intelligence/SKILL.md
  (target architecture, invariants, scale targets, testing expectations).
- crates.io API (retrieved 2026-08-23): notify max_stable_version 8.2.0 with
  9.0.0-rc.4 published 2026-05-02; crossbeam-channel 0.5.16 published
  2026-07-06.
- Rust std library semantics: `std::sync::mpsc` multi-producer/single-consumer
  channels with clonable senders; `Mutex<Arc<T>>` publish pattern (stable std
  API surface).
