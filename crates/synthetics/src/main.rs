//! Synthetics daemon entrypoint (T10).
//!
//! Wires a config source, a metric sink, and the scheduler together, then runs
//! until a shutdown signal, reloading config on SIGHUP without missing a check
//! (V3, V4). Reconfiguring live requires no restart.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, ValueEnum};
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
    /// Path to the TOML config file.
    #[arg(long, default_value = "synthetics.toml")]
    config: PathBuf,

    /// Address of the local Datadog agent's DogStatsD UDP socket.
    #[arg(long, default_value = "127.0.0.1:8125")]
    dogstatsd: SocketAddr,

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
    let source = FileConfigSource::new(&args.config);

    // Initial load is fatal on error — the daemon cannot start with no config.
    let initial = source.load().await?;
    let max_concurrency = initial.max_concurrency;
    let endpoints = initial.endpoints;

    let sink = Arc::new(DogStatsdSink::udp(args.dogstatsd).await?);
    let mut scheduler = Scheduler::new(sink, max_concurrency);
    scheduler.reconcile(endpoints.into_iter().map(Arc::new).collect());

    tracing::info!(
        config = %args.config.display(),
        dogstatsd = %args.dogstatsd,
        endpoints = scheduler.running_ids().len(),
        "synthetics daemon started",
    );

    let worker = Worker::new(source, scheduler);

    // Signal wiring: SIGHUP -> reload, SIGTERM/SIGINT -> shutdown.
    let (reload_tx, reload_rx) = mpsc::unbounded_channel();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    spawn_signal_handlers(reload_tx, shutdown_tx)?;

    worker.run(reload_rx, shutdown_rx).await;
    tracing::info!("synthetics daemon stopped");
    Ok(())
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
