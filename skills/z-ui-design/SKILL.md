---
name: z-ui-design
description: Z Desktop visual language and UX principles — minimal, calm, native-feeling desktop-first design; panel system, progressive disclosure, customization. Use when designing or reviewing any user-facing surface.
---

# Z UI Design

## When this skill applies
Designing or implementing any visible surface: chat, panels, terminal,
diffs, settings, dialogs.

## The language

- **Minimal**: every pixel earns its place. If an element can be removed
  without losing function, remove it.
- **Calm**: no gratuitous motion, color, or badges. Status is communicated
  quietly; attention is spent only where the user needs it.
- **Native-feeling**: follows platform conventions (window controls,
  menus, focus rings). NOT a web page pretending to be an app.
- **Desktop-first**: keyboard-driven, multi-panel, resizable, dockable.
  Mouse-only flows are incomplete by definition.
- **Fast**: perceived latency targets — input echo < 16 ms, panel switch
  < 50 ms, streaming text renders without jank.

## References (inspiration, not imitation)

Zed (editor craft), Claude/Codex (agent conversation clarity), Apple HIG
(restraint), Linear (density discipline). Z Desktop has its own identity:
calm engineering instrument, not a SaaS dashboard.

## Structure

- Panels are first-class: visible/hidden, resizable, dockable positions.
- Progressive disclosure: simple by default; depth one click away.
- Empty states teach; error states explain recovery; loading states show
  real progress (streaming deltas), not spinners-of-lies.

## Customization direction

Theme tokens (color/space/radius/type) from z-tokens drive everything;
user-facing theming overrides tokens, never hardcodes colors in views.
Layout profiles persist per workspace.

## Anti-patterns (forbidden)

- Giant WebView chrome.
- Gradient-everywhere AI-startup aesthetic.
- Modal dialogs for recoverable actions.
- Fake progress indicators.

## Definition of Done

- New surface reviewed against this list; screenshots captured (--shot).
- Works at 100%–200% zoom/DPI; keyboard-navigable end to end.