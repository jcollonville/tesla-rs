// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};

use crate::AuthError;

/// PKCE pair using the `S256` method (RFC 7636).
///
/// The Tesla third-party OpenID configuration advertises `code_challenge_methods_supported: ["S256"]`.
/// The HTML parameter tables omit PKCE; this client sends it on every authorization-code login.
#[derive(Clone)]
pub struct Pkce {
    verifier: SecretString,
    challenge: String,
}

impl Pkce {
    /// Generates a 32-byte verifier.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::Entropy`] when the system RNG fails.
    pub fn generate() -> Result<Self, AuthError> {
        let mut bytes = [0_u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|_| AuthError::Entropy)?;
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        Self::from_verifier(verifier)
    }

    /// Builds a challenge from an existing verifier.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::InvalidPkceVerifier`] when `verifier` is not 43–128 characters from
    /// the RFC 7636 unreserved set.
    pub fn from_verifier(verifier: impl Into<String>) -> Result<Self, AuthError> {
        let verifier = verifier.into();
        if !valid_verifier(&verifier) {
            return Err(AuthError::InvalidPkceVerifier);
        }
        let digest = Sha256::digest(verifier.as_bytes());
        let challenge = URL_SAFE_NO_PAD.encode(digest);
        Ok(Self {
            verifier: SecretString::from(verifier),
            challenge,
        })
    }

    /// `code_challenge` value. Safe to place in the authorize URL and in logs.
    #[must_use]
    pub fn challenge(&self) -> &str {
        &self.challenge
    }

    pub(crate) fn expose_verifier(&self) -> &str {
        self.verifier.expose_secret()
    }
}

impl fmt::Debug for Pkce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Pkce")
            .field("verifier", &"<redacted>")
            .field("challenge", &self.challenge)
            .finish()
    }
}

fn valid_verifier(verifier: &str) -> bool {
    let len = verifier.chars().count();
    (43..=128).contains(&len)
        && verifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~'))
}

#[cfg(test)]
mod tests {
    use super::Pkce;

    #[test]
    fn matches_rfc7636_appendix_b() {
        let pkce =
            Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").expect("verifier");
        assert_eq!(
            pkce.challenge(),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        let rendered = format!("{pkce:?}");
        assert!(!rendered.contains("dBjftJeZ4CVP"));
        assert!(rendered.contains("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
    }

    #[test]
    fn generate_is_s256_and_redacted() {
        let pkce = Pkce::generate().expect("entropy");
        assert_eq!(pkce.challenge().len(), 43);
        assert!(!format!("{pkce:?}").contains(pkce.expose_verifier()));
    }
}
