//! Control-plane authoring model (T22).
//!
//! The config is authored over a hierarchy of [`Node`]s with a generic parent
//! reference (ADR 0017), so levels can be added above product (e.g. an
//! organization) without a schema change. Each [`Endpoint`] hangs off an
//! environment node and may override inherited defaults. Assertions reuse the
//! worker crate's [`Assertion`] type. Resolution of the hierarchy into flat
//! worker checks is a separate step (T23).

use std::time::Duration;

use serde::{Deserialize, Serialize};
use synthetics::config::Assertion;

/// A node in the config hierarchy (organization, product, customer, environment,
/// ...). The `kind` is free-form so the hierarchy can grow new levels.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    /// Stable unique id (e.g. a hierarchical path like `web/globex`).
    pub id: String,
    /// Node kind: `org`, `product`, `customer`, `environment`, ...
    pub kind: String,
    /// Human-readable name.
    pub name: String,
    /// Parent node id; `None` for a root.
    pub parent: Option<String>,
    /// Defaults inherited by descendants (endpoints may override).
    #[serde(default)]
    pub defaults: NodeDefaults,
}

/// Defaults set at a hierarchy level and inherited downward.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeDefaults {
    /// Default check interval for descendants.
    #[serde(default, with = "humantime_serde::option")]
    pub interval: Option<Duration>,
    /// Default per-check timeout for descendants.
    #[serde(default, with = "humantime_serde::option")]
    pub timeout: Option<Duration>,
    /// Tags contributed to every descendant endpoint's metrics.
    #[serde(default)]
    pub tags: Vec<String>,
}

/// An endpoint to check, hanging off an environment node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    /// Stable unique id — the hierarchical path (rendezvous + diff key, V32).
    pub id: String,
    /// Id of the environment node this endpoint belongs to.
    pub environment: String,
    /// Human-readable name.
    pub name: String,
    /// What to check.
    pub target: TargetSpec,
    /// Interval override (falls back to inherited defaults).
    #[serde(default, with = "humantime_serde::option")]
    pub interval: Option<Duration>,
    /// Timeout override (falls back to inherited defaults).
    #[serde(default, with = "humantime_serde::option")]
    pub timeout: Option<Duration>,
    /// Tags added on top of inherited tags.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Assertions evaluated each check.
    #[serde(default)]
    pub assertions: Vec<Assertion>,
    /// Opt-in to let the worker connect to an internal/private address for this
    /// endpoint (SSRF guard, #5). Still gated by the worker's operator allowlist.
    #[serde(default)]
    pub allow_internal: bool,
}

// Target specs are the CP↔worker wire types, defined once in the worker crate
// and re-exported here so both sides share one definition (T26 contract).
pub use synthetics::wire::{HttpSpec, TargetSpec, TcpSpec, WsSpec};

/// A registered worker and its last heartbeat (I.membership).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Worker {
    /// Unique worker id.
    pub id: String,
    /// Region the worker runs in (checks run only on matching-region workers).
    pub region: String,
    /// Last heartbeat, Unix milliseconds.
    pub last_heartbeat_ms: i64,
}
