// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

use crate::{AuthError, Scope};

/// OAuth token response, with secrets redacted in [`Debug`].
///
/// Refresh tokens are single-use and expire after three months. Tesla also accepts the most
/// recently used refresh token for up to 24 hours when an application fails to store the rotated
/// value. Always persist the token set returned by the latest successful refresh.
#[derive(Clone)]
pub struct TokenSet {
    access_token: SecretString,
    refresh_token: Option<SecretString>,
    id_token: Option<SecretString>,
    token_type: String,
    expires_at: SystemTime,
    scopes: Vec<Scope>,
}

impl TokenSet {
    /// Bearer (or other) access token.
    #[must_use]
    pub fn access_token(&self) -> &SecretString {
        &self.access_token
    }

    /// Rotating refresh token. Absent for partner client-credentials tokens.
    #[must_use]
    pub fn refresh_token(&self) -> Option<&SecretString> {
        self.refresh_token.as_ref()
    }

    /// OpenID ID token, when Tesla returned one. This crate stores it and does not validate it.
    #[must_use]
    pub fn id_token(&self) -> Option<&SecretString> {
        self.id_token.as_ref()
    }

    /// `token_type` from the token response. Fleet API calls use `Bearer`.
    #[must_use]
    pub fn token_type(&self) -> &str {
        &self.token_type
    }

    /// Builds a token set from values the caller already holds.
    ///
    /// `id_token` is left empty. Use this when importing tokens that were issued outside this
    /// client. Prefer [`FileTokenStore`](crate::FileTokenStore) for tokens this crate issued.
    #[must_use]
    pub fn bearer(
        access_token: SecretString,
        refresh_token: Option<SecretString>,
        expires_at: SystemTime,
        scopes: Vec<Scope>,
    ) -> Self {
        Self {
            access_token,
            refresh_token,
            id_token: None,
            token_type: "Bearer".to_owned(),
            expires_at,
            scopes,
        }
    }

    /// Absolute expiry time computed from `expires_in` when the response was received.
    #[must_use]
    pub fn expires_at(&self) -> SystemTime {
        self.expires_at
    }

    /// Scopes associated with the token. Taken from the token response when Tesla sends them,
    /// otherwise from the scopes requested by this client.
    #[must_use]
    pub fn scopes(&self) -> &[Scope] {
        &self.scopes
    }

    /// Whether the access token should be refreshed before use.
    ///
    /// `skew` is subtracted from the expiry so a token that is about to expire is refreshed
    /// early. A token that is already expired always needs a refresh.
    #[must_use]
    pub fn needs_refresh(&self, skew: Duration, now: SystemTime) -> bool {
        match self.expires_at.checked_sub(skew) {
            Some(deadline) => now >= deadline,
            None => true,
        }
    }

    pub(crate) fn from_response(
        response: TokenResponse,
        fallback_scopes: Vec<Scope>,
        now: SystemTime,
    ) -> Result<Self, AuthError> {
        let scopes = match response.scope {
            Some(scope) if !scope.trim().is_empty() => Scope::parse_list(&scope)?,
            _ => fallback_scopes,
        };
        let Some(expires_in) = response.expires_in else {
            return Err(AuthError::MalformedResponse);
        };
        Ok(Self {
            access_token: SecretString::from(response.access_token),
            refresh_token: response.refresh_token.map(SecretString::from),
            id_token: response.id_token.map(SecretString::from),
            token_type: response.token_type.unwrap_or_else(|| "Bearer".to_owned()),
            expires_at: now + Duration::from_secs(expires_in),
            scopes,
        })
    }

    pub(crate) fn to_file_bytes(&self) -> Result<Vec<u8>, AuthError> {
        let file = TokenFile {
            access_token: self.access_token.expose_secret().to_owned(),
            refresh_token: self
                .refresh_token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
            id_token: self
                .id_token
                .as_ref()
                .map(|token| token.expose_secret().to_owned()),
            token_type: self.token_type.clone(),
            expires_at: system_time_secs(self.expires_at),
            scopes: self.scopes.iter().map(ToString::to_string).collect(),
        };
        serde_json::to_vec_pretty(&file).map_err(AuthError::token_store)
    }

    pub(crate) fn from_file_bytes(bytes: &[u8]) -> Result<Self, AuthError> {
        let mut file: TokenFile = serde_json::from_slice(bytes)
            .map_err(|_| AuthError::token_store("token file is not a valid token set"))?;
        let scope_names = std::mem::take(&mut file.scopes);
        let mut scopes = Vec::with_capacity(scope_names.len());
        for scope in scope_names {
            scopes.push(Scope::new(&scope)?);
        }
        Ok(Self {
            access_token: SecretString::from(std::mem::take(&mut file.access_token)),
            refresh_token: file.refresh_token.take().map(SecretString::from),
            id_token: file.id_token.take().map(SecretString::from),
            token_type: std::mem::take(&mut file.token_type),
            expires_at: UNIX_EPOCH + Duration::from_secs(file.expires_at),
            scopes,
        })
    }
}

impl fmt::Debug for TokenSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenSet")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("id_token", &self.id_token.as_ref().map(|_| "<redacted>"))
            .field("token_type", &self.token_type)
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .finish()
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct TokenResponse {
    pub(crate) access_token: String,
    #[serde(default)]
    pub(crate) refresh_token: Option<String>,
    #[serde(default)]
    pub(crate) id_token: Option<String>,
    #[serde(default)]
    pub(crate) token_type: Option<String>,
    #[serde(default)]
    pub(crate) expires_in: Option<u64>,
    #[serde(default)]
    pub(crate) scope: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct TokenFile {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    id_token: Option<String>,
    token_type: String,
    expires_at: u64,
    #[serde(default)]
    scopes: Vec<String>,
}

impl Drop for TokenFile {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(token) = self.refresh_token.as_mut() {
            token.zeroize();
        }
        if let Some(token) = self.id_token.as_mut() {
            token.zeroize();
        }
    }
}

fn system_time_secs(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use secrecy::ExposeSecret;

    use super::TokenSet;
    use crate::Scope;

    #[test]
    fn debug_and_file_round_trip_hide_secrets_in_debug() {
        let response = super::TokenResponse {
            access_token: "access-secret".to_owned(),
            refresh_token: Some("refresh-secret".to_owned()),
            id_token: Some("id-secret".to_owned()),
            token_type: Some("Bearer".to_owned()),
            expires_in: Some(3600),
            scope: Some("openid offline_access".to_owned()),
        };
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let tokens = TokenSet::from_response(response, Vec::new(), now).unwrap();
        let rendered = format!("{tokens:?}");
        assert!(!rendered.contains("access-secret"));
        assert!(!rendered.contains("refresh-secret"));
        assert!(!rendered.contains("id-secret"));
        assert_eq!(tokens.scopes(), &[Scope::OpenId, Scope::OfflineAccess][..]);
        assert!(!tokens.needs_refresh(Duration::from_secs(60), now));
        assert!(tokens.needs_refresh(Duration::from_secs(60), now + Duration::from_secs(3600)));

        let bytes = tokens.to_file_bytes().unwrap();
        let loaded = TokenSet::from_file_bytes(&bytes).unwrap();
        assert_eq!(loaded.access_token().expose_secret(), "access-secret");
        assert!(!format!("{loaded:?}").contains("access-secret"));
    }
}
