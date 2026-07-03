//! Protocol check executors.
//!
//! Each executor performs one check against a target and returns a rich,
//! protocol-specific outcome (phase timings + response data) or a per-function
//! `thiserror` enum (V1). Turning an outcome into assertions (T14) and a
//! [`CheckReport`](crate::metric::CheckReport) happens in a later layer, so the
//! executors stay focused on "do the request, measure it, report what happened".

pub mod http;
pub mod tcp;
