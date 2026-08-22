---
name: z-security
description: Z Desktop sandbox and security — command execution, process isolation (Job Objects/process groups), secrets handling and redaction, filesystem boundaries, plugin/MCP permissions, destructive actions. Use for any security-relevant change or review.
---

# Z Sandbox & Security

## When this skill applies
Any change to process execution, filesystem access, secret handling,
plugin/MCP permission surfaces, or anything that could widen an attack
surface. When in doubt, treat the change as security-relevant.

## Implemented defenses (verify in source before relying on them)

- **Process tree isolation** (`z-core/src/sandbox.rs`): Windows Job Objects
  with KILL_ON_JOB_CLOSE; children spawn CREATE_SUSPENDED and are assigned
  to the job BEFORE resume — no escape window. Unix: own process group +
  group SIGKILL. No breakaway flag → `start`/detached grandchildren stay in
  the job. Regression tests prove grandchild death on timeout AND normal
  exit.
- **Bounded execution**: default 120 s timeout, hard ceiling 600 s; partial
  output preserved on kill; output capped (8 MiB stdout / 2 MiB stderr).
- **Filesystem scope** (`tools.rs scoped()`): lexical normalization against
  canonical project root; `..` traversal rejected; not-yet-existing write
  targets checkable; Windows verbatim-prefix handled via char codes.
- **Secret redaction** (`z-core/src/redact.rs`): fingerprinted redaction of
  provider tokens, bearer headers, key=value assignments on ALL tool output.
- **Risk classification**: read-only tools auto-allowed in-scope; writes and
  execution require explicit user approval through the gate.

## Rules for new capabilities

1. Every new tool declares a Risk. Unknown tools fail CLOSED as Execute-risk.
2. Never log, persist, or transmit unredacted secrets. New output surfaces
   route through redact().
3. Plugin/MCP permissions are deny-by-default; grants are explicit, scoped,
   revocable, and surfaced in UI.
4. Destructive operations (delete, overwrite outside scope, force git ops)
   always require approval — no "trusted mode" bypasses.
5. Security fixes come with regression tests reproducing the attack.

## Known debt (do not re-introduce)

- Mid-run cancellation of in-flight tool calls is not yet possible (tool
  runs synchronously); cancel flag inside sandbox wait loop is planned.
- Redaction covers tool output; runtime logs + journal events need it too.
- BYOK config.json stores keys plaintext locally; OS-keychain storage is
  planned (DPAPI/keyring).

## Review checklist for security-relevant PRs

- [ ] Scope checks on every path input
- [ ] Timeout + tree-kill on every spawn
- [ ] Redaction on every new output surface
- [ ] Fail-closed defaults
- [ ] Tests reproduce the threat

## Definition of Done

Security features are done when their regression tests fail without the fix
and pass with it — demonstrated, not asserted.