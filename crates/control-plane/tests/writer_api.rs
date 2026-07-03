//! Writer API tests (T24): per-product scoping (V31), write validation (V37),
//! and hierarchy/endpoint CRUD, driven against MemoryStore + StaticTokenAuth via
//! axum oneshot — no network, no DB.

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::Router;
use serde_json::Value;
use synthetics_cp::api::{writer_router, AppState};
use synthetics_cp::auth::StaticTokenAuth;
use synthetics_cp::model::{Endpoint, HttpSpec, Node, NodeDefaults, TargetSpec, TcpSpec};
use synthetics_cp::MemoryStore;
use tower::ServiceExt;

fn app() -> Router {
    let store = Arc::new(MemoryStore::new());
    let auth = Arc::new(
        StaticTokenAuth::new()
            .with("t-web", "web")
            .with("t-data", "data"),
    );
    writer_router(AppState::new(store, auth))
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

fn put(uri: &str, token: Option<&str>, body: &[u8]) -> Request<Body> {
    let mut b = Request::builder()
        .method("PUT")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::from(body.to_vec())).unwrap()
}

fn get(uri: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap()
}

fn tcp_endpoint(id: &str, env: &str, port: u16) -> Vec<u8> {
    let ep = Endpoint {
        id: id.into(),
        environment: env.into(),
        name: "check".into(),
        target: TargetSpec::Tcp(TcpSpec {
            host: "db".into(),
            port,
        }),
        interval: None,
        timeout: None,
        tags: vec![],
        assertions: vec![],
        allow_internal: false,
        retries: None,
        retry_backoff: None,
    };
    serde_json::to_vec(&ep).unwrap()
}

#[tokio::test]
async fn authorized_write_then_read_back() {
    let app = app();

    let (status, _) = send(
        &app,
        put(
            "/endpoints/web/prod/db",
            Some("t-web"),
            &tcp_endpoint("web/prod/db", "web/prod", 5432),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, body) = send(&app, get("/endpoints/web/prod/db", "t-web")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["id"], "web/prod/db");
    assert_eq!(body["target"]["port"], 5432);
}

#[tokio::test]
async fn missing_or_bad_token_is_unauthorized() {
    let app = app();
    let (status, _) = send(
        &app,
        put(
            "/endpoints/web/prod/db",
            None,
            &tcp_endpoint("web/prod/db", "web/prod", 1),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let (status, _) = send(
        &app,
        put(
            "/endpoints/web/prod/db",
            Some("nope"),
            &tcp_endpoint("web/prod/db", "web/prod", 1),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn writing_outside_your_product_is_forbidden() {
    let app = app();
    // t-web may not write under product "data".
    let (status, _) = send(
        &app,
        put(
            "/endpoints/data/prod/db",
            Some("t-web"),
            &tcp_endpoint("data/prod/db", "data/prod", 1),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn endpoint_environment_pointing_at_another_product_is_forbidden() {
    let app = app();
    // Id is inside t-web's scope, but the environment references another
    // product's node. Inheritance would pull that node's config/tags into this
    // check — a cross-tenant leak. Must be forbidden, not just id-scoped.
    let (status, _) = send(
        &app,
        put(
            "/endpoints/web/prod/db",
            Some("t-web"),
            &tcp_endpoint("web/prod/db", "data/prod", 5432),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn invalid_endpoint_is_rejected_with_field_scoped_error() {
    let app = app();
    // Port 0 is invalid.
    let (status, body) = send(
        &app,
        put(
            "/endpoints/web/prod/db",
            Some("t-web"),
            &tcp_endpoint("web/prod/db", "web/prod", 0),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["error"], "validation");
    assert_eq!(body["field"], "target.port");
}

#[tokio::test]
async fn internal_literal_ip_target_without_optin_is_rejected() {
    let app = app();
    // http target at the cloud metadata address, no allow_internal: the CP
    // rejects it at write time (defense in depth for literal IPs; hostnames are
    // caught by the worker at connect time).
    let ep = Endpoint {
        id: "web/prod/meta".into(),
        environment: "web/prod".into(),
        name: "meta".into(),
        target: TargetSpec::Http(HttpSpec {
            url: "http://169.254.169.254/latest/meta-data/".into(),
            method: None,
            headers: Default::default(),
            body: None,
            allow_insecure: false,
        }),
        interval: None,
        timeout: None,
        tags: vec![],
        assertions: vec![],
        allow_internal: false,
        retries: None,
        retry_backoff: None,
    };
    let (status, body) = send(
        &app,
        put(
            "/endpoints/web/prod/meta",
            Some("t-web"),
            &serde_json::to_vec(&ep).unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["field"], "target.url");

    // With the opt-in, the CP accepts it (the worker still gates on its
    // allowlist at connect time).
    let mut ok = ep.clone();
    ok.allow_internal = true;
    let (status, _) = send(
        &app,
        put(
            "/endpoints/web/prod/meta",
            Some("t-web"),
            &serde_json::to_vec(&ok).unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn invalid_http_url_is_rejected() {
    let app = app();
    let ep = Endpoint {
        id: "web/prod/x".into(),
        environment: "web/prod".into(),
        name: "x".into(),
        target: TargetSpec::Http(HttpSpec {
            url: "ftp://nope".into(),
            method: None,
            headers: Default::default(),
            body: None,
            allow_insecure: false,
        }),
        interval: None,
        timeout: None,
        tags: vec![],
        assertions: vec![],
        allow_internal: false,
        retries: None,
        retry_backoff: None,
    };
    let (status, body) = send(
        &app,
        put(
            "/endpoints/web/prod/x",
            Some("t-web"),
            &serde_json::to_vec(&ep).unwrap(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(body["field"], "target.url");
}

#[tokio::test]
async fn list_is_scoped_to_your_product() {
    let app = app();
    send(
        &app,
        put(
            "/endpoints/web/prod/db",
            Some("t-web"),
            &tcp_endpoint("web/prod/db", "web/prod", 5432),
        ),
    )
    .await;
    send(
        &app,
        put(
            "/endpoints/data/prod/db",
            Some("t-data"),
            &tcp_endpoint("data/prod/db", "data/prod", 5432),
        ),
    )
    .await;

    let (status, body) = send(&app, get("/endpoints", "t-web")).await;
    assert_eq!(status, StatusCode::OK);
    let arr = body.as_array().unwrap();
    assert_eq!(arr.len(), 1, "web sees only its own endpoints");
    assert_eq!(arr[0]["id"], "web/prod/db");
}

#[tokio::test]
async fn delete_then_missing() {
    let app = app();
    send(
        &app,
        put(
            "/nodes/web",
            Some("t-web"),
            &serde_json::to_vec(&Node {
                id: "web".into(),
                kind: "product".into(),
                name: "web".into(),
                parent: None,
                defaults: NodeDefaults::default(),
            })
            .unwrap(),
        ),
    )
    .await;

    let del = Request::builder()
        .method("DELETE")
        .uri("/nodes/web")
        .header("authorization", "Bearer t-web")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&app, del).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let del2 = Request::builder()
        .method("DELETE")
        .uri("/nodes/web")
        .header("authorization", "Bearer t-web")
        .body(Body::empty())
        .unwrap();
    let (status, _) = send(&app, del2).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}
