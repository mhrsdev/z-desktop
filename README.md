# Z Desktop

A local-first, desktop-native AI engineering workspace: a single native
Rust application where a supervised team of agents codes, debugs, tests,
researches, and manages real projects — with evidence-based completion,
durable state, and zero telemetry.

**Status**: early development (personal-first phase). See
[docs/Z-DESKTOP-MASTER-SPEC.md](docs/Z-DESKTOP-MASTER-SPEC.md) for the
canonical product/architecture specification and
[docs/DEVELOPMENT-STATE.md](docs/DEVELOPMENT-STATE.md) for current state.

## What it is

- **Agent core** (`z desktop/crates/z-core`): turn loop, tool runtime with
  risk classification and approval gates, process-tree sandboxing, secret
  redaction, token budgeting, repository indexing, provider streaming.
- **Protocol** (`z desktop/crates/z-protocol`): the Command/Event contract
  between core and UI. Additive-only evolution.
- **UI stack**: custom GPU-rendered Rust (`z-gpui`) over winit/wgpu with
  accesskit accessibility, a pure shell model (`z-shell`), and design
  tokens (`z-tokens`).
- **App composition** (`zero-app`): wires everything; headless `--check`
  validation and `--shot` screenshot capture.

## Build & test

Requires Rust (stable). From the workspace root:

```bash
cargo test --manifest-path "z desktop/Cargo.toml" --workspace
cargo run -p zero-app -- --check     # headless validation
```

## Repository layout

| Path | Contents |
|---|---|
| `z desktop/` | Cargo workspace: protocol, core, shell, gpui, tokens, app |
| `docs/` | Master spec, task ledger, research, ADRs |
| `skills/` | Operational engineering skills for agent sessions |
| `tools/` | Developer utilities (task generator, security scan) |

## Principles (short version)

Personal-first · local-first · model-agnostic · performance budgets as
design inputs · evidence before "done" claims · minimal token waste ·
security boundaries are load-bearing · recovery is a feature.

Full list: Master Spec §4.

## License

No license chosen yet — see Master Spec §67. Do not redistribute until
one is decided.