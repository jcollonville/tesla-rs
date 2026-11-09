// SPDX-License-Identifier: MIT OR Apache-2.0

use tesla_fleet_api::{AccessTokenProvider, FleetClient, VehicleTag};

use crate::CommandError;
use crate::error::validate_vin;
use crate::transport::{CommandTransport, WakeStatus};

/// Fleet API transport for `POST /api/1/vehicles/{vin}/signed_command`.
///
/// The HTTP client, bearer token, and error mapping come from `tesla-fleet-api`. This type does
/// not log the routable message.
pub struct FleetTransport<T> {
    client: FleetClient<T>,
    tag: VehicleTag,
    vin: String,
}

impl<T> std::fmt::Debug for FleetTransport<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FleetTransport")
            .field("vin", &self.vin)
            .field("client", &self.client)
            .finish_non_exhaustive()
    }
}

impl<T: AccessTokenProvider> FleetTransport<T> {
    /// Binds `client` to `vin`.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidVin`] when `vin` is not 17 ASCII letters or digits.
    pub fn new(client: FleetClient<T>, vin: impl Into<String>) -> Result<Self, CommandError> {
        let vin = vin.into();
        validate_vin(&vin)?;
        let tag = VehicleTag::new(vin.clone()).map_err(|_| CommandError::InvalidVin)?;
        Ok(Self { client, tag, vin })
    }
}

impl<T: AccessTokenProvider> CommandTransport for FleetTransport<T> {
    fn vin(&self) -> &str {
        &self.vin
    }

    async fn exchange(&self, message: &[u8]) -> Result<Vec<u8>, CommandError> {
        self.client
            .signed_command(&self.tag, message)
            .await
            .map_err(CommandError::from)
    }

    async fn wake(&self) -> Result<WakeStatus, CommandError> {
        let vehicle = self.client.wake_up(&self.tag).await?;
        Ok(WakeStatus {
            online: vehicle.is_online(),
            state: vehicle.state,
        })
    }
}
