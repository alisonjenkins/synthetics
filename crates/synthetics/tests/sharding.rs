//! Rendezvous-hash ownership tests (T19). Cites V26 (no coordinator), V27
//! (minimal reshuffle on membership change), V33 (region-aware).

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use synthetics::sharding::{owner, owns, Worker};

fn fleet(ids: &[&str], region: &str) -> Vec<Worker> {
    ids.iter().map(|id| Worker::new(*id, region)).collect()
}

#[test]
fn owner_is_deterministic_and_order_independent() {
    let a = fleet(&["a", "b", "c"], "eu");
    let mut b = a.clone();
    b.reverse();
    for i in 0..200 {
        let key = format!("check-{i}");
        assert_eq!(owner(&key, &a).unwrap().id, owner(&key, &b).unwrap().id);
    }
}

#[test]
fn exactly_one_worker_owns_each_key() {
    let workers = fleet(&["a", "b", "c"], "eu");
    let no_loc: Vec<String> = vec![];
    for i in 0..200 {
        let key = format!("check-{i}");
        let owners: Vec<_> = workers
            .iter()
            .filter(|w| owns(w, &key, &workers, &no_loc))
            .collect();
        assert_eq!(owners.len(), 1, "key {key} must have exactly one owner");
        assert_eq!(owners[0].id, owner(&key, &workers).unwrap().id);
    }
}

#[test]
fn ownership_is_reasonably_balanced() {
    let workers = fleet(&["w0", "w1", "w2"], "eu");
    let mut counts: HashMap<String, usize> = HashMap::new();
    for i in 0..3000 {
        let key = format!("check-{i}");
        *counts
            .entry(owner(&key, &workers).unwrap().id.clone())
            .or_default() += 1;
    }
    for w in &workers {
        let c = *counts.get(&w.id).unwrap_or(&0);
        // Perfect balance is 1000 each; allow wide slack, just prove no worker
        // is starved or hogging.
        assert!(
            c > 600 && c < 1400,
            "worker {} owns {c} (expected ~1000)",
            w.id
        );
    }
}

#[test]
fn removing_a_worker_only_moves_its_own_keys() {
    let full = fleet(&["a", "b", "c"], "eu");
    let reduced = fleet(&["a", "b"], "eu");

    for i in 0..1000 {
        let key = format!("check-{i}");
        let before = owner(&key, &full).unwrap().id.clone();
        let after = owner(&key, &reduced).unwrap().id.clone();
        if before != "c" {
            // Keys not owned by the departed worker keep their owner (V27).
            assert_eq!(before, after, "key {key} moved unnecessarily");
        } else {
            // c's keys move to a survivor.
            assert!(after == "a" || after == "b");
        }
    }
}

#[test]
fn adding_a_worker_only_pulls_keys_to_it() {
    let before_fleet = fleet(&["a", "b", "c"], "eu");
    let after_fleet = fleet(&["a", "b", "c", "d"], "eu");

    for i in 0..1000 {
        let key = format!("check-{i}");
        let before = owner(&key, &before_fleet).unwrap().id.clone();
        let after = owner(&key, &after_fleet).unwrap().id.clone();
        // A key either stays where it was or moves to the new worker — never
        // reshuffles among the existing workers (V27).
        assert!(
            after == before || after == "d",
            "key {key}: {before} -> {after}"
        );
    }
}

#[test]
fn region_aware_ownership() {
    let mut workers = fleet(&["eu1", "eu2"], "eu");
    workers.extend(fleet(&["us1", "us2"], "us"));
    let eu_only = vec!["eu".to_string()];

    for i in 0..200 {
        let key = format!("check-{i}");
        // A eu-scoped check is never owned by a us worker (V33).
        for w in &workers {
            let is_owner = owns(w, &key, &workers, &eu_only);
            if is_owner {
                assert_eq!(w.region, "eu", "us worker owned a eu-only check");
            }
        }
        // Exactly one eu worker owns it.
        let eu_owners = workers
            .iter()
            .filter(|w| owns(w, &key, &workers, &eu_only))
            .count();
        assert_eq!(eu_owners, 1);
    }
}

#[test]
fn no_candidate_region_means_no_owner() {
    let workers = fleet(&["us1", "us2"], "us");
    let eu_only = vec!["eu".to_string()];
    // No eu worker exists — a eu-only check is unowned here (would be covered by
    // a eu worker pool; V33).
    assert!(workers
        .iter()
        .all(|w| !owns(w, "check-1", &workers, &eu_only)));
}

#[test]
fn empty_locations_allows_any_region() {
    let mut workers = fleet(&["eu1"], "eu");
    workers.extend(fleet(&["us1"], "us"));
    let anywhere: Vec<String> = vec![];
    let owners = workers
        .iter()
        .filter(|w| owns(w, "check-1", &workers, &anywhere))
        .count();
    assert_eq!(owners, 1);
}
