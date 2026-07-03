//! Config validation tests (T2). Every bad config must be rejected wholesale
//! with a precise, endpoint-named error (V11, V1). None must parse into a
//! partially-applied `Config`.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use synthetics::config::{
    load_config, parse_config, ConfigLoadError, ConfigParseError, ConfigValidateError,
};

/// Assert a config string is rejected at the validation stage with `want`.
fn reject(text: &str) -> ConfigValidateError {
    match parse_config(text) {
        Err(ConfigParseError::Invalid(e)) => e,
        Err(ConfigParseError::Toml(e)) => panic!("expected validation error, got TOML error: {e}"),
        Ok(_) => panic!("expected config to be rejected, but it parsed"),
    }
}

#[test]
fn rejects_duplicate_ids() {
    let text = r#"
[[endpoint]]
id = "dup"
  [endpoint.tcp]
  host = "a"
  port = 1

[[endpoint]]
id = "dup"
  [endpoint.tcp]
  host = "b"
  port = 2
"#;
    assert_eq!(
        reject(text),
        ConfigValidateError::DuplicateId { id: "dup".into() }
    );
}

#[test]
fn rejects_empty_id() {
    let text = r#"
[[endpoint]]
id = ""
  [endpoint.tcp]
  host = "a"
  port = 1
"#;
    assert_eq!(reject(text), ConfigValidateError::EmptyId);
}

#[test]
fn rejects_endpoint_with_no_target() {
    let text = r#"
[[endpoint]]
id = "lonely"
"#;
    assert_eq!(
        reject(text),
        ConfigValidateError::NoTarget {
            id: "lonely".into()
        }
    );
}

#[test]
fn rejects_endpoint_with_two_targets() {
    let text = r#"
[[endpoint]]
id = "both"
  [endpoint.http]
  url = "https://x/"
  [endpoint.tcp]
  host = "a"
  port = 1
"#;
    assert_eq!(
        reject(text),
        ConfigValidateError::MultipleTargets { id: "both".into() }
    );
}

#[test]
fn rejects_zero_interval() {
    let text = r#"
[[endpoint]]
id = "z"
interval = "0s"
  [endpoint.tcp]
  host = "a"
  port = 1
"#;
    assert_eq!(
        reject(text),
        ConfigValidateError::ZeroInterval { id: "z".into() }
    );
}

#[test]
fn rejects_non_http_scheme() {
    let text = r#"
[[endpoint]]
id = "ftp"
  [endpoint.http]
  url = "ftp://example.com/"
"#;
    assert_eq!(
        reject(text),
        ConfigValidateError::UnsupportedScheme {
            id: "ftp".into(),
            scheme: "ftp".into()
        }
    );
}

#[test]
fn rejects_zero_port() {
    let text = r#"
[[endpoint]]
id = "p"
  [endpoint.tcp]
  host = "a"
  port = 0
"#;
    assert_eq!(
        reject(text),
        ConfigValidateError::ZeroPort { id: "p".into() }
    );
}

#[test]
fn rejects_status_assertion_with_both_matchers() {
    let text = r#"
[[endpoint]]
id = "s"
  [endpoint.http]
  url = "https://x/"
  [[endpoint.assert]]
  type = "status"
  equals = 200
  in_range = [200, 299]
"#;
    assert_eq!(
        reject(text),
        ConfigValidateError::BadStatusAssertion { id: "s".into() }
    );
}

#[test]
fn rejects_inverted_status_range() {
    let text = r#"
[[endpoint]]
id = "s"
  [endpoint.http]
  url = "https://x/"
  [[endpoint.assert]]
  type = "status"
  in_range = [299, 200]
"#;
    assert_eq!(
        reject(text),
        ConfigValidateError::InvertedStatusRange {
            id: "s".into(),
            min: 299,
            max: 200
        }
    );
}

#[test]
fn rejects_header_assertion_without_matcher() {
    let text = r#"
[[endpoint]]
id = "h"
  [endpoint.http]
  url = "https://x/"
  [[endpoint.assert]]
  type = "header"
  name = "content-type"
"#;
    assert_eq!(
        reject(text),
        ConfigValidateError::BadHeaderAssertion {
            id: "h".into(),
            name: "content-type".into()
        }
    );
}

#[test]
fn rejects_invalid_body_regex() {
    let text = r#"
[[endpoint]]
id = "r"
  [endpoint.http]
  url = "https://x/"
  [[endpoint.assert]]
  type = "body_regex"
  pattern = "("
"#;
    assert!(matches!(reject(text), ConfigValidateError::InvalidRegex { id, .. } if id == "r"));
}

#[test]
fn rejects_bad_sha256() {
    let text = r#"
[[endpoint]]
id = "sha"
  [endpoint.http]
  url = "https://x/"
  [[endpoint.assert]]
  type = "body_sha256"
  hex = "not-hex"
"#;
    assert!(matches!(reject(text), ConfigValidateError::InvalidSha256 { id, .. } if id == "sha"));
}

#[test]
fn rejects_unknown_field_as_toml_error() {
    // deny_unknown_fields: a typo'd key is a hard error, not silently ignored.
    let text = r#"
[[endpoint]]
id = "typo"
intervl = "10s"
  [endpoint.tcp]
  host = "a"
  port = 1
"#;
    assert!(matches!(parse_config(text), Err(ConfigParseError::Toml(_))));
}

#[test]
fn empty_config_is_valid_with_no_endpoints() {
    let cfg = parse_config("").expect("empty config is valid");
    assert!(cfg.endpoints.is_empty());
}

#[test]
fn load_config_reports_read_error_for_missing_file() {
    let err = load_config("/no/such/synthetics/config.toml").unwrap_err();
    assert!(matches!(err, ConfigLoadError::Read { .. }));
}

#[test]
fn load_config_reads_and_parses_file() {
    let dir = std::env::temp_dir().join("synthetics-test-config");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ok.toml");
    std::fs::write(
        &path,
        r#"
[[endpoint]]
id = "file-ep"
  [endpoint.tcp]
  host = "a"
  port = 9
"#,
    )
    .unwrap();

    let cfg = load_config(&path).expect("file config should load");
    assert_eq!(cfg.endpoints.len(), 1);
    assert_eq!(cfg.endpoints[0].id, "file-ep");
}
