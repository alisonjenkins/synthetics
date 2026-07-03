//! Structured logging setup (T12).
//!
//! All diagnostics go through `tracing` (V14): no `println!`/`eprintln!` on
//! production paths. This module installs the subscriber (I.log). JSON is the
//! default for log collectors; a human-readable format is available for local
//! dev. The level is controlled by `RUST_LOG` (via `EnvFilter`), defaulting to
//! `info`.

use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;

/// Output format for logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    /// Machine-parseable JSON, one object per event (default for collectors).
    Json,
    /// Human-readable text, for local development.
    Pretty,
}

/// Install the global `tracing` subscriber in the chosen format. Idempotent: a
/// second call is a no-op (the first subscriber wins), so tests and repeated
/// setup do not panic.
pub fn init(format: LogFormat) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let builder = fmt::Subscriber::builder().with_env_filter(filter);
    match format {
        LogFormat::Json => {
            let _ = builder.json().try_init();
        }
        LogFormat::Pretty => {
            let _ = builder.try_init();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn init_is_idempotent() {
        // A second install must be a no-op, not a panic.
        init(LogFormat::Json);
        init(LogFormat::Json);
    }
}
