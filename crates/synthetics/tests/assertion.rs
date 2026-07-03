//! Assertion engine tests (T14). Each typed assertion is evaluated against a
//! check context, producing per-assertion pass/fail (V18) with a low-cardinality
//! id (V22). Cites I.assert, V18, V9, V22.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use synthetics::assert::{all_passed, evaluate, AssertContext};
use synthetics::config::Assertion;

fn ctx() -> AssertContext<'static> {
    AssertContext {
        status: Some(200),
        headers: &[],
        body: b"Please Sign in to continue",
        response_time: Duration::from_millis(50),
        cert_days_to_expiry: Some(57),
    }
}

#[test]
fn status_equals_and_range() {
    let a = evaluate(
        &[Assertion::Status {
            equals: Some(200),
            in_range: None,
        }],
        &ctx(),
    );
    assert_eq!(a[0].id, "status");
    assert!(a[0].passed);

    let a = evaluate(
        &[Assertion::Status {
            equals: Some(404),
            in_range: None,
        }],
        &ctx(),
    );
    assert!(!a[0].passed);

    let a = evaluate(
        &[Assertion::Status {
            equals: None,
            in_range: Some([200, 299]),
        }],
        &ctx(),
    );
    assert!(a[0].passed);

    let a = evaluate(
        &[Assertion::Status {
            equals: None,
            in_range: Some([300, 399]),
        }],
        &ctx(),
    );
    assert!(!a[0].passed);
}

#[test]
fn status_fails_when_no_response() {
    let mut c = ctx();
    c.status = None;
    let a = evaluate(
        &[Assertion::Status {
            equals: Some(200),
            in_range: None,
        }],
        &c,
    );
    assert!(!a[0].passed);
}

#[test]
fn header_equals_contains_and_case_insensitive_name() {
    let headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    let mut c = ctx();
    c.headers = &headers;

    // Name match is case-insensitive; value equals.
    let a = evaluate(
        &[Assertion::Header {
            name: "content-type".to_string(),
            equals: Some("application/json".to_string()),
            contains: None,
        }],
        &c,
    );
    assert_eq!(a[0].id, "header:content-type");
    assert!(a[0].passed);

    // Contains.
    let a = evaluate(
        &[Assertion::Header {
            name: "Content-Type".to_string(),
            equals: None,
            contains: Some("json".to_string()),
        }],
        &c,
    );
    assert!(a[0].passed);

    // Missing header fails.
    let a = evaluate(
        &[Assertion::Header {
            name: "x-absent".to_string(),
            equals: Some("y".to_string()),
            contains: None,
        }],
        &c,
    );
    assert!(!a[0].passed);
}

#[test]
fn latency_threshold() {
    let a = evaluate(
        &[Assertion::Latency {
            max: Duration::from_millis(100),
        }],
        &ctx(),
    );
    assert_eq!(a[0].id, "latency");
    assert!(a[0].passed);

    let a = evaluate(
        &[Assertion::Latency {
            max: Duration::from_millis(10),
        }],
        &ctx(),
    );
    assert!(!a[0].passed);
}

#[test]
fn cert_min_age() {
    let a = evaluate(&[Assertion::CertMinAge { days: 30 }], &ctx());
    assert_eq!(a[0].id, "cert_min_age");
    assert!(a[0].passed); // 57 >= 30

    let a = evaluate(&[Assertion::CertMinAge { days: 90 }], &ctx());
    assert!(!a[0].passed); // 57 < 90

    let mut c = ctx();
    c.cert_days_to_expiry = None;
    let a = evaluate(&[Assertion::CertMinAge { days: 1 }], &c);
    assert!(!a[0].passed); // no cert => fail
}

#[test]
fn body_contains_and_not_contains() {
    let a = evaluate(
        &[Assertion::BodyContains {
            value: "Sign in".to_string(),
        }],
        &ctx(),
    );
    assert_eq!(a[0].id, "body_contains");
    assert!(a[0].passed);

    let a = evaluate(
        &[Assertion::BodyContains {
            value: "Welcome to nginx".to_string(),
        }],
        &ctx(),
    );
    assert!(!a[0].passed);

    let a = evaluate(
        &[Assertion::BodyNotContains {
            value: "Welcome to nginx".to_string(),
        }],
        &ctx(),
    );
    assert_eq!(a[0].id, "body_not_contains");
    assert!(a[0].passed);

    let a = evaluate(
        &[Assertion::BodyNotContains {
            value: "Sign in".to_string(),
        }],
        &ctx(),
    );
    assert!(!a[0].passed);
}

#[test]
fn body_regex() {
    let a = evaluate(
        &[Assertion::BodyRegex {
            pattern: r"Sign\s+in".to_string(),
        }],
        &ctx(),
    );
    assert_eq!(a[0].id, "body_regex");
    assert!(a[0].passed);

    let a = evaluate(
        &[Assertion::BodyRegex {
            pattern: r"^nginx".to_string(),
        }],
        &ctx(),
    );
    assert!(!a[0].passed);
}

#[test]
fn body_sha256() {
    // sha256("Please Sign in to continue")
    use sha2::{Digest, Sha256};
    let hex: String = Sha256::digest(b"Please Sign in to continue")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let a = evaluate(&[Assertion::BodySha256 { hex: hex.clone() }], &ctx());
    assert_eq!(a[0].id, "body_sha256");
    assert!(a[0].passed);

    let wrong = "0".repeat(64);
    let a = evaluate(&[Assertion::BodySha256 { hex: wrong }], &ctx());
    assert!(!a[0].passed);
}

#[test]
fn all_passed_helper() {
    let outcomes = evaluate(
        &[
            Assertion::Status {
                equals: Some(200),
                in_range: None,
            },
            Assertion::BodyContains {
                value: "Sign in".to_string(),
            },
        ],
        &ctx(),
    );
    assert_eq!(outcomes.len(), 2);
    assert!(all_passed(&outcomes));

    let outcomes = evaluate(
        &[
            Assertion::Status {
                equals: Some(200),
                in_range: None,
            },
            Assertion::BodyContains {
                value: "absent".to_string(),
            },
        ],
        &ctx(),
    );
    assert!(!all_passed(&outcomes));
}
