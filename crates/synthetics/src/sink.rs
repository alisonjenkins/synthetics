//! Concrete [`MetricSink`](crate::metric::MetricSink) backends.
//!
//! Each backend translates the neutral [`CheckReport`](crate::metric::CheckReport)
//! into its own wire format. The rest of the program depends only on the trait
//! (V2), so backends are swappable here without touching check or scheduler code.

pub mod dogstatsd;
