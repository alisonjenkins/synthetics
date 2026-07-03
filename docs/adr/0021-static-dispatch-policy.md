# ADR 0021 — Static dispatch on runtime/hot paths (no `dyn`/`async_trait`)

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: V35, V15
- Supersedes: the dispatch mechanism of ADR 0005 (the trait-abstraction intent of ADR 0005 stands)

## Context

The initial `MetricSink` design used `#[async_trait]` on top of
`Arc<dyn MetricSink>`. `async_trait` works by boxing a fresh
`Box<dyn Future>` on every call — a heap allocation plus a vtable
indirection on every single check emission. That cost lands squarely on the
hot path (check execution, metric emission, scheduling, ownership
computation) and directly conflicts with the minimal-footprint goal (V15).

## Decision

Runtime and hot-path abstractions use static dispatch instead:

1. **Native async fn in traits via RPITIT**, with an explicit `+ Send`
   bound on the returned future so tasks can still be spawned across
   threads. The `MetricSink` trait is rewritten this way:

   ```rust
   trait MetricSink {
       fn record(&self, r: &CheckReport) -> impl Future<Output = Result<(), SinkError>> + Send;
   }
   ```

2. **Generics plus monomorphization** for consumers of these traits, e.g.
   `Scheduler<M: MetricSink, S: ConfigSource>`, so each concrete
   combination is compiled as its own specialized type with no dynamic
   dispatch at the call site.

3. **`Arc<ConcreteType>`** for shared ownership where needed. `Arc` is a
   reference count, not a dynamic dispatch mechanism, so this doesn't
   reintroduce vtable indirection.

4. **Enum-dispatch** — a hand-rolled `enum AnySink { DogStatsd(..), Mock(..) }`
   implementing the trait via a `match` — for the case where the concrete
   backend is chosen at runtime from a small, known set. This stays static
   and heap-free while remaining runtime-selectable.

The `async_trait` dependency is dropped entirely. This binds hot/runtime
paths specifically: check execution, metric emission, scheduling, and
ownership computation. `dyn` and boxing remain acceptable on cold paths —
for example `#[source] Box<dyn Error>` on error types, or one-time startup
wiring — where the cost is paid once, not per operation.

## Consequences

- No per-emit heap allocation or vtable indirection on the hot path;
  everything is monomorphized and inlinable by the compiler.
- Slightly more type parameters get threaded through code that consumes
  these traits, and one enum wrapper is needed per runtime-selected trait.
- T3 and T4 must be refactored — dropping `async_trait` and moving to
  RPITIT with `+ Send` — before T5 can build on top of the sink. The
  `AnySink` enum-dispatch wrapper is *not* built yet: only one real backend
  (DogStatsD) exists, so a one-variant enum would be premature abstraction
  (V36). It lands when a second backend does.

## Alternatives considered

- **Keep `async_trait` for ergonomics** — rejected; the per-call boxing
  cost lands on the hot path and conflicts with V15.
- **`Box<dyn Trait>` everywhere** — rejected; vtable indirection plus
  allocation on the hot path, for the same reason.
