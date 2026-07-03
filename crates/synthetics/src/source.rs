//! Config sources (T18).
//!
//! A [`ConfigSource`] is where the worker gets the flat, resolved set of checks
//! to run (I.configsource, V24). The worker runtime depends only on this trait
//! and behaves identically regardless of source. [`FileConfigSource`] reads the
//! local TOML config (dev, single-node, tests, air-gapped); the control-plane
//! source (fleet) lands later behind the same seam. Static dispatch (V35): the
//! trait is a native async fn (RPITIT), consumers are generic over `S`.

use std::future::Future;
use std::path::{Path, PathBuf};

use crate::config::{Config, ConfigLoadError};

/// Errors from [`ConfigSource::load`].
#[derive(Debug, thiserror::Error)]
pub enum ConfigSourceError {
    /// Loading from a file failed (read / parse / validate).
    #[error(transparent)]
    File(#[from] ConfigLoadError),
    /// A control-plane request failed (network, status, or decode).
    #[error("control plane request failed: {0}")]
    ControlPlane(String),
}

/// A source of validated worker configuration.
pub trait ConfigSource: Send + Sync {
    /// Load the current configuration. Returns the whole validated config or an
    /// error; never a partially-applied config (V11).
    fn load(&self) -> impl Future<Output = Result<Config, ConfigSourceError>> + Send;
}

/// Sharing a source (e.g. between the [`Worker`](crate::worker::Worker) and a
/// heartbeat loop) is just an `Arc` — it forwards to the inner source.
impl<T: ConfigSource> ConfigSource for std::sync::Arc<T> {
    fn load(&self) -> impl Future<Output = Result<Config, ConfigSourceError>> + Send {
        (**self).load()
    }
}

/// A [`ConfigSource`] backed by a local TOML file.
#[derive(Debug, Clone)]
pub struct FileConfigSource {
    path: PathBuf,
}

impl FileConfigSource {
    /// Create a file source reading `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The path this source reads.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ConfigSource for FileConfigSource {
    async fn load(&self) -> Result<Config, ConfigSourceError> {
        Ok(crate::config::load_config(&self.path)?)
    }
}
