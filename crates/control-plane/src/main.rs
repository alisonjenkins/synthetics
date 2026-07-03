//! Control-plane server binary (T30).
//!
//! Serves the writer + worker APIs backed by a Postgres [`PgStore`], with
//! graceful shutdown. Auth uses the simple concrete impls for now
//! (`StaticTokenAuth` for writers, `AcceptWorkerAuth` for workers); the OIDC/mTLS
//! verifiers are separate, later impls behind the same traits.

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, ValueEnum};
use synthetics::telemetry::{self, LogFormat};
use synthetics_cp::api::{worker_router, writer_router, AppState};
use synthetics_cp::auth::{AcceptWorkerAuth, StaticTokenAuth};
use synthetics_cp::PgStore;

/// Synthetics control plane: config hierarchy storage + resolution + API.
#[derive(Debug, Parser)]
#[command(name = "synthetics-cp", version, about)]
struct Args {
    /// Address to listen on.
    #[arg(long, env = "CP_LISTEN", default_value = "0.0.0.0:8080")]
    listen: SocketAddr,

    /// Postgres connection URL.
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Writer credential in the form `token:product` (repeatable).
    #[arg(long = "writer-token", value_name = "TOKEN:PRODUCT")]
    writer_tokens: Vec<String>,

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
            tracing::error!(%error, "control plane exited with error");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(PgStore::connect(&args.database_url).await?);

    let mut writer_auth = StaticTokenAuth::new();
    for spec in &args.writer_tokens {
        let (token, product) = spec
            .split_once(':')
            .ok_or("--writer-token must be TOKEN:PRODUCT")?;
        writer_auth = writer_auth.with(token, product);
    }

    let app = writer_router(AppState::new(Arc::clone(&store), Arc::new(writer_auth))).merge(
        worker_router(AppState::new(store, Arc::new(AcceptWorkerAuth))),
    );

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!(listen = %args.listen, "control plane started");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("control plane stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sig) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            sig.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("shutdown signal received");
}
