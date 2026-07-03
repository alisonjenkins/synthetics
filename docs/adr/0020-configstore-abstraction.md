# ADR 0020 — Storage behind a `ConfigStore` trait; Postgres impl first

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: I.store, V30, V34
- Relates to: ADR 0005 (same trait-abstraction pattern), ADR 0021 (static dispatch)

## Context

Postgres is the intended production store for the control plane, but the CP
should not be hard-wired to it. There is a real desire for flexibility —
SQLite for local development, and potentially something else down the line —
without rewriting the control plane's core logic every time the backend
changes.

## Decision

A `ConfigStore` trait is defined, using static dispatch per ADR 0021. It
covers hierarchy CRUD, resolving effective (fully inherited) checks, worker
heartbeat upsert and listing live workers, and a config version paired with
a generic change-signal exposed via `watch()`. The CP's core logic depends
only on this trait — no SQL or backend-specific code exists outside the
trait's implementations (V34).

Postgres is the first implementation. Its change-signal uses `LISTEN`/`NOTIFY`
to hit the sub-five-second propagation target from ADR 0018. A backend
without a native push mechanism — SQLite, for instance — implements the same
change-signal by polling instead: the contract is identical, only the
latency degrades, and no API changes are needed to swap backends. An
in-memory implementation mirrors the `MockSink` pattern from ADR 0005 for
tests, requiring zero database infrastructure. As established in ADR 0014,
the store holds config and membership only — never results (V30).

## Consequences

- Storage backends are swappable behind one trait, with Postgres as the
  production choice and SQLite or in-memory available for dev and tests.
- Tests run with zero database infrastructure using the in-memory
  implementation.
- The sub-five-second propagation target is a property of the Postgres
  implementation specifically, not a guarantee of the trait contract itself.
- There is one trait whose contract must be honored consistently across all
  implementations, and the `LISTEN`/`NOTIFY` specifics stay confined to the
  Postgres module rather than leaking into the CP core.

## Alternatives considered

- **Hard-wire Postgres, using `sqlx` directly in the CP** — rejected; this
  removes any flexibility to swap backends and forces every test to stand
  up a real database.
- **An ORM abstraction** — rejected as heavier than necessary compared to a
  purpose-built trait scoped exactly to the CP's needs.
