//! Local mock Datadog server (T29): receive DogStatsD on UDP, serve a viewer on
//! HTTP. For local/test only — nothing is forwarded to real Datadog (V39).

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;
use mock_datadog::{router, Capture};
use tokio::net::{TcpListener, UdpSocket};
use tracing_subscriber::filter::EnvFilter;

/// Mock Datadog: DogStatsD receiver + web viewer.
#[derive(Debug, Parser)]
#[command(name = "mock-datadog", version, about)]
struct Args {
    /// UDP address to receive DogStatsD on.
    #[arg(long, env = "DSD_LISTEN", default_value = "0.0.0.0:8125")]
    dogstatsd: SocketAddr,

    /// HTTP address to serve the viewer on.
    #[arg(long, env = "HTTP_LISTEN", default_value = "0.0.0.0:8080")]
    http: SocketAddr,

    /// Maximum number of recent metrics to retain.
    #[arg(long, default_value_t = 5000)]
    capacity: usize,
}

#[tokio::main]
async fn main() -> ExitCode {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .try_init();

    match run(Args::parse()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "mock datadog exited with error");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    let capture = Arc::new(Capture::new(args.capacity));

    let socket = UdpSocket::bind(args.dogstatsd).await?;
    {
        let capture = Arc::clone(&capture);
        tokio::spawn(async move {
            let mut buf = vec![0u8; 65535];
            loop {
                match socket.recv_from(&mut buf).await {
                    Ok((n, _)) => {
                        if let Ok(text) = std::str::from_utf8(&buf[..n]) {
                            capture.ingest(text);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, "udp receive error");
                        break;
                    }
                }
            }
        });
    }

    let listener = TcpListener::bind(args.http).await?;
    tracing::info!(dogstatsd = %args.dogstatsd, http = %args.http, "mock datadog started");
    axum::serve(listener, router(capture))
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    tracing::info!("mock datadog stopped");
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
}
