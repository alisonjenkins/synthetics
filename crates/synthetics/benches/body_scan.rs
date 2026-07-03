//! Benchmark the SIMD body scan against the scalar reference (T13, V16).
//!
//! Proves whether the AVX2 substring search beats scalar on a realistic body.
//! If it does not win here, the SIMD path should be dropped (V6/ADR 0003).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::hint::black_box;

use criterion::{criterion_group, criterion_main, Criterion, Throughput};
use synthetics::simd::{contains, contains_scalar};

fn body_scan(c: &mut Criterion) {
    // 64 KiB body with the needle only near the very end (worst case: the whole
    // buffer is scanned before a match is found).
    let mut haystack = vec![b'x'; 64 * 1024];
    let needle = b"Welcome to the login page";
    let start = haystack.len() - needle.len();
    haystack[start..].copy_from_slice(needle);

    let mut group = c.benchmark_group("body_contains_64kib");
    group.throughput(Throughput::Bytes(haystack.len() as u64));
    group.bench_function("scalar", |b| {
        b.iter(|| contains_scalar(black_box(&haystack), black_box(needle)))
    });
    group.bench_function("simd", |b| {
        b.iter(|| contains(black_box(&haystack), black_box(needle)))
    });
    group.finish();
}

criterion_group!(benches, body_scan);
criterion_main!(benches);
