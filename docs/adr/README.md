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
