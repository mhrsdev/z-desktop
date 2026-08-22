---
name: z-repository-intelligence
description: Z Desktop repository intelligence — tree-sitter parsing, AST/symbol/reference graphs, imports, dependency graphs, incremental indexing, affected-file analysis, code search. Use when working on repo.rs, index actors, parsers, symbol lookup, or retrieval quality.
---

# Z Repository Intelligence

## When this skill applies
Work on `z desktop/crates/z-core/src/repo.rs`, future tree-sitter/parsing
actors, symbol/reference storage, incremental re-indexing, or any feature
that answers "what is in this project and what does changing X affect".

## Current state (verify first)

`RepoIndex` today: file walk (skips .git/node_modules/target/dist/build/
__pycache__/.next/venv), lightweight symbol extraction, map_text() for the
system prompt, file_count/symbol_count. This is the seed, not the target.

## Target architecture

- **Index actor**: a single owning thread; requests via channel; snapshots
  out. Never share mutable index state across threads.
- **Parse layer**: tree-sitter grammars per language; parse to concrete
  syntax trees; extract symbols (defs), references, imports.
- **Storage**: per-file records keyed by content fingerprint (hash of bytes
  + mtime). Unchanged fingerprint = reuse cached AST results untouched.
- **Graphs**: symbols → definitions/references; files → imports; modules →
  dependencies. Store edges, not prose.
- **Incremental update**: on file-change events, re-parse only changed files,
  diff their symbol sets, update reverse-reference edges for affected files.
- **Retrieval**: lexical search (trigram or token inverted index) first;
  structural search (by symbol kind/edge type) second; semantic/embedding
  search later and always as a supplement, never a replacement for exact
  source access.

## Invariants

1. The index is a CACHE. Any answer used for an edit must be rehydrated from
   the real file at edit time. Stale index data must never cause a wrong
   write.
2. Indexing must never block the UI thread or the agent turn thread.
3. A corrupt/missing index degrades gracefully to "no map", never to a crash.
4. Memory is bounded: million-file repos require on-disk spill or compact
   structures, not one giant in-memory graph (design for it from day one).

## Scale expectations

Design targets: initial index of a 100k-file repo in minutes not hours;
incremental update of one file < 50 ms; symbol lookup < 10 ms; search p95
< 200 ms on medium repos. Measure with synthetic fixtures before claiming.

## Testing expectations

- Fixture repos under test fixtures (small deterministic trees).
- Incremental correctness: change one file, assert ONLY its symbols changed.
- Fingerprint collision/negative tests (same mtime, different content).

## Definition of Done

- Actor isolation proven by tests (no data races possible by construction).
- Incremental path has parity tests vs full rebuild.
- Benchmarks recorded in docs when a scale milestone lands.