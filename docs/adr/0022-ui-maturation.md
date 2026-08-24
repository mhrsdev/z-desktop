# ADR-0022: UI maturation — from prototype shell to product design language

Ledger: records the honest state of the UI track after Wave 4 (kb nav,
thread selection) and decides how the shell matures visually without
betraying its architecture. Directly unblocks ui-020..022 (component
library), ui-100+ (panel hosts, which must not each invent their own row,
badge and toolbar drawing), theme-001..012 (token catalog, second theme,
reduced-motion), and gpui-004/006 (frame gate, DPI audit). Amends nothing
in ADR-0019 — the panel seam, event flow, and scene-as-value invariants
are load-bearing and stay exactly as decided there.

## Status

Proposed (2026-08-24). Justification: the workspace is green — 545 tests,
ledger at 124 IMPLEMENTED + 4 PARTIAL (docs/DEVELOPMENT-STATE.md,
2026-08-24) — and the next wave is panel-host-heavy (ui-050 usage
dashboard, ui-090 terminal, ui-100 editor). Every one of those surfaces
will copy-paste presentation decisions from `z-app/src/view.rs` unless the
token and component layers are consolidated first. This ADR writes down
what is real today, what is prototype-grade, and the phase order that
fixes it cheaply.

## Context — honest audit of the current UI

**What genuinely exists and is product-grade.** The structural half of the
UI is further along than most prototypes ever get:

- **Panel seam**: seven registered panels with declarative constraints,
  capabilities, collapse priorities, and an `essential` flag protecting
  security-bearing chrome (`z desktop/crates/z-shell/src/panel.rs:111-126`,
  `PanelId` :21-29). Widths clamp in the data layer, not the drag handler
  (`Constraints::clamp`, panel.rs:85-88), so a hand-edited config cannot
  produce absurd geometry.
- **Responsive honesty**: narrow windows collapse rails by priority and
  *report* what folded away instead of clipping (`ShellFrame::collapsed`,
  region.rs:90-93; collapse loop region.rs:156-190). Chat provably stays
  the dominant surface and the largest region (region.rs tests :314-332).
- **Theme foundation**: role-named semantic colors only — widgets can
  literally not name a primitive (`Semantic`, z-tokens/theme.rs:35-64;
  primitives are a private module, theme.rs:6-31). The default theme ships
  with an executable WCAG AA contrast gate over 15 foreground/background
  pairs (theme.rs:121-141, test :182-191), a purple-accent ban test
  (:220-235), and translucent borders justified over any surface tone.
- **Type discipline**: nine named text styles with leading-ratio and
  scale-ordering tests, mono separation for code and live readouts, and a
  compile-time assertion that METRIC uses tabular figures
  (`z-tokens/typography.rs:95-132`) — the Performance Strip cannot shudder.
- **Accessibility spine**: accesskit nodes on interactive elements,
  namespaced stable ids, focus restored into each rebuilt tree, and the
  focus ring drawn on a topmost layer so a focused control is always
  visible (`z-app/src/view.rs:253-259, 296-306`).
- **Drag-to-dock geometry** exists as pure, tested math awaiting a drag
  controller (`z-shell/src/dock_indicators.rs:38+`).

**What is still prototype-grade.** Named, with receipts:

1. **Token discipline is leaking.** `z-app/src/view.rs` opens by claiming
   "no literal hex values and no magic pixel constants" (:4-5). That was
   true once; it no longer is. Counted today: `INPUT_BAR_HEIGHT = 92.0`
   (:159), `SURFACE_TOOLBAR_ROW_HEIGHT = 44.0` (:163), `ROW_HEIGHT = 20.0`
   (:1529), `STROKE = 1.4` (:2751), a hand-rolled 16×16 badge
   (:2355-2358), focus-ring inset `-2.0` and border width `2.0`
   (:303-305), sidebar `row_height = 38.0` (:1290), inter-row gap `+2.0`
   (:1306), a `40.0` thread-row budget (:1313), an icon-rail threshold of
   `120.0` (:1289), and two-line row rects of `22.0` / `18.0` with a
   `+20.0` baseline offset (:2914-2928). None of these are named tokens;
   several are not even on the 4px rhythm the spacing scale enforces
   elsewhere (metrics.rs:142-147). Each new panel copies these numbers by
   eye. This is how a design system dies — not with a decision but with
   forty small exceptions.
2. **Fixed typography, no user scaling.** The nine styles are compile-time
   constants. `XL` (typography.rs:105) is defined and used nowhere in the
   app; there is no global text-size adjustment, no density-coupled type,
   nothing answering §13's promised "font family/size/weight per role"
   customization (master-spec :962).
3. **No motion system at all.** Zero duration or easing tokens exist.
   `z-gpui/src/timing.rs` is frame-*budget* instrumentation (`Stage`,
   :23-33), not animation. The only spec'd motion tokens —
   `motion.fast/base/slow` (master-spec §76, :3209) — are paper. Reduced-
   motion honoring (§14 :985; task theme-012 :1662) is currently
   unimplementable because there is no motion to reduce.
4. **One theme, hardcoded choice.** `Theme::zero_dark()` is constructed
   directly in `WorkspaceView::new()` (view.rs:196). `Theme` is plain
   `Copy` data — "themes are data … never code" (theme.rs:66-68) — so
   runtime switching is trivial plumbing that simply does not exist yet.
   Light and high-contrast remain PLANNED (theme-003 :1635, theme-004
   :1638).
5. **No component vocabulary.** Every surface hand-draws its own rows and
   chips: `nav_row`, `sidebar_two_line_row` (:2903-2928),
   `evidence_badge_row` (:1525+), an ad-hoc active-task counter badge
   (:2355). Three near-clones of "small labelled pill" already exist.
   ui-020/ui-021/ui-022 (button/input/list/tabs/dialog; tooltip/badge/
   empty/skeleton/toast; splitter) are still PLANNED (Z-DESKTOP-TASKS.md
   :2113-2121), while §64's component contract — tooltip delayed 400 ms,
   counts capped "99+", toast stack max 3, errors persist until dismissed
   (master-spec :2879-2896) — has no implementation to carry it.
6. **No density control.** `grep -ri density crates/` returns nothing,
   against §13's "density presets" row (:963).
7. **Missing token families block the next panels.** §76 names
   `syntax.*`, `terminal.ansi0..15`, and `chart.categorical1..8` families
   (:3203-3207; tasks theme-008/009/010). ui-090 (terminal), ui-100
   (editor), ui-130/140 (diagram/DB grid) cannot ship tokens-clean without
   them.
8. **Color-only state signaling risk.** Evidence badges encode pass/fail
   through the `ok` flag and color pair; the studied grok-build permission
   prompter explicitly refuses to rely on color alone for state
   (`references/external/grok-build/crates/codegen/xai-grok-workspace/
   src/permission/prompter.rs:214-215`). We have adopted that lesson
   nowhere systematically — no shape/icon redundancy rule, no
   don't-rely-on-color test. (Note: the grok-build clone contains no TUI/
   pager crate — its crates are build/codegen/common — so the reference
   lesson here is architectural, not stylistic.)

The pattern is consistent: **architecture mature, presentation skin
prototype.** That is the correct order to have built it in, and the wrong
order to keep building in.

## Design language — Z Desktop's own, stated plainly

Not a port of anyone's design system. The language already latent in the
tokens gets named so future decisions have something to violate visibly:

- **Calm surface, powerful core.** Elevation reads from five ordered
  charcoal tones, never heavy shadow (theme.rs:9-11, ordering test
  :207-217); the coral accent is reserved for identity, selection, and
  focus — scarce by policy (theme.rs:24-26). This is §35.8 "Calm Native
  UX" (master-spec :1714) and §119's quiet-by-default posture (:4337-4340)
  expressed as pixels.
- **Chat owns the room.** Chat is the dominant surface by law
  (panel.rs:293-298 test; region.rs chat-is-largest test :314-323), with a
  bounded readable measure (`CHAT_MEASURE` 760 px, metrics.rs:72-73).
- **Keyboard-honest.** Focus is always visible (top-layer ring), hit
  targets clear `MIN_HIT_TARGET` 28 px (metrics.rs:80, test :170-174), and
  hidden things keep their accessible names even in icon-only rails
  (view.rs:1300-1305).
- **Tokens or it doesn't ship.** §10.2's rule stands: "Hardcoding a
  color/size in a view is a defect" (master-spec :902-903). The audit
  above is a defect inventory, not a style preference.

**Token vocabulary to consolidate (Phase A):** keep `Spacing` (4px base,
S1..S10, metrics.rs:13-30), `Radius` SM/FULL (:37-43), `Shell` band
dimensions (:53-81), the nine `Typography` styles, and `Semantic` colors
as the single source of truth in z-tokens. Add the missing members the
audit surfaced: control/row heights (`row.md` ≈ the 38px sidebar row, the
20px index row, the 44px toolbar row, the 92px composer), focus-ring
width/offset, hairline (the four `hairline_*` helpers all assume 1.0,
view.rs:2941-2960), stroke weight (1.4), chip/badge size (16), and the
icon-rail threshold (120). Extend `contrast_pairs()`-style gates to any
new family. Then delete every literal listed in audit item 1 — theme-011's
hardcoded-color lint (:1659) generalized to magic pixels.

**Type:** keep the fixed nine-style scale for M3; mark `XL` used-or-deleted
at Phase A review. User-facing size scaling and per-role overrides wait
for §13 Developer Mode exposure (Phase D) — a multiplier over the scale,
not a second scale.

**Color roles:** `Semantic` already implements §10.1's semantic layer and
most of §76's catalog. Phase D adds the three missing families (syntax,
terminal ANSI, chart categorical) as sibling role structs behind the same
private-primitive pattern.

## Decision — phased maturation

### Phase A — Token consolidation (single source of truth)

Move every presentation constant in `z-app/src/view.rs` into
`z-tokens` (new `components` or extended `metrics` module). Mechanical,
testable, zero visual change: the parity discipline from ADR-0019 D3
(byte-comparable scenes) applies directly. Includes the theme-011 lint as
a CI grep gate so the leak cannot reopen, and the `XL` keep-or-kill call.
Exit criterion: `grep -nE '[0-9]+\.[0-9]' z-app/src/view.rs` returns only
token imports and geometry derived from named constants.

### Phase B — Component library (ui-020/021/022)

Extract the hand-drawn rows, badges, buttons and inputs into shared
scene-level components carrying their §64 contracts (tooltip 400 ms delay,
badge "99+" cap, empty states with one primary action, toast stack ≤3,
errors persist). The three existing pill clones become one badge component
with an a11y label and a non-color state channel (shape/glyph), adopting
the grok-build prompter lesson. Components render through the same scene
pipeline as everything else — no new rendering concept (ADR-0019 ledger,
lines 3-8). Exit criterion: ui-050 (usage dashboard) and ui-090 (terminal)
compose from the library without drawing their own primitives.

### Phase C — Motion + reduced-motion

Introduce `motion.fast/base/slow` duration tokens (§76 :3209) and a
minimal easing pair. Motion scope is deliberately tiny: streaming-text
reveal, panel collapse transitions, approval-card entrance — state-change
signaling only, each interruptible by the existing scene-diff (a mid-
animation frame is just another scene value; the queue-wake-drain flow is
untouched). Implement theme-012 reduced-motion honor (§14 :985) as a
global switch that collapses all durations to zero — which requires the
tokens from Phase A and is why this phase follows it. Note the ledger
correction: the delegated brief attributed reduced-motion to "layout-012";
that task is actually *empty-state focus when last panel closes*
(Z-DESKTOP-TASKS.md :1700). The correct vehicle is theme-012 (:1662).
Developer-mode duration/easing overrides (§13 :965) ride the same tokens.

### Phase D — Theming + developer-mode deep customization

Runtime theme selection (load `Theme` data at startup and on change;
`WorkspaceView.theme` is already a field). Ship z-light (theme-003) and
high-contrast AAA (theme-004) through the existing `contrast_pairs()` gate
— the gate was exposed as API precisely so future themes clear the same
bar (theme.rs:118-120). Theme file versioning + unknown-token warnings
(theme-005), import/export (theme-007), live-preview editor in Developer
Mode (theme-006, §10.3 :907-909, §12 :946-948). Density presets and
per-role typography overrides surface here per §13, opt-in, defaults
untouched ("Depth is opt-in … never required for basic operation",
master-spec :973-975).

Ordering is strict: A before B (components consume tokens), B before the
ui-100+ panel-host wave (hosts compose components), C and D independent of
each other but both downstream of A. Zed remains the queued study target
for pane/palette/text work when ui-100+ starts
(docs/research/REFERENCE-IMPLEMENTATION-MAP.md :54-57) — INSPIRE input to
Phase B/D execution, not a license to import its look.

## What we deliberately do NOT do

- **No blur, glass, translucency showcase.** Elevation is tone-ordered
  charcoals with translucent hairlines; adding acrylic/vibrancy would
  fight the AA contrast gate and cost GPU budget for zero information.
- **No Electron-style web rendering.** §4.4 is explicit: native windowing,
  "No WebView-shell aesthetics or constraints" (master-spec :230-232).
  The planned browser pane (§36.5) is a pooled WebView for *content*, not
  a rendering substrate for our UI.
- **No animation-heavy UX.** Quiet by default (§119.2); skeletons never
  fake duration (§64 :2892); no parallax, no spring physics, no delight
  passes. If a proposed animation cannot state which state-change it
  signals, it does not get built.
- **No design-token codegen pipeline or storybook tooling** — the token
  surface is a few hundred `const`s with tests; a build step would be
  ceremony (§4.14, maximum useful capability per necessary line).
- **No pixel-perfect multi-platform theme fleet at M3** — one dark theme
  done right, then light, then AAA. Cross-platform polish waits for
  gpui-006 DPI audit evidence.

## Revisit triggers

This ADR is reopened when any of these fire:

1. **A third near-clone** of any existing hand-drawn component appears in
   a panel host before Phase B lands — pull B forward, freeze the copying.
2. **Extension themes become real** (§8.9 panels/extensions loading theme
   files): theme file versioning moves from Phase D to blocking, because
   unknown-token tolerance becomes a security/robustness surface, not a
   convenience.
3. **A frame-budget miss (gpui-004 gate) attributable to motion** — motion
   tokens get a perf budget column or the offending transition dies.
4. **An accessibility audit failure** on focus visibility, contrast, or
   reduced-motion — the relevant phase's exit criterion reopens
   immediately regardless of schedule.
5. **ADR-0019's flat-`WorkspaceView` cap (~ten content fields) is
   reached**: the D1 grouping refactor lands alongside Phase B, not after,
   since components will need per-panel content structs anyway.
6. **A fourth theme consumer** (high-contrast, syntax family, terminal
   palette) needs primitives beyond the private-module pattern — promote
   z-tokens to an explicit public token API with semver, per theme-005.

## Consequences

Immediate cost is Phase A: roughly one focused session, mechanical, gated
by byte-comparable scene parity and the existing 545-test suite. The
compounding benefit is that every subsequent panel host inherits the
design language for free instead of forking it — the same argument that
justified ADR-0019's seam, applied one layer up. The debt named here
(token leaks, missing motion, single theme) is accepted until its phase
because none of it blocks correctness, accessibility foundations, or the
panel-host architecture; all of it would multiply if the ui-100+ wave
started first.

## Sources

- Repo inspection (2026-08-24): `z desktop/crates/z-tokens/src/theme.rs`
  (Semantic :35-64, zero_dark :78-114, contrast_pairs :121-141, tests
  :182-247); `z-tokens/src/metrics.rs` (Spacing :13-30, Radius :37-43,
  Shell :53-81); `z-tokens/src/typography.rs` (scale :95-113, tabular
  assert :131-132); `z-shell/src/panel.rs` (PanelSpec :111-126, clamp
  :85-88, chat-priority test :293-298); `z-shell/src/region.rs`
  (collapse loop :156-190, collapsed reporting :90-93);
  `z-shell/src/dock_indicators.rs` (:1-38+); `z-gpui/src/timing.rs`
  (budget stages :23-60); `z-app/src/view.rs` (token claim :1-7,
  literals :159/:163/:2751/:2355-2358/:303-305/:1289-1313/:1529/:2914-2928,
  dispatch :264-286, focus restore :253-259, focus ring :296-306,
  badge strip :1520-1533, theme construction :196).
- docs/Z-DESKTOP-TASKS.md: ui-020/021/022 (:2113-2121), ui-030/040
  IMPLEMENTED (:2122-2127), ui-100+ hosts (:2143+), theme-001..012
  (:1629-1663), layout-012 actual content (:1700), kb-001 PARTIAL
  (:1705).
- docs/Z-DESKTOP-MASTER-SPEC.md: §4.4 desktop-first (:230-232), §10
  Theme System incl. hardcode-defect rule (:884-913), §12 Developer Mode
  (:940-952), §13 Customization Depth (:954-975), §14 Accessibility
  (:976-989), §35.8 Calm Native UX (:1714-1718), §36 surface specs
  (:1722+), §64 component table (:2875-2896), §76 token catalog
  (:3196-3216), §119 Notification Center (:4331-4345).
- docs/research/REFERENCE-IMPLEMENTATION-MAP.md (zed clone trigger
  :54-57); references/external/grok-build
  `.../xai-grok-workspace/src/permission/prompter.rs:214-215`
  (color-independence of state).
- docs/adr/0019-ui-shell-architecture.md (seam, event flow, parity-test
  discipline reused here); docs/DEVELOPMENT-STATE.md (545 tests, ledger
  state, 2026-08-24).
