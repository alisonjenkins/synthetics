# ADR 0012 — Content assertions evaluated in-daemon; bodies never leave the process

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: V22, V18, V6, I.assert
- Relates to: ADR 0005 (MetricSink), ADR 0003 (SIMD)

## Context

A target can return HTTP 200 while serving the wrong content — e.g. an nginx
default landing page instead of the expected login page. Status-only checks miss
this. Something must inspect the response body and decide pass/fail.

Two places could do it:

1. **In the daemon** — assert on the body we already downloaded, emit only the
   result.
2. **On the Datadog side** — ship the body (as logs/metric context) to Datadog
   and assert there.

## Decision

Evaluate all content assertions **in the daemon**. The raw response body never
leaves the process — it is not pushed to any `MetricSink` and not written to any
log (V22). The sink and logs receive only:

- the assertion **outcome** (pass/fail), and
- a **low-cardinality assertion id** naming which assertion failed
  (e.g. `assertion=body_login_present`).

Body assertion types (I.assert): `body_contains`, `body_not_contains`,
`body_regex`, `body_sha256` (fingerprint against a known-good page). The nginx
case is caught by `body_contains("<login marker>")` combined with
`body_not_contains("Welcome to nginx")`.

`body_contains` / `body_not_contains` are byte-substring scans and run on the
SIMD path (ADR 0003), which is exactly the hot scan that decision benchmarks.

## Consequences

- **Cost:** we do not ship response bodies to Datadog. Shipping bodies is
  high-volume, high-cardinality log traffic — paying Datadog for the very thing
  this project exists to replace. Only tiny pass/fail signals cross the wire.
- **Privacy/security:** bodies routinely contain CSRF tokens, session hints,
  and PII. Keeping them in-process avoids exporting sensitive data to an
  external service.
- **Correctness ownership:** the assertion *is* the check outcome (V18), so it
  belongs with the check, not in a downstream system that only sees emitted
  metrics.
- **Datadog's role** is reduced to alerting on the emitted failure metric —
  monitors/alerts on `assertion_failed`, not content inspection.
- The daemon must hold the body in memory long enough to assert, then drop it;
  combined with V15 this means bounded body buffers (a max-body-bytes cap per
  check) so a huge response cannot blow memory.

## Alternatives considered

- **Datadog-side content assertion** — requires exporting bodies (cost + PII),
  and re-introduces dependence on the paid synthetics/log-processing we are
  replacing. Rejected.
- **Hash-only, computed in daemon, compared in Datadog** — leaks less than raw
  bodies, but still pushes per-check fingerprints and moves the pass/fail
  decision off-box for no benefit; the daemon can compare hashes itself.
