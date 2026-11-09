// SPDX-License-Identifier: MIT OR Apache-2.0

use std::future::Future;

use crate::CommandError;

/// How a vehicle answered a wake request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeStatus {
    /// Fleet API `state`, when the transport reported one.
    pub state: Option<String>,
    /// `true` when `state` is `online`.
    pub online: bool,
}

/// Moves `RoutableMessage` bytes to and from a vehicle.
///
/// [`crate::FleetTransport`] posts them to Fleet API `signed_command`. A future BLE transport can
/// implement the same trait. BLE framing is not implemented.
pub trait CommandTransport: Send + Sync {
    /// VIN this transport is bound to. It is the session personalization string.
    fn vin(&self) -> &str;

    /// Sends one encoded `RoutableMessage` and returns the encoded response.
    fn exchange(
        &self,
        message: &[u8],
    ) -> impl Future<Output = Result<Vec<u8>, CommandError>> + Send;

    /// Wakes the vehicle without sending a signed command.
    ///
    /// Fleet API wake is billable. This crate does not call it except from [`Self::wake`].
    fn wake(&self) -> impl Future<Output = Result<WakeStatus, CommandError>> + Send;
}
