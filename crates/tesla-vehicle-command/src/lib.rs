// SPDX-License-Identifier: MIT OR Apache-2.0

//! Tesla vehicle-command protocol.
//!
//! This crate signs virtual-key commands for the vehicle-security and infotainment domains and
//! sends them through a [`CommandTransport`]. [`FleetTransport`] posts the bytes to Fleet API
//! `signed_command`. BLE is not implemented; the transport trait is the place to add it.
//!
//! The protocol definition and the vendored protobufs come from Tesla's
//! [vehicle-command](https://github.com/teslamotors/vehicle-command) repository. See `NOTICE`.
//! This crate is not affiliated with Tesla.
//!
//! # Security
//!
//! * The P-256 scalar and the 16-byte session key are zeroized on drop and are not included in
//!   [`Debug`].
//! * Session-info and command MACs are compared in constant time.
//! * SHA-1 is used only where the protocol defines `K = SHA1(x)[:16]`.
//! * Responses that carry AES-GCM data are authenticated. Plaintext responses from firmware older
//!   than 2024.38 are not authenticated; that matches the reference client.
//! * [`Vehicle`] does not call wake except through [`Vehicle::wake`]. Fleet API wake is billable.
//!
//! # Example
//!
//! ```no_run
//! use tesla_fleet_api::{BearerToken, FleetClient, Region};
//! use tesla_vehicle_command::{FleetTransport, PrivateKey, Vehicle};
//!
//! # async fn demo() -> Result<(), tesla_vehicle_command::CommandError> {
//! let fleet = FleetClient::for_region(Region::Europe, BearerToken::new("access-token"))
//!     .map_err(tesla_vehicle_command::CommandError::from)?;
//! let transport = FleetTransport::new(fleet, "5YJ00000000000000")?;
//! let key = PrivateKey::from_pem("-----BEGIN EC PRIVATE KEY-----\n...\n-----END EC PRIVATE KEY-----")?;
//! let mut vehicle = Vehicle::new(transport, key)?;
//! vehicle.wake().await?;
//! vehicle.unlock().await?;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod client;
mod commands;
mod crypto;
mod domain;
mod error;
mod fleet;
mod key;
mod message;
mod metadata;
mod replay;
mod session;
mod transport;

#[allow(clippy::all, missing_docs, dead_code)]
pub mod proto {
    include!(concat!(env!("OUT_DIR"), "/wire.rs"));
}

pub use client::Vehicle;
pub use domain::Domain;
pub use error::{CommandError, MessageFault};
pub use fleet::FleetTransport;
pub use key::{PrivateKey, PrivateKeyFormat};
pub use message::FLAG_ENCRYPT_RESPONSE;
/// Top-level vehicle-command envelope.
pub use proto::universal_message::RoutableMessage;
pub use transport::{CommandTransport, WakeStatus};

#[cfg(test)]
mod protocol_tests;
