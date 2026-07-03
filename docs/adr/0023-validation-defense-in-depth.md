# ADR 0023 — Validation defense-in-depth + provider UX

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: V37, V11, V1
- Relates to: ADR 0019 (provider strategy), ADR 0004 (per-function errors)

## Context

Config is authored by humans through Terraform/OpenTofu across many teams. Two
things follow. First, mistakes should be caught as early and as clearly as
possible — ideally at `terraform plan`, not at `apply` and certainly not after
a check silently misbehaves in production. Second, the control plane cannot
trust its clients: a bad or malicious request, a stale provider, or a direct API
call must never persist invalid config. The daemon already rejects bad config
wholesale (V11); that guarantee must extend up to the write path.

## Decision

Validate at **two layers with one shared rule set**.

**1. Provider, at plan time (UX layer).** The Terraform provider validates as
much as possible during `plan`, before anything is sent to the CP:

- Typed, per-resource schemas (`synthetics_http_check` / `synthetics_tcp_check`
  — separate resources per ADR 0019) so field validity is static: an HTTP-only
  field on a TCP check is a schema error, not an apply-time surprise.
- Custom attribute validators for value-level rules: URL well-formedness and
  scheme, interval/timeout bounds and ordering, port range, assertion params
  (regex compiles, sha256 is 64 hex chars, status matcher is exactly one of
  equals/range), duration formats.
- Error messages are actionable and field-scoped — they name the attribute, the
  bad value, and the fix — matching the quality of the daemon's
  `ConfigValidateError` (ADR 0004).

Plan-time validation is best-effort UX: anything that needs cross-resource or
server-side state (e.g. product-scope authorization, uniqueness of a check path
across the fleet) cannot be fully checked at plan time and falls to layer 2.

**2. Control plane, on every write (authority layer).** The CP re-validates
every write request independently and authoritatively — it never assumes the
provider validated correctly. It applies the same value rules plus the checks
only it can do (authorization scope per ADR 0018, hierarchy referential
integrity, global uniqueness). Rejections return clear, field-scoped API errors
(structured: field, code, message) that the provider surfaces verbatim.

**One rule set.** The value-level rules (bounds, formats, assertion validity)
are defined once as the source of truth and consumed by both the CP validator
and — via the API contract / generated docs (T26) — the provider, so the two
layers never drift. The provider ships generated documentation from its schema.

## Consequences

- Most mistakes fail fast at `plan` with a helpful message; the rest fail at
  `apply` with a clear CP error — never silently.
- The CP is safe against any client (stale provider, curl, another language's
  provider): invalid config cannot be persisted.
- Keeping the rule set single-sourced is real work — the contract (T26) must
  express the value rules precisely enough for the provider to mirror them, and
  a drift test should assert the two agree.
- Great error messages are a maintained artifact, not an afterthought — held to
  the same bar as the daemon's typed errors.

## Alternatives considered

- **CP-only validation** — simplest, but every mistake waits until `apply` and
  the plan looks deceptively clean; poor operator UX.
- **Provider-only validation** — good UX but unsafe: the CP would trust clients
  and could persist invalid config from any non-provider caller.
- **Duplicated rules in each layer** — fast to write, but the two inevitably
  drift; a shared rule set + drift test is worth the upfront cost.
