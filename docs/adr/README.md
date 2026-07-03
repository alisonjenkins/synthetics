# Architecture Decision Records

Each ADR captures one load-bearing decision: context, the choice, and its
consequences. ADRs are immutable once `Accepted` — to reverse one, add a new
ADR that supersedes it rather than editing history.

Format: [MADR](https://adr.github.io/madr/)-lite.

| ADR | Title | Status |
|-----|-------|--------|
| [0001](0001-http-stack-hyper-rustls.md) | Low-level HTTP stack (hyper + tokio-rustls) for phase timings | Accepted |
| [0002](0002-hot-reload-via-sighup.md) | Hot reload via SIGHUP + config diff | Accepted |
| [0003](0003-simd-std-arch-runtime-detect.md) | SIMD via `std::arch` intrinsics with runtime detection | Accepted |
| [0004](0004-per-function-error-enums.md) | Per-function `thiserror` error enums | Accepted |
| [0005](0005-metricsink-trait-abstraction.md) | `MetricSink` trait; first impl DogStatsD to local agent | Accepted |
| [0006](0006-tokio-bounded-scheduler.md) | Tokio runtime + bounded-concurrency scheduler | Accepted |
| [0007](0007-structured-logging-tracing.md) | Structured logging via `tracing` (JSON) | Accepted |
| [0008](0008-clippy-deny-unwrap-expect.md) | Clippy deny on `unwrap`/`expect` | Accepted |
| [0009](0009-workspace-layout.md) | Single-member cargo workspace | Accepted |
| [0010](0010-tdd-and-benchmarks.md) | TDD workflow + criterion benchmarks | Accepted |
| [0011](0011-fault-isolation.md) | Fault isolation between targets (timeouts, no starvation, panic isolation) | Accepted |
| [0012](0012-content-assertions-in-daemon.md) | Content assertions in-daemon; bodies never leave the process | Accepted |
| [0013](0013-supply-chain-cargo-deny.md) | Supply-chain gating with cargo-deny (licenses, advisories, sources) | Accepted |
| [0014](0014-control-plane-config-source-of-truth.md) | Control plane + Postgres as config source of truth; results still flow via Datadog | Accepted |
| [0015](0015-configsource-abstraction.md) | `ConfigSource` trait; flat resolved-check runtime model (file or control-plane) | Accepted |
| [0016](0016-rendezvous-sharding.md) | Rendezvous-hash sharding, no coordinator; overlap-over-gap rebalance | Accepted |
| [0017](0017-hierarchy-model-and-resolution.md) | Generic hierarchy model with CP-side inheritance resolution | Accepted |
| [0018](0018-auth-and-propagation.md) | OIDC for writers, mTLS for workers; sub-5s config propagation | Accepted |
| [0019](0019-tf-provider-strategy.md) | Terraform/OpenTofu provider strategy — separate resources per check type | Accepted |
| [0020](0020-configstore-abstraction.md) | `ConfigStore` trait; Postgres impl first (LISTEN/NOTIFY change-signal) | Accepted |
| [0021](0021-static-dispatch-policy.md) | Static dispatch on runtime/hot paths (no `dyn`/`async_trait`) | Accepted |
| [0022](0022-design-for-change.md) | Design for change — seams on known axes only, anti-gold-plating | Accepted |
| [0023](0023-validation-defense-in-depth.md) | Validation defense-in-depth (plan-time provider + authoritative CP) | Accepted |
| [0024](0024-hybrid-jsonb-schema.md) | Hybrid jsonb + scalar-column schema for the config store | Accepted |
