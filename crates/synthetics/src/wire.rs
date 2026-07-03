//! Control-plane ↔ worker wire types.
//!
//! These live in the worker crate so a worker can consume the control plane's
//! HTTP responses without depending on the control-plane crate (which depends on
//! this crate — the reverse dependency would be a cycle). The control plane
//! re-exports them, so both sides share one definition (the contract of T26).

use std::collections::BTreeMap;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::config::Assertion;

/// A fully resolved check ready to hand to a worker (hierarchy already merged,
/// V25). This is the control-plane → worker contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedCheck {
    /// Stable endpoint id (rendezvous + diff key).
    pub id: String,
    /// Effective check interval.
    #[serde(with = "humantime_serde")]
    pub interval: Duration,
    /// Effective per-check timeout.
    #[serde(with = "humantime_serde")]
    pub timeout: Duration,
    /// Effective tags (hierarchy auto-tags + inherited + endpoint tags).
    pub tags: Vec<String>,
    /// What to check.
    pub target: TargetSpec,
    /// Assertions to evaluate.
    pub assertions: Vec<Assertion>,
    /// Opt-in to reach an internal/private resolved address (SSRF guard, #5).
    /// The worker still requires the address to be in its operator allowlist.
    #[serde(default)]
    pub allow_internal: bool,
}

/// A protocol-specific target specification (as authored / resolved).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TargetSpec {
    /// An HTTP(S) target.
    Http(HttpSpec),
    /// A TCP target.
    Tcp(TcpSpec),
    /// A WebSocket target.
    Ws(WsSpec),
}

/// HTTP target specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpSpec {
    /// Target URL.
    pub url: String,
    /// HTTP method; defaults to `GET` when absent.
    #[serde(default)]
    pub method: Option<String>,
    /// Request headers.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// Optional request body.
    #[serde(default)]
    pub body: Option<String>,
    /// Skip TLS verification for self-signed internal endpoints.
    #[serde(default)]
    pub allow_insecure: bool,
}

/// TCP target specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TcpSpec {
    /// Hostname or IP to connect to.
    pub host: String,
    /// TCP port to connect to.
    pub port: u16,
}

/// WebSocket target specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WsSpec {
    /// Target URL (`ws`/`wss`).
    pub url: String,
    /// Skip TLS verification for self-signed internal `wss` endpoints.
    #[serde(default)]
    pub allow_insecure: bool,
}

/// The `/checks` response: resolved checks plus the version they belong to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecksResponse {
    /// Config version these checks correspond to.
    pub version: u64,
    /// Flat, resolved checks.
    pub checks: Vec<ResolvedCheck>,
}

/// The `/config/version` response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionResponse {
    /// Monotonic config version.
    pub version: u64,
}

/// A worker heartbeat/registration body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    /// Worker id.
    pub id: String,
    /// Worker region.
    pub region: String,
}
