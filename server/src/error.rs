// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::time::Duration;

use tesla_auth::AuthError;
use tesla_fleet_api::FleetError;

/// Failure that is safe to show to an operator.
///
/// Display text has database passwords and token-like URL userinfo removed.
/// It does not include OAuth tokens, the client secret, or the private key.
#[derive(Debug, thiserror::Error)]
pub(crate) enum AppError {
    /// A required environment variable was absent.
    #[error("missing environment variable {0}")]
    MissingEnv(&'static str),
    /// Configuration or local validation failed.
    #[error("{0}")]
    Message(String),
    /// Postgres returned an error. The text is redacted.
    #[error("database error: {0}")]
    Database(String),
    /// OAuth failed. The text comes from `tesla-auth` and does not include tokens.
    #[error("tesla auth error: {0}")]
    Auth(String),
    /// The Fleet API failed. The text does not include the access token.
    #[error("fleet api error: {0}")]
    Fleet(String),
    /// Tesla asked the client to slow down.
    #[error("rate limited; retry after {}s", retry_after.as_secs())]
    RateLimited {
        /// Delay before the next Fleet API call.
        retry_after: Duration,
    },
    /// A filesystem operation failed.
    #[error("io error: {0}")]
    Io(String),
}

impl AppError {
    pub(crate) fn message(text: impl Into<String>) -> Self {
        Self::Message(text.into())
    }
}

impl From<sqlx::Error> for AppError {
    fn from(err: sqlx::Error) -> Self {
        Self::Database(redact_secrets(&err.to_string()))
    }
}

impl From<AuthError> for AppError {
    fn from(err: AuthError) -> Self {
        Self::Auth(redact_secrets(&err.to_string()))
    }
}

impl From<sqlx::migrate::MigrateError> for AppError {
    fn from(err: sqlx::migrate::MigrateError) -> Self {
        Self::Database(redact_secrets(&err.to_string()))
    }
}

impl From<std::io::Error> for AppError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err.to_string())
    }
}

pub(crate) fn fleet_error(err: FleetError) -> AppError {
    match err {
        FleetError::RateLimited(info) => {
            let retry_after = match info.retry_after {
                Some(wait) if !wait.is_zero() => wait,
                _ => Duration::from_secs(60),
            };
            AppError::RateLimited { retry_after }
        }
        other => AppError::Fleet(redact_secrets(&other.to_string())),
    }
}

/// Replaces `scheme://user:password@` with `scheme://user:***@`.
pub(crate) fn redact_secrets(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find("://") {
        out.push_str(&rest[..index + 3]);
        rest = &rest[index + 3..];
        let Some(at) = rest.find('@') else {
            continue;
        };
        let userinfo = &rest[..at];
        if let Some(colon) = userinfo.find(':')
            && !userinfo[..colon].contains('/')
        {
            out.push_str(&userinfo[..colon]);
            out.push_str(":***");
            rest = &rest[at..];
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::redact_secrets;

    #[test]
    fn redacts_database_password() {
        let text = "error connecting to postgres://tesla:s3cret@db:5432/tesla_logger";
        let redacted = redact_secrets(text);
        assert!(!redacted.contains("s3cret"));
        assert!(redacted.contains("postgres://tesla:***@db:5432/tesla_logger"));
    }
}
