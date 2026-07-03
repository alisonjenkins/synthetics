# ADR 0004 — Per-function `thiserror` error enums

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: V1, V13

## Context

Errors must be precise: every fallible function returns its own error enum with
one variant per distinct failure mode of *that* function (V1) — no shared
god-error that lists every failure in the program. Errors must be handled where
they can be (recover/retry/degrade at source) and only propagated when the
caller must decide (V13).

## Decision

- Each fallible function (or tight group of functions forming one operation)
  gets a dedicated `#[derive(thiserror::Error)]` enum named for the operation,
  e.g. `HttpCheckError`, `ConfigLoadError`, `DogStatsdSendError`.
- Each enum variant names one concrete failure, with `#[from]` used only where a
  lower-level error maps one-to-one to a single variant. `#[source]` preserves
  the causal chain for structured logging.
- Higher layers that call several operations compose these into a slightly
  larger enum (still operation-scoped, not global) via explicit `#[from]`, so
  the call graph of errors mirrors the call graph of code.
- Recoverable conditions are handled at the source and do **not** become error
  variants of the caller: e.g. a transient DNS failure retried with backoff
  only surfaces an error variant if retries are exhausted (V13).

## Consequences

- A function's signature documents exactly how it can fail; matching on the
  enum is exhaustive and the compiler flags unhandled new variants.
- Slightly more enum types than a single global error, but each is small and
  local; this is the intended trade (precision over convenience).
- No `Box<dyn Error>` in library APIs — callers get typed, matchable errors.
  `anyhow`-style dynamic errors, if ever used, are confined to the top-level
  binary where the process simply logs and continues.
- Pairs with ADR 0008: because errors are typed and handled, `unwrap`/`expect`
  are unnecessary on production paths and are denied by clippy.

## Alternatives considered

- **One global `Error` enum** — convenient, but every caller sees every possible
  failure regardless of relevance; violates V1.
- **`anyhow` everywhere** — fast to write, but erases types so callers cannot
  match to recover/degrade; violates V1 and hampers V13.
