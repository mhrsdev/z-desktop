# ADR-0007: Tree-sitter for repository indexing

Ledger: idx-003. Unblocks: idx-004..006 (grammar packs), idx-007..017 (extraction,
containment, incremental reparse), M2 exit criteria.

## Status

Accepted (2026-08-23). Justification: the spec-level direction is already binding —
§31 M2 exits on a "tree-sitter index actor; incremental updates; symbol/reference
lookup; repo-map v2", §10 names the tree-sitter index actor as flagship work, §374
plans "tree-sitter ASTs, references, incremental updates", and §88.4 designs parser
failure containment around it. This ADR performs the §52 dependency evaluation
required before crates enter the tree, records measured costs, and fixes scope. It
narrows §52's "under evaluation" entry for tree-sitter only (ropey/git2/notify/
keyring remain open).

## Context

The index must deliver symbol/reference/import extraction, reverse-reference and
affected-analysis queries, agent-facing go-to-def/find-refs, call graphs, and
incremental reparse (capability matrix #41–48; differentiators #41–52). These are
structural facts — definitions, bindings, import edges — not text patterns; they
require syntax trees.

Current `repo.rs` (v0) cannot provide them:

- `extract_symbols()` is line-prefix matching (`fn `, `def `, …) capped at 40
  symbols/file over the first 4,000 lines; no references, imports, or spans;
  comments and strings yield false positives; nested items are invisible.
- Fingerprint-driven rescans (mtime,size) correctly reuse entries, but the reused
  payload has no reusable structure.
- It stays: fallback extraction for languages without a loaded pack, and
  `map_text()` production regardless of grammar availability.

Budgets (skills/z-repository-intelligence, enforced by idx-021/026..029): initial
index of 100k files in minutes; one-file incremental update <50 ms; lookup <10 ms;
search p95 <200 ms.

## Considered options

Version/license facts verified 2026-08-23 via crates.io + GitHub APIs and direct
inspection of the published crates. Build cost row measured here (gcc 13.3.0,
Ubuntu 24.04 x86_64, `cc -O2 -c` per shipped C file; grammar crates compile
`parser.c` + `scanner.c` via the cc crate, C11, `-utf-8` on MSVC):

| Crate | Version (released) | License | Shipped generated C | Measured −O2 |
|---|---|---|---|---|
| tree-sitter | 0.26.13 (2026-08-23) | MIT | ~350 KB runtime `lib.c` | 3.7 s → 263 KB obj |
| tree-sitter-rust | 0.24.2 (2026-03-27) | MIT | 6.5 MB `parser.c` (205,906 ln) + 12.6 KB `scanner.c` | 0.8 s → 1,118 KB |
| tree-sitter-python | 0.25.0 (2025-09-11) | MIT | 3.4 MB (129,742 ln) + 15.5 KB `scanner.c` | 0.6 s → 514 KB |
| tree-sitter-javascript | 0.25.0 (2025-09-01) | MIT | 2.9 MB (94,268 ln) + 10.6 KB `scanner.c` | 0.7 s → 432 KB |
| tree-sitter-typescript | 0.23.2 (2024-11-11) | MIT | ts 8.7 MB + tsx 8.8 MB (≈283k ln each) + 10 KB `scanner.h` | 1.0 + 1.1 s → 2,850 KB |

Maintenance health: core repo 26.7k stars, pushed 2026-08-23, ten tagged 0.26.x
releases between Dec 2025 and Aug 2026; 34.2M lifetime downloads; MSRV 1.77
(workspace floor 1.85). Grammar crates release rarely by design — grammars freeze
once stable (rust 2026-03, python/js 2025-09; javascript repo push 2025-11).
Outlier: tree-sitter-typescript — last crates.io release 2024-11-11 (~21 months
stale at decision time), repo push 2025-08-29. Flagged as revisit trigger, not a
blocker.

Security posture: published crates ship pre-generated `parser.c` — deterministic
LR tables produced by `tree-sitter generate`; no codegen runs during our build.
Human-authored audit surface per grammar is the small `scanner.c` (10–16 KB).
Parse errors produce ERROR nodes rather than aborting, but this remains third-party
C linked into the process: containment is mandatory (idx-016 `catch_unwind` per
file per §88.4; idx-017 errored-file registry with retry on grammar upgrade).
Upstream has no OSS-Fuzz project today (checked google/oss-fuzz 2026-08-23);
assurance rests on very wide deployment (Zed, Neovim, Helix) plus our own guards.
Default-feature deps of the core crate: regex (default features off), regex-syntax,
tree-sitter-language, streaming-iterator; build-deps cc + serde_json. wasmtime
enters only behind the opt-in `wasm` feature. No tokio, no async anywhere.

Options:

**(a) Keep regex/prefix heuristics only.** Zero deps, zero supply-chain surface —
and structurally unable to answer refs/imports/call-graph (idx-008/009/031..034)
or power affected-analysis (#47). Rejected as end state; retained as fallback.

**(b) Trigram-only lexical search (ripgrep-style).** Answers "which files contain
this token", never "what defines/references/imports this symbol". Orthogonal, not
substitute: it is the lexical layer already planned independently (idx-018..022)
and proceeds regardless of this decision.

**(c) Defer ASTs past M2.** M2's exit criteria ARE the AST deliverables (§31);
deferral relocates the decision without shrinking it while blocking
idle-time indexing (#15), change intelligence (#47), and def/ref tools
(idx-031/032). Rejected in favor of incremental adoption: one grammar pack at a
time, Rust first.

**(d) LSP-based indexing.** External per-language servers mean per-project install
and config burden, uneven capability matrices, session-shaped semantics, and
cold-start latency; indexing a repo must not require its toolchain (local-first).
LSP client ecosystems lean on the async runtimes banned in core (ADR-0001).
Editors that pair the two (Zed, Helix, Neovim) embed tree-sitter for exactly this
reason and reserve LSP for semantics. Rejected for the index; a diagnostics LSP
client remains planned separately (editor surfaces).

## Decision

Adopt tree-sitter now, scoped:

1. **Crates**: `tree-sitter` 0.26.x; grammars registered via
   `tree-sitter-language`. Exact versions pinned in `[workspace.dependencies]`;
   Cargo.lock committed; crates.io sources only (no git deps). Do not enable the
   `wasm` feature (keeps wasmtime out of the graph).
2. **Grammar order**: Rust first (idx-004) — dogfood on our own tree; then Python
   (idx-006) and the JavaScript half of idx-005; TypeScript/TSX last (idx-005
   tail) given upstream staleness. One pack = one grammar crate + query sets
   (symbols/references/imports) + fixture tests.
3. **Architecture** unchanged (skills/z-repository-intelligence): single index
   actor thread (ADR-0001), snapshot-only reads (idx-002), content-fingerprint
   cache (idx-011), reparse-changed-files-only (idx-012). Tree-sitter is
   synchronous C invoked from the actor thread; zero async interaction.
4. **In-process embedded parsers** — not wasm, not external processes — per the
   measurements above.
5. `repo.rs` heuristics persist as the no-pack fallback and `map_text()` source.

## Consequences

**Build time**: +4–5 s serial clean-release compile for all five components on one
core; cargo parallelizes across crates, and debug-profile cost is equivalent
(parse-table size dominates, not optimization). Grammar bumps recompile one file
each.

**Binary size**: +~5.9 MB object code with all packs; Rust-first ships +~1.9 MB
including the 263 KB runtime. Acceptable for a desktop application; CI artifact
sizes make drift visible.

**Security/supply-chain mitigations**: exact pins + committed lockfile;
review-on-bump discipline where the diff review focuses on `scanner.c` and the
grammar changelog (generated tables churn wholesale by nature); per-file
`catch_unwind` + parse timeouts (idx-016); errored-file registry retried on
grammar upgrade (idx-017); grammars obtained only from crates.io.

**Incremental reparse**: batch indexing reparses whole changed files keyed by
content fingerprint. Tree-sitter's intra-file incremental API (`tree.edit()`)
matters for live buffers (editor v1, ropey integration) and is deliberately out of
scope for the index actor; adopting it later changes no interface defined here.

**Accepted debt**: tree-sitter-typescript staleness (~21 months without a release).
Mitigated by shipping it last. Revisit triggers (audit cadence per
DEVELOPMENT-STATE):

- typescript crate exceeds 24 months stale, or breaks against core 0.27+ → drop
  the TSX half, keep the JS grammar + TS-aware heuristics, supersede via new ADR;
- any grammar's object size grows ~3× or build time breaches budget;
- memory-cap stress failures on XL corpora (idx-030) — spill design (idx-022) is
  unaffected by this choice but remains the pressure valve;
- a security defect in the C runtime goes unfixed upstream for one release cycle →
  vendor a patched fork, recorded in THIRD_PARTY_RESEARCH_AND_REUSE.md and here.

## Sources

- crates.io API (retrieved 2026-08-23): tree-sitter 0.26.13; -rust 0.24.2;
  -typescript 0.23.2; -javascript 0.25.0; -python 0.25.0. All MIT.
- GitHub API (retrieved 2026-08-23): tree-sitter/tree-sitter pushed 2026-08-23
  (26,730 stars); tree-sitter/tree-sitter-typescript pushed 2025-08-29.
- Vendored crate inspection (2026-08-23): `Cargo.toml(.orig)` — MSRV 1.77, MIT,
  default features `std`, wasmtime opt-in; `bindings/rust/build.rs` — cc/C11,
  `-utf-8` on MSVC; `src/parser.c` byte/line counts as tabulated.
- Local measurement (2026-08-23): gcc 13.3.0, Ubuntu 24.04, `cc -O2 -c` timings
  and object sizes as tabulated; google/oss-fuzz project listing checked (no
  tree-sitter project).
