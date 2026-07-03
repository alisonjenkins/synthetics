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
            Some(product) => Ok(WriterIdentity {
                product: product.clone(),
            }),
            None => Err(AuthError::Invalid),
        }
    }
}
