//! Control plane for the synthetics fleet.
//!
//! Stores the config hierarchy and worker membership behind a [`ConfigStore`]
//! trait (ADR 0020), resolves the hierarchy into flat worker checks (T23), and
//! serves them to the fleet (T24/T25). The store holds config + membership only
//! — check results flow worker → DogStatsD → Datadog, never through here (V30).

pub mod api;
pub mod auth;
pub mod checks_cache;
pub mod memory;
pub mod model;
pub mod pg;
pub mod resolve;
pub mod store;
pub mod validate;

pub use memory::MemoryStore;
pub use pg::PgStore;
pub use resolve::{resolve, ResolveError, ResolvedCheck};
pub use store::{ConfigStore, StoreError};
