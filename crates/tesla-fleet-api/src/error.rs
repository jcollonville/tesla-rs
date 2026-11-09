// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;

use tesla_auth::AuthError;

/// Failure from a Fleet API call.
///
/// Display text uses Tesla's short `error` code and `error_description` when the body is JSON.
/// Raw bodies are not included, so a non-JSON body cannot leak a token that Tesla echoed.
#[derive(Debug)]
#[non_exhaustive]
pub enum FleetError {
    /// HTTP transport failure. The source does not include the `Authorization` header.
    Http(Box<reqwest::Error>),
    /// The response was not the JSON envelope this client understands.
    Decode,
    /// The access token was rejected or expired.
    Unauthorized {
        /// Tesla `error` code, when present.
        error: Option<String>,
        /// Tesla `error_description`, when present.
        description: Option<String>,
    },
    /// The developer account has no payment method or has exceeded its billing limit.
    PaymentRequired,
    /// The token is missing a scope, or the command requires the Vehicle Command protocol.
    Forbidden {
        /// Tesla `error` code, when present.
        error: Option<String>,
        /// Tesla `error_description`, when present.
        description: Option<String>,
    },
    /// The VIN or other resource does not exist. Cached vehicle data also returns 404 when Tesla
    /// has nothing stored.
    NotFound {
        /// Tesla `error` code, when present.
        error: Option<String>,
        /// Tesla `error_description`, when present.
        description: Option<String>,
    },
    /// The vehicle was asleep, offline, or too slow to answer (HTTP 408).
    ///
    /// This client does not call `wake_up` in response.
    DeviceUnavailable {
        /// Tesla `error` code, when present.
        error: Option<String>,
        /// Tesla `error_description`, when present.
        description: Option<String>,
    },
    /// A precondition failed. HTTP 412 is also how Tesla reports an unregistered partner account,
    /// and how `vehicle_data_cached` reports an unsupported vehicle.
    PreconditionFailed {
        /// Tesla `error` code, when present.
        error: Option<String>,
        /// Tesla `error_description`, when present.
        description: Option<String>,
    },
    /// The token's subject is not in this Fleet API region (HTTP 421).
    IncorrectRegion {
        /// Tesla `error` code, when present.
        error: Option<String>,
        /// Tesla `error_description`, when present.
        description: Option<String>,
    },
    /// The JSON body failed semantic validation (HTTP 422).
    InvalidResource {
        /// Tesla `error` code, when present.
        error: Option<String>,
        /// Tesla `error_description`, when present.
        description: Option<String>,
    },
    /// Tesla locked the account (HTTP 423).
    Locked,
    /// HTTP 429. See [`RateLimit`] for `Retry-After` and `RateLimit-*` headers.
    RateLimited(RateLimit),
    /// The query is blocked by privacy or region rules (HTTP 451).
    UnavailableForLegalReasons {
        /// Tesla `error_description`, when present.
        description: Option<String>,
    },
    /// Any other non-success status.
    Api {
        /// HTTP status code.
        status: u16,
        /// Tesla `error` code, when present.
        error: Option<String>,
        /// Tesla `error_description`, when present.
        description: Option<String>,
    },
    /// The token provider failed. The message comes from [`AuthError`] and does not include secrets.
    Token(String),
    /// A vehicle tag, domain, or request field failed local validation.
    InvalidRequest(&'static str),
}

/// Rate-limit details from `Retry-After` and `RateLimit-*` headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateLimit {
    /// Delay Tesla asked the client to wait, when a header carried one.
    pub retry_after: Option<std::time::Duration>,
    /// Smallest `RateLimit-*-Limit` value seen on the response.
    pub limit: Option<u64>,
    /// Smallest `RateLimit-*-Remaining` value seen on the response.
    pub remaining: Option<u64>,
}

impl FleetError {
    /// Delay advertised by a [`Self::RateLimited`] error.
    #[must_use]
    pub fn retry_after(&self) -> Option<std::time::Duration> {
        match self {
            Self::RateLimited(info) => info.retry_after,
            _ => None,
        }
    }

    /// Whether the vehicle did not answer because it was asleep, offline, or slow.
    #[must_use]
    pub fn is_device_unavailable(&self) -> bool {
        matches!(self, Self::DeviceUnavailable { .. })
    }

    pub(crate) fn http(err: reqwest::Error) -> Self {
        Self::Http(Box::new(err))
    }
}

impl fmt::Display for FleetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Http(_) => f.write_str("http request failed"),
            Self::Decode => f.write_str("fleet api response could not be decoded"),
            Self::Unauthorized { error, description } => {
                write_status(f, "unauthorized", error.as_ref(), description.as_ref())
            }
            Self::PaymentRequired => f.write_str("payment required"),
            Self::Forbidden { error, description } => {
                write_status(f, "forbidden", error.as_ref(), description.as_ref())
            }
            Self::NotFound { error, description } => {
                write_status(f, "not found", error.as_ref(), description.as_ref())
            }
            Self::DeviceUnavailable { error, description } => write_status(
                f,
                "vehicle unavailable",
                error.as_ref(),
                description.as_ref(),
            ),
            Self::PreconditionFailed { error, description } => write_status(
                f,
                "precondition failed",
                error.as_ref(),
                description.as_ref(),
            ),
            Self::IncorrectRegion { error, description } => {
                write_status(f, "incorrect region", error.as_ref(), description.as_ref())
            }
            Self::InvalidResource { error, description } => {
                write_status(f, "invalid resource", error.as_ref(), description.as_ref())
            }
            Self::Locked => f.write_str("account locked"),
            Self::RateLimited(info) => match info.retry_after {
                Some(delay) => write!(f, "rate limited; retry after {}s", delay.as_secs()),
                None => f.write_str("rate limited"),
            },
            Self::UnavailableForLegalReasons { description } => write_status(
                f,
                "unavailable for legal reasons",
                None,
                description.as_ref(),
            ),
            Self::Api {
                status,
                error,
                description,
            } => write_status(
                f,
                &format!("fleet api status {status}"),
                error.as_ref(),
                description.as_ref(),
            ),
            Self::Token(message) => write!(f, "access token error: {message}"),
            Self::InvalidRequest(message) => f.write_str(message),
        }
    }
}

fn write_status(
    f: &mut fmt::Formatter<'_>,
    label: &str,
    error: Option<&String>,
    description: Option<&String>,
) -> fmt::Result {
    f.write_str(label)?;
    if let Some(error) = error.filter(|text| !text.is_empty()) {
        write!(f, " ({error})")?;
    }
    if let Some(description) = description.filter(|text| !text.is_empty()) {
        write!(f, ": {description}")?;
    }
    Ok(())
}

impl std::error::Error for FleetError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(err) => Some(err),
            _ => None,
        }
    }
}

impl From<AuthError> for FleetError {
    fn from(err: AuthError) -> Self {
        Self::Token(err.to_string())
    }
}
