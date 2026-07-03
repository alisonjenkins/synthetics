//! End-to-end control-plane ↔ worker integration (T20, T21): the worker's
//! ControlPlaneConfigSource pulls resolved checks from a live CP worker API and
//! maps them to runtime config, heartbeats its membership, and survives a CP
//! outage by serving cached config (V29). Cites I.cp, V24, V29, V26.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use synthetics::config::Target;
use synthetics::control_plane::ControlPlaneConfigSource;
use synthetics::source::ConfigSource;
use synthetics_cp::api::{worker_router, AppState};
use synthetics_cp::auth::AcceptWorkerAuth;
use synthetics_cp::model::{Endpoint, Node, NodeDefaults, TargetSpec, TcpSpec};
use synthetics_cp::{ConfigStore, MemoryStore};
use tokio::sync::oneshot;

async fn seed(store: &MemoryStore) {
    store
        .upsert_node(Node {
            id: "web".into(),
            kind: "product".into(),
            name: "web".into(),
            parent: None,
            defaults: NodeDefaults {
                interval: Some(Duration::from_secs(30)),
                timeout: None,
                tags: vec![],
                retries: None,
                retry_backoff: None,
            },
        })
        .await
        .unwrap();
    store
        .upsert_node(Node {
            id: "web/prod".into(),
            kind: "environment".into(),
            name: "web-prod".into(),
            parent: Some("web".into()),
            defaults: NodeDefaults::default(),
        })
        .await
        .unwrap();
    store
        .upsert_endpoint(Endpoint {
            id: "web/prod/db".into(),
            environment: "web/prod".into(),
            name: "db".into(),
            target: TargetSpec::Tcp(TcpSpec {
                host: "db.internal".into(),
                port: 5432,
            }),
            interval: None,
            timeout: None,
            tags: vec![],
            assertions: vec![],
            allow_internal: false,
            retries: None,
            retry_backoff: None,
        })
        .await
        .unwrap();
}

/// Spawn the CP worker API on an ephemeral port. Returns the base URL, the
/// shared store, and a shutdown sender.
async fn spawn_cp() -> (String, Arc<MemoryStore>, oneshot::Sender<()>) {
    let store = Arc::new(MemoryStore::new());
    seed(&store).await;
    let app = worker_router(AppState::new(
        Arc::clone(&store),
        Arc::new(AcceptWorkerAuth),
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await;
    });
    (format!("http://{addr}"), store, tx)
}

#[tokio::test]
async fn pulls_and_maps_resolved_checks() {
    let (base, _store, _shutdown) = spawn_cp().await;
    let source = ControlPlaneConfigSource::new(base, "w1", "w1", "eu-west");

    let config = source.load().await.expect("should load from CP");
    assert_eq!(config.endpoints.len(), 1);
    let ep = &config.endpoints[0];
    assert_eq!(ep.id, "web/prod/db");
    // Interval inherited from the product node (V25).
    assert_eq!(ep.interval, Duration::from_secs(30));
    // Auto hierarchy tag present.
    assert!(ep.tags.iter().any(|t| t == "product:web"));
    match &ep.target {
        Target::Tcp(t) => {
            assert_eq!(t.host, "db.internal");
            assert_eq!(t.port, 5432);
        }
        other => panic!("expected tcp target, got {other:?}"),
    }
}

#[tokio::test]
async fn heartbeat_registers_worker() {
    let (base, store, _shutdown) = spawn_cp().await;
    let source = ControlPlaneConfigSource::new(base, "w1", "w1", "eu-west");

    source.heartbeat().await.expect("heartbeat should succeed");

    let live = store.list_live_workers(0).await.unwrap();
    assert_eq!(live.len(), 1);
    assert_eq!(live[0].id, "w1");
    assert_eq!(live[0].region, "eu-west");
}

#[tokio::test]
async fn fresh_worker_starts_from_disk_cache_during_cp_outage() {
    let (base, _store, shutdown) = spawn_cp().await;

    // A unique cache file (derive uniqueness from the ephemeral CP port so
    // parallel test runs don't collide).
    let port = base.rsplit(':').next().unwrap();
    let cache = std::env::temp_dir().join(format!("synthetics-cache-{port}.json"));
    let _ = std::fs::remove_file(&cache);

    // Worker A loads from the live CP, which writes the disk cache.
    let source_a = ControlPlaneConfigSource::new(base.clone(), "w1", "w1", "eu-west")
        .with_cache_path(cache.clone());
    let first = source_a.load().await.expect("initial load from live CP");
    assert_eq!(first.endpoints.len(), 1);
    assert!(cache.exists(), "load should have written the disk cache");

    // Take the CP down.
    let _ = shutdown.send(());
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Worker B is brand new (empty in-memory cache) and the CP is unreachable.
    // It must still start by reading the on-disk cache.
    let source_b = ControlPlaneConfigSource::new(base, "w1", "w1", "eu-west")
        .with_cache_path(cache.clone());
    let cold = source_b
        .load()
        .await
        .expect("cold start must fall back to the disk cache");
    assert_eq!(cold.endpoints.len(), 1);
    assert_eq!(cold.endpoints[0].id, "web/prod/db");

    let _ = std::fs::remove_file(&cache);
}

#[tokio::test]
async fn skips_checks_pull_when_version_is_unchanged() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use axum::extract::State;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use synthetics::wire::{ChecksResponse, ResolvedCheck, TargetSpec, TcpSpec, VersionResponse};
    use synthetics_cp::model::Worker;

    // A mock CP whose /config/version is fixed at 1 but whose /checks changes
    // between calls. A version-gating worker must serve the first /checks result
    // on the second poll (since the version did not move) and never call /checks
    // twice.
    #[derive(Clone, Default)]
    struct Counter(Arc<AtomicUsize>);

    async fn version() -> Json<VersionResponse> {
        Json(VersionResponse { version: 1 })
    }
    async fn workers() -> Json<Vec<Worker>> {
        Json(vec![Worker {
            id: "w1".into(),
            region: "eu-west".into(),
            last_heartbeat_ms: 0,
        }])
    }
    async fn checks(State(count): State<Counter>) -> Json<ChecksResponse> {
        let n = count.0.fetch_add(1, Ordering::SeqCst);
        let id = if n == 0 { "a" } else { "b" };
        Json(ChecksResponse {
            version: 1,
            checks: vec![ResolvedCheck {
                id: id.into(),
                interval: Duration::from_secs(30),
                timeout: Duration::from_secs(5),
                tags: vec![],
                target: TargetSpec::Tcp(TcpSpec {
                    host: "h".into(),
                    port: 1,
                }),
                assertions: vec![],
                allow_internal: false,
                retries: 0,
                retry_backoff: Duration::ZERO,
            }],
        })
    }
    async fn heartbeat() -> axum::http::StatusCode {
        axum::http::StatusCode::NO_CONTENT
    }

    let count = Counter::default();
    let app = Router::new()
        .route("/config/version", get(version))
        .route("/workers", get(workers))
        .route("/checks", get(checks))
        .route("/workers/heartbeat", post(heartbeat))
        .with_state(count.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let source = ControlPlaneConfigSource::new(format!("http://{addr}"), "w1", "w1", "eu-west");

    let first = source.load().await.expect("first load");
    assert_eq!(first.endpoints[0].id, "a");

    let second = source.load().await.expect("second load");
    assert_eq!(second.endpoints[0].id, "a", "stale-but-unchanged config kept");
    assert_eq!(
        count.0.load(Ordering::SeqCst),
        1,
        "/checks must be pulled once while the version is unchanged"
    );
}

#[tokio::test]
async fn heartbeat_times_out_against_a_hung_server() {
    use std::time::Instant;
    // A listener that accepts connections and holds them open forever without
    // ever writing a response — the classic slow/dead control plane. Without a
    // request timeout the client would wait indefinitely and stall the worker's
    // heartbeat loop (and, on shutdown, block the process).
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let mut held = Vec::new();
        loop {
            if let Ok((sock, _)) = listener.accept().await {
                held.push(sock); // keep the socket open; never respond
            }
        }
    });

    let source = ControlPlaneConfigSource::new(format!("http://{addr}"), "w1", "w1", "eu-west")
        .with_request_timeout(Duration::from_millis(300));

    let start = Instant::now();
    let res = source.heartbeat().await;
    assert!(
        res.is_err(),
        "hung server must surface as an error, not a hang"
    );
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "heartbeat must give up on the configured timeout, not hang: took {:?}",
        start.elapsed()
    );
}

#[tokio::test]
async fn serves_cached_config_when_cp_is_down() {
    let (base, _store, shutdown) = spawn_cp().await;
    let source = ControlPlaneConfigSource::new(base, "w1", "w1", "eu-west");

    // Prime the cache from the live CP.
    let first = source.load().await.expect("initial load");
    assert_eq!(first.endpoints.len(), 1);

    // Take the CP down.
    let _ = shutdown.send(());
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Load again — CP is unreachable, so the cached config is returned (V29).
    let second = source.load().await.expect("cached config on CP outage");
    assert_eq!(second.endpoints.len(), 1);
    assert_eq!(second.endpoints[0].id, "web/prod/db");
}
