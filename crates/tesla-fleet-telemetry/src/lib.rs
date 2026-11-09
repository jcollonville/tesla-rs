// SPDX-License-Identifier: MIT OR Apache-2.0

//! Receive and decode Tesla Fleet Telemetry.
//!
//! This crate is not affiliated with Tesla. Protobuf definitions and the client CA bundles are
//! vendored from [teslamotors/fleet-telemetry](https://github.com/teslamotors/fleet-telemetry)
//! commit `bb7c8a931e46da3b3eb6d20a67541c8f755f1ee6` (Apache-2.0). See `NOTICE`.
//!
//! Raw prost messages live in [`proto`]. [`decode_vehicle_data`] and the other decoders expose
//! every `Field` and `Value` oneof arm, and they apply the same body fixes as the reference
//! server. The FlatBuffers websocket envelope is reconstructed in [`envelope`] from the generated
//! Go in that repository. The `server` feature (on by default) accepts the vehicle's mutual-TLS
//! websocket and yields decoded records. It does not store them.
//!
//! # Config JSON
//!
//! ```
//! use tesla_fleet_telemetry::{DeliveryPolicy, FleetTelemetryConfig};
//!
//! let config = FleetTelemetryConfig::builder("telemetry.example.com", 443, "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n")
//!     .known_field(tesla_fleet_telemetry::proto::vehicle_data::Field::Soc, 60)
//!     .field("Location", 10)
//!     .delivery_policy(DeliveryPolicy::Latest)
//!     .build()
//!     .expect("config");
//! let body = config.request_body(["5YJ3E1EA7KF000001"]).expect("json");
//! assert_eq!(body["config"]["delivery_policy"], "latest");
//! assert_eq!(body["config"]["fields"]["Soc"]["interval_seconds"], 60);
//! ```

#![forbid(unsafe_code)]

pub mod config;
pub mod envelope;
pub mod error;
pub mod identity;
pub mod record;

#[allow(missing_docs, clippy::all)]
pub mod proto {
    /// `vehicle_data.proto`.
    pub mod vehicle_data {
        include!(concat!(env!("OUT_DIR"), "/telemetry.vehicle_data.rs"));
    }
    /// `vehicle_alert.proto`.
    pub mod vehicle_alerts {
        include!(concat!(env!("OUT_DIR"), "/telemetry.vehicle_alerts.rs"));
    }
    /// `vehicle_error.proto`.
    pub mod vehicle_error {
        include!(concat!(env!("OUT_DIR"), "/telemetry.vehicle_error.rs"));
    }
    /// `vehicle_connectivity.proto`.
    pub mod vehicle_connectivity {
        include!(concat!(
            env!("OUT_DIR"),
            "/telemetry.vehicle_connectivity.rs"
        ));
    }
    /// `vehicle_metric.proto`.
    pub mod vehicle_metrics {
        include!(concat!(env!("OUT_DIR"), "/telemetry.vehicle_metrics.rs"));
    }
}

#[cfg(feature = "server")]
pub mod server;

pub use config::{DeliveryPolicy, FieldConfig, FleetTelemetryConfig, FleetTelemetryConfigBuilder};
pub use envelope::{
    Envelope, MessageKind, StreamParts, decode_envelope, encode_ack, encode_format_error,
    encode_stream,
};
pub use error::TelemetryError;
pub use identity::Identity;
pub use proto::vehicle_data::Field;
pub use record::{
    DatumValue, DecodeOptions, Decoded, DecodedDatum, FieldKey, Timestamp, TypedEnum, VehicleAlert,
    VehicleAlerts, VehicleConnectivity, VehicleData, VehicleErrors, decode_alerts,
    decode_connectivity, decode_errors, decode_topic, decode_vehicle_data,
};

#[cfg(feature = "server")]
pub use server::{ClientCa, IncomingRecord, ServerConfig, TelemetryServer};
