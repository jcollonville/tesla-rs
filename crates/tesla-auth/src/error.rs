// SPDX-License-Identifier: MIT OR Apache-2.0

use std::error::Error;
use std::fmt;

/// Failure from the OAuth client, token store, or local callback listener.
///
/// Display text includes OAuth error codes and Tesla's `error_description`. It does not include
/// access tokens, refresh tokens, authorization codes, PKCE verifiers, or client secrets unless
/// Tesla placed that material in `error_description` itself.
#[derive(Debug)]
#[non_exhaustive]
pub enum AuthError {
    /// Tesla rejected the grant.
    Oauth {
        /// HTTP status, or `0` when the error arrived on the browser callback.
        status: u16,
        /// OAuth `error` code, for example `invalid_auth_code`.
        error: String,
        /// OAuth `error_description`, when Tesla sent one.
        description: Option<String>,
    },
    /// The refresh token can no longer be used. The user must sign in again.
    ///
    /// Tesla returns this when the refresh token expired, was rotated out, the user reset their
    /// password, or the user revoked the application.
    LoginRequired {
        /// Server-supplied description, when present.
        description: Option<String>,
    },
    /// No tokens are stored.
    NotAuthenticated,
    /// The callback `state` does not match the value generated for this login.
    StateMismatch,
    /// The callback did not include an authorization code.
    MissingAuthorizationCode,
    /// `client_id`, a secret, a redirect URI, or a scope failed validation.
    InvalidConfig(&'static str),
    /// A scope name is empty or contains unsupported characters.
    InvalidScope,
    /// The region string is not `na`, `eu`, or `cn`.
    UnknownRegion,
    /// Token or authorize URL is not HTTPS and not an HTTP loopback URL.
    InsecureEndpoint,
    /// Redirect URI is not HTTPS and not an HTTP loopback URL.
    InsecureRedirectUri,
    /// A URL constant or override could not be parsed.
    InvalidUrl,
    /// The operating system could not supply random bytes.
    Entropy,
    /// PKCE verifier is not 43–128 unreserved characters.
    InvalidPkceVerifier,
    /// Reading or writing the token store failed.
    ///
    /// The message describes the I/O or parse failure and does not include token material.
    TokenStore {
        /// Failure description without token contents.
        message: String,
    },
    /// The loopback callback listener failed.
    Callback {
        /// Failure description. Query values from the callback are not included.
        message: String,
    },
    /// The callback listener waited until the deadline without a matching request.
    Timeout,
    /// Transport failure. The source does not include request headers or the body.
    Http(Box<reqwest::Error>),
    /// The token endpoint returned JSON without `access_token` or `expires_in`.
    MalformedResponse,
}

impl AuthError {
    /// Returns whether the user has to start the authorization-code flow again.
    #[must_use]
    pub fn is_login_required(&self) -> bool {
        matches!(self, Self::LoginRequired { .. })
    }

    pub(crate) fn token_store(err: impl fmt::Display) -> Self {
        Self::TokenStore {
            message: err.to_string(),
        }
    }

    pub(crate) fn callback(err: impl fmt::Display) -> Self {
        Self::Callback {
            message: err.to_string(),
        }
    }

    pub(crate) fn http(err: reqwest::Error) -> Self {
        Self::Http(Box::new(err))
    }
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oauth {
                error, description, ..
            } => write_labeled(f, "oauth error", error, description.as_ref()),
            Self::LoginRequired { description } => {
                write_labeled(f, "login required", "", description.as_ref())
            }
            Self::NotAuthenticated => f.write_str("not authenticated"),
            Self::StateMismatch => f.write_str("oauth state mismatch"),
            Self::MissingAuthorizationCode => {
                f.write_str("oauth callback is missing the authorization code")
            }
            Self::InvalidConfig(message) => f.write_str(message),
            Self::InvalidScope => f.write_str("invalid oauth scope"),
            Self::UnknownRegion => f.write_str("unknown fleet api region"),
            Self::InsecureEndpoint => f.write_str("refusing insecure oauth endpoint"),
            Self::InsecureRedirectUri => f.write_str("refusing insecure redirect uri"),
            Self::InvalidUrl => f.write_str("invalid url"),
            Self::Entropy => f.write_str("entropy source failed"),
            Self::InvalidPkceVerifier => f.write_str("invalid pkce verifier"),
            Self::TokenStore { message } => write!(f, "token store error: {message}"),
            Self::Callback { message } => write!(f, "oauth callback error: {message}"),
            Self::Timeout => f.write_str("timed out waiting for the oauth callback"),
            Self::Http(_) => f.write_str("http request failed"),
            Self::MalformedResponse => f.write_str("token response is missing a required field"),
        }
    }
}

fn write_labeled(
    f: &mut fmt::Formatter<'_>,
    label: &str,
    code: &str,
    description: Option<&String>,
) -> fmt::Result {
    if code.is_empty() {
        f.write_str(label)?;
    } else {
        write!(f, "{label} {code}")?;
    }
    if let Some(description) = description.filter(|text| !text.is_empty()) {
        write!(f, ": {description}")?;
    }
    Ok(())
}

impl Error for AuthError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Http(err) => Some(err),
            _ => None,
        }
    }
}

impl From<url::ParseError> for AuthError {
    fn from(_: url::ParseError) -> Self {
        Self::InvalidUrl
    }
}
