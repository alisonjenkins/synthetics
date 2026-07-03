//! ConfigStore contract (T22): one behavior suite run against every impl —
//! MemoryStore (no infra) and PgStore (ephemeral Postgres container). Both must
//! agree, which is what makes the store swappable (V34).

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use synthetics::config::Assertion;
use synthetics_cp::model::{Endpoint, HttpSpec, Node, NodeDefaults, TargetSpec};
use synthetics_cp::{ConfigStore, MemoryStore, PgStore};

async fn exercise<S: ConfigStore>(store: &S) {
    assert_eq!(store.config_version().await.unwrap(), 0);

    // --- nodes ---
    let node = Node {
        id: "web".into(),
        kind: "product".into(),
        name: "Web".into(),
        parent: None,
        defaults: NodeDefaults {
            interval: Some(Duration::from_secs(30)),
            timeout: None,
            tags: vec!["product:web".into()],
        },
    };
    store.upsert_node(node.clone()).await.unwrap();
    assert_eq!(store.get_node("web").await.unwrap(), Some(node.clone()));
    assert_eq!(store.list_nodes().await.unwrap(), vec![node]);
    assert_eq!(store.config_version().await.unwrap(), 1);

    // --- endpoints ---
    let ep = Endpoint {
        id: "web/prod/login".into(),
        environment: "web/prod".into(),
        name: "login".into(),
        target: TargetSpec::Http(HttpSpec {
            url: "https://x/login".into(),
            method: None,
            headers: Default::default(),
            body: None,
            allow_insecure: false,
        }),
        interval: None,
        timeout: None,
        tags: vec![],
        assertions: vec![Assertion::Status {
            equals: Some(200),
            in_range: None,
        }],
    };
    store.upsert_endpoint(ep.clone()).await.unwrap();
    assert_eq!(
        store.get_endpoint("web/prod/login").await.unwrap(),
        Some(ep.clone())
    );
    assert_eq!(store.list_endpoints().await.unwrap(), vec![ep]);
    assert_eq!(store.config_version().await.unwrap(), 2);

    // --- workers (membership; does not bump config version) ---
    store.heartbeat_worker("w1", "eu-west", 1000).await.unwrap();
    store.heartbeat_worker("w2", "eu-west", 2000).await.unwrap();
    let live = store.list_live_workers(1500).await.unwrap();
    assert_eq!(live.len(), 1, "only w2 is at/after the cutoff");
    assert_eq!(live[0].id, "w2");
    assert_eq!(store.config_version().await.unwrap(), 2);

    // --- delete ---
    assert!(store.delete_endpoint("web/prod/login").await.unwrap());
    assert!(!store.delete_endpoint("web/prod/login").await.unwrap());
    assert!(store.list_endpoints().await.unwrap().is_empty());
    assert_eq!(store.config_version().await.unwrap(), 3);
}

#[tokio::test]
async fn memory_store_satisfies_contract() {
    let store = MemoryStore::new();
    exercise(&store).await;
}

#[tokio::test]
async fn pg_store_satisfies_contract() {
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let store = PgStore::connect(&url).await.unwrap();
    exercise(&store).await;
}

#[tokio::test]
async fn concurrent_migrations_are_safe() {
    // Several control-plane replicas starting at once (a rolling upgrade) must
    // not race their migrations — the advisory lock serializes them.
    use testcontainers::runners::AsyncRunner;
    use testcontainers_modules::postgres::Postgres;

    let container = Postgres::default().start().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");

    let (a, b, c) = tokio::join!(
        PgStore::connect(&url),
        PgStore::connect(&url),
        PgStore::connect(&url),
    );
    let store = a.expect("connect a");
    b.expect("connect b");
    c.expect("connect c");

    // Schema is intact and usable.
    assert_eq!(store.config_version().await.unwrap(), 0);
}
