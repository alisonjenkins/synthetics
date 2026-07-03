//! synthetics — high-performance synthetic monitoring daemon.
//!
//! Runs many HTTP/TCP synthetic checks on independent schedules, records full
//! Datadog-parity signals per check, and pushes them through a swappable
//! [`MetricSink`]. Hot-reloadable (SIGHUP) without missing a scheduled check;
//! one unhealthy target never affects the checks of healthy targets.
//!
//! See `SPEC.md` and `docs/adr/` for the invariants and decisions behind this
//! crate.

/// Crate/service name. Surfaced in diagnostics and used as a default metric
/// tag source.
pub const NAME: &str = "synthetics";

#[cfg(test)]
mod tests {
    #[test]
    fn crate_name_is_synthetics() {
        assert_eq!(crate::NAME, "synthetics");
    }
}
