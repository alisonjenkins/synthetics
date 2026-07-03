//! Config parsing + validation tests (T2). Drives the public `config` surface.
//! Cites: I.config, V1 (per-fn error enums), V11 (reject bad config wholesale).

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use synthetics::config::{parse_config, Assertion, Config, Target};

const MINIMAL_HTTP: &str = r#"
[[endpoint]]
id = "login-page"
interval = "15s"
timeout = "5s"

  [endpoint.http]
  url = "https://example.com/login"
"#;

#[test]
fn parses_minimal_http_endpoint() {
    let cfg: Config = parse_config(MINIMAL_HTTP).expect("valid config should parse");

    assert_eq!(cfg.endpoints.len(), 1);
    let ep = &cfg.endpoints[0];
    assert_eq!(ep.id, "login-page");
    assert_eq!(ep.interval, Duration::from_secs(15));
    assert_eq!(ep.timeout, Duration::from_secs(5));

    match &ep.target {
        Target::Http(http) => {
            assert_eq!(http.url.as_str(), "https://example.com/login");
            // Method defaults to GET when unspecified.
            assert_eq!(http.method, "GET");
        }
        other => panic!("expected HTTP target, got {other:?}"),
    }
}

#[test]
fn parses_tcp_endpoint_and_assertions() {
    let text = r#"
[[endpoint]]
id = "db-port"
interval = "30s"
timeout = "3s"

  [endpoint.tcp]
  host = "db.internal"
  port = 5432

[[endpoint]]
id = "home"
interval = "10s"
timeout = "5s"
tags = ["team:web", "env:prod"]

  [endpoint.http]
  url = "https://example.com/"

  [[endpoint.assert]]
  type = "status"
  equals = 200

  [[endpoint.assert]]
  type = "body_contains"
  value = "Sign in"

  [[endpoint.assert]]
  type = "body_not_contains"
  value = "Welcome to nginx"
"#;

    let cfg = parse_config(text).expect("valid config should parse");
    assert_eq!(cfg.endpoints.len(), 2);

    let db = &cfg.endpoints[0];
    match &db.target {
        Target::Tcp(tcp) => {
            assert_eq!(tcp.host, "db.internal");
            assert_eq!(tcp.port, 5432);
        }
        other => panic!("expected TCP target, got {other:?}"),
    }

    let home = &cfg.endpoints[1];
    assert_eq!(
        home.tags,
        vec!["team:web".to_string(), "env:prod".to_string()]
    );
    assert_eq!(home.assertions.len(), 3);
    assert!(matches!(
        home.assertions[0],
        Assertion::Status {
            equals: Some(200),
            ..
        }
    ));
    assert!(matches!(&home.assertions[1], Assertion::BodyContains { value } if value == "Sign in"));
    assert!(
        matches!(&home.assertions[2], Assertion::BodyNotContains { value } if value == "Welcome to nginx")
    );
}
