//! CP server wiring (T30): the merged writer+worker app over a real Postgres —
//! author via the writer API, read back resolved via the worker API.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::Value;
use synthetics_cp::api::{worker_router, writer_router, AppState};
use synthetics_cp::auth::{AcceptWorkerAuth, StaticTokenAuth};
use synthetics_cp::PgStore;
use tower::ServiceExt;

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

fn put(uri: &str, token: &str, body: Value) -> Request<Body> {
    Request::builder()
        .method("PUT")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

#[tokio::test]
async fn author_via_writer_then_read_resolved_via_worker_over_postgres() {
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let store = Arc::new(PgStore::connect(&url).await.unwrap());
    let app = writer_router(AppState::new(
        Arc::clone(&store),
        Arc::new(StaticTokenAuth::new().with("t-web", "web")),
    ))
    .merge(worker_router(AppState::new(
        store,
        Arc::new(AcceptWorkerAuth),
    )));

    // Author a product with a default interval, an environment, and an endpoint.
    let (s, _) = send(
        &app,
        put(
            "/nodes/web",
            "t-web",
            serde_json::json!({
                "id": "web", "kind": "product", "name": "web", "parent": null,
                "defaults": { "interval": "30s", "tags": [] }
            }),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (s, _) = send(
        &app,
        put(
            "/nodes/web/prod",
            "t-web",
            serde_json::json!({
                "id": "web/prod", "kind": "environment", "name": "web-prod", "parent": "web"
            }),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    let (s, _) = send(
        &app,
        put(
            "/endpoints/web/prod/db",
            "t-web",
            serde_json::json!({
                "id": "web/prod/db", "environment": "web/prod", "name": "db",
                "target": { "type": "tcp", "host": "db", "port": 5432 }
            }),
        ),
    )
    .await;
    assert_eq!(s, StatusCode::NO_CONTENT);

    // Read back resolved through the worker API.
    let checks = Request::builder()
        .method("GET")
        .uri("/checks")
        .header("authorization", "Bearer w1")
        .body(Body::empty())
        .unwrap();
    let (s, body) = send(&app, checks).await;
    assert_eq!(s, StatusCode::OK);
    let arr = body["checks"].as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["id"], "web/prod/db");
    assert_eq!(arr[0]["interval"], "30s"); // inherited from the product node
    assert!(arr[0]["tags"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t == "product:web"));
}
