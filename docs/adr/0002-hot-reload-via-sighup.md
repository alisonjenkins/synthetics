# ADR 0002 — Hot reload via SIGHUP + config diff

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: I.config, I.ctl, V3, V4, V11

## Context

Operators must add or remove scraped endpoints without restarting the daemon
(V4), and no check that is already scheduled may be dropped or skipped across
the reload (V3). A bad new config must be rejected while the old config keeps
running (V11).

Two common triggers exist: a filesystem watch (`notify` crate) that reloads on
file change, or an explicit signal (SIGHUP) that reloads on demand.

## Decision

Reload on **SIGHUP**. On signal:

1. Read + parse + validate the config file. On any error, log it (structured)
   and keep the running config unchanged — the new config is never partially
   applied (V11).
2. Diff old vs new endpoint sets by a stable endpoint key:
   - **added** endpoints get scheduled immediately;
   - **removed** endpoints are cancelled after their in-flight check (if any)
     finishes;
   - **unchanged** endpoints keep their existing schedule and next-fire time —
     they are *not* reset, so no tick is skipped or doubled (V3).
3. Swap the active config behind an `ArcSwap`/`RwLock` so readers never block
   and never see a half-updated set (V4).

The config is held in a single shared structure; the scheduler reads the
current set each tick, so a swap takes effect on the next tick without missing
the current one.

## Consequences

- Deterministic, operator-controlled reloads; easy to trigger from deploy
  tooling (`kill -HUP`) and from tests (send the signal, assert schedule
  continuity).
- No inotify/kqueue dependency now, keeping the dependency surface and memory
  footprint minimal (V15).
- Requires the diff to be keyed stably so unchanged endpoints are preserved;
  this key is defined once in the config model.
- File-watch auto-reload can be added later as an *additional* trigger that
  feeds the same reload path — the mechanism (validate → diff → swap) is
  independent of what triggers it. Left as a future extension; I.ctl already
  reads "SIGHUP and/or config-file watch".

## Alternatives considered

- **`notify` file-watch** — zero-touch reloads, but adds a dependency, fires on
  partial writes (needs debounce), and is harder to drive deterministically in
  tests. Deferred, not rejected.
- **Control socket / HTTP admin endpoint** — most flexible, but a larger
  surface (auth, protocol) than needed for "reload the file".
