# ADR 0005 — `MetricSink` trait; first impl DogStatsD to local agent

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: I.sink, I.ddagent, V2, V9

## Context

Metric pushing must be swappable without touching check or scheduler code
(V2). The first backend is a local Datadog agent over DogStatsD (I.ddagent),
but the design must allow other backends (OTLP, Prometheus remote-write, a
file, a test double) later.

## Decision

Define a single trait, the only path by which the rest of the program emits
metrics:

```rust
#[async_trait]
pub trait MetricSink: Send + Sync {
    async fn record(&self, report: &CheckReport) -> Result<(), SinkError>;
}
```

- `CheckReport` is a backend-neutral value carrying the full V9/V17 signal set
  (outcome, phase timings, sizes, status, cert age, assertion results, tags).
  It knows nothing about DogStatsD line protocol.
- Check and scheduler code hold a `Arc<dyn MetricSink>` and only ever call
  `record`. No DogStatsD string, UDP socket, or tag-formatting appears outside
  a sink impl (V2).
- First impl `DogStatsdSink` formats `CheckReport` into DogStatsD lines and
  sends them to the local agent over UDP or a Unix datagram socket.
- Tests use a `MockSink` that records every `CheckReport` in memory, so check
  behavior is asserted without a real agent.

## Consequences

- Swapping backends is adding one `impl MetricSink` and one line of wiring; no
  check code changes (V2).
- The trait is `async` because network sinks do I/O; `async_trait` is used for
  ergonomics on stable Rust.
- A degraded sink (agent down) is handled inside the impl per V13 — dropped
  with a counter and structured warning, never panicking the check loop.
- `CheckReport` being backend-neutral means adding a new recorded signal is a
  field addition consumed by each sink, keeping V17 parity centralized.

## Alternatives considered

- **Call the Datadog agent directly from checks** — simplest, but hard-wires the
  backend and violates V2.
- **Generic `Sink<T>` over metric type** — more flexible but over-abstracted for
  the current need; one `CheckReport` type covers HTTP/TCP/WS uniformly.
