---
name: z-cross-platform
description: Z Desktop cross-platform engineering — Windows/Linux/macOS, x86-64/ARM64, filesystem differences, shells, process handling, GPU backends. Use when writing platform-conditional code or porting features.
---

# Z Cross-Platform

## When this skill applies
Any `#[cfg]` usage, platform-specific dependency, path/shell/process
handling, or porting work.

## Platform matrix

| Concern | Windows | Linux | macOS |
|---|---|---|---|
| Process tree | Job Objects (KILL_ON_JOB_CLOSE) | process groups + SIGKILL | process groups + SIGKILL |
| Shell | cmd /d /s /c (skip AutoRun) | sh -c | sh -c |
| Paths | verbatim \\?\ prefix — strip via char codes | POSIX | POSIX |
| Secrets | DPAPI/keychain planned | keyring planned | keychain planned |
| GPU | D3D12 primary | Vulkan primary | Metal primary |

## Rules

1. Platform code is isolated in cfg-gated modules with identical trait
   surfaces (see sandbox.rs Guard enum pattern). Callers never branch on OS.
2. Never assume case sensitivity, path separators, or executable extensions
   (.exe). Use std::path APIs and MAIN_SEPARATOR consciously.
3. ARM64 is a first-class target: no x86 intrinsics without a runtime-
   dispatched fallback; CI must cover both arches eventually.
4. Line endings: text tools normalize on read/write boundaries; never
   compare raw bytes of user-edited files across platforms.
5. Feature parity tracking: the task ledger tags per-platform tasks;
   shipping a feature on one platform requires an explicit decision to
   defer others, recorded in the ledger.

## Testing expectations

- Platform tests are cfg-gated and run in the normal suite on their host.
- Sandbox tests exist for both job-object and process-group paths.
- CI matrix (when it lands): windows-latest, ubuntu-latest, macos-latest,
  plus arm64 runners as available.

## Definition of Done

- No new unconditional Unix-only or Windows-only API calls outside gated
  modules; cross-platform behavior covered by tests on at least the dev
  platform with documented status for others.