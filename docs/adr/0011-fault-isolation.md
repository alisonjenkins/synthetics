# ADR 0011 — Fault isolation between targets

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: V19, V20, V21, V8, V5
- Refines: ADR 0006 (bounded scheduler)

## Context

The system exists to monitor many systems that may be unhealthy — that is the
job, not an edge case. A slow, hung, or dead target must never delay, block, or
fail the checks of any other target (V20). This is the central reliability
property: the monitor must stay healthy while the things it monitors are not.

Three failure modes threaten this:

1. **Unbounded checks** — a target that accepts a connection then never responds
   would hang a check forever (V19).
2. **Head-of-line blocking / starvation** — many dead targets holding execution
   resources could stop healthy targets from being checked on time (V20).
3. **Panic cascade** — a panic while checking one target must not take down the
   worker or other in-flight checks (V21).

## Decision

**1. Hard timeouts, layered (V19).** Every check has a total-deadline wrapper
(`tokio::time::timeout`) and per-phase timeouts (DNS, connect, TLS, TTFB,
overall). A timeout is a normal *failure outcome* recorded via the sink, not a
hang and not an error that skips reporting. No network operation is issued
without a deadline.

**2. Independent tasks + drift-free timers (V20).** Each endpoint's check is its
own Tokio task fired by its own timer (ADR 0006). A slow task's `await` yields
the worker thread back to the runtime, so other endpoints' tasks run
concurrently — one hung target parks in the reactor, it does not occupy a
thread. Fire times are computed from a fixed epoch so a late-finishing check
does not push out the next endpoint's schedule.

**3. No starvation under mass failure (V20).** The global concurrency semaphore
(ADR 0006) bounds *peak* parallelism, but because every permit is released
within the check timeout (decision 1), a wave of dead targets cannot hold
permits indefinitely and lock out healthy ones. Additionally:
   - at most **one in-flight check per endpoint** (an endpoint that is still
     running when its next tick arrives skips that tick and records an overrun,
     rather than piling up), so one slow target cannot consume multiple permits;
   - the concurrency cap is sized so that `cap` >> expected simultaneously-slow
     targets; if permit pressure is observed, it is itself a metric.

**4. Panic isolation (V21).** Each check runs in a task whose `JoinHandle` is
awaited by the scheduler; a panicked task surfaces as `Err(JoinError)`, which the
scheduler converts into a recorded failure outcome for that endpoint and
continues. `panic = "unwind"` is kept (not `abort`) so a single check panic is
contained rather than aborting the process. Combined with ADR 0008 (no
`unwrap`/`expect`), panics should be rare; this is defense in depth.

## Consequences

- The monitor's health is decoupled from the targets' health — the core purpose.
- Every check path must thread a deadline; this is enforced by the check
  function signatures taking a timeout budget, not by discipline alone.
- Test surface (T15): a stub server that hangs/refuses/panics, asserting that a
  healthy endpoint's check still fires on time and reports while the bad one
  times out. This is the invariant that most needs an automated guard.
- Slight overhead of a per-check timeout wrapper and per-endpoint in-flight
  flag; negligible against the reliability gained.

## Alternatives considered

- **Thread-per-target isolation** — strong isolation, but violates V8/V15
  (memory scales with targets) and does not by itself bound a hung check.
- **No per-endpoint in-flight limit** — simpler, but a persistently slow target
  could occupy several permits across ticks and pressure the healthy set;
  rejected for V20.
- **`panic = "abort"` + supervisor** — a process restart loses the
  no-missed-check guarantee (V3) and all idle scheduling state; in-process
  isolation is strictly better here.
