//! MetricSink trait + neutral CheckReport + MockSink (T3).
//! Cites: I.sink (swappable backend), V2 (all emission goes through the trait),
//! V9 (report carries outcome + latency + detail), V22 (assertion outcomes are
//! low-cardinality ids, never body content).

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use synthetics::metric::{
    AssertionOutcome, CheckDetail, CheckReport, HttpDetail, MetricSink, MockSink, Outcome,
};

fn http_success_report() -> CheckReport {
    CheckReport {
        endpoint_id: "home".to_string(),
        tags: vec!["env:prod".to_string()],
        outcome: Outcome::Success,
        duration: Duration::from_millis(42),
        detail: CheckDetail::Http(HttpDetail {
            status: Some(200),
            dns: Some(Duration::from_millis(3)),
            connect: Some(Duration::from_millis(5)),
            tls_handshake: Some(Duration::from_millis(9)),
            ttfb: Some(Duration::from_millis(20)),
            download: Some(Duration::from_millis(5)),
            response_bytes: Some(1024),
            redirects: 0,
            cert_days_to_expiry: Some(57),
        }),
        assertions: vec![AssertionOutcome {
            id: "status".to_string(),
            passed: true,
        }],
        failure: None,
    }
}

#[tokio::test]
async fn mock_sink_records_reports_in_order() {
    let sink = MockSink::new();
    sink.record(&http_success_report()).await.unwrap();

    let mut second = http_success_report();
    second.endpoint_id = "db".to_string();
    second.outcome = Outcome::Failure;
    sink.record(&second).await.unwrap();

    let recorded = sink.reports();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].endpoint_id, "home");
    assert_eq!(recorded[0].outcome, Outcome::Success);
    assert_eq!(recorded[1].endpoint_id, "db");
    assert_eq!(recorded[1].outcome, Outcome::Failure);
}

#[tokio::test]
async fn mock_sink_usable_as_trait_object() {
    // V2: check/scheduler code holds `Arc<dyn MetricSink>` and only calls record.
    let sink: Arc<dyn MetricSink> = Arc::new(MockSink::new());
    sink.record(&http_success_report()).await.unwrap();

    // Downcast-free observation: the concrete mock exposes its buffer, but the
    // emission path only ever saw the trait.
    let clone = Arc::clone(&sink);
    clone.record(&http_success_report()).await.unwrap();
}

#[tokio::test]
async fn report_carries_full_signal_set() {
    // V9/V17: outcome, latency, and protocol detail are all present.
    let report = http_success_report();
    match &report.detail {
        CheckDetail::Http(http) => {
            assert_eq!(http.status, Some(200));
            assert_eq!(http.ttfb, Some(Duration::from_millis(20)));
            assert_eq!(http.cert_days_to_expiry, Some(57));
        }
        other => panic!("expected http detail, got {other:?}"),
    }
    // V22: assertion outcomes are ids + pass/fail, never body content.
    assert_eq!(report.assertions[0].id, "status");
    assert!(report.assertions[0].passed);
}
