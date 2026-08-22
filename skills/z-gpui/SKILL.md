---
name: z-gpui
description: Z-GPUI rendering architecture — window/renderer/scene pipeline, GPU efficiency, layout, event handling, resource scheduling, animation, virtualization, accessibility. Use when working in crates/z-gpui or z-app view code.
---

# Z-GPUI / Rendering

## When this skill applies
Work in `z desktop/crates/z-gpui` (window, renderer, scene, a11y, timing) or
view composition in `z-app`.

## Pipeline (verify against source)

UI thread → scene build (from shell model) → renderer submits GPU work →
present. Events arrive via winit; a11y via accesskit. The frame loop drains
the EventQueue at frame start — events never mutate the scene mid-draw.

## Rules

1. **Scene is immutable per frame**: rebuild or diff, never mutate during
   draw. Frame-time budget: keep scene build under ~2 ms for typical UI.
2. **GPU efficiency**: batch by material/texture; avoid per-frame allocation
   of vertex buffers where possible; measure with the timing module before
   optimizing.
3. **Text**: cosmic-text for shaping/layout; cache shaped runs; re-shape only
   on width/font change.
4. **Virtualization**: long lists (chats with thousands of messages, huge
   diffs) must render only visible items. No unbounded item trees.
5. **Animation**: token-driven durations from z-tokens; respect reduced-
   motion preference; animations never block input.
6. **Accessibility**: every interactive element gets an accesskit node with
   label + role. A11y is not optional polish.
7. **Resource scheduling**: texture/font atlas uploads amortized; no
   synchronous GPU stalls on the UI thread.

## Cross-platform notes

- wgpu backends differ (D3D12/Vulkan/Metal); do not assume swapchain
  behavior. Test --check and screenshot paths per platform when touching
  presentation.

## Testing expectations

- `cargo run -p zero-app -- --check` validates headless startup.
- `--shot <dir>` captures screenshots for visual regression.
- Scene-building logic should be unit-testable without a window (pure
  functions from shell state → scene).

## Definition of Done

- No dropped-frame regression on the standard demo workload.
- New widgets ship with a11y nodes and keyboard navigation.
- Timing measurements recorded for any hot-path change.