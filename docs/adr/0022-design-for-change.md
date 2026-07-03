# ADR 0022 — Design for change — seams on known axes, anti-gold-plating

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: V36, V7

## Context

The system needs to be easy to change over time, but "easy to change" is
often misread as "abstract everything up front." In practice that produces
premature indirection — seams built for axes of variation that never
materialize — which makes change harder, not easier, because every real
change now has to thread through layers that don't correspond to anything
real.

## Decision

Seams are placed only on the axes that are already known to vary: check
protocols (HTTP, TCP, WebSocket, and others to come), assertion types,
metric sinks, config sources, and storage backends. Each of these sits
behind a trait and/or an enum, so that adding a new variant is additive,
localized, and compiler-guided — an exhaustive `match` combined with
required trait implementations turns "every place that must handle the new
variant" into a compile error, not a runtime surprise waiting to be
discovered later (V36).

The rule is stated explicitly as an anti-gold-plating constraint: no seam is
added without a named axis of variation, and speculative axes are not
abstracted just in case. The traits already agreed on — `MetricSink`,
`ConfigSource`, `ConfigStore`, and a coming `Check` trait — each map to a
real, concrete axis of variation that the system is known to need.

As a concrete example: adding a DNS check means implementing `Check` for a
new `DnsCheck` type and adding a `Target::Dns` variant. The compiler then
flags every place that must handle it — the config parser, the scheduler's
dispatch logic, and the report's detail formatting — as exact, compile-time
errors, rather than leaving those sites to be found by testing or by
production incidents.

Small, single-purpose modules and the TDD suite (V7) are what make these
changes safe in practice — the seams give the compiler leverage, but it is
the tests and module boundaries that make exercising a new variant quick to
verify.

## Consequences

- Changes along a known axis of variation are local, and the compiler
  guides the contributor to every site that needs updating.
- The codebase avoids the bloat and indirection that comes from abstracting
  axes that turn out never to vary.
- Contributors have a clear, explicit rule for when a new abstraction is
  warranted versus when it isn't.
- Judgement is still required to correctly identify when something is a new
  axis worth a seam, versus a one-off that doesn't need one — this rule
  doesn't eliminate that call, it just names it.

## Alternatives considered

- **Abstract every boundary up front** — rejected; produces premature
  indirection that makes future changes harder rather than easier.
- **No abstractions, everything concrete** — rejected; changes along axes
  that genuinely do vary (protocols, sinks, sources, backends) turn into
  shotgun edits scattered across the codebase.
