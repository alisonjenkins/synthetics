# SPEC — synthetics

Caveman format. Identifiers/paths verbatim. Numbering monotonic.

## §G — goal

Rust daemon. Run many HTTP + TCP synthetic checks (WebSocket later). Push
result metrics thru swappable sink (first sink = local Datadog agent). Hot
reconfigure — add/remove endpoints, no process restart, no missed scheduled
check. Replace pricey Datadog synthetics.

## §C — constraints

- Idiomatic Rust.
- `thiserror`. Per-function error = own enum. One variant per distinct
  failure that fn can hit. No shared god-error.
- SIMD where real win. Scalar fallback correct + equal result.
- Scale: many endpoints, one daemon. Async + bounded concurrency, no
  thread-per-endpoint blowup.
- Metric push behind trait. Swap sink, no touch check code.
- Initial sink: local Datadog agent (DogStatsD, UDP or UDS).
- Reconfigure live. No restart. No dropped/skipped check during reload.
- TDD. Test first, fail, then code.
- Clippy `deny` on `unwrap` + `expect`. No panic-on-error in prod paths.
- Structured logging via `tracing`. Machine-parseable events, no free-form
  printf. Spans across check lifecycle.
- Minimal mem + CPU footprint. Small idle cost, flat mem under many
  endpoints. No per-check heap churn on hot path.
- Benchmarks. Bench hot paths (SIMD scan, scheduler tick, check exec).
  Guard perf regressions.
- Datadog-synthetic parity: record every signal a Datadog synthetic would
  (phase timings, sizes, status, cert age, redirects, assertion results).
- Fault isolation. Purpose = monitor many possibly-unhealthy systems. Slow,
  hung, or dead target must never delay, block, or fail checks of other
  targets. Failure is the expected case, not the exception.
- WebSocket = future, design must not block it.

## §I — external surfaces

- `I.config` — config source: endpoint list + per-endpoint interval +
  target params. Reloadable.
- `I.ctl` — reload trigger: SIGHUP and/or config-file watch.
- `I.http` — HTTP target endpoints under test.
- `I.tcp` — TCP target endpoints under test.
- `I.ws` — WebSocket targets. Future.
- `I.assert` — per-endpoint assertions. Drive ok/fail outcome. Types:
  `status` (code/range), `header` (name→match), `latency` (threshold),
  `cert_min_age` (TLS days-to-expiry), and body checks — `body_contains`,
  `body_not_contains`, `body_regex`, `body_sha256` (fingerprint vs
  known-good). Body checks catch "200 but wrong content" (e.g. nginx
  default page instead of login).
- `I.sink` — `MetricSink` trait. Swappable metric backend.
- `I.ddagent` — local Datadog agent DogStatsD socket. First `I.sink` impl.
- `I.log` — `tracing` subscriber: structured JSON events + spans to
  stdout/collector.

## §V — invariants

- V1 — every fn that can fail returns own `thiserror` enum; one variant per
  distinct failure mode of that fn.
- V2 — no metric backend call outside an `I.sink` impl. Check + schedule
  code talk only to `MetricSink` trait.
- V3 — reconfigure never drops or skips a check already due. No coverage gap
  across reload.
- V4 — reconfigure mutates running process only. No restart, no re-exec.
- V5 — scheduler honors per-endpoint interval within tolerance. No unbounded
  drift.
- V6 — SIMD path + scalar fallback produce identical result. Fallback always
  present.
- V7 — every feature TDD: failing test committed before impl.
- V8 — N endpoints scale sublinear in OS threads. Bounded concurrency pool.
- V9 — each check emits metric: outcome (ok/fail), latency, and detail
  (status code / connect ms).
- V10 — HTTP check records phase timings (resolve, connect, TTFB, total);
  TCP check records connect time.
- V11 — reload with bad config = reject new config, keep old running. No
  crash, no partial apply.
- V12 — `clippy::unwrap_used` + `clippy::expect_used` at deny. CI fails on
  either. Test code may allow via scoped attr.
- V13 — errors handled where possible: recover/retry/degrade at source when
  fn can. Propagate only when caller must decide. No swallow-silent, no
  blind bubble-to-top.
- V14 — all logging via `tracing`: structured fields + spans. No `println!`
  / `eprintln!` in prod paths. Each check runs in a span carrying endpoint
  id + outcome.
- V15 — low footprint: idle CPU near zero (event-driven, no busy poll);
  mem flat as endpoint count grows (bounded buffers, reused allocations,
  no per-check heap churn on hot path).
- V16 — hot paths have benchmarks (`criterion` or bencher). SIMD path
  benched vs scalar to prove win (else drop SIMD per V6). Bench in CI to
  catch regression.
- V17 — Datadog parity. HTTP check records: DNS resolve, TCP connect, TLS
  handshake, TTFB, download, total time, response size bytes, status code,
  redirect count, TLS cert days-to-expiry. TCP check records: connect time,
  (opt) send/recv bytes. All emitted per check thru `I.sink`.
- V18 — assertions per endpoint (`I.assert`) evaluated each check; each
  assertion emits pass/fail; any fail = check outcome fail (V9). Assertion
  set is reloadable config (V3/V11).
- V19 — every check bounded by hard timeout (per-phase + total). No check
  runs unbounded. Timeout = emitted failure outcome (V9), not a hang.
- V20 — fault isolation: one target slow/hung/dead never delays or blocks
  another target's scheduled check. No head-of-line blocking. Dead targets
  cannot starve healthy targets of concurrency/execution.
- V21 — panic in one check task is caught + isolated; that check = failure
  outcome, daemon + all other checks keep running. No cascade.
- V22 — response bodies asserted in-daemon only. Raw body never pushed to
  any sink or log. Sink/log receive assertion outcomes + low-cardinality
  assertion id (which check failed), never body content. Cost + PII guard.
  Body scans (`body_contains`/`body_not_contains`) use SIMD path (V6).

## §T — tasks

| id | st | task | cites |
|----|----|------|-------|
| T1 | x | scaffold cargo workspace, CI, test harness, clippy lint gate | V7,V12 |
| T2 | x | config model + parse (endpoints, intervals, params) | I.config,V1,V11 |
| T3 | . | `MetricSink` trait + mock/no-op sink for tests | I.sink,V2,V9 |
| T4 | . | Datadog agent sink (DogStatsD UDP/UDS) | I.ddagent,V2 |
| T5 | . | HTTP check executor + per-fn error enum + full DD-parity timings + timeouts | I.http,V1,V9,V10,V17,V19 |
| T6 | . | TCP check executor + per-fn error enum + connect timing + timeout | I.tcp,V1,V9,V10,V17,V19 |
| T7 | . | scheduler: per-endpoint interval, bounded concurrency, fault isolation | V5,V8,V20,V21 |
| T15 | . | fault-isolation harness: timeout, panic catch, no-starvation test | V19,V20,V21 |
| T8 | . | hot reload: watch/signal, diff config, no missed check | I.config,I.ctl,V3,V4,V11 |
| T9 | . | SIMD hot paths (body_contains scan, header scan) + scalar fallback | V6,V22 |
| T10 | . | daemon wiring: startup, graceful shutdown | V4 |
| T12 | . | `tracing` subscriber: JSON structured logs + span wiring | I.log,V14 |
| T13 | . | benchmarks for hot paths (SIMD vs scalar, scheduler, check) | V16,V6,V15 |
| T14 | . | assertion engine (status/header/latency/cert-age + body contains/regex/sha256) | I.assert,V18,V9,V22 |
| T11 | . | WebSocket check (future) | I.ws,V1,V9 |

## §B — bugs

| id | date | cause | fix |
|----|------|-------|-----|
