//! WebSocket check tests (T11): real handshake against a local WS server,
//! phase timings, error mapping, config parsing, and the orchestrator arm.
//! Cites I.ws, V1, V9, V10, V36.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::time::Duration;

use synthetics::check::run_endpoint;
use synthetics::check::ws::{run_ws_check, WsCheckError};
use synthetics::config::{parse_config, EndpointConfig, Target, WsTarget};
use synthetics::metric::{CheckDetail, Outcome};
use tokio::net::TcpListener;
use url::Url;

/// A WebSocket server that completes the handshake for every connection.
async fn ws_server() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                // Completing the accept handshake is enough for an uptime check.
                let _ = tokio_tungstenite::accept_async(stream).await;
            });
        }
    });
    addr
}

fn ws_target(url: &str) -> WsTarget {
    WsTarget {
        url: Url::parse(url).unwrap(),
        allow_insecure: false,
    }
}

#[tokio::test]
async fn handshake_succeeds_and_records_timings() {
    let addr = ws_server().await;
    let out = run_ws_check(&ws_target(&format!("ws://{addr}/")), Duration::from_secs(5))
        .await
        .unwrap();

    assert!(out.timings.dns.is_some());
    assert!(out.timings.connect.is_some());
    assert!(out.timings.handshake.is_some(), "ws handshake timed");
    assert!(out.timings.tls_handshake.is_none()); // plain ws
    assert!(out.cert_days_to_expiry.is_none());
    assert!(out.timings.total > Duration::ZERO);
}

#[tokio::test]
async fn wrong_scheme_is_unsupported() {
    let err = run_ws_check(&ws_target("http://127.0.0.1/"), Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(
        matches!(err, WsCheckError::UnsupportedScheme { .. }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn connection_refused_is_a_connect_error() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);
    let err = run_ws_check(&ws_target(&format!("ws://{addr}/")), Duration::from_secs(2))
        .await
        .unwrap_err();
    assert!(matches!(err, WsCheckError::Connect { .. }), "got {err:?}");
}

#[test]
fn config_parses_ws_endpoint() {
    let cfg = parse_config(
        r#"
[[endpoint]]
id = "chat"
  [endpoint.ws]
  url = "wss://chat.example.com/socket"
"#,
    )
    .unwrap();
    assert_eq!(cfg.endpoints.len(), 1);
    assert!(matches!(cfg.endpoints[0].target, Target::Ws(_)));
}

#[test]
fn config_rejects_non_ws_scheme() {
    let err = parse_config(
        r#"
[[endpoint]]
id = "bad"
  [endpoint.ws]
  url = "https://not-a-ws/"
"#,
    );
    assert!(err.is_err());
}

#[tokio::test]
async fn orchestrator_produces_ws_report() {
    let addr = ws_server().await;
    let ep = EndpointConfig {
        id: "chat".to_string(),
        interval: Duration::from_secs(30),
        timeout: Duration::from_secs(5),
        tags: vec![],
        target: Target::Ws(ws_target(&format!("ws://{addr}/"))),
        assertions: vec![],
    };

    let report = run_endpoint(&ep).await;
    assert_eq!(report.outcome, Outcome::Success);
    match report.detail {
        CheckDetail::Ws(ws) => assert!(ws.handshake.is_some()),
        other => panic!("expected ws detail, got {other:?}"),
    }
}
