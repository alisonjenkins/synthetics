//! Control plane for the synthetics fleet.
//!
//! Stores the config hierarchy and worker membership behind a [`ConfigStore`]
//! trait (ADR 0020), resolves the hierarchy into flat worker checks (T23), and
//! serves them to the fleet (T24/T25). The store holds config + membership only
//! — check results flow worker → DogStatsD → Datadog, never through here (V30).

pub mod memory;
pub mod model;
pub mod pg;
pub mod store;

pub use memory::MemoryStore;
pub use pg::PgStore;
pub use store::{ConfigStore, StoreError};
