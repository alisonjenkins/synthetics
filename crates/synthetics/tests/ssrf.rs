//! Connect-time SSRF enforcement (security audit #5): a check must refuse to
//! connect to an internal/private resolved address unless the endpoint opts in
//! AND the operator allowlist covers it. Enforced after DNS so it also defends
//! against rebinding.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use synthetics::check::http::{run_http_check, HttpCheckError};
use synthetics::config::HttpTarget;
use synthetics::net_guard::NetPolicy;

fn http_target(url: &str) -> HttpTarget {
    HttpTarget {
        url: url.parse().unwrap(),
        method: "GET".to_string(),
        headers: vec![],
        body: None,
        allow_insecure: false,
    }
}

#[tokio::test]
async fn blocks_internal_target_without_optin() {
    // Loopback is internal; the endpoint did not opt in.
    let target = http_target("http://127.0.0.1:9/");
    let policy = NetPolicy::new::<&str>(&[]).unwrap();
    let err = run_http_check(&target, Duration::from_secs(2), 0, false, &policy)
        .await
        .unwrap_err();
    assert!(
        matches!(err, HttpCheckError::Blocked { .. }),
        "expected Blocked, got {err:?}"
    );
}

#[tokio::test]
async fn blocks_internal_target_when_optin_but_not_allowlisted() {
    let target = http_target("http://127.0.0.1:9/");
    // Operator approved a different range only.
    let policy = NetPolicy::new(&["10.0.0.0/8"]).unwrap();
    let err = run_http_check(&target, Duration::from_secs(2), 0, true, &policy)
        .await
        .unwrap_err();
    assert!(
        matches!(err, HttpCheckError::Blocked { .. }),
        "expected Blocked, got {err:?}"
    );
}

#[tokio::test]
async fn allows_internal_target_when_optin_and_allowlisted() {
    // Opted in and 127.0.0.0/8 approved: the guard passes, so we reach connect
    // (which fails against the closed port — a Connect error, not Blocked).
    let target = http_target("http://127.0.0.1:9/");
    let policy = NetPolicy::new(&["127.0.0.0/8"]).unwrap();
    let err = run_http_check(&target, Duration::from_secs(2), 0, true, &policy)
        .await
        .unwrap_err();
    assert!(
        !matches!(err, HttpCheckError::Blocked { .. }),
        "guard should have passed; got {err:?}"
    );
}
