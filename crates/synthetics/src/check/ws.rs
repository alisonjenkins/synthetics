//! WebSocket check executor (T11).
//!
//! Opens a WebSocket connection and times each phase (DNS, TCP connect, TLS
//! handshake for `wss`, and the WebSocket upgrade handshake), bounded by a
//! single deadline (V19). Success means the handshake completed — a working
//! WebSocket uptime check. Reuses the shared TLS config (`check::tls`) and the
//! HTTP cert parser. Per-function error enum (V1); static dispatch (V35).

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{lookup_host, TcpStream};
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::TlsConnector;
use tokio_tungstenite::client_async;

use crate::check::tls::{insecure_client_config, secure_client_config};
use crate::config::WsTarget;

/// Per-phase timings for a WebSocket check (V10, V17).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WsTimings {
    /// DNS resolution time.
    pub dns: Option<Duration>,
    /// TCP connect time.
    pub connect: Option<Duration>,
    /// TLS handshake time (`None` for `ws`).
    pub tls_handshake: Option<Duration>,
    /// WebSocket upgrade handshake time.
    pub handshake: Option<Duration>,
    /// Total wall-clock time of the check.
    pub total: Duration,
}

/// The result of a successful WebSocket handshake.
#[derive(Debug, Clone)]
pub struct WsOutcome {
    /// Phase timings.
    pub timings: WsTimings,
    /// Days until the served TLS certificate expires (`None` for `ws`).
    pub cert_days_to_expiry: Option<i64>,
}

/// Errors from [`run_ws_check`]. One variant per distinct failure mode (V1).
// Variant fields are documented by their `#[error]` messages.
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error)]
pub enum WsCheckError {
    #[error("url has no host")]
    MissingHost,
    #[error("unsupported url scheme {scheme:?} (expected ws or wss)")]
    UnsupportedScheme { scheme: String },
    #[error("check timed out during {phase}")]
    Timeout { phase: &'static str },
    #[error("DNS resolution for {host:?} failed: {source}")]
    Dns {
        host: String,
        #[source]
        source: std::io::Error,
    },
    #[error("DNS resolution for {host:?} returned no addresses")]
    NoAddress { host: String },
    #[error("TCP connect to {addr} failed: {source}")]
    Connect {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("{host:?} is not a valid TLS server name")]
    InvalidDnsName { host: String },
    #[error("TLS handshake with {host:?} failed: {source}")]
    Tls {
        host: String,
        #[source]
        source: std::io::Error,
    },
    #[error("WebSocket handshake failed: {source}")]
    Handshake {
        #[source]
        source: tokio_tungstenite::tungstenite::Error,
    },
}

/// Perform a WebSocket check against `target`, bounded by `timeout` (V19).
pub async fn run_ws_check(target: &WsTarget, timeout: Duration) -> Result<WsOutcome, WsCheckError> {
    let start = Instant::now();
    let deadline = start + timeout;

    let secure = match target.url.scheme() {
        "ws" => false,
        "wss" => true,
        other => {
            return Err(WsCheckError::UnsupportedScheme {
                scheme: other.to_string(),
            })
        }
    };
    let host = target
        .url
        .host_str()
        .ok_or(WsCheckError::MissingHost)?
        .to_string();
    let port = target
        .url
        .port_or_known_default()
        .unwrap_or(if secure { 443 } else { 80 });
    let url = target.url.as_str();

    let mut timings = WsTimings::default();

    // --- DNS ---
    let t = Instant::now();
    let addrs = with_deadline(deadline, "dns", lookup_host((host.clone(), port)))
        .await?
        .map_err(|source| WsCheckError::Dns {
            host: host.clone(),
            source,
        })?;
    let addr = addrs
        .into_iter()
        .next()
        .ok_or_else(|| WsCheckError::NoAddress { host: host.clone() })?;
    timings.dns = Some(t.elapsed());

    // --- TCP connect ---
    let t = Instant::now();
    let tcp = with_deadline(deadline, "connect", TcpStream::connect(addr))
        .await?
        .map_err(|source| WsCheckError::Connect { addr, source })?;
    timings.connect = Some(t.elapsed());

    let cert_days;
    if !secure {
        timings.handshake =
            Some(with_deadline(deadline, "handshake", ws_handshake(url, tcp)).await??);
        cert_days = None;
    } else {
        // --- TLS handshake + cert expiry ---
        let t = Instant::now();
        let tls = tls_connect(tcp, &host, target.allow_insecure, deadline).await?;
        timings.tls_handshake = Some(t.elapsed());
        cert_days = tls
            .get_ref()
            .1
            .peer_certificates()
            .and_then(|certs| certs.first())
            .and_then(crate::check::http::cert_days_to_expiry);
        timings.handshake =
            Some(with_deadline(deadline, "handshake", ws_handshake(url, tls)).await??);
    }

    timings.total = start.elapsed();
    Ok(WsOutcome {
        timings,
        cert_days_to_expiry: cert_days,
    })
}

/// Perform the WebSocket upgrade handshake over an established stream and return
/// how long it took. The connection is dropped afterwards.
async fn ws_handshake<S>(url: &str, stream: S) -> Result<Duration, WsCheckError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let t = Instant::now();
    let (_ws, _response) = client_async(url, stream)
        .await
        .map_err(|source| WsCheckError::Handshake { source })?;
    Ok(t.elapsed())
}

async fn tls_connect(
    tcp: TcpStream,
    host: &str,
    allow_insecure: bool,
    deadline: Instant,
) -> Result<tokio_rustls::client::TlsStream<TcpStream>, WsCheckError> {
    let config = if allow_insecure {
        insecure_client_config()
    } else {
        secure_client_config()
    };
    let server_name =
        ServerName::try_from(host.to_string()).map_err(|_| WsCheckError::InvalidDnsName {
            host: host.to_string(),
        })?;
    let connector = TlsConnector::from(config);
    with_deadline(deadline, "tls", connector.connect(server_name, tcp))
        .await?
        .map_err(|source| WsCheckError::Tls {
            host: host.to_string(),
            source,
        })
}

async fn with_deadline<F: std::future::Future>(
    deadline: Instant,
    phase: &'static str,
    fut: F,
) -> Result<F::Output, WsCheckError> {
    let now = Instant::now();
    if now >= deadline {
        return Err(WsCheckError::Timeout { phase });
    }
    tokio::time::timeout(deadline - now, fut)
        .await
        .map_err(|_| WsCheckError::Timeout { phase })
}
