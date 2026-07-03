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
- `cargo-deny` in CI: vet dependency licenses (allow-list only, no toxic
  copyleft), security advisories, source provenance.
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
- Horizontal scale to a worker fleet (5k–50k+ checks). Checks sharded across
  workers by rendezvous hash over a live heartbeat worker-set; no
  coordinator; rebalance prefers overlap over gap (at-least-once).
- Central config via a control plane. Authoring hierarchy
  (org?→product→customer→environment→endpoint) stored behind a `ConfigStore`
  trait (Postgres first). Control plane resolves inheritance server-side and
  serves flat, materialized checks. Config = source of truth; results never
  stored (they go to the sink → Datadog).
- Config authored via Terraform/OpenTofu. Provider deferred; CP API contract
  specced now. Writers auth via OIDC (scoped per-product); workers auth via
  mTLS (read-only). Config change reaches workers < 5s.
- TF provider = great UX: generated docs, actionable error messages on
  mistakes, validation at plan time where possible (typed schema +
  validators). CP re-validates every write authoritatively (never trust
  client). Provider + CP share one rule set.
- Worker consumes a flat resolved-check list via a `ConfigSource` trait
  (File for dev/single-node/tests; ControlPlane for fleet). Same runtime,
  swappable source. Worker caches config, survives control-plane outage.
- Static dispatch on runtime/hot paths. No `dyn` trait objects or
  `async_trait` boxing on check/emit/schedule paths — use RPITIT + `Send`,
  generics/monomorphization, `Arc<Concrete>`, enum-dispatch for
  runtime-selected backends. `dyn`/boxing only in cold paths (startup, error
  `source`).
- Deployable to Kubernetes via a Helm chart (worker fleet + control plane).
  Local dev/test on a KIND cluster with a local image registry;
  CloudNativePG operator provisions the Postgres store for local testing.
  IaC-first.
- Nix flake: devshell (dev tooling) + a package per binary (worker daemon,
  control-plane server) + nix-built OCI container images bundling each
  binary with its runtime deps. Reproducible builds.
- Local orchestration: `just` recipes + Tilt stand up the whole local stack
  (KIND + registry + CloudNativePG + build/push images + deploy Helm) with
  one command.
- Local testing uses a mock Datadog (DogStatsD receiver + viewer), never
  real Datadog. Exercises the real `DogStatsdSink` wire path; captured
  metrics viewable locally.
- Rust docs done well: rustdoc on every public item (`missing_docs` lint
  on, `cargo doc` clean), doc examples where useful. Repo `README.md` with
  overview + quickstart + an SVG architecture diagram.
- Easy to change. Seams on KNOWN axes of variation (check protocols,
  assertion types, sinks, config sources, storage backends): a new variant =
  additive, localized, compiler-guided (exhaustive match / trait impl). No
  speculative abstraction — no seam without a named axis (anti-gold-plating).
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
- `I.configsource` — `ConfigSource` trait feeding the worker: `load()` +
  `watch()` a stream of flat resolved checks. Impls: File (TOML, wraps
  `I.config`), ControlPlane (CP client). Static dispatch.
- `I.store` — `ConfigStore` trait behind the control plane: hierarchy CRUD,
  resolve-effective-checks, worker heartbeat upsert + list-live, config
  version + generic change-signal. Impls: Postgres (NOTIFY-backed), in-memory
  (tests), maybe SQLite (dev). No SQL outside the impl.
- `I.cp` — control-plane API. Writer side (OIDC, per-product): hierarchy
  CRUD. Worker side (mTLS, read-only): pull resolved checks + worker-set,
  watch change-feed (< 5s), version stamp. Contract specced for a future TF
  provider.
- `I.membership` — worker registry: heartbeat/lease; live worker-set drives
  rendezvous ownership.
- `I.auth` — OIDC for writers (scoped per-product), mTLS for workers.

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
- V23 — every dependency license on `cargo-deny` allow-list (permissive
  only; copyleft/toxic denied by omission). CI fails on unapproved license,
  RUSTSEC advisory, unknown source, or wildcard dep.
- V24 — worker runtime consumes a flat resolved-check list via `ConfigSource`
  (`I.configsource`). Behavior identical across File and ControlPlane sources.
- V25 — control plane resolves hierarchy inheritance server-side; workers
  never see the hierarchy — only materialized flat checks + tags.
- V26 — check ownership by rendezvous hash over the live worker-set; each
  worker selects its own checks independently. No coordinator.
- V27 — rebalance on membership change prefers overlap over gap: no check
  ever unowned (at-least-once). Brief duplicate runs acceptable.
- V28 — config change propagates to workers < 5s (change-signal + version
  stamp). Reconfigure still misses no check (V3 at fleet scale).
- V29 — control-plane/store outage: worker keeps running last-known cached
  config indefinitely; never stops checking. Membership frozen during outage
  (documented worker-death gap risk).
- V30 — store holds config + fleet membership only. Never check results /
  time-series (results → sink → Datadog).
- V31 — writes authenticated via OIDC, scoped per-product (identity may
  mutate only its product subtree). Workers authenticate via mTLS, read-only.
- V32 — check identity is hierarchical + stable
  (product/customer/environment/endpoint), unique. Serves as rendezvous key
  and reload-diff key (extends V3's stable-key need).
- V33 — region-aware: checks carry location(s), workers carry region; a check
  runs only on workers in its location(s).
- V34 — control plane depends only on `ConfigStore` (`I.store`); no
  backend-specific code (SQL, `LISTEN/NOTIFY`) outside the impl. Swappable.
- V35 — runtime/hot-path abstractions are static-dispatch (RPITIT+`Send`,
  generics, `Arc<Concrete>`, enum-dispatch). No `dyn`/`async_trait` on
  check/emit/schedule paths. `dyn`/boxing only in cold paths.
- V36 — extension is localized + compiler-guided: adding a protocol /
  assertion / sink / source / store touches only its module + the enum/trait
  def; exhaustive matches + trait impls flag every site needing update. Seams
  on known axes only (no speculative abstraction).
- V37 — validation defense-in-depth: TF provider validates at plan time
  where possible (typed schema + custom validators) with actionable,
  field-scoped messages; CP independently validates every write and rejects
  invalid config with clear field-scoped errors (never trusts the client).
  One shared rule set drives both. Provider ships generated docs.
- V38 — every public Rust item documented (`missing_docs` lint on; CI
  `-D warnings` enforces; `cargo doc` builds clean). Repo `README.md`
  present with overview, quickstart, and an SVG architecture diagram.
- V39 — local/test runs never emit to real Datadog. Default local target is
  the mock Datadog receiver; real Datadog endpoints require explicit
  non-default config.

## §T — tasks

| id | st | task | cites |
|----|----|------|-------|
| T1 | x | scaffold cargo workspace, CI, test harness, clippy + cargo-deny gates | V7,V12,V23 |
| T2 | x | config model + parse (endpoints, intervals, params) | I.config,V1,V11 |
| T3 | x | `MetricSink` trait + mock/no-op sink for tests | I.sink,V2,V9 |
| T4 | x | Datadog agent sink (DogStatsD UDP/UDS) | I.ddagent,V2 |
| T5 | x | HTTP check executor + per-fn error enum + full DD-parity timings + timeouts | I.http,V1,V9,V10,V17,V19,V35,V36 |
| T6 | x | TCP check executor + per-fn error enum + connect timing + timeout | I.tcp,V1,V9,V10,V17,V19,V35,V36 |
| T7 | x | scheduler: per-endpoint interval, bounded concurrency, fault isolation | V5,V8,V20,V21 |
| T15 | x | fault-isolation harness: timeout, panic catch, no-starvation test | V19,V20,V21 |
| T8 | x | hot reload: watch/signal, diff config, no missed check | I.config,I.ctl,V3,V4,V11 |
| T9 | x | SIMD hot paths (body_contains scan) + scalar fallback | V6,V22 |
| T10 | x | daemon wiring: startup, graceful shutdown | V4 |
| T12 | x | `tracing` subscriber: JSON structured logs + span wiring | I.log,V14 |
| T13 | x | benchmarks for hot paths (SIMD vs scalar body scan) | V16,V6,V15 |
| T14 | x | assertion engine (status/header/latency/cert-age + body contains/regex/sha256) | I.assert,V18,V9,V22 |
| T11 | . | WebSocket check (future) | I.ws,V1,V9,V35,V36 |
| T16 | x | refactor `MetricSink` to static dispatch (RPITIT+Send, drop async_trait; enum-dispatch deferred to 2nd backend per V36) | V35,V36 |
| T17 | x | endpoint→CheckReport orchestrator (match Target, run executor, apply assertions, build report; no trait per V36) | V9,V18,V35,V36 |
| T18 | ~ | `ConfigSource` trait + FileConfigSource (wraps T2) + scheduler/reload consume it | I.configsource,V24,V35 |
| T19 | x | rendezvous-hash ownership + worker identity + region/location match | V26,V27,V32,V33 |
| T20 | x | worker heartbeat client + membership consumption | I.membership,V26 |
| T21 | x | ControlPlaneConfigSource: CP client + change-feed + local cache + outage survival | I.cp,I.configsource,V28,V29,V24 |
| T22 | x | `ConfigStore` trait + in-memory impl + Postgres impl + migrations | I.store,V30,V34,V35 |
| T23 | x | CP inheritance resolver (hierarchy → flat resolved checks) | I.store,V25 |
| T24 | x | CP writer API (OIDC, per-product scope, hierarchy CRUD) + write validation | I.cp,I.auth,V31,V32,V37 |
| T25 | x | CP worker API (mTLS, read + version stamp, heartbeat) | I.cp,I.auth,V28,V33 |
| T26 | x | CP API contract doc (OpenAPI) + validation rules for future TF provider | I.cp,V37 |
| T27 | x | repo README + SVG architecture diagram + `missing_docs` lint + rustdoc pass | V38 |
| T30 | x | control-plane server binary (PgStore + writer/worker routers + serve) | I.cp,I.store,I.auth |
| T31 | x | worker daemon control-plane mode (ControlPlaneConfigSource + heartbeat + poll reload) | I.configsource,I.membership,V28 |
| T28 | . | Helm chart (worker + CP + CNPG) + KIND cluster + local registry | I.cp,I.store |
| T32 | . | nix flake: devshell + packages (both binaries) + nix-built containers | (build/deploy) |
| T33 | . | just + Tilt stand up local stack (kind+registry+cnpg+images+helm) | (build/deploy) |
| T29 | . | mock Datadog (DogStatsD receiver + viewer) for local/test | I.ddagent,V39 |

## §B — bugs

| id | date | cause | fix |
|----|------|-------|-----|
