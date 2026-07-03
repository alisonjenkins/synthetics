//! Postgres [`ConfigStore`] (T22, ADR 0020): the production backend. All SQL and
//! Postgres specifics are confined here (V34). Entities are stored as `jsonb`
//! keyed by id for exact serde round-trips; the config version is bumped in the
//! same transaction as each write so a reader never sees a torn update.

use sqlx::postgres::PgPoolOptions;
use sqlx::types::Json;
use sqlx::PgPool;

use crate::model::{Endpoint, Node, Worker};
use crate::store::{ConfigStore, StoreError};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS node (
    id     text PRIMARY KEY,
    kind   text NOT NULL,
    parent text,
    data   jsonb NOT NULL
);
CREATE TABLE IF NOT EXISTS endpoint (
    id          text PRIMARY KEY,
    environment text NOT NULL,
    data        jsonb NOT NULL
);
CREATE TABLE IF NOT EXISTS worker (
    id                text PRIMARY KEY,
    region            text NOT NULL,
    last_heartbeat_ms bigint NOT NULL
);
CREATE TABLE IF NOT EXISTS config_state (
    id      int PRIMARY KEY DEFAULT 1,
    version bigint NOT NULL DEFAULT 0,
    CHECK (id = 1)
);
INSERT INTO config_state (id, version) VALUES (1, 0) ON CONFLICT DO NOTHING;

-- Indexes on the scalar columns the CP filters by (ADR 0024). Payload lives in
-- jsonb; the columns we query are normalized and indexed.
CREATE INDEX IF NOT EXISTS node_parent_idx ON node (parent);
CREATE INDEX IF NOT EXISTS endpoint_environment_idx ON endpoint (environment);
"#;

fn backend<E: std::fmt::Display>(e: E) -> StoreError {
    StoreError::Backend(e.to_string())
}

/// A Postgres-backed config store.
#[derive(Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    /// Connect to Postgres at `url` and apply the schema.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new()
            .max_connections(8)
            .connect(url)
            .await
            .map_err(backend)?;
        let store = Self { pool };
        store.migrate().await?;
        Ok(store)
    }

    /// Build a store from an existing pool (already migrated by [`Self::connect`]
    /// or by the caller).
    pub fn from_pool(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Apply the schema (idempotent). Guarded by a transaction-scoped advisory
    /// lock so that during a rolling upgrade several control-plane replicas
    /// starting at once serialize their migrations rather than racing — one
    /// applies, the rest wait and then find everything already present. The lock
    /// releases automatically when the transaction ends.
    pub async fn migrate(&self) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(MIGRATION_LOCK_KEY)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;
        sqlx::raw_sql(SCHEMA)
            .execute(&mut *tx)
            .await
            .map_err(backend)?;
        tx.commit().await.map_err(backend)
    }
}

/// Arbitrary fixed key identifying the schema-migration advisory lock.
const MIGRATION_LOCK_KEY: i64 = 0x53_59_4e_54_48_00; // "SYNTH"

impl ConfigStore for PgStore {
    async fn upsert_node(&self, node: Node) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        sqlx::query(
            "INSERT INTO node (id, kind, parent, data) VALUES ($1, $2, $3, $4)
             ON CONFLICT (id) DO UPDATE SET kind = $2, parent = $3, data = $4",
        )
        .bind(&node.id)
        .bind(&node.kind)
        .bind(&node.parent)
        .bind(Json(&node))
        .execute(&mut *tx)
        .await
        .map_err(backend)?;
        bump_version(&mut tx).await?;
        tx.commit().await.map_err(backend)
    }

    async fn get_node(&self, id: &str) -> Result<Option<Node>, StoreError> {
        let row: Option<Json<Node>> = sqlx::query_scalar("SELECT data FROM node WHERE id = $1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map_err(backend)?;
        Ok(row.map(|j| j.0))
    }

    async fn delete_node(&self, id: &str) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let affected = sqlx::query("DELETE FROM node WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(backend)?
            .rows_affected();
        if affected > 0 {
            bump_version(&mut tx).await?;
        }
        tx.commit().await.map_err(backend)?;
        Ok(affected > 0)
    }

    async fn list_nodes(&self) -> Result<Vec<Node>, StoreError> {
        let rows: Vec<Json<Node>> = sqlx::query_scalar("SELECT data FROM node")
            .fetch_all(&self.pool)
            .await
            .map_err(backend)?;
        Ok(rows.into_iter().map(|j| j.0).collect())
    }

    async fn upsert_endpoint(&self, endpoint: Endpoint) -> Result<(), StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        sqlx::query(
            "INSERT INTO endpoint (id, environment, data) VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET environment = $2, data = $3",
        )
        .bind(&endpoint.id)
        .bind(&endpoint.environment)
        .bind(Json(&endpoint))
        .execute(&mut *tx)
        .await
        .map_err(backend)?;
        bump_version(&mut tx).await?;
        tx.commit().await.map_err(backend)
    }

    async fn get_endpoint(&self, id: &str) -> Result<Option<Endpoint>, StoreError> {
        let row: Option<Json<Endpoint>> =
            sqlx::query_scalar("SELECT data FROM endpoint WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map_err(backend)?;
        Ok(row.map(|j| j.0))
    }

    async fn delete_endpoint(&self, id: &str) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let affected = sqlx::query("DELETE FROM endpoint WHERE id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(backend)?
            .rows_affected();
        if affected > 0 {
            bump_version(&mut tx).await?;
        }
        tx.commit().await.map_err(backend)?;
        Ok(affected > 0)
    }

    async fn list_endpoints(&self) -> Result<Vec<Endpoint>, StoreError> {
        let rows: Vec<Json<Endpoint>> = sqlx::query_scalar("SELECT data FROM endpoint")
            .fetch_all(&self.pool)
            .await
            .map_err(backend)?;
        Ok(rows.into_iter().map(|j| j.0).collect())
    }

    async fn heartbeat_worker(
        &self,
        id: &str,
        region: &str,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        sqlx::query(
            "INSERT INTO worker (id, region, last_heartbeat_ms) VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET region = $2, last_heartbeat_ms = $3",
        )
        .bind(id)
        .bind(region)
        .bind(now_ms)
        .execute(&self.pool)
        .await
        .map_err(backend)?;
        Ok(())
    }

    async fn list_live_workers(&self, cutoff_ms: i64) -> Result<Vec<Worker>, StoreError> {
        let rows: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT id, region, last_heartbeat_ms FROM worker WHERE last_heartbeat_ms >= $1",
        )
        .bind(cutoff_ms)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;
        Ok(rows
            .into_iter()
            .map(|(id, region, last_heartbeat_ms)| Worker {
                id,
                region,
                last_heartbeat_ms,
            })
            .collect())
    }

    async fn config_version(&self) -> Result<u64, StoreError> {
        let version: i64 = sqlx::query_scalar("SELECT version FROM config_state WHERE id = 1")
            .fetch_one(&self.pool)
            .await
            .map_err(backend)?;
        Ok(version as u64)
    }
}

async fn bump_version(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<(), StoreError> {
    sqlx::query("UPDATE config_state SET version = version + 1 WHERE id = 1")
        .execute(&mut **tx)
        .await
        .map_err(backend)?;
    Ok(())
}
