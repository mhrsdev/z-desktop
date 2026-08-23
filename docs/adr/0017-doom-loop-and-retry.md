# ADR-0017: Doom-loop breaker & provider retry policy

Ledger: fixes the detection, firing, and retry contracts behind core-013
(doom-loop breaker), core-014 (per-turn fingerprint counter), core-015
(threshold setting), core-016 (retry classification), core-017
(Retry-After), core-018 (byte-identical retry payloads), and core-019
(retry journaling); subsumes core-033's args-hashing utility and fixes
the seam prov-001 (retry/backoff module) plugs into. Unblocks directly:
jour-025 (provider_called records with attempt field), and transitively
prov-001 and the M6 "honest completion" milestone item.

## Status

Accepted (2026-08-23). Justification: §17's failure table currently
promises "Provider timeout/error | one retry (round 0), then turn fails"
(:1067), and today's implementation is a string sniff plus a loop
`continue` (runtime.rs:550) that silently burns one of 24 tool rounds
and rebuilds the whole request. §M6 puts the doom-loop breaker on the
honest-completion milestone (:1504), the provider contract binds retry
policy to the runtime layer — "Must not retry internally (retry policy
lives in runtime)" (:1293) — and §8.18 requires a failed provider call
to never lose the user's message. Nothing in the ledger pins down what
"classify", "honor Retry-After", or "byte-identical" concretely mean
against a provider layer whose entire error surface is a `String`
(provider.rs:76). This ADR makes those commitments implementable
without a second decision round and adds no dependency (§52 untouched).

## Context

The turn loop and its single provider call site:

- `run_turn` iterates `for round in 0..max_tool_rounds`
  (runtime.rs:508) with settings snapshotted once at turn start
  (:503–506, ADR-0011 D1.3 discipline). Each round calls
  `build_request` once (:533) and `provider.stream` once (:536) — the
  only `stream` call site in the crate.
- The entire retry policy is runtime.rs:548–552: on
  `Err(e)`, `if round == 0 && e.contains("stream read failed") {
  continue; }`. The `continue` re-enters the round loop, i.e. it
  retries by spending another round, re-running `build_request`
  (repo map re-render, budget trim re-walk), and re-streaming. It fires
  only on round 0, only for one substring, and classifies nothing.
  Every other error persists the thread and fails the turn (:553–556).
- Tool rounds have zero repetition protection. A model that repeats
  `fs_read` with byte-identical arguments — the classic doom loop —
  runs until `max_tool_rounds` exhausts and the turn ends with
  "stopped after N tool rounds" (:701). Each repeat pays full context
  price for a result the model already holds.

What the provider layer actually returns (all errors are `String`,
provider.rs:72–77):

| Shape | Origin | Example |
|---|---|---|
| `stream read failed: {e}` | mid-stream I/O (read_sse, :86) | connection dropped during SSE |
| `request failed: {other}` | transport, pre-response (ureq mapping :165–171, :299–305) | DNS, connect, TLS, timeout |
| `provider returned HTTP {status}: {snippet}` | any non-2xx status (`http_error`, :95–99) | 429, 401, 500, 400 — collapsed into prose |

HTTP statuses are flattened into text; headers are consumed and
discarded (`resp.into_string()` only), so `Retry-After` is already
gone by the time the runtime sees anything. Wire bodies are pure
functions of `ChatRequest` — model/messages/stream/max_tokens/tools
only (:147–152, :278–292), no timestamps or nonces.

Available building blocks: `fingerprint::fnv1a64` (fingerprint.rs:16–23);
serde_json without the `preserve_order` feature (workspace Cargo.toml:42)
serializes objects through `BTreeMap`, so parse→re-serialize canonicalizes
key order for free; the settings recipe for one more scalar is
settings.rs:20–28 (consts/bounds) + :78–97 (load) + :104–114 (store);
`Runtime::journal_record` (:314) accepts arbitrary JSON payloads and
`JournalKind::Other(s)` (journal.rs:63) carries custom kinds additively.

Constraints inherited: blocking threads, no async (ADR-0001) — a retry
backoff is a `thread::sleep` on the turn worker, by design; retry lives
in the runtime, never in adapters (§ spec provider contract :1293);
snapshot reads for per-turn settings (ADR-0011); prefix-stability is an
economic invariant, not cosmetics (ADR-0015 D2/D3: provider prompt
caching discounts byte-exact prefixes; tok-001 guards it byte-exactly).

## Considered options

**(a) Breaker fires by failing the turn at threshold N.** Cheapest and
protects tokens, but the fingerprint (name + args, below) has known
benign collisions: `fs_read(path)` between successive edits repeats
identical arguments while results legitimately differ, because the
agent itself changed the file. Read-modify-write cycles are the most
productive pattern we have; killing one at the third strike trades a
real workflow for a heuristic. Rejected as the sole response.

**(b) Breaker injects a steering note only.** Never hard-stops a
genuine loop; a model stubborn enough to repeat 3× can repeat to
round 24 forever, paying full context price each round. Rejected as
the sole response.

**(c) Escalation ladder — steer at N, fail at N+2.** The note is the
cheap recovery attempt (models reliably course-correct when told
explicitly they are repeating); the hard stop bounds the damage when
they don't. Worst-case waste per stuck turn: 2 extra rounds past the
note, still capped by `max_tool_rounds`. Chosen.

**(d) Fingerprint includes the previous tool-result digest**
(name + args + hash of last output). Kills the benign read-after-write
collision outright, but breaks the other direction: tool outputs are
routinely volatile (line numbers, timestamps, diffs, command banners),
so genuinely stuck loops stop matching and the detector misses the
cases it exists for. Name + args is stable; option (c) absorbs its
false positives at the cost of one note. Rejected; revisit trigger
recorded in Consequences.

**(e) Structured error type on `Provider::stream`** (enum with status,
class, headers) instead of `String`. The correct end state, but it
breaks the trait, both adapters, and every test that constructs errors
— a cross-cutting refactor this ADR doesn't need to ship its value.
Rejected for now; D2 is written so the swap is mechanical.

**(f) Parse `Retry-After` out of the error-string body snippet.**
Bodies are provider-specific JSON/HTML prose truncated at 400 chars;
some providers echo a retry hint, some don't, and the header was
dropped at provider.rs:95–99 regardless. Unreliable by construction.
Rejected.

**(g) Plumb response headers through `http_error` into the string.**
Keeps `String` but encodes structured data in prose — a second
parsing contract layered on the first, touching both adapters' error
paths for one header we've decided (below) not to honor yet. Rejected.

**(h) Retry by calling `build_request` again per attempt** (today's
`continue`). Wasteful (repo map re-render, full trim walk) and unsafe
economically: nothing guarantees byte-identical output across builds,
and a drifted prefix forfeits the provider's cache hit — ADR-0015's
whole point. Replaced by D4.

**(i) Reuse the already-built `ChatRequest` across attempts.** Adapter
serialization is a pure function of the request, so the retried wire
body is byte-identical by construction. Chosen (D4).

## Decision

### D1 — Doom-loop breaker: per-turn counter, escalate steer→fail (core-013/014)

1. **Counter**: a `HashMap<u64, u32>` local to `run_turn`, created at
   turn start beside the settings snapshot (:505). Per-turn lifetime
   means it dies with the turn — no `Shared` field, no persistence, no
   reset logic. core-014 is this map and nothing more.
2. **Fingerprint**: `fnv1a64(tool_name_bytes ++ 0x00 ++
   canonical_args_bytes)` where `canonical_args` re-serializes the
   `arguments_json` through `serde_json::Value` — the parse already
   happens at runtime.rs:599–600, and BTreeMap-backed serialization
   makes key order irrelevant. Incremented per call, inside the
   per-call loop (:593), before execution, covering all calls across
   all rounds. This function *is* core-033's "args hashing utility":
   two lines, landed where used, extracted if a second caller appears.
3. **Fire — steer at N**: when a fingerprint's count reaches
   `settings.doom_threshold` (D6), push one steering-style user
   message (same mechanics as the drain path :519–531):
   `[doom-loop guard] You have now issued <tool> with identical
   arguments <N> times in this turn with no progress. Do not repeat
   it; take a different approach or say plainly why you cannot.` Then
   persist and continue the turn.
4. **Fire — fail at N+2**: if the same fingerprint reaches N+2, fail
   the turn with an actionable message ("stopped: repeated <tool>
   with identical arguments <N+2> times — change approach before
   retrying"), persisting first exactly as the other exit paths do.
   The user's message survives either way (§8.18).
5. Scope honesty: the counter sees executed tool calls, so it catches
   tool-level doom loops (reads, patches, commands). It does not model
   provider-round loops; `max_tool_rounds` remains the outer bound.

### D2 — Retry classification: one pure function over today's strings (core-016)

`pub fn classify(err: &str) -> RetryClass` in z-core's provider module,
with `enum RetryClass { Transport, RateLimited, ServerError, Auth,
Fatal }`. Rules against the three shapes provider.rs emits:

| Predicate | Class | Auto-retry |
|---|---|---|
| starts with `"stream read failed"` | Transport | yes |
| starts with `"request failed"` | Transport | yes |
| `"provider returned HTTP 429"` | RateLimited | yes |
| `"provider returned HTTP 5xx"` | ServerError | yes |
| `"provider returned HTTP 401"` / `403` | Auth | **no** |
| anything else (incl. other 4xx) | Fatal | **no** |

Rationale: transport and 5xx failures are the same request hitting a
transient wall; 429 is the server pricing concurrency; a 401 with the
same key will fail identically forever — retrying auth errors only
adds latency to an error only the user can fix.

Wiring: the runtime error branch (:547–556) becomes: classify; if the
class auto-retries, re-attempt `provider.stream` **within the same
round**, up to 3 attempts total (initial + 2), backing off
`1s × 2^attempt` capped at 30 s via `thread::sleep` (worker-thread
blocking is the house style, ADR-0001); exhaustion falls through to
persist + fail as today. This replaces the `round == 0 &&
contains("stream read failed")` special case and deliberately allows
retries at any round, not just round 0 — mid-turn transient blips
shouldn't kill an otherwise healthy 10-round turn. Classification has
exactly one home (the ADR-0011 D2.3 "failure seam" discipline):
prov-001 and prov-007 attach here or nowhere.
ponytail: string sniffing is load-bearing until `Provider` returns
structured errors (option e); when that refactor happens, only
`classify`'s body changes and its call sites don't notice.

### D3 — Retry-After: acknowledged, not honored yet (core-017)

Headers are destroyed inside the adapters (provider.rs:95–99 consume
only the body); honoring `Retry-After` (RFC 9110 §10.2.3:
delay-seconds or HTTP-date; mandatory-ish on 503, permitted on 429 per
RFC 6585 §4) requires the structured-error refactor deferred in D2.
Interim decision: 429 rides D2's fixed backoff schedule; server hints
are ignored in both directions — we may retry earlier than asked
(bounded attempts mean we give up rather than hammer) and later than
needed (costs latency only). core-017 closes when headers become part
of the provider error type; until then it inherits this ADR's
classification plumbing, which is why it depends on core-016 in the
ledger.

### D4 — Byte-identical retry payloads: reuse the built request (core-018)

A retry re-attempts `provider.stream(&request, …)` with the exact
`ChatRequest` built at :533 for that round; `build_request` is never
re-entered for an attempt. Because both adapters serialize purely from
the request (:120–160, :244–293), the retried HTTP body is
byte-identical to the failed one — which is precisely what keeps the
provider-side prompt-prefix cache warm across the retry (ADR-0015
D2/D3: byte-exact prefixes convert repeat turns into discounted cache
reads; tok-001 will guard this byte-exactly). Reuse also skips the
repo-map re-render and trim re-walk, making the lazy path and the
correct path the same path. Known artifact, accepted: text deltas
already forwarded from a dead attempt (:536–544) are not un-sent, so a
retried stream re-emits overlapping text until the final persisted
snapshot renders authoritatively — identical to today's behavior at
:550, noted as future `StreamReset` event territory, out of scope.

### D5 — Retry journaling: one record per attempt (core-019)

Each attempt — success or failure — appends one journal record via
`Runtime::journal_record` (:314) using `JournalKind::Other(
"provider_called")` until jour-025 promotes a real kind (additive
evolution is the journal's stated design, journal.rs:34). Payload:
`{ thread_id, provider: describe(), attempt, class?, ok, error?
(≤200 chars) }`. Replay then shows retry storms directly, feeding
jour-025's attempt field and jour-028's error records without schema
breakage. Doom-loop events ride the existing message/journal flow via
the steering message (D1.3) and the turn-failure summary (D1.4); no
new kind.

### D6 — Threshold setting (core-015)

`settings.doom_threshold: u32`, default **3**, validated range 2..=20
(2 = aggressive; above 20 the breaker fires after `max_tool_rounds`
anyway). Added by the standard recipe — const defaults and bounds
(settings.rs:20–28 pattern), one typed load block (:78–97 pattern),
one key in `store()` (:104–114). Snapshotted at turn start like its
siblings, so a concurrent change applies next turn. Retry constants
(attempts, backoff) stay consts: no setting for values with no
requested variation.

## Consequences

**Immediate shape of the work**: core-014 ≈ a local HashMap plus a
two-line hash helper; core-013 ≈ two branches on the counter;
core-015 ≈ the settings recipe verbatim; core-016 ≈ one function plus
table tests over the literal error strings; core-017 adds zero code
beyond D2 (it is a documented deferral); core-018 replaces the
`continue` at :550 with an inner attempt loop that reuses `&request`;
core-019 ≈ one `journal_record` call wrapped around the attempt loop.
No protocol, trait, or dependency changes.

**Supersession**: §17's failure-table row "one retry (round 0)"
(:1067) is superseded by D2 (classified retry, any round, ≤3 attempts,
backoff); the spec row should be regenerated accordingly at the next
spec pass. Everything else in §17 stands, including
user-message-preservation.

**Accepted debt**: (1) string-based classification misclassifies if an
adapter ever rewords its error prefixes — contained by keeping all
shapes in one function and one table (D2); (2) `Retry-After` ignored
until structured errors (D3); (3) duplicated streamed text on
mid-stream retries (D4); (4) name+args fingerprints flag legitimate
read-after-write cycles — bounded to one steering note by the N/N+2
ladder (D1); (5) backoff sleeps block the turn worker — inherent to
the blocking design, invisible at personal scale.

**Revisit triggers**: adapters diverge in error richness or a second
consumer needs statuses/headers → structured provider error type
(successor ADR; D2/D3 swap mechanically); logged false-positive rate
of the breaker becomes annoying → add previous-result digest to the
fingerprint (option d) behind the same counter (core-033 extension);
router failover ships (prov-007) → `RetryClass` becomes the input to
chain evaluation at the same single site.

## Sources

- Repo inspection (2026-08-23): `z desktop/crates/z-core/src/runtime.rs`
  — settings snapshot (:503–506), round loop (:508), per-round
  build_request/stream (:533–536, sole stream call site), crude retry +
  fail paths (:547–556), tool-call loop and args parse (:593–601),
  execute/result recording (:669–682), round-exhaustion stop (:701),
  `journal_record` (:314), `trim_history`/`build_request` (:840/:877);
  `crates/z-core/src/provider.rs` — `Result<_, String>` trait
  (:72–77), `read_sse` error shape (:86), `http_error` body-only
  consumption (:95–99), ureq error mapping for both adapters
  (:165–171, :299–305), pure body construction (:147–152, :278–292);
  `crates/z-core/src/settings.rs` — defaults/bounds (:20–28), typed
  load recipe (:78–97), store (:104–114); `crates/z-core/src/fingerprint.rs`
  — fnv1a64 (:16–23); `crates/z-core/src/journal.rs` — JournalKind +
  Other additive rule (:34–63); workspace `Cargo.toml:42`
  (`serde_json = "1"`, no `preserve_order` → sorted object keys).
- Z-DESKTOP-TASKS.md (retrieved 2026-08-23): core-013..019 definitions
  and edges (:57–76), core-033 (:117–118), jour-025 (:230–231),
  jour-028 (:239–240), prov-001 (:804–805).
- Z-DESKTOP-MASTER-SPEC.md (retrieved 2026-08-23): §17 failure table
  (:1060–1082), provider contract "must not retry internally" (:1293),
  M6 doom-loop breaker milestone (:1504).
- docs/adr/0011 (D1.3 snapshot access; D2.3 failure-seam discipline
  this ADR implements), docs/adr/0015 (D2/D3 prefix stability and
  provider-cache economics D4 preserves).
- Web (one fact, cited): RFC 9110 §10.2.3 (retrieved 2026-08-23) —
  `Retry-After = HTTP-date / delay-seconds`, sent with 503 and 3xx;
  RFC 6585 §4 permits it on 429.
