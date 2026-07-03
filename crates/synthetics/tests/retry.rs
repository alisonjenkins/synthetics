//! Retry policy (#6, V13): a failing check is retried up to `retries` times with
//! `retry_backoff` between attempts before it is reported down, so a single
//! transient blip does not raise a false alarm; a succeeding check is never
//! retried.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use synthetics::check::run_endpoint;
use synthetics::config::{EndpointConfig, Target, TcpTarget};
use synthetics::metric::Outcome;
use synthetics::net_guard::NetPolicy;
use tokio::net::TcpListener;

fn tcp_endpoint(addr: SocketAddr, retries: u32, retry_backoff: Duration) -> EndpointConfig {
    EndpointConfig {
        id: "t".into(),
        interval: Duration::from_secs(30),
        timeout: Duration::from_secs(2),
        tags: vec![],
        target: Target::Tcp(TcpTarget {
            host: addr.ip().to_string(),
            port: addr.port(),
        }),
        assertions: vec![],
        allow_internal: false,
        retries,
        retry_backoff,
    }
}

#[tokio::test]
async fn retries_a_failing_check_with_backoff() {
    // Bind then drop to get an address that refuses connections every time.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let ep = tcp_endpoint(addr, 2, Duration::from_millis(75));
    let start = Instant::now();
    let report = run_endpoint(&ep, &NetPolicy::unrestricted()).await;

    assert_eq!(report.outcome, Outcome::Failure, "refused port stays failed");
    // Two retries at 75ms each means the whole thing took at least ~150ms — proof
    // the retries (and their backoff) actually happened.
    assert!(
        start.elapsed() >= Duration::from_millis(150),
        "expected two backoffs to elapse, took {:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn does_not_retry_a_successful_check() {
    // A listener that accepts (so the TCP connect succeeds first try).
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let _ = listener.accept().await;
        }
    });

    // A long backoff would dominate the runtime if a retry ever fired.
    let ep = tcp_endpoint(addr, 5, Duration::from_secs(2));
    let start = Instant::now();
    let report = run_endpoint(&ep, &NetPolicy::unrestricted()).await;

    assert_eq!(report.outcome, Outcome::Success);
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "a successful check must not wait on any backoff, took {:?}",
        start.elapsed()
    );
}
