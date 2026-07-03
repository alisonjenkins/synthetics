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
