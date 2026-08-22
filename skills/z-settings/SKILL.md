---
name: z-settings
description: Z Desktop settings architecture — User Mode vs Developer Mode, schema-driven settings, search, progressive disclosure, presets, safe defaults, migrations. Use when adding any user-configurable option.
---

# Z Settings

## When this skill applies
Adding or changing any configurable behavior, preference, or toggle.

## Two modes

- **User Mode** (default): only decisions a normal user makes — provider
  key/model, project folder, theme, a handful of agent behaviors. Under 20
  visible options at launch.
- **Developer Mode**: full control — context/token policy, cache behavior,
  tool grants, MCP permissions, panel/layout internals, animation, resource
  limits, diagnostics verbosity, experimental features. Progressive
  disclosure: categories → sections → expert fields.

## Architecture rules

1. **Schema-driven**: settings are declared as schema (id, type, default,
   range, category, mode-visibility, restart-required). UI renders from the
   schema; no hand-built form per setting.
2. **Searchable**: every setting reachable via search by name/description.
3. **Safe defaults**: the default must be correct for a non-expert; risky
   options default off and explain consequences inline.
4. **Presets**: named bundles (e.g., "Battery saver", "Max performance")
   that set multiple values atomically.
5. **Migrations**: stored settings carry a version; upgrades transform old
   files forward, never crash on unknown keys (unknown = keep + warn).
6. **Secrets are not settings**: API keys live in credential storage, never
   in the general settings file.

## Anti-patterns

- A dump of 200 toggles with no hierarchy.
- Settings that require reading source code to understand.
- Hidden "magic env vars" duplicating settings (env overrides allowed for
  CI/dev, but documented and discoverable).

## Testing expectations

- Schema validation test (every setting has valid type/default/category).
- Migration tests (v(n) file loads into v(n+1)).
- Default-reset round trip.

## Definition of Done

- New setting appears in schema, renders in both modes appropriately,
  persists, migrates, and is searchable.