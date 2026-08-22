# Z Desktop — Reference Research & Clone Playbook

> **Purpose**: how to study external projects for ideas, what may be
> reused under which conditions, and how to keep the reuse ledger honest.
> **Companion docs**: `docs/THIRD_PARTY_RESEARCH_AND_REUSE.md` (ledger),
> `docs/research/*.md` (findings), Master Spec §24 (classification).

---

## 1. Rules of Engagement

1. **Study, don't copy blindly.** Every borrowed idea gets a classification:
   INSPIRE / ADAPT / REIMPLEMENT / REUSE / REJECT.
2. **License first.** Before any code-level reuse: read the license. MIT /
   Apache-2.0 → REUSE possible with attribution. GPL/AGPL → architecture
   reference only (INSPIRE), never code. Closed source → docs/release-notes
   study only.
3. **Ledger entry before merge.** Any reused code or adapted design is
   recorded in `docs/THIRD_PARTY_RESEARCH_AND_REUSE.md` with source URL,
   license, and what was taken.
4. **References stay out of git.** Cloned repos live in `references/`
   (gitignored). Never commit third-party trees.
5. **No vendoring without decision.** Vendored code requires an ADR
   documenting why a dependency wasn't used instead.

## 2. Study Targets & What to Extract

| Project | License | Extract | Avoid |
|---|---|---|---|
| xai grok-build | check repo | agent loop structure, steering, compaction, task durability | any direct code without license check |
| OpenAI Codex CLI | Apache-2.0 | sandbox/approval UX patterns, Rust structure | cloud assumptions |
| Cline | Apache-2.0 | task lifecycle, checkpoint UX, MCP breadth | VSCode coupling |
| OpenHands | MIT | event model, runtime isolation, eval harnesses | Python-centric runtime |
| OpenCode | check repo | provider abstraction, client/server split | server-first assumptions |
| Aider | Apache-2.0 | repo-map concept, edit formats, lint/test loops | git-heavy workflow assumptions |
| Gemini CLI | Apache-2.0 | checkpointing, extensions, non-interactive runs | telemetry defaults |
| Zed | GPL/AGPL mix | GPUI-class rendering concepts (architecture reading) | ANY code copying |
| Claude Code / Cursor / Windsurf | closed | release notes, documented behaviors | nothing else |

## 3. Clone Procedure

```bash
# from workspace root; references/ is gitignored
git clone --depth 1 <url> references/<name>
```

Then: skim README + license + top-level structure; write findings into
`docs/research/<name>-notes.md` with the classification table above;
update the reuse ledger if anything concrete was adopted.

## 4. Findings Index

| Document | Status |
|---|---|
| docs/research/grok-build-dissection.md | complete |
| docs/research/capability-matrix.md | complete |
| docs/research/differentiating-capabilities.md | complete (170-item backlog) |

New studies append here with date + one-line takeaway.

## 5. Refresh Cadence

Re-check active targets every ~2 months or at major releases of the
studied project. Stale findings are marked as such rather than deleted
(history has value).