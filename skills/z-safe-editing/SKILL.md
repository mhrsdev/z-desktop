---
name: z-safe-editing
description: Z Desktop safe editing engine — file fingerprints, stale edit prevention, structured patches, atomic writes, rollback, user-edit preservation, multi-agent conflict prevention. Use when implementing or reviewing any file-modification feature.
---

# Z Safe Editing

## When this skill applies
Any code path that writes to user files: fs_write tool, patch application,
worktree merges, refactors, generated-file updates.

## Non-negotiable rules

1. **Fingerprint before write**: capture the target file's content hash when
   the agent READS it; verify at WRITE time. Mismatch = stale edit → refuse
   and force re-read. This prevents clobbering concurrent user edits.
2. **Atomic writes**: write temp file + rename, never in-place truncation.
   A crash mid-write must leave either old or new content, never garbage.
3. **Scope check**: every path passes through the scoped() boundary check
   (lexical normalization against canonical project root; traversal and
   symlink escapes rejected). Do not bypass it "just this once".
4. **Rollback**: multi-file operations stage changes so they can be undone;
   journal records enough to reverse completed steps.
5. **User edits win**: if the user modified a file after the agent read it,
   the agent's change is rejected with a clear message, not merged blindly.

## Patch formats

- Full-content replace is allowed only for files the agent just read fully.
- Structured patches (search/replace blocks, line-anchored diffs) are the
  default for large files; they carry their own context fingerprints.
- Never apply a patch whose anchor text is absent — that means drift; fail.

## Multi-agent conflicts

Write grants are exclusive per file (see z-subagents skill). Merge-time
fingerprint checks catch races between worktree branches.

## Testing expectations

- Stale-write test: modify file between read and write → write refused.
- Atomicity test: simulate crash (kill writer) → file intact.
- Traversal tests already exist in tools.rs — keep them green.
- Rollback test: failed multi-step op leaves zero partial state.

## Definition of Done

- No new write path without fingerprint + atomicity + scope checks.
- Failure modes produce actionable messages ("file changed since you read
  it — re-read"), never silent corruption.