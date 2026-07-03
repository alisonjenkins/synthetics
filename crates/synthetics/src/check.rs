//! Protocol check executors.
//!
//! Each executor performs one check against a target and returns a rich,
//! protocol-specific outcome (phase timings + response data) or a per-function
//! `thiserror` enum (V1). Turning an outcome into assertions (T14) and a
//! [`CheckReport`] happens in a later layer, so the
//! executors stay focused on "do the request, measure it, report what happened".

pub mod http;
pub mod tcp;
pub(crate) mod tls;
pub mod ws;

use crate::assert::{self, AssertContext};
use crate::config::{EndpointConfig, Target};
use crate::metric::{
    CheckDetail, CheckReport, FailureKind, HttpDetail, Outcome, TcpDetail, WsDetail,
};
use http::{HttpCheckError, HttpOutcome};
use tcp::TcpCheckError;
use ws::{WsCheckError, WsOutcome};

/// Default redirect cap applied to HTTP endpoints by the orchestrator.
use http::DEFAULT_MAX_REDIRECTS;

/// Run one endpoint's check end-to-end and produce a backend-neutral
/// [`CheckReport`] (T17): dispatch on the target protocol (exhaustive match — a
/// new protocol is a compiler-flagged variant, V36), evaluate assertions (V18),
/// and map success/failure/detail (V9). This is the unit the scheduler drives.
pub async fn run_endpoint(endpoint: &EndpointConfig) -> CheckReport {
    match &endpoint.target {
        Target::Http(target) => {
            let result =
                http::run_http_check(target, endpoint.timeout, DEFAULT_MAX_REDIRECTS).await;
            build_http_report(endpoint, result)
        }
        Target::Tcp(target) => {
            let result = tcp::run_tcp_check(target, endpoint.timeout).await;
            build_tcp_report(endpoint, result)
        }
        Target::Ws(target) => {
            let result = ws::run_ws_check(target, endpoint.timeout).await;
            build_ws_report(endpoint, result)
        }
    }
}

fn build_http_report(
    endpoint: &EndpointConfig,
    result: Result<HttpOutcome, HttpCheckError>,
) -> CheckReport {
    match result {
        Ok(outcome) => {
            let ctx = AssertContext {
                status: Some(outcome.status),
                headers: &outcome.headers,
                body: &outcome.body,
                response_time: outcome.timings.total,
                cert_days_to_expiry: outcome.cert_days_to_expiry,
            };
            let assertions = assert::evaluate(&endpoint.assertions, &ctx);
            let passed = assert::all_passed(&assertions);
            let detail = CheckDetail::Http(HttpDetail {
                status: Some(outcome.status),
                dns: outcome.timings.dns,
                connect: outcome.timings.connect,
                tls_handshake: outcome.timings.tls_handshake,
                ttfb: outcome.timings.ttfb,
                download: outcome.timings.download,
                response_bytes: Some(outcome.response_bytes),
                redirects: outcome.redirects,
                cert_days_to_expiry: outcome.cert_days_to_expiry,
            });
            CheckReport {
                endpoint_id: endpoint.id.clone(),
                tags: endpoint.tags.clone(),
                outcome: if passed {
                    Outcome::Success
                } else {
                    Outcome::Failure
                },
                duration: outcome.timings.total,
                detail,
                assertions,
                failure: (!passed).then_some(FailureKind::Assertion),
            }
        }
        Err(error) => CheckReport {
            endpoint_id: endpoint.id.clone(),
            tags: endpoint.tags.clone(),
            outcome: Outcome::Failure,
            duration: http_error_duration(&error),
            detail: CheckDetail::Http(HttpDetail::default()),
            assertions: Vec::new(),
            failure: Some(http_failure_kind(&error)),
        },
    }
}

fn build_tcp_report(
    endpoint: &EndpointConfig,
    result: Result<tcp::TcpOutcome, TcpCheckError>,
) -> CheckReport {
    match result {
        Ok(outcome) => {
            let ctx = AssertContext {
                status: None,
                headers: &[],
                body: &[],
                response_time: outcome.timings.total,
                cert_days_to_expiry: None,
            };
            let assertions = assert::evaluate(&endpoint.assertions, &ctx);
            let passed = assert::all_passed(&assertions);
            CheckReport {
                endpoint_id: endpoint.id.clone(),
                tags: endpoint.tags.clone(),
                outcome: if passed {
                    Outcome::Success
                } else {
                    Outcome::Failure
                },
                duration: outcome.timings.total,
                detail: CheckDetail::Tcp(TcpDetail {
                    connect: outcome.timings.connect,
                }),
                assertions,
                failure: (!passed).then_some(FailureKind::Assertion),
            }
        }
        Err(error) => CheckReport {
            endpoint_id: endpoint.id.clone(),
            tags: endpoint.tags.clone(),
            outcome: Outcome::Failure,
            duration: std::time::Duration::ZERO,
            detail: CheckDetail::Tcp(TcpDetail::default()),
            assertions: Vec::new(),
            failure: Some(tcp_failure_kind(&error)),
        },
    }
}

fn http_failure_kind(error: &HttpCheckError) -> FailureKind {
    match error {
        HttpCheckError::Timeout { .. } => FailureKind::Timeout,
        HttpCheckError::Dns { .. } | HttpCheckError::NoAddress { .. } => FailureKind::Dns,
        HttpCheckError::Connect { .. } => FailureKind::Connect,
        HttpCheckError::Tls { .. } | HttpCheckError::InvalidDnsName { .. } => FailureKind::Tls,
        HttpCheckError::MissingHost
        | HttpCheckError::UnsupportedScheme { .. }
        | HttpCheckError::InvalidMethod { .. }
        | HttpCheckError::Request { .. }
        | HttpCheckError::Protocol { .. }
        | HttpCheckError::BadRedirect { .. }
        | HttpCheckError::TooManyRedirects { .. } => FailureKind::Protocol,
    }
}

fn http_error_duration(_error: &HttpCheckError) -> std::time::Duration {
    // The executor does not surface partial timings on error; the scheduler
    // records wall-clock separately if needed.
    std::time::Duration::ZERO
}

fn tcp_failure_kind(error: &TcpCheckError) -> FailureKind {
    match error {
        TcpCheckError::Timeout { .. } => FailureKind::Timeout,
        TcpCheckError::Dns { .. } | TcpCheckError::NoAddress { .. } => FailureKind::Dns,
        TcpCheckError::Connect { .. } => FailureKind::Connect,
    }
}

fn build_ws_report(
    endpoint: &EndpointConfig,
    result: Result<WsOutcome, WsCheckError>,
) -> CheckReport {
    match result {
        Ok(outcome) => {
            let ctx = AssertContext {
                status: None,
                headers: &[],
                body: &[],
                response_time: outcome.timings.total,
                cert_days_to_expiry: outcome.cert_days_to_expiry,
            };
            let assertions = assert::evaluate(&endpoint.assertions, &ctx);
            let passed = assert::all_passed(&assertions);
            CheckReport {
                endpoint_id: endpoint.id.clone(),
                tags: endpoint.tags.clone(),
                outcome: if passed {
                    Outcome::Success
                } else {
                    Outcome::Failure
                },
                duration: outcome.timings.total,
                detail: CheckDetail::Ws(WsDetail {
                    dns: outcome.timings.dns,
                    connect: outcome.timings.connect,
                    tls_handshake: outcome.timings.tls_handshake,
                    handshake: outcome.timings.handshake,
                    cert_days_to_expiry: outcome.cert_days_to_expiry,
                }),
                assertions,
                failure: (!passed).then_some(FailureKind::Assertion),
            }
        }
        Err(error) => CheckReport {
            endpoint_id: endpoint.id.clone(),
            tags: endpoint.tags.clone(),
            outcome: Outcome::Failure,
            duration: std::time::Duration::ZERO,
            detail: CheckDetail::Ws(WsDetail::default()),
            assertions: Vec::new(),
            failure: Some(ws_failure_kind(&error)),
        },
    }
}

fn ws_failure_kind(error: &WsCheckError) -> FailureKind {
    match error {
        WsCheckError::Timeout { .. } => FailureKind::Timeout,
        WsCheckError::Dns { .. } | WsCheckError::NoAddress { .. } => FailureKind::Dns,
        WsCheckError::Connect { .. } => FailureKind::Connect,
        WsCheckError::Tls { .. } | WsCheckError::InvalidDnsName { .. } => FailureKind::Tls,
        WsCheckError::MissingHost
        | WsCheckError::UnsupportedScheme { .. }
        | WsCheckError::Handshake { .. } => FailureKind::Protocol,
    }
}
