---
name: z-research
description: Z Desktop research and reuse workflow — external source analysis, license verification, SHA pinning, adaptation vs copying, concept extraction. Use before studying any external project or reusing external code/ideas.
---

# Z Research & Reuse

## When this skill applies
Studying an external project for architecture ideas; considering code reuse;
recording research outcomes.

## Workflow

1. **Verify identity first**: confirm the official repository (owner + name)
   before cloning. Name similarity is NOT proof. Archived repos are marked
   HISTORICAL_REFERENCE.
2. **Clone policy**: `git clone --filter=blob:none <repo>
   references/external/<project>` — only when actively researching that
   subsystem. Never eagerly clone everything. Clones are git-ignored and
   NEVER pushed.
3. **Pin**: record repo URL, commit SHA (`git -C <dir> rev-parse HEAD`),
   inspection date, license, subsystem studied.
4. **Classify each finding**: INSPIRE (idea only) / ADAPT (reimplement in
   our design) / REUSE (copy requires license review) / REJECT (with reason).
5. **Record** in docs/research/<topic>.md and update
   docs/THIRD_PARTY_RESEARCH_AND_REUSE.md for anything reused.

## License rules

- REUSE requires explicit license compatibility review recorded in the
  third-party ledger (origin, SHA, license, files, modifications,
  attribution).
- GPL-family code is treated as architecture reference ONLY unless a
  specific component's license is separately verified compatible.
- Closed-source products: study official docs/release notes/engineering
  posts only. NEVER clone unofficial mirrors or source dumps.

## Anti-patterns

- Copying code "temporarily" without ledger entry.
- Treating an archived project's patterns as current best practice.
- Burning hours on competitor comparison without an engineering decision.

## Current pinned references

See docs/Z-DESKTOP-REFERENCE-RESEARCH.md (playbook) and
docs/THIRD_PARTY_RESEARCH_AND_REUSE.md (ledger). grok-build dissection:
docs/research/grok-build-dissection.md.

## Definition of Done

Research produces: pinned reference entry, classified findings, and at
least one concrete engineering decision or task ID — not just notes.