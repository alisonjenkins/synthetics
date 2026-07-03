//! Metric emission abstraction (T3).
//!
//! [`CheckReport`] is a backend-neutral record of one check's result carrying
//! the full Datadog-parity signal set (V9, V17). [`MetricSink`] is the single
//! trait through which the rest of the program emits — check and scheduler code
//! hold a `Arc<dyn MetricSink>` and never touch a backend directly (V2, ADR
//! 0005). Concrete backends (DogStatsD, ...) live in the `sink` module; tests
//! use [`MockSink`].
//!
//! Per V22, assertion results are carried as low-cardinality ids plus a
//! pass/fail flag — never the response body.

use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;

/// The result of one check: success or failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failure,
}

/// Protocol-specific timings and signals for one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckDetail {
    Http(HttpDetail),
    Tcp(TcpDetail),
}

/// Full Datadog-parity signal set for an HTTP check (V17). Every phase timing is
/// optional: a check that fails early (e.g. DNS) records only the phases it
/// reached.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HttpDetail {
    /// Final HTTP status code, if a response was received.
    pub status: Option<u16>,
    /// DNS resolution time.
    pub dns: Option<Duration>,
    /// TCP connect time.
    pub connect: Option<Duration>,
    /// TLS handshake time (absent for plain HTTP).
    pub tls_handshake: Option<Duration>,
    /// Time to first response byte.
    pub ttfb: Option<Duration>,
    /// Body download time (first byte to last).
    pub download: Option<Duration>,
    /// Response size in bytes.
    pub response_bytes: Option<u64>,
    /// Number of redirects followed.
    pub redirects: u32,
    /// Days until the served TLS certificate expires (negative if expired).
    pub cert_days_to_expiry: Option<i64>,
}

/// Signals for a TCP check (V10): the connect time.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TcpDetail {
    pub connect: Option<Duration>,
}

/// The outcome of one assertion. `id` is a low-cardinality identifier of *which*
/// assertion this was (e.g. `status`, `body_login_present`) — never body
/// content (V22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionOutcome {
    pub id: String,
    pub passed: bool,
}

/// Why a check failed. Drives a low-cardinality failure-reason tag; carries no
/// free-form or body-derived data (V22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The check exceeded its deadline (V19).
    Timeout,
    /// DNS resolution failed.
    Dns,
    /// TCP connection failed.
    Connect,
    /// TLS handshake failed.
    Tls,
    /// A protocol-level error occurred exchanging the request/response.
    Protocol,
    /// The request succeeded but one or more assertions failed (V18).
    Assertion,
    /// The check task panicked and was isolated (V21).
    Panic,
}

/// A backend-neutral record of one check's result. Sinks translate this into
/// their wire format; it knows nothing about any backend (V2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckReport {
    /// Stable endpoint id (matches `EndpointConfig::id`).
    pub endpoint_id: String,
    /// Low-cardinality tags to attach to emitted metrics.
    pub tags: Vec<String>,
    /// Overall success/failure (V9).
    pub outcome: Outcome,
    /// Total wall-clock time of the check (V9).
    pub duration: Duration,
    /// Protocol-specific timings and signals (V10, V17).
    pub detail: CheckDetail,
    /// Per-assertion pass/fail (V18, V22).
    pub assertions: Vec<AssertionOutcome>,
    /// Set when `outcome == Failure`, describing why.
    pub failure: Option<FailureKind>,
}

/// Error returned by [`MetricSink::record`]. This is the single failure contract
/// of the emission path; concrete sinks map their internal errors into it.
///
/// Per V13/ADR 0005, sinks are expected to handle transient backend trouble
/// internally (drop-with-counter, reconnect); `record` only returns an error
/// for conditions the caller genuinely cannot ignore. The scheduler logs it and
/// continues — a failing sink never stops checks.
#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    /// The backend is currently unreachable.
    #[error("metric sink unavailable: {0}")]
    Unavailable(String),
    /// The report could not be encoded for this backend.
    #[error("failed to encode report for sink: {0}")]
    Encode(String),
    /// A transport-level error occurred sending to the backend.
    #[error("metric sink transport error")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// The one path through which checks emit metrics (I.sink, V2).
#[async_trait]
pub trait MetricSink: Send + Sync {
    /// Record one check's report. Implementations should be resilient: a backend
    /// hiccup should degrade (drop + count) rather than propagate where possible
    /// (V13).
    async fn record(&self, report: &CheckReport) -> Result<(), SinkError>;
}

/// An in-memory [`MetricSink`] for tests: records every report it is given.
#[derive(Debug, Default)]
pub struct MockSink {
    recorded: Mutex<Vec<CheckReport>>,
}

impl MockSink {
    /// Create an empty mock sink.
    pub fn new() -> Self {
        Self::default()
    }

    /// A snapshot of all reports recorded so far, in order.
    pub fn reports(&self) -> Vec<CheckReport> {
        match self.recorded.lock() {
            Ok(guard) => guard.clone(),
            // A poisoned lock means a test thread panicked while holding it; the
            // recorded data is still readable, so recover it rather than panic.
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Number of reports recorded so far.
    pub fn len(&self) -> usize {
        self.reports().len()
    }

    /// Whether any report has been recorded.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl MetricSink for MockSink {
    async fn record(&self, report: &CheckReport) -> Result<(), SinkError> {
        match self.recorded.lock() {
            Ok(mut guard) => guard.push(report.clone()),
            Err(poisoned) => poisoned.into_inner().push(report.clone()),
        }
        Ok(())
    }
}
