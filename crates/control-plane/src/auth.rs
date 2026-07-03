//! Writer authentication (T24, I.auth).
//!
//! Writers (Terraform/CI) authenticate to the control plane and are scoped to a
//! single product (V31). Verification is behind the [`WriterAuth`] trait so the
//! API and authorization logic are tested with a fake, and the concrete OIDC
//! verifier is a separate, isolated impl (deferred). [`StaticTokenAuth`] is a
//! simple token→product map — enough for tests and small single-team setups.

use std::collections::HashMap;
use std::future::Future;

/// An authenticated writer and the product it may mutate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterIdentity {
    /// The product this identity is authorized to write under.
    pub product: String,
}

/// Errors authenticating a writer.
#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    /// No credentials were presented.
    #[error("missing credentials")]
    Missing,
    /// Credentials were presented but rejected.
    #[error("invalid credentials")]
    Invalid,
}

/// Verifies writer credentials (an OIDC token in production).
pub trait WriterAuth: Send + Sync {
    /// Authenticate a bearer token into a [`WriterIdentity`].
    fn authenticate(
        &self,
        token: &str,
    ) -> impl Future<Output = Result<WriterIdentity, AuthError>> + Send;
}

/// A [`WriterAuth`] backed by a static token→product map.
#[derive(Debug, Default, Clone)]
pub struct StaticTokenAuth {
    tokens: HashMap<String, String>,
}

impl StaticTokenAuth {
    /// Create an empty map.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `token` as authorized to write under `product`.
    pub fn with(mut self, token: &str, product: &str) -> Self {
        self.tokens.insert(token.to_string(), product.to_string());
        self
    }
}

impl WriterAuth for StaticTokenAuth {
    async fn authenticate(&self, token: &str) -> Result<WriterIdentity, AuthError> {
        match self.tokens.get(token) {
            // Fail closed on an empty product (e.g. a misconfigured `token:`
            // mapping): an empty product would match any id with an empty first
            // path segment (`/foo`), so it must never authorize anything (#12).
            Some(product) if !product.is_empty() => Ok(WriterIdentity {
                product: product.clone(),
            }),
            _ => Err(AuthError::Invalid),
        }
    }
}

/// An authenticated worker (identified by its mTLS client cert in production).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerIdentity {
    /// The worker's id.
    pub worker_id: String,
}

/// Verifies worker credentials (an mTLS client certificate in production).
pub trait WorkerAuth: Send + Sync {
    /// Authenticate a worker credential into a [`WorkerIdentity`].
    fn authenticate(
        &self,
        credential: &str,
    ) -> impl Future<Output = Result<WorkerIdentity, AuthError>> + Send;
}

/// A [`WorkerAuth`] that accepts any non-empty credential, treating it as the
/// worker id. Stands in for mTLS in tests and simple setups.
#[derive(Debug, Default, Clone)]
pub struct AcceptWorkerAuth;

impl WorkerAuth for AcceptWorkerAuth {
    async fn authenticate(&self, credential: &str) -> Result<WorkerIdentity, AuthError> {
        if credential.is_empty() {
            Err(AuthError::Missing)
        } else {
            Ok(WorkerIdentity {
                worker_id: credential.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    // Panicking is the failure signal in tests (ADR 0008).
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[tokio::test]
    async fn empty_product_token_is_rejected() {
        // A token mapped to an empty product (a `token:` misconfiguration) must
        // not authenticate — otherwise it would scope-match `/`-prefixed ids.
        let auth = StaticTokenAuth::new().with("t", "");
        assert!(auth.authenticate("t").await.is_err());
    }

    #[tokio::test]
    async fn normal_token_authenticates_to_its_product() {
        let auth = StaticTokenAuth::new().with("t", "web");
        assert_eq!(auth.authenticate("t").await.unwrap().product, "web");
    }
}
