// SPDX-License-Identifier: MIT OR Apache-2.0

//! Unofficial Tesla Owner API login and streaming client.
//!
//! This crate is not affiliated with Tesla, Inc. The Owner API and the streaming websocket are
//! legacy interfaces. Tesla can remove them without notice. They are not a substitute for the
//! Fleet API.
//!
//! Token material is held in [`secrecy::SecretString`]. [`Debug`] redacts access tokens, refresh
//! tokens, authorization codes, and PKCE verifiers. The HTTP client does not log headers or
//! bodies. Redirects are disabled.
//!
//! The subscribe frame returned by [`subscribe_frame`] contains the access token. Do not log it.
//!
//! # Streaming units
//!
//! `speed`, `odometer`, `range`, and `est_range` are miles. `elevation` is feet. `power` is
//! kilowatts. `soc` is percent. Latitude and longitude are decimal degrees.

#![forbid(unsafe_code)]

mod auth;
mod error;
mod pkce;
mod stream;
mod vehicles;

pub use auth::{OwnerClient, OwnerLogin, OwnerRegion, OwnerTokens};
pub use error::OwnerError;
pub use secrecy::{ExposeSecret, SecretString};
pub use stream::{
    FailureClass, Incoming, PumpEnd, ReconnectPlan, STREAM_COLUMNS, StreamSample,
    classify_stream_error, classify_transport_message, parse_message, plan_reconnect, pump_stream,
    subscribe_frame,
};
pub use vehicles::OwnerVehicle;
