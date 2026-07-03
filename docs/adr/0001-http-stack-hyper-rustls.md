# ADR 0001 — Low-level HTTP stack (hyper + tokio-rustls) for phase timings

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: I.http, V10, V17

## Context

The daemon must record every signal a Datadog synthetic records (V17): DNS
resolution time, TCP connect time, TLS handshake time, time-to-first-byte
(TTFB), download time, total time, response size, status code, redirect
count, and TLS certificate days-to-expiry.

These are *phase* timings. A high-level client (`reqwest`, `ureq`) performs
DNS + connect + TLS + request as one opaque `send()` call and does not expose
the boundaries between phases. Extracting per-phase timings from such a client
means reaching into internals it does not promise to keep stable, or not
getting them at all.

## Decision

Build the HTTP check on the low-level stack and drive each phase explicitly:

1. **DNS** — `tokio::net::lookup_host`, timed.
2. **TCP connect** — `tokio::net::TcpStream::connect` to a resolved address,
   timed.
3. **TLS** — `tokio-rustls` handshake over the TCP stream, timed. The
   negotiated peer certificate is parsed (`x509-parser`) for days-to-expiry.
4. **Request/response** — `hyper` client connection over the (TLS or plain)
   stream. TTFB measured from request send to first response byte; download
   time from first byte to body end; response size from the received bytes.
5. **Redirects** — followed manually so each hop is itself measured and
   counted.

## Consequences

- Full V17 parity: every phase boundary is under our control, so every timing
  is real, not estimated.
- More code than calling `reqwest::get`. We own connection setup, redirect
  logic, and timeouts. Mitigated by keeping it in one `check::http` module
  behind a small function surface.
- `rustls` (not OpenSSL) keeps the build pure-Rust, cross-compilable, and
  avoids a system TLS dependency — aligns with the minimal-footprint
  constraint (V15).
- WebSocket (I.ws, future) reuses the same connect+TLS path, then upgrades —
  the low-level stack makes the upgrade handshake straightforward.

## Alternatives considered

- **reqwest** — ergonomic, but phase timings are not exposed; would miss V17.
- **isahc/curl** — libcurl exposes some timings via `CURLINFO_*`, but pulls in
  a C dependency, conflicting with V15 and pure-Rust cross-compilation.
- **Raw `std::net` blocking sockets** — simplest, but blocking-per-endpoint
  does not scale to many endpoints on one daemon (V8).
