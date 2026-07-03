//! Version-keyed cache of the resolved `/checks` response (performance #7).
//!
//! Every worker polls `/checks` on its interval, and each poll otherwise means
//! listing all nodes + endpoints and resolving the whole hierarchy — O(fleet ×
//! endpoints) DB and CPU per cycle. The resolved set only changes when the config
//! version bumps, so we cache the serialized response keyed by that version and
//! serve it verbatim until the next write. A version mismatch is a miss.

use std::sync::Mutex;

use axum::body::Bytes;

/// A single-slot cache of the serialized checks response for one config version.
/// The payload is [`Bytes`] so a cache hit is a refcount bump, not a copy.
#[derive(Default)]
pub struct ChecksCache {
    inner: Mutex<Option<(u64, Bytes)>>,
}

impl ChecksCache {
    /// The cached bytes for `version`, or `None` if the cache holds a different
    /// (or no) version.
    pub fn get(&self, version: u64) -> Option<Bytes> {
        let guard = self.lock();
        guard
            .as_ref()
            .filter(|(v, _)| *v == version)
            .map(|(_, bytes)| bytes.clone())
    }

    /// Replace the cache with `bytes` for `version` (evicting any older entry).
    pub fn put(&self, version: u64, bytes: Bytes) {
        *self.lock() = Some((version, bytes));
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<(u64, Bytes)>> {
        match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        }
    }
}

#[cfg(test)]
mod tests {
    // Panicking is the failure signal in tests (ADR 0008).
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn caches_and_invalidates_by_version() {
        let cache = ChecksCache::default();
        assert!(cache.get(1).is_none(), "empty cache misses");

        cache.put(1, Bytes::from_static(&[1, 2, 3]));
        assert_eq!(cache.get(1).unwrap().as_ref(), &[1, 2, 3]);

        // A different version must miss — that is what forces a re-resolve after
        // a config write.
        assert!(cache.get(2).is_none());

        // A newer version evicts the old entry.
        cache.put(2, Bytes::from_static(&[4]));
        assert!(cache.get(1).is_none(), "stale version evicted");
        assert_eq!(cache.get(2).unwrap().as_ref(), &[4]);
    }
}
