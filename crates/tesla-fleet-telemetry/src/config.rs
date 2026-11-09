// SPDX-License-Identifier: MIT OR Apache-2.0

//! JSON body for `POST /api/1/vehicles/fleet_telemetry_config`.
//!
//! The shape matches Tesla's fleet telemetry config and the fields
//! `tesla_fleet_api::TelemetryConfig` forwards: hostname, CA, port, fields,
//! intervals, `minimum_delta`, `resend_interval_seconds`, `include_fields`,
//! alert types, expiry, and `delivery_policy`. `prefer_typed` is not emitted.
//! Vehicles ignore it and always send typed values.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::TelemetryError;
use crate::proto::vehicle_data::Field;

/// How the vehicle treats records the server has not acknowledged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryPolicy {
    /// Resend every unacknowledged record. Requires Fleet Telemetry client 1.0.0 or later.
    Latest,
}

/// Per-field streaming options.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldConfig {
    /// Minimum seconds between publishes. Required and non-zero.
    pub interval_seconds: u32,
    /// Numeric deadband. Location fields measure this in meters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_delta: Option<f64>,
    /// Publish again after this many seconds even when the value did not change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resend_interval_seconds: Option<u32>,
    /// Fields included in the same payload when this field is published.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_fields: Vec<String>,
}

/// Fleet telemetry config document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FleetTelemetryConfig {
    /// Hostname the vehicle dials. No scheme and no port.
    pub hostname: String,
    /// PEM certificate chain the vehicle uses to authenticate the server.
    pub ca: String,
    /// TCP port. Tesla's reference server uses 443.
    pub port: u16,
    /// Signals to stream, keyed by the protobuf `Field` name such as `Soc`.
    pub fields: BTreeMap<String, FieldConfig>,
    /// Alert audiences, for example `service`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alert_types: Vec<String>,
    /// Unix expiry in seconds. `None` asks Tesla to keep the config.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
    /// Acknowledgement policy. Omitted when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_policy: Option<DeliveryPolicy>,
}

impl FleetTelemetryConfig {
    /// Starts a config for `hostname:port` authenticated with `ca_pem`.
    #[must_use]
    pub fn builder(
        hostname: impl Into<String>,
        port: u16,
        ca_pem: impl Into<String>,
    ) -> FleetTelemetryConfigBuilder {
        FleetTelemetryConfigBuilder {
            hostname: hostname.into(),
            port,
            ca: ca_pem.into(),
            fields: BTreeMap::new(),
            alert_types: Vec::new(),
            exp: None,
            delivery_policy: None,
            allow_unknown_fields: false,
        }
    }

    /// Serializes the config object used as `config` in the Fleet API request.
    ///
    /// # Errors
    ///
    /// Returns [`TelemetryError::InvalidConfig`] when a number cannot be represented as JSON.
    pub fn to_value(&self) -> Result<serde_json::Value, TelemetryError> {
        serde_json::to_value(self)
            .map_err(|_| TelemetryError::InvalidConfig("config is not valid json"))
    }

    /// Serializes `{ "vins": [...], "config": ... }` for the create endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`TelemetryError::InvalidConfig`] when the config cannot be represented as JSON.
    pub fn request_body(
        &self,
        vins: impl IntoIterator<Item = impl Into<String>>,
    ) -> Result<serde_json::Value, TelemetryError> {
        Ok(serde_json::json!({
            "vins": vins.into_iter().map(Into::into).collect::<Vec<_>>(),
            "config": self.to_value()?,
        }))
    }
}

/// Builder for [`FleetTelemetryConfig`].
#[derive(Debug)]
pub struct FleetTelemetryConfigBuilder {
    hostname: String,
    port: u16,
    ca: String,
    fields: BTreeMap<String, FieldConfig>,
    alert_types: Vec<String>,
    exp: Option<u64>,
    delivery_policy: Option<DeliveryPolicy>,
    allow_unknown_fields: bool,
}

impl FleetTelemetryConfigBuilder {
    /// Accepts field names that are not in the vendored `Field` enum.
    #[must_use]
    pub fn allow_unknown_fields(mut self) -> Self {
        self.allow_unknown_fields = true;
        self
    }

    /// Adds a field sampled at `interval_seconds`.
    #[must_use]
    pub fn field(self, name: &str, interval_seconds: u32) -> Self {
        self.field_with(
            name,
            FieldConfig {
                interval_seconds,
                minimum_delta: None,
                resend_interval_seconds: None,
                include_fields: Vec::new(),
            },
        )
    }

    /// Adds a field from the vendored protobuf enum.
    #[must_use]
    pub fn known_field(self, field: Field, interval_seconds: u32) -> Self {
        self.field(field.as_str_name(), interval_seconds)
    }

    /// Adds a field with deadband, resend, or included fields.
    #[must_use]
    pub fn field_with(mut self, name: &str, config: FieldConfig) -> Self {
        self.fields.insert(name.to_owned(), config);
        self
    }

    /// Alert audiences the vehicle should publish.
    #[must_use]
    pub fn alert_types<I, S>(mut self, types: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.alert_types = types.into_iter().map(Into::into).collect();
        self
    }

    /// Unix expiry in seconds.
    #[must_use]
    pub fn expires_at(mut self, unix_seconds: u64) -> Self {
        self.exp = Some(unix_seconds);
        self
    }

    /// Sets `delivery_policy`.
    #[must_use]
    pub fn delivery_policy(mut self, policy: DeliveryPolicy) -> Self {
        self.delivery_policy = Some(policy);
        self
    }

    /// Validates and builds the config.
    ///
    /// # Errors
    ///
    /// Returns [`TelemetryError::InvalidConfig`] when the hostname, CA, port, or a field option is
    /// empty or inconsistent.
    pub fn build(self) -> Result<FleetTelemetryConfig, TelemetryError> {
        if self.hostname.is_empty()
            || self.hostname.chars().any(char::is_whitespace)
            || self.hostname.contains("://")
            || self.hostname.contains('/')
            || self.hostname.contains(':')
        {
            return Err(TelemetryError::InvalidConfig(
                "hostname must be a hostname without a scheme or port",
            ));
        }
        if self.port == 0 {
            return Err(TelemetryError::InvalidConfig("port must be non-zero"));
        }
        if self.ca.trim().is_empty() {
            return Err(TelemetryError::InvalidConfig(
                "ca must be the server certificate chain",
            ));
        }
        if self.fields.is_empty() {
            return Err(TelemetryError::InvalidConfig(
                "at least one telemetry field is required",
            ));
        }
        for (name, field) in &self.fields {
            validate_field_name(name, self.allow_unknown_fields)?;
            if field.interval_seconds == 0 {
                return Err(TelemetryError::InvalidConfig(
                    "interval_seconds must be non-zero",
                ));
            }
            if field
                .minimum_delta
                .is_some_and(|delta| !delta.is_finite() || delta < 0.0)
            {
                return Err(TelemetryError::InvalidConfig(
                    "minimum_delta must be a finite number that is not negative",
                ));
            }
            if field.resend_interval_seconds == Some(0) {
                return Err(TelemetryError::InvalidConfig(
                    "resend_interval_seconds must be non-zero",
                ));
            }
            for included in &field.include_fields {
                validate_field_name(included, self.allow_unknown_fields)?;
            }
        }
        for alert in &self.alert_types {
            if alert.is_empty() || alert.chars().any(char::is_whitespace) {
                return Err(TelemetryError::InvalidConfig(
                    "alert type must be a non-empty token",
                ));
            }
        }
        Ok(FleetTelemetryConfig {
            hostname: self.hostname,
            ca: self.ca,
            port: self.port,
            fields: self.fields,
            alert_types: self.alert_types,
            exp: self.exp,
            delivery_policy: self.delivery_policy,
        })
    }
}

fn validate_field_name(name: &str, allow_unknown: bool) -> Result<(), TelemetryError> {
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return Err(TelemetryError::InvalidConfig(
            "field name must be a non-empty token",
        ));
    }
    if !allow_unknown && Field::from_str_name(name).is_none() {
        return Err(TelemetryError::InvalidConfig(
            "field name is not in the vendored Field enum",
        ));
    }
    Ok(())
}
