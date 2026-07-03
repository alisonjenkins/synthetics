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

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{watch, Semaphore};
use tokio::time::Instant;

use crate::config::{EndpointConfig, Target};
use crate::metric::{
    CheckDetail, CheckReport, FailureKind, HttpDetail, MetricSink, Outcome, TcpDetail,
};

/// Schedules and runs checks for a set of endpoints, emitting each result to a
/// [`MetricSink`].
pub struct Scheduler<M> {
    sink: Arc<M>,
    concurrency: Arc<Semaphore>,
    epoch: Instant,
}

impl<M: MetricSink + 'static> Scheduler<M> {
    /// Create a scheduler emitting to `sink`, with at most `max_concurrency`
    /// checks in flight at once (ADR 0006).
    pub fn new(sink: Arc<M>, max_concurrency: usize) -> Self {
        Self {
            sink,
            concurrency: Arc::new(Semaphore::new(max_concurrency.max(1))),
            epoch: Instant::now(),
        }
    }

    /// Run every endpoint on its interval until `shutdown` flips to `true`.
    /// Returns once all per-endpoint loops have stopped. In-flight checks are
    /// allowed to finish.
    pub async fn run(self, endpoints: Vec<Arc<EndpointConfig>>, shutdown: watch::Receiver<bool>) {
        let mut handles = Vec::with_capacity(endpoints.len());
        for endpoint in endpoints {
            let sink = Arc::clone(&self.sink);
            let sem = Arc::clone(&self.concurrency);
            let epoch = self.epoch;
            let shutdown = shutdown.clone();
            handles.push(tokio::spawn(endpoint_loop(
                endpoint, sink, sem, epoch, shutdown,
            )));
        }
        for handle in handles {
            let _ = handle.await;
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
        let _ = sink.record(&report).await;
        drop(permit);
    }
}

/// Run one endpoint's check in a spawned task and convert a panic into a failure
/// report (V21) so the scheduler and other endpoints are unaffected.
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
