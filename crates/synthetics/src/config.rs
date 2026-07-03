//! Configuration model, parsing, and validation (T2).
//!
//! Cites: I.config (reloadable endpoint list + intervals + params + assertions),
//! V1 (each fallible fn has its own `thiserror` enum), V11 (a bad config is
//! rejected wholesale — never partially applied). The endpoint [`EndpointConfig::id`]
//! is the stable key used by reload diffing (ADR 0002).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use url::Url;

/// Fallback interval when neither the endpoint nor `[defaults]` sets one.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(60);
/// Fallback per-check timeout when neither the endpoint nor `[defaults]` sets one.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
/// Fallback concurrency cap (ADR 0006) when `[defaults]` does not set one.
pub const DEFAULT_MAX_CONCURRENCY: usize = 256;

/// A fully validated configuration ready for the scheduler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// Global concurrency cap across all in-flight checks (ADR 0006, V8).
    pub max_concurrency: usize,
    /// Endpoints to check, in declared order.
    pub endpoints: Vec<EndpointConfig>,
}

/// One endpoint under test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointConfig {
    /// Stable identity; unique across the config. Reload diff key (ADR 0002).
    pub id: String,
    /// How often this endpoint is checked (V5).
    pub interval: Duration,
    /// Hard per-check deadline (V19).
    pub timeout: Duration,
    /// Low-cardinality metric tags applied to this endpoint's reports.
    pub tags: Vec<String>,
    /// What to check.
    pub target: Target,
    /// Assertions evaluated each check; any failure = check fail (V18).
    pub assertions: Vec<Assertion>,
}

/// The protocol-specific target of a check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// An HTTP(S) endpoint.
    Http(HttpTarget),
    /// A raw TCP endpoint.
    Tcp(TcpTarget),
}

/// An HTTP(S) request to issue and time (I.http).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpTarget {
    /// Target URL (`http`/`https`).
    pub url: Url,
    /// Upper-cased HTTP method; defaults to `GET`.
    pub method: String,
    /// Request headers, sorted by name for deterministic ordering.
    pub headers: Vec<(String, String)>,
    /// Optional request body.
    pub body: Option<String>,
    /// Skip TLS certificate verification (for internal endpoints with
    /// self-signed certs). Cert expiry is still recorded. Default `false`.
    pub allow_insecure: bool,
}

/// A TCP endpoint whose connect is timed (I.tcp).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpTarget {
    /// Hostname or IP to connect to.
    pub host: String,
    /// TCP port to connect to.
    pub port: u16,
}

/// A per-endpoint assertion (I.assert). Body assertions are evaluated in-daemon
/// only; the body never leaves the process (V22). `body_regex` / `body_sha256`
/// hold their spec verbatim and are compiled/checked at evaluation time (T14);
/// validation here proves they are well-formed (V11).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Assertion {
    /// Assert on the HTTP status code. Exactly one of `equals`/`in_range` is set.
    Status {
        /// Require an exact status code.
        #[serde(default)]
        equals: Option<u16>,
        /// Require the status within an inclusive `[min, max]` range.
        #[serde(default)]
        in_range: Option<[u16; 2]>,
    },
    /// Assert on a response header. At least one of `equals`/`contains` is set.
    Header {
        /// Header name (matched case-insensitively).
        name: String,
        /// Require the header value to equal this string.
        #[serde(default)]
        equals: Option<String>,
        /// Require the header value to contain this substring.
        #[serde(default)]
        contains: Option<String>,
    },
    /// Assert the total response time is at most `max`.
    Latency {
        /// Maximum acceptable response time.
        #[serde(with = "humantime_serde")]
        max: Duration,
    },
    /// Assert the served TLS certificate has at least `days` until expiry.
    CertMinAge {
        /// Minimum acceptable days-to-expiry.
        days: u32,
    },
    /// Assert the body contains this substring (SIMD-scanned, in-daemon; V22).
    BodyContains {
        /// Substring the body must contain.
        value: String,
    },
    /// Assert the body does not contain this substring.
    BodyNotContains {
        /// Substring the body must not contain.
        value: String,
    },
    /// Assert the body matches this regular expression.
    BodyRegex {
        /// Regex pattern the body must match.
        pattern: String,
    },
    /// Assert the body's SHA-256 fingerprint equals this hex digest.
    BodySha256 {
        /// Expected lower-case hex SHA-256 of the body.
        hex: String,
    },
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Errors from [`parse_config`].
#[derive(Debug, thiserror::Error)]
pub enum ConfigParseError {
    /// The text was not valid TOML or did not match the schema.
    #[error("config is not valid TOML: {0}")]
    Toml(#[from] toml::de::Error),
    /// The config parsed but failed validation.
    #[error(transparent)]
    Invalid(#[from] ConfigValidateError),
}

/// Errors from [`load_config`].
// Variant fields are documented by their `#[error]` messages.
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error)]
pub enum ConfigLoadError {
    /// The config file could not be read.
    #[error("failed to read config file {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// The file was read but its contents were rejected.
    #[error(transparent)]
    Parse(#[from] ConfigParseError),
}

/// Errors from validating a parsed config (V11). Each variant names the
/// endpoint at fault so operators can find it.
// Variant fields are documented by their `#[error]` messages.
#[allow(missing_docs)]
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ConfigValidateError {
    #[error("endpoint has an empty id")]
    EmptyId,
    #[error("duplicate endpoint id {id:?}")]
    DuplicateId { id: String },
    #[error("endpoint {id:?} defines no target (expected [endpoint.http] or [endpoint.tcp])")]
    NoTarget { id: String },
    #[error("endpoint {id:?} defines more than one target (only one of http/tcp allowed)")]
    MultipleTargets { id: String },
    #[error("endpoint {id:?} has a zero interval")]
    ZeroInterval { id: String },
    #[error("endpoint {id:?} has a zero timeout")]
    ZeroTimeout { id: String },
    #[error("endpoint {id:?} http url scheme {scheme:?} is not http or https")]
    UnsupportedScheme { id: String, scheme: String },
    #[error("endpoint {id:?} tcp host is empty")]
    EmptyHost { id: String },
    #[error("endpoint {id:?} tcp port is zero")]
    ZeroPort { id: String },
    #[error("endpoint {id:?} status assertion must set exactly one of `equals` or `in_range`")]
    BadStatusAssertion { id: String },
    #[error("endpoint {id:?} status assertion range {min}..{max} is inverted")]
    InvertedStatusRange { id: String, min: u16, max: u16 },
    #[error("endpoint {id:?} header assertion {name:?} sets neither `equals` nor `contains`")]
    BadHeaderAssertion { id: String, name: String },
    #[error("endpoint {id:?} body_regex is invalid: {source}")]
    InvalidRegex {
        id: String,
        #[source]
        source: Box<regex::Error>,
    },
    #[error("endpoint {id:?} body_sha256 {hex:?} is not 64 lowercase hex chars")]
    InvalidSha256 { id: String, hex: String },
}

/// Parse and validate a config from a TOML string.
///
/// Returns the whole config or an error; never a partially applied config (V11).
pub fn parse_config(text: &str) -> Result<Config, ConfigParseError> {
    let raw: raw::RawConfig = toml::from_str(text)?;
    let cfg = build_and_validate(raw)?;
    Ok(cfg)
}

/// Read, parse, and validate a config file.
pub fn load_config(path: impl AsRef<Path>) -> Result<Config, ConfigLoadError> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path).map_err(|source| ConfigLoadError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    let cfg = parse_config(&text)?;
    Ok(cfg)
}

// ---------------------------------------------------------------------------
// Raw (deserialize) layer -> validated domain model
// ---------------------------------------------------------------------------

mod raw {
    use super::*;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct RawConfig {
        #[serde(default)]
        pub defaults: Defaults,
        #[serde(default, rename = "endpoint")]
        pub endpoints: Vec<RawEndpoint>,
    }

    #[derive(Deserialize, Default)]
    #[serde(deny_unknown_fields)]
    pub(super) struct Defaults {
        #[serde(default, with = "humantime_serde::option")]
        pub interval: Option<Duration>,
        #[serde(default, with = "humantime_serde::option")]
        pub timeout: Option<Duration>,
        #[serde(default)]
        pub max_concurrency: Option<usize>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct RawEndpoint {
        pub id: String,
        #[serde(default, with = "humantime_serde::option")]
        pub interval: Option<Duration>,
        #[serde(default, with = "humantime_serde::option")]
        pub timeout: Option<Duration>,
        #[serde(default)]
        pub tags: Vec<String>,
        pub http: Option<RawHttp>,
        pub tcp: Option<RawTcp>,
        #[serde(default, rename = "assert")]
        pub assertions: Vec<Assertion>,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct RawHttp {
        pub url: Url,
        #[serde(default)]
        pub method: Option<String>,
        #[serde(default)]
        pub headers: BTreeMap<String, String>,
        #[serde(default)]
        pub body: Option<String>,
        #[serde(default)]
        pub allow_insecure: bool,
    }

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    pub(super) struct RawTcp {
        pub host: String,
        pub port: u16,
    }
}

fn build_and_validate(raw: raw::RawConfig) -> Result<Config, ConfigValidateError> {
    let default_interval = raw.defaults.interval.unwrap_or(DEFAULT_INTERVAL);
    let default_timeout = raw.defaults.timeout.unwrap_or(DEFAULT_TIMEOUT);
    let max_concurrency = raw
        .defaults
        .max_concurrency
        .unwrap_or(DEFAULT_MAX_CONCURRENCY);

    let mut seen_ids: BTreeMap<String, ()> = BTreeMap::new();
    let mut endpoints = Vec::with_capacity(raw.endpoints.len());

    for ep in raw.endpoints {
        if ep.id.is_empty() {
            return Err(ConfigValidateError::EmptyId);
        }
        if seen_ids.insert(ep.id.clone(), ()).is_some() {
            return Err(ConfigValidateError::DuplicateId { id: ep.id });
        }

        let target = build_target(&ep.id, ep.http, ep.tcp)?;

        let interval = ep.interval.unwrap_or(default_interval);
        if interval.is_zero() {
            return Err(ConfigValidateError::ZeroInterval { id: ep.id });
        }
        let timeout = ep.timeout.unwrap_or(default_timeout);
        if timeout.is_zero() {
            return Err(ConfigValidateError::ZeroTimeout { id: ep.id });
        }

        validate_assertions(&ep.id, &ep.assertions)?;

        endpoints.push(EndpointConfig {
            id: ep.id,
            interval,
            timeout,
            tags: ep.tags,
            target,
            assertions: ep.assertions,
        });
    }

    Ok(Config {
        max_concurrency,
        endpoints,
    })
}

fn build_target(
    id: &str,
    http: Option<raw::RawHttp>,
    tcp: Option<raw::RawTcp>,
) -> Result<Target, ConfigValidateError> {
    match (http, tcp) {
        (Some(_), Some(_)) => Err(ConfigValidateError::MultipleTargets { id: id.to_string() }),
        (None, None) => Err(ConfigValidateError::NoTarget { id: id.to_string() }),
        (Some(http), None) => {
            let scheme = http.url.scheme();
            if scheme != "http" && scheme != "https" {
                return Err(ConfigValidateError::UnsupportedScheme {
                    id: id.to_string(),
                    scheme: scheme.to_string(),
                });
            }
            let mut headers: Vec<(String, String)> = http.headers.into_iter().collect();
            headers.sort();
            let method = http
                .method
                .map(|m| m.to_uppercase())
                .unwrap_or_else(|| "GET".to_string());
            Ok(Target::Http(HttpTarget {
                url: http.url,
                method,
                headers,
                body: http.body,
                allow_insecure: http.allow_insecure,
            }))
        }
        (None, Some(tcp)) => {
            if tcp.host.is_empty() {
                return Err(ConfigValidateError::EmptyHost { id: id.to_string() });
            }
            if tcp.port == 0 {
                return Err(ConfigValidateError::ZeroPort { id: id.to_string() });
            }
            Ok(Target::Tcp(TcpTarget {
                host: tcp.host,
                port: tcp.port,
            }))
        }
    }
}

fn validate_assertions(id: &str, assertions: &[Assertion]) -> Result<(), ConfigValidateError> {
    for assertion in assertions {
        match assertion {
            Assertion::Status { equals, in_range } => match (equals, in_range) {
                (Some(_), None) => {}
                (None, Some([min, max])) => {
                    if min > max {
                        return Err(ConfigValidateError::InvertedStatusRange {
                            id: id.to_string(),
                            min: *min,
                            max: *max,
                        });
                    }
                }
                _ => {
                    return Err(ConfigValidateError::BadStatusAssertion { id: id.to_string() });
                }
            },
            Assertion::Header {
                name,
                equals,
                contains,
            } => {
                if equals.is_none() && contains.is_none() {
                    return Err(ConfigValidateError::BadHeaderAssertion {
                        id: id.to_string(),
                        name: name.clone(),
                    });
                }
            }
            Assertion::BodyRegex { pattern } => {
                if let Err(source) = regex::Regex::new(pattern) {
                    return Err(ConfigValidateError::InvalidRegex {
                        id: id.to_string(),
                        source: Box::new(source),
                    });
                }
            }
            Assertion::BodySha256 { hex } => {
                if !is_sha256_hex(hex) {
                    return Err(ConfigValidateError::InvalidSha256 {
                        id: id.to_string(),
                        hex: hex.clone(),
                    });
                }
            }
            Assertion::Latency { .. }
            | Assertion::CertMinAge { .. }
            | Assertion::BodyContains { .. }
            | Assertion::BodyNotContains { .. } => {}
        }
    }
    Ok(())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
