//! Fault-isolation harness (T15): a slow/hung/dead target must never delay or
//! block a healthy target's schedule (V20), and a panicking check must not take
//! down the fleet (V21, covered by scheduler unit tests). Cites V19, V20, V21.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use synthetics::config::{EndpointConfig, HttpTarget, Target, TcpTarget};
use synthetics::metric::{MockSink, Outcome};
use synthetics::scheduler::Scheduler;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::sync::watch;
use url::Url;

/// A TCP server that accepts and reads but never responds — hangs every check.
async fn hung_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                std::future::pending::<()>().await; // never respond
            });
        }
    });
    addr
}

#[tokio::test]
async fn healthy_endpoint_fires_on_schedule_despite_hung_neighbor() {
    // Healthy TCP target (a bound listener accepts instantly).
    let healthy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let healthy_addr = healthy_listener.local_addr().unwrap();

    // Hung HTTP target: connects, never responds; each check runs until its
    // own timeout.
    let hung_addr = hung_server().await;

    let healthy = Arc::new(EndpointConfig {
        id: "healthy".to_string(),
        interval: Duration::from_millis(40),
        timeout: Duration::from_secs(2),
        tags: vec![],
        target: Target::Tcp(TcpTarget {
            host: healthy_addr.ip().to_string(),
            port: healthy_addr.port(),
        }),
        assertions: vec![],
    });
    let hung = Arc::new(EndpointConfig {
        id: "hung".to_string(),
        interval: Duration::from_millis(40),
        // Long timeout so the check stays in-flight for the whole test window.
        timeout: Duration::from_secs(30),
        tags: vec![],
        target: Target::Http(HttpTarget {
            url: Url::parse(&format!("http://{hung_addr}/")).unwrap(),
            method: "GET".to_string(),
            headers: vec![],
            body: None,
            allow_insecure: false,
        }),
        assertions: vec![],
    });

    let sink = Arc::new(MockSink::new());
    // Concurrency ample so the hung check cannot starve the healthy one.
    let sched = Scheduler::new(Arc::clone(&sink), 16);
    let (tx, rx) = watch::channel(false);

    let handle = tokio::spawn(sched.run(vec![healthy, hung], rx));
    tokio::time::sleep(Duration::from_millis(300)).await;
    tx.send(true).unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(2), handle).await;

    let reports = sink.reports();
    let healthy_fires = reports
        .iter()
        .filter(|r| r.endpoint_id == "healthy")
        .count();

    // The healthy endpoint kept firing on its ~40ms cadence despite the hung
    // neighbor holding a permit the entire time (V20).
    assert!(
        healthy_fires >= 3,
        "healthy endpoint should keep firing; got {healthy_fires} fires"
    );
    assert!(reports
        .iter()
        .filter(|r| r.endpoint_id == "healthy")
        .all(|r| r.outcome == Outcome::Success));
}
