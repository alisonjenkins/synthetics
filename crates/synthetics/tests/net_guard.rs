//! SSRF guard tests (security audit #5): resolved target addresses are classified
//! and gated so a writer cannot aim the fleet at loopback / link-local / private
//! ranges (e.g. the cloud metadata endpoint) unless the endpoint opts in AND the
//! operator has pre-approved that range.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::IpAddr;

use synthetics::net_guard::{is_internal_ip, GuardError, NetPolicy};

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

#[test]
fn classifies_internal_ranges() {
    // Loopback, private, link-local (incl. the cloud metadata address),
    // unspecified, and IPv6 ULA / loopback all count as internal.
    for s in [
        "127.0.0.1",
        "10.0.0.5",
        "192.168.1.1",
        "172.16.0.1",
        "169.254.169.254", // cloud metadata
        "0.0.0.0",
        "::1",
        "fc00::1",
        "fe80::1",
    ] {
        assert!(is_internal_ip(ip(s)), "{s} should be internal");
    }
}

#[test]
fn classifies_public_ranges() {
    for s in ["1.1.1.1", "8.8.8.8", "93.184.216.34", "2606:4700:4700::1111"] {
        assert!(!is_internal_ip(ip(s)), "{s} should be public");
    }
}

#[test]
fn public_address_is_always_allowed() {
    let policy = NetPolicy::new::<&str>(&[]).unwrap();
    // allow_internal is irrelevant for a public address.
    assert!(policy.guard(ip("8.8.8.8"), false).is_ok());
    assert!(policy.guard(ip("8.8.8.8"), true).is_ok());
}

#[test]
fn internal_address_is_denied_without_the_flag() {
    let policy = NetPolicy::new(&["10.0.0.0/8"]).unwrap();
    // Even though 10.0.0.0/8 is operator-approved, the endpoint did not opt in.
    let err = policy.guard(ip("10.0.0.5"), false).unwrap_err();
    assert!(matches!(err, GuardError::InternalNotAllowed { .. }));
}

#[test]
fn internal_address_with_flag_but_outside_allowlist_is_denied() {
    // Flag set, but the operator only approved a different range: fail closed.
    let policy = NetPolicy::new(&["10.0.0.0/8"]).unwrap();
    let err = policy.guard(ip("169.254.169.254"), true).unwrap_err();
    assert!(matches!(err, GuardError::NotInAllowlist { .. }));
}

#[test]
fn internal_address_with_flag_inside_allowlist_is_allowed() {
    let policy = NetPolicy::new(&["10.0.0.0/8", "192.168.0.0/16"]).unwrap();
    assert!(policy.guard(ip("10.1.2.3"), true).is_ok());
    assert!(policy.guard(ip("192.168.5.5"), true).is_ok());
}

#[test]
fn empty_allowlist_fails_closed_for_internal_even_with_flag() {
    let policy = NetPolicy::new::<&str>(&[]).unwrap();
    assert!(policy.guard(ip("10.0.0.5"), true).is_err());
}

#[test]
fn rejects_malformed_cidr() {
    assert!(NetPolicy::new(&["not-a-cidr"]).is_err());
}
