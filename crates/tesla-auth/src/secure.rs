// SPDX-License-Identifier: MIT OR Apache-2.0

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::AuthError;

pub(crate) fn random_token() -> Result<String, AuthError> {
    let mut bytes = [0_u8; 32];
    getrandom::getrandom(&mut bytes).map_err(|_| AuthError::Entropy)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// Compares strings without returning on the first differing byte.
pub(crate) fn constant_time_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    let mut diff = u8::from(left.len() != right.len());
    let len = left.len().min(right.len());
    let mut index = 0;
    while index < len {
        diff |= left[index] ^ right[index];
        index += 1;
    }
    diff == 0
}

pub(crate) fn host_is_loopback(url: &url::Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"))
}

pub(crate) fn ensure_secret_transport(url: &url::Url) -> Result<(), AuthError> {
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AuthError::InsecureEndpoint);
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" if host_is_loopback(url) => Ok(()),
        _ => Err(AuthError::InsecureEndpoint),
    }
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn equality_checks_full_string() {
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
        assert!(!constant_time_eq("abc", "abcd"));
    }
}
