# Control Plane API contract (T26)

The machine-readable contract is [`control-plane-api.yaml`](control-plane-api.yaml)
(OpenAPI 3.1). This page adds the human context and the **validation rules** a
Terraform/OpenTofu provider should mirror at plan time (V37, ADR 0023).

## Two surfaces

| Surface | Auth | Purpose |
|---------|------|---------|
| **Writer** (`/nodes`, `/endpoints`) | OIDC bearer, scoped to one **product** (V31) | Terraform/CI authors the hierarchy + endpoints |
| **Worker** (`/checks`, `/config/version`, `/workers`, `/workers/heartbeat`) | mTLS, read-only + heartbeat | The fleet pulls resolved checks and registers membership |

Ids are hierarchical paths (`web/globex/prod/login`); the **first segment is the
product** and determines write scope — an OIDC identity may only write ids under
its own product (403 otherwise).

Results are never stored or served here — they flow worker → DogStatsD → Datadog
(V30).

## Change propagation

Workers poll `GET /config/version` (cheap) and pull `GET /checks` when it
changes. The `<5s` push path (Postgres `LISTEN/NOTIFY`) is a store-level
optimization behind the same contract; polling is the correct fallback.

## Validation rules (shared rule set)

The control plane re-validates every write authoritatively and returns a
field-scoped `422` (`{ "error": "validation", "field": ..., "message": ... }`).
A provider should apply the **same** rules at plan time so mistakes fail before
`apply`. These are the single source of truth:

| Entity | Field | Rule |
|--------|-------|------|
| Node | `id`, `kind`, `name` | non-empty |
| Endpoint | `id`, `environment` | non-empty |
| Endpoint (http) | `target.url` | parseable URL; scheme `http` or `https` |
| Endpoint (tcp) | `target.host` | non-empty |
| Endpoint (tcp) | `target.port` | 1–65535 (non-zero) |
| Assertion `status` | — | exactly one of `equals` / `in_range`; `in_range` is `[min ≤ max]` |
| Assertion `header` | `name` | non-empty; at least one of `equals` / `contains` |
| Assertion `body_regex` | `pattern` | compiles as a regex |
| Assertion `body_sha256` | `hex` | exactly 64 lowercase hex chars |

Cross-resource rules that can only be enforced server-side (per-product
authorization, referential integrity, global id uniqueness) are **not**
checkable at plan time and surface as `403`/`422` at `apply`.

## Provider shape (recommended)

Separate resources per check type so field validity is static in HCL (ADR 0019):
`synthetics_http_check`, `synthetics_tcp_check`, with `synthetics_product` /
`synthetics_customer` / `synthetics_environment` for the hierarchy. Assertions
are typed nested blocks, not a stringly-typed `target` (an improvement over
Datadog's schema).
