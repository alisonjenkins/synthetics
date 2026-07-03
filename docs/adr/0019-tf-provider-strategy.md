# ADR 0019 — Terraform/OpenTofu provider strategy

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: I.cp, I.http, I.tcp, I.assert, V36
- Relates to: ADR 0017

## Context

Config is authored via Terraform/OpenTofu across many projects, so a
provider is a required part of the system eventually. Before committing to a
design, Datadog's `datadog_synthetics_test` resource was studied as prior
art. Its central weakness is that it crams every check subtype into a single
resource gated by a `subtype` field, so field validity — which fields are
valid for an HTTP test versus a TCP test — can only be checked at apply
time, not at plan time; a Terraform plan can look fine and still fail on
apply. Its good idea, worth keeping, is a clean three-way structural split:
a request definition, repeatable assertion blocks, and an options list for
scheduling.

## Decision

Building the provider itself is deferred. What is being decided now is the
strategy: spec the CP's API contract so that a provider, in any language,
can target it later. A Terraform provider is idiomatically written in Go
using `terraform-plugin-framework`, and will live in its own separate
repository rather than in this codebase.

When it is built, the provider will use **separate resources per check
type** — `synthetics_http_check`, `synthetics_tcp_check`, and later
`synthetics_ws_check` — instead of one `subtype`-gated resource. This lets
Terraform statically enforce which fields are valid for which check type,
sidestepping Datadog's biggest weakness at the schema level rather than at
apply time. The hierarchy (ADR 0017) is likewise modeled as separate
resources with parent references: `synthetics_product`, `synthetics_customer`,
`synthetics_environment`.

The request/assertion/options three-way split from Datadog's design is kept,
but improved on: assertion targets are typed enums rather than Datadog's
single universal string `target` field, so an assertion resource can only
reference fields that are actually valid for its check type.

```hcl
resource "synthetics_http_check" "checkout_health" {
  name        = "checkout-health"
  environment = synthetics_environment.checkout_prod.id
  url         = "https://checkout.example.com/health"
  interval    = "30s"

  assertion {
    type   = "status_code"
    target = 200
  }

  assertion {
    type   = "response_time"
    target = "500ms"
  }

  options {
    timeout  = "5s"
    locations = ["us-east", "eu-west"]
  }
}
```

## Consequences

- Field validity is enforced statically by Terraform's schema, not deferred
  to apply time.
- Assertions are typed, closing off the class of bug where an assertion
  field is valid for the wrong check type.
- The provider is decoupled from the Rust core by the API contract alone,
  so it can be built and iterated on independently, as a downstream Go
  project.
- The API contract has to be stable and versioned once the provider starts
  depending on it, since it becomes a cross-repository interface.

## Alternatives considered

- **Single generic check resource, Datadog-style** — rejected; reproduces
  Datadog's weak apply-time-only validation.
- **A grouped monitor resource** — rejected; less direct and less aligned
  with how checks are actually authored per type.
- **Codegen the provider from an OpenAPI spec** — noted as possible in the
  future, but codegen'd providers are often clunky in practice, so it is not
  the starting approach.
