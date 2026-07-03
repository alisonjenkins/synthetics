//! Worker supervisor (T8).
//!
//! Ties a [`ConfigSource`] to a [`Scheduler`]: loads config and reconciles the
//! running checks, on demand (reload) and on shutdown. A reload re-loads the
//! source and reconciles (V3/V4); if the new config is invalid the reload fails
//! and the previous config keeps running untouched (V11) — a bad config never
//! disrupts a running fleet. The reload trigger is a channel so the mechanism is
//! testable; the daemon (T10) feeds SIGHUP into it (I.ctl).

use std::sync::Arc;

use tokio::sync::{mpsc, watch};

use crate::metric::MetricSink;
use crate::scheduler::Scheduler;
use crate::source::{ConfigSource, ConfigSourceError};

/// Supervises a scheduler from a config source.
pub struct Worker<S, M> {
    source: S,
    scheduler: Scheduler<M>,
}

impl<S: ConfigSource, M: MetricSink + 'static> Worker<S, M> {
    /// Create a worker driving `scheduler` from `source`.
    pub fn new(source: S, scheduler: Scheduler<M>) -> Self {
        Self { source, scheduler }
    }

    /// Load the current config and reconcile the scheduler to it. On error the
    /// scheduler is left untouched — the previous config keeps running (V11).
    pub async fn reload(&mut self) -> Result<(), ConfigSourceError> {
        let config = self.source.load().await?;
        let endpoints = config.endpoints.into_iter().map(Arc::new).collect();
        self.scheduler.reconcile(endpoints);
        Ok(())
    }

    /// Ids of the currently running endpoints (delegates to the scheduler).
    pub fn running_ids(&self) -> Vec<String> {
        self.scheduler.running_ids()
    }

    /// Run until `shutdown` flips to `true`, reloading whenever a `()` arrives on
    /// `reload`. A failed reload is logged and ignored, keeping the running
    /// config (V11). Assumes an initial [`reload`](Worker::reload) has succeeded.
    pub async fn run(
        mut self,
        mut reload: mpsc::UnboundedReceiver<()>,
        mut shutdown: watch::Receiver<bool>,
    ) {
        loop {
            tokio::select! {
                maybe = reload.recv() => {
                    match maybe {
                        Some(()) => {
                            if let Err(error) = self.reload().await {
                                tracing::warn!(
                                    %error,
                                    "config reload failed; keeping previous config",
                                );
                            } else {
                                tracing::info!(
                                    endpoints = self.scheduler.running_ids().len(),
                                    "config reloaded",
                                );
                            }
                        }
                        None => break, // reload channel closed
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
        self.scheduler.shutdown().await;
    }
}
