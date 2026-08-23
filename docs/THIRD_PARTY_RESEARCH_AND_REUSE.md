# Third-Party Research & Reuse Ledger

Rule: no code enters the Z Desktop source tree without an entry here recording
origin, license, files, modifications, and attribution.

## Research references (not reused, read-only)

| Repository | Location | License | Commit/Rev | Purpose |
|---|---|---|---|---|
| xai-org/grok-build | `references/external/grok-build` | Apache-2.0 (first-party); vendored deps keep own licenses | SOURCE_REV `7d67deacbeb1c1093fdb4f9bcbfab2630e18a6aa`; re-cloned 2026-08-23, HEAD `07b2f7144fd5c5c9d3dd1966937a87852d2dbdb8` | Architecture dissection; steering study (`xai-interjection-core`: capped event-queue buffer, "User sent a message while you were working" framing, truncation threshold) |
| deepseek-ai/deepseek-harness | `deepseek-harness/` | See its LICENSE (review pending) | shallow clone 2026-08-22 | User-requested reference |

## Reused code

None yet.

## Adapted concepts (no code copied)

See `docs/research/grok-build-dissection.md` — each subsystem records what was
adapted conceptually. Concepts are not copyrightable; implementations are ours.