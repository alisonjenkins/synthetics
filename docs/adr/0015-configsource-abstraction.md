# ADR 0015 — `ConfigSource` abstraction; flat resolved-check runtime model

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: I.configsource, V24, V35
- Relates to: ADR 0005, ADR 0014

## Context

The worker was built during T1-T4 around a simple model: run a flat list of
checks loaded from a TOML file. Introducing a control plane (ADR 0014) adds a
fleet-scale source of that same list, but it must not force a rewrite of the
worker's runtime, and the existing local/dev/offline path must keep working
unchanged — for single-node deployments, tests, and air-gapped environments
where a control plane is not available or not wanted.

## Decision

A `ConfigSource` trait is introduced as the seam between "where checks come
from" and "how checks are run." It exposes `load()`, which returns the
current flat set of resolved checks, and `watch()`, which yields a stream of
updated check-sets as they change. The trait uses static dispatch, per the
policy in ADR 0021.

Two implementations exist. `FileConfigSource` wraps the existing TOML config
introduced in T2 and is used for dev, single-node deployments, tests, and
air-gapped operation. `ControlPlaneConfigSource` is used for fleet
deployments; it is a CP client that consumes the change-feed described in
ADR 0018 and maintains a local cache so the worker keeps running through
transient CP unavailability.

The worker runtime consumes only the `ConfigSource` trait and behaves
identically regardless of which implementation backs it (V24). All hierarchy
resolution, inheritance, and control-plane awareness live above this seam;
by the time a check reaches the worker, it is flat and fully resolved. The
worker has no concept of the hierarchy at all.

## Consequences

- The T1-T4 worker runtime is preserved as-is; the control plane is additive
  behind a seam that was already proven out before the CP existed.
- Tests can run against `FileConfigSource` or an in-memory source with zero
  infrastructure, and single-node deployment remains as simple as pointing
  at a TOML file.
- There are now two implementations of the trait that must be kept
  behavior-compatible, which is an ongoing (if small) maintenance cost.

## Alternatives considered

- **Control-plane only, dropping file mode** — rejected; this removes the
  dev/offline deployment path entirely and would force every test to stand
  up a CP stub instead of using a plain file or in-memory source.
