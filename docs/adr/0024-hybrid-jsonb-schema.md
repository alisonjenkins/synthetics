# ADR 0024 — Hybrid jsonb + scalar-column schema for the config store

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: I.store, V30, V34, V36
- Relates to: ADR 0020 (ConfigStore abstraction)

## Context

The Postgres `ConfigStore` (ADR 0020) must persist the config hierarchy (nodes,
endpoints) and worker membership. A natural question is whether to fully
normalize every field into columns, or store the config payload as `jsonb`.
`jsonb` has a reputation for being slower than plain columns — deservedly, when
misused.

The relevant workload characteristics:

- **Writes are rare** — config is authored via Terraform at human/CI cadence,
  not high-QPS OLTP.
- **Reads query by scalar key** — an endpoint is fetched by `id`, or the whole
  set is listed so a worker can compute rendezvous ownership locally. We never
  filter or aggregate on fields *inside* the config.
- **Whole-row replacement** — endpoints/nodes are replaced, never partially
  updated.
- **No results stored** (V30) — the store never becomes a large time-series
  table; it holds config + membership only, staying small.

## Decision

Use a **hybrid** schema: normalize into scalar columns exactly the fields the
control plane queries, filters, or joins on, and store the remaining opaque
payload as `jsonb`.

- `node(id PK, kind, parent, data jsonb)`
- `endpoint(id PK, environment, data jsonb)`
- `worker(id PK, region, last_heartbeat_ms)`
- `config_state(version)` bumped in the same transaction as each write.
- Indexes on the scalar filter columns: `node(parent)`, `endpoint(environment)`.

The `data` jsonb holds the target spec, assertions, defaults, tags, and
interval/timeout overrides — the parts we always read as a whole and never query
into. Serde round-trips it exactly.

## Consequences

- Queries the CP actually runs (`WHERE id = $1`, `WHERE environment = $1`,
  `SELECT data` list-all of a small table) hit indexed scalar columns or a cheap
  sequential scan; `jsonb` is on the payload only, so its cost is a negligible
  parse, dwarfed by network time.
- The schema is stable as the model evolves — a new assertion type or target
  field is a serde change with **zero migration** (V36, easy to change). Full
  normalization would require a schema/migration change per new field.
- We give up efficient filtering *inside* the config (e.g. "all endpoints with a
  `body_regex`"). We never need that: workers pull everything and resolve
  locally. If it is ever needed, a GIN index on `data` handles moderate cases
  before a full normalize is warranted.
- Because results are not stored (V30), the store stays small and read-light, so
  the seq-scan list-all remains cheap at 5k–50k endpoints.

## Alternatives considered

- **Full normalization** (separate assertion table, target columns) — best for
  querying inside the config, which we do not do; costs join complexity and a
  migration per model change (fights V36). Rejected for this workload.
- **jsonb for everything** including id/kind/parent/environment — would force
  filtering on json fields, the actual jsonb anti-pattern. Rejected; those are
  scalar columns.
