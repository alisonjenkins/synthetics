//! In-memory [`ConfigStore`] (T22): the zero-infrastructure implementation used
//! by tests and single-node/dev setups, mirroring the worker crate's `MockSink`.

use std::collections::BTreeMap;
use std::sync::Mutex;

use crate::model::{Endpoint, Node, Worker};
use crate::store::{ConfigStore, StoreError};

#[derive(Default)]
struct Inner {
    nodes: BTreeMap<String, Node>,
    endpoints: BTreeMap<String, Endpoint>,
    workers: BTreeMap<String, Worker>,
    version: u64,
}

/// An in-memory config store.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<Inner>,
}

impl MemoryStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    fn with<R>(&self, f: impl FnOnce(&mut Inner) -> R) -> R {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        f(&mut guard)
    }
}

impl ConfigStore for MemoryStore {
    async fn upsert_node(&self, node: Node) -> Result<(), StoreError> {
        self.with(|inner| {
            inner.nodes.insert(node.id.clone(), node);
            inner.version += 1;
        });
        Ok(())
    }

    async fn get_node(&self, id: &str) -> Result<Option<Node>, StoreError> {
        Ok(self.with(|inner| inner.nodes.get(id).cloned()))
    }

    async fn delete_node(&self, id: &str) -> Result<bool, StoreError> {
        Ok(self.with(|inner| {
            let existed = inner.nodes.remove(id).is_some();
            if existed {
                inner.version += 1;
            }
            existed
        }))
    }

    async fn list_nodes(&self) -> Result<Vec<Node>, StoreError> {
        Ok(self.with(|inner| inner.nodes.values().cloned().collect()))
    }

    async fn upsert_endpoint(&self, endpoint: Endpoint) -> Result<(), StoreError> {
        self.with(|inner| {
            inner.endpoints.insert(endpoint.id.clone(), endpoint);
            inner.version += 1;
        });
        Ok(())
    }

    async fn get_endpoint(&self, id: &str) -> Result<Option<Endpoint>, StoreError> {
        Ok(self.with(|inner| inner.endpoints.get(id).cloned()))
    }

    async fn delete_endpoint(&self, id: &str) -> Result<bool, StoreError> {
        Ok(self.with(|inner| {
            let existed = inner.endpoints.remove(id).is_some();
            if existed {
                inner.version += 1;
            }
            existed
        }))
    }

    async fn list_endpoints(&self) -> Result<Vec<Endpoint>, StoreError> {
        Ok(self.with(|inner| inner.endpoints.values().cloned().collect()))
    }

    async fn heartbeat_worker(
        &self,
        id: &str,
        region: &str,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        self.with(|inner| {
            inner.workers.insert(
                id.to_string(),
                Worker {
                    id: id.to_string(),
                    region: region.to_string(),
                    last_heartbeat_ms: now_ms,
                },
            );
        });
        Ok(())
    }

    async fn list_live_workers(&self, cutoff_ms: i64) -> Result<Vec<Worker>, StoreError> {
        Ok(self.with(|inner| {
            inner
                .workers
                .values()
                .filter(|w| w.last_heartbeat_ms >= cutoff_ms)
                .cloned()
                .collect()
        }))
    }

    async fn config_version(&self) -> Result<u64, StoreError> {
        Ok(self.with(|inner| inner.version))
    }
}
