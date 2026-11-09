// SPDX-License-Identifier: MIT OR Apache-2.0

//! Response and request bodies for the Fleet API.
//!
//! JSON objects tolerate missing keys, JSON null, and unknown fields. Unknown fields are kept on
//! `additional` so a newer Tesla firmware does not fail decoding. Credential-like vehicle fields
//! (`tokens`, `backseat_token`) are redacted in [`Debug`].

use std::collections::BTreeMap;
use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

/// String whose [`Debug`] output is `<redacted>`.
///
/// Serde still reads and writes the underlying value. Use [`Self::expose`] at the point of use.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct SensitiveString(String);

impl SensitiveString {
    /// Borrows the underlying value.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SensitiveString {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Vehicle tag accepted in Fleet API paths.
///
/// Tesla's reference calls this `vehicle_tag` and accepts either a VIN or the numeric `id`.
/// Only ASCII letters, digits, `_`, and `-` are accepted so a tag cannot change the request path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct VehicleTag(String);

impl VehicleTag {
    /// Validates `tag`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::FleetError::InvalidRequest`] when `tag` is empty or contains any other character.
    pub fn new(tag: impl Into<String>) -> Result<Self, crate::FleetError> {
        let tag = tag.into();
        let ok = !tag.is_empty()
            && tag
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if ok {
            Ok(Self(tag))
        } else {
            Err(crate::FleetError::InvalidRequest(
                "vehicle tag must be an ascii vin or id",
            ))
        }
    }

    /// Borrowed tag.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for VehicleTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `endpoints` query values documented for `vehicle_data`.
///
/// The reference lists the variants other than [`Self::Custom`]. Tesla has added endpoint names
/// over time; pass those through [`Self::Custom`].
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum VehicleDataEndpoint {
    /// `charge_state`.
    ChargeState,
    /// `climate_state`.
    ClimateState,
    /// `closures_state`.
    ClosuresState,
    /// `drive_state`.
    DriveState,
    /// `gui_settings`.
    GuiSettings,
    /// `location_data`. Required on firmware 2023.38+ to receive location.
    LocationData,
    /// `vehicle_config`.
    VehicleConfig,
    /// `vehicle_state`.
    VehicleState,
    /// `vehicle_data_combo`.
    VehicleDataCombo,
    /// An endpoint name this crate does not special-case.
    Custom(String),
}

impl VehicleDataEndpoint {
    /// Parses one endpoint name.
    ///
    /// # Errors
    ///
    /// Returns [`crate::FleetError::InvalidRequest`] when the name is empty or contains a
    /// character that would change the query string.
    pub fn new(name: &str) -> Result<Self, crate::FleetError> {
        if !valid_endpoint(name) {
            return Err(crate::FleetError::InvalidRequest(
                "vehicle data endpoint name is invalid",
            ));
        }
        Ok(match name {
            "charge_state" => Self::ChargeState,
            "climate_state" => Self::ClimateState,
            "closures_state" => Self::ClosuresState,
            "drive_state" => Self::DriveState,
            "gui_settings" => Self::GuiSettings,
            "location_data" => Self::LocationData,
            "vehicle_config" => Self::VehicleConfig,
            "vehicle_state" => Self::VehicleState,
            "vehicle_data_combo" => Self::VehicleDataCombo,
            other => Self::Custom(other.to_owned()),
        })
    }

    /// Wire value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        match self {
            Self::ChargeState => "charge_state",
            Self::ClimateState => "climate_state",
            Self::ClosuresState => "closures_state",
            Self::DriveState => "drive_state",
            Self::GuiSettings => "gui_settings",
            Self::LocationData => "location_data",
            Self::VehicleConfig => "vehicle_config",
            Self::VehicleState => "vehicle_state",
            Self::VehicleDataCombo => "vehicle_data_combo",
            Self::Custom(name) => name,
        }
    }

    pub(crate) fn join(endpoints: &[Self]) -> String {
        endpoints
            .iter()
            .map(Self::as_str)
            .collect::<Vec<_>>()
            .join(";")
    }
}

fn valid_endpoint(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One page from `GET /api/1/vehicles`.
#[derive(Clone, Debug)]
pub struct VehicleList {
    /// Vehicles on this page.
    pub vehicles: Vec<Vehicle>,
    /// Pagination object, when Tesla sent one.
    pub pagination: Option<Pagination>,
    /// Top-level `count`, which is not always equal to `pagination.count`.
    pub count: Option<i64>,
}

/// Pagination object used by list endpoints.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pagination {
    /// Previous page cursor.
    #[serde(default)]
    pub previous: Option<PageRef>,
    /// Next page cursor.
    #[serde(default)]
    pub next: Option<PageRef>,
    /// Current page number.
    #[serde(default)]
    pub current: Option<i64>,
    /// Page size requested or applied by the server.
    #[serde(default)]
    pub per_page: Option<i64>,
    /// Total item count reported inside `pagination`.
    #[serde(default)]
    pub count: Option<i64>,
    /// Total pages.
    #[serde(default)]
    pub pages: Option<i64>,
}

/// A `next` or `previous` cursor. The published example uses integers; strings are also accepted.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PageRef {
    /// Numeric page.
    Number(i64),
    /// Non-numeric cursor.
    Text(String),
}

/// Query for [`crate::FleetClient::list_vehicles`].
#[derive(Clone, Debug, Default)]
pub struct ListVehiclesQuery {
    /// 1-based page number.
    pub page: Option<u32>,
    /// Page size. Tesla's default is 100.
    pub per_page: Option<u32>,
}

/// Account vehicle summary from list, vehicle, and wake responses.
#[derive(Clone, Serialize, Deserialize)]
pub struct Vehicle {
    /// Fleet API vehicle id.
    pub id: i64,
    /// User id, when present.
    #[serde(default)]
    pub user_id: Option<i64>,
    /// Legacy vehicle id.
    #[serde(default)]
    pub vehicle_id: Option<i64>,
    /// Vehicle identification number.
    pub vin: String,
    /// Paint color name, when present.
    #[serde(default)]
    pub color: Option<String>,
    /// Access level such as `OWNER`.
    #[serde(default)]
    pub access_type: Option<String>,
    /// Name shown in the Tesla app. Often absent from `vehicle_data`.
    #[serde(default)]
    pub display_name: Option<String>,
    /// Option codes, when the list endpoint sends them.
    #[serde(default)]
    pub option_codes: Option<String>,
    /// Share-access flags. `hide_private` suppresses location.
    #[serde(default)]
    pub granular_access: Option<GranularAccess>,
    /// Streaming tokens. These are not OAuth tokens. Redacted in [`Debug`].
    #[serde(default, deserialize_with = "empty_vec_if_null")]
    pub tokens: Vec<SensitiveString>,
    /// `online`, `asleep`, or `offline`.
    #[serde(default)]
    pub state: Option<String>,
    /// Whether the vehicle is in service.
    #[serde(default)]
    pub in_service: Option<bool>,
    /// String form of [`Self::id`].
    #[serde(default)]
    pub id_s: Option<String>,
    /// Calendar feature flag.
    #[serde(default)]
    pub calendar_enabled: Option<bool>,
    /// Vehicle API version advertised by the car.
    #[serde(default)]
    pub api_version: Option<i64>,
    /// Backseat token. Redacted in [`Debug`].
    #[serde(default)]
    pub backseat_token: Option<SensitiveString>,
    /// Timestamp paired with [`Self::backseat_token`].
    #[serde(default)]
    pub backseat_token_updated_at: Option<i64>,
    /// Fields this crate does not model yet, including newer state blocks on a bare vehicle object.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

impl Vehicle {
    /// `state == "online"`.
    #[must_use]
    pub fn is_online(&self) -> bool {
        self.state.as_deref() == Some("online")
    }

    /// `state == "asleep"`.
    #[must_use]
    pub fn is_asleep(&self) -> bool {
        self.state.as_deref() == Some("asleep")
    }

    /// `state == "offline"`.
    #[must_use]
    pub fn is_offline(&self) -> bool {
        self.state.as_deref() == Some("offline")
    }
}

impl fmt::Debug for Vehicle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Vehicle")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("vehicle_id", &self.vehicle_id)
            .field("vin", &self.vin)
            .field("color", &self.color)
            .field("access_type", &self.access_type)
            .field("display_name", &self.display_name)
            .field("option_codes", &self.option_codes)
            .field("granular_access", &self.granular_access)
            .field("state", &self.state)
            .field("in_service", &self.in_service)
            .field("id_s", &self.id_s)
            .field("calendar_enabled", &self.calendar_enabled)
            .field("api_version", &self.api_version)
            .field("tokens", &format_args!("<redacted {}>", self.tokens.len()))
            .field(
                "backseat_token",
                &self.backseat_token.as_ref().map(|_| "<redacted>"),
            )
            .field("backseat_token_updated_at", &self.backseat_token_updated_at)
            .field(
                "additional_keys",
                &self.additional.keys().collect::<Vec<_>>(),
            )
            .finish_non_exhaustive()
    }
}

/// `granular_access` object.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GranularAccess {
    /// When true, location is omitted even if `vehicle_location` was granted.
    #[serde(default)]
    pub hide_private: Option<bool>,
    /// Unknown share-access flags.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// Live or cached vehicle data.
///
/// `drive_state` can contain precise location. Debug output includes that location because it is
/// vehicle data the caller asked for; OAuth material is not part of this struct.
#[derive(Clone, Debug, Serialize)]
pub struct VehicleData {
    /// Identity fields.
    pub vehicle: Vehicle,
    /// Charge and battery block.
    pub charge_state: Option<ChargeState>,
    /// Cabin climate block.
    pub climate_state: Option<ClimateState>,
    /// Drive and location block.
    pub drive_state: Option<DriveState>,
    /// Display units.
    pub gui_settings: Option<GuiSettings>,
    /// Vehicle configuration.
    pub vehicle_config: Option<VehicleConfig>,
    /// Locks, odometer, software, and tire pressure.
    pub vehicle_state: Option<VehicleState>,
}

impl VehicleData {
    fn from_value(mut value: Value) -> Result<Self, serde_json::Error> {
        let Some(obj) = value.as_object_mut() else {
            return Err(serde::de::Error::custom(
                "vehicle data response is not an object",
            ));
        };
        let charge_state = take_optional(obj, "charge_state")?;
        let climate_state = take_optional(obj, "climate_state")?;
        let drive_state = take_optional(obj, "drive_state")?;
        let gui_settings = take_optional(obj, "gui_settings")?;
        let vehicle_config = take_optional(obj, "vehicle_config")?;
        let vehicle_state = take_optional(obj, "vehicle_state")?;
        let vehicle = serde_json::from_value(value)?;
        Ok(Self {
            vehicle,
            charge_state,
            climate_state,
            drive_state,
            gui_settings,
            vehicle_config,
            vehicle_state,
        })
    }
}

impl<'de> Deserialize<'de> for VehicleData {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = Value::deserialize(deserializer)?;
        Self::from_value(value).map_err(serde::de::Error::custom)
    }
}

fn take_optional<T: DeserializeOwned>(
    obj: &mut Map<String, Value>,
    key: &str,
) -> Result<Option<T>, serde_json::Error> {
    match obj.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value).map(Some),
    }
}

/// Subset of `charge_state` used by logging and charge history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChargeState {
    /// State of charge percent.
    #[serde(default)]
    pub battery_level: Option<f64>,
    /// Rated range.
    #[serde(default)]
    pub battery_range: Option<f64>,
    /// Usable state of charge percent.
    #[serde(default)]
    pub usable_battery_level: Option<f64>,
    /// Estimated range.
    #[serde(default)]
    pub est_battery_range: Option<f64>,
    /// Ideal range.
    #[serde(default)]
    pub ideal_battery_range: Option<f64>,
    /// Charging state such as `Disconnected` or `Charging`.
    #[serde(default)]
    pub charging_state: Option<String>,
    /// Charge limit percent.
    #[serde(default)]
    pub charge_limit_soc: Option<f64>,
    /// Energy added in the current session, kWh.
    #[serde(default)]
    pub charge_energy_added: Option<f64>,
    /// Instantaneous charger power, kW.
    #[serde(default)]
    pub charger_power: Option<f64>,
    /// Charger voltage.
    #[serde(default)]
    pub charger_voltage: Option<f64>,
    /// Measured charge current.
    #[serde(default)]
    pub charger_actual_current: Option<f64>,
    /// Charge rate in the car's distance units per hour.
    #[serde(default)]
    pub charge_rate: Option<f64>,
    /// Hours until the charge limit.
    #[serde(default)]
    pub time_to_full_charge: Option<f64>,
    /// Minutes until the charge limit.
    #[serde(default)]
    pub minutes_to_full_charge: Option<f64>,
    /// Requested charge current, amperes.
    #[serde(default)]
    pub charge_current_request: Option<f64>,
    /// Scheduled charging start, minutes after midnight. Null when unset.
    #[serde(default)]
    pub scheduled_charging_start_time: Option<f64>,
    /// Charge port door.
    #[serde(default)]
    pub charge_port_door_open: Option<bool>,
    /// Charge port latch, such as `Engaged`.
    #[serde(default)]
    pub charge_port_latch: Option<String>,
    /// Whether a DC fast charger is connected.
    #[serde(default)]
    pub fast_charger_present: Option<bool>,
    /// Fast charger type.
    #[serde(default)]
    pub fast_charger_type: Option<String>,
    /// Charge cable description.
    #[serde(default)]
    pub conn_charge_cable: Option<String>,
    /// Block timestamp in milliseconds.
    #[serde(default)]
    pub timestamp: Option<i64>,
    /// Unknown charge fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// Subset of `climate_state`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ClimateState {
    /// Cabin temperature in Celsius.
    #[serde(default)]
    pub inside_temp: Option<f64>,
    /// Outside temperature in Celsius.
    #[serde(default)]
    pub outside_temp: Option<f64>,
    /// Driver setpoint in Celsius.
    #[serde(default)]
    pub driver_temp_setting: Option<f64>,
    /// Passenger setpoint in Celsius.
    #[serde(default)]
    pub passenger_temp_setting: Option<f64>,
    /// Whether climate is currently on.
    #[serde(default)]
    pub is_climate_on: Option<bool>,
    /// Whether preconditioning is active.
    #[serde(default)]
    pub is_preconditioning: Option<bool>,
    /// Battery heater. Tesla also sends this as `battery_heater_on` on some cars.
    #[serde(default)]
    pub battery_heater: Option<bool>,
    /// Climate keeper mode.
    #[serde(default)]
    pub climate_keeper_mode: Option<String>,
    /// Driver seat heater level.
    #[serde(default)]
    pub seat_heater_left: Option<i64>,
    /// Passenger seat heater level.
    #[serde(default)]
    pub seat_heater_right: Option<i64>,
    /// Fan status.
    #[serde(default)]
    pub fan_status: Option<i64>,
    /// Block timestamp in milliseconds.
    #[serde(default)]
    pub timestamp: Option<i64>,
    /// Unknown climate fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// Subset of `drive_state`, including location when `location_data` was requested.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DriveState {
    /// WGS84 latitude.
    #[serde(default)]
    pub latitude: Option<f64>,
    /// WGS84 longitude.
    #[serde(default)]
    pub longitude: Option<f64>,
    /// Compass heading in degrees.
    #[serde(default)]
    pub heading: Option<f64>,
    /// Speed. Null when the car is stationary and not reporting it.
    #[serde(default)]
    pub speed: Option<f64>,
    /// Instantaneous power, kW.
    #[serde(default)]
    pub power: Option<f64>,
    /// Shift state such as `D`, `P`, `R`, or `N`.
    #[serde(default)]
    pub shift_state: Option<String>,
    /// GPS timestamp in seconds.
    #[serde(default)]
    pub gps_as_of: Option<i64>,
    /// Block timestamp in milliseconds.
    #[serde(default)]
    pub timestamp: Option<i64>,
    /// Unknown drive fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// Display units from `gui_settings`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuiSettings {
    /// Distance unit. The sample uses values such as `mi/hr`.
    #[serde(default)]
    pub gui_distance_units: Option<String>,
    /// Temperature unit.
    #[serde(default)]
    pub gui_temperature_units: Option<String>,
    /// Charge-rate unit.
    #[serde(default)]
    pub gui_charge_rate_units: Option<String>,
    /// 24-hour clock.
    #[serde(default)]
    pub gui_24_hour_time: Option<bool>,
    /// Block timestamp in milliseconds.
    #[serde(default)]
    pub timestamp: Option<i64>,
    /// Unknown GUI fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// Subset of `vehicle_config`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VehicleConfig {
    /// Car type such as `modely`.
    #[serde(default)]
    pub car_type: Option<String>,
    /// Trim badge.
    #[serde(default)]
    pub trim_badging: Option<String>,
    /// Exterior color.
    #[serde(default)]
    pub exterior_color: Option<String>,
    /// Wheel package.
    #[serde(default)]
    pub wheel_type: Option<String>,
    /// Charge port region.
    #[serde(default)]
    pub charge_port_type: Option<String>,
    /// Efficiency package.
    #[serde(default)]
    pub efficiency_package: Option<String>,
    /// Block timestamp in milliseconds.
    #[serde(default)]
    pub timestamp: Option<i64>,
    /// Unknown configuration fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// Subset of `vehicle_state`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VehicleState {
    /// Odometer in miles.
    #[serde(default)]
    pub odometer: Option<f64>,
    /// Whether the vehicle is locked.
    #[serde(default)]
    pub locked: Option<bool>,
    /// Sentry mode.
    #[serde(default)]
    pub sentry_mode: Option<bool>,
    /// Whether a user is present.
    #[serde(default)]
    pub is_user_present: Option<bool>,
    /// Firmware version string.
    #[serde(default)]
    pub car_version: Option<String>,
    /// Vehicle name stored on the car.
    #[serde(default)]
    pub vehicle_name: Option<String>,
    /// Software update progress.
    #[serde(default)]
    pub software_update: Option<SoftwareUpdate>,
    /// Driver front door.
    #[serde(default)]
    pub df: Option<i64>,
    /// Driver rear door.
    #[serde(default)]
    pub dr: Option<i64>,
    /// Passenger front door.
    #[serde(default)]
    pub pf: Option<i64>,
    /// Passenger rear door.
    #[serde(default)]
    pub pr: Option<i64>,
    /// Front trunk.
    #[serde(default)]
    pub ft: Option<i64>,
    /// Rear trunk.
    #[serde(default)]
    pub rt: Option<i64>,
    /// Driver front window. `0` is closed. A nonzero value is open.
    #[serde(default)]
    pub fd_window: Option<i64>,
    /// Passenger front window. `0` is closed. A nonzero value is open.
    #[serde(default)]
    pub fp_window: Option<i64>,
    /// Driver rear window. `0` is closed. A nonzero value is open.
    #[serde(default)]
    pub rd_window: Option<i64>,
    /// Passenger rear window. `0` is closed. A nonzero value is open.
    #[serde(default)]
    pub rp_window: Option<i64>,
    /// Front-left tire pressure, bar.
    #[serde(default)]
    pub tpms_pressure_fl: Option<f64>,
    /// Front-right tire pressure, bar.
    #[serde(default)]
    pub tpms_pressure_fr: Option<f64>,
    /// Rear-left tire pressure, bar.
    #[serde(default)]
    pub tpms_pressure_rl: Option<f64>,
    /// Rear-right tire pressure, bar.
    #[serde(default)]
    pub tpms_pressure_rr: Option<f64>,
    /// Center display state.
    #[serde(default)]
    pub center_display_state: Option<i64>,
    /// Block timestamp in milliseconds.
    #[serde(default)]
    pub timestamp: Option<i64>,
    /// Unknown vehicle-state fields, including media.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// `software_update` object inside `vehicle_state`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SoftwareUpdate {
    /// Download percent.
    #[serde(default)]
    pub download_perc: Option<f64>,
    /// Install percent.
    #[serde(default)]
    pub install_perc: Option<f64>,
    /// Expected install duration in seconds.
    #[serde(default)]
    pub expected_duration_sec: Option<i64>,
    /// Update status.
    #[serde(default)]
    pub status: Option<String>,
    /// Version being installed.
    #[serde(default)]
    pub version: Option<String>,
    /// Unknown update fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// `POST /api/1/vehicles/fleet_status` response.
///
/// The published example nests `firmware_version` under `vehicle_info`. The narrative also lists
/// `vehicle_command_protocol_required`, `fleet_telemetry_version`, `total_number_of_keys`,
/// `safety_screen_streaming_toggle_enabled`, and `discounted_device_data` without showing where
/// they sit. Those fields are decoded when they appear on the per-VIN object. `discounted_device_data`
/// is documented only for a single-VIN request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FleetStatus {
    /// VINs that have the partner virtual key.
    #[serde(default)]
    pub key_paired_vins: Vec<String>,
    /// VINs that do not have the partner virtual key.
    #[serde(default)]
    pub unpaired_vins: Vec<String>,
    /// Per-VIN details keyed by VIN.
    #[serde(default)]
    pub vehicle_info: BTreeMap<String, FleetVehicleInfo>,
    /// Unknown top-level fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// Per-VIN object inside [`FleetStatus::vehicle_info`].
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct FleetVehicleInfo {
    /// Vehicle firmware version.
    #[serde(default)]
    pub firmware_version: Option<String>,
    /// Whether signed commands are required.
    #[serde(default)]
    pub vehicle_command_protocol_required: Option<bool>,
    /// Fleet telemetry client version.
    #[serde(default)]
    pub fleet_telemetry_version: Option<String>,
    /// Keys installed on the vehicle. Tesla documents a maximum of 20.
    #[serde(default)]
    pub total_number_of_keys: Option<u32>,
    /// Safety-screen streaming toggle on older Intel Model S and Model X.
    #[serde(default)]
    pub safety_screen_streaming_toggle_enabled: Option<bool>,
    /// Whether `vehicle_data` calls for this VIN are discounted. Documented for single-VIN requests.
    #[serde(default)]
    pub discounted_device_data: Option<bool>,
    /// Unknown per-VIN fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// Body of `POST /api/1/vehicles/fleet_telemetry_config`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FleetTelemetryConfigRequest {
    /// Vehicles to configure.
    pub vins: Vec<String>,
    /// Desired streaming configuration.
    pub config: TelemetryConfig,
}

/// Fleet telemetry server configuration.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Hostname of the self-hosted fleet-telemetry server.
    pub hostname: String,
    /// CA certificate presented to vehicles.
    pub ca: String,
    /// Server port.
    pub port: u16,
    /// Fields to stream. Keys are Tesla field names such as `DriveRail`.
    pub fields: BTreeMap<String, TelemetryField>,
    /// Alert types the vehicle should publish.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alert_types: Vec<String>,
    /// Expiry as a unix timestamp in seconds. Absent means Tesla keeps the config indefinitely.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
    /// When `latest`, the vehicle resends every unacknowledged message.
    ///
    /// Requires Fleet Telemetry client 1.0.0 or later. Omit the field for the vehicle default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery_policy: Option<String>,
}

/// One streamed field.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct TelemetryField {
    /// Minimum seconds between published samples.
    pub interval_seconds: u32,
    /// Optional numeric deadband. Tesla recommends this to cut streaming cost.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum_delta: Option<f64>,
    /// Resend the field after this many seconds even when the value did not change.
    ///
    /// Requires firmware 2024.44.32 or later.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resend_interval_seconds: Option<u32>,
    /// Extra fields published in the same payload when this field is sent.
    ///
    /// Requires Fleet Telemetry client 1.3.0 or later.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include_fields: Vec<String>,
}

/// Response from creating or deleting a fleet telemetry config.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FleetTelemetryConfigUpdate {
    /// Vehicles whose configuration changed.
    #[serde(default)]
    pub updated_vehicles: u32,
    /// VINs Tesla refused to configure, grouped by reason.
    #[serde(default)]
    pub skipped_vehicles: SkippedVehicles,
    /// Unknown response fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// VINs skipped by fleet telemetry configuration.
///
/// Tesla's narrative mentions both a limit of three streaming configs and a `max_configs` reason
/// at five configs. Both keys are decoded; any other reason is kept in [`Self::other`].
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct SkippedVehicles {
    /// Virtual key is not on the vehicle.
    #[serde(default)]
    pub missing_key: Vec<String>,
    /// Hardware cannot run fleet telemetry.
    #[serde(default)]
    pub unsupported_hardware: Vec<String>,
    /// Firmware is older than the minimum for this request path.
    #[serde(default)]
    pub unsupported_firmware: Vec<String>,
    /// The vehicle already has the maximum number of configs.
    #[serde(default)]
    pub max_configs: Vec<String>,
    /// Reasons added after this crate was written.
    #[serde(default, flatten)]
    pub other: Map<String, Value>,
}

/// `GET /api/1/vehicles/{vin}/fleet_telemetry_config`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FleetTelemetryConfig {
    /// Whether the vehicle has adopted the config.
    pub synced: bool,
    /// Whether the vehicle cannot accept another application's config.
    #[serde(default)]
    pub limit_reached: Option<bool>,
    /// Configured target, when Tesla returned one.
    #[serde(default)]
    pub config: Option<TelemetryConfig>,
    /// Unknown response fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// Query for `GET /api/1/dx/charging/history`.
///
/// `start_time` and `end_time` use Tesla's timestamp format, for example
/// `2023-07-27T11:43:45-07:00`.
#[derive(Clone, Debug, Default)]
pub struct ChargingHistoryQuery {
    /// Limit the history to one VIN.
    pub vin: Option<String>,
    /// Window start.
    pub start_time: Option<String>,
    /// Window end.
    pub end_time: Option<String>,
    /// 1-based page number (`pageNo`).
    pub page_no: Option<u32>,
    /// Page size (`pageSize`).
    pub page_size: Option<u32>,
    /// Sort field (`sortBy`).
    pub sort_by: Option<String>,
    /// Sort direction.
    pub sort_order: Option<SortOrder>,
}

/// `sortOrder` query value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortOrder {
    /// Ascending.
    Asc,
    /// Descending.
    Desc,
}

impl SortOrder {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

/// `response` object from charging history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChargingHistory {
    /// Charging sessions.
    #[serde(default)]
    pub data: Vec<ChargingSession>,
    /// Unknown history fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// One charging session.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChargingSession {
    /// Session id.
    #[serde(default)]
    pub session_id: Option<i64>,
    /// Vehicle identification number.
    #[serde(default)]
    pub vin: Option<String>,
    /// Site name.
    #[serde(default)]
    pub site_location_name: Option<String>,
    /// Charge start timestamp.
    #[serde(default)]
    pub charge_start_date_time: Option<String>,
    /// Charge stop timestamp.
    #[serde(default)]
    pub charge_stop_date_time: Option<String>,
    /// Cable unlatch timestamp.
    #[serde(default)]
    pub unlatch_date_time: Option<String>,
    /// Country code.
    #[serde(default)]
    pub country_code: Option<String>,
    /// Fees charged for the session.
    #[serde(default)]
    pub fees: Vec<ChargingFee>,
    /// Billing type such as `IMMEDIATE`.
    #[serde(default)]
    pub billing_type: Option<String>,
    /// Invoices for the session.
    #[serde(default)]
    pub invoices: Vec<ChargingInvoice>,
    /// Vehicle make code.
    #[serde(default)]
    pub vehicle_make_type: Option<String>,
    /// Unknown session fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// One fee inside a charging session.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChargingFee {
    /// Fee id.
    #[serde(default)]
    pub session_fee_id: Option<i64>,
    /// Fee type such as `CHARGING` or `PARKING`.
    #[serde(default)]
    pub fee_type: Option<String>,
    /// ISO currency code.
    #[serde(default)]
    pub currency_code: Option<String>,
    /// Pricing type.
    #[serde(default)]
    pub pricing_type: Option<String>,
    /// Unit of measure, such as `kwh` or `min`.
    #[serde(default)]
    pub uom: Option<String>,
    /// Amount due.
    #[serde(default)]
    pub total_due: Option<f64>,
    /// Whether the fee is paid.
    #[serde(default)]
    pub is_paid: Option<bool>,
    /// Payment status.
    #[serde(default)]
    pub status: Option<String>,
    /// Unknown fee fields, including the tier breakdown.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// Invoice metadata. The PDF itself is a separate endpoint and is not downloaded here.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChargingInvoice {
    /// File name.
    #[serde(default)]
    pub file_name: Option<String>,
    /// Content id used with the invoice endpoint.
    #[serde(default)]
    pub content_id: Option<String>,
    /// Invoice type.
    #[serde(default)]
    pub invoice_type: Option<String>,
    /// Unknown invoice fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// `POST /api/1/partner_accounts` body.
#[derive(Clone, Debug, Serialize)]
pub struct RegisterPartnerAccount {
    /// Application domain. Tesla requires it to be lowercase.
    pub domain: String,
    /// Optional certificate signing request. The virtual-key public key is hosted at
    /// `https://{domain}/.well-known/appspecific/com.tesla.3p.public-key.pem` and is not sent here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub csr: Option<String>,
}

/// Partner account record.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartnerAccount {
    /// Partner client id.
    #[serde(default)]
    pub client_id: Option<String>,
    /// Account name.
    #[serde(default)]
    pub name: Option<String>,
    /// Account description.
    #[serde(default)]
    pub description: Option<String>,
    /// Registered domain.
    #[serde(default)]
    pub domain: Option<String>,
    /// CA certificate, when Tesla has issued one.
    #[serde(default)]
    pub ca: Option<String>,
    /// Creation timestamp.
    #[serde(default)]
    pub created_at: Option<String>,
    /// Update timestamp.
    #[serde(default)]
    pub updated_at: Option<String>,
    /// Enterprise tier.
    #[serde(default)]
    pub enterprise_tier: Option<String>,
    /// Account id.
    #[serde(default)]
    pub account_id: Option<String>,
    /// Issuer.
    #[serde(default)]
    pub issuer: Option<String>,
    /// Stored CSR.
    #[serde(default)]
    pub csr: Option<String>,
    /// When the CSR was updated.
    #[serde(default)]
    pub csr_updated_at: Option<String>,
    /// Registered public key, hex encoded.
    #[serde(default)]
    pub public_key: Option<String>,
    /// Unknown account fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// `GET /api/1/partner_accounts/public_key`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PartnerPublicKey {
    /// Hex-encoded public key registered for the domain.
    pub public_key: String,
    /// Unknown fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

/// `GET /api/1/users/region`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserRegion {
    /// Region code such as `na`, `eu`, or `cn`.
    pub region: String,
    /// Fleet API base URL for this user.
    pub fleet_api_base_url: String,
    /// Unknown fields.
    #[serde(default, flatten)]
    pub additional: Map<String, Value>,
}

impl UserRegion {
    /// Maps the response onto a known [`tesla_auth::Region`], preferring the base URL.
    #[must_use]
    pub fn known_region(&self) -> Option<tesla_auth::Region> {
        tesla_auth::Region::from_fleet_api_base(&self.fleet_api_base_url)
            .or_else(|| self.region.parse().ok())
    }
}

fn empty_vec_if_null<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    let value = Option::<Vec<T>>::deserialize(deserializer)?;
    Ok(value.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::VehicleData;

    #[test]
    fn vehicle_data_decodes_charge_closure_and_climate_fields() {
        let data: VehicleData = serde_json::from_str(
            r#"{
                "id": 1,
                "vin": "TEST00000000VIN01",
                "charge_state": {
                    "charge_current_request": 16,
                    "scheduled_charging_start_time": 1320,
                    "time_to_full_charge": 1.5
                },
                "climate_state": {"battery_heater": true, "is_preconditioning": false},
                "vehicle_state": {
                    "fd_window": 0,
                    "df": 1,
                    "tpms_pressure_fl": 2.9
                }
            }"#,
        )
        .unwrap();
        let charge = data.charge_state.unwrap();
        assert_eq!(charge.charge_current_request, Some(16.0));
        assert_eq!(charge.scheduled_charging_start_time, Some(1320.0));
        assert!(charge.additional.is_empty());
        let climate = data.climate_state.unwrap();
        assert_eq!(climate.battery_heater, Some(true));
        assert_eq!(climate.is_preconditioning, Some(false));
        let state = data.vehicle_state.unwrap();
        assert_eq!(state.fd_window, Some(0));
        assert_eq!(state.df, Some(1));
        assert_eq!(state.tpms_pressure_fl, Some(2.9));
    }
}
