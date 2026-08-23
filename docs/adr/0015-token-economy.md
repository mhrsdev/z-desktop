# ADR-0015: Token economy (prefix stability & output caching)

Ledger: decides the concrete mechanics behind tok-001 (byte-exact prefix
guard) and tok-002 (prefix change impact documentation), tok-003..005
(tool-result cache: structure, hit path, fingerprint-mismatch invalidation),
and fixes tok-020's detection basis (redundant reads ride the cache). It also
sets the one constraint tok-006/007 (lazy tool advertisement) must respect to
coexist with prompt caching. Unblocks downstream: tok-021 (repo-map cache —
same key scheme), tok-017 (`cache_hit_rate` measures what this ADR builds),
and closes the deferral ADR-0013 D6 made explicitly ("tool-result caching
keyed by fingerprints … separate mechanism"). Out of scope here, by name:
usage extraction and calibration (tok-010..014), structured summaries
(tok-008/009), the context delta protocol (tok-018), cost dashboards
(tok-015/016).

## Status

Accepted (2026-08-23). Justification: §8.14 marks Token Economy PARTIAL —
"estimator + budgeting implemented; caching layers, lazy tools, structured
outputs PLANNED" — and prescribes the ladder ("don't-send → cache-hit →
send-less → compress-representation") with "byte-stable prefixes" as the
cache-hit rung. §8.15 defines the cache contract this ADR implements for its
first row class ("Tool results | tool + args + input fingerprints | any input
change"; "Prompt prefix | byte-exact prefix | any prefix edit (expensive!)")
under the principle "caches accelerate; they never decide". §35.2 lists the
components verbatim ("prompt-prefix stability guard", "tool-result cache with
fingerprint keys") and names the failure modes this design forecloses ("stale
cache serving wrong content (fingerprint discipline)"). §55.7 makes the guard
and the hit path M7 acceptance criteria ("byte-identical prefix test in CI";
"fingerprint-keyed; hit path verified end-to-end").
skills/z-token-economy supplies the absolute rule (accuracy is never traded
for tokens) and the key formula ("(tool, args, relevant-input-fingerprints)").
This ADR adds no dependency (§52 untouched): everything is std collections
plus the existing fnv1a64 machinery.

## Context

**What exists.** The estimator (tokens.rs:15–47, chars/4 + CJK + code
correction) and the budget walk (runtime.rs:800–801 limits, 891–904 fixed
cost then history budget) are done. `build_request` (runtime.rs:862–952)
already assembles every request as: one system message, then rendered
history, plus a top-level `tools` array. The system string is identity +
rules + project root + repo map + active-model label (runtime.rs:877–888);
the map comes from the index snapshot via `map_text(160)` (runtime.rs:869–875)
and rescans coalesce per ADR-0009. Both providers serialize the same tools
array into the wire body (provider.rs:153–160 OpenAI, 287–292 Anthropic), so
system message **and** tool definitions jointly form the provider-visible
prompt head. Nothing volatile (timestamp, counter, random id) appears in
either — stability holds today by construction but is asserted nowhere.

**The economics.** Provider prompt caches key on an exact byte prefix of the
request; any changed byte invalidates the whole cached span (§8.14;
skills/z-token-economy "treat prefix edits as expensive operations"). One
external fact anchors sizing (docs.claude.com prompt-caching docs, retrieved
2026-08-23): minimum cacheable prompt length is model-dependent — 512 to
4,096 tokens across current Claude models — and sub-minimum prompts are
*silently* processed uncached, no error. Z's fixed head comfortably clears
the floor (repo map capped at 160 lines plus a closed tool-definition set,
typically ≫ 1k estimated tokens by tokens.rs), which means the discipline's
payoff is available on every turn — provided bytes stay equal.

**The waste.** Tool results are bounded at `MAX_OUTPUT_CHARS = 12_000`
characters (tools.rs:21, `bound` :23–31) and enter history through
`StoredToolCall.summary` carriers. When the model re-reads a file it already
read this turn — common during multi-file edits — the full text re-enters
context a second time at full price, while sitting verbatim in the earlier
carrier of the *same* request. Meanwhile the safe-edit pipeline already
fingerprints every file read: `fs_read` computes fnv1a64 over the bytes and
records it per thread+path (tools.rs:287–308 → fingerprint.rs:60–62), and
`fs_write` refuses stale writes against that record (edit-002/003,
tools.rs:382–411). The infrastructure to notice "this exact content was
already seen" exists and is unused by the token economy.

Constraints inherited: blocking threads, no async in core (ADR-0001); caches
never authority (ADR-0009 Consequences; §8.15); accuracy absolute
(z-token-economy); personal-first scale (one user, one open project);
`fs_read` refuses files > 512 KiB (tools.rs:291–293), so any cached entry is
small and bounded by construction.

## Considered options

**(a) Guard test asserts two consecutive `build_request` calls produce
byte-identical system string + serialized tools.** Direct, dependency-free,
fails on any accidental volatility, and matches §55.7's criterion verbatim.
Chosen (D1).

**(b) Hash-based guard (compare fnv1a64 of prefix instead of raw bytes).**
One collision-shaped blind spot for zero benefit — the strings are already in
memory; comparing `Vec<u8>` costs nothing extra. Rejected: compare bytes.

**(c) Snapshot the prefix to disk and diff across process restarts.** Catches
cross-restart drift, but cross-restart equality is meaningless for provider
caches (the cache is cold anyway after a restart gap) and drags in file I/O
and state location decisions. Rejected; in-process double-build is the whole
contract (D5 boundary list governs the rest).

**(d) Tool cache persisted to disk under `data_dir/` so hits survive
restart.** Restart-cold is fine at personal scale (first re-read repopulates),
persistence adds staleness surfaces (disk entries vs changed files) for zero
accuracy benefit — the fingerprint check at call time is what guarantees
correctness, not storage. Rejected; in-process only (D3).

**(e) Cache keyed by mtime+size instead of content fingerprint.** Cheaper
lookup, but edit-001 exists precisely because mtime-equal content changes
defeat stat-based keys, and §8.15 mandates input fingerprints. Rejected:
fingerprint is the only honest key and we must stream the file to compute it
anyway (see D4 for the double-read honesty note).

**(f) Generalize the cache to all read-only tools immediately**
(fs_search, git_diff, git_log). No cheap honest fingerprint source exists for
search corpora or git refs; wrong hits would violate the absolute rule.
Rejected: fs_read only until tok-021 gives the repo map (and successors give
other tools) real fingerprint inputs (D6).

**(g) Redundant-read short-circuit inside `tools::execute`.** The tool layer
doesn't know turn boundaries, so "twice *in a turn*" is undecidable there.
The runtime turn loop owns turn_id (it already threads it through events,
runtime.rs:703–726). Rejected; detection lives behind a `turn_id` plumbed
into `ToolInvocation` (D5).

**(h) Serve duplicate reads from cache with the full text again.** Saves
nothing token-wise — the point of tok-020 is that the second copy never
re-enters context. Rejected; duplicates get a pointer, not a re-paste (D5).

## Decision

### D1 — Stable prefix definition & byte-exact guard (tok-001)

The stable prefix is exactly:

1. the system string built at runtime.rs:877–888 (identity/rules block,
   `Project root: {root}` line, `Repository map:\n{repo_map}`, `Active
   model: {label}`);
2. the tool-definition array returned by `tools::definitions()` in its
   existing order, serialized into the request body (runtime.rs:949;
   provider.rs:153–160 / 287–292);
3. their mutual order — system message first, tools alongside — ending at
   the first non-system message.

The guard test (lives beside the existing `budget_tests` module pattern,
runtime.rs:954): construct a `Shared` + `Thread` fixture, call `build_request`
twice, assert `system` bytes equal and `serde_json::to_vec(&request.tools)`
bytes equal. A control case mutates one input on a declared boundary (below)
and asserts the bytes *do* diverge — proving the test can fail. Volatility is
barred structurally: the format! has no clock/counter argument today, and the
test keeps it that way. This is the whole tok-001 deliverable: one test file
section, no production code.

### D2 — Declared boundaries & impact documentation (tok-002)

The prefix changes only at these boundaries, each a deliberate whole-prefix
invalidation accepted because they are rare bursts, not per-turn drift:

| Boundary | Input touched | Already coalesced/guarded by |
|---|---|---|
| Project switch | root line (runtime.rs:881) | user-initiated command |
| Provider switch | label line (runtime.rs:886) | user-initiated command |
| Completed reindex | repo map (runtime.rs:869–875) | rescan coalescing (ADR-0009) |
| Tool-set change | tools array (incl. future tok-006 category tiers) | settings/mode switch |

tok-002 ships this table as documentation; it is normative for review: any PR
touching these inputs without a declared-boundary rationale fails the guard
test's spirit even if bytes happen to match that day. **Constraint on
tok-006/007**: lazy tool advertisement may select categories only at session
start or explicit mode switch — never per-turn — or it destroys the cache
economics tok-001 protects. Per-turn tool-set churn is forbidden.

### D3 — Tool-output cache structure (tok-003)

New module `z-core/src/tool_cache.rs`, sibling of fingerprint.rs, same
process-local pattern (OnceLock<Mutex<…>>, fingerprint.rs:52–57):

```rust
/// Keyed by canonical path + fnv1a64 of the file bytes at last serve.
struct Entry { fp: u64, text: String }        // text is bound()-sized ≤ 12k chars
// HashMap<String, Entry> + VecDeque<String> eviction queue
const MAX_ENTRIES: usize = 256;               // ≈ 3 MiB worst case; evict oldest-inserted
```

Scope: `fs_read` only (option f). Key includes the fingerprint itself, so
"invalidation" is structural — a changed file lands on a different key and no
delete logic, watcher, or mtime trust exists anywhere. Old keys age out via
the cap. In-process only, never persisted (option d). Eviction is naive
oldest-inserted; ponytail: upgrade to LRU if telemetry ever shows thrash past
256 live files in a session. Cache stores output text, never decides
anything: the caller still fingerprints current bytes every call (D4) —
"caches accelerate; they never decide" (§8.15).

### D4 — Hit path & mismatch behavior (tok-004/tok-005)

`fs_read` becomes: scope path (tools.rs:289) → size guard (:291) →
`file_fingerprint(path)` → lookup `(path, fp)`:

- **Hit**: fp matches a stored entry ⇒ serve `entry.text`; skip
  `read_to_string`. Record the fingerprint for edit-002 exactly as today
  (tools.rs:297–301) — the write-refusal pipeline is untouched.
- **Miss** (no entry, or fp differs = tok-005's mismatch case): read, bind,
  insert under `(path, fp)`. Mismatch needs no separate branch — it *is* a
  miss onto a new key.

Honesty notes. (1) On a miss we stream the file twice (fingerprint pass +
read); files are ≤ 512 KiB and misses are the minority path — accepted, and
it is the price of the only key that cannot lie (option e). (2) Local I/O
savings are ~zero either way; **this cache is a token-economy mechanism**, its
real payoff is D5 (duplicates never re-enter context) and tok-021 (the repo
map gains the same key scheme). Stale serving is impossible by construction:
a hit requires a fingerprint computed from the current bytes microseconds
earlier — the accuracy-absolute rule is satisfied mechanically, which is what
tok-004's end-to-end test proves: create → read → assert single entry →
mutate file → read again → assert fresh correct content and a second key.

### D5 — Redundant-read short-circuit (tok-020)

Plumb `turn_id: &'a str` into `ToolInvocation` (additive next to thread_id,
tools.rs:34–41; empty string in tests that don't care, same convention).
The runtime turn loop already owns turn_id (runtime.rs:703–726). The cache
keeps a per-turn seen set `(thread_id, turn_id, path, fp)`; when an fs_read
hit repeats within the same turn, serve a pointer instead of the body:

```
[duplicate read: <path> unchanged this turn (fp 0x…); the full content is in
the earlier tool result above]
```

Safety argument: within a live turn the original carrier cannot have been
trimmed — `trim_history` cuts only at clean user-message boundaries from the
front (runtime.rs:825–856) and the running turn is always the newest suffix
(the no-clean-boundary fallback sends untrimmed rather than lose validity).
Next-turn re-reads get the full body again (per-turn scope); different
arguments (future range/offset params) form a different key and bypass the
short-circuit. Test: fabricate one turn, read the same file twice, assert the
second output starts with the marker and contains none of the body.

### D6 — Out of scope

Semantic/embedding caches (mem track); cross-session or disk-persisted tool
caches (restart = cold, correctness-neutral); provider-side caching APIs —
`cache_control` breakpoints, TTL selection, pre-warming: automatic caching
applies prefix discipline for free and tok-017 will *measure* hit rate from
usage fields rather than manage breakpoints; context delta protocol
(tok-018 RESEARCH); usage extraction/calibration (tok-010..014 — the
estimator's ±10% claim is calibrated elsewhere); structured large-output
summaries (tok-008/009); lazy-tool mechanics beyond the D2 constraint
(tok-006/007 remain PLANNED with their own decision when reached); caches for
non-fingerprintable tools (option f).

## Consequences

**Immediate**: tok-001 is one test, zero production lines; tok-002 is one doc
page rendered from D2; tok-003..005 are one ~100-line module, a three-line
branch in `fs_read`, and three tests; tok-020 is the `turn_id` field plus one
branch and its test. Nothing else moves; the estimator, budgets, trim, and
edit pipeline are untouched (edit-002 recording preserved verbatim on the
hit path).

**Prefix economics become real**: with bytes stable across turns and history
append-only between trims, provider automatic caching converts repeat turns
into discount cache reads; declared boundaries are the only full-price
moments. The external minimums (512–4,096 tokens, silent below) are cleared
by Z's head on every realistic configuration — worth re-checking only if the
prefix ever shrinks by an order of magnitude.

**Duplicate content stops paying twice**: tok-020 turns the common
re-read-during-edit pattern from 2×N chars of context into N + ~30. Combined
with `bound()`'s existing cap, worst-case per-file context cost is paid once
per turn.

**Accepted debt**: double stream on cache miss (bounded by the 512 KiB
guard); oldest-inserted eviction (ponytail-flagged LRU upgrade); per-process
cold start; TOCTOU window between fingerprint and serve identical in shape to
today's meta-then-read window (tools.rs:290–294). None of these can serve
wrong content; all are latency/memory bounds, not accuracy risks.

**Testing obligations locked in**: byte-equal double-build guard + can-fail
control (tok-001); hit/miss/invalidate triple against a temp fixture
(tok-004/005); duplicate-read marker test (tok-020); eviction-cap unit test
(MAX_ENTRIES honored); guard test runs in CI per §55.7.

## Sources

- Repo inspection (2026-08-23): `z desktop/crates/z-core/src/runtime.rs` —
  `Shared` state (:93–113), `record_result` turn_id flow (:703–726), budget
  consts (:800–801), `trim_history` clean-boundary walk (:825–856),
  `build_request` system/map/label assembly (:862–901), tools serialization
  (:949), `budget_tests` pattern (:954+); `crates/z-core/src/tools.rs` —
  `MAX_OUTPUT_CHARS` + `bound` (:20–31), `ToolInvocation` (:34–41),
  `fs_read` size guard/read/fingerprint-record (:287–308), write-side
  take/refuse/re-record (:382–411); `crates/z-core/src/fingerprint.rs` —
  fnv1a64 (:16–23), streaming file_fingerprint (:27–43), registry pattern
  (:52–68); `crates/z-core/src/tokens.rs` — estimate (:15–47),
  estimate_tool_def (:66–68); `crates/z-core/src/provider.rs` — tools into
  wire bodies (:153–160, :287–292).
- Z-DESKTOP-MASTER-SPEC.md (retrieved 2026-08-23): §8.14 Token Economy
  (:606–612), §8.15 Cache Architecture table + principle (:615–627), §35.2
  component list & failure modes (:1632–1647), §55.7 M7 acceptance criteria
  (:2579–2586).
- docs/Z-DESKTOP-TASKS.md (retrieved 2026-08-23): tok-001..006 ledger
  (:618–637), tok-020 dependency on tok-003 (:675–676), tok-021 (:678–679).
- skills/z-token-economy/SKILL.md: accuracy-absolute rule; optimization
  ladder; byte-stability rules; "(tool, args, relevant-input-fingerprints)"
  key formula.
- docs/adr/0009 (rescan coalescing; cache-never-authority), 0013 (D5 prefix
  contract this ADR operationalizes; D6 deferral of tok-003..005 resolved
  here).
- Web (one fact, cited): docs.claude.com/en/docs/build-with-claude/
  prompt-caching, retrieved 2026-08-23 — minimum cacheable prompt length is
  model-dependent (512–4,096 tokens across current Claude models); shorter
  prompts are processed uncached without error; breakpoint blocks must stay
  identical across requests or the prefix hash never matches.
