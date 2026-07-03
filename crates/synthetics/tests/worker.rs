//! Worker reload tests (T8): reload picks up config changes; a bad config is
//! rejected and the previous config keeps running (V11); an unchanged endpoint
//! keeps firing across a reload with no gap (V3). Cites I.config, I.ctl, V3, V11.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use synthetics::metric::MockSink;
use synthetics::scheduler::Scheduler;
use synthetics::source::FileConfigSource;
use synthetics::worker::Worker;
use tokio::net::TcpListener;

fn write(path: &PathBuf, contents: &str) {
    std::fs::write(path, contents).unwrap();
}

fn unique_config(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join("synthetics-worker-test");
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[tokio::test]
async fn reload_picks_up_added_and_removed_endpoints() {
    let path = unique_config("reload-diff.toml");
    write(
        &path,
        r#"
[[endpoint]]
id = "a"
  [endpoint.tcp]
  host = "127.0.0.1"
  port = 9
"#,
    );

    let sink = Arc::new(MockSink::new());
    let scheduler = Scheduler::new(sink, 16);
    let mut worker = Worker::new(FileConfigSource::new(&path), scheduler);

    worker.reload().await.expect("initial load");
    assert_eq!(worker.running_ids(), vec!["a".to_string()]);

    // Rewrite: drop a, add b.
    write(
        &path,
        r#"
[[endpoint]]
id = "b"
  [endpoint.tcp]
  host = "127.0.0.1"
  port = 9
"#,
    );
    worker.reload().await.expect("reload");
    assert_eq!(worker.running_ids(), vec!["b".to_string()]);
}

#[tokio::test]
async fn failed_reload_keeps_previous_config() {
    let path = unique_config("reload-bad.toml");
    write(
        &path,
        r#"
[[endpoint]]
id = "a"
  [endpoint.tcp]
  host = "127.0.0.1"
  port = 9
"#,
    );

    let sink = Arc::new(MockSink::new());
    let scheduler = Scheduler::new(sink, 16);
    let mut worker = Worker::new(FileConfigSource::new(&path), scheduler);
    worker.reload().await.expect("initial load");
    assert_eq!(worker.running_ids(), vec!["a".to_string()]);

    // Invalid config: duplicate-free but broken (zero port).
    write(
        &path,
        r#"
[[endpoint]]
id = "a"
  [endpoint.tcp]
  host = "127.0.0.1"
  port = 0
"#,
    );
    let err = worker.reload().await;
    assert!(err.is_err(), "bad config should be rejected");
    // Previous config still running (V11).
    assert_eq!(worker.running_ids(), vec!["a".to_string()]);
}

#[tokio::test]
async fn unchanged_endpoint_keeps_firing_across_reload() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let port = addr.port();

    let path = unique_config("reload-continuity.toml");
    let cfg_a = format!(
        "[[endpoint]]\nid = \"a\"\ninterval = \"40ms\"\ntimeout = \"1s\"\n  [endpoint.tcp]\n  host = \"127.0.0.1\"\n  port = {port}\n"
    );
    write(&path, &cfg_a);

    let sink = Arc::new(MockSink::new());
    let scheduler = Scheduler::new(Arc::clone(&sink), 16);
    let mut worker = Worker::new(FileConfigSource::new(&path), scheduler);
    worker.reload().await.expect("initial load");

    tokio::time::sleep(Duration::from_millis(150)).await;
    let before = sink
        .reports()
        .iter()
        .filter(|r| r.endpoint_id == "a")
        .count();
    assert!(before >= 1, "endpoint a should have fired before reload");

    // Reload adding b, keeping a unchanged.
    let cfg_ab = format!(
        "{cfg_a}\n[[endpoint]]\nid = \"b\"\ninterval = \"40ms\"\ntimeout = \"1s\"\n  [endpoint.tcp]\n  host = \"127.0.0.1\"\n  port = {port}\n"
    );
    write(&path, &cfg_ab);
    worker.reload().await.expect("reload");
    assert_eq!(worker.running_ids(), vec!["a".to_string(), "b".to_string()]);

    tokio::time::sleep(Duration::from_millis(150)).await;
    let after = sink
        .reports()
        .iter()
        .filter(|r| r.endpoint_id == "a")
        .count();
    // a kept firing across the reload — its count grew, no reset/gap (V3).
    assert!(
        after > before,
        "endpoint a kept firing: {before} -> {after}"
    );

    worker.reload().await.ok();
}
