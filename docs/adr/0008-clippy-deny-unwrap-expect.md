# ADR 0008 — Clippy deny on `unwrap`/`expect`

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: V12, V13, V1

## Context

Production paths must not panic on error (V12). `unwrap`/`expect` convert a
recoverable `Result`/`Option` into a panic, which for a long-running daemon
means a crashed check loop. Errors should instead be handled or propagated via
the typed enums of ADR 0004 (V1, V13).

## Decision

- Set at crate root:
  ```rust
  #![deny(clippy::unwrap_used, clippy::expect_used)]
  ```
- CI runs `cargo clippy --all-targets -- -D warnings`; either lint firing fails
  the build (V12).
- Test code may opt out locally with a scoped
  `#[allow(clippy::unwrap_used, clippy::expect_used)]` on the test module —
  panicking *is* the failure signal in a test, so this is acceptable and
  explicit, not a blanket exemption.
- Genuine "cannot fail" spots (e.g. a regex compiled from a string literal)
  either use a fallible constructor with proper error handling, or, if truly
  infallible, are documented with an `expect` only under an explicit,
  reviewed, scoped `#[allow]` with a comment stating the invariant. The default
  is: no `unwrap`/`expect`.

## Consequences

- The daemon degrades instead of crashing: a failed check is recorded as a
  failure and the loop continues.
- Forces the per-function error enums (ADR 0004) to actually be used — you
  cannot paper over a `Result` with `.unwrap()`.
- Slightly more verbose error handling at call sites; this is the intended cost
  of a daemon that stays up.

## Alternatives considered

- **Rely on `panic = "abort"` + supervisor restart** — a restart loses the
  no-missed-check guarantee (V3) and idle state; handling errors in-process is
  strictly better for a synthetics daemon.
- **Lint at `warn`** — warnings get ignored; `deny` in CI is what actually
  enforces V12.
