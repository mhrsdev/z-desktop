---
name: z-extensions
description: Z Desktop plugin/extension platform — extension contracts, UI extension points, tool/provider/parser/renderer plugins, commands, versioning, isolation. Use when designing extensibility or deciding core-vs-plugin placement.
---

# Z Extensions / Plugin Platform

## When this skill applies
Designing extension points, evaluating whether a feature should be a plugin,
building the plugin SDK, or reviewing third-party integration surfaces.

## Decision rule: plugin-first when possible

If a capability does NOT need core privileges (raw filesystem outside scope,
process spawn, network to arbitrary hosts), it should be an extension, not
core code. Built-ins dogfood the same API.

## Extension kinds (planned surface)

| Kind | Contract | Examples |
|---|---|---|
| Tool | name, schema, risk class, execute fn | custom linters, deploy scripts |
| Provider | config schema, stream impl | new model vendors |
| Parser | language grammar + symbol extractor | niche languages |
| Panel | view contract + state | dashboards, inspectors |
| Renderer | artifact type → scene | diagram formats |
| Command | id, args, handler | workflows |

## Isolation & permissions

- Deny-by-default permission model: extensions declare needed capabilities;
  user approves per-extension, revocable, visible in UI.
- Crash isolation: a panicking extension must not take down the app
  (process boundary for untrusted; panic-hook containment for trusted v1).
- Versioning: extensions declare compatible protocol range; host refuses
  incompatible loads with a clear message.

## SDK direction

Manifest (TOML/JSON) + entry points + typed ABI. Start in-process with
strict trait contracts; out-of-process comes when third-party trust demands.

## Testing expectations

- Contract tests: every extension kind has a conformance suite a plugin can
  run against itself.
- Malicious-path tests: extension requesting undeclared capability fails.

## Definition of Done

- An extension can be added WITHOUT touching core code paths.
- Permission grant/revoke flows have UI + tests.