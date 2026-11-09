// SPDX-License-Identifier: MIT OR Apache-2.0

//! Async client for the Tesla Fleet API.
//!
//! The crate is not affiliated with Tesla. It speaks the documented Fleet API over HTTPS with
//! `rustls`. Callers pass an [`AccessTokenProvider`]: a static [`BearerToken`] or a
//! [`tesla_auth::Session`] that refreshes OAuth tokens.
//!
//! # Billing
//!
//! `vehicle_data` and `wake_up` are billable. This client never wakes a vehicle as a side effect of
//! reading data. [`FleetClient::vehicle_data`] returns [`FleetError::DeviceUnavailable`] when the
//! car is asleep or offline. Check [`FleetClient::fleet_status`] or
//! [`FleetClient::vehicle_data_cached`] first.
//!
//! HTTP 429 is returned as [`FleetError::RateLimited`]. Automatic retry is off unless a
//! [`RequestPolicy`] opts in, and mutating calls (wake, telemetry writes, partner registration)
//! are not retried unless that policy says so.
//!
//! # Example
//!
//! ```no_run
//! use tesla_fleet_api::{BearerToken, FleetClient, Region, VehicleDataEndpoint, VehicleTag};
//!
//! # async fn demo() -> Result<(), tesla_fleet_api::FleetError> {
//! let client = FleetClient::for_region(Region::Europe, BearerToken::new("access-token"))?;
//! let vehicles = client.list_vehicles(&Default::default()).await?;
//! if let Some(vehicle) = vehicles.vehicles.first() {
//!     let tag = VehicleTag::new(&vehicle.vin)?;
//!     let _status = client.fleet_status(&[vehicle.vin.clone()]).await?;
//!     if vehicle.is_online() {
//!         let _data = client
//!             .vehicle_data(&tag, &[VehicleDataEndpoint::ChargeState])
//!             .await?;
//!     }
//! }
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod client;
mod error;
mod models;
mod policy;
mod token;

pub use client::{FleetClient, FleetClientBuilder};
pub use error::{FleetError, RateLimit};
pub use models::{
    ChargeState, ChargingFee, ChargingHistory, ChargingHistoryQuery, ChargingInvoice,
    ChargingSession, ClimateState, DriveState, FleetStatus, FleetTelemetryConfig,
    FleetTelemetryConfigRequest, FleetTelemetryConfigUpdate, FleetVehicleInfo, GranularAccess,
    GuiSettings, ListVehiclesQuery, PageRef, Pagination, PartnerAccount, PartnerPublicKey,
    RegisterPartnerAccount, SensitiveString, SkippedVehicles, SoftwareUpdate, SortOrder,
    TelemetryConfig, TelemetryField, UserRegion, Vehicle, VehicleConfig, VehicleData,
    VehicleDataEndpoint, VehicleList, VehicleState, VehicleTag,
};
pub use policy::RequestPolicy;
pub use tesla_auth::Region;
pub use token::{AccessTokenProvider, BearerToken};
