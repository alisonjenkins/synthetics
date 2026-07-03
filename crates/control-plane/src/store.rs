//! The [`ConfigStore`] abstraction (T22, ADR 0020).
//!
//! The control plane depends only on this trait — no backend-specific code
//! (SQL, `LISTEN/NOTIFY`) lives outside an impl (V34). It stores config +
//! fleet membership only, never results (V30). Static dispatch: native async
//! fns (RPITIT) with `Send` futures (V35).

use std::future::Future;

use crate::model::{Endpoint, Node, Worker};

/// Errors from a [`ConfigStore`].
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The storage backend failed (connection, query, ...).
    #[error("storage backend error: {0}")]
    Backend(String),
    /// Stored data could not be (de)serialized.
    #[error("data serialization error: {0}")]
    Serde(String),
}

/// Persistence for the config hierarchy and fleet membership.
pub trait ConfigStore: Send + Sync {
    /// Insert or replace a hierarchy node.
    fn upsert_node(&self, node: Node) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Fetch a node by id.
    fn get_node(&self, id: &str) -> impl Future<Output = Result<Option<Node>, StoreError>> + Send;
    /// Delete a node; returns whether it existed.
    fn delete_node(&self, id: &str) -> impl Future<Output = Result<bool, StoreError>> + Send;
    /// List all nodes.
    fn list_nodes(&self) -> impl Future<Output = Result<Vec<Node>, StoreError>> + Send;

    /// Insert or replace an endpoint.
    fn upsert_endpoint(
        &self,
        endpoint: Endpoint,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// Fetch an endpoint by id.
    fn get_endpoint(
        &self,
        id: &str,
    ) -> impl Future<Output = Result<Option<Endpoint>, StoreError>> + Send;
    /// Delete an endpoint; returns whether it existed.
    fn delete_endpoint(&self, id: &str) -> impl Future<Output = Result<bool, StoreError>> + Send;
    /// List all endpoints.
    fn list_endpoints(&self) -> impl Future<Output = Result<Vec<Endpoint>, StoreError>> + Send;

    /// Record a worker heartbeat (upsert), with `now_ms` as the current time.
    fn heartbeat_worker(
        &self,
        id: &str,
        region: &str,
        now_ms: i64,
    ) -> impl Future<Output = Result<(), StoreError>> + Send;
    /// List workers whose last heartbeat is at or after `cutoff_ms` (the live
    /// worker-set that drives rendezvous ownership).
    fn list_live_workers(
        &self,
        cutoff_ms: i64,
    ) -> impl Future<Output = Result<Vec<Worker>, StoreError>> + Send;

    /// Monotonic config version, bumped on any node/endpoint write. Workers poll
    /// this to detect changes (V28).
    fn config_version(&self) -> impl Future<Output = Result<u64, StoreError>> + Send;
}
