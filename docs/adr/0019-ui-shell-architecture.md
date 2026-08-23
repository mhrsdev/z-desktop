# ADR-0019: UI shell architecture (state flow, panel seam)

Ledger: decides the concrete shape behind ui-001..020 foundations — how
runtime events become pixels, who owns which piece of UI state, and the seam
every future panel host walks through. Unblocks directly: ui-020..022
(component library renders through the same scene pipeline), ui-030 (sidebar
scaffold is the first new panel to enter through the seam), every panel host
ui-040..290 (they are one `PanelSpec` + one dispatch arm each after this),
core-021 (multi-thread wiring adds a thread-list surface, not a rewrite),
gpui-005 (per-window shells reuse the whole model unchanged), and shell-001/
002 (persistence and focus hardening edit state this ADR partitions, not
code paths it invents).

## Status

Accepted (2026-08-23). Justification: §9 fixes the frame lifecycle ("Drain
EventQueue → update shell model → build/diff scene → submit"; "no event
mutates the scene mid-draw; the scene is a value") and §11 fixes the dock
model ("Layout = tree of regions … leaf panels", presets, persistence).
The z-shell crate already implements the §11.1 model — `PanelRegistry`,
`LayoutState`, `ViewState`, `Workspace` (lib.rs:8–12) — with 48 passing crate
tests. What has never been written down is the *composition contract*: which
state lives where, when mutation is legal, and what a new panel must touch.
This ADR records that contract against today's code so ui-track work stops
improvising it. It adds no dependency and no crate boundary change (§52
stack table, master-spec :835–845, is untouched).

## Context

Three crates cooperate today, and the seams between them are cleaner than
the ledger gives them credit for:

**z-gpui knows nothing about the product.** It defines `SceneSource`
(window.rs:23–25): the host builds a `Scene` for a viewport; the window loop
requests redraws when input handlers return `true` (window.rs:274–276) or
when a cross-thread wake arrives (window.rs:377–383). On every
`RedrawRequested` it calls `source.build(...)`, diffs the result against the
previous scene — `damage_against` (window.rs:294–296; scene.rs:300–303,
which unions changed quad/text rects into one damage region and returns
`None` for identical frames) — and skips the GPU submit entirely when
nothing changed (window.rs:298). Work scheduling is priority-classed
(lib.rs:44–68): nothing may delay P0 input.

**z-shell is the headless workspace model.** `Workspace = {registry, layout,
view}` (lib.rs:31–35): `PanelRegistry` is the authority on what each of the
seven panels can do (panel.rs:130; `PanelId::ALL` :32–40, capabilities
:92–107, size constraints clamped in the data layer :69–88), `LayoutState`
records the user's arrangement and swaps wholesale per preset
(layout.rs:49, 159), and `ViewState` records what is *inside* panels —
active tab strip, nav selection, per-`PanelId` scroll offsets, disclosure
state (view.rs:356–371). The two are separate fields "and no code path
copies between them" (lib.rs:48–50); preset switching provably preserves
scroll/tab/disclosure (lib.rs:69–81 test). `solve()` turns viewport +
registry + layout into a `ShellFrame` of rectangles (region.rs:80, 119).
Everything is `serde` — shell-001's persistence format already exists.

**z-app composes.** `App` (main.rs:34–56) owns the one `WorkspaceView`,
session mirrors (`streaming`, `step_tools`, `pending_call`, `thread_id`),
the command channel, and an `EventQueue = Arc<Mutex<Vec<Event>>>`
(main.rs:30–32). A pump thread forwards runtime events into the queue and
wakes the window, one wake per event (main.rs:319–333). Events are applied
by exactly one function, `apply_event` (main.rs:134–232), which mutates
*only* view fields and returns whether anything visible changed;
`drain_events` folds a taken batch and scrolls chat to end once
(main.rs:119–132). Draining happens both on wake (latency path,
main.rs:335–337) and at the top of `SceneSource::build` (correctness path,
main.rs:309–313) — idempotent by `mem::take`, so a batch that raced the
wake still lands before rendering.

`WorkspaceView` (view.rs:49–78) is one flat struct: the embedded z-shell
`Workspace` (:50), theme, the conversation projection, composer input,
status line, steering depth, pending approval, plus two caches kept across
frames because rebuilding them would be wrong, not just slow — measured chat
row heights in a `VirtualList<VariableHeights>` (:58–61) and the focused
`NodeId` (:62–64), restored into each freshly built semantic tree
(view.rs:253–257). `build()` (view.rs:224–261) is pure-over-state scene
construction: solve the frame, draw seven regions via seven private methods
(:236–249), restore focus, done. Input translation goes through one
exhaustive `ViewCommand` enum (view.rs:122–135) so keyboard, mouse, and
accesskit activation cannot drift into different command paths
(main.rs:385–388).

Constraints inherited: blocking threads, no async in core (ADR-0001); the
UI must be restartable without killing running work (z-gpui lib.rs:3–6);
personal-first scale — one user, one window, one thread today (core-021
brings the second); §9.5 budgets — scene build < 2 ms, panel switch < 50 ms,
no per-frame heap churn (master-spec :876–880). Scale honesty: ui-090..290
name ~20 future panel hosts; §11 names drag-to-dock, floating, and layout
profiles that do not exist yet.

## Considered options

**(a) Split UI state per panel behind a shell-level state registry**
(`HashMap<PanelId, Box<dyn PanelState>>` or a trait `Panel { fn state_mut()
… }`). Uniform, plugin-shaped, and wrong for today: every panel's state
would live behind a trait object, so the exhaustive, compiler-checked event
handling in `apply_event` degrades into downcasts, the serde story for
shell-001 fragments per-panel, and there is exactly one non-shell content
state (the conversation) to justify any of it. Rejected at M3 scale;
revisit only if a third independent content source appears.

**(b) Codify the current channel + flag flow; rely on the existing scene
diff for damage.** Producers push events and wake; one drain point applies
them; the boolean return means "request a redraw", not "repaint the screen";
what actually repaints is decided by comparing scenes, per frame, in
z-gpui. This is already a dirty-*frame* scheme with dirty-*region*
enforcement one layer down, and the region granularity (single quads) is
finer than any per-panel flag would be. Chosen.

**(c) Introduce an app-level dirty-region scheme** (panels declare invalid
rects; build() reconstructs only damaged regions). Rejected twice over.
First, it duplicates `damage_against`: the renderer already skips idle
frames and submits only damaged rects (window.rs:294–315), and scene.rs
notes a per-node pass can narrow damage further *inside* z-gpui if ever
needed (:298–299) — that is where refinement belongs, not in z-app. Second,
partial scene construction breaks the invariant that the scene is a value
rebuilt from state (§9.2): stale sub-regions become a class of bug whose
test suite is the debug story. The real cost driver at scale — measuring
10k chat rows per frame — is already bounded by the `VirtualList`, which
measures O(visible) rows.

**(d) Immediate dock-tree layout engine** (recursive splits, floating
windows, drag-to-dock per §11.2). Spec-correct, premature: `Preset` +
`solve()` already produce every layout §11.1 names as a preset
(`Preset::BUILT_IN`, layout.rs:74), presets switch in one assignment with
view state preserved (lib.rs:48–50), and nothing in the ui-ledger depends
on user-*arranged* trees until drag-to-dock ships — ui-030..290 need
*dockable-in-principle* panels, not draggable ones today. Rejected; see D4
for the seam that keeps it cheap later.

**(e) Trait-object plugin panels now** (`Box<dyn PanelRenderer>` registered
at startup). The §8.9 extension kinds include panels, but extensions arrive
with the permissioned host, versioned contracts, and deny-by-default grants
— none of which exist. An enum dispatch arm is exhaustive, zero-allocation,
and greppable; a trait registry is none of those until the plugin sandbox
exists. Rejected until §8.9 lands.

**(f) Push rendering state (focus, scroll caches) into z-shell so
`ViewState` owns everything.** Rejected: z-shell is deliberately GPU-free
and headless (lib.rs:3–5); `NodeId` and row-height caches are renderer-
domain values. The existing split — shell model in z-shell, presentation
caches in z-app — is the correct dependency direction and costs one
conversion struct (`Frame`, view.rs:18–47) documented at the boundary.

## Decision

### D1 — State flow: one view struct, three owners, mutation at two gates

UI state stays a **single `WorkspaceView`** through M3. What this ADR fixes
is not the struct's shape but *ownership and write points*, which is what
actually prevents the god-object failure mode:

| Owner | Contents | Written by | Persisted by |
|---|---|---|---|
| z-shell `Workspace` | panel registry, layout/preset, tabs, nav, per-panel scroll, disclosures | preset switch, tab/nav/scroll commands | shell-001 (serde already done) |
| `App` session mirrors | `thread_id`, `streaming`, `step_tools`, `pending_call` | `apply_event`, command senders | never (runtime truth lives in z-core/journal) |
| `WorkspaceView` presentation | conversation projection, composer text, status line, steering depth, approval card, focus id, chat row cache | `apply_event` and input handlers only | shell-001 scope-limited (input draft optional) |

Two gates, no exceptions: runtime state enters the view **only** inside
`apply_event` during `drain_events`; user intent enters **only** through
input/access handlers returning their bool. `build()` reads everything,
writes nothing except restoring focus into the new tree. This is §9.2's
lifecycle stated as type-level fact: the scene is a value derived from
state; no event mutates a scene mid-draw because events never see a scene.

Per-panel *content* state (a usage dashboard's series, a memory inspector's
tree) is added as plain fields on `WorkspaceView` when its first consumer
ships (ui-050/ui-060), one field per panel — not a registry, not a trait.
If a fourth or fifth content panel makes the flat struct unwieldy, the
migration is mechanical (group fields under per-panel structs, keep the
gates), and that refactor — not this ADR — is where splitting happens.
Registry-of-states (option a) stays rejected until an out-of-tree panel
consumer exists at all.

### D2 — Event → render: codify queue + wake + drain; damage lives in the scene

The current pattern is the pattern:

1. **Producers never touch view state.** The runtime sends `Event` into the
   `Vec`-backed queue and pokes one `HostEvent::Wake` per event
   (main.rs:324–332). Wakes are cheap; winit coalesces them.
2. **One drain point applies events.** `drain_events` takes the whole batch
   under the mutex, applies each via `apply_event`, ORs the changed flags,
   and performs once-per-batch side effects (scroll-to-end) rather than
   per-event ones (main.rs:119–132). Batch-take makes overflow impossible
   to interleave and makes the double call site (wake + build-top)
   harmless — the second drain sees an empty batch.
3. **The bool means "request redraw," not "paint."** Handlers return
   whether scene *inputs* changed; z-gpui converts true into exactly one
   `request_redraw` (window.rs:274–276, 377–383). Redraws are free when
   idle: the frame still runs `build` + diff, and an unchanged scene costs
   no GPU work (window.rs:298).
4. **Dirty-region is the renderer's job, permanently.** No app-side dirty
   tracking, no per-panel invalidation flags, ever, unless a profiled
   workload shows `build()` itself exceeding the 2 ms budget with
   virtualization already correct — at which point the fix is narrowing
   `damage_against` per-node inside z-gpui (its own stated upgrade path,
   scene.rs:298–299), not teaching z-app about rectangles.

Cost accounting for the honest objection — "you rebuild the whole scene
every redraw": scene build today is token-derived quads with no allocation
churn in steady state, chat measurement is O(visible) via the retained
`VirtualList` (view.rs:56–61), and gpui-002/gpui-004 exist precisely to
measure and gate this. Optimizing before that telemetry is option-c
thinking: speculative machinery against a budget we have not missed.

### D3 — Panel seam: registry-driven geometry, enum-driven rendering

The seam that lets panels land without rewriting `WorkspaceView` is three
small changes, none speculative, all inside existing shapes:

1. **Geometry by `PanelId`, not by field copy.** z-app's `Frame` struct
   hand-mirrors `ShellFrame`'s seven fields (view.rs:24–47). Add
   `ShellFrame::rect(self, id: PanelId) -> &Rect` in z-shell (region.rs) so
   z-app deletes `Frame` and reads `frame.rect(PanelId::Chat)` directly.
   An eighth panel then requires **zero** changes to geometry plumbing —
   `solve()` lays it out, every renderer reads it by id.
2. **Rendering through one dispatch.** `build()`'s fixed method calls
   (view.rs:236–249) become a loop over `PanelId::ALL` into
   `fn render_panel(&mut self, scene: &mut Scene, id: PanelId, rect: Rect)`
   — an exhaustive `match` delegating to today's seven methods verbatim.
   Byte-identical output on day one; every future panel host (ui-090..290)
   is one new `match` arm from then on.
3. **Registration + view state follow the existing patterns.** A new panel
   registers a `PanelSpec` (docks, constraints, capabilities — panel.rs:111+
   ), gets its preferences slotted into `ViewState` following the
   `scroll: BTreeMap<PanelId, f32>` precedent (view.rs:370), and — if it
   presents runtime data — gets a `WorkspaceView` content field per D1 plus
   its `apply_event` arms. Nothing else moves: no callbacks, no trait, no
   layout engine.

What this deliberately does **not** buy yet: drag-to-dock, floating
windows, user-authored layout profiles, per-panel event subscriptions.
Those are §11.2/§8.9 features with their own tasks; the D3 seam is chosen
so each lands as a local addition — dock trees replace `solve()`'s
internals behind the same `ShellFrame::rect` API; extension panels swap
the enum for a registry behind the same `render_panel` signature — instead
of as rewrites.

### D4 — Multi-window and multi-thread are consequences, not work

gpui-005 (shell per-window) falls out: a window is one `App`-shaped host —
own `WorkspaceView`, own session mirrors, own queue — sharing one runtime
through the existing command/event channels; z-gpui's `SceneSource` already
parameterizes the host. core-021 (thread list) falls out: it is a nav
surface or tab kind through D3's dispatch, backed by a `ThreadsChanged`-
style event projected per D1 — no architectural delta. If either eventually
needs shared cross-window state, that is a successor ADR amending D1's
owner table.

## Consequences

**Immediate**: D3 items 1–2 are a small, provably behavior-preserving
refactor (`--check` and `--shot` outputs byte-comparable before/after);
item 3 is documentation of practice. ui-030 (sidebar scaffold) becomes the
seam's first real customer and its acceptance test: a panel that reaches
rendering through `PanelSpec` + dispatch arm alone, with no edits to
`build()`'s structure.

**The event path stays boring on purpose**: one queue, one drain, one
boolean, one damage pass. Debugging streaming glitches means reading
`apply_event` top to bottom — every runtime→pixel transition lives in one
function that fits on two screens. tests-006 (pump backpressure) targets
the queue; shell-003 (streaming buffer caps) targets the conversation
projection; neither needs new plumbing.

**Persistence is already scoped**: shell-001 serializes the z-shell
`Workspace` (serde derives exist throughout z-shell) plus the whitelisted
presentation fields D1 names — corrupt-layout tolerance (shell-005) guards
exactly those bytes. Session mirrors are excluded by construction; a
restart replays truth from the journal, never from the view.

**Accepted debt**: the flat `WorkspaceView` grows linearly with content
panels until someone executes the D1 grouping refactor — capped, by the
ledger's current shape, at roughly ten fields before ui-050/060 force the
question. Preset-switch-by-arrow-keys (main.rs:295–305) remains a stopgap
until a settings/keybinding surface exists. `damage_against` unions into a
single rect, so one moving quad in a corner repaints its bounding region —
measured acceptable at current scene densities; the per-node refinement
note in scene.rs is z-gpui's debt to file, not z-app's.

**Testing obligations locked in**: parity test — `render_panel` dispatch
vs. today's direct calls produce identical scenes across all presets
(prove-by-absence, ADR-0011 discipline); preset-switch-preserves-view test
already in z-shell (lib.rs:69–81) extended to cover the new panel ids;
drain-idempotence test (two consecutive `drain_events` calls, second is a
no-op); idle-frame-no-GPU-work assertion exists in scene tests
(scene.rs:423) and gains a variant asserting a wake with zero events
requests no repaint; shell-006 property tests cover preset × viewport
transitions through the seam.

## Sources

- Repo inspection (2026-08-23): `z desktop/crates/z-app/src/main.rs` —
  EventQueue alias (:30–32), `App` struct (:34–56), `drain_events`
  (:119–132), `apply_event` (:134–232), `SceneSource::build` drain site
  (:308–313), event pump + wake proxy (:319–333), `on_wake` (:335–337),
  preset stepping (:295–305);
  `crates/z-app/src/view.rs` — `Frame` mirror (:18–47), `WorkspaceView`
  fields incl. retained `VirtualList` and focus id (:49–78),
  `ViewCommand` (:121–135), `build()` fixed panel calls (:224–261), focus
  restoration (:251–257);
  `crates/z-shell/src/lib.rs` — Workspace triple (:31–35), preset switch
  preserving view state (:44–56, test :64–95); `panel.rs` — `PanelId`
  (:19–53), `Capabilities` (:90–107), clamped `Constraints` (:67–88),
  `PanelSpec`/`PanelRegistry` (:109+, :130+); `layout.rs` — `LayoutState`
  (:49), `Preset::BUILT_IN` (:60–74), `from_preset` (:159); `view.rs` —
  separation doctrine (:1–13), `ViewState` with per-panel scroll
  (:356–371); `region.rs` — `ShellFrame` (:80), `solve` (:119);
  `crates/z-gpui/src/window.rs` — `SceneSource` (:22–69), redraw-on-bool
  (:273–276), `RedrawRequested` build + `damage_against` + idle skip
  (:279–298, :329), Wake→redraw (:377–383); `scene.rs` — `damage_against`
  union + per-node upgrade note (:294–330, test :423); `lib.rs` —
  product-free charter (:1–13), `Priority` (:39–68).
- Z-DESKTOP-MASTER-SPEC.md (retrieved 2026-08-23): §9.1 stack table
  (:835–845), §9.2 frame lifecycle + scene-is-a-value invariant
  (:847–855), §9.4 current/planned surfaces (:866–874), §9.5 performance
  contracts (:876–880), §11 Dock & Layout model/persistence (:915–936),
  §8.9 extension kinds incl. panels (:554–556).
- docs/Z-DESKTOP-TASKS.md (retrieved 2026-08-23): ui-010..029 foundations
  and component library (:2110–2123), panel hosts ui-040..290
  (:2125–2201), gpui-001..010 incl. multi-window (:2205–2233), shell-001..
  006 (:2237–2253), core-021 multi-thread UI wiring (:81), tests-004/006
  (:2315–2322).
- docs/adr/0009 (snapshot-read discipline mirrored by the view's
  projection-only rule), 0011 (prove-by-absence migration pattern reused
  for the D3 parity test), 0013 (ADR tone and Decision/Consequences
  structure).
