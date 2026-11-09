// SPDX-License-Identifier: MIT OR Apache-2.0

/// One vehicle from `GET /api/1/vehicles`.
///
/// [`Self::vehicle_id`] is the streaming tag. [`Self::id`] is the REST id. They are different
/// numbers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerVehicle {
    /// Vehicle identification number.
    pub vin: String,
    /// Owner API REST id.
    pub id: i64,
    /// Streaming `tag`. This is the `vehicle_id` field, not [`Self::id`].
    pub vehicle_id: i64,
    /// `online`, `asleep`, or `offline`, when Tesla sent it.
    pub state: Option<String>,
}
