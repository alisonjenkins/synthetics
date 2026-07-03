# ADR 0007 — Structured logging via `tracing` (JSON)

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: I.log, V14

## Context

Logging must be structured and machine-parseable — no free-form `println!`
(V14) — and must carry per-check context (endpoint id, outcome) via spans so
logs correlate with the check that produced them.

## Decision

- Use the `tracing` facade for all diagnostics and `tracing-subscriber` with the
  JSON formatter as the default subscriber (I.log).
- Each check runs inside a `#[instrument]` span carrying the endpoint id; the
  span records the outcome and key timings as structured fields when it closes.
- Log **events** carry typed fields (`endpoint`, `status`, `latency_ms`,
  `error`), never interpolated prose, so downstream tooling can filter and
  aggregate.
- Log level is configurable via `RUST_LOG` / `EnvFilter`; default is a sane
  production level. Human-readable pretty output is available for local dev via
  a config switch, but JSON is the default for collectors.
- `println!`/`eprintln!` are forbidden on production paths (enforced by review
  and, where practical, a clippy lint), matching V14.

## Consequences

- Logs are queryable structured events, and spans tie every event to its check
  context without threading ids by hand.
- Metrics (ADR 0005) and logs stay separate concerns: `MetricSink` carries
  numeric signals to Datadog; `tracing` carries diagnostic events. They are not
  conflated.
- A single subscriber init at startup; the rest of the code just emits
  `tracing` macros, so swapping the output format/collector is one place.

## Alternatives considered

- **`log` + `env_logger`** — simpler, but no spans, so per-check correlation
  would be manual; weaker structured-field support.
- **Hand-rolled JSON to stdout** — full control, but reinvents span propagation,
  filtering, and formatting that `tracing` already provides.
