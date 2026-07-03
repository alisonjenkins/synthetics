//! Inheritance resolution tests (T23): defaults inherit from the nearest
//! ancestor, endpoint overrides win, tags accumulate + auto-generate, and a
//! dangling environment errors without failing other endpoints (V25).

// Panicking is the failure signal in tests (ADR 0008).
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::time::Duration;

use synthetics_cp::model::{Endpoint, Node, NodeDefaults, TargetSpec, TcpSpec};
use synthetics_cp::resolve::{resolve, ResolveError};

fn node(id: &str, kind: &str, name: &str, parent: Option<&str>, defaults: NodeDefaults) -> Node {
    Node {
        id: id.into(),
        kind: kind.into(),
        name: name.into(),
        parent: parent.map(str::to_string),
        defaults,
    }
}

fn tcp_endpoint(id: &str, environment: &str, interval: Option<Duration>) -> Endpoint {
    Endpoint {
        id: id.into(),
        environment: environment.into(),
        name: "check".into(),
        target: TargetSpec::Tcp(TcpSpec {
            host: "db".into(),
            port: 5432,
        }),
        interval,
        timeout: None,
        tags: vec![],
        assertions: vec![],
        allow_internal: false,
    }
}

fn hierarchy() -> Vec<Node> {
    vec![
        node(
            "web",
            "product",
            "web",
            None,
            NodeDefaults {
                interval: Some(Duration::from_secs(60)),
                timeout: None,
                tags: vec!["team:web".into()],
            },
        ),
        node(
            "web/globex",
            "customer",
            "globex",
            Some("web"),
            NodeDefaults::default(),
        ),
        node(
            "web/globex/prod",
            "environment",
            "globex-prod",
            Some("web/globex"),
            NodeDefaults {
                interval: Some(Duration::from_secs(30)),
                timeout: None,
                tags: vec!["env:prod".into()],
            },
        ),
    ]
}

#[test]
fn inherits_nearest_ancestor_interval_and_accumulates_tags() {
    let nodes = hierarchy();
    let ep = tcp_endpoint("web/globex/prod/db", "web/globex/prod", None);

    let (checks, errors) = resolve(&nodes, &[ep]);
    assert!(errors.is_empty());
    let c = &checks[0];

    // Nearest ancestor (environment) sets 30s; product's 60s is overridden.
    assert_eq!(c.interval, Duration::from_secs(30));
    // No timeout anywhere -> default.
    assert_eq!(c.timeout, synthetics::config::DEFAULT_TIMEOUT);
    // Auto <kind>:<name> tags for every level + inherited default tags.
    for want in [
        "product:web",
        "team:web",
        "customer:globex",
        "environment:globex-prod",
        "env:prod",
    ] {
        assert!(
            c.tags.contains(&want.to_string()),
            "missing tag {want}: {:?}",
            c.tags
        );
    }
}

#[test]
fn endpoint_override_wins() {
    let nodes = hierarchy();
    let ep = tcp_endpoint(
        "web/globex/prod/db",
        "web/globex/prod",
        Some(Duration::from_secs(300)),
    );
    let (checks, _) = resolve(&nodes, &[ep]);
    assert_eq!(checks[0].interval, Duration::from_secs(300));
}

#[test]
fn default_interval_when_nothing_set() {
    let nodes = vec![node(
        "solo",
        "environment",
        "solo",
        None,
        NodeDefaults::default(),
    )];
    let ep = tcp_endpoint("solo/db", "solo", None);
    let (checks, _) = resolve(&nodes, &[ep]);
    assert_eq!(checks[0].interval, synthetics::config::DEFAULT_INTERVAL);
}

#[test]
fn missing_environment_errors_without_failing_others() {
    let nodes = hierarchy();
    let good = tcp_endpoint("web/globex/prod/db", "web/globex/prod", None);
    let orphan = tcp_endpoint("x/db", "does-not-exist", None);

    let (checks, errors) = resolve(&nodes, &[good, orphan]);
    assert_eq!(checks.len(), 1, "good endpoint still resolves");
    assert_eq!(checks[0].id, "web/globex/prod/db");
    assert!(matches!(
        &errors[0],
        ResolveError::MissingEnvironment { endpoint, .. } if endpoint == "x/db"
    ));
}

#[test]
fn tags_are_deduplicated() {
    // Endpoint repeats an inherited tag; it should appear once.
    let nodes = hierarchy();
    let mut ep = tcp_endpoint("web/globex/prod/db", "web/globex/prod", None);
    ep.tags = vec!["env:prod".into(), "extra:1".into()];
    let (checks, _) = resolve(&nodes, &[ep]);
    let count = checks[0].tags.iter().filter(|t| *t == "env:prod").count();
    assert_eq!(
        count, 1,
        "env:prod should be deduplicated: {:?}",
        checks[0].tags
    );
}
