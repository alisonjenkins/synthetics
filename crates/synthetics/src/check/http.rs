//! HTTP check executor (T5).
//!
//! Drives the request phases explicitly over a low-level stack (ADR 0001) so
//! every Datadog-parity timing (V17) is real: DNS resolution, TCP connect, TLS
//! handshake (T5b), time-to-first-byte, and body download are each measured. A
//! single deadline bounds the whole check and every phase within it (V19): no
//! await is issued without a remaining-time budget, so a slow or hung target
//! fails as a timeout rather than hanging (V20). Errors are a per-function enum
//! (V1). Static dispatch throughout (V35) — no `dyn`.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Method, Request};
use hyper_util::rt::TokioIo;
use tokio::net::{lookup_host, TcpStream};
use url::Url;

use crate::config::HttpTarget;

/// Maximum response body bytes retained for assertions; larger bodies are
/// truncated so one huge response cannot blow memory (V15, V22).
pub const MAX_BODY_BYTES: usize = 2 * 1024 * 1024;

/// Default cap on redirects followed before giving up.
pub const DEFAULT_MAX_REDIRECTS: u32 = 5;

/// Per-phase timings for an HTTP check (V10, V17). Each phase is optional: a
/// check that fails early records only the phases it reached.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HttpTimings {
    /// DNS resolution time.
    pub dns: Option<Duration>,
    /// TCP connect time.
    pub connect: Option<Duration>,
    /// TLS handshake time (`None` for plain HTTP; filled by T5b for HTTPS).
    pub tls_handshake: Option<Duration>,
    /// Time from request send to first response byte.
    pub ttfb: Option<Duration>,
    /// Body download time (first byte to last / truncation).
    pub download: Option<Duration>,
    /// Total wall-clock time of the whole check (all hops).
    pub total: Duration,
}

/// The result of a successful HTTP exchange (transport succeeded; assertions are
/// evaluated separately in T14).
#[derive(Debug, Clone)]
pub struct HttpOutcome {
    /// Final response status code (after following redirects).
    pub status: u16,
    /// Phase timings; `total` spans all redirect hops, other phases are the
    /// final hop.
    pub timings: HttpTimings,
    /// Bytes of body retained (may be truncated at [`MAX_BODY_BYTES`]).
    pub response_bytes: u64,
    /// Whether the body was truncated at the cap.
    pub body_truncated: bool,
    /// Number of redirects followed to reach the final response.
    pub redirects: u32,
    /// Final response headers (name lower-cased by the peer as sent).
    pub headers: Vec<(String, String)>,
    /// Final response body (possibly truncated). Never leaves the process
    /// except as assertion outcomes (V22).
    pub body: Vec<u8>,
    /// Days until the served TLS certificate expires (`None` for plain HTTP;
    /// filled by T5b for HTTPS).
    pub cert_days_to_expiry: Option<i64>,
}

/// Errors from [`run_http_check`]. One variant per distinct failure mode (V1).
#[derive(Debug, thiserror::Error)]
pub enum HttpCheckError {
    /// The URL has no host component.
    #[error("url has no host")]
    MissingHost,
    /// The URL scheme is not supported by this executor.
    #[error("unsupported url scheme {scheme:?}")]
    UnsupportedScheme { scheme: String },
    /// The request method string is not a valid HTTP method.
    #[error("invalid HTTP method {method:?}")]
    InvalidMethod { method: String },
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
    /// Building the HTTP request failed (bad header, URI, ...).
    #[error("failed to build request: {source}")]
    Request {
        #[source]
        source: hyper::http::Error,
    },
    /// A protocol-level error occurred exchanging the request/response.
    #[error("HTTP protocol error: {source}")]
    Protocol {
        #[source]
        source: hyper::Error,
    },
    /// A redirect target was not a valid URL.
    #[error("redirect Location {location:?} is not a valid url: {source}")]
    BadRedirect {
        location: String,
        #[source]
        source: url::ParseError,
    },
    /// More than the allowed number of redirects were encountered.
    #[error("exceeded max redirects ({max})")]
    TooManyRedirects { max: u32 },
}

/// Perform an HTTP check against `target`, bounded by `timeout` (total + per
/// phase, V19), following up to `max_redirects` redirects.
pub async fn run_http_check(
    target: &HttpTarget,
    timeout: Duration,
    max_redirects: u32,
) -> Result<HttpOutcome, HttpCheckError> {
    let start = Instant::now();
    let deadline = start + timeout;

    let mut url = target.url.clone();
    let mut redirects = 0u32;

    loop {
        let hop = fetch_once(&url, target, deadline).await?;

        if is_redirect(hop.status) {
            if let Some(location) = hop.location.as_deref() {
                if redirects >= max_redirects {
                    return Err(HttpCheckError::TooManyRedirects { max: max_redirects });
                }
                url = url
                    .join(location)
                    .map_err(|source| HttpCheckError::BadRedirect {
                        location: location.to_string(),
                        source,
                    })?;
                redirects += 1;
                continue;
            }
            // A 3xx without a Location is treated as the final response.
        }

        return Ok(HttpOutcome {
            status: hop.status,
            timings: HttpTimings {
                total: start.elapsed(),
                ..hop.timings
            },
            response_bytes: hop.body.len() as u64,
            body_truncated: hop.truncated,
            redirects,
            headers: hop.headers,
            body: hop.body,
            cert_days_to_expiry: hop.cert_days_to_expiry,
        });
    }
}

/// One request/response hop (no redirect following).
struct Hop {
    status: u16,
    timings: HttpTimings,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    truncated: bool,
    location: Option<String>,
    cert_days_to_expiry: Option<i64>,
}

async fn fetch_once(
    url: &Url,
    target: &HttpTarget,
    deadline: Instant,
) -> Result<Hop, HttpCheckError> {
    let scheme = url.scheme();
    if scheme != "http" {
        // TLS (https) lands in T5b.
        return Err(HttpCheckError::UnsupportedScheme {
            scheme: scheme.to_string(),
        });
    }
    let host = url
        .host_str()
        .ok_or(HttpCheckError::MissingHost)?
        .to_string();
    let port = url.port_or_known_default().unwrap_or(80);

    let mut timings = HttpTimings::default();

    // --- DNS ---
    let t = Instant::now();
    let addrs = with_deadline(deadline, "dns", lookup_host((host.clone(), port)))
        .await?
        .map_err(|source| HttpCheckError::Dns {
            host: host.clone(),
            source,
        })?;
    let addr = addrs
        .into_iter()
        .next()
        .ok_or_else(|| HttpCheckError::NoAddress { host: host.clone() })?;
    timings.dns = Some(t.elapsed());

    // --- TCP connect ---
    let t = Instant::now();
    let stream = with_deadline(deadline, "connect", TcpStream::connect(addr))
        .await?
        .map_err(|source| HttpCheckError::Connect { addr, source })?;
    timings.connect = Some(t.elapsed());

    // --- HTTP/1.1 handshake over the stream ---
    let (mut sender, conn) = with_deadline(deadline, "connect", async {
        hyper::client::conn::http1::handshake(TokioIo::new(stream)).await
    })
    .await?
    .map_err(|source| HttpCheckError::Protocol { source })?;
    // Drive the connection concurrently; it ends when the response completes.
    tokio::spawn(async move {
        let _ = conn.await;
    });

    let request = build_request(url, &host, port, target)?;

    // --- send + TTFB ---
    let t = Instant::now();
    let response = with_deadline(deadline, "request", sender.send_request(request))
        .await?
        .map_err(|source| HttpCheckError::Protocol { source })?;
    timings.ttfb = Some(t.elapsed());

    let status = response.status().as_u16();
    let headers: Vec<(String, String)> = response
        .headers()
        .iter()
        .map(|(n, v)| {
            (
                n.as_str().to_string(),
                v.to_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let location = response
        .headers()
        .get(hyper::header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    // --- download (bounded) ---
    let t = Instant::now();
    let (body, truncated) = read_body_capped(response.into_body(), deadline).await?;
    timings.download = Some(t.elapsed());

    Ok(Hop {
        status,
        timings,
        headers,
        body,
        truncated,
        location,
        cert_days_to_expiry: None,
    })
}

fn build_request(
    url: &Url,
    host: &str,
    port: u16,
    target: &HttpTarget,
) -> Result<Request<Full<Bytes>>, HttpCheckError> {
    let method = Method::from_bytes(target.method.as_bytes()).map_err(|_| {
        HttpCheckError::InvalidMethod {
            method: target.method.clone(),
        }
    })?;

    // Origin-form request target: path plus optional query.
    let mut path_and_query = url.path().to_string();
    if let Some(query) = url.query() {
        path_and_query.push('?');
        path_and_query.push_str(query);
    }

    // Host header includes the port only when non-default.
    let authority = if url.port().is_some() {
        format!("{host}:{port}")
    } else {
        host.to_string()
    };

    let mut builder = Request::builder()
        .method(method)
        .uri(path_and_query)
        .header(hyper::header::HOST, authority);
    for (name, value) in &target.headers {
        builder = builder.header(name, value);
    }

    let body = Bytes::from(target.body.clone().unwrap_or_default());
    builder
        .body(Full::new(body))
        .map_err(|source| HttpCheckError::Request { source })
}

/// Read the response body up to [`MAX_BODY_BYTES`], within the deadline, and
/// report whether it was truncated.
async fn read_body_capped(
    mut body: hyper::body::Incoming,
    deadline: Instant,
) -> Result<(Vec<u8>, bool), HttpCheckError> {
    let mut buf = Vec::new();
    let mut truncated = false;
    while let Some(frame) = with_deadline(deadline, "body", body.frame()).await? {
        let frame = frame.map_err(|source| HttpCheckError::Protocol { source })?;
        if let Some(chunk) = frame.data_ref() {
            let take = (MAX_BODY_BYTES - buf.len()).min(chunk.len());
            buf.extend_from_slice(&chunk[..take]);
            if buf.len() >= MAX_BODY_BYTES {
                truncated = chunk.len() > take || truncated;
                break;
            }
        }
    }
    Ok((buf, truncated))
}

/// Run `fut` with the time remaining until `deadline`; map expiry to a phase
/// timeout (V19).
async fn with_deadline<F: std::future::Future>(
    deadline: Instant,
    phase: &'static str,
    fut: F,
) -> Result<F::Output, HttpCheckError> {
    let now = Instant::now();
    if now >= deadline {
        return Err(HttpCheckError::Timeout { phase });
    }
    tokio::time::timeout(deadline - now, fut)
        .await
        .map_err(|_| HttpCheckError::Timeout { phase })
}

fn is_redirect(status: u16) -> bool {
    matches!(status, 301 | 302 | 303 | 307 | 308)
}
