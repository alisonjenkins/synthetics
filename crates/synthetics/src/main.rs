//! Synthetics daemon entrypoint (T10, T31).
//!
//! Wires a config source, a metric sink, and the scheduler together, then runs
//! until a shutdown signal, reloading config without missing a check (V3, V4).
//! Two modes:
//!   * **file** (default) — config from a local TOML, reload on SIGHUP;
//!   * **control-plane** (`--control-plane <url>`) — pull owned checks from the
//!     control plane, heartbeat membership, and poll for config changes.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, ValueEnum};
use synthetics::control_plane::ControlPlaneConfigSource;
use synthetics::scheduler::Scheduler;
use synthetics::sink::dogstatsd::DogStatsdSink;
use synthetics::source::{ConfigSource, FileConfigSource};
use synthetics::telemetry::{self, LogFormat};
use synthetics::worker::Worker;
use tokio::signal::unix::{signal, SignalKind};
use tokio::sync::{mpsc, watch};

/// High-performance synthetic monitoring daemon (HTTP/TCP checks → DogStatsD).
#[derive(Debug, Parser)]
#[command(name = "synthetics", version, about)]
struct Args {
    /// Path to the TOML config file (file mode).
    #[arg(long, default_value = "synthetics.toml")]
    config: PathBuf,

    /// DogStatsD target as `host:port` (resolved at startup; a name is fine).
    #[arg(long, default_value = "127.0.0.1:8125")]
    dogstatsd: String,

    /// Control-plane base URL. When set, run in control-plane mode.
    #[arg(long)]
    control_plane: Option<String>,

    /// This worker's id (control-plane mode).
    #[arg(long, env = "HOSTNAME", default_value = "worker")]
    worker_id: String,

    /// This worker's region (control-plane mode).
    #[arg(long, default_value = "default")]
    region: String,

    /// Worker credential presented to the control plane (control-plane mode).
    #[arg(long, env = "CP_TOKEN", default_value = "")]
    cp_token: String,

    /// Seconds between config polls (control-plane mode).
    #[arg(long, default_value_t = 15)]
    poll_secs: u64,

    /// Seconds between heartbeats (control-plane mode).
    #[arg(long, default_value_t = 10)]
    heartbeat_secs: u64,

    /// Log output format.
    #[arg(long, value_enum, default_value_t = LogFmt::Json)]
    log_format: LogFmt,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogFmt {
    Json,
    Pretty,
}

impl From<LogFmt> for LogFormat {
    fn from(value: LogFmt) -> Self {
        match value {
            LogFmt::Json => LogFormat::Json,
            LogFmt::Pretty => LogFormat::Pretty,
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    telemetry::init(args.log_format.into());

    match run(args).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "daemon exited with error");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let dogstatsd = resolve_dogstatsd(&args.dogstatsd).await;
    let sink = Arc::new(DogStatsdSink::udp(dogstatsd).await?);

    let (reload_tx, reload_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    spawn_signal_handlers(reload_tx.clone(), shutdown_tx)?;

    match &args.control_plane {
        Some(url) => {
            let source = Arc::new(ControlPlaneConfigSource::new(
                url.clone(),
                args.cp_token.clone(),
                args.worker_id.clone(),
                args.region.clone(),
            ));
            spawn_heartbeat(
                Arc::clone(&source),
                Duration::from_secs(args.heartbeat_secs),
            );
            spawn_poll(reload_tx, Duration::from_secs(args.poll_secs));
            tracing::info!(
                control_plane = %url,
                worker_id = %args.worker_id,
                region = %args.region,
                "synthetics daemon starting (control-plane mode)",
            );
            serve(source, sink, reload_rx, shutdown_rx).await
        }
        None => {
            tracing::info!(config = %args.config.display(), "synthetics daemon starting (file mode)");
            serve(
                FileConfigSource::new(&args.config),
                sink,
                reload_rx,
                shutdown_rx,
            )
            .await
        }
    }
}

/// Load the initial config, start the scheduler, and run the worker until
/// shutdown. Generic over the source so both modes share one path.
async fn serve<S: ConfigSource + 'static>(
    source: S,
    sink: Arc<DogStatsdSink>,
    reload_rx: mpsc::UnboundedReceiver<()>,
    shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Initial load is fatal — the daemon cannot start with no config.
    let initial = source.load().await?;
    let mut scheduler = Scheduler::new(sink, initial.max_concurrency);
    scheduler.reconcile(initial.endpoints.into_iter().map(Arc::new).collect());
    tracing::info!(
        endpoints = scheduler.running_ids().len(),
        "synthetics daemon started"
    );

    let worker = Worker::new(source, scheduler);
    worker.run(reload_rx, shutdown_rx).await;
    tracing::info!("synthetics daemon stopped");
    Ok(())
}

/// Resolve the DogStatsD `host:port` target to an address. If it does not
/// resolve (e.g. the local mock/agent is not up yet), fall back to a loopback
/// discard address so the sink still exists and drops gracefully (V13).
async fn resolve_dogstatsd(target: &str) -> SocketAddr {
    match tokio::net::lookup_host(target)
        .await
        .ok()
        .and_then(|mut a| a.next())
    {
        Some(addr) => addr,
        None => {
            tracing::warn!(
                target,
                "could not resolve DogStatsD target; metrics will be dropped"
            );
            SocketAddr::from(([127, 0, 0, 1], 9))
        }
    }
}

fn spawn_heartbeat(source: Arc<ControlPlaneConfigSource>, period: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(period);
        loop {
            ticker.tick().await;
            if let Err(error) = source.heartbeat().await {
                tracing::warn!(%error, "heartbeat to control plane failed");
            }
        }
    });
}

fn spawn_poll(reload_tx: mpsc::UnboundedSender<()>, period: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(period);
        ticker.tick().await; // consume the immediate first tick
        loop {
            ticker.tick().await;
            if reload_tx.send(()).is_err() {
                break;
            }
        }
    });
}

fn spawn_signal_handlers(
    reload_tx: mpsc::UnboundedSender<()>,
    shutdown_tx: watch::Sender<bool>,
) -> Result<(), std::io::Error> {
    let mut sighup = signal(SignalKind::hangup())?;
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = sighup.recv() => {
                    tracing::info!("received SIGHUP; reloading config");
                    if reload_tx.send(()).is_err() {
                        break;
                    }
                }
                _ = sigterm.recv() => {
                    tracing::info!("received SIGTERM; shutting down");
                    let _ = shutdown_tx.send(true);
                    break;
                }
                _ = sigint.recv() => {
                    tracing::info!("received SIGINT; shutting down");
                    let _ = shutdown_tx.send(true);
                    break;
                }
            }
        }
    });

    Ok(())
}
