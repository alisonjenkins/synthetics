//! Check scheduler (T7).
//!
//! Drives each endpoint on its own drift-free interval (V5) as an independent
//! task, so one slow, hung, or dead target never delays another's schedule
//! (V20, ADR 0006/0011). A global [`Semaphore`] bounds how many checks run at
//! once (V8); because each check self-bounds with a timeout (V19), permits are
//! always released promptly, so dead targets cannot starve healthy ones (V20).
//! Each check runs in a spawned task whose panic is caught and turned into a
//! failure report — the daemon and all other checks survive (V21). Static
//! dispatch throughout: generic over `M: MetricSink`, no `dyn` (V35).

use std::collections::HashMap;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{watch, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::config::{EndpointConfig, Target};
use crate::metric::{
    CheckDetail, CheckReport, FailureKind, HttpDetail, MetricSink, Outcome, TcpDetail, WsDetail,
};

/// A running per-endpoint loop and the state needed to stop or diff it.
struct Running {
    config: Arc<EndpointConfig>,
    stop: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

/// Schedules and runs checks for a live, reconfigurable set of endpoints,
/// emitting each result to a [`MetricSink`]. Each endpoint runs as an
/// independent loop; [`reconcile`](Scheduler::reconcile) adds, removes, and
/// restarts loops to match a desired set without disturbing unchanged endpoints
/// (V3).
pub struct Scheduler<M> {
    sink: Arc<M>,
    concurrency: Arc<Semaphore>,
    epoch: Instant,
    running: HashMap<String, Running>,
}

impl<M: MetricSink + 'static> Scheduler<M> {
    /// Create a scheduler emitting to `sink`, with at most `max_concurrency`
    /// checks in flight at once (ADR 0006). Starts with no endpoints.
    pub fn new(sink: Arc<M>, max_concurrency: usize) -> Self {
        Self {
            sink,
            concurrency: Arc::new(Semaphore::new(max_concurrency.max(1))),
            epoch: Instant::now(),
            running: HashMap::new(),
        }
    }

    /// Reconcile the running loops to `desired` (V3, V4): start loops for added
    /// endpoints, stop loops for removed ones, restart loops whose config
    /// changed, and leave unchanged endpoints running untouched — so their
    /// schedule continues with no missed or doubled tick across a reload.
    pub fn reconcile(&mut self, desired: Vec<Arc<EndpointConfig>>) {
        use std::collections::HashSet;
        let desired_ids: HashSet<&str> = desired.iter().map(|e| e.id.as_str()).collect();

        // Stop endpoints no longer desired.
        let stale: Vec<String> = self
            .running
            .keys()
            .filter(|id| !desired_ids.contains(id.as_str()))
            .cloned()
            .collect();
        for id in stale {
            if let Some(running) = self.running.remove(&id) {
                let _ = running.stop.send(true);
            }
        }

        // Add new endpoints and restart changed ones; leave unchanged as-is.
        for endpoint in desired {
            match self.running.get(&endpoint.id) {
                Some(running) if *running.config == *endpoint => {
                    // Unchanged: keep its loop and its next-fire time (V3).
                }
                _ => {
                    if let Some(old) = self.running.remove(&endpoint.id) {
                        let _ = old.stop.send(true);
                    }
                    self.start(endpoint);
                }
            }
        }
    }

    fn start(&mut self, endpoint: Arc<EndpointConfig>) {
        let (stop, rx) = watch::channel(false);
        let handle = tokio::spawn(endpoint_loop(
            Arc::clone(&endpoint),
            Arc::clone(&self.sink),
            Arc::clone(&self.concurrency),
            self.epoch,
            rx,
        ));
        let id = endpoint.id.clone();
        self.running.insert(
            id,
            Running {
                config: endpoint,
                stop,
                handle,
            },
        );
    }

    /// The ids of currently running endpoints, sorted (for inspection/tests).
    pub fn running_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.running.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Stop every endpoint loop and wait for them to exit. In-flight checks are
    /// allowed to finish.
    pub async fn shutdown(mut self) {
        for (_, running) in self.running.drain() {
            let _ = running.stop.send(true);
            let _ = running.handle.await;
        }
    }
}

async fn endpoint_loop<M: MetricSink + 'static>(
    endpoint: Arc<EndpointConfig>,
    sink: Arc<M>,
    concurrency: Arc<Semaphore>,
    epoch: Instant,
    mut shutdown: watch::Receiver<bool>,
) {
    let interval = endpoint.interval;
    // Fire times are aligned to the epoch so per-check latency does not
    // accumulate into schedule drift (V5).
    let mut next = epoch + interval;

    loop {
        // Skip any missed slots (overrun) so a slow endpoint catches up to the
        // next future boundary rather than firing a burst.
        let now = Instant::now();
        while next <= now {
            next += interval;
        }

        tokio::select! {
            _ = tokio::time::sleep_until(next) => {}
            _ = shutdown.changed() => {}
        }
        if *shutdown.borrow() {
            return;
        }
        next += interval;

        // Bounded concurrency: wait for a permit (or shutdown). One endpoint
        // holds at most one permit at a time — the loop awaits the check before
        // looping, so a slow target never occupies multiple permits (V20).
        let permit = tokio::select! {
            p = Semaphore::acquire_owned(Arc::clone(&concurrency)) => p,
            _ = shutdown.changed() => {
                if *shutdown.borrow() { return; } else { continue; }
            }
        };
        let permit = match permit {
            Ok(permit) => permit,
            Err(_closed) => return,
        };

        let report = run_check_isolated(Arc::clone(&endpoint)).await;
        tracing::info!(
            endpoint = %report.endpoint_id,
            outcome = ?report.outcome,
            failure = ?report.failure,
            latency_ms = report.duration.as_secs_f64() * 1000.0,
            "check completed",
        );
        let _ = sink.record(&report).await;
        drop(permit);
    }
}

/// Run one endpoint's check in a spawned task and convert a panic into a failure
/// report (V21) so the scheduler and other endpoints are unaffected. Runs inside
/// a span carrying the endpoint id (V14).
#[tracing::instrument(skip_all, fields(endpoint = %endpoint.id))]
async fn run_check_isolated(endpoint: Arc<EndpointConfig>) -> CheckReport {
    let id = endpoint.id.clone();
    let tags = endpoint.tags.clone();
    let detail = empty_detail(&endpoint.target);
    let fut = async move { crate::check::run_endpoint(&endpoint).await };
    isolate(id, tags, detail, fut).await
}

/// Await `fut` in an isolated task; if it panics, synthesize a
/// [`FailureKind::Panic`] report from the endpoint identity.
async fn isolate(
    endpoint_id: String,
    tags: Vec<String>,
    detail: CheckDetail,
    fut: impl Future<Output = CheckReport> + Send + 'static,
) -> CheckReport {
    match tokio::spawn(fut).await {
        Ok(report) => report,
        Err(_join_error) => CheckReport {
            endpoint_id,
            tags,
            outcome: Outcome::Failure,
            duration: Duration::ZERO,
            detail,
            assertions: Vec::new(),
            failure: Some(FailureKind::Panic),
        },
    }
}

fn empty_detail(target: &Target) -> CheckDetail {
    match target {
        Target::Http(_) => CheckDetail::Http(HttpDetail::default()),
        Target::Tcp(_) => CheckDetail::Tcp(TcpDetail::default()),
        Target::Ws(_) => CheckDetail::Ws(WsDetail::default()),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[tokio::test]
    async fn isolate_converts_panic_to_failure_report() {
        let report = isolate(
            "boom".to_string(),
            vec!["env:test".to_string()],
            CheckDetail::Tcp(TcpDetail::default()),
            async {
                panic!("kaboom");
            },
        )
        .await;

        assert_eq!(report.endpoint_id, "boom");
        assert_eq!(report.outcome, Outcome::Failure);
        assert_eq!(report.failure, Some(FailureKind::Panic));
    }

    #[tokio::test]
    async fn isolate_passes_through_a_normal_report() {
        let expected = CheckReport {
            endpoint_id: "ok".to_string(),
            tags: vec![],
            outcome: Outcome::Success,
            duration: Duration::from_millis(1),
            detail: CheckDetail::Tcp(TcpDetail::default()),
            assertions: vec![],
            failure: None,
        };
        let clone = expected.clone();
        let report = isolate(
            "ok".to_string(),
            vec![],
            CheckDetail::Tcp(TcpDetail::default()),
            async move { clone },
        )
        .await;
        assert_eq!(report, expected);
    }
}
