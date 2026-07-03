//! HTTP check executor tests (T5a, plaintext). Real requests against local
//! throwaway TCP servers — no mocks (V7). Cites I.http, V1, V10, V17, V19.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::net::SocketAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use synthetics::check::http::{run_http_check, HttpCheckError, DEFAULT_MAX_REDIRECTS};
use synthetics::config::HttpTarget;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

fn target(url: &str) -> HttpTarget {
    HttpTarget {
        url: Url::parse(url).unwrap(),
        method: "GET".to_string(),
        headers: vec![],
        body: None,
    }
}

/// Serve a fixed raw HTTP/1.1 response to every connection.
async fn serve_fixed(response: &'static str) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(pair) => pair,
                Err(_) => return,
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await; // consume request
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.flush().await;
            });
        }
    });
    addr
}

#[tokio::test]
async fn plaintext_get_records_status_size_and_phase_timings() {
    let addr = serve_fixed(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 5\r\n\r\nhello",
    )
    .await;

    let out = run_http_check(
        &target(&format!("http://{addr}/")),
        Duration::from_secs(5),
        0,
    )
    .await
    .unwrap();

    assert_eq!(out.status, 200);
    assert_eq!(out.body, b"hello");
    assert_eq!(out.response_bytes, 5);
    assert_eq!(out.redirects, 0);
    assert!(!out.body_truncated);
    // Phase timings present for the phases a plaintext check reaches (V17).
    assert!(out.timings.dns.is_some(), "dns timed");
    assert!(out.timings.connect.is_some(), "connect timed");
    assert!(out.timings.ttfb.is_some(), "ttfb timed");
    assert!(out.timings.download.is_some(), "download timed");
    assert!(out.timings.total > Duration::ZERO);
    // No TLS on plaintext.
    assert!(out.timings.tls_handshake.is_none());
    assert!(out.cert_days_to_expiry.is_none());
}

#[tokio::test]
async fn follows_redirect_and_counts_it() {
    // Stateful server: first connection 302s to /final, second returns 200.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits2 = Arc::clone(&hits);
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let n = hits2.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                let resp = if n == 0 {
                    "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\n\r\n"
                        .to_string()
                } else {
                    "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok".to_string()
                };
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
            });
        }
    });

    let out = run_http_check(
        &target(&format!("http://{addr}/")),
        Duration::from_secs(5),
        DEFAULT_MAX_REDIRECTS,
    )
    .await
    .unwrap();

    assert_eq!(out.status, 200);
    assert_eq!(out.redirects, 1);
    assert_eq!(out.body, b"ok");
}

#[tokio::test]
async fn too_many_redirects_is_an_error() {
    // Always 302s to itself.
    let addr =
        serve_fixed("HTTP/1.1 302 Found\r\nLocation: /again\r\nContent-Length: 0\r\n\r\n").await;

    let err = run_http_check(
        &target(&format!("http://{addr}/")),
        Duration::from_secs(5),
        2,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, HttpCheckError::TooManyRedirects { max: 2 }));
}

#[tokio::test]
async fn hung_server_times_out_on_request_phase() {
    // Accept + read, but never respond.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut sock, _) = match listener.accept().await {
                Ok(p) => p,
                Err(_) => return,
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let _ = sock.read(&mut buf).await;
                // hang forever
                std::future::pending::<()>().await;
            });
        }
    });

    let err = run_http_check(
        &target(&format!("http://{addr}/")),
        Duration::from_millis(300),
        0,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, HttpCheckError::Timeout { .. }), "got {err:?}");
}

#[tokio::test]
async fn connection_refused_is_a_connect_error() {
    // Bind then drop to obtain an address nobody is listening on.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    let err = run_http_check(
        &target(&format!("http://{addr}/")),
        Duration::from_secs(2),
        0,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, HttpCheckError::Connect { .. }), "got {err:?}");
}

#[tokio::test]
async fn unresolvable_host_is_a_dns_error() {
    let err = run_http_check(
        &target("http://nonexistent.invalid/"),
        Duration::from_secs(3),
        0,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, HttpCheckError::Dns { .. }), "got {err:?}");
}

#[tokio::test]
async fn https_is_unsupported_until_tls_lands() {
    let err = run_http_check(&target("https://127.0.0.1/"), Duration::from_secs(2), 0)
        .await
        .unwrap_err();
    assert!(
        matches!(err, HttpCheckError::UnsupportedScheme { .. }),
        "got {err:?}"
    );
}
