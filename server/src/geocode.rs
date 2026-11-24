// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::time::Duration;

use serde_json::Value;
use tokio::sync::Mutex;
use tokio::time::Instant;
use url::Url;

use crate::config::Config;
use crate::error::AppError;

/// Reverse-geocode result stored in `addresses`.
#[derive(Clone, Debug)]
pub(crate) struct ParsedAddress {
    pub display_name: Option<String>,
    pub name: Option<String>,
    pub house_number: Option<String>,
    pub road: Option<String>,
    pub neighbourhood: Option<String>,
    pub city: Option<String>,
    pub county: Option<String>,
    pub state: Option<String>,
    pub state_district: Option<String>,
    pub postcode: Option<String>,
    pub country: Option<String>,
    pub osm_id: Option<i64>,
    pub osm_type: Option<String>,
    pub raw: Value,
}

/// Nominatim client with a cache key and a minimum interval between requests.
///
/// Enabled flag, base URL, and interval are read from the runtime snapshot on
/// each reverse lookup so settings changes apply without a restart.
pub(crate) struct Geocoder {
    http: reqwest::Client,
    runtime: Option<tokio::sync::watch::Receiver<crate::settings::RuntimeSnapshot>>,
    /// Fallback when no runtime watch is attached (tests, disabled).
    fallback_enabled: bool,
    fallback_base: Url,
    fallback_interval: Duration,
    last: Mutex<Option<Instant>>,
}

impl Geocoder {
    pub(crate) fn from_runtime(
        runtime: tokio::sync::watch::Receiver<crate::settings::RuntimeSnapshot>,
        config: &Config,
    ) -> Result<Self, AppError> {
        let snap = runtime.borrow().clone();
        let base = Url::parse(snap.nominatim_url.trim_end_matches('/'))
            .map_err(|_| AppError::message("NOMINATIM_URL is invalid"))?;
        if snap.nominatim_enabled && base.scheme() != "https" {
            return Err(AppError::message("NOMINATIM_URL must be https"));
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(15))
            .user_agent(user_agent(config.nominatim_contact.as_deref()))
            .build()
            .map_err(|_| AppError::message("could not build the geocoder http client"))?;
        Ok(Self {
            http,
            runtime: Some(runtime),
            fallback_enabled: snap.nominatim_enabled,
            fallback_base: base,
            fallback_interval: snap.nominatim_min_interval,
            last: Mutex::new(None),
        })
    }

    #[allow(dead_code)]
    pub(crate) fn disabled() -> Self {
        Self {
            http: reqwest::Client::new(),
            runtime: None,
            fallback_enabled: false,
            fallback_base: Url::parse("https://nominatim.openstreetmap.org").expect("static url"),
            fallback_interval: Duration::from_secs(1),
            last: Mutex::new(None),
        }
    }

    pub(crate) async fn reverse(
        &self,
        latitude: f64,
        longitude: f64,
    ) -> Result<Option<ParsedAddress>, AppError> {
        let (enabled, base, min_interval) = self.live_params()?;
        if !enabled || !latitude.is_finite() || !longitude.is_finite() {
            return Ok(None);
        }
        self.wait_turn(min_interval).await;
        let mut url = base
            .join("reverse")
            .map_err(|_| AppError::message("NOMINATIM_URL is invalid"))?;
        url.query_pairs_mut()
            .append_pair("format", "jsonv2")
            .append_pair("addressdetails", "1")
            .append_pair("lat", &format!("{latitude:.7}"))
            .append_pair("lon", &format!("{longitude:.7}"));
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|_| AppError::message("nominatim request failed"))?;
        if !response.status().is_success() {
            return Err(AppError::message(format!(
                "nominatim status {}",
                response.status()
            )));
        }
        let body: Value = response
            .json()
            .await
            .map_err(|_| AppError::message("nominatim response was not json"))?;
        Ok(parse_nominatim(&body))
    }

    fn live_params(&self) -> Result<(bool, Url, Duration), AppError> {
        if let Some(runtime) = &self.runtime {
            let snap = runtime.borrow().clone();
            let base = Url::parse(snap.nominatim_url.trim_end_matches('/'))
                .map_err(|_| AppError::message("NOMINATIM_URL is invalid"))?;
            if snap.nominatim_enabled && base.scheme() != "https" {
                return Err(AppError::message("NOMINATIM_URL must be https"));
            }
            return Ok((snap.nominatim_enabled, base, snap.nominatim_min_interval));
        }
        Ok((
            self.fallback_enabled,
            self.fallback_base.clone(),
            self.fallback_interval,
        ))
    }

    async fn wait_turn(&self, min_interval: Duration) {
        let mut last = self.last.lock().await;
        if let Some(previous) = *last
            && let Some(wait) = min_interval.checked_sub(previous.elapsed())
        {
            tokio::time::sleep(wait).await;
        }
        *last = Some(Instant::now());
    }
}

pub(crate) fn cache_key(latitude: f64, longitude: f64) -> String {
    format!("{latitude:.4},{longitude:.4}")
}

pub(crate) fn parse_nominatim(value: &Value) -> Option<ParsedAddress> {
    if value.get("error").is_some() {
        return None;
    }
    let address = value.get("address")?.as_object()?;
    let text = |key: &str| {
        address
            .get(key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned)
    };
    let city = text("city")
        .or_else(|| text("town"))
        .or_else(|| text("village"))
        .or_else(|| text("municipality"));
    Some(ParsedAddress {
        display_name: value
            .get("display_name")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        name: text("amenity").or_else(|| text("building")),
        house_number: text("house_number"),
        road: text("road").or_else(|| text("pedestrian")),
        neighbourhood: text("neighbourhood").or_else(|| text("suburb")),
        city,
        county: text("county"),
        state: text("state"),
        state_district: text("state_district"),
        postcode: text("postcode"),
        country: text("country"),
        osm_id: value.get("osm_id").and_then(json_i64),
        osm_type: value
            .get("osm_type")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned),
        raw: value.clone(),
    })
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn user_agent(contact: Option<&str>) -> String {
    match contact.filter(|value| !value.is_empty()) {
        Some(contact) => format!("tesla-logger/0.1 (self-hosted; {contact})"),
        None => "tesla-logger/0.1 (self-hosted)".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{cache_key, parse_nominatim};
    use serde_json::json;

    #[test]
    fn parses_a_nominatim_payload_and_rounds_the_cache_key() {
        let parsed = parse_nominatim(&json!({
            "display_name": "1 Rue de Rivoli, Paris, France",
            "osm_id": 42,
            "osm_type": "way",
            "address": {
                "house_number": "1",
                "road": "Rue de Rivoli",
                "city": "Paris",
                "postcode": "75001",
                "country": "France"
            }
        }))
        .unwrap();
        assert_eq!(parsed.city.as_deref(), Some("Paris"));
        assert_eq!(parsed.road.as_deref(), Some("Rue de Rivoli"));
        assert_eq!(parsed.osm_id, Some(42));
        assert_eq!(cache_key(48.856_64, 2.352_21), "48.8566,2.3522");
        assert!(parse_nominatim(&json!({"error": "Unable to geocode"})).is_none());
    }
}
