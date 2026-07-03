//! Assertion engine (T14).
//!
//! Evaluates the typed assertions from [`Assertion`](crate::config::Assertion)
//! against a check's result, producing one [`AssertionOutcome`] per assertion
//! (V18). Each outcome carries a low-cardinality id naming *which* assertion it
//! was (V22) — never body content. Body scans use a scalar search here; the
//! SIMD path (T9) replaces the hot `body_contains` scan later behind the same
//! behavior (V6).

use std::sync::Mutex;
use std::time::Duration;

use sha2::{Digest, Sha256};

use crate::config::Assertion;
use crate::metric::AssertionOutcome;

/// The data an assertion is evaluated against. Assembled from a check outcome
/// (HTTP or TCP). Fields absent for a protocol are `None`/empty.
#[derive(Debug, Clone, Copy)]
pub struct AssertContext<'a> {
    /// Final HTTP status code, if a response was received.
    pub status: Option<u16>,
    /// Response headers (name as sent by the peer).
    pub headers: &'a [(String, String)],
    /// Response body bytes (possibly truncated upstream).
    pub body: &'a [u8],
    /// Total response time (drives `latency` assertions).
    pub response_time: Duration,
    /// Days until the served TLS certificate expires, if HTTPS.
    pub cert_days_to_expiry: Option<i64>,
}

/// Evaluate every assertion against `ctx`, in order.
pub fn evaluate(assertions: &[Assertion], ctx: &AssertContext) -> Vec<AssertionOutcome> {
    assertions
        .iter()
        .map(|assertion| AssertionOutcome {
            id: assertion_id(assertion),
            passed: passes(assertion, ctx),
        })
        .collect()
}

/// Whether every outcome passed. An empty set passes (nothing to violate).
pub fn all_passed(outcomes: &[AssertionOutcome]) -> bool {
    outcomes.iter().all(|o| o.passed)
}

/// Low-cardinality identifier of an assertion (V22): the type, plus the header
/// name for header assertions (config-defined, so bounded).
fn assertion_id(assertion: &Assertion) -> String {
    match assertion {
        Assertion::Status { .. } => "status".to_string(),
        Assertion::Header { name, .. } => format!("header:{}", name.to_lowercase()),
        Assertion::Latency { .. } => "latency".to_string(),
        Assertion::CertMinAge { .. } => "cert_min_age".to_string(),
        Assertion::BodyContains { .. } => "body_contains".to_string(),
        Assertion::BodyNotContains { .. } => "body_not_contains".to_string(),
        Assertion::BodyRegex { .. } => "body_regex".to_string(),
        Assertion::BodySha256 { .. } => "body_sha256".to_string(),
    }
}

fn passes(assertion: &Assertion, ctx: &AssertContext) -> bool {
    match assertion {
        Assertion::Status { equals, in_range } => match ctx.status {
            None => false,
            Some(status) => {
                let eq_ok = equals.map(|e| status == e).unwrap_or(true);
                let range_ok = in_range
                    .map(|[min, max]| status >= min && status <= max)
                    .unwrap_or(true);
                eq_ok && range_ok
            }
        },
        Assertion::Header {
            name,
            equals,
            contains,
        } => match find_header(ctx.headers, name) {
            None => false,
            Some(value) => {
                let eq_ok = equals.as_ref().map(|e| value == e).unwrap_or(true);
                let contains_ok = contains.as_ref().map(|c| value.contains(c)).unwrap_or(true);
                eq_ok && contains_ok
            }
        },
        Assertion::Latency { max } => ctx.response_time <= *max,
        Assertion::CertMinAge { days } => ctx
            .cert_days_to_expiry
            .map(|d| d >= i64::from(*days))
            .unwrap_or(false),
        Assertion::BodyContains { value } => crate::simd::contains(ctx.body, value.as_bytes()),
        Assertion::BodyNotContains { value } => !crate::simd::contains(ctx.body, value.as_bytes()),
        Assertion::BodyRegex { pattern } => match compiled_regex(pattern) {
            Some(re) => re.is_match(ctx.body),
            None => false,
        },
        Assertion::BodySha256 { hex } => sha256_hex(ctx.body).eq_ignore_ascii_case(hex),
    }
}

fn find_header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((byte & 0x0f) as u32, 16).unwrap_or('0'));
    }
    out
}

/// Process-wide cache of compiled body-regex patterns, so a pattern is compiled
/// once rather than on every check (V15). Patterns are validated at config load
/// (V11), so compilation here should succeed.
fn compiled_regex(pattern: &str) -> Option<regex::bytes::Regex> {
    static CACHE: Mutex<Option<std::collections::HashMap<String, regex::bytes::Regex>>> =
        Mutex::new(None);
    let mut guard = match CACHE.lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let map = guard.get_or_insert_with(std::collections::HashMap::new);
    if let Some(re) = map.get(pattern) {
        return Some(re.clone());
    }
    let re = regex::bytes::Regex::new(pattern).ok()?;
    map.insert(pattern.to_string(), re.clone());
    Some(re)
}
