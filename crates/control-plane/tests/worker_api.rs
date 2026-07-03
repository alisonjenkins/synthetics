//! Worker API tests (T25): read-only serving of resolved checks + version +
//! live worker-set, plus heartbeat, behind worker auth. Cites I.cp, V25, V28.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::Value;
use synthetics_cp::api::{worker_router, AppState};
use synthetics_cp::auth::AcceptWorkerAuth;
use synthetics_cp::model::{Endpoint, Node, NodeDefaults, TargetSpec, TcpSpec};
use synthetics_cp::{ConfigStore, MemoryStore};
use tower::ServiceExt;

async fn seeded_app() -> Router {
    let store = Arc::new(MemoryStore::new());
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
                host: "db".into(),
                port: 5432,
            }),
            interval: None,
            timeout: None,
            tags: vec![],
            assertions: vec![],
        })
        .await
        .unwrap();
    worker_router(AppState::new(store, Arc::new(AcceptWorkerAuth)))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let json = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, json)
}

fn get(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn checks_are_served_resolved() {
    let app = seeded_app().await;
    let (status, body) = send(&app, get("/checks", Some("w1"))).await;
    assert_eq!(status, StatusCode::OK);
    let checks = body["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0]["id"], "web/prod/db");
    // Inherited from the product node (V25).
    assert_eq!(checks[0]["interval"], "30s");
    let tags = checks[0]["tags"].as_array().unwrap();
    assert!(
        tags.iter().any(|t| t == "product:web"),
        "auto tag present: {tags:?}"
    );
    assert!(body["version"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn version_is_served() {
    let app = seeded_app().await;
    let (status, body) = send(&app, get("/config/version", Some("w1"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["version"].as_u64().unwrap() >= 1);
}

#[tokio::test]
async fn reads_require_worker_auth() {
    let app = seeded_app().await;
    let (status, _) = send(&app, get("/checks", None)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn heartbeat_registers_a_live_worker() {
    let app = seeded_app().await;

    // No workers yet.
    let (_, body) = send(&app, get("/workers", Some("w1"))).await;
    assert_eq!(body.as_array().unwrap().len(), 0);

    let hb = Request::builder()
        .method("POST")
        .uri("/workers/heartbeat")
        .header("authorization", "Bearer w1")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"id":"w1","region":"eu-west"}"#))
        .unwrap();
    let (status, _) = send(&app, hb).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = send(&app, get("/workers", Some("w1"))).await;
    assert_eq!(status, StatusCode::OK);
    let workers = body.as_array().unwrap();
    assert_eq!(workers.len(), 1);
    assert_eq!(workers[0]["id"], "w1");
    assert_eq!(workers[0]["region"], "eu-west");
}
