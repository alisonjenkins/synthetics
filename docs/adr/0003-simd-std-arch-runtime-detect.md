# ADR 0003 — SIMD via `std::arch` intrinsics with runtime detection

- Status: Accepted
- Date: 2026-07-03
- Deciders: project owner
- Spec refs: V6, V16, V9

## Context

The codebase should use SIMD where there is a real win (V6). Hot byte-scanning
work exists on the check path: locating header boundaries (CRLF) in a response,
and matching body-content assertions. These run once per check per endpoint, so
at high endpoint counts they are hot.

Constraints: the toolchain is **stable** Rust (`std::simd` is nightly-only),
the binary must run on hosts that may lack AVX2, and every SIMD path must
produce a result identical to its scalar fallback (V6). SIMD must be benchmarked
against scalar to prove the win, or be dropped (V16).

## Decision

Implement SIMD with `core::arch::x86_64` intrinsics, gated at runtime:

1. A **scalar** implementation is always present and is the source of truth for
   correctness.
2. An **AVX2** implementation is written in an `#[target_feature(enable = "avx2")]`
   function.
3. At the call site, `std::is_x86_feature_detected!("avx2")` selects the AVX2
   path once (cached), else the scalar path. On non-x86 or AVX2-less hosts, the
   scalar path runs — the binary still works everywhere.
4. A property test asserts SIMD output == scalar output over random inputs and
   all length remainders (the tail past the last full vector), which is where
   hand-written SIMD usually breaks (V6).
5. A criterion benchmark compares both. If AVX2 does not beat scalar on the
   target, the SIMD path is removed rather than kept for show (V16).

## Consequences

- Portable binary: one build runs on AVX2 and non-AVX2 hosts.
- Correctness is pinned by the scalar reference plus the equality property
  test, so the `unsafe` intrinsics cannot silently diverge.
- The `unsafe` blocks are confined to the `simd` module and justified by the
  benchmark; the rest of the codebase stays safe.
- AVX-512 is available on the dev host but not assumed; AVX2 is the widest
  baseline that is near-universal on modern x86-64 servers. AVX-512 can be
  added later as a third tier behind the same detection pattern.

## Alternatives considered

- **`std::simd` (portable SIMD)** — cleaner and portable across ISAs, but
  nightly-only; rejected to stay on stable.
- **`memchr` crate** — already SIMD-optimized and excellent for single-byte
  search; will be used where it fits, but does not cover multi-byte /
  structural scans, and the spec calls for a demonstrated in-tree SIMD path.
- **Autovectorization only** — relying on the optimizer is unpredictable and
  cannot be asserted or benchmarked as "SIMD is used".
