//! Endpoint orchestrator tests (T17): run an endpoint end-to-end and produce a
//! CheckReport — run executor, apply assertions, map outcome/failure/detail.
//! Cites V9, V18, V35.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::time::Duration;

use synthetics::check::run_endpoint;
use synthetics::config::{Assertion, EndpointConfig, HttpTarget, Target, TcpTarget};
use synthetics::metric::{CheckDetail, FailureKind, Outcome};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

async fn serve_fixed(response: &'static str) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((mut sock, _)) = listener.accept().await {
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.flush().await;
            });
        }
    });
    addr
}

fn http_endpoint(id: &str, url: &str, assertions: Vec<Assertion>) -> EndpointConfig {
    EndpointConfig {
        id: id.to_string(),
        interval: Duration::from_secs(30),
        timeout: Duration::from_secs(5),
        tags: vec!["env:test".to_string()],
        target: Target::Http(HttpTarget {
            url: Url::parse(url).unwrap(),
            method: "GET".to_string(),
            headers: vec![],
            body: None,
            allow_insecure: false,
        }),
        assertions,
        allow_internal: false,
    }
}

#[tokio::test]
async fn http_success_with_passing_assertions() {
    let addr = serve_fixed("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello").await;
    let ep = http_endpoint(
        "home",
        &format!("http://{addr}/"),
        vec![
            Assertion::Status {
                equals: Some(200),
                in_range: None,
            },
            Assertion::BodyContains {
                value: "hello".to_string(),
            },
        ],
    );

    let report = run_endpoint(&ep).await;

    assert_eq!(report.endpoint_id, "home");
    assert_eq!(report.tags, vec!["env:test".to_string()]);
    assert_eq!(report.outcome, Outcome::Success);
    assert!(report.failure.is_none());
    assert_eq!(report.assertions.len(), 2);
    assert!(report.assertions.iter().all(|a| a.passed));
    assert!(report.duration > Duration::ZERO);
    match report.detail {
        CheckDetail::Http(h) => {
            assert_eq!(h.status, Some(200));
            assert_eq!(h.response_bytes, Some(5));
            assert!(h.connect.is_some());
        }
        other => panic!("expected http detail, got {other:?}"),
    }
}

#[tokio::test]
async fn http_success_but_failing_assertion_is_a_failure() {
    let addr = serve_fixed("HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello").await;
    let ep = http_endpoint(
        "home",
        &format!("http://{addr}/"),
        vec![Assertion::BodyContains {
            value: "Welcome to nginx".to_string(),
        }],
    );

    let report = run_endpoint(&ep).await;

    assert_eq!(report.outcome, Outcome::Failure);
    assert_eq!(report.failure, Some(FailureKind::Assertion));
    assert_eq!(report.assertions.len(), 1);
    assert!(!report.assertions[0].passed);
    // Detail still recorded: the request succeeded, the assertion did not.
    assert!(matches!(report.detail, CheckDetail::Http(ref h) if h.status == Some(200)));
}

#[tokio::test]
async fn http_transport_failure_maps_to_failure_kind() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let ep = http_endpoint("dead", &format!("http://{addr}/"), vec![]);
    let report = run_endpoint(&ep).await;

    assert_eq!(report.outcome, Outcome::Failure);
    assert_eq!(report.failure, Some(FailureKind::Connect));
    assert!(report.assertions.is_empty());
    assert!(matches!(report.detail, CheckDetail::Http(_)));
}

#[tokio::test]
async fn tcp_success() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let ep = EndpointConfig {
        id: "db".to_string(),
        interval: Duration::from_secs(30),
        timeout: Duration::from_secs(5),
        tags: vec![],
        target: Target::Tcp(TcpTarget {
            host: addr.ip().to_string(),
            port: addr.port(),
        }),
        assertions: vec![],
        allow_internal: false,
    };

    let report = run_endpoint(&ep).await;
    assert_eq!(report.outcome, Outcome::Success);
    assert!(report.failure.is_none());
    match report.detail {
        CheckDetail::Tcp(t) => assert!(t.connect.is_some()),
        other => panic!("expected tcp detail, got {other:?}"),
    }
}

#[tokio::test]
async fn tcp_connect_refused_is_a_failure() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let ep = EndpointConfig {
        id: "db".to_string(),
        interval: Duration::from_secs(30),
        timeout: Duration::from_secs(2),
        tags: vec![],
        target: Target::Tcp(TcpTarget {
            host: addr.ip().to_string(),
            port: addr.port(),
        }),
        assertions: vec![],
        allow_internal: false,
    };

    let report = run_endpoint(&ep).await;
    assert_eq!(report.outcome, Outcome::Failure);
    assert_eq!(report.failure, Some(FailureKind::Connect));
}
