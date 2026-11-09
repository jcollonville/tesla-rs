// SPDX-License-Identifier: MIT OR Apache-2.0

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};

use crate::OwnerError;

#[derive(Clone)]
pub(crate) struct Pkce {
    verifier: SecretString,
    challenge: String,
}

impl Pkce {
    pub(crate) fn generate() -> Result<Self, OwnerError> {
        let mut bytes = [0_u8; 32];
        getrandom::getrandom(&mut bytes).map_err(|_| OwnerError::Entropy)?;
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let digest = Sha256::digest(verifier.as_bytes());
        Ok(Self {
            verifier: SecretString::from(verifier),
            challenge: URL_SAFE_NO_PAD.encode(digest),
        })
    }

    pub(crate) fn challenge(&self) -> &str {
        &self.challenge
    }

    pub(crate) fn verifier(&self) -> &str {
        self.verifier.expose_secret()
    }
}

impl std::fmt::Debug for Pkce {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Pkce")
            .field("verifier", &"<redacted>")
            .field("challenge", &self.challenge)
            .finish()
    }
}

pub(crate) fn random_state() -> Result<String, OwnerError> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| OwnerError::Entropy)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub(crate) fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (left, right) in left.bytes().zip(right.bytes()) {
        diff |= left ^ right;
    }
    diff == 0
}
