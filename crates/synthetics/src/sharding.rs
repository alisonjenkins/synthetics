//! Rendezvous-hash check ownership (T19).
//!
//! Each worker independently decides which checks it owns — no coordinator (V26,
//! ADR 0016). For a check, the owner is the worker with the highest weight
//! `hash(check_id, worker_id)` (highest-random-weight / rendezvous hashing).
//! When the worker-set changes, only the minority of checks whose owner left (or
//! whose new joiner now wins) move; everything else stays put — which is what
//! keeps a reload from disturbing unaffected endpoints (V27). Ownership is
//! region-aware: only workers whose region matches a check's locations are
//! candidates (V33).
//!
//! The hash is a fixed, dependency-free FNV-1a + splitmix64 finalizer so every
//! worker in the fleet computes identical weights.

/// A fleet member for ownership purposes: a stable id and its region.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worker {
    /// Stable worker id (the rendezvous key input).
    pub id: String,
    /// Region the worker runs in.
    pub region: String,
}

impl Worker {
    /// Convenience constructor.
    pub fn new(id: impl Into<String>, region: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            region: region.into(),
        }
    }
}

/// The worker that owns `key` among `candidates`, or `None` if there are no
/// candidates. Ties (equal weight) break by worker id for determinism.
pub fn owner<'a>(key: &str, candidates: &'a [Worker]) -> Option<&'a Worker> {
    candidates.iter().max_by(|a, b| {
        weight(key, &a.id)
            .cmp(&weight(key, &b.id))
            .then_with(|| a.id.cmp(&b.id))
    })
}

/// Whether `me` owns `key`, considering only workers whose region is in
/// `locations` (an empty `locations` means any region — a check with no location
/// constraint runs anywhere). Returns `false` if `me` is not a candidate.
pub fn owns(me: &Worker, key: &str, workers: &[Worker], locations: &[String]) -> bool {
    let candidates: Vec<Worker> = workers
        .iter()
        .filter(|w| locations.is_empty() || locations.contains(&w.region))
        .cloned()
        .collect();
    match owner(key, &candidates) {
        Some(o) => o.id == me.id,
        None => false,
    }
}

/// Rendezvous weight of `key` for `worker_id`. Fixed hash so all workers agree.
fn weight(key: &str, worker_id: &str) -> u64 {
    // FNV-1a over worker_id, a separator, then key.
    let mut h: u64 = 0xcbf29ce4_84222325;
    for byte in worker_id
        .bytes()
        .chain(std::iter::once(0u8))
        .chain(key.bytes())
    {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    // splitmix64 finalizer for better avalanche.
    let mut z = h.wrapping_add(0x9e3779b9_7f4a7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58476d_1ce4e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d049bb_133111eb);
    z ^ (z >> 31)
}
