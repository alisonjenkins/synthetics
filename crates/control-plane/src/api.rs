//! Control-plane HTTP API (T24 writer surface).
//!
//! The writer API is authored by Terraform/CI: hierarchy + endpoint CRUD,
//! authenticated per-product (V31) via the [`WriterAuth`] trait, and validated
//! on every write (V37). Handlers are generic over the [`ConfigStore`] (the
//! trait is not `dyn`-compatible; the CP is a cold path so generics are fine)
//! and the authenticator, so tests drive them against `MemoryStore` +
//! `StaticTokenAuth`.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, put};
use axum::{Json, Router};

use crate::auth::{WriterAuth, WriterIdentity};
use crate::model::{Endpoint, Node};
use crate::store::ConfigStore;
use crate::validate::{validate_endpoint, validate_node, FieldError};

/// Shared handler state: the config store and the writer authenticator.
pub struct AppState<S, A> {
    /// The config store.
    pub store: Arc<S>,
    /// The writer authenticator.
    pub auth: Arc<A>,
}

impl<S, A> Clone for AppState<S, A> {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
            auth: Arc::clone(&self.auth),
        }
    }
}

impl<S, A> AppState<S, A> {
    /// Assemble state from a store and authenticator.
    pub fn new(store: Arc<S>, auth: Arc<A>) -> Self {
        Self { store, auth }
    }
}

/// An API error rendered as a JSON body with an appropriate status.
#[derive(Debug)]
pub enum ApiError {
    /// Missing or invalid credentials.
    Unauthorized,
    /// Authenticated, but not permitted to touch this resource.
    Forbidden,
    /// Resource not found.
    NotFound,
    /// Request body failed validation (field-scoped, V37).
    Validation(FieldError),
    /// An unexpected backend error.
    Internal(String),
}

impl From<FieldError> for ApiError {
    fn from(e: FieldError) -> Self {
        ApiError::Validation(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, body) = match self {
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                serde_json::json!({"error": "unauthorized"}),
            ),
            ApiError::Forbidden => (
                StatusCode::FORBIDDEN,
                serde_json::json!({"error": "forbidden"}),
            ),
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                serde_json::json!({"error": "not_found"}),
            ),
            ApiError::Validation(fe) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                serde_json::json!({"error": "validation", "field": fe.field, "message": fe.message}),
            ),
            ApiError::Internal(message) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                serde_json::json!({"error": "internal", "message": message}),
            ),
        };
        (status, Json(body)).into_response()
    }
}

fn internal<E: std::fmt::Display>(e: E) -> ApiError {
    ApiError::Internal(e.to_string())
}

/// The product an id belongs to (its first path segment).
fn product_of(id: &str) -> &str {
    id.split('/').next().unwrap_or("")
}

fn in_scope(identity: &WriterIdentity, id: &str) -> bool {
    product_of(id) == identity.product
}

fn require_scope(identity: &WriterIdentity, id: &str) -> Result<(), ApiError> {
    if in_scope(identity, id) {
        Ok(())
    } else {
        Err(ApiError::Forbidden)
    }
}

async fn authenticate<A: WriterAuth>(
    auth: &A,
    headers: &HeaderMap,
) -> Result<WriterIdentity, ApiError> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .ok_or(ApiError::Unauthorized)?;
    auth.authenticate(token)
        .await
        .map_err(|_| ApiError::Unauthorized)
}

/// Build the writer API router.
pub fn writer_router<S, A>(state: AppState<S, A>) -> Router
where
    S: ConfigStore + 'static,
    A: WriterAuth + 'static,
{
    Router::new()
        // Ids are hierarchical paths ("web/globex/prod"), so capture the rest of
        // the path with a wildcard.
        .route("/nodes", get(list_nodes::<S, A>))
        .route(
            "/nodes/{*id}",
            put(put_node::<S, A>)
                .get(get_node::<S, A>)
                .delete(delete_node::<S, A>),
        )
        .route("/endpoints", get(list_endpoints::<S, A>))
        .route(
            "/endpoints/{*id}",
            put(put_endpoint::<S, A>)
                .get(get_endpoint::<S, A>)
                .delete(delete_endpoint::<S, A>),
        )
        .with_state(state)
}

async fn put_node<S: ConfigStore, A: WriterAuth>(
    State(state): State<AppState<S, A>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(mut node): Json<Node>,
) -> Result<StatusCode, ApiError> {
    let identity = authenticate(&*state.auth, &headers).await?;
    node.id = id;
    require_scope(&identity, &node.id)?;
    validate_node(&node)?;
    state.store.upsert_node(node).await.map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_node<S: ConfigStore, A: WriterAuth>(
    State(state): State<AppState<S, A>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Node>, ApiError> {
    let identity = authenticate(&*state.auth, &headers).await?;
    require_scope(&identity, &id)?;
    match state.store.get_node(&id).await.map_err(internal)? {
        Some(node) => Ok(Json(node)),
        None => Err(ApiError::NotFound),
    }
}

async fn delete_node<S: ConfigStore, A: WriterAuth>(
    State(state): State<AppState<S, A>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let identity = authenticate(&*state.auth, &headers).await?;
    require_scope(&identity, &id)?;
    if state.store.delete_node(&id).await.map_err(internal)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

async fn list_nodes<S: ConfigStore, A: WriterAuth>(
    State(state): State<AppState<S, A>>,
    headers: HeaderMap,
) -> Result<Json<Vec<Node>>, ApiError> {
    let identity = authenticate(&*state.auth, &headers).await?;
    let nodes = state.store.list_nodes().await.map_err(internal)?;
    Ok(Json(
        nodes
            .into_iter()
            .filter(|n| in_scope(&identity, &n.id))
            .collect(),
    ))
}

async fn put_endpoint<S: ConfigStore, A: WriterAuth>(
    State(state): State<AppState<S, A>>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(mut endpoint): Json<Endpoint>,
) -> Result<StatusCode, ApiError> {
    let identity = authenticate(&*state.auth, &headers).await?;
    endpoint.id = id;
    require_scope(&identity, &endpoint.id)?;
    validate_endpoint(&endpoint)?;
    state
        .store
        .upsert_endpoint(endpoint)
        .await
        .map_err(internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_endpoint<S: ConfigStore, A: WriterAuth>(
    State(state): State<AppState<S, A>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Endpoint>, ApiError> {
    let identity = authenticate(&*state.auth, &headers).await?;
    require_scope(&identity, &id)?;
    match state.store.get_endpoint(&id).await.map_err(internal)? {
        Some(endpoint) => Ok(Json(endpoint)),
        None => Err(ApiError::NotFound),
    }
}

async fn delete_endpoint<S: ConfigStore, A: WriterAuth>(
    State(state): State<AppState<S, A>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let identity = authenticate(&*state.auth, &headers).await?;
    require_scope(&identity, &id)?;
    if state.store.delete_endpoint(&id).await.map_err(internal)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

async fn list_endpoints<S: ConfigStore, A: WriterAuth>(
    State(state): State<AppState<S, A>>,
    headers: HeaderMap,
) -> Result<Json<Vec<Endpoint>>, ApiError> {
    let identity = authenticate(&*state.auth, &headers).await?;
    let endpoints = state.store.list_endpoints().await.map_err(internal)?;
    Ok(Json(
        endpoints
            .into_iter()
            .filter(|e| in_scope(&identity, &e.id))
            .collect(),
    ))
}
