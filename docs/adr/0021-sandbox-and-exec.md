# ADR-0021: Command execution & sandbox posture

Ledger: fixes the execution-isolation contract behind term-004 (kill-on-close
via job/group guards — the mechanism already ships in `sandbox.rs`, its ledger
row is still `[PLANNED]` pending a test-evidenced flip), scopes the seam that
sec-010..014 (MCP behind Risk machinery, extension permission system, egress
policy) were planned against, and records what Z Desktop adopts from OpenAI
Codex's exec/sandbox/approval machinery — the "Rust agent architecture,
sandbox/approval UX" reference §93 already names (:1174). Unblocks
term-001..003 (PTY trait/backends), which inherit the same spawn choke point.

## Status

Accepted (2026-08-24). Justification: the trust boundary is already binding —
§16.1 fixes "Core ──sandboxed──▶ agent-spawned processes", principle 15 makes
"scope checks, sandboxing, and approval gates … not removable conveniences"
(:257), §16.2 lists the implemented mechanisms (suspended-spawn job
assignment, tree kill on timeout, redaction, fail-closed risk classification,
approval gate), and §28.5 pins `terminal_exec` to them. What nothing decides
yet is how far that isolation must *grow* — toward OS-enforced
filesystem/network confinement or not — on which code seam, and with what
fallback when an OS primitive is unavailable. This ADR decides those against
Codex's shipped design instead of re-deriving them under pressure later.

## Context

**Z Desktop today** (single-user personal desktop app; Rust; threads-over-
async per ADR-0001). Every agent command takes one path:
`tools::classify` assigns a `Risk` (unknown tools fail closed to Execute,
tools.rs:63–75; enum in z-protocol lib.rs:29) → non-ReadOnly calls emit
`ApprovalRequested` and park on the approval gate until the UI resolves or
`approval_timeout_secs` (default 300 s) expires (runtime.rs:69–99,
:1331–1358) → `terminal_exec` calls `sandbox::run(command, project_root,
timeout)` (tools.rs:609). The sandbox layer implements the §16.2 contract:
mandatory wall-clock budget clamped to 120 s default / 600 s ceiling
(sandbox.rs:24–26); own process group on unix, CREATE_SUSPENDED plus
kill-on-close Job Object assignment-before-resume on Windows so no child
instruction executes outside the job (:175–204, :260–297); concurrent pipe
drain capped at 8 MiB stdout / 2 MiB stderr (:114–119, :155–171) feeding the
10 MiB scrollback ring (term-005/018, SCROLLBACK_CAP :29); partial output
preserved through timeout kills; output redacted before the model sees it
(tools.rs:613–614).

Honest gaps against §16.4's threat model ("protect the user from the AGENT"):
an approved command inherits the parent's full environment — including any
secret the process holds in env vars — has unrestricted network egress, may
write anywhere the user can, and nothing detects or reports confinement
denials because nothing confines. The approval gate is one wall, evaluated
per call, never per capability. Two bookkeeping gaps deserve naming: term-004
is `[PLANNED]` (Z-DESKTOP-TASKS.md:937) while its mechanism exists in
`sandbox.rs` with grandchild-survival regression tests — the row needs an
evidenced flip, not reimplementation; and the module doc's orphan-safety claim
is weaker than it reads on unix — `Guard::Group` has no `Drop`, so "the group
is killed on drop best-effort" (:8–11) is aspirational there, while Windows
gets true KILL_ON_JOB_CLOSE reaping (:333–340).

**What Codex actually does** (reference clone `references/external/codex`,
codex-rs HEAD `068c49f075cf`, inspected 2026-08-24):

1. *Backend selection.* `SandboxType {None, MacosSeatbelt, LinuxSeccomp,
   WindowsRestrictedToken}` (sandboxing/src/manager.rs:37) chosen by target
   OS (`get_platform_sandbox`, :62); the Windows backend additionally
   requires an opt-in flag threaded through config.
2. *Policy shape.* `SandboxPolicy` (protocol/src/protocol.rs:1010):
   DangerFullAccess | ReadOnly{network_access} | ExternalSandbox |
   WorkspaceWrite{writable_roots, network_access, exclude_tmpdir_env_var,
   exclude_slash_tmp}. `WritableRoot` (:1066) carries read_only_subpaths plus
   protected_metadata_names so `.git/hooks`/`.codex` under a writable root
   stay read-only — explicit anti-privilege-escalation carve-outs.
   `SandboxMode` (config_types.rs:104) defaults to read-only.
3. *Linux enforcement is layered.* Filesystem via bubblewrap namespaces
   (linux-sandbox/src/linux_run_main.rs; WSL1 refused outright); network via
   a seccomp filter installed on the spawning thread after PR_SET_NO_NEW_PRIVS
   (landlock.rs:42, :120, :169) denying ptrace/process_vm_readv/connect and
   restricting socket()/socketpair() to AF_UNIX unless proxy-routed
   (:188–246). A Landlock ruleset survives as legacy/fallback backend: whole-
   disk read + writable_roots rw + /dev/null at ABI V5 BestEffort, where a
   NotEnforced result is a hard error — fail-closed (:137–165).
4. *macOS:* Seatbelt SBPL profiles generated per-policy from include_str!'d
   templates (sandboxing/src/seatbelt.rs:24–30), executed via fixed absolute
   path `/usr/bin/sandbox-exec` (:56 — defends against PATH injection),
   args `-p <policy>` (:1022).
5. *Windows:* restricted token via CreateRestrictedToken
   (windows-sandbox-rs/src/token.rs:481), deny-write/deny-read ACLs (acl.rs,
   workspace_acl.rs, deny_read_acl.rs), optional private desktop, network
   denial via WFP filters bound to the sandbox account SID (wfp.rs:79); an
   elevated backend variant exists (sandboxing/src/windows.rs:37).
6. *Network proxy mode* turns blanket denial into audited egress: a managed
   MITM proxy with per-host approval decisions
   (`ReviewDecision::NetworkPolicyAmendment`) and a ProxyRouted seccomp mode
   letting IP sockets reach only the local bridge (landlock.rs:219–246).
7. *Approval ladder.* `AskForApproval` (protocol.rs:924): UnlessTrusted |
   OnRequest (default) | Granular(GranularApprovalConfig) | Never; decisions
   (`ReviewDecision`, :3883) include ApprovedForSession (session-scoped
   approval cache), ExecpolicyAmendment (persist a command-prefix allow rule,
   execpolicy/src/policy.rs:109–128), NetworkPolicyAmendment, Denied,
   TimedOut. The model may request elevated permissions per call
   (approvals.rs:25); requesting escalation under a Never policy is rejected
   with an actionable string back to the model
   (exec_command.rs:292).
8. *Denial→escalation loop.* Sandboxes deny silently, so Codex heuristically
   classifies failures as denials: keyword scan ("operation not permitted",
   "permission denied", …) plus exit 128+SIGSYS for seccomp, excluding
   ordinary shell exit codes 2/126/127 (sandboxing/src/denial.rs:13–40); a
   hit becomes `SandboxErr::Denied` (core/src/exec.rs:787), which surfaces as
   a user approval request — the sandbox failure IS the escalation trigger.
9. *Confinement markers.* children get CODEX_SANDBOX_NETWORK_DISABLED=1 /
   CODEX_SANDBOX=<backend> so scripts and tests can detect their own
   confinement honestly (core/src/spawn.rs:21, :26, :87).
10. *Patches bypass the shell.* `apply_patch` commands are intercepted out of
    exec into a dedicated parser/executor crate with its own upstream safety
    assessment (exec_command.rs:321; runtimes/apply_patch.rs:141–145) rather
    than generic exec approval.

Codex can afford four backends, a network proxy, and an escalation socket
wrapper (patched shells report intercepted exec() calls out-of-band over
CODEX_ESCALATE_SOCKET; shell-escalation/src/unix/escalate_protocol.rs:11,
EscalationDecision Run/Escalate/Deny :37) because it ships to millions across
three OSes for users who did not write it. Z Desktop is one user's machine
defending itself against its own agent; the design should be sized to that.

## Considered options

**(a) Approval-gate-only forever** (today's posture, formalized). Zero new
code, but a single per-call prompt is the entire defense: nothing sits behind
human attention, and approval is all-or-nothing (once granted, a command gets
full env, full disk, full network). §16.4 names prompt-injected agents
exfiltrating secrets as a primary threat — precisely the case a prompt cannot
stop when the agent asks nicely first. Rejected.

**(b) Port Codex's stack wholesale now** (bubblewrap + seccomp + SBPL +
restricted tokens + managed proxy). Maximum fidelity, maximum cost: four
platform specializations maintained by a personal project before any user
need appeared; most of the surface (proxy mode, granular config, escalation
sockets, prefix-rule policy engine) solves multi-model/multi-host problems we
do not have. Rejected as a unit; cherry-picked by option (e).

**(c) Containers-by-default** (every terminal_exec inside docker/podman). The
right tool for CI runners, wrong for a personal desktop product: requires a
daemon most desktop machines do not run, adds seconds of startup and
volume-mounting complexity to every command, breaks native/GPU tooling UX,
and the marginal containment beyond Phase-2 OS primitives matters mainly for
hostile multi-tenancy — not one user's machine. Rejected; re-evaluate only if
the extension ecosystem (sec-010..014) requires running untrusted third-party
toolchains routinely.

**(d) Full VM / microVM per command.** Strongest isolation available;
GB-scale images, multi-second boot per call, virtualization requirements, and
file/git-credential sharing becomes a sync problem that erodes the benefit
for our core workflow (an agent editing THIS repo). Cost/benefit fails at
interactive personal scale. Rejected.

**(e) Phased native isolation behind a trait seam.** Keep Phase 1 cheap and
real, add OS enforcement where the platform provides it, degrade gracefully
where it does not, and grow capability granularity last. Chosen (D1–D3).

**(f) Adopt Codex's denial-heuristic auto-escalation now.** Attractive
symmetry — their sandbox failure triggers the approval prompt — but we have
nothing to deny yet, and keyword-heuristic false positives would nag a single
user who is also the admin. Deferred to diagnostics-only in D4.

## Decision

### D1 — Phase 1 (now): harden the existing choke point (term-004 + hygiene)

`sandbox::run` remains the ONLY spawn path (§16.2 invariant; the git facade
keeps its direct-argv exception per ADR-0008). Three additions:

1. **Finish term-004 honestly**: the guard code exists; land the unix orphan
   fix (`prctl(PR_SET_PDEATHSIG)` after spawn, or a `Drop` on `Guard::Group`)
   plus a regression test, then flip the ledger row with evidence — closing
   both the ledger skew and the real gap named in Context.
2. **Env scrubbing**: build the child environment from a small explicit
   allowlist (PATH, HOME, TMPDIR, LANG/LC_ALL, TERM) instead of inheriting
   everything — closes the `env`/`printenv` secret-dump exfiltration path.
   Build-tool commands work through PATH+HOME; growing the list is a named-
   constant change, deliberately not user config.
3. cwd pinning stays `project_root`; timeout clamp, pipe caps, and the
   scrollback ring are unchanged.

No new dependencies.

### D2 — Phase 2: platform sandbox behind a trait seam, graceful no-op fallback

New module (proposed `z-core/src/isolation.rs`) defining a minimal policy
struct `{write_roots: Vec<PathBuf>, network: bool}` and a small trait with
per-platform impls: **macOS Seatbelt** (generated profile; keep Codex's fixed
absolute `/usr/bin` lesson), **Linux Landlock** (whole-disk read +
write_roots rw, NotEnforced → treat as unavailable — the bwrap subprocess is
deliberately NOT adopted: one less external binary to locate or bundle, and
our single-workspace policies fit Landlock ABI V5), **Windows restricted
token** (+ ACL carve-outs). An availability probe runs at startup; an
unavailable primitive selects a NoopFallback impl that logs ONE warn per
session and provides Phase-1 guarantees only — degradation never blocks the
tool. Ambiguous enforcement degrades loudly rather than pretending. Each
platform impl introduces its dependency through the §52 ADR process
(ADR-0007 precedent). Confinement markers ship here (`ZD_SANDBOX=<backend>`
env var, mirroring Codex's markers).

### D3 — Phase 3: per-tool capability grants on the existing Risk system

Extend static `classify` Risks with per-call grant requests (execute,
network, write-outside-root), resolved through the existing ApprovalGate —
same event flow, richer detail strings. Add session-scoped approval caching
(Codex `ApprovedForSession` semantics) keyed on (tool, capability, normalized
args prefix), persisted as shape-only journal records (both serde arms, house
rule). This is the seam sec-010 (MCP behind Risk machinery) and sec-011
(extension permission system) already assume; `.git/` and the app data dir
become read-only carve-outs inside any writable root, adopting Codex's
protected-metadata lesson.

### D4 — Escalation semantics stay pre-execution

Approval remains BEFORE execution (current design), not Codex's deny-then-
escalate. After Phase 2 lands, port denial classification for diagnostics
only (journal record + warn naming the likely denied path/syscall), never as
automatic prompt generation — heuristic false positives cost a solo user
trust in the prompt channel.

### D5 — Patches keep their dedicated pipeline

edit_patch/fs_write already run through the in-process safe-editing pipeline
(edit-*); we do not adopt Codex's shell-intercept approach to patching. Its
reason to exist (patches arriving through shell strings) does not exist here.

## Consequences

**Immediate shape**: Phase 1 ≈ one prctl/Drop fix + an allowlist env builder +
tests in sandbox.rs/tools.rs; Phase 2 = new module + three platform impls +
probe + noop fallback, one §52 ADR per platform dependency; Phase 3 =
classify/gate extension + session cache + journal kind. No protocol changes
in Phases 1–2.

**Accepted debt**: (1) unix orphan safety stays best-effort until D1.1 lands
— tested-for on Windows only today; (2) NoopFallback means platforms without
their primitive quietly run Phase-1-strength isolation — mitigated by the
one-warn-per-session and a future UI badge, not solved; (3) env scrubbing
breaks exotic commands needing inherited variables until the allowlist grows
— deliberate friction whose escape hatch is a code change, not config;
(4) Landlock over bubblewrap trades Codex-proven namespace coverage for zero
external binaries — acceptable while policies remain single-write-root.

**Revisit triggers**: extension/MCP work lands (sec-010..014) → D3 must exist
first, and option (c) is re-evaluated for third-party toolchains specifically;
any journal/warn evidence of out-of-scope writes or egress attempts → pull
Phase 2 forward and reconsider denial-driven prompts (option f); a second
writable root becomes necessary (worktrees per the spec isolation ladder) →
grow the policy struct before growing the trait; the Rust landlock crate
stalls → re-open bubblewrap; Windows restricted-token maintenance proves
heavier than Job Objects alone → ship Landlock/Seatbelt first, keep Windows
at Phase 1 + Job Objects.

## Sources

- Repo inspection (2026-08-24, z-desktop working tree @ `76581a699757`):
  `z desktop/crates/z-core/src/sandbox.rs` — timeouts (:24–26), ring cap
  (:29), run/spawn/guard/job-object pipeline (:93–341), missing Group Drop
  (:207–238 vs doc claim :8–11); `tools.rs` — classify (:63–75),
  terminal_exec (:598–634), redaction (:613–614); `runtime.rs` —
  ApprovalGate (:69–99), risk gating (:1331–1358), write grants (:1377ff);
  `settings.rs:21`; `z-protocol/src/lib.rs:29`.
- docs/Z-DESKTOP-TASKS.md (retrieved 2026-08-24): term-004 (:937), sec-*
  section (:1419–1476).
- docs/Z-DESKTOP-MASTER-SPEC.md (retrieved 2026-08-24): principle 15 (:257),
  §16.1–16.4 (:1023–1058), §17 failure row (:1068), §28.5 terminal_exec
  (:1369–1376), §93 Codex reference row (:1174).
- docs/adr/0007 (dependency ADR process), docs/adr/0008 (git facade argv
  exception), docs/adr/0017 (house style).
- OpenAI Codex `codex-rs` @ `068c49f075cf` (inspected 2026-08-24): line cites
  inline above — sandboxing/src/{manager,seatbelt,denial,windows,spawn}.rs,
  linux-sandbox/src/landlock.rs, protocol/src/{protocol,config_types,
  approvals}.rs, core/src/{exec,spawn}.rs,
  core/src/tools/handlers/unified_exec/exec_command.rs,
  core/src/tools/runtimes/apply_patch.rs, shell-escalation/src/unix/
  escalate_protocol.rs, windows-sandbox-rs/src/{token,wfp}.rs,
  execpolicy/src/policy.rs.
