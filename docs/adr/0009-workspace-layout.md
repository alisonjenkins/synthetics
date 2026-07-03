# ADR 0009 — Single-member cargo workspace

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: T1, V7, V16

## Context

The project needs room to grow (benches, possibly a separate metrics-sink crate
or an integration-test crate later) without a disruptive restructure, while
staying minimal today (V15).

## Decision

Start as a cargo **workspace** with a single member crate `synthetics` that is
both a library (`lib.rs`, all logic, unit-testable) and a binary (`main.rs`, the
daemon that wires the library together).

```
Cargo.toml            # [workspace] + shared lints/deps via workspace inheritance
crates/synthetics/
  Cargo.toml
  src/lib.rs          # modules: config, metric, sink, check, assert,
                      #          scheduler, reload, simd, telemetry, daemon
  src/main.rs         # thin: parse args, init tracing, run daemon
  benches/            # criterion benches (ADR 0010)
  tests/              # integration tests
```

- Logic lives in the library so it is unit-testable without spawning the binary
  (V7); the binary stays thin.
- Workspace-level `[workspace.lints]` centralizes the clippy deny config
  (ADR 0008) so every future member inherits it.

## Consequences

- Adding a second crate later (e.g. `synthetics-sink-otlp`) is dropping a
  directory and a workspace member — no migration.
- One `Cargo.lock`, shared dependency versions across future members.
- Marginally more structure than a bare `src/main.rs` today, justified by the
  library/binary split that unit testing needs anyway.

## Alternatives considered

- **Bare single-crate package** — less nesting, but retrofitting a workspace
  later churns paths and CI; cheap to do right now.
- **Many crates up front** (per-module crates) — premature; compile-time and
  cognitive overhead with no present benefit.
