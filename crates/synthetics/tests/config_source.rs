//! ConfigSource tests (T18): the FileConfigSource loads validated config from a
//! TOML file behind the trait the worker depends on (I.configsource, V24).

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use synthetics::source::{ConfigSource, FileConfigSource};

#[tokio::test]
async fn file_source_loads_validated_config() {
    let dir = std::env::temp_dir().join("synthetics-source-test");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("checks.toml");
    std::fs::write(
        &path,
        r#"
[[endpoint]]
id = "home"
  [endpoint.http]
  url = "https://example.com/"
"#,
    )
    .unwrap();

    let source = FileConfigSource::new(&path);
    let config = source.load().await.expect("should load");
    assert_eq!(config.endpoints.len(), 1);
    assert_eq!(config.endpoints[0].id, "home");
}

#[tokio::test]
async fn file_source_surfaces_load_error() {
    let source = FileConfigSource::new("/no/such/synthetics.toml");
    assert!(source.load().await.is_err());
}
