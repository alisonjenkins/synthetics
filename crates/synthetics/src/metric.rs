//! Metric emission abstraction (T3).
//!
//! [`CheckReport`] is a backend-neutral record of one check's result carrying
//! the full Datadog-parity signal set (V9, V17). [`MetricSink`] is the single
//! trait through which the rest of the program emits — check and scheduler code
//! are generic over `M: MetricSink` and share it as `Arc<M>` (static dispatch,
//! never `dyn`; ADR 0021), and never touch a backend directly (V2, ADR 0005).
//! Concrete backends (DogStatsD, ...) live in the `sink` module; tests use
//! [`MockSink`].
//!
//! Per V22, assertion results are carried as low-cardinality ids plus a
//! pass/fail flag — never the response body.

use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

/// The result of one check: success or failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The check passed (transport succeeded and all assertions held).
    Success,
    /// The check failed (transport error, timeout, panic, or a failed assertion).
    Failure,
}

/// Protocol-specific timings and signals for one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckDetail {
    /// Signals from an HTTP(S) check.
    Http(HttpDetail),
    /// Signals from a TCP check.
    Tcp(TcpDetail),
    /// Signals from a WebSocket check.
    Ws(WsDetail),
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
    /// TCP connect time.
    pub connect: Option<Duration>,
}

/// Signals for a WebSocket check (V17): phase timings + cert age.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WsDetail {
    /// DNS resolution time.
    pub dns: Option<Duration>,
    /// TCP connect time.
    pub connect: Option<Duration>,
    /// TLS handshake time (`None` for `ws`).
    pub tls_handshake: Option<Duration>,
    /// WebSocket upgrade handshake time.
    pub handshake: Option<Duration>,
    /// Days until the served TLS certificate expires (`None` for `ws`).
    pub cert_days_to_expiry: Option<i64>,
}

/// The outcome of one assertion. `id` is a low-cardinality identifier of *which*
/// assertion this was (e.g. `status`, `body_login_present`) — never body
/// content (V22).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionOutcome {
    /// Low-cardinality identifier of which assertion this was (e.g. `status`).
    pub id: String,
    /// Whether the assertion held.
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
///
/// Static dispatch (ADR 0021, V35): `record` is a native async fn in the trait
/// (RPITIT) with an explicit `Send` bound on the returned future so consumers
/// can spawn it. Consumers take `impl MetricSink` / a generic `M: MetricSink`,
/// not `dyn MetricSink` — no per-emit boxing or vtable on the hot path.
pub trait MetricSink: Send + Sync {
    /// Record one check's report. Implementations should be resilient: a backend
    /// hiccup should degrade (drop + count) rather than propagate where possible
    /// (V13).
    fn record(&self, report: &CheckReport) -> impl Future<Output = Result<(), SinkError>> + Send;
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

impl MetricSink for MockSink {
    async fn record(&self, report: &CheckReport) -> Result<(), SinkError> {
        match self.recorded.lock() {
            Ok(mut guard) => guard.push(report.clone()),
            Err(poisoned) => poisoned.into_inner().push(report.clone()),
        }
        Ok(())
    }
}
