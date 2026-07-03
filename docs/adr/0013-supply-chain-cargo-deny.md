# ADR 0013 — Supply-chain gating with cargo-deny

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: V23, V15

## Context

The daemon runs unattended and links a tree of third-party crates. Two risks
must be gated in CI, not left to review:

1. **Toxic licenses** — a transitive dependency under a copyleft license (GPL,
   AGPL, LGPL, SSPL) could impose obligations incompatible with how this
   software is distributed.
2. **Known vulnerabilities / bad provenance** — a dependency with a RUSTSEC
   advisory, a yanked version, or pulled from an unexpected source.

## Decision

Adopt `cargo-deny` with an **allow-list** license policy (`deny.toml`), run as a
CI gate (`cargo deny check`). Policy:

- **Licenses:** only permissive SPDX licenses are allowed (MIT, Apache-2.0,
  BSD-2/3-Clause, ISC, Zlib, Unicode-3.0/DFS-2016, MPL-2.0, CC0-1.0). Anything
  not on the list — including all strong copyleft — fails the build by
  omission. Adding a license is a deliberate, reviewed edit to `deny.toml`.
- **Advisories:** fail on any RUSTSEC advisory; deny yanked crates.
- **Sources:** only crates.io; unknown registries and git sources are denied
  unless explicitly allow-listed.
- **Bans:** wildcard version requirements are denied; multiple major versions
  of one crate warn (transitive duplicates are not always fixable, so not a
  hard fail).

## Consequences

- License and vulnerability posture is enforced continuously, so a risky
  dependency is caught at PR time, not at audit time.
- Adding a dependency under a new license is intentionally a small friction
  (edit `deny.toml`) — the point is that it is a conscious decision.
- `cargo-deny` is a CI tool only; it adds no runtime dependency and does not
  affect the binary footprint (V15).
- MPL-2.0 is allowed (file-level copyleft, compatible with distribution here);
  if that stance changes it is one line in `deny.toml`.

## Alternatives considered

- **`cargo-audit` alone** — covers advisories but not licenses or sources;
  cargo-deny is a superset.
- **Manual license review** — does not scale across a churning transitive
  tree and is easily forgotten; a CI gate is reliable.
- **Deny-list of bad licenses** — brittle; a new copyleft variant would slip
  through. Allow-list fails closed, which is the safer default.
