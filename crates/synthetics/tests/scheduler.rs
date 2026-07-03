//! Scheduler tests (T7): endpoints fire on their interval and shutdown stops
//! the scheduler. Cites V5, V8.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use synthetics::config::{EndpointConfig, Target, TcpTarget};
use synthetics::metric::{MockSink, Outcome};
use synthetics::scheduler::Scheduler;
use tokio::net::TcpListener;
use tokio::sync::watch;

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
    })
}

#[tokio::test]
async fn endpoint_fires_repeatedly_on_its_interval() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let sink = Arc::new(MockSink::new());
    let sched = Scheduler::new(Arc::clone(&sink), 16);
    let (tx, rx) = watch::channel(false);

    let ep = tcp_endpoint("healthy", addr, Duration::from_millis(40));
    let handle = tokio::spawn(sched.run(vec![ep], rx));

    tokio::time::sleep(Duration::from_millis(250)).await;
    tx.send(true).unwrap();
    handle.await.unwrap();

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
    let sched = Scheduler::new(Arc::clone(&sink), 16);
    let (tx, rx) = watch::channel(false);

    let ep = tcp_endpoint("healthy", addr, Duration::from_millis(50));
    let handle = tokio::spawn(sched.run(vec![ep], rx));

    tx.send(true).unwrap();
    // run() must return after shutdown rather than hang.
    tokio::time::timeout(Duration::from_secs(2), handle)
        .await
        .expect("scheduler should stop promptly after shutdown")
        .unwrap();
}
