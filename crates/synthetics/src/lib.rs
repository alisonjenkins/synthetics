//! synthetics — high-performance synthetic monitoring daemon.
//!
//! Runs many HTTP/TCP synthetic checks on independent schedules, records full
//! Datadog-parity signals per check, and pushes them through a swappable
//! [`MetricSink`](metric::MetricSink). Hot-reloadable (SIGHUP) without missing a
//! scheduled check;
//! one unhealthy target never affects the checks of healthy targets.
//!
//! See `SPEC.md` and `docs/adr/` for the invariants and decisions behind this
//! crate.

/// Crate/service name. Surfaced in diagnostics and used as a default metric
/// tag source.
pub const NAME: &str = "synthetics";

pub mod assert;
pub mod check;
pub mod config;
pub mod control_plane;
pub mod metric;
pub mod scheduler;
pub mod sharding;
pub mod simd;
pub mod sink;
pub mod source;
pub mod telemetry;
pub mod wire;
pub mod worker;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_name_is_synthetics() {
        assert_eq!(crate::NAME, "synthetics");
    }
}
