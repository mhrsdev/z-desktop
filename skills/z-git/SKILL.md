---
name: z-git
description: Z Desktop git engineering — branches, worktrees, diffs, commits, safe automation rules, preserving user changes, GitHub publication workflow. Use for any git operation performed by agent or developer.
---

# Z Git Engineering

## When this skill applies
Any git operation: commits, branches, worktrees, pushes, or implementing
git features inside Z Desktop.

## Safety rules for agents operating git (absolute)

1. NEVER rewrite history (rebase/amend/filter) on shared or user branches.
2. NEVER `reset --hard`, checkout -- ., or clean without explicit approval.
3. NEVER force-push. If remote conflicts, inspect and report.
4. NEVER commit secrets, credentials, or data/ directories.
5. User's uncommitted work is sacred: stage selectively, never wholesale
   `git add -A` when unrelated changes exist.
6. Worktrees created by agents are tracked and cleaned up; no orphan dirs.

## Commit practice

- Meaningful messages: `feat:`/`fix:`/`docs:`/`perf:`/`test:` prefixes,
  subject ≤ 72 chars, body explains WHY when non-obvious.
- Logical grouping: source baseline / docs / tooling as separate commits
  where practical; no artificial micro-commit spam.
- Canonical remote: https://github.com/mhrsdev/z-desktop.git (origin).
- Push only verified states: tests green, security scan clean.

## In-app git features (planned surface)

status/diff/stage views → branch management → worktree orchestration for
sub-agents → merge/conflict assistance → cherry-pick tooling. All read
operations auto-allowed; ALL history-mutating operations require approval.

## Repository hygiene

- .gitignore excludes: target/, data/, references/external/, external
  clones, secrets, OS junk. Keep it accurate; audit before each push.
- tools/security_scan.py runs pre-push; findings must be triaged (real vs
  test fixture) and documented.

## Definition of Done

Git operations complete with: clean status for touched paths, tests green,
scan clean, push verified by SHA comparison.