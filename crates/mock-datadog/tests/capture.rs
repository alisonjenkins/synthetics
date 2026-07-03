//! Mock Datadog tests (T29): parse DogStatsD, capture with eviction, and a real
//! UDP round-trip of the wire format the DogStatsdSink emits (V39).

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use mock_datadog::{parse_line, Capture};
use tokio::net::UdpSocket;

#[test]
fn parses_a_dogstatsd_line() {
    let m = parse_line("synthetics.check.duration_ms:42|ms|#endpoint:home,env:prod").unwrap();
    assert_eq!(m.name, "synthetics.check.duration_ms");
    assert_eq!(m.value, 42.0);
    assert_eq!(m.kind, "ms");
    assert_eq!(
        m.tags,
        vec!["endpoint:home".to_string(), "env:prod".to_string()]
    );
}

#[test]
fn rejects_malformed_lines() {
    assert!(parse_line("").is_none());
    assert!(parse_line("no-value").is_none());
    assert!(parse_line("name:notanumber|g").is_none());
}

#[test]
fn ingests_multiline_datagram_and_evicts_oldest() {
    let cap = Capture::new(2);
    cap.ingest("a:1|g\nb:2|c\nc:3|g"); // three lines, capacity 2
    let snap = cap.snapshot();
    assert_eq!(snap.len(), 2);
    // Oldest ("a") evicted; "b" then "c" remain in order.
    assert_eq!(snap[0].name, "b");
    assert_eq!(snap[1].name, "c");
}

#[tokio::test]
async fn udp_roundtrip_captures_sent_metrics() {
    let receiver = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let addr = receiver.local_addr().unwrap();
    let sender = UdpSocket::bind("127.0.0.1:0").await.unwrap();

    sender
        .send_to(b"synthetics.check.up:1|g|#endpoint:home,env:prod", addr)
        .await
        .unwrap();

    let mut buf = [0u8; 2048];
    let (n, _) = receiver.recv_from(&mut buf).await.unwrap();

    let capture = Capture::new(100);
    capture.ingest(std::str::from_utf8(&buf[..n]).unwrap());

    assert_eq!(capture.len(), 1);
    let m = &capture.snapshot()[0];
    assert_eq!(m.name, "synthetics.check.up");
    assert_eq!(m.value, 1.0);
    assert_eq!(m.kind, "g");
    assert!(m.tags.contains(&"endpoint:home".to_string()));
}
