# ADR-0011: Settings storage & provider router hooks

Ledger: fixes the storage and access contracts behind set-002 (initial
schema population) and set-003 (typed accessor cache); fixes the seam
contracts behind prov-004 (model capability registry) and core-031
(failover hook points). Unblocks directly: core-011 (MAX_TOOL_ROUNDS via
settings), core-012 (approval timeout wired to gate deadline), core-015,
orch-021, ctx-009, mcp-010, red-007, auto-008 — every task gated on
set-002 — and transitively the router chain prov-005..008.

## Status

Accepted (2026-08-23). Justification: §101.2 already prescribes the
settings runtime flow ("Load file → migrate to current version → validate
against schema … → typed accessor cache → SetSetting commands update +
persist + emit change event") and §75 fixes the initial keys including
`agent.max_tool_rounds` (int, 24, dev) and `agent.approval_timeout_s`
(int, 300, dev) with the rule "secrets NEVER here; migrations versioned".
§12 scopes launch surface to "< 20 options", schema-driven, "secrets
excluded". On the provider side, z-protocol's `ProviderConfig` doc comment
binds us: "One active provider at a time in Personal v0.1; the registry
already accepts several so the Router can grow without a breaking change",
and §8.17 fixes router invariants (decisions logged with reasons; fallback
never silently downgrades declared hard requirements). This ADR makes
those commitments concrete enough to implement without a second decision
round. It adds no dependency (§52 untouched; keyring remains under
evaluation for the secrets side, per ADR-0006).

## Context

Two hardcoded consts in the agent runtime are the immediate consumers:

- `MAX_TOOL_ROUNDS: usize = 24` (runtime.rs:166) bounds the turn loop
  (runtime.rs:491) and the stop message (runtime.rs:650).
- `APPROVAL_TIMEOUT: Duration = 300 s` (runtime.rs:167) is the approval
  gate deadline passed to `gate.wait` (runtime.rs:595).

core-011/012 exist precisely to make these configurable; both are blocked
on the settings tasks. Today there is no settings store at all — the only
persisted configuration is `data/config.json`, written by
`configure_provider` as the raw serde JSON of one `ProviderConfig`
including `api_key` (runtime.rs:357–361), and restored at startup by the
app shell, which re-sends `Command::ConfigureProvider` (z-app
main.rs:594–601). Core itself never reads the file; persistence is an
app-layer concern by design.

The provider side has exactly one active slot: `Shared.provider:
Mutex<Option<Arc<dyn Provider>>>` (runtime.rs:94), swapped wholesale by
`configure_provider` after `from_config` validates kind/base_url
(runtime.rs:342–356). There is no retry, no health check, no second
provider to fail over to — prov-001/002 are separate planned work. What
prov-004 and core-031 must not do is paint the single-slot design into a
corner that later requires a protocol break.

Constraints inherited from prior decisions: blocking threads + channels,
no async/tokio in core (ADR-0001); snapshot reads over shared mutable
state (ADR-0009 applied this to the index actor); protocol variants
additive-only with serde defaults (§28 stability table); personal-first —
no team sync or multi-user abstractions (ADR-0005); minimal dependencies
(§52). Scale honesty: one user, a handful of providers configured at most,
a settings file edited by hand until set-009 ships a UI.

## Considered options

**(a) Settings in `data/config.json`, extending the existing file.**
Rejected on the secret boundary. §75's first rule is "secrets NEVER here",
and config.json already contains `api_key` — merging settings into it
would either put non-secret, diffable, searchable values inside a
secret-bearing file or force the migration/validation machinery (set-005/
set-007) to special-case one subtree forever. It also couples settings
migrations to the `ProviderConfig` serde shape owned by the protocol
crate. The file split *is* the security boundary: settings.json can be
logged, journaled, and diffed freely; config.json cannot.

**(b) SQLite for settings.** Rejected for v1. §4/§52 discipline: JSONL/
JSON before SQLite was the explicit ADR-0004 posture for the journal, and
the same reasoning holds here — one writer (command loop), tiny row count
(<20 keys at launch per §12), human-editable before any UI exists. Revisit
only if settings grow join-shaped structure, which §75 gives no sign of.

**(c) Eager multi-provider registry now (map of named configs, router
picking per request).** Rejected as premature. There is no second provider
consumer yet; §54 pre-commits the escalation (">5 providers → extract
provider registry crate; config UI"), implying the registry is
trigger-driven, not baseline. Building the map before prov-004's
capability data exists means inventing its schema twice. What v1 owes the
future is stable hook points and an additive config evolution path, not
the machinery.

**(d) Hot-reload via file watcher (notify).** Rejected for now: notify is
still under evaluation in §52, and a watcher solves a problem we don't
have — the sanctioned write path is `SetSetting` (set-004), which updates
in memory and persists atomically; hand-edited files apply on relaunch.
Swap-on-write with per-turn snapshot reads gives fresh-value-without-
restart semantics for free at turn granularity, with zero new deps.

## Decision

### D1 — Settings: separate versioned `data/settings.json`, snapshot-cached access

1. **File**: `<data_dir>/settings.json`, shape `{ "version": 1, "values":
   { "<setting-id>": value, ... } }`, ids exactly the §75 keys
   (`agent.max_tool_rounds`, `agent.approval_timeout_s`, ...). Absent keys
   fall back to the §75 default recorded in the schema — a missing or
   corrupt file reproduces today's behavior (24 rounds, 300 s) bit-for-bit.
   Secrets never enter this file (§75); credentials stay in config.json
   until ADR-0006's keychain successor lands.
2. **Schema first**: set-001 defines `SettingDef` per §101.1 (id, kind,
   default, category, mode User|Developer, restart_required, constraints);
   set-002 populates it verbatim from §75. Defaults live in the schema
   table, nowhere else — runtime consts become *readers* of schema
   defaults, eliminating the current two-sources-of-truth drift risk
   between runtime.rs:166–167 and §75.
3. **Access pattern**: one `Mutex<Arc<SettingsSnapshot>>` in `Shared`;
   readers clone the `Arc` once per turn start and read typed values
   (set-003 accessor cache) with no lock held during the turn — the same
   snapshot-read discipline ADR-0009 adopted for index state. Writes go
   through the command loop: `SetSetting` (set-004) validates against the
   schema (set-005), swaps the `Arc`, persists the file, emits a
   SettingChanged event. Unknown keys kept+warned; constraint violations
   reset-to-default+warned (§101.2, set-006).
4. **Reload semantics**: turn-granular freshness without a watcher —
   core-011 reads `agent.max_tool_rounds` at turn start, so a changed
   value applies to the next turn mid-session; `restart_required: true`
   is reserved for defs feeding construction-time structures. Hand edits
   apply on relaunch (option d above).
5. **No UI in scope**: set-002 + set-003 alone unblock core-011/012 and
   every other set-002-gated task; values are editable by hand until
   set-004 (command path) and set-009/010 (schema-rendered pages, mode
   filter) arrive. The schema-driven search index (§101.3, set-008) comes
   from `SettingDef`s, not from any UI.

### D2 — Providers: single active slot retained; hook points fixed now

1. **Slot stays**: `Shared.provider` remains the only live provider for
   Personal v1. No registry struct, no selection logic, no per-request
   routing until prov-005+ exist.
2. **Hook point A — resolution seam (core-031)**: introduce one internal
   function, `fn provider_for(...) -> Option<Arc<dyn Provider>>`, as the
   sole read path for the slot; the turn loop stops touching
   `shared.provider` directly. Its default implementation returns the
   configured slot unchanged. When routing arrives, only this function's
   body changes — callers never learn about failover.
3. **Hook point B — failure seam (core-031)**: the provider-error branch
   in the turn loop becomes the single classification site where prov-001
   (retry/backoff) and prov-007 (fallback chain) attach later. Contract
   fixed here per existing invariants: a failed provider call never loses
   the user message (§8.18), any failover decision is logged with its
   reason (§8.17), and a fallback may not silently violate declared hard
   model requirements (§8.17; enforcement via prov-008).
4. **Capability registry (prov-004)**: static, code-level data tables —
   model id → {context window, vision, tool-use tier, cost/latency class}
   per §8.17 — published as a plain module in z-core. Pure data, no
   behavior change, consumed by nothing until prov-005; its job is to fix
   the schema early enough that router policy DSL parsing (prov-005) and
   decision logging (prov-006) don't redesign it.
5. **Additive config evolution**: `ProviderConfig` already carries `name`,
   and the protocol comment promises multi-provider growth without a
   breaking change. Honoring that: when a second provider must persist,
   config.json migrates to `{ "active": "<name>", "providers": {
   "<name>": ProviderConfig } }`, deserialized with a legacy fallback that
   accepts today's bare-`ProviderConfig` object as a single-entry map.
   `Command::ConfigureProvider` gains fields/defaults only (additive rule,
   §28); the app-shell restore path keeps working unchanged.
6. **Out of scope**: full settings UI (set-009/010), router policy DSL and
   chains (prov-005..008 do that), provider registry crate extraction
   (§54 trigger: >5 providers), per-project setting overrides (single
   user, one active project at a time — YAGNI until a real workflow asks),
   team/policy sync (excluded by ADR-0005), keyring migration of
   config.json (separate ADR when §52 resolves keyring).

## Consequences

**Immediate**: core-011/012 reduce to reading two snapshot values at turn
start and deleting two consts; behavior with no settings file is
byte-identical to today, making the change low-risk and testable by
absence. Every future tunable follows the same three-line recipe: add a
§75-style schema row, read via the accessor, done — no per-setting
plumbing.

**Consistency debt removed**: runtime.rs:166–167 currently duplicate §75's
defaults with nothing enforcing agreement; after set-002 the schema is the
single source and the consts are gone. A reset-to-default round trip
(set-012) doubles as the guard test that defaults match documented values.

**Failure-path discipline**: fixing hook B as the *only* error
classification site prevents the classic failover sprawl where each call
site grows private retry logic. prov-001/007 plug into one place; journal
records gain a reason field at that point (jour-024 pattern of shape
summaries extends to router decisions).

**Accepted debt**: hand-edited JSON until set-004/009 (acceptable —
developer-first audience); settings reload is turn-granular, not instant;
config.json remains plaintext with `api_key` until the keyring ADR
lands (already accepted as ADR-0006 debt). Multi-provider persistence is
deliberately unspecified beyond the legacy-fallback shape above; its real
design waits for prov-005's policy data.

**Revisit triggers**: settings exceeding simple key/value shape (nested
per-project scopes requested) → re-decide storage; second live provider
shipping → formalize the registry map + active-pointer swap semantics in
a successor ADR; >5 providers or a config-UI demand → §54 escalation to a
registry crate.

## Sources

- Z-DESKTOP-MASTER-SPEC §12 (launch <20 options, secrets excluded),
  §8.17 Model Router (capability tags, fallback chains, logging + no-silent-
  downgrade invariants), §8.18 Provider Layer (failure/user-message and
  secret invariants), §28 protocol table (`Command` additive-only;
  `SetSetting` PLANNED; `ProviderConfig` key-handling rule), §52
  dependency policy (notify/keyring under evaluation; approved stack
  sufficient here), §54 evolution table (>5-providers trigger), §75
  Initial Settings Schema Draft (`agent.max_tool_rounds` = 24 dev,
  `agent.approval_timeout_s` = 300 dev; "secrets NEVER here"), §101.1–101.3
  (SettingDef, load→migrate→validate→cache flow, search index).
- Z-DESKTOP-TASKS: set-001..012 definitions and dependency edges
  (core-011←set-002, core-012←set-003, core-015/orch-021/ctx-009/red-007/
  mcp-010/auto-008 ← set-002; core-031←prov-004; prov-005..008 chain).
- Code (inspected 2026-08-23): `z desktop/crates/z-core/src/runtime.rs`
  lines 93–95 (single-slot `Shared.provider`), 166–167 (hardcoded consts),
  342–362 (configure_provider + config.json persistence incl. api_key),
  491/595/650 (const consumption sites); `crates/z-protocol/src/lib.rs`
  57–70 (ProviderConfig with `name`, single-active doc commitment);
  `crates/z-app/src/main.rs` 594–601 (startup restore re-sends
  ConfigureProvider; core stays file-agnostic).
- docs/adr/0006 (BYOK plaintext interim, keychain planned), 0008/0009
  (format precedent; 0009's snapshot-read discipline reused in D1.3).
