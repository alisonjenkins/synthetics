# ADR 0017 — Config hierarchy model + CP-side inheritance resolution

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: I.store, V25, V32

## Context

Config is authored against a real organizational hierarchy: an optional
organization level, then product, then customer, then environment (of kind
`prod` or `nonprod`), then endpoint. Teams set sensible defaults at whichever
level makes sense and override them per endpoint where needed. This
hierarchy is also expected to grow additional levels above product over
time, so the model needs room to extend without a rewrite.

## Decision

The hierarchy is modeled as nodes with a generic parent reference, rather
than four hard-coded levels. This means a new level — for example, an
Organization node above Product — can be added later without a schema
rewrite. Each node may carry defaults: interval, timeout, tags, and
assertions.

The control plane resolves inheritance server-side: it walks the tree from
root to endpoint, merging defaults down the path, with endpoint-level
overrides always winning. As part of this walk, the hierarchy automatically
contributes tags to the resolved check — `product:X`, `customer:Y`,
`env:prod`, `environment:Z`, and so on — so metrics are tagged with their
ownership context without any manual tagging by the author. Workers receive
the fully materialized, flat check and never see the hierarchy itself (V25).

Check identity is the hierarchical path — `product/customer/environment/endpoint`
— which is stable and unique. This same path is used both as the rendezvous
key for sharding (ADR 0016) and as the diff key when reconciling a reloaded
config (V32).

For example, if the `environment` node sets `interval = 30s` and the
`endpoint` node overrides `interval = 5m`, the resolved check carries
`interval = 5m` — the endpoint override wins — while still inheriting all of
the tags contributed by the nodes above it:

```
environment node:  interval = 30s
endpoint node:      interval = 5m   (override)

resolved check:
  interval = 5m
  tags = [product:X, customer:Y, env:prod, environment:Z]
```

## Consequences

- Authoring is DRY: defaults are set once at the appropriate level and
  inherited everywhere below it, rather than repeated per endpoint.
- Metrics are automatically tagged with their place in the hierarchy, with
  no separate tagging step required from the author.
- Workers stay simple, since all resolution logic is centralized in the CP
  rather than duplicated across the fleet.
- The CP now owns a non-trivial piece of resolution logic that needs its own
  test coverage, since a resolution bug affects every check beneath the
  buggy node.
- The generic-parent model lets the hierarchy grow upward later without
  migration churn.

## Alternatives considered

- **Flat environment-to-endpoint config plus tags only** — rejected; this
  loses inheritance and the structured, hierarchical notion of ownership.
- **Workers resolve from the raw hierarchy** — rejected; this pushes merge
  logic and a larger data payload onto every worker instead of centralizing
  it once in the CP.
