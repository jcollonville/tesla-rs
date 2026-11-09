// SPDX-License-Identifier: MIT OR Apache-2.0

/// Failure from Owner API login, refresh, or the streaming socket setup.
///
/// Display text does not include access tokens, refresh tokens, authorization codes, or PKCE
/// verifiers. A transport error may include the request URL. Token requests put secrets in the
/// body, not the URL.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OwnerError {
    /// The HTTP client could not complete the call.
    #[error("owner API request failed")]
    Transport(#[source] reqwest::Error),
    /// Tesla rejected the grant or the callback reported an error.
    #[error("owner API rejected the sign-in ({status})")]
    Rejected {
        /// HTTP status, or `0` when the error was on the pasted redirect URL.
        status: u16,
    },
    /// The token or vehicle response was missing a required field.
    #[error("owner API response was incomplete")]
    Malformed,
    /// The pasted redirect `state` does not match this login.
    #[error("owner sign-in state did not match")]
    StateMismatch,
    /// The pasted redirect did not include an authorization code.
    #[error("owner sign-in did not include a code")]
    MissingCode,
    /// A token or vehicle URL was not HTTPS (loopback HTTP is allowed for tests).
    #[error("owner URL must be https, or http on loopback")]
    InsecureUrl,
    /// The streaming URL was not `wss` (loopback `ws` is allowed for tests).
    #[error("stream URL must be wss, or ws on loopback")]
    InsecureStreamUrl,
    /// The refresh token was rejected. The user has to sign in again.
    #[error("owner token can no longer be refreshed")]
    LoginRequired,
    /// The system RNG failed.
    #[error("system entropy failed")]
    Entropy,
}

impl OwnerError {
    /// Whether the user must sign in again.
    #[must_use]
    pub const fn is_login_required(&self) -> bool {
        matches!(self, Self::LoginRequired)
    }
}
