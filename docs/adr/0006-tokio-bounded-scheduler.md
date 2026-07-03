# ADR 0006 — Tokio runtime + bounded-concurrency scheduler

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: V5, V8, V15, V3

## Context

One daemon must run checks for many endpoints, each on its own interval (V5),
without spawning a thread per endpoint (V8) and with a flat memory profile and
near-zero idle CPU (V15). Reloads must not skip a due check (V3).

## Decision

- **Async runtime: Tokio** (multi-thread), one worker pool sized to CPU count.
  Each check is a task, not a thread; thousands of endpoints map to thousands
  of cheap futures over a small thread pool (V8).
- **Scheduling: per-endpoint timers, not a busy loop.** Each scheduled endpoint
  is driven by `tokio::time` (interval / sleep-until), so an idle daemon parks
  in the reactor with no CPU spin (V15). No polling every N ms to see what is
  due.
- **Concurrency cap: a `Semaphore`.** The number of checks executing at once is
  bounded by a permit pool, independent of how many endpoints exist. This caps
  peak memory and open sockets regardless of endpoint count (V15) and prevents
  a thundering herd when many intervals align.
- **Next-fire time is owned per endpoint and preserved across reload** so an
  unchanged endpoint keeps its schedule when config swaps (V3, see ADR 0002).
- **Timing tolerance:** fire time is computed from a fixed epoch + interval
  (not "now + interval" after each run), so per-check execution latency does not
  accumulate into schedule drift (V5).

## Consequences

- Scales to many endpoints on modest hardware; the cost is per-task memory
  (small) not per-thread (large).
- Idle cost is essentially the reactor waiting on timers — near-zero CPU (V15).
- A slow or hung check consumes one permit and one task, not the whole pool; a
  per-check timeout bounds it so it cannot hold a permit forever.
- Requires care that dropping/cancelling a task on reload does not leave a
  half-sent metric; the check completes or is cancelled at an await point with
  no partial external effect.

## Alternatives considered

- **Thread per endpoint** — simple mental model, but memory and context-switch
  cost grow linearly with endpoints; violates V8/V15.
- **Single global tick loop scanning all endpoints each tick** — easy, but
  either wastes CPU at fine resolution or drifts at coarse resolution;
  per-endpoint timers give precise, cheap scheduling.
- **`async-std` / `smol`** — viable, but Tokio has the widest ecosystem
  (hyper, tokio-rustls, tracing) the rest of the stack already uses.
