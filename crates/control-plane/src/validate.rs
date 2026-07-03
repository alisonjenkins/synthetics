//! Write validation (T24, V37).
//!
//! The control plane re-validates every write authoritatively — it never trusts
//! the client (V37). Errors are field-scoped so the caller (and, via the API,
//! the Terraform provider) can point at the exact problem. These are the same
//! rules the provider should mirror at plan time (the shared rule set of ADR
//! 0023).

use synthetics::config::Assertion;

use crate::model::{Endpoint, Node, TargetSpec};

/// A validation failure naming the offending field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{field}: {message}")]
pub struct FieldError {
    /// Dotted path to the offending field (e.g. `target.url`).
    pub field: String,
    /// Human-readable, actionable message.
    pub message: String,
}

fn err(field: &str, message: &str) -> FieldError {
    FieldError {
        field: field.to_string(),
        message: message.to_string(),
    }
}

/// Validate a hierarchy node.
pub fn validate_node(node: &Node) -> Result<(), FieldError> {
    if node.id.trim().is_empty() {
        return Err(err("id", "must not be empty"));
    }
    if node.kind.trim().is_empty() {
        return Err(err("kind", "must not be empty"));
    }
    if node.name.trim().is_empty() {
        return Err(err("name", "must not be empty"));
    }
    Ok(())
}

/// Validate an endpoint (target well-formed, assertions well-formed).
pub fn validate_endpoint(endpoint: &Endpoint) -> Result<(), FieldError> {
    if endpoint.id.trim().is_empty() {
        return Err(err("id", "must not be empty"));
    }
    if endpoint.environment.trim().is_empty() {
        return Err(err("environment", "must not be empty"));
    }
    match &endpoint.target {
        TargetSpec::Http(http) => {
            let url = url::Url::parse(&http.url)
                .map_err(|e| err("target.url", &format!("invalid url: {e}")))?;
            if url.scheme() != "http" && url.scheme() != "https" {
                return Err(err("target.url", "scheme must be http or https"));
            }
        }
        TargetSpec::Tcp(tcp) => {
            if tcp.host.trim().is_empty() {
                return Err(err("target.host", "must not be empty"));
            }
            if tcp.port == 0 {
                return Err(err("target.port", "must be non-zero"));
            }
        }
    }
    for (i, assertion) in endpoint.assertions.iter().enumerate() {
        validate_assertion(i, assertion)?;
    }
    Ok(())
}

fn validate_assertion(index: usize, assertion: &Assertion) -> Result<(), FieldError> {
    let field = |suffix: &str| format!("assertions[{index}].{suffix}");
    match assertion {
        Assertion::Status { equals, in_range } => match (equals, in_range) {
            (Some(_), None) | (None, Some(_)) => {
                if let Some([min, max]) = in_range {
                    if min > max {
                        return Err(err(&field("in_range"), "min must be <= max"));
                    }
                }
            }
            _ => {
                return Err(err(
                    &field("type"),
                    "status assertion must set exactly one of equals/in_range",
                ))
            }
        },
        Assertion::Header {
            name,
            equals,
            contains,
        } => {
            if name.trim().is_empty() {
                return Err(err(&field("name"), "must not be empty"));
            }
            if equals.is_none() && contains.is_none() {
                return Err(err(
                    &field("type"),
                    "header assertion needs equals or contains",
                ));
            }
        }
        Assertion::BodyRegex { pattern } => {
            regex::Regex::new(pattern)
                .map_err(|e| err(&field("pattern"), &format!("invalid regex: {e}")))?;
        }
        Assertion::BodySha256 { hex } => {
            let ok = hex.len() == 64
                && hex
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
            if !ok {
                return Err(err(&field("hex"), "must be 64 lowercase hex chars"));
            }
        }
        Assertion::Latency { .. }
        | Assertion::CertMinAge { .. }
        | Assertion::BodyContains { .. }
        | Assertion::BodyNotContains { .. } => {}
    }
    Ok(())
}
