//! TCP check executor (T6).
//!
//! Resolves the target and opens a TCP connection, timing the DNS and connect
//! phases (V10, V17). A single deadline bounds the whole check and each phase
//! (V19): a target that neither accepts nor refuses fails as a timeout rather
//! than hanging (V20). Errors are a per-function enum (V1). Static dispatch (V35).

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::net::{lookup_host, TcpStream};

use crate::config::TcpTarget;

/// Per-phase timings for a TCP check (V10, V17).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TcpTimings {
    /// DNS resolution time.
    pub dns: Option<Duration>,
    /// TCP connect time.
    pub connect: Option<Duration>,
    /// Total wall-clock time of the check.
    pub total: Duration,
}

/// The result of a successful TCP connect.
#[derive(Debug, Clone)]
pub struct TcpOutcome {
    /// Phase timings.
    pub timings: TcpTimings,
    /// The peer address that was connected to.
    pub peer: SocketAddr,
}

/// Errors from [`run_tcp_check`]. One variant per distinct failure mode (V1).
// Variant fields are documented by their `#[error]` messages.
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error)]
pub enum TcpCheckError {
    /// A phase exceeded the check deadline (V19).
    #[error("check timed out during {phase}")]
    Timeout { phase: &'static str },
    /// DNS resolution failed.
    #[error("DNS resolution for {host:?} failed: {source}")]
    Dns {
        host: String,
        #[source]
        source: std::io::Error,
    },
    /// DNS resolution returned no addresses.
    #[error("DNS resolution for {host:?} returned no addresses")]
    NoAddress { host: String },
    /// TCP connect failed.
    #[error("TCP connect to {addr} failed: {source}")]
    Connect {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    /// The resolved address was refused by the SSRF network policy (#5).
    #[error("connection to {addr} blocked: {source}")]
    Blocked {
        addr: SocketAddr,
        #[source]
        source: crate::net_guard::GuardError,
    },
}

/// Perform a TCP check against `target`, bounded by `timeout` (total + per
/// phase, V19).
pub async fn run_tcp_check(
    target: &TcpTarget,
    timeout: Duration,
    allow_internal: bool,
    policy: &crate::net_guard::NetPolicy,
) -> Result<TcpOutcome, TcpCheckError> {
    let start = Instant::now();
    let deadline = start + timeout;

    let mut timings = TcpTimings::default();

    // --- DNS ---
    let t = Instant::now();
    let addrs = with_deadline(
        deadline,
        "dns",
        lookup_host((target.host.clone(), target.port)),
    )
    .await?
    .map_err(|source| TcpCheckError::Dns {
        host: target.host.clone(),
        source,
    })?;
    let addr = addrs
        .into_iter()
        .next()
        .ok_or_else(|| TcpCheckError::NoAddress {
            host: target.host.clone(),
        })?;
    timings.dns = Some(t.elapsed());

    // SSRF guard (#5): refuse an internal resolved address unless opted in and
    // operator-allowlisted (checked post-DNS to defend against rebinding).
    policy
        .guard(addr.ip(), allow_internal)
        .map_err(|source| TcpCheckError::Blocked { addr, source })?;

    // --- TCP connect ---
    let t = Instant::now();
    let _stream = with_deadline(deadline, "connect", TcpStream::connect(addr))
        .await?
        .map_err(|source| TcpCheckError::Connect { addr, source })?;
    timings.connect = Some(t.elapsed());

    timings.total = start.elapsed();
    Ok(TcpOutcome {
        timings,
        peer: addr,
    })
}

/// Run `fut` with the time remaining until `deadline`; map expiry to a phase
/// timeout (V19).
async fn with_deadline<F: std::future::Future>(
    deadline: Instant,
    phase: &'static str,
    fut: F,
) -> Result<F::Output, TcpCheckError> {
    let now = Instant::now();
    if now >= deadline {
        return Err(TcpCheckError::Timeout { phase });
    }
    tokio::time::timeout(deadline - now, fut)
        .await
        .map_err(|_| TcpCheckError::Timeout { phase })
}
