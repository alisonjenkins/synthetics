//! Scheduler tests (T7 timing/shutdown; T18 reconcile). Cites V5, V8, V3, V4.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use synthetics::config::{EndpointConfig, Target, TcpTarget};
use synthetics::metric::{MockSink, Outcome};
use synthetics::scheduler::Scheduler;
use tokio::net::TcpListener;

fn tcp_endpoint(id: &str, addr: std::net::SocketAddr, interval: Duration) -> Arc<EndpointConfig> {
    Arc::new(EndpointConfig {
        id: id.to_string(),
        interval,
        timeout: Duration::from_secs(2),
        tags: vec![],
        target: Target::Tcp(TcpTarget {
            host: addr.ip().to_string(),
            port: addr.port(),
        }),
        assertions: vec![],
        allow_internal: false,
    })
}

#[tokio::test]
async fn endpoint_fires_repeatedly_on_its_interval() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let sink = Arc::new(MockSink::new());
    let mut sched = Scheduler::new(Arc::clone(&sink), 16);
    sched.reconcile(vec![tcp_endpoint(
        "healthy",
        addr,
        Duration::from_millis(40),
    )]);

    tokio::time::sleep(Duration::from_millis(250)).await;
    sched.shutdown().await;

    let reports = sink.reports();
    // ~6 fires expected over 250ms at 40ms; assert a safe lower bound.
    assert!(
        reports.len() >= 3,
        "expected several fires, got {}",
        reports.len()
    );
    assert!(reports.iter().all(|r| r.endpoint_id == "healthy"));
    assert!(reports.iter().all(|r| r.outcome == Outcome::Success));
}

#[tokio::test]
async fn shutdown_stops_the_scheduler_promptly() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let sink = Arc::new(MockSink::new());
    let mut sched = Scheduler::new(Arc::clone(&sink), 16);
    sched.reconcile(vec![tcp_endpoint(
        "healthy",
        addr,
        Duration::from_millis(50),
    )]);

    // shutdown() must return promptly rather than hang.
    tokio::time::timeout(Duration::from_secs(2), sched.shutdown())
        .await
        .expect("scheduler should stop promptly");
}

#[tokio::test]
async fn reconcile_adds_removes_and_keeps_endpoints() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let interval = Duration::from_secs(60); // long: we only inspect membership

    let sink = Arc::new(MockSink::new());
    let mut sched = Scheduler::new(Arc::clone(&sink), 16);

    sched.reconcile(vec![
        tcp_endpoint("a", addr, interval),
        tcp_endpoint("b", addr, interval),
    ]);
    assert_eq!(sched.running_ids(), vec!["a".to_string(), "b".to_string()]);

    // Drop b, add c, keep a.
    sched.reconcile(vec![
        tcp_endpoint("a", addr, interval),
        tcp_endpoint("c", addr, interval),
    ]);
    assert_eq!(sched.running_ids(), vec!["a".to_string(), "c".to_string()]);

    sched.reconcile(vec![]);
    assert!(sched.running_ids().is_empty());

    sched.shutdown().await;
}
