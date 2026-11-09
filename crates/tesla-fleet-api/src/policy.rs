// SPDX-License-Identifier: MIT OR Apache-2.0

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::header::HeaderMap;

use crate::RateLimit;

/// Which calls may be retried after HTTP 429.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CallClass {
    /// Safe to send again. GET, and POST queries that do not change vehicle state.
    Idempotent,
    /// Wake, registration, and telemetry writes. Each call can bill or change configuration.
    Mutation,
}

/// How [`crate::FleetClient`] treats HTTP 429.
///
/// The default does not retry. Wakes are limited to three per minute and device-data calls are
/// billed, so hiding a 429 behind a sleep is opt-in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestPolicy {
    /// Extra attempts after the first 429 for read-only requests.
    pub max_rate_limit_retries: u32,
    /// Longest `Retry-After` the client will sleep for. Longer delays are returned to the caller.
    pub max_rate_limit_wait: Duration,
    /// Also retry wakes, telemetry writes, and partner registration. Leave this `false` unless
    /// duplicate calls are acceptable.
    pub retry_non_idempotent: bool,
}

impl Default for RequestPolicy {
    fn default() -> Self {
        Self {
            max_rate_limit_retries: 0,
            max_rate_limit_wait: Duration::from_secs(2),
            retry_non_idempotent: false,
        }
    }
}

pub(crate) fn parse_rate_limit(headers: &HeaderMap, now: SystemTime) -> RateLimit {
    let mut retry_after = None;
    let mut limit = None;
    let mut remaining = None;
    for (name, value) in headers {
        let Ok(text) = value.to_str() else {
            continue;
        };
        match header_kind(name.as_str()) {
            HeaderKind::Delay => {
                if let Some(delay) = parse_delay(text, now) {
                    retry_after = Some(max_delay(retry_after, delay));
                }
            }
            HeaderKind::Limit => {
                if let Ok(parsed) = text.trim().parse::<u64>() {
                    limit = Some(min_num(limit, parsed));
                }
            }
            HeaderKind::Remaining => {
                if let Ok(parsed) = text.trim().parse::<u64>() {
                    remaining = Some(min_num(remaining, parsed));
                }
            }
            HeaderKind::Other => {}
        }
    }
    RateLimit {
        retry_after,
        limit,
        remaining,
    }
}

enum HeaderKind {
    Delay,
    Limit,
    Remaining,
    Other,
}

fn header_kind(name: &str) -> HeaderKind {
    if name == "retry-after" {
        return HeaderKind::Delay;
    }
    let Some(rest) = name.strip_prefix("ratelimit-") else {
        return HeaderKind::Other;
    };
    if rest == "reset" || rest.ends_with("-reset") {
        HeaderKind::Delay
    } else if rest == "limit" || rest.ends_with("-limit") {
        HeaderKind::Limit
    } else if rest == "remaining" || rest.ends_with("-remaining") {
        HeaderKind::Remaining
    } else {
        HeaderKind::Other
    }
}

fn parse_delay(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(delay_from_number(seconds, now));
    }
    let date = httpdate::parse_http_date(value).ok()?;
    Some(date.duration_since(now).unwrap_or(Duration::ZERO))
}

fn delay_from_number(seconds: u64, now: SystemTime) -> Duration {
    // Values past 2001-09-09 are unix timestamps. Smaller values are delays in seconds,
    // which is how Tesla's `Retry-After` and `RateLimit-Reset` headers are documented.
    if seconds > 1_000_000_000 {
        let instant = UNIX_EPOCH + Duration::from_secs(seconds);
        return instant.duration_since(now).unwrap_or(Duration::ZERO);
    }
    Duration::from_secs(seconds)
}

fn max_delay(current: Option<Duration>, next: Duration) -> Duration {
    current.map_or(next, |current| current.max(next))
}

fn min_num(current: Option<u64>, next: u64) -> u64 {
    current.map_or(next, |current| current.min(next))
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use reqwest::header::{HeaderMap, HeaderValue};

    use super::parse_rate_limit;

    #[test]
    fn parses_retry_after_delay_and_http_date() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mut headers = HeaderMap::new();
        headers.insert("Retry-After", HeaderValue::from_static("7"));
        headers.insert("RateLimit-Remaining", HeaderValue::from_static("0"));
        headers.insert("RateLimit-Limit", HeaderValue::from_static("60"));
        let info = parse_rate_limit(&headers, now);
        assert_eq!(info.retry_after, Some(Duration::from_secs(7)));
        assert_eq!(info.remaining, Some(0));
        assert_eq!(info.limit, Some(60));

        let mut dated = HeaderMap::new();
        dated.insert(
            "Retry-After",
            HeaderValue::from_static("Thu, 16 Nov 2023 00:00:07 GMT"),
        );
        // 1700000007 - 1700000000 = 7, and that date is 2023-11-14, not the 16th.
        // Use a date computed from the fixed `now` via httpdate so the assertion is exact.
        let when = now + Duration::from_secs(9);
        let formatted = httpdate::fmt_http_date(when);
        dated.insert(
            "Retry-After",
            HeaderValue::from_str(&formatted).expect("header"),
        );
        let info = parse_rate_limit(&dated, now);
        assert_eq!(info.retry_after, Some(Duration::from_secs(9)));
    }
}
