// SPDX-License-Identifier: MIT OR Apache-2.0

use std::future::Future;

use secrecy::SecretString;
use tesla_auth::{Session, TokenStore};

use crate::FleetError;

/// Supplies the bearer token for Fleet API calls.
///
/// [`BearerToken`] is a fixed token. [`Session`] refreshes a user token and stores the rotation.
/// Partner tokens should use [`BearerToken`] (or a separate session) so they do not overwrite the
/// user token file.
pub trait AccessTokenProvider: Send + Sync {
    /// Returns an access token that is valid at the time of the call.
    fn access_token(&self) -> impl Future<Output = Result<SecretString, FleetError>> + Send;
}

/// Access token that does not refresh.
#[derive(Clone)]
pub struct BearerToken(SecretString);

impl BearerToken {
    /// Stores `token` as a secret.
    #[must_use]
    pub fn new(token: impl Into<String>) -> Self {
        Self(SecretString::from(token.into()))
    }

    /// Stores an existing secret without copying it through a [`String`] first when the caller
    /// already has one.
    #[must_use]
    pub fn from_secret(token: SecretString) -> Self {
        Self(token)
    }
}

impl std::fmt::Debug for BearerToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BearerToken(<redacted>)")
    }
}

impl AccessTokenProvider for BearerToken {
    fn access_token(&self) -> impl Future<Output = Result<SecretString, FleetError>> + Send {
        std::future::ready(Ok(self.0.clone()))
    }
}

impl<S> AccessTokenProvider for Session<S>
where
    S: TokenStore,
{
    async fn access_token(&self) -> Result<SecretString, FleetError> {
        Session::access_token(self).await.map_err(FleetError::from)
    }
}
