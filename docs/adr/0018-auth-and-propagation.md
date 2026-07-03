# ADR 0018 — Auth (OIDC writers, mTLS workers) + near-real-time propagation

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: I.cp, I.auth, V28, V31

## Context

Writers of config — CI pipelines and Terraform runs — and readers of config
— the worker fleet — have very different trust models and lifecycles.
Writers are short-lived human or CI identities that need to be scoped to
what they own. Workers are long-lived fleet members that only ever need
read access. On top of that, config changes need to reach running workers
quickly: an operator applying a Terraform change should see it take effect
in the fleet in seconds, not minutes.

## Decision

The write path is authenticated via OIDC, using short-lived CI/SSO
identities rather than long-lived tokens. Authorization is scoped
per-product: an identity may only mutate the product subtree it owns (V31),
not the whole config tree.

The read path, used by workers, is authenticated via mutual TLS client
certificates and is read-only — a worker's certificate can never be used to
write config.

Propagation from a write to the fleet is required to happen in under five
seconds (V28). This is achieved via a change-signal emitted by the store
(Postgres `LISTEN`/`NOTIFY`, see ADR 0020) combined with a monotonic config
version stamp. Workers long-poll or subscribe to this signal and pull a
delta whenever the version advances, with a periodic reconcile as a fallback
in case a signal is missed.

```
Write flow (OIDC):
  CI/Terraform --OIDC token--> CP
                                 |-- validate token, authorize against product scope
                                 |-- write accepted, config version bumped
                                 |-- store emits change-signal

Read flow (mTLS):
  Worker --mTLS handshake--> CP
                                |-- validate client certificate
                                |-- read-only subscribe to change-signal
                                |-- pull delta on version change
```

## Consequences

- No long-lived write tokens exist to leak or rotate.
- Writers (human/CI) and readers (fleet workers) are cleanly separated by
  both mechanism and privilege — a compromised worker certificate cannot
  write config, and a compromised write token is scoped to one product.
- Operators get fast feedback: a Terraform apply is visible in the running
  fleet within seconds.
- This requires certificate issuance and an OIDC identity provider to be
  operated as real infrastructure, which is new operational machinery.
- Store backends without a native push signal degrade to polling latency
  (ADR 0020) rather than failing — correctness is preserved, only the speed
  of propagation changes.

## Alternatives considered

- **A single shared static token** — rejected; provides weak isolation
  between writers and no meaningful revocation story.
- **Per-project bearer tokens for Terraform** — rejected in favor of OIDC,
  which avoids long-lived secrets entirely.
- **Coarse polling only, no push signal** — rejected; this makes adding or
  removing a check take minutes rather than seconds.
