//! Hierarchy inheritance resolution (T23).
//!
//! The control plane resolves the authoring hierarchy into flat, materialized
//! checks server-side (V25) — workers never see the hierarchy. Each endpoint
//! inherits `interval`/`timeout` from the nearest ancestor that sets them
//! (endpoint overrides win), accumulates tags from every ancestor plus an
//! auto-generated `<kind>:<name>` tag per level, and passes its target and
//! assertions through. The result, [`ResolvedCheck`], is the wire form workers
//! consume (T21/T26).

use std::collections::HashMap;

use synthetics::config::{DEFAULT_INTERVAL, DEFAULT_TIMEOUT};

use crate::model::{Endpoint, Node};

// The resolved-check wire type is defined in the worker crate and re-exported
// here (T26 contract).
pub use synthetics::wire::ResolvedCheck;

/// Why an endpoint could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    /// The endpoint references an environment node that does not exist.
    #[error("endpoint {endpoint:?} references unknown environment {environment:?}")]
    MissingEnvironment {
        /// Endpoint id.
        endpoint: String,
        /// The missing environment node id.
        environment: String,
    },
}

/// Resolve every endpoint against the hierarchy. Returns the successfully
/// resolved checks and the per-endpoint errors (a bad endpoint never fails the
/// others).
pub fn resolve(nodes: &[Node], endpoints: &[Endpoint]) -> (Vec<ResolvedCheck>, Vec<ResolveError>) {
    let node_map: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let mut checks = Vec::new();
    let mut errors = Vec::new();
    for endpoint in endpoints {
        match resolve_one(&node_map, endpoint) {
            Ok(check) => checks.push(check),
            Err(error) => errors.push(error),
        }
    }
    (checks, errors)
}

/// Resolve a single endpoint against a node lookup map.
pub fn resolve_one(
    node_map: &HashMap<&str, &Node>,
    endpoint: &Endpoint,
) -> Result<ResolvedCheck, ResolveError> {
    // Ancestor chain, nearest (environment) first up to the root.
    let chain = ancestors(node_map, &endpoint.environment).ok_or_else(|| {
        ResolveError::MissingEnvironment {
            endpoint: endpoint.id.clone(),
            environment: endpoint.environment.clone(),
        }
    })?;

    let interval = endpoint
        .interval
        .or_else(|| chain.iter().find_map(|n| n.defaults.interval))
        .unwrap_or(DEFAULT_INTERVAL);
    let timeout = endpoint
        .timeout
        .or_else(|| chain.iter().find_map(|n| n.defaults.timeout))
        .unwrap_or(DEFAULT_TIMEOUT);

    // Tags: root-first so higher levels come first, then endpoint tags.
    let mut tags = Vec::new();
    for node in chain.iter().rev() {
        tags.push(format!("{}:{}", node.kind, node.name));
        tags.extend(node.defaults.tags.iter().cloned());
    }
    tags.extend(endpoint.tags.iter().cloned());
    dedup_preserving_order(&mut tags);

    Ok(ResolvedCheck {
        id: endpoint.id.clone(),
        interval,
        timeout,
        tags,
        target: endpoint.target.clone(),
        assertions: endpoint.assertions.clone(),
    })
}

/// The ancestor chain of `start`, nearest first. `None` if `start` is unknown.
fn ancestors<'a>(node_map: &HashMap<&str, &'a Node>, start: &str) -> Option<Vec<&'a Node>> {
    let first = node_map.get(start).copied()?;
    let mut chain = vec![first];
    let mut current = first.parent.as_deref();
    // Bound the walk so a malformed cycle cannot loop forever.
    for _ in 0..1024 {
        let Some(parent_id) = current else { break };
        let Some(parent) = node_map.get(parent_id).copied() else {
            break;
        };
        if chain.iter().any(|n| n.id == parent.id) {
            break; // cycle guard
        }
        chain.push(parent);
        current = parent.parent.as_deref();
    }
    Some(chain)
}

fn dedup_preserving_order(tags: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    tags.retain(|t| seen.insert(t.clone()));
}
