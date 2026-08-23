# ADR-0020: Local model support

Ledger: closes [RESEARCH] prov-016 (local backend trait alignment study);
fixes the contracts behind prov-017..023 (server process manager, model
catalog, download manager, disk preflight, local Provider impl, VRAM/RAM
declarations, offline detection) and clarifies the hw-001 dependency edge
feeding hw-009 (sizing recommendations). Extends ADR-0011's router seams
(provider_for resolution, failure-classification site, additive
`{active, providers}` config evolution) to locally hosted backends.

## Status

Accepted (2026-08-23). Justification: §49.13 already prescribes the shape —
"Backend abstraction: llama.cpp server process managed by us (first), others
later — all behind Provider trait" — and §8.19 fixes the purpose (offline
use, privacy, cost control) plus the integrity commitment ("model downloads
managed with integrity checks"). This ADR makes those commitments concrete
enough that prov-017..023 implement without a second decision round, and
records why the FFI alternative loses. It adds **zero** new Rust
dependencies (§52 untouched): the local backend is reached over HTTP by the
OpenAI-compatible client path that already exists in `provider.rs`.

## Context

The spec commits to local inference three times: §8.19 (planned local
models behind the Provider trait, hardware-driven quantization/context
sizing, scheduler protection, integrity-checked downloads), §49.13 (the
design bullet quoted above, plus catalog manifests and VRAM/RAM admission),
and §93's `local-offline` router policy whose fallback is the explicit
error `"no local model installed"`. §12 notes the whole app works offline
"except model calls themselves" — local models are what close that gap.
The task ledger sequences it: prov-016 (this study) → prov-017 (llama.cpp
server process manager) → prov-018 (catalog manifest) → prov-019 (download
manager with hash verification) → prov-020 (disk-space preflight), with
prov-021 (Local provider behind Provider trait) hanging off prov-017, and
prov-022/023 off prov-021. hw-009 (sizing recommendation) depends on
hw-001 + prov-022.

Constraints inherited from prior decisions: blocking threads + channels,
no async/tokio in core (ADR-0001); one active provider slot with the
`provider_for` seam and a single failure-classification site (ADR-0011
D2); minimal dependencies, approved stack has no C++ toolchain story
(§52); personal-first, one user (ADR-0005); per-platform release
artifacts are zip/tar.gz + checksums (§53). The `Provider` trait
(`z-core/src/provider.rs:68`) is two methods — `describe` and streaming
`stream` — and an `OpenAiProvider` SSE adapter already lives beside it.

Three open engineering questions, none answered by the spec lines alone:

1. **Runtime strategy** — embed llama.cpp via FFI, spawn an external
   server we manage, or accept user-run servers (ollama, LM Studio)?
2. **Probe ordering** — prov-022 declares VRAM/RAM requirements and the
   ledger edges it to hw-001; does anything in the local chain actually
   block on the hardware probe?
3. **Model acquisition** — in-app GGUF download with checksums, or
   user-provided file paths only at v1?

## Considered options

**(a) Embed llama.cpp in-process via FFI (`llama-cpp-2` class bindings).**
Rejected. §52 bans heavyweight dependencies and requires every addition to
justify platform support and build-time impact: an FFI crate drags a
C/C++ toolchain, bindgen, and per-OS linking into all three target
platforms, and forces us to pick GPU backends (CUDA/Metal/Vulkan) at
*our* build time instead of shipping llama.cpp's own prebuilts. It also
collapses the crash domain: a segfaulting quantized matmul takes down the
app shell, violating the spirit of §8.19's scheduler isolation intent.
Finally it duplicates state — weights loading, KV cache, eviction — that a
separate process owns naturally. No upside at desktop token-generation
latencies, where an extra localhost HTTP hop is noise.

**(b) Support only user-run OpenAI-compatible servers (point at ollama /
LM Studio / manually started llama-server).**
Rejected as the *whole* answer. It outsources the exact experience §8.19
promises (managed lifecycle, sizing, integrity-checked downloads) and
makes the offline story "install another tool first". But as a *mode* it
is nearly free and kept — see D1.

**(c) Both runtimes: FFI for small models, server for large.**
Rejected: two code paths for the same capability, double the test matrix,
and the FFI half still carries option (a)'s build/isolation problems.
One transport, one lifecycle owner.

**(d) Block catalog/sizing on hw-001.**
Rejected as a sequencing trap. Requirement *declarations* are static
manifest data (§49.13 lists size/quant/ctx/license in the catalog);
only their *evaluation against this machine* needs a snapshot. Making
prov-018/019 wait on hw-001 would delay offline capability for a
recommendation nicety. The ledger's prov-022←hw-001 edge stays — it
guards the fit-check/recommendation consumer (hw-009), not the schema.

**(e) Downloads without verification, or paths-only forever.**
Verification-less downloads rejected outright: §8.19 says "integrity
checks", §49.13 says "download manager verifies hashes", and a corrupted
GGUF fails at load time far from the cause. Paths-only-forever rejected:
it strands non-technical users and contradicts §49.13's catalog. But
paths-only-*first* is correct sequencing (see D3).

## Decision

### D1 — External `llama-server` process, spoken to over its OpenAI-compatible API; no FFI

1. **Transport**: the local backend is an HTTP client, not a linked
   library. We spawn `llama-server` (prov-017: locate binary via the
   `local.llama_server_path` setting, default PATH lookup; bind
   `127.0.0.1:<ephemeral>`; poll `/v1/health`; supervise with restart +
   clean shutdown on app exit) and talk to it with the existing
   OpenAI-compatible request/SSE machinery from `provider.rs` — a
   `LocalProvider` is a configured `OpenAiProvider` plus a process
   handle. Currency check: llama.cpp's server README advertises "OpenAI
   API compatible chat completions, responses, and embeddings routes"
   plus `/v1/health` on master (fetched 2026-08-23); this surface has
   been stable since 2024, and Anthropic-compatible routes also exist.
   Contract tests (prov-015 pattern) pin whatever subset we rely on.
2. **User-managed mode**: any user-supplied OpenAI-compatible base URL
   (ollama `:11434/v1`, LM Studio `:1234/v1`, hand-started llama-server)
   is just another `ProviderConfig` in ADR-0011's additive
   `{active, providers}` map with `kind: openai_compatible` and no spawn
   handle. Zero extra code beyond config validation; lifecycle ownership
   (spawn vs. attach) is the only difference between the two modes.
3. **Isolation & GPU**: running inference out-of-process contains crashes,
   lets the resource scheduler (§8.21) treat the PID as a killable
   background workload during interactive spikes, and inherits
   llama.cpp's own CUDA/Metal/Vulkan support from whatever binary the
   user points at — none of which enters our build (§52, §53).
4. **Capability honesty**: local catalog entries (prov-004 registry +
   prov-018 manifest) declare `tools_tier: none|basic` conservatively
   until a model/template passes contract replay; router policies must
   not assume frontier tool-use from a local tier (§8.17 capability
   tags; §93 hard requirements bypass downgrades).
5. **Non-goal**: in-process generation (FFI or otherwise) for v1.
   Revisit only if a latency-critical feature demands sub-100ms
   in-app completions (e.g., editor ghost text) — nothing on the
   roadmap does.

### D2 — Declarations are static manifest data; only fit-evaluation waits on hw-001

1. prov-018's manifest gains `min_ram_gb`, `min_vram_gb`, `recommended_ctx`
   fields at schema time; these ship with the catalog whether or not the
   probe exists. Nothing in prov-017..021 blocks on hw-001.
2. Evaluating those numbers against *this machine* — hw-009 sizing
   recommendations and §49.13/§8.21 scheduler admission control —
   consumes a hw-001 snapshot; that is what the ledger's prov-022←hw-001
   and hw-009←(hw-001, prov-022) edges encode.
3. Degradation rule: with no probe snapshot, the UI surfaces declared
   requirements next to each model and lets the user choose; we never
   hard-block a launch attempt on missing hardware data — llama-server's
   own load failure is the backstop, surfaced through the normal
   provider-error path (§8.18: failures never lose the user message).
4. prov-023 (offline detection) is connectivity state feeding the §93
   `local-offline` policy; it shares nothing with the probe.

### D3 — Model acquisition: user-provided GGUF path day one; verified in-app download second

1. **v1**: `local.model_path` points at any GGUF file; prov-017 passes it
   to `llama-server --model`. Cost is near zero, and it delivers the
   privacy/offline value before any downloader exists.
2. **prov-019**: in-app download from URLs pinned in the curated catalog
   manifest (prov-018: size, quant, ctx, license), SHA-256 verified
   before an atomic temp-file→rename into the models dir; prov-020
   prefights disk space against the manifest size (§49.13 verbatim).
   Hash mismatch = delete + clear error, never load.
3. Out of scope for v1: resumable/chunked downloads, P2P/mirrors,
   automatic model updates, multi-model hot-swap (one loaded model per
   llama-server instance; switching = restart the child).

## Consequences

**Immediate**: prov-016 closes as decided-by-ADR; prov-017 gets its
process contract (locate, spawn, health-poll, supervise, shutdown);
prov-021 collapses to configuration glue over existing SSE code;
prov-022/023/hw-009 have unambiguous blocking semantics. The router
treats the local backend like any other provider: failover flows through
ADR-0011's single classification site, and prov-024's E2E covers
cloud→local transparent continuation including the `"no local model
installed"` terminal error (§93).

**Dependency budget**: zero new crates. The heaviest "dependency" is the
llama-server executable itself, resolved at runtime; shipping bundled
prebuilts becomes a §53 packaging task, not a build-system change.

**Accepted debt**: one localhost HTTP hop per request (negligible vs.
token generation); binary resolution is manual until bundling lands;
path-mode GGUFs carry no integrity guarantee (user-owned files);
tool-calling quality varies by model template and is gated behind
conservative capability tiers; single active local model at a time.

**Revisit triggers**: a latency-critical in-process inference feature →
reopen FFI as a successor ADR; demand for several concurrently loaded
local models → multi-instance supervision design; llama-server's
OpenAI-compatible surface drift breaks replay fixtures → pin a minimum
server version in the manifest and warn on mismatch.

## Sources

- Z-DESKTOP-MASTER-SPEC §8.19 Local Models (purpose + integrity-checked
  downloads, PLANNED, depends on hardware intelligence + scheduler);
  §49.13 ("llama.cpp server process managed by us (first)... all behind
  Provider trait"; catalog manifest fields; hash verification; disk
  checks; VRAM/RAM admission); §49.14 (probe consumers incl. "recommended
  local model size"); §8.21 Resource Scheduler (priority classes,
  admission control); §12 line 175 (app functions offline except model
  calls); §93 (`local-offline` policy, fallback error string);
  §8.17/§8.18 (capability tags, no-silent-downgrade, failures never lose
  the user message); §52 dependency policy; §53 release artifacts.
- Z-DESKTOP-TASKS.md: prov-016..025 definitions and dependency edges
  (incl. prov-022 ← prov-021 + hw-001; hw-009 ← hw-001 + prov-022);
  hw-001..003 probe chain.
- Code (inspected 2026-08-23): `z desktop/crates/z-core/src/provider.rs`
  — `trait Provider` (:68), `StreamItem`/`StreamOutcome`, shared SSE
  reader, existing `OpenAiProvider`.
- docs/adr/0011 (provider_for seam, failure-classification site,
  additive multi-provider config shape), ADR-0005 (personal-first),
  ADR-0001 (blocking threads, no async in core, as cited by ADR-0019).
- Web (1 search, fetched 2026-08-23): ggml-org/llama.cpp
  `tools/server/README.md` @ master — "OpenAI API compatible chat
  completions, responses, and embeddings routes"; Anthropic Messages-
  compatible route; public `/v1/health`.
