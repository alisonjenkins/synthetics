//! SIMD substring-scan tests (T9). The SIMD path must return exactly what the
//! scalar reference returns for every input, especially around the 32-byte
//! block boundary where hand-written SIMD usually breaks (V6).

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use synthetics::simd::{contains, contains_scalar};

#[test]
fn edge_cases_match_scalar() {
    let cases: &[(&[u8], &[u8])] = &[
        (b"", b""),
        (b"", b"x"),
        (b"abc", b""),
        (b"abc", b"abc"),
        (b"abc", b"abcd"),
        (b"abcabc", b"cab"),
        (b"the quick brown fox", b"fox"),
        (b"the quick brown fox", b"the"),
        (b"aaaaaa", b"aaa"),
        (b"needle at the very end.", b"end."),
    ];
    for (h, n) in cases {
        assert_eq!(contains(h, n), contains_scalar(h, n), "h={h:?} n={n:?}");
    }
}

#[test]
fn simd_equals_scalar_over_pseudorandom_sweep() {
    // Deterministic xorshift; small alphabet so matches actually occur.
    let mut state: u64 = 0x9E3779B97F4A7C15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    const ALPHABET: &[u8] = b"abcd";

    // Cover lengths straddling the 32-byte SIMD block boundary and beyond.
    for hay_len in 0..200usize {
        let haystack: Vec<u8> = (0..hay_len)
            .map(|_| ALPHABET[(next() % ALPHABET.len() as u64) as usize])
            .collect();
        for needle_len in 1..8usize {
            let needle: Vec<u8> = (0..needle_len)
                .map(|_| ALPHABET[(next() % ALPHABET.len() as u64) as usize])
                .collect();
            assert_eq!(
                contains(&haystack, &needle),
                contains_scalar(&haystack, &needle),
                "mismatch: hay_len={hay_len} needle={needle:?}"
            );
        }
    }
}

#[test]
fn finds_match_spanning_block_boundary() {
    // Needle placed so it starts just before a 32-byte boundary.
    let mut haystack = vec![b'x'; 64];
    haystack[30..34].copy_from_slice(b"MARK");
    assert!(contains(&haystack, b"MARK"));
    assert_eq!(
        contains(&haystack, b"MARK"),
        contains_scalar(&haystack, b"MARK")
    );
}
