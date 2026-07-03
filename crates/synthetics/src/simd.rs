//! SIMD-accelerated byte scanning (T9).
//!
//! [`contains`] is the hot substring search behind `body_contains` /
//! `body_not_contains` assertions (V22). It dispatches at runtime to an AVX2
//! implementation where available, falling back to [`contains_scalar`] — the
//! scalar reference and source of truth — everywhere else (ADR 0003). The two
//! always return identical results (V6), asserted by the equality property test
//! and benchmarked against each other (V16); if AVX2 does not win, the SIMD path
//! should be dropped rather than kept.

/// Whether `haystack` contains `needle`. Uses AVX2 on x86-64 when available,
/// otherwise the scalar fallback. Result is identical to [`contains_scalar`].
pub fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            // SAFETY: only called after confirming AVX2 is present at runtime.
            return unsafe { contains_avx2(haystack, needle) };
        }
    }
    contains_scalar(haystack, needle)
}

/// Scalar substring search — the correctness reference (V6). Always available.
pub fn contains_scalar(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if needle.len() > haystack.len() {
        return false;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// AVX2 substring search: vectorize the search for the needle's first byte over
/// 32-byte windows, then verify full matches at each candidate (memchr-style).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn contains_avx2(haystack: &[u8], needle: &[u8]) -> bool {
    use std::arch::x86_64::{
        __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8, _mm256_set1_epi8,
    };

    let n = needle.len();
    if n == 0 {
        return true;
    }
    if n > haystack.len() {
        return false;
    }

    // Last index at which the needle could start.
    let last_start = haystack.len() - n;
    let first = _mm256_set1_epi8(needle[0] as i8);

    let mut i = 0usize;
    // Vectorize while a full 32-byte window of candidate starts remains within
    // bounds (loading 32 bytes at i needs i + 32 <= haystack.len()).
    while i + 32 <= last_start + 1 {
        let block = _mm256_loadu_si256(haystack.as_ptr().add(i) as *const __m256i);
        let eq = _mm256_cmpeq_epi8(block, first);
        let mut mask = _mm256_movemask_epi8(eq) as u32;
        while mask != 0 {
            let pos = i + mask.trailing_zeros() as usize;
            if pos <= last_start && &haystack[pos..pos + n] == needle {
                return true;
            }
            mask &= mask - 1; // clear lowest set bit
        }
        i += 32;
    }

    // Scalar tail for the remaining candidate positions.
    while i <= last_start {
        if haystack[i] == needle[0] && &haystack[i..i + n] == needle {
            return true;
        }
        i += 1;
    }
    false
}
