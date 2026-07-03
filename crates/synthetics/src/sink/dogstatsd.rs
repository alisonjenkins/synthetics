//! DogStatsD sink (T4): the first real [`MetricSink`] backend (I.ddagent).
//!
//! Encodes a [`CheckReport`] into DogStatsD line protocol and sends it to a
//! local Datadog agent over UDP or a Unix datagram socket. A datagram protocol
//! is fire-and-forget: if the agent is down, sends are dropped and counted, and
//! `record` still returns `Ok` — a sick agent never stops checks (V13, V2).

use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tokio::net::UdpSocket;

use crate::metric::{CheckDetail, CheckReport, FailureKind, MetricSink, Outcome, SinkError};

/// Metric namespace prefix for all emitted series.
pub const DEFAULT_NAMESPACE: &str = "synthetics";

/// Errors constructing a [`DogStatsdSink`].
// Variant fields are documented by their `#[error]` messages.
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error)]
pub enum DogStatsdInitError {
    /// Binding the local UDP socket failed.
    #[error("failed to bind local UDP socket: {0}")]
    UdpBind(#[source] io::Error),
    /// Connecting the UDP socket to the agent address failed.
    #[error("failed to connect UDP socket to agent {addr}: {source}")]
    UdpConnect {
        addr: SocketAddr,
        #[source]
        source: io::Error,
    },
    /// Connecting the Unix datagram socket to the agent path failed.
    #[cfg(unix)]
    #[error("failed to connect unix datagram socket to {path}: {source}")]
    UnixConnect {
        path: std::path::PathBuf,
        #[source]
        source: io::Error,
    },
}

/// A [`MetricSink`] that emits DogStatsD to a local Datadog agent.
#[derive(Debug)]
pub struct DogStatsdSink {
    transport: Transport,
    namespace: String,
    dropped: AtomicU64,
}

#[derive(Debug)]
enum Transport {
    Udp(UdpSocket),
    #[cfg(unix)]
    Unix(tokio::net::UnixDatagram),
}

impl DogStatsdSink {
    /// Connect a UDP sink to the agent at `addr` (typically `127.0.0.1:8125`).
    pub async fn udp(addr: SocketAddr) -> Result<Self, DogStatsdInitError> {
        let bind: SocketAddr = if addr.is_ipv4() {
            ([0, 0, 0, 0], 0).into()
        } else {
            ([0u16, 0, 0, 0, 0, 0, 0, 0], 0).into()
        };
        let socket = UdpSocket::bind(bind)
            .await
            .map_err(DogStatsdInitError::UdpBind)?;
        socket
            .connect(addr)
            .await
            .map_err(|source| DogStatsdInitError::UdpConnect { addr, source })?;
        Ok(Self::from_transport(Transport::Udp(socket)))
    }

    /// Connect a Unix-datagram sink to the agent socket at `path` (typically
    /// `/var/run/datadog/dsd.socket`).
    #[cfg(unix)]
    pub async fn unix(path: impl AsRef<std::path::Path>) -> Result<Self, DogStatsdInitError> {
        let path = path.as_ref();
        let socket = tokio::net::UnixDatagram::unbound().map_err(|source| {
            DogStatsdInitError::UnixConnect {
                path: path.to_path_buf(),
                source,
            }
        })?;
        socket
            .connect(path)
            .map_err(|source| DogStatsdInitError::UnixConnect {
                path: path.to_path_buf(),
                source,
            })?;
        Ok(Self::from_transport(Transport::Unix(socket)))
    }

    fn from_transport(transport: Transport) -> Self {
        Self {
            transport,
            namespace: DEFAULT_NAMESPACE.to_string(),
            dropped: AtomicU64::new(0),
        }
    }

    /// Number of report datagrams dropped because the agent was unreachable (V13).
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    async fn send(&self, buf: &[u8]) -> io::Result<usize> {
        match &self.transport {
            Transport::Udp(s) => s.send(buf).await,
            #[cfg(unix)]
            Transport::Unix(s) => s.send(buf).await,
        }
    }
}

impl MetricSink for DogStatsdSink {
    async fn record(&self, report: &CheckReport) -> Result<(), SinkError> {
        let lines = encode(report, &self.namespace);
        if lines.is_empty() {
            return Ok(());
        }
        let payload = lines.join("\n");
        if let Err(source) = self.send(payload.as_bytes()).await {
            // V13: degrade. A down/overloaded agent must not stop checks, so we
            // drop the datagram, bump a counter, and log — but return Ok.
            self.dropped.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                endpoint = %report.endpoint_id,
                error = %source,
                dropped_total = self.dropped(),
                "dropped DogStatsD datagram; agent unreachable",
            );
        }
        Ok(())
    }
}

/// Encode one report into DogStatsD line-protocol metrics (one string per line).
///
/// Public so the exact wire format can be asserted directly in tests. Body
/// content is never encoded — only outcomes and low-cardinality tags (V22).
pub fn encode(report: &CheckReport, namespace: &str) -> Vec<String> {
    let tags = tags_for(report);
    let mut lines = Vec::new();

    let up = match report.outcome {
        Outcome::Success => 1,
        Outcome::Failure => 0,
    };
    lines.push(format!("{namespace}.check.up:{up}|g|#{tags}"));
    lines.push(format!(
        "{namespace}.check.duration_ms:{}|ms|#{tags}",
        ms(report.duration)
    ));

    match &report.detail {
        CheckDetail::Http(http) => {
            push_timing(&mut lines, namespace, "http.dns_ms", http.dns, &tags);
            push_timing(
                &mut lines,
                namespace,
                "http.connect_ms",
                http.connect,
                &tags,
            );
            push_timing(
                &mut lines,
                namespace,
                "http.tls_ms",
                http.tls_handshake,
                &tags,
            );
            push_timing(&mut lines, namespace, "http.ttfb_ms", http.ttfb, &tags);
            push_timing(
                &mut lines,
                namespace,
                "http.download_ms",
                http.download,
                &tags,
            );
            if let Some(bytes) = http.response_bytes {
                lines.push(format!("{namespace}.http.response_bytes:{bytes}|g|#{tags}"));
            }
            lines.push(format!(
                "{namespace}.http.redirects:{}|g|#{tags}",
                http.redirects
            ));
            if let Some(days) = http.cert_days_to_expiry {
                lines.push(format!(
                    "{namespace}.http.cert_days_to_expiry:{days}|g|#{tags}"
                ));
            }
        }
        CheckDetail::Tcp(tcp) => {
            push_timing(&mut lines, namespace, "tcp.connect_ms", tcp.connect, &tags);
        }
    }

    for assertion in &report.assertions {
        let result = if assertion.passed { "pass" } else { "fail" };
        lines.push(format!(
            "{namespace}.check.assertion:1|c|#{tags},assertion:{},result:{result}",
            assertion.id
        ));
    }

    lines
}

fn push_timing(lines: &mut Vec<String>, ns: &str, name: &str, value: Option<Duration>, tags: &str) {
    if let Some(d) = value {
        lines.push(format!("{ns}.{name}:{}|ms|#{tags}", ms(d)));
    }
}

/// Base tag set: `endpoint:<id>`, the endpoint's own tags, then `status:<code>`
/// (if a response was received) and `failure:<kind>` (if the check failed).
fn tags_for(report: &CheckReport) -> String {
    let mut tags = format!("endpoint:{}", report.endpoint_id);
    for tag in &report.tags {
        tags.push(',');
        tags.push_str(tag);
    }
    if let CheckDetail::Http(http) = &report.detail {
        if let Some(status) = http.status {
            tags.push_str(&format!(",status:{status}"));
        }
    }
    if let Some(failure) = report.failure {
        tags.push_str(&format!(",failure:{}", failure_tag(failure)));
    }
    tags
}

fn failure_tag(failure: FailureKind) -> &'static str {
    match failure {
        FailureKind::Timeout => "timeout",
        FailureKind::Dns => "dns",
        FailureKind::Connect => "connect",
        FailureKind::Tls => "tls",
        FailureKind::Protocol => "protocol",
        FailureKind::Assertion => "assertion",
        FailureKind::Panic => "panic",
    }
}

/// Duration as fractional milliseconds; whole values print without a decimal
/// (`42`), fractional values keep their precision (`9.5`).
fn ms(d: Duration) -> f64 {
    d.as_micros() as f64 / 1000.0
}
