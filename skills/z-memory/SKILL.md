---
name: z-memory
description: Z Desktop memory architecture — working, session, project, semantic and episodic memory; decisions, consolidation, invalidation, provenance. Use when designing or implementing any persistent agent memory feature.
---

# Z Memory

## When this skill applies
Designing/implementing memory layers, decision records, cross-session
recall, memory consolidation, or memory UI surfaces.

## Memory layers

| Layer | Scope | Lifetime | Example |
|---|---|---|---|
| Working | current turn | ephemeral | tool results in flight |
| Session | one thread | session + journal | conversation history |
| Project | one repo | until invalidated | "tests run with cargo test -p z-core" |
| Semantic | global facts | long-lived, confidence-scored | user preferences, learned conventions |
| Episodic | past task narratives | long-lived, compressed | "how we migrated the protocol last month" |

## Rules every layer must obey

1. **Provenance**: every memory records where it came from (message id,
   tool call, user statement). Unprovenanced memories are garbage.
2. **Confidence & superseding**: newer evidence supersedes older; both are
   retained for audit; retrieval prefers the winner.
3. **Invalidation**: memories tied to file fingerprints or repo state die
   when their anchor changes. Stale detection runs at retrieval time.
4. **User control**: the user can view, correct, and delete any memory.
   Correction is a first-class operation that also supersedes dependents.
5. **Consolidation**: episodic entries compress into semantic facts only via
   an explicit consolidation pass — never silently during a turn.

## Anti-patterns (forbidden)

- Memory as an unqueryable blob ("the model will figure it out").
- Injecting unbounded memory into prompts (memory enters context through
  the context engine's budget/priority system, capped).
- Treating chat history AS project memory (chat is not project state).
- Silent TTL expiry of user-corrected facts.

## Storage direction

Append-only journal events → materialized views per layer. Replay rebuilds
all layers from the journal; views are caches, the journal is truth.

## Testing expectations

- Superseding/conflict tests (two contradictory facts resolve deterministically).
- Invalidation tests (anchor change removes dependent memories).
- Replay tests (journal replay reproduces identical memory state).

## Definition of Done

- Every memory write path records provenance + confidence.
- User-facing inspection/correction surface exists before the layer is
  considered shipped.