//! Control-plane config source + heartbeat client (T20, T21).
//!
//! [`ControlPlaneConfigSource`] is the fleet implementation of
//! [`ConfigSource`]: it pulls the resolved checks
//! and the live worker-set from the control plane's worker API, keeps only the
//! checks this worker owns by rendezvous hashing (V26, V33), and maps them to
//! the flat runtime [`EndpointConfig`]. It caches the last good result and, if
//! the control plane is unreachable, keeps running the cached config rather than
//! stopping (V29). [`ControlPlaneConfigSource::heartbeat`] registers this worker
//! with the control plane (T20).

use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::config::{
    Config, EndpointConfig, HttpTarget, Target, TcpTarget, WsTarget, DEFAULT_MAX_CONCURRENCY,
};
use crate::sharding::{self, Worker};
use crate::source::{ConfigSource, ConfigSourceError};
use crate::wire::{ChecksResponse, HeartbeatRequest, ResolvedCheck, TargetSpec, VersionResponse};

/// A [`ConfigSource`] backed by the control plane's worker API.
pub struct ControlPlaneConfigSource {
    client: reqwest::Client,
    base_url: String,
    token: String,
    me: Worker,
    max_concurrency: usize,
    cache: Mutex<Option<CachedState>>,
    /// Optional on-disk mirror of the last good config, so a freshly started
    /// worker can begin serving during a control-plane outage (V29, #14).
    cache_path: Option<PathBuf>,
}

/// The serializable subset of [`CachedState`] written to disk. The mapped config
/// is rebuilt from these on load, so only wire-serializable data is persisted.
#[derive(Serialize, Deserialize)]
struct PersistedState {
    version: u64,
    raw_checks: Vec<ResolvedCheck>,
    fleet: Vec<Worker>,
}

/// The last successful poll's state, retained so a poll can be skipped when the
/// config version is unchanged (#8) and so the last good config survives a
/// control-plane outage (V29). The raw resolved checks are kept so ownership can
/// be recomputed against a changed fleet without re-pulling `/checks`.
#[derive(Clone)]
struct CachedState {
    version: u64,
    raw_checks: Vec<ResolvedCheck>,
    fleet: Vec<Worker>,
    config: Config,
}

/// Minimal view of a worker from the control plane's `/workers` response.
#[derive(Debug, Deserialize)]
struct FleetWorker {
    id: String,
    region: String,
}

/// Default per-request timeout for calls to the control plane. Bounds how long a
/// slow or dead control plane can stall a heartbeat or reload; kept well under
/// the worker's default poll/heartbeat intervals so a hung request cannot pile up
/// across cycles or block graceful shutdown (RELIABILITY).
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Build an HTTP client that gives up on a request (or connect) after `timeout`
/// rather than waiting forever. Falls back to an untimed default client only if
/// the builder somehow rejects the configuration (never expected).
fn build_client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

impl ControlPlaneConfigSource {
    /// Create a source pointing at `base_url` (e.g. `http://cp:8080`), presenting
    /// `token` as this worker's credential, identified by `worker_id`/`region`.
    pub fn new(
        base_url: impl Into<String>,
        token: impl Into<String>,
        worker_id: impl Into<String>,
        region: impl Into<String>,
    ) -> Self {
        Self {
            client: build_client(DEFAULT_REQUEST_TIMEOUT),
            base_url: base_url.into(),
            token: token.into(),
            me: Worker::new(worker_id, region),
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            cache: Mutex::new(None),
            cache_path: None,
        }
    }

    /// Override the per-request timeout (default [`DEFAULT_REQUEST_TIMEOUT`]).
    /// Builder-style so existing call sites are unaffected.
    #[must_use]
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.client = build_client(timeout);
        self
    }

    /// Persist each good config to `path` and, on a cold start during a
    /// control-plane outage, load the last good config from it (#14).
    #[must_use]
    pub fn with_cache_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.cache_path = Some(path.into());
        self
    }

    /// Register/refresh this worker's heartbeat with the control plane (T20).
    pub async fn heartbeat(&self) -> Result<(), ConfigSourceError> {
        let body = HeartbeatRequest {
            id: self.me.id.clone(),
            region: self.me.region.clone(),
        };
        self.client
            .post(format!("{}/workers/heartbeat", self.base_url))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| ConfigSourceError::ControlPlane(e.to_string()))?;
        Ok(())
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> Result<T, ConfigSourceError> {
        self.client
            .get(format!("{}{path}", self.base_url))
            .bearer_auth(&self.token)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|e| ConfigSourceError::ControlPlane(e.to_string()))?
            .json::<T>()
            .await
            .map_err(|e| ConfigSourceError::ControlPlane(e.to_string()))
    }

    async fn fetch(&self) -> Result<Config, ConfigSourceError> {
        // Version-gate the poll (#8): a cheap /config/version check first, so the
        // full /checks payload is pulled only when the config actually changed.
        let version: VersionResponse = self.get_json("/config/version").await?;
        let fleet = self.fetch_fleet().await?;

        if let Some(state) = self.cached_state() {
            if state.version == version.version {
                if state.fleet == fleet {
                    // Nothing moved — reuse the mapped config verbatim.
                    return Ok(state.config);
                }
                // Config unchanged but the fleet changed: recompute ownership
                // from the retained raw checks, no /checks transfer needed.
                let config = self.build_config(&state.raw_checks, &fleet);
                self.store_state(version.version, state.raw_checks, fleet, config.clone());
                return Ok(config);
            }
        }

        // Version moved (or first poll): pull the resolved checks.
        let checks: ChecksResponse = self.get_json("/checks").await?;
        let raw = checks.checks;
        let config = self.build_config(&raw, &fleet);
        self.store_state(version.version, raw, fleet, config.clone());
        Ok(config)
    }

    /// Pull the live worker-set and make sure we are in it (our own heartbeat may
    /// not have propagated to the set we just fetched).
    async fn fetch_fleet(&self) -> Result<Vec<Worker>, ConfigSourceError> {
        let fleet_raw: Vec<FleetWorker> = self.get_json("/workers").await?;
        let mut fleet: Vec<Worker> = fleet_raw
            .into_iter()
            .map(|w| Worker::new(w.id, w.region))
            .collect();
        if !fleet.iter().any(|w| w.id == self.me.id) {
            fleet.push(self.me.clone());
        }
        Ok(fleet)
    }

    /// Map the resolved checks this worker owns (by rendezvous hashing over
    /// `fleet`) into the flat runtime [`Config`].
    fn build_config(&self, raw: &[ResolvedCheck], fleet: &[Worker]) -> Config {
        // No per-check locations yet; region filtering is a no-op until they are
        // carried on the wire.
        let no_locations: Vec<String> = Vec::new();
        let endpoints: Vec<EndpointConfig> = raw
            .iter()
            .filter(|c| sharding::owns(&self.me, &c.id, fleet, &no_locations))
            .cloned()
            .filter_map(to_endpoint)
            .collect();
        Config {
            max_concurrency: self.max_concurrency,
            endpoints,
        }
    }

    fn cached_state(&self) -> Option<CachedState> {
        match self.cache.lock() {
            Ok(g) => g.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    fn store_state(
        &self,
        version: u64,
        raw_checks: Vec<ResolvedCheck>,
        fleet: Vec<Worker>,
        config: Config,
    ) {
        self.persist(version, &raw_checks, &fleet);
        let state = CachedState {
            version,
            raw_checks,
            fleet,
            config,
        };
        match self.cache.lock() {
            Ok(mut g) => *g = Some(state),
            Err(poisoned) => *poisoned.into_inner() = Some(state),
        }
    }

    /// Best-effort write of the last good config to disk (#14). Failures are
    /// logged, not fatal — the daemon keeps running on the in-memory state.
    fn persist(&self, version: u64, raw_checks: &[ResolvedCheck], fleet: &[Worker]) {
        let Some(path) = &self.cache_path else {
            return;
        };
        let state = PersistedState {
            version,
            raw_checks: raw_checks.to_vec(),
            fleet: fleet.to_vec(),
        };
        // Write to a temp sibling then rename so a reader never sees a torn file.
        let tmp = path.with_extension("tmp");
        let result = serde_json::to_vec(&state)
            .map_err(|e| e.to_string())
            .and_then(|bytes| std::fs::write(&tmp, bytes).map_err(|e| e.to_string()))
            .and_then(|()| std::fs::rename(&tmp, path).map_err(|e| e.to_string()));
        if let Err(error) = result {
            tracing::warn!(%error, path = %path.display(), "failed to persist config cache");
        }
    }

    /// Load the last good config from disk, rebuilding the mapped config from the
    /// persisted raw checks + fleet. `None` if no cache path or the file is
    /// missing/unreadable/corrupt.
    fn load_from_disk(&self) -> Option<Config> {
        let path = self.cache_path.as_ref()?;
        let bytes = std::fs::read(path).ok()?;
        let state: PersistedState = serde_json::from_slice(&bytes).ok()?;
        let config = self.build_config(&state.raw_checks, &state.fleet);
        // Prime the in-memory cache so subsequent polls have a baseline.
        self.store_state(state.version, state.raw_checks, state.fleet, config.clone());
        Some(config)
    }
}

impl ConfigSource for ControlPlaneConfigSource {
    async fn load(&self) -> Result<Config, ConfigSourceError> {
        match self.fetch().await {
            Ok(config) => Ok(config),
            Err(error) => match self.cached_state() {
                // V29: keep running the last-known config on control-plane outage.
                Some(state) => {
                    tracing::warn!(%error, "control plane unreachable; using cached config");
                    Ok(state.config)
                }
                // Cold start during an outage: fall back to the on-disk cache so
                // a fresh worker can still begin serving (#14).
                None => match self.load_from_disk() {
                    Some(config) => {
                        tracing::warn!(
                            %error,
                            "control plane unreachable on startup; using on-disk config cache"
                        );
                        Ok(config)
                    }
                    None => Err(error),
                },
            },
        }
    }
}

/// Map a resolved wire check to the flat runtime [`EndpointConfig`]. Returns
/// `None` for a malformed target (e.g. an unparseable URL) so one bad check does
/// not fail the whole load.
fn to_endpoint(check: ResolvedCheck) -> Option<EndpointConfig> {
    let target = match check.target {
        TargetSpec::Http(http) => {
            let url = Url::parse(&http.url).ok()?;
            let mut headers: Vec<(String, String)> = http.headers.into_iter().collect();
            headers.sort();
            let method = http
                .method
                .map(|m| m.to_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            Target::Http(HttpTarget {
                url,
                method,
                headers,
                body: http.body,
                allow_insecure: http.allow_insecure,
            })
        }
        TargetSpec::Tcp(tcp) => Target::Tcp(TcpTarget {
            host: tcp.host,
            port: tcp.port,
        }),
        TargetSpec::Ws(ws) => {
            let url = Url::parse(&ws.url).ok()?;
            Target::Ws(WsTarget {
                url,
                allow_insecure: ws.allow_insecure,
            })
        }
    };
    Some(EndpointConfig {
        id: check.id,
        interval: check.interval,
        timeout: check.timeout,
        tags: check.tags,
        target,
        assertions: check.assertions,
        allow_internal: check.allow_internal,
        retries: check.retries,
        retry_backoff: check.retry_backoff,
    })
}
