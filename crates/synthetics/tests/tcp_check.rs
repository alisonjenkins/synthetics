//! TCP check executor tests (T6). Real connects against local listeners — no
//! mocks (V7). Cites I.tcp, V1, V10, V17, V19.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use synthetics::check::tcp::{run_tcp_check, TcpCheckError};
use synthetics::config::TcpTarget;
use tokio::net::TcpListener;

#[tokio::test]
async fn connect_success_records_dns_and_connect_timings() {
    // A bound listener accepts connections at the OS level even without accept().
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let target = TcpTarget {
        host: addr.ip().to_string(),
        port: addr.port(),
    };
    let out = run_tcp_check(
        &target,
        Duration::from_secs(5),
        false,
        &synthetics::net_guard::NetPolicy::unrestricted(),
    )
    .await
    .unwrap();

    assert_eq!(out.peer, addr);
    assert!(out.timings.dns.is_some(), "dns timed");
    assert!(out.timings.connect.is_some(), "connect timed");
    assert!(out.timings.total > Duration::ZERO);
}

#[tokio::test]
async fn connection_refused_is_a_connect_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener); // nobody listening now

    let target = TcpTarget {
        host: addr.ip().to_string(),
        port: addr.port(),
    };
    let err = run_tcp_check(
        &target,
        Duration::from_secs(2),
        false,
        &synthetics::net_guard::NetPolicy::unrestricted(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, TcpCheckError::Connect { .. }), "got {err:?}");
}

#[tokio::test]
async fn unresolvable_host_is_a_dns_error() {
    let target = TcpTarget {
        host: "nonexistent.invalid".to_string(),
        port: 80,
    };
    let err = run_tcp_check(
        &target,
        Duration::from_secs(3),
        false,
        &synthetics::net_guard::NetPolicy::unrestricted(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, TcpCheckError::Dns { .. }), "got {err:?}");
}

#[tokio::test]
async fn zero_budget_times_out_before_connecting() {
    // A deadline already in the past fails deterministically as a timeout (V19).
    let target = TcpTarget {
        host: "127.0.0.1".to_string(),
        port: 9,
    };
    let err = run_tcp_check(
        &target,
        Duration::ZERO,
        false,
        &synthetics::net_guard::NetPolicy::unrestricted(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, TcpCheckError::Timeout { .. }), "got {err:?}");
}
