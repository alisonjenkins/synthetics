# ADR 0010 — TDD workflow + criterion benchmarks

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: V7, V16, V6, V15

## Context

Development must be test-driven (V7): a failing test is written and committed
before the implementation. Hot paths must be benchmarked (V16), both to prove a
SIMD path beats scalar (V6) and to catch performance regressions in the
low-footprint hot loop (V15).

## Decision

- **TDD cycle** per feature: write a failing test → watch it fail for the right
  reason → write minimal code to pass → refactor green. Enforced by the
  `/build` command flow and this ADR. Each `§T` task lands with its tests.
- **Test runner: `cargo nextest`** for fast parallel runs locally and in CI;
  falls back to `cargo test` where nextest is unavailable.
- **Benchmarks: `criterion`** as a dev-dependency, benches under `benches/`.
  Benched targets: the SIMD scan vs its scalar fallback (V6/V16), a scheduler
  tick, and a check execution against a local stub server.
- **CI gates:** test suite (nextest), `clippy -D warnings` (ADR 0008), and
  `cargo bench --no-run` to keep benches compiling. A regression threshold on
  the SIMD bench guards V16 over time.
- **Correctness-vs-SIMD tie-in:** the SIMD equality property test (ADR 0003) is
  a normal test in the TDD suite; the benchmark only decides whether to *keep*
  the SIMD path, never whether it is *correct*.

## Consequences

- Every production function has a test that was seen to fail first, so tests
  provably exercise behavior (V7).
- Performance is a tracked, regression-guarded property, not a one-off
  measurement (V16).
- Slightly slower feature throughput than code-first, traded for regression
  safety on a daemon meant to run unattended.

## Alternatives considered

- **Tests-after** — faster to type, but tests written after code pass
  immediately and prove nothing; rejected by V7.
- **`#[bench]` (libtest)** — nightly-only and less capable than criterion
  (no statistics, no regression tracking); rejected to stay on stable.
