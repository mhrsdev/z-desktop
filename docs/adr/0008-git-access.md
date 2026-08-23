# ADR-0008: Git access strategy (CLI shell-out vs git2 vs gix)

Ledger: edit-025. Unblocks: edit-026/027 (approved write tools) directly,
edit-028 (history-rewrite prohibition tests) and orch-007..009 (worktree
manager, orphan scan, quarantine) transitively; fixes the backend contract
for the edit-022..024 read tools already planned independently.

## Status

Accepted (2026-08-23). Justification: §52 lists "git2 (or shelling out
initially)" as under evaluation and §8.12 sketches a "git2/rust binding
layer" — both hedges, not measurements. This ADR performs the required §52
dependency evaluation, records current upstream facts, and resolves the
hedge. It narrows §52's entry for git access only (ropey/notify/keyring
remain open) and realizes §8.12's layer over a different backend than the
sketch assumed; §8.12's invariants (agents never rewrite history or
force-push; worktrees are tracked resources) are unaffected.

## Context

The safe-editing engine's git surface (Z-DESKTOP-TASKS edit-022..029;
skills/z-git):

- Reads, auto-allowed: status (dirty-state input to fingerprint checks),
  diff (agent-facing change explanation), log (history context) —
  edit-022..024.
- Writes, approval-gated: add/commit (edit-026), branch (edit-027). All
  history-mutating operations are prohibited (skills/z-git rules 1–3);
  edit-028 converts that prohibition into enforcement tests.
- Later: worktree orchestration for sub-agents and Best-of-N (orch-007..009),
  merge/conflict assistance, cherry-pick.

Repo principles bearing on the choice: minimal-dependency discipline (§52);
blocking threads over async (ADR-0001) — all options here are synchronous or
wrap synchronously, so runtime compatibility is not the discriminator;
security posture — personal-first, but agents open third-party codebases, so
anything parsing repository content treats it as semi-trusted input;
developer-first audience (machines that already have git).

## Considered options

Version/license/maintenance facts verified 2026-08-23 via crates.io and
GitHub APIs plus direct inspection of upstream manifests:

| Option | Version (released) | License | MSRV | Native/deps cost |
|---|---|---|---|---|
| git2 (libgit2 bindings) | 0.21.0 (2026-05-18) | MIT OR Apache-2.0 | 1.87 | Vendored libgit2 1.9.x — 8.3 MB of C source compiled by the cc/pkg-config build script; libz-sys; opt-in `https`/`ssh` pull openssl-sys/libssh2-sys |
| git CLI (shell-out) | v2.52..v2.55 current (2026) | GPL-2.0-only, executed not linked | n/a | None: user-installed binary; zero crates; Cargo.lock untouched |
| gix (gitoxide) | 0.87.0 (2026-08-22) | MIT OR Apache-2.0 | 1.85 | Pure Rust; umbrella crate carries 71 direct deps (28 optional) |

Maintenance health: git2-rs pushed 2026-08-21, 15.5M recent (90-day)
downloads, regular 0.20.x/0.21 point releases; its libgit2-sys vendors
upstream libgit2 (0.18.8+1.9.7 shipped 2026-08-21 — eight days after the
libgit2 release it tracks). gitoxide pushes near-daily and releases roughly
monthly: fourteen releases 0.74.1→0.87.0 between 2025-10-23 and 2026-08-22,
with per-release MSRV movement recorded on crates.io (1.82 → 1.85 at 0.84).
git.git latest commit 2026-08-20; its security fixes ship through OS package
managers rather than application rebuilds.

Security posture, 2024–2026 record (NVD, retrieved 2026-08-23). libgit2
absorbed seven CVEs in 2026 alone: CVE-2026-5917 (published 2026-08-11,
CVSS 9.6 CRITICAL — shell-command injection through the libssh2 transport,
affecting v0.27.0–v1.9.0, fixed v1.9.7) and the CVE-2026-53583..53587
cluster (on NVD 2026-08-20; fixed in the 2026-07-18 security releases
v1.8.6/v1.9.5, including a heap-buffer-overflow in bundled PCRE reachable
through revspec parsing and an HTTP redirect/auth flaw, CVE-2026-53586,
CVSS 6.5), on top of CVE-2024-24575/-24577 (2024-02-06). Two structural
implications: bundled-libgit2 consumers inherit every parser defect as
in-process attack surface and propagate fixes only as fast as binding bump +
app release; the git CLI concentrates the same parsing in a separate process
that distributions patch. The CLI is not immune — 2.45.1 (2024-05) fixed the
submodule symlink RCE CVE-2024-32002, and the July-2025 maintenance releases
(2.43.7..2.50.1, per RelNotes/2.49.1.adoc) fixed seven CVEs including
CVE-2025-48384 — but those fixes reach users without a new Z Desktop build.
gix's memory safety is structural (pure Rust); upstream's own stability
ladder (README, retrieved 2026-08-23) places only gix-lock at Tier 1 and
gix-tempfile at Tier 2, leaving the core object/odb/status crates sub-1.0
stabilization candidates.

Mechanism precedent (main branches inspected 2026-08-23): Zed shells out —
its git crate declares no git2/gix dependency and spawns `git` via
`Command`, using `status --porcelain=v1 -z` and `worktree list --porcelain`;
VS Code's built-in git extension spawns the CLI; GitHub Desktop drives a
packaged CLI through dugite ^3.2.3. Cargo simultaneously ships both
libraries (git2 0.21 with `https`,`ssh`; gix 0.85 with `dirwalk`,`status`) —
proof that both are production-viable and neither is disqualifying.

Options:

**(a) Adopt git2-rs now.** Capability-complete for our surface, and its
default feature set is empty (OpenSSL/libssh2 stay out unless we enable
network transports we do not need — remotes stay user-driven per
skills/z-git). Rejected on three §52 grounds: MSRV 1.87 exceeds our 1.85
workspace floor, so a library choice would force a toolchain-bump side
decision; the vendored C tree adds multi-megabyte native build/binary weight
and places libgit2's demonstrated 2026 CVE stream inside our process, with
remediation gated on binding bumps and app releases; and every capability it
offers for status/diff/log/add/commit/branch/worktree is equally exposed by
the CLI our developers already run.

**(b) Shell out to the git CLI behind one thin internal facade.** Same
binary as the user: behavioral parity with their hooks, config, and version;
crash and parser-compromise isolation for free; zero new crates; naturally
synchronous (`std::process::Command` on a worker thread, ADR-0001-style).
Costs are real but bounded: spawn overhead (ms-scale; measured in edit-022),
machine-output parsing discipline, and reliance on an installed git —
standard on dev machines (macOS CLT, Git for Windows, distro packages) and
cheaply detected.

**(c) Adopt gix now.** Best paper fit: MIT/Apache dual license, MSRV 1.85
equals our floor exactly, pure-Rust safety, cargo-proven status/dirwalk
components. Rejected for now on churn-versus-scope: fourteen releases in ten
months and an upstream self-assessment putting the crates we would lean on
below Stability Tier 1 imply continuous bump-and-port work across a ~70-direct-dep
graph, to cover a git surface (three read verbs, two approved write verbs)
the CLI already handles. Named successor candidate, not today's answer.

**(d) Defer; ship reads on ad-hoc CLI calls, revisit at edit-026.**
Deferral relocates the decision without shrinking it: write tools would then
be designed against an undocumented backend. Absorbed into (b) — the facade
is decided now, library adoption becomes trigger-driven, and edit-026/027
proceed without a second ADR round-trip.

## Decision

Adopt (b): all Z Desktop git access goes through a single internal git
facade backed by the git CLI.

1. **Backend**: the user's installed `git`, invoked with direct argv
   (`std::process::Command`, never shell strings), from one serialized
   worker thread — single-flight ordering avoids concurrent
   `.git/index.lock` contention between reads and approved writes.
2. **Machine-readable output only**: status `--porcelain=v2 --branch -z`;
   diff via `--numstat -z` / `--raw -z`; log via explicit `--format` with
   `%x00` separators. Under `-z`, paths are emitted unquoted
   (core.quotePath becomes irrelevant); porcelain v2 is documented stable
   and extensible, so decoders ignore unrecognized keys. Human-facing text
   comes from stderr and is never parsed for logic; exit codes are
   authoritative; `LC_ALL=C` is set defensively for coarse classification.
3. **Environment**: reads run with `GIT_OPTIONAL_LOCKS=0` so background
   refreshes never take the index lock; approved writes run WITHOUT identity
   overrides, so the user's git identity, hooks, and signing configuration
   apply unchanged — approved writes are the user's writes (skills/z-git).
4. **Version gate**: `git --version` checked at project open; require ≥2.20
   (porcelain v2 shipped in 2.11, 2016 — the floor leaves margin); missing
   binary produces an actionable message naming the install step.
5. **Safety mapping preserved**: reads auto-allowed; add/commit/branch pass
   the approval gate (edit-026/027); the facade exposes no amend/rebase/
   reset/filter verbs, making edit-028's prohibition assertions structural
   rather than merely behavioral.
6. **No new dependencies**: nothing enters `[workspace.dependencies]`;
   Cargo.lock untouched; §52's git entry closes as "resolved: CLI facade".
7. **Successor clause**: if embedding ever proves necessary, gix is the
   designated candidate (license, MSRV 1.85, memory safety) and git2 stays
   declined (MSRV floor, C supply-chain record); adoption supersedes this
   ADR via a new one, behind the same facade interface.

## Consequences

**Parsing discipline**: exactly one module constructs arguments and decodes
output; callers receive typed structs. Decode is tolerant: split on NUL,
ignore unknown keys, handle rename entries' multiple path fields
explicitly. Format drift is contained by construction — no other module may
spawn git; edit-028 adds a guard test asserting the facade is the sole
`Command::new("git")` site.

**Error handling**: exit code plus captured stderr; timeouts with child kill
on cancel; `index.lock` presence classified as retryable; missing binary
surfaces at project open, not mid-operation. Failures produce actionable
messages per skills/z-safe-editing's DoD ("git not found — install …",
"index locked by another git process; retrying").

**Performance**: expectation pending measurement: spawn adds milliseconds
per call while status/diff/log remain dominated by the O(worktree/history)
work both backends perform; the shared on-disk index keeps repeated reads
warm. edit-022 records actual p95 figures against the
repository-intelligence budgets; a breach fires the gix benchmark trigger
below.

**Security/supply-chain**: repository parsing happens outside our process;
CLI CVE fixes arrive via OS updates at zero shipping cost to us. Remaining
exposure reduces to argv hygiene — paths passed verbatim after `scoped()`
canonicalization, no interpolation — and output-size caps on hostile-repo
stdout.

**Accepted debt**: reliance on an externally managed binary; porcelain v2
semantics owned upstream (stable by documentation since 2.11, extensible by
design). Revisit triggers (audit cadence per DEVELOPMENT-STATE):

- edit-026/027 landing: revalidate commit/branch/worktree flows end-to-end
  through the facade (hooks, signing, index transitions). Expected to hold;
  only CLI-semantics blockers warrant a superseding ADR.
- Measured read-path overhead breaches repository-intelligence budgets on XL
  corpora → benchmark gix (`dirwalk`/`status`) behind the facade before
  deciding.
- A distribution target without git available (sandboxed/non-developer
  packaging) → adopt gix, supersede this ADR.
- git announces a porcelain v2 breaking change → pin the maximum supported
  version in the gate, adapt the facade, record here.
- A git security advisory materially affecting our invocation patterns →
  reassess within one release cycle, including the gix trade.

## Sources

- crates.io API (retrieved 2026-08-23): git2 0.21.0 released 2026-05-18,
  MIT OR Apache-2.0, empty default feature set (`https`/`ssh` opt-in),
  15,545,752 recent downloads; gix 0.87.0 released 2026-08-22,
  rust_version 1.85, MIT OR Apache-2.0, fourteen releases 2025-10-23 →
  2026-08-22 (MSRV 1.82→1.85 at 0.84), 71 direct dependencies; libgit2-sys
  0.18.8+1.9.7 released 2026-08-21.
- git2-rs main-branch manifests (retrieved 2026-08-23): workspace
  rust-version 1.87; libgit2-sys/Cargo.toml — libc + libz-sys, optional
  openssl-sys (unix) / libssh2-sys, build-deps cc + pkg-config.
- GitHub API (retrieved 2026-08-23): rust-lang/git2-rs pushed 2026-08-21;
  Byron/gitoxide pushed 2026-08-23 (11.8k stars) with README stability
  tiers (gix-lock Tier 1, gix-tempfile Tier 2); libgit2 releases v1.9.7 /
  v1.8.7 (2026-08-13, security release disclosing CVE-2026-5917) and
  v1.9.5 / v1.8.6 (2026-07-18, security release incl. bundled-PCRE heap
  overflow via revspec); libgit2 languages: C 8,316,230 bytes; git/git
  tags v2.52.0..v2.55.0 plus 2.56 rcs, latest commit 2026-08-20.
- NVD (retrieved 2026-08-23): CVE-2024-24575 / CVE-2024-24577 (published
  2024-02-06); CVE-2024-32002 (2024-05-14, submodule RCE class);
  CVE-2026-5917 (2026-08-11, CVSS 9.6, ssh command injection, ≤v1.9.0);
  CVE-2026-53586 (2026-08-20, CVSS 6.5) within the CVE-2026-53583..53587
  cluster.
- git documentation, master (retrieved 2026-08-23): git-status.adoc —
  porcelain output "guaranteed" stable, "`-z` format recommended for
  machine parsing", paths quoted only without `-z`, Version 2 extensible;
  git.adoc — GIT_OPTIONAL_LOCKS; Documentation/RelNotes/2.49.1.adoc —
  CVE-2025-27613, -27614, -46334, -46835, -48384, -48385, -48386 fixed
  2025-07.
- Upstream code inspection (main branches, retrieved 2026-08-23):
  zed-industries/zed crates/git/Cargo.toml (no git2/gix dependency) and
  crates/git/src/repository.rs (Command-spawned git; `status
  --porcelain=v1 -z`; `worktree list --porcelain`);
  microsoft/vscode extensions/git/src/git.ts (spawn-based);
  desktop/desktop app/package.json (dugite ^3.2.3); rust-lang/cargo
  Cargo.toml (git2 0.21 `https`,`ssh` alongside gix 0.85
  `dirwalk`,`status`).
