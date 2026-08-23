# ADR-0018: Protocol evolution & versioning policy

Ledger: codifies the additive-evolution discipline core-020 already
practiced, decides tests-001's scope (unknown-field tolerance vs
unknown-variant rejection), and fixes when `PROTOCOL_VERSION` stops being a
documented const and becomes a checked envelope field. No proto-* tasks
exist in the ledger; this ADR is the owner of that gap until one is filed.
Unblocks nothing immediately — it prevents re-deciding per variant, which
is exactly how strict protocols rot into tolerant mush.

## Status

Accepted (2026-08-23). Justification: §1.3 fixes the posture ("semver for
crates once external consumers exist; until then, the protocol crate
(`z-protocol`) carries compatibility discipline internally (additive changes
only)", :124-125); §27.1 assigns per-type stability rules — Command/Event
"additive only" (:1252-1253), Risk "closed set; new variants are security
decisions" (:1255) — plus invariants (zero internal deps; every enum variant
round-trips through JSON, :1258-1261) and failure modes ("unknown variant on
deserialize → error at boundary, not panic", :1263-1264); §37's domain table
points Protocol at this policy (:3470). The spec says *additive-only* but
leaves open what "unknown" means on each axis (fields vs variants), whether
a fallback variant belongs in the enums, and when the version const gets
enforced. This ADR answers those three with repo evidence.

## Context

What exists today (repo inspection 2026-08-23):

- **The wire is not wires yet.** Commands travel as typed Rust values over
  an in-process `std::sync::mpsc::channel()` (`z desktop/crates/z-app/src/
  main.rs:847`); sender and receiver compile against one enum definition.
  JSON round-trips for Command/Event exist only inside z-protocol's test
  module (lib.rs:111-152). Nothing serializes a Command or Event across any
  process boundary anywhere in the workspace.
- **The version const is advisory only.** `PROTOCOL_VERSION: u32 = 1`
  (lib.rs:11-12) is referenced by no transport; its module doc promises
  "an envelope mismatch is a hard error" (lib.rs:5-7), but there is no
  envelope to carry it. It documents intent; it enforces nothing.
- **Tagging shape**: both enums are internally tagged,
  `#[serde(tag = "type", rename_all = "snake_case")]` (lib.rs:40, :82).
  Serde semantics follow from this choice: unknown *fields* inside a known
  variant are silently ignored (free forward-compatibility for additive
  fields); unknown *variants* (unrecognized `"type"` tag) are hard
  deserialize errors. Derive cannot express an `Other(serde_json::Value)`
  fallback on internally-tagged enums — `#[serde(other)]` supports unit
  variants only — so a data-bearing escape hatch means hand-written
  Deserialize impls for both enums.
- **The journal already solved its own versioning problem differently**:
  `JournalKind` has hand-written Serialize/Deserialize via string mapping
  with lossy fallback to `Other(String)` (journal.rs:36-51, :78-91),
  documented "additive evolution by design"; `Record.payload` stays
  `serde_json::Value` "so new fields can be added without breaking old
  readers" (journal.rs:96-103). That tolerance exists because journal data
  outlives binaries — old files replay under new code and vice versa.
  Commands never persist; ProviderConfig does (config.json, runtime.rs:371)
  and got its own legacy-fallback migration plan in ADR-0011 D2.5.
- **core-020 is the worked example of the discipline**: new
  `Command::EnqueueMessage` (lib.rs:47) and `Event::SteeringQueued`
  (lib.rs:89) appended, each with a JSON round-trip test pinning the exact
  snake_case tag (lib.rs:136-151). No existing line changed semantically.

Scale honesty: one user, one desktop process, UI and core ship as one
binary updated together. There is no version skew to defend against today;
the consumers this policy protects are future ones (out-of-process UI,
plugins §8.9's "versioned contracts", :556, or a remote client).

## Considered options

**(1) Unknown-variant handling for Command/Event.**

*(a)* Add `Other(serde_json::Value)` fallback variants now, JournalKind-style.
Requires abandoning derive serde for hand-written impls on both enums (derive
can't do data-bearing fallbacks on tagged enums), and every match site in
z-core gains an unreachable-in-practice arm. Tolerance would guard against a
skew that cannot occur across an mpsc channel inside one binary. Worse, it is
the wrong default for commands: a client whose `resume_turn` was silently
swallowed waits forever, whereas a hard deserialize error surfaces
immediately. Rejected until a real boundary exists.

*(b)* Stay strict (unknown variant → error), rely on serde's free
unknown-field tolerance for additive fields, and pin both behaviors in
tests-001. Matches §27.1's own failure-mode text verbatim (:1263-1264).
Chosen.

*(c)* Version-gated dual-parse (try vN+1, fall back to vN) prepared in
advance. Two decoders maintained for zero current readers. Rejected.

**(2) Explicit version header on the wire at M3.**

*(a)* Wrap every message in `{v: PROTOCOL_VERSION, ...}` now so the
mechanism is ready. Ready for whom? No serialized byte leaves the process;
the compiler rejects mismatches harder than any header check. Ceremony.
Rejected.

*(b)* Delete the unused const until needed. Loses the cheapest carrier of
the policy — the doc comment binding bump-on-breaking-change to the const
(lib.rs:11) — and §27.1/§37 treat protocol versioning as decided intent.
Rejected.

*(c)* Keep `PROTOCOL_VERSION` as the declared contract version; git history
is the schema record while one binary ships both sides; the const becomes an
envelope field checked hard-error-at-boundary on the day an actual IPC
boundary lands. Chosen — it is literally what the module doc already
promises; this ADR just dates the promise's activation trigger.

**(3) Where evolution rules live.**

*(a)* Per-task judgment calls. That is how rename accidents happen.
Rejected. *(b)* This ADR + tests-001 assertions. Chosen.

## Decision

### D1 — Strict unknown-variant rejection; no `Other` variant on Command/Event

Unknown `"type"` tags fail deserialization loudly (already serde's behavior
for internally-tagged enums; §27.1 :1263-1264 confirms intent). Unknown
*fields* within known variants are ignored — free, pinned by tests. The
journal keeps its `Other(String)` hatch because journals outlive binaries;
commands don't. If an out-of-process boundary arrives (plugin host, remote
client), the boundary owner writes a successor ADR choosing per-direction
tolerance there — likely tolerate-and-log for events, reject for commands —
with real skew data, not speculation.

### D2 — `PROTOCOL_VERSION` stays a const; envelope enforcement deferred to first external boundary

Bump rule (binding now): increment on any change that breaks decode of
previously-valid messages — variant removal/rename, field removal/rename or
type change without a default, tag-string change. Pure additions do not bump.
Until an envelope exists, the compatibility checker is `cargo build` +
round-trip tests across one binary; git blame on lib.rs is the schema
changelog. When plugins/out-of-process UI land (§8.9), the envelope carries
`PROTOCOL_VERSION` and mismatch is a hard error per the standing doc comment
— not best-effort negotiation, which Personal scale doesn't need.

### D3 — Evolution rules (codifying core-020's practice)

1. **New variant**: append to the end of the enum; add a JSON round-trip
   test asserting the exact `"type"` tag string and payload fields
   (pattern: lib.rs:136-151). Position is wire-irrelevant for tagged enums
   but append-only keeps diffs reviewable and mirrors §27.1's planned-variant
   lists.
2. **New field**: add with `#[serde(default)]` whenever an older reader
   could legitimately see the type (anything persisted or journaled);
   in-memory-only additions may skip the default. Old-reader-tolerates-new-
   writer comes from serde's ignored-unknown-fields; new-reader-sees-old-
   writer requires the default. Never rely on field order.
3. **Never rename**: not variants, not fields, not derived tag strings —
   `rename_all = "snake_case"` makes renames silent wire breaks. Renames
   are remove+add across two releases once external consumers exist.
4. **Breaking changes** (removal/rename/type narrowing): forbidden while
   versions coexist; when justified, bump `PROTOCOL_VERSION`, write an
   explicit migration at the owning boundary (config.json precedent:
   ADR-0011 D2.5's legacy fallback), never silent tolerance.
5. **Risk stays closed** (§27.1 :1255): new Risk variants are security
   decisions requiring an ADR, not additive edits.
6. **Test obligations (tests-001 scope)**: per-variant-family round-trip
   already mandated (§27.1 invariant 2); add (i) unknown-field tolerance —
   valid variant + extra key decodes fine; (ii) unknown-variant rejection —
   `{"type":"time_travel"}` errors, matching §27.1's failure mode; (iii)
   missing-defaulted-field decode where defaults exist. Three small tests,
   one file, no fixtures.

## Consequences

**Zero code changes now.** This ADR ratifies existing behavior — strict
variants, ignored unknown fields, advisory const — which is why it costs
nothing: the discipline was already practiced (core-020) and half-written
down (§27.1, lib.rs doc comments). The only new work item is tests-001's
three assertions.

**Accepted debt**: `PROTOCOL_VERSION` is dead code until an envelope exists
(kept deliberately — it is policy documentation that compiles). Until the
first external consumer, "compatible" means "same commit-ish", enforced by
the build, which is honest for a single-binary product.

**Trigger to revisit**: any of — out-of-process plugin host (§8.9), second
process speaking the protocol, persisted command/event logs, or a network
listener — each converts protocol skew from impossible to routine and
activates D2's envelope plus D1's per-boundary tolerance decision. None is
planned for M3/M4; §54 has no escalation row for protocol work yet, which is
correct at Personal scale.

**Failure mode this prevents**: someone adds `Other(Value)` "for safety",
every runtime match grows a dead arm, a typo'd tag string starts silently
discarding steering commands, and the bug surfaces as a hung UI three
screens away from the cause. Loud beats lenient at a boundary you control
end to end.

## Sources

- Code (inspected 2026-08-23): `z desktop/crates/z-protocol/src/lib.rs` —
  module doc + `PROTOCOL_VERSION` (:1-12), Command tagging (:39-41),
  EnqueueMessage (:47), Event tagging (:80-83), SteeringQueued (:89),
  round-trip/tag test patterns (:111-152); `z desktop/crates/z-app/src/
  main.rs:847` (in-process mpsc channel — no serialization boundary);
  `z desktop/crates/z-core/src/journal.rs` — JournalKind Other escape hatch
  and rationale (:34-51), manual lossy serde impls (:78-91), Value payload
  rationale (:96-103); `runtime.rs:371` (ProviderConfig persisted to
  config.json).
- Z-DESKTOP-MASTER-SPEC.md: §1.3 versioning intent (:124-125); §27.1 owned
  types + stability rules (:1250-1256), invariants (:1258-1261), failure
  modes (:1263-1264); §8.9 plugin "versioned contracts" (:556); §37 domain
  table (:3470).
- docs/Z-DESKTOP-TASKS.md: core-020 [IMPLEMENTED] (:78-79, the worked
  additive extension), core-005 (:33-35), tests-001 [PLANNED] (:2306-2307,
  scope fixed by D3.6); no proto-* tasks exist (verified).
- docs/adr/0011 (D2.5 config migration precedent; §28 stability-table
  reading), 0016 (JournalKind tolerance analysis, :38-44 — the contrast
  case for D1).
