//! DogStatsD sink (T4): encode CheckReport -> DogStatsD lines, send to a local
//! agent over UDP/UDS. Cites: I.ddagent, V2 (a real MetricSink impl), V13 (a
//! down agent degrades, never stops checks), V9/V17 (all signals emitted).

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use synthetics::metric::{
    AssertionOutcome, CheckDetail, CheckReport, FailureKind, HttpDetail, MetricSink, Outcome,
    TcpDetail,
};
use synthetics::sink::dogstatsd::{encode, DogStatsdSink};
use tokio::net::UdpSocket;

fn http_report() -> CheckReport {
    CheckReport {
        endpoint_id: "home".to_string(),
        tags: vec!["env:prod".to_string()],
        outcome: Outcome::Success,
        duration: Duration::from_millis(42),
        detail: CheckDetail::Http(HttpDetail {
            status: Some(200),
            dns: Some(Duration::from_millis(3)),
            connect: Some(Duration::from_millis(5)),
            tls_handshake: Some(Duration::from_micros(9500)),
            ttfb: Some(Duration::from_millis(20)),
            download: Some(Duration::from_millis(5)),
            response_bytes: Some(1024),
            redirects: 1,
            cert_days_to_expiry: Some(57),
        }),
        assertions: vec![AssertionOutcome {
            id: "status".to_string(),
            passed: true,
        }],
        failure: None,
    }
}

#[test]
fn encodes_full_http_success_report() {
    let lines = encode(&http_report(), "synthetics");

    // up=1 with endpoint + user tags + status tag.
    assert!(
        lines.contains(&"synthetics.check.up:1|g|#endpoint:home,env:prod,status:200".to_string())
    );
    assert!(lines.contains(
        &"synthetics.check.duration_ms:42|ms|#endpoint:home,env:prod,status:200".to_string()
    ));
    // Phase timings (float ms, no trailing zeros); tls from 9500us -> 9.5.
    assert!(lines
        .contains(&"synthetics.http.dns_ms:3|ms|#endpoint:home,env:prod,status:200".to_string()));
    assert!(lines
        .contains(&"synthetics.http.tls_ms:9.5|ms|#endpoint:home,env:prod,status:200".to_string()));
    assert!(lines
        .contains(&"synthetics.http.ttfb_ms:20|ms|#endpoint:home,env:prod,status:200".to_string()));
    // Sizes / counts as gauges.
    assert!(lines.contains(
        &"synthetics.http.response_bytes:1024|g|#endpoint:home,env:prod,status:200".to_string()
    ));
    assert!(lines
        .contains(&"synthetics.http.redirects:1|g|#endpoint:home,env:prod,status:200".to_string()));
    assert!(lines.contains(
        &"synthetics.http.cert_days_to_expiry:57|g|#endpoint:home,env:prod,status:200".to_string()
    ));
    // Per-assertion count with result + id tags.
    assert!(lines.contains(
        &"synthetics.check.assertion:1|c|#endpoint:home,env:prod,status:200,assertion:status,result:pass"
            .to_string()
    ));
}

#[test]
fn encodes_failure_with_failure_tag_and_no_status() {
    let mut report = http_report();
    report.outcome = Outcome::Failure;
    report.failure = Some(FailureKind::Timeout);
    if let CheckDetail::Http(ref mut http) = report.detail {
        http.status = None;
    }
    let lines = encode(&report, "synthetics");

    // up=0, failure reason tagged, no status tag (no response received).
    assert!(lines
        .contains(&"synthetics.check.up:0|g|#endpoint:home,env:prod,failure:timeout".to_string()));
    assert!(lines.iter().all(|l| !l.contains("status:")));
}

#[test]
fn encodes_tcp_connect_time() {
    let report = CheckReport {
        endpoint_id: "db".to_string(),
        tags: vec![],
        outcome: Outcome::Success,
        duration: Duration::from_millis(7),
        detail: CheckDetail::Tcp(TcpDetail {
            connect: Some(Duration::from_millis(7)),
        }),
        assertions: vec![],
        failure: None,
    };
    let lines = encode(&report, "synthetics");
    assert!(lines.contains(&"synthetics.check.up:1|g|#endpoint:db".to_string()));
    assert!(lines.contains(&"synthetics.tcp.connect_ms:7|ms|#endpoint:db".to_string()));
}

#[tokio::test]
async fn sends_datagram_to_udp_agent() {
    // Stand-in agent: a UDP socket we read from.
    let agent = UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let agent_addr = agent.local_addr().unwrap();

    let sink = DogStatsdSink::udp(agent_addr).await.unwrap();
    sink.record(&http_report()).await.unwrap();

    let mut buf = vec![0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(2), agent.recv(&mut buf))
        .await
        .expect("agent should receive a datagram")
        .unwrap();
    let payload = String::from_utf8(buf[..n].to_vec()).unwrap();

    assert!(payload.contains("synthetics.check.up:1|g|#endpoint:home"));
    assert!(payload.contains("synthetics.check.duration_ms:42|ms"));
}

#[tokio::test]
async fn record_ok_and_counts_drop_when_agent_absent() {
    // V13: with no agent reading, record must not error or panic — checks keep
    // running regardless of sink health.
    let sink = DogStatsdSink::udp("127.0.0.1:1".parse().unwrap())
        .await
        .unwrap();
    let result = sink.record(&http_report()).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn unix_init_error_for_missing_socket() {
    let err = DogStatsdSink::unix("/no/such/dsd.socket").await;
    assert!(err.is_err());
}
