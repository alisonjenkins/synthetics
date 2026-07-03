# ADR 0016 — Rendezvous-hash sharding, no coordinator; overlap-over-gap rebalance

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: I.membership, V26, V27, V29, V32, V33
- Refines: ADR 0006

## Context

At fleet scale — five thousand to fifty thousand-plus checks — those checks
must be distributed across a worker fleet that scales horizontally. Workers
need to join and leave without a coordinator, without a rebalancing
migration, and above all without ever leaving a check completely unmonitored
during a membership change.

## Decision

Each worker registers itself with the control plane via a heartbeat/lease
(I.membership); the current set of live workers is readable from the CP.
From that live set, each worker independently computes which checks it owns
using **rendezvous hashing** (highest-random-weight): for a given check, the
owning worker is the one that maximizes `hash(check.id, worker.id)` across
all live, eligible workers. A worker schedules exactly the checks it owns.
There is no coordinator and no persisted assignment state — ownership is a
pure function of the current worker set and the check identity.

Ownership is region-aware (V33): only workers whose region matches the
check's configured location(s) are candidates for that check, so the argmax
is taken over a filtered candidate set, not the whole fleet.

```rust
// sketch: ownership computation for a single check
fn owner<'a>(check_id: &CheckId, live_workers: &'a [WorkerId], region: Region) -> &'a WorkerId {
    live_workers
        .iter()
        .filter(|w| w.region() == region)
        .max_by_key(|w| hash(check_id, *w))
        .expect("at least one worker in region")
}
```

When the worker set changes, ownership shifts for only a minority of checks
and self-heals without any external action. Rebalancing deliberately prefers
**overlap over gap** (V27): during a membership transition, two workers may
briefly both run the same check, producing a harmless duplicate data point
downstream, but no check is ever left with zero owners.

## Consequences

- No coordinator needs to be built or operated; assignment is deterministic
  and self-healing purely from the live worker set.
- A worker joining or leaving only moves roughly `1/N` of checks, keeping
  rebalance cheap and localized.
- During a control-plane outage, the worker set is frozen from every
  worker's point of view — a worker cannot observe a dead peer without the
  CP. If a worker dies mid-outage, its checks gap until the CP returns and
  membership can be re-observed. This is an accepted trade-off of the
  run-cached-forever policy (V29): workers keep running their last-known
  checks through a CP outage, at the cost of not being able to redistribute
  a dead peer's load until the CP is back.
- Rebalance windows can produce duplicate data points for the same check;
  this is treated as harmless noise downstream rather than an error.

## Alternatives considered

- **Central coordinator assigns shards** — rejected; introduces a new
  stateful, failure-prone component that the rendezvous approach avoids
  entirely.
- **Queue of check jobs** — rejected; a job queue breaks per-check interval
  fidelity (V5) and adds a broker as a new dependency.
- **Consistent hashing (ring)** — viable, but rendezvous hashing gives
  simpler, more even distribution with weights and no ring bookkeeping to
  maintain.
