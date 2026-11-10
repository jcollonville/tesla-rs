// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use secrecy::SecretString;
use tesla_auth::Region;
use url::Url;

use crate::error::{AppError, redact_secrets};

/// Process configuration. Secrets are redacted in [`Debug`].
pub(crate) struct Config {
    pub database_url: String,
    pub listen_addr: SocketAddr,
    pub domain: String,
    pub public_base_url: String,
    pub redirect_uri: String,
    pub region: Region,
    pub client_id: String,
    pub client_secret: SecretString,
    pub private_key_path: PathBuf,
    pub token_store_path: PathBuf,
    pub token_encryption_key: Option<String>,
    pub ui_password: SecretString,
    pub drive_poll: Duration,
    pub charge_poll: Duration,
    pub parked_poll: Duration,
    pub asleep_poll: Duration,
    pub backoff_poll: Duration,
    pub parked_live_limit: u32,
    pub gps_fallback_poll: Duration,
    pub nominatim_enabled: bool,
    pub nominatim_url: String,
    pub nominatim_contact: Option<String>,
    pub nominatim_min_interval: Duration,
    pub mqtt_enabled: bool,
    pub mqtt_host: String,
    pub mqtt_port: u16,
    pub mqtt_username: Option<String>,
    pub mqtt_password: Option<SecretString>,
    pub mqtt_prefix: String,
    pub telemetry_enabled: bool,
    pub telemetry_bind: SocketAddr,
    pub telemetry_cert_path: Option<PathBuf>,
    pub telemetry_key_path: Option<PathBuf>,
    pub owner_stream_enabled: bool,
    pub owner_token_store_path: PathBuf,
    /// IANA zone for stats buckets. `TESLA_TIMEZONE`, default `Europe/Paris`.
    pub timezone: String,
    /// Optional currency per kWh used when a charge has no stored cost.
    pub energy_price_per_kwh: Option<f64>,
    /// Keys present in the environment or dotenv file. Values are not stored here.
    explicit_env: BTreeSet<String>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Config")
            .field("database_url", &redact_secrets(&self.database_url))
            .field("listen_addr", &self.listen_addr)
            .field("domain", &self.domain)
            .field("public_base_url", &self.public_base_url)
            .field("redirect_uri", &self.redirect_uri)
            .field("region", &self.region)
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("private_key_path", &self.private_key_path)
            .field("token_store_path", &self.token_store_path)
            .field(
                "token_encryption_key",
                &self.token_encryption_key.as_ref().map(|_| "<redacted>"),
            )
            .field("ui_password", &"<redacted>")
            .field("mqtt_enabled", &self.mqtt_enabled)
            .field("mqtt_host", &self.mqtt_host)
            .field(
                "mqtt_password",
                &self.mqtt_password.as_ref().map(|_| "<redacted>"),
            )
            .field("timezone", &self.timezone)
            .field("energy_price_per_kwh", &self.energy_price_per_kwh)
            .finish_non_exhaustive()
    }
}

impl Config {
    pub(crate) fn load() -> Result<Self, AppError> {
        Self::from_map(&environment())
    }

    pub(crate) fn database_url() -> Result<String, AppError> {
        required(&environment(), "DATABASE_URL")
    }

    pub(crate) fn from_map(map: &BTreeMap<String, String>) -> Result<Self, AppError> {
        let domain = required(map, "TESLA_DOMAIN")?;
        if domain.contains("://") || domain.contains('/') || domain.contains(' ') {
            return Err(AppError::message(
                "TESLA_DOMAIN must be a bare hostname, for example tsl.b-612.fr",
            ));
        }
        let audience = required(map, "TESLA_FLEET_AUDIENCE")?;
        let region = Region::from_fleet_api_base(&audience).ok_or_else(|| {
            AppError::message(
                "TESLA_FLEET_AUDIENCE must be a Fleet API base URL such as https://fleet-api.prd.eu.vn.cloud.tesla.com",
            )
        })?;
        let public_base_url = match optional(map, "TESLA_PUBLIC_BASE_URL") {
            Some(value) => value,
            None => format!("https://{domain}"),
        };
        validate_public_base(&public_base_url)?;
        let token_store_path = PathBuf::from(required(map, "TESLA_TOKEN_STORE_PATH")?);
        let owner_stream_enabled = optional_bool(map, "OWNER_STREAM_ENABLED")?.unwrap_or(true);
        let owner_token_store_path = optional(map, "TESLA_OWNER_TOKEN_STORE_PATH")
            .map_or_else(|| owner_token_path(&token_store_path), PathBuf::from);
        let listen_addr = optional(map, "TESLA_LISTEN_ADDR")
            .unwrap_or_else(|| "0.0.0.0:4000".to_owned())
            .parse::<SocketAddr>()
            .map_err(|_| AppError::message("TESLA_LISTEN_ADDR is not a socket address"))?;
        let nominatim_enabled = optional_bool(map, "NOMINATIM_ENABLED")?.unwrap_or(true);
        let mqtt_enabled = optional_bool(map, "MQTT_ENABLED")?.unwrap_or(false);
        let telemetry_enabled = optional_bool(map, "TELEMETRY_ENABLED")?.unwrap_or(false);
        let telemetry_bind = optional(map, "TELEMETRY_BIND")
            .unwrap_or_else(|| "0.0.0.0:4443".to_owned())
            .parse::<SocketAddr>()
            .map_err(|_| AppError::message("TELEMETRY_BIND is not a socket address"))?;
        Ok(Self {
            database_url: required(map, "DATABASE_URL")?,
            listen_addr,
            domain: domain.clone(),
            public_base_url,
            redirect_uri: format!("https://{domain}/auth/callback"),
            region,
            client_id: required(map, "TESLA_CLIENT_ID")?,
            client_secret: SecretString::from(required(map, "TESLA_CLIENT_SECRET")?),
            private_key_path: PathBuf::from(required(map, "TESLA_PRIVATE_KEY_PATH")?),
            token_store_path,
            token_encryption_key: optional(map, "TESLA_TOKEN_ENCRYPTION_KEY"),
            ui_password: SecretString::from(required(map, "TESLA_LOGGER_UI_PASSWORD")?),
            drive_poll: Duration::from_secs(optional_u64(map, "POLL_DRIVE_SECS", 15)?),
            charge_poll: Duration::from_secs(optional_u64(map, "POLL_CHARGE_SECS", 60)?),
            parked_poll: Duration::from_secs(optional_u64(map, "POLL_PARKED_SECS", 180)?),
            asleep_poll: Duration::from_secs(optional_u64(map, "POLL_ASLEEP_SECS", 300)?),
            backoff_poll: Duration::from_secs(optional_u64(map, "POLL_BACKOFF_SECS", 720)?),
            parked_live_limit: u32::try_from(optional_u64(map, "POLL_PARKED_LIVE_LIMIT", 2)?)
                .map_err(|_| AppError::message("POLL_PARKED_LIVE_LIMIT is too large"))?,
            gps_fallback_poll: Duration::from_secs(optional_u64(
                map,
                "POLL_GPS_FALLBACK_SECS",
                300,
            )?),
            nominatim_enabled,
            nominatim_url: optional(map, "NOMINATIM_URL")
                .unwrap_or_else(|| "https://nominatim.openstreetmap.org".to_owned()),
            nominatim_contact: optional(map, "NOMINATIM_CONTACT"),
            nominatim_min_interval: Duration::from_millis(optional_u64(
                map,
                "NOMINATIM_MIN_INTERVAL_MS",
                1100,
            )?),
            mqtt_enabled,
            mqtt_host: optional(map, "MQTT_HOST").unwrap_or_else(|| "127.0.0.1".to_owned()),
            mqtt_port: u16::try_from(optional_u64(map, "MQTT_PORT", 1883)?)
                .map_err(|_| AppError::message("MQTT_PORT is not a port"))?,
            mqtt_username: optional(map, "MQTT_USERNAME"),
            mqtt_password: optional(map, "MQTT_PASSWORD").map(SecretString::from),
            mqtt_prefix: optional(map, "MQTT_TOPIC_PREFIX")
                .unwrap_or_else(|| "tesla_logger".to_owned()),
            telemetry_enabled,
            telemetry_bind,
            telemetry_cert_path: optional(map, "TELEMETRY_CERT_PATH").map(PathBuf::from),
            telemetry_key_path: optional(map, "TELEMETRY_KEY_PATH").map(PathBuf::from),
            owner_stream_enabled,
            owner_token_store_path,
            timezone: timezone_name(map)?,
            energy_price_per_kwh: optional_f64(map, "ENERGY_PRICE_PER_KWH")?,
            explicit_env: map.keys().cloned().collect(),
        })
    }

    pub(crate) fn public_settings(&self, admin: bool) -> Vec<PublicSetting> {
        let mut rows = Vec::new();
        let mut push = |label: &'static str, key: &str, value: String| {
            rows.push(PublicSetting {
                label,
                value,
                source: self.source_label(key),
            });
        };
        push("Domaine", "TESLA_DOMAIN", self.domain.clone());
        push(
            "URL publique",
            "TESLA_PUBLIC_BASE_URL",
            self.public_base_url.clone(),
        );
        push("Région", "TESLA_FLEET_AUDIENCE", self.region.to_string());
        if admin {
            push("Client ID", "TESLA_CLIENT_ID", self.client_id.clone());
        }
        push("Écoute", "TESLA_LISTEN_ADDR", self.listen_addr.to_string());
        push(
            "Poll conduite (s)",
            "POLL_DRIVE_SECS",
            self.drive_poll.as_secs().to_string(),
        );
        push(
            "Poll charge (s)",
            "POLL_CHARGE_SECS",
            self.charge_poll.as_secs().to_string(),
        );
        push(
            "Poll à l'arrêt (s)",
            "POLL_PARKED_SECS",
            self.parked_poll.as_secs().to_string(),
        );
        push(
            "Poll endormi (s)",
            "POLL_ASLEEP_SECS",
            self.asleep_poll.as_secs().to_string(),
        );
        push(
            "Poll repli (s)",
            "POLL_BACKOFF_SECS",
            self.backoff_poll.as_secs().to_string(),
        );
        push(
            "Lectures à l'arrêt",
            "POLL_PARKED_LIVE_LIMIT",
            self.parked_live_limit.to_string(),
        );
        push(
            "Repli GPS (s)",
            "POLL_GPS_FALLBACK_SECS",
            self.gps_fallback_poll.as_secs().to_string(),
        );
        push("Fuseau", "TESLA_TIMEZONE", self.timezone.clone());
        push(
            "Prix du kWh",
            "ENERGY_PRICE_PER_KWH",
            self.energy_price_per_kwh
                .map_or_else(|| "—".to_owned(), |price| price.to_string()),
        );
        push(
            "Nominatim",
            "NOMINATIM_ENABLED",
            self.nominatim_enabled.to_string(),
        );
        push("Nominatim URL", "NOMINATIM_URL", self.nominatim_url.clone());
        push(
            "Nominatim contact",
            "NOMINATIM_CONTACT",
            self.nominatim_contact
                .clone()
                .unwrap_or_else(|| "—".to_owned()),
        );
        push(
            "Nominatim intervalle (ms)",
            "NOMINATIM_MIN_INTERVAL_MS",
            self.nominatim_min_interval.as_millis().to_string(),
        );
        push("MQTT", "MQTT_ENABLED", self.mqtt_enabled.to_string());
        push("MQTT hôte", "MQTT_HOST", self.mqtt_host.clone());
        push("MQTT port", "MQTT_PORT", self.mqtt_port.to_string());
        if admin {
            push(
                "MQTT utilisateur",
                "MQTT_USERNAME",
                self.mqtt_username.clone().unwrap_or_else(|| "—".to_owned()),
            );
        }
        push(
            "MQTT préfixe",
            "MQTT_TOPIC_PREFIX",
            self.mqtt_prefix.clone(),
        );
        push(
            "Télémétrie",
            "TELEMETRY_ENABLED",
            self.telemetry_enabled.to_string(),
        );
        push(
            "Télémétrie écoute",
            "TELEMETRY_BIND",
            self.telemetry_bind.to_string(),
        );
        push(
            "Stream Owner",
            "OWNER_STREAM_ENABLED",
            self.owner_stream_enabled.to_string(),
        );
        rows
    }

    fn source_label(&self, key: &str) -> &'static str {
        if self.explicit_env.contains(key) {
            "env"
        } else {
            "défaut"
        }
    }
}

pub(crate) struct PublicSetting {
    pub label: &'static str,
    pub value: String,
    pub source: &'static str,
}

fn owner_token_path(fleet_tokens: &std::path::Path) -> PathBuf {
    match fleet_tokens.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join("owner-tokens.json"),
        _ => PathBuf::from("owner-tokens.json"),
    }
}

pub(crate) fn environment() -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let path = std::env::var("TESLA_LOGGER_ENV_FILE").unwrap_or_else(|_| ".env".to_owned());
    if let Ok(text) = std::fs::read_to_string(path) {
        for (key, value) in parse_dotenv(&text) {
            map.insert(key, value);
        }
    }
    for (key, value) in std::env::vars() {
        map.insert(key, value);
    }
    map
}

pub(crate) fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty() || key.contains(char::is_whitespace) {
            continue;
        }
        let value = unquote(value.trim());
        if value.is_empty() {
            continue;
        }
        pairs.push((key.to_owned(), value));
    }
    pairs
}

fn unquote(value: &str) -> String {
    let bytes = value.as_bytes();
    if bytes.len() >= 2 {
        let quote = bytes[0];
        if (quote == b'"' || quote == b'\'') && bytes[bytes.len() - 1] == quote {
            return value[1..value.len() - 1].to_owned();
        }
    }
    value.to_owned()
}

fn required(map: &BTreeMap<String, String>, key: &'static str) -> Result<String, AppError> {
    match map
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        Some(value) => Ok(value.to_owned()),
        None => Err(AppError::MissingEnv(key)),
    }
}

fn optional(map: &BTreeMap<String, String>, key: &str) -> Option<String> {
    map.get(key)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn optional_u64(
    map: &BTreeMap<String, String>,
    key: &'static str,
    default: u64,
) -> Result<u64, AppError> {
    match optional(map, key) {
        None => Ok(default),
        Some(value) => value
            .parse::<u64>()
            .map_err(|_| AppError::message(format!("{key} must be a positive integer"))),
    }
}

fn timezone_name(map: &BTreeMap<String, String>) -> Result<String, AppError> {
    let value = optional(map, "TESLA_TIMEZONE").unwrap_or_else(|| "Europe/Paris".to_owned());
    let ok = (1..=64).contains(&value.len())
        && !value.starts_with('/')
        && !value.ends_with('/')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '/' | '_' | '+' | '-'));
    if ok {
        Ok(value)
    } else {
        Err(AppError::message(
            "TESLA_TIMEZONE must be an IANA name such as Europe/Paris",
        ))
    }
}

fn optional_f64(
    map: &BTreeMap<String, String>,
    key: &'static str,
) -> Result<Option<f64>, AppError> {
    match optional(map, key) {
        None => Ok(None),
        Some(value) => {
            let parsed = value
                .parse::<f64>()
                .map_err(|_| AppError::message(format!("{key} must be a non-negative number")))?;
            if parsed.is_finite() && parsed >= 0.0 {
                Ok(Some(parsed))
            } else {
                Err(AppError::message(format!(
                    "{key} must be a non-negative number"
                )))
            }
        }
    }
}

fn optional_bool(
    map: &BTreeMap<String, String>,
    key: &'static str,
) -> Result<Option<bool>, AppError> {
    match optional(map, key) {
        None => Ok(None),
        Some(value) => match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(Some(true)),
            "0" | "false" | "no" | "off" => Ok(Some(false)),
            _ => Err(AppError::message(format!("{key} must be true or false"))),
        },
    }
}

fn validate_public_base(value: &str) -> Result<(), AppError> {
    let url =
        Url::parse(value).map_err(|_| AppError::message("TESLA_PUBLIC_BASE_URL is invalid"))?;
    let host = url.host_str().unwrap_or("");
    let loopback = matches!(host, "localhost" | "127.0.0.1" | "::1");
    if url.scheme() == "https" || (url.scheme() == "http" && loopback) {
        Ok(())
    } else {
        Err(AppError::message(
            "TESLA_PUBLIC_BASE_URL must be https, or http on localhost",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, parse_dotenv};

    #[test]
    fn parses_dotenv_without_overriding_later_process_values() {
        let pairs = parse_dotenv(
            "# comment\nexport TESLA_DOMAIN=\"tsl.b-612.fr\"\nEMPTY=\nTESLA_CLIENT_ID=abc\n",
        );
        assert_eq!(
            pairs,
            vec![
                ("TESLA_DOMAIN".to_owned(), "tsl.b-612.fr".to_owned()),
                ("TESLA_CLIENT_ID".to_owned(), "abc".to_owned()),
            ]
        );
    }

    #[test]
    fn debug_redacts_secrets() {
        let mut map = std::collections::BTreeMap::new();
        map.insert("TESLA_CLIENT_ID".to_owned(), "id".to_owned());
        map.insert(
            "TESLA_CLIENT_SECRET".to_owned(),
            "client-secret-value".to_owned(),
        );
        map.insert(
            "TESLA_FLEET_AUDIENCE".to_owned(),
            "https://fleet-api.prd.eu.vn.cloud.tesla.com".to_owned(),
        );
        map.insert("TESLA_DOMAIN".to_owned(), "tsl.b-612.fr".to_owned());
        map.insert(
            "DATABASE_URL".to_owned(),
            "postgres://tesla:db-secret@localhost/tesla".to_owned(),
        );
        map.insert(
            "TESLA_PRIVATE_KEY_PATH".to_owned(),
            "/keys/key.pem".to_owned(),
        );
        map.insert(
            "TESLA_TOKEN_STORE_PATH".to_owned(),
            "/data/tokens.json".to_owned(),
        );
        map.insert(
            "TESLA_LOGGER_UI_PASSWORD".to_owned(),
            "ui-secret-value".to_owned(),
        );
        map.insert(
            "TESLA_TOKEN_ENCRYPTION_KEY".to_owned(),
            "encrypt-secret".to_owned(),
        );
        let config = Config::from_map(&map).unwrap();
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("client-secret-value"));
        assert!(!rendered.contains("db-secret"));
        assert!(!rendered.contains("ui-secret-value"));
        assert!(!rendered.contains("encrypt-secret"));
        let settings = config
            .public_settings(true)
            .into_iter()
            .map(|row| row.value)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!settings.contains("client-secret-value"));
        assert!(!settings.contains("db-secret"));
        assert!(!settings.contains("ui-secret-value"));
        assert!(!settings.contains("encrypt-secret"));
        assert!(settings.contains("tsl.b-612.fr"));
        let hidden = config
            .public_settings(false)
            .into_iter()
            .map(|row| row.label)
            .collect::<Vec<_>>();
        assert!(!hidden.contains(&"Client ID"));
        assert!(!hidden.contains(&"MQTT utilisateur"));
        assert_eq!(config.redirect_uri, "https://tsl.b-612.fr/auth/callback");
        assert_eq!(config.region, tesla_auth::Region::Europe);
        assert_eq!(config.timezone, "Europe/Paris");
        assert_eq!(config.energy_price_per_kwh, None);
        assert!(config.owner_stream_enabled);
        assert_eq!(
            config.gps_fallback_poll,
            std::time::Duration::from_secs(300)
        );
        assert_eq!(
            config.owner_token_store_path,
            std::path::PathBuf::from("/data/owner-tokens.json")
        );
    }
}
