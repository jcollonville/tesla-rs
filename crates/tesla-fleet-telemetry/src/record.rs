// SPDX-License-Identifier: MIT OR Apache-2.0

//! Idiomatic views of the Fleet Telemetry protobuf bodies.
//!
//! [`decode_vehicle_data`] and the other decoders keep the prost message available through
//! [`VehicleData::raw`] and apply the same body fixes as `telemetry/record.go` in the reference
//! server: location strings such as `(37.4 N, 122.1 W)` become [`LocationValue`], scientific-notation
//! strings are rewritten with five decimal places, client version `1.3.0` shifts
//! `AutomaticEmergencyBrakingState` by one, and alert timestamps whose `seconds` field is actually
//! unix milliseconds are converted.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use prost::Message;

use crate::error::TelemetryError;
use crate::proto::vehicle_alerts::{self, Audience};
use crate::proto::vehicle_connectivity::{self, ConnectivityEvent};
use crate::proto::vehicle_data::value::Value as RawOneof;
use crate::proto::vehicle_data::{
    self, AutomaticEmergencyBrakingStateValue, BmsStateValue, BuckleStatus,
    CabinOverheatProtectionModeState, CabinPortKeepOnState, CableType, CarTypeValue,
    ChargePortLatchValue, ChargePortValue, ChargeUnitPreference, ChargingState,
    ClimateKeeperModeState, ClimateOverheatProtectionTempLimit, DefrostModeState,
    DetailedChargeStateValue, DisplayState, DistanceUnit, Doors, DriveInverterState, FastCharger,
    Field, FollowDistance, ForwardCollisionSensitivity, GuestModeMobileAccess, HvacAutoModeState,
    HvacPowerState, HvilStatus, LaneAssistLevel, LocationValue, MediaStatus, Payload,
    PowershareState, PowershareStopReasonStatus, PowershareTypeStatus, PressureUnit,
    ScheduledChargingModeValue, SeatFoldPosition, SentryModeState, ShiftState, SpeedAssistLevel,
    SunroofInstalledState, TemperatureUnit, Time, TireLocation, TonneauPositionState,
    TonneauTentModeState, TractorAirStatus, TrailerAirStatus, TurnSignalState, WindowState,
};
use crate::proto::vehicle_error;

/// Reference-server limit for a timestamp `seconds` field that is really unix milliseconds.
const MAX_SECONDS_IN_DURATION: i64 = 315_576_000_000;

/// Options for the reference server's payload fixes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Replaces the VIN inside the protobuf. The reference server writes the certificate device id.
    pub vin_override: Option<String>,
    /// Vehicle `Version` header. `1.3.0` remaps legacy automatic-emergency-braking values.
    pub device_client_version: Option<String>,
}

/// Unix timestamp copied out of `google.protobuf.Timestamp`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timestamp {
    /// Seconds since the unix epoch.
    pub seconds: i64,
    /// Non-negative nanosecond fraction, matching protobuf's normalized form after conversion.
    pub nanos: i32,
}

impl Timestamp {
    fn from_prost(value: &prost_types::Timestamp) -> Self {
        Self {
            seconds: value.seconds,
            nanos: value.nanos,
        }
    }

    /// Converts this timestamp into [`SystemTime`] when it falls in range.
    #[must_use]
    pub fn to_system_time(self) -> Option<SystemTime> {
        if self.seconds < 0 || self.nanos < 0 {
            return None;
        }
        UNIX_EPOCH
            .checked_add(Duration::from_secs(u64::try_from(self.seconds).ok()?))
            .and_then(|time| {
                time.checked_add(Duration::from_nanos(u64::from(
                    u32::try_from(self.nanos).ok()?,
                )))
            })
    }
}

/// A protobuf enum value, or a discriminant the vendored schema does not name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedEnum<T> {
    /// Discriminant present in the vendored protobuf.
    Known(T),
    /// Discriminant the current schema does not name.
    Unknown(i32),
}

impl<T: TryFrom<i32>> TypedEnum<T> {
    fn from_i32(value: i32) -> Self {
        match T::try_from(value) {
            Ok(value) => Self::Known(value),
            Err(_) => Self::Unknown(value),
        }
    }
}

/// Field key on a datum. Unknown numbers stay available instead of collapsing to `Field::Unknown`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldKey {
    /// Field named in `vehicle_data.proto`.
    Known(Field),
    /// Number that is not in the vendored schema.
    Unknown(i32),
}

impl FieldKey {
    /// Proto enum name, or `field_<n>` for a number this schema does not name.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::Known(field) => field.as_str_name().to_owned(),
            Self::Unknown(value) => format!("field_{value}"),
        }
    }

    fn from_i32(value: i32) -> Self {
        match Field::try_from(value) {
            Ok(field) => Self::Known(field),
            Err(_) => Self::Unknown(value),
        }
    }
}

/// One typed `Value` oneof arm.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum DatumValue {
    /// Protobuf field 1.
    String(String),
    /// Protobuf field 2.
    Int(i32),
    /// Protobuf field 3.
    Long(i64),
    /// Protobuf field 4.
    Float(f32),
    /// Protobuf field 5.
    Double(f64),
    /// Protobuf field 6.
    Boolean(bool),
    /// Protobuf field 7.
    Location(LocationValue),
    /// Protobuf field 8.
    ChargingState(TypedEnum<ChargingState>),
    /// Protobuf field 9.
    ShiftState(TypedEnum<ShiftState>),
    /// Protobuf field 10. Tesla uses this arm to mark an unusable sample.
    Invalid(bool),
    /// Protobuf field 11.
    LaneAssistLevel(TypedEnum<LaneAssistLevel>),
    /// Protobuf field 12.
    ScheduledChargingMode(TypedEnum<ScheduledChargingModeValue>),
    /// Protobuf field 13.
    SentryModeState(TypedEnum<SentryModeState>),
    /// Protobuf field 14.
    SpeedAssistLevel(TypedEnum<SpeedAssistLevel>),
    /// Protobuf field 15.
    BmsState(TypedEnum<BmsStateValue>),
    /// Protobuf field 16.
    BuckleStatus(TypedEnum<BuckleStatus>),
    /// Protobuf field 17.
    CarType(TypedEnum<CarTypeValue>),
    /// Protobuf field 18.
    ChargePort(TypedEnum<ChargePortValue>),
    /// Protobuf field 19.
    ChargePortLatch(TypedEnum<ChargePortLatchValue>),
    /// Protobuf field 21.
    Doors(Doors),
    /// Protobuf field 22.
    DriveInverterState(TypedEnum<DriveInverterState>),
    /// Protobuf field 23.
    HvilStatus(TypedEnum<HvilStatus>),
    /// Protobuf field 24.
    WindowState(TypedEnum<WindowState>),
    /// Protobuf field 25.
    SeatFoldPosition(TypedEnum<SeatFoldPosition>),
    /// Protobuf field 26.
    TractorAirStatus(TypedEnum<TractorAirStatus>),
    /// Protobuf field 27.
    FollowDistance(TypedEnum<FollowDistance>),
    /// Protobuf field 28.
    ForwardCollisionSensitivity(TypedEnum<ForwardCollisionSensitivity>),
    /// Protobuf field 29.
    GuestModeMobileAccess(TypedEnum<GuestModeMobileAccess>),
    /// Protobuf field 30.
    TrailerAirStatus(TypedEnum<TrailerAirStatus>),
    /// Protobuf field 31.
    Time(Time),
    /// Protobuf field 32.
    DetailedChargeState(TypedEnum<DetailedChargeStateValue>),
    /// Protobuf field 33.
    HvacAutoMode(TypedEnum<HvacAutoModeState>),
    /// Protobuf field 34.
    CabinOverheatProtectionMode(TypedEnum<CabinOverheatProtectionModeState>),
    /// Protobuf field 35.
    CabinOverheatProtectionTempLimit(TypedEnum<ClimateOverheatProtectionTempLimit>),
    /// Protobuf field 36.
    DefrostMode(TypedEnum<DefrostModeState>),
    /// Protobuf field 37.
    ClimateKeeperMode(TypedEnum<ClimateKeeperModeState>),
    /// Protobuf field 38.
    HvacPower(TypedEnum<HvacPowerState>),
    /// Protobuf field 39.
    TireLocation(TireLocation),
    /// Protobuf field 40.
    FastCharger(TypedEnum<FastCharger>),
    /// Protobuf field 41.
    CableType(TypedEnum<CableType>),
    /// Protobuf field 42.
    TonneauTentMode(TypedEnum<TonneauTentModeState>),
    /// Protobuf field 43.
    TonneauPosition(TypedEnum<TonneauPositionState>),
    /// Protobuf field 44.
    PowershareType(TypedEnum<PowershareTypeStatus>),
    /// Protobuf field 45.
    PowershareState(TypedEnum<PowershareState>),
    /// Protobuf field 46.
    PowershareStopReason(TypedEnum<PowershareStopReasonStatus>),
    /// Protobuf field 47.
    DisplayState(TypedEnum<DisplayState>),
    /// Protobuf field 48.
    DistanceUnit(TypedEnum<DistanceUnit>),
    /// Protobuf field 49.
    TemperatureUnit(TypedEnum<TemperatureUnit>),
    /// Protobuf field 50.
    PressureUnit(TypedEnum<PressureUnit>),
    /// Protobuf field 51.
    ChargeUnitPreference(TypedEnum<ChargeUnitPreference>),
    /// Protobuf field 52.
    TurnSignalState(TypedEnum<TurnSignalState>),
    /// Protobuf field 53.
    MediaStatus(TypedEnum<MediaStatus>),
    /// Protobuf field 54.
    SunroofInstalledState(TypedEnum<SunroofInstalledState>),
    /// Protobuf field 55.
    CabinPortKeepOn(TypedEnum<CabinPortKeepOnState>),
    /// Protobuf field 57.
    PowerTransferStatus(TypedEnum<vehicle_data::PowerTransferStatusValue>),
    /// Protobuf field 60.
    AutomaticEmergencyBrakingState(TypedEnum<AutomaticEmergencyBrakingStateValue>),
    /// The `Value` message was present and its oneof was empty.
    Absent,
}

impl fmt::Display for DatumValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String(value) => write!(formatter, "{value}"),
            Self::Int(value) => write!(formatter, "{value}"),
            Self::Long(value) => write!(formatter, "{value}"),
            Self::Float(value) => write!(formatter, "{value}"),
            Self::Double(value) => write!(formatter, "{value}"),
            Self::Boolean(value) => write!(formatter, "{value}"),
            Self::Location(value) => {
                write!(formatter, "{},{}", value.latitude, value.longitude)
            }
            Self::Invalid(value) => write!(formatter, "invalid:{value}"),
            Self::Doors(value) => write!(formatter, "{value:?}"),
            Self::Time(value) => {
                write!(
                    formatter,
                    "{:02}:{:02}:{:02}",
                    value.hour, value.minute, value.second
                )
            }
            Self::TireLocation(value) => write!(formatter, "{value:?}"),
            Self::ChargingState(value) => write_enum(formatter, value, ChargingState::as_str_name),
            Self::ShiftState(value) => write_enum(formatter, value, ShiftState::as_str_name),
            Self::LaneAssistLevel(value) => {
                write_enum(formatter, value, LaneAssistLevel::as_str_name)
            }
            Self::ScheduledChargingMode(value) => {
                write_enum(formatter, value, ScheduledChargingModeValue::as_str_name)
            }
            Self::SentryModeState(value) => {
                write_enum(formatter, value, SentryModeState::as_str_name)
            }
            Self::SpeedAssistLevel(value) => {
                write_enum(formatter, value, SpeedAssistLevel::as_str_name)
            }
            Self::BmsState(value) => write_enum(formatter, value, BmsStateValue::as_str_name),
            Self::BuckleStatus(value) => write_enum(formatter, value, BuckleStatus::as_str_name),
            Self::CarType(value) => write_enum(formatter, value, CarTypeValue::as_str_name),
            Self::ChargePort(value) => write_enum(formatter, value, ChargePortValue::as_str_name),
            Self::ChargePortLatch(value) => {
                write_enum(formatter, value, ChargePortLatchValue::as_str_name)
            }
            Self::DriveInverterState(value) => {
                write_enum(formatter, value, DriveInverterState::as_str_name)
            }
            Self::HvilStatus(value) => write_enum(formatter, value, HvilStatus::as_str_name),
            Self::WindowState(value) => write_enum(formatter, value, WindowState::as_str_name),
            Self::SeatFoldPosition(value) => {
                write_enum(formatter, value, SeatFoldPosition::as_str_name)
            }
            Self::TractorAirStatus(value) => {
                write_enum(formatter, value, TractorAirStatus::as_str_name)
            }
            Self::FollowDistance(value) => {
                write_enum(formatter, value, FollowDistance::as_str_name)
            }
            Self::ForwardCollisionSensitivity(value) => {
                write_enum(formatter, value, ForwardCollisionSensitivity::as_str_name)
            }
            Self::GuestModeMobileAccess(value) => {
                write_enum(formatter, value, GuestModeMobileAccess::as_str_name)
            }
            Self::TrailerAirStatus(value) => {
                write_enum(formatter, value, TrailerAirStatus::as_str_name)
            }
            Self::DetailedChargeState(value) => {
                write_enum(formatter, value, DetailedChargeStateValue::as_str_name)
            }
            Self::HvacAutoMode(value) => {
                write_enum(formatter, value, HvacAutoModeState::as_str_name)
            }
            Self::CabinOverheatProtectionMode(value) => write_enum(
                formatter,
                value,
                CabinOverheatProtectionModeState::as_str_name,
            ),
            Self::CabinOverheatProtectionTempLimit(value) => write_enum(
                formatter,
                value,
                ClimateOverheatProtectionTempLimit::as_str_name,
            ),
            Self::DefrostMode(value) => write_enum(formatter, value, DefrostModeState::as_str_name),
            Self::ClimateKeeperMode(value) => {
                write_enum(formatter, value, ClimateKeeperModeState::as_str_name)
            }
            Self::HvacPower(value) => write_enum(formatter, value, HvacPowerState::as_str_name),
            Self::FastCharger(value) => write_enum(formatter, value, FastCharger::as_str_name),
            Self::CableType(value) => write_enum(formatter, value, CableType::as_str_name),
            Self::TonneauTentMode(value) => {
                write_enum(formatter, value, TonneauTentModeState::as_str_name)
            }
            Self::TonneauPosition(value) => {
                write_enum(formatter, value, TonneauPositionState::as_str_name)
            }
            Self::PowershareType(value) => {
                write_enum(formatter, value, PowershareTypeStatus::as_str_name)
            }
            Self::PowershareState(value) => {
                write_enum(formatter, value, PowershareState::as_str_name)
            }
            Self::PowershareStopReason(value) => {
                write_enum(formatter, value, PowershareStopReasonStatus::as_str_name)
            }
            Self::DisplayState(value) => write_enum(formatter, value, DisplayState::as_str_name),
            Self::DistanceUnit(value) => write_enum(formatter, value, DistanceUnit::as_str_name),
            Self::TemperatureUnit(value) => {
                write_enum(formatter, value, TemperatureUnit::as_str_name)
            }
            Self::PressureUnit(value) => write_enum(formatter, value, PressureUnit::as_str_name),
            Self::ChargeUnitPreference(value) => {
                write_enum(formatter, value, ChargeUnitPreference::as_str_name)
            }
            Self::TurnSignalState(value) => {
                write_enum(formatter, value, TurnSignalState::as_str_name)
            }
            Self::MediaStatus(value) => write_enum(formatter, value, MediaStatus::as_str_name),
            Self::SunroofInstalledState(value) => {
                write_enum(formatter, value, SunroofInstalledState::as_str_name)
            }
            Self::CabinPortKeepOn(value) => {
                write_enum(formatter, value, CabinPortKeepOnState::as_str_name)
            }
            Self::PowerTransferStatus(value) => write_enum(
                formatter,
                value,
                vehicle_data::PowerTransferStatusValue::as_str_name,
            ),
            Self::AutomaticEmergencyBrakingState(value) => write_enum(
                formatter,
                value,
                AutomaticEmergencyBrakingStateValue::as_str_name,
            ),
            Self::Absent => formatter.write_str("absent"),
        }
    }
}

fn write_enum<T>(
    formatter: &mut fmt::Formatter<'_>,
    value: &TypedEnum<T>,
    name: fn(&T) -> &'static str,
) -> fmt::Result {
    match value {
        TypedEnum::Known(value) => formatter.write_str(name(value)),
        TypedEnum::Unknown(value) => write!(formatter, "unknown:{value}"),
    }
}

/// One datum after the reference-server transforms.
#[derive(Clone, Debug, PartialEq)]
pub struct DecodedDatum {
    /// Signal name.
    pub field: FieldKey,
    /// Typed value. [`DatumValue::Absent`] means the protobuf omitted the oneof.
    pub value: DatumValue,
}

/// `Payload` for topic `V`.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleData {
    raw: Payload,
}

impl VehicleData {
    /// Prost message after the reference-server transforms.
    #[must_use]
    pub fn raw(&self) -> &Payload {
        &self.raw
    }

    /// VIN from the payload, after an optional certificate override.
    #[must_use]
    pub fn vin(&self) -> &str {
        &self.raw.vin
    }

    /// Whether the vehicle marked this payload as a resend.
    #[must_use]
    pub fn is_resend(&self) -> bool {
        self.raw.is_resend
    }

    /// Payload creation time.
    #[must_use]
    pub fn created_at(&self) -> Option<Timestamp> {
        self.raw.created_at.as_ref().map(Timestamp::from_prost)
    }

    /// Signals in payload order.
    pub fn iter(&self) -> impl Iterator<Item = DecodedDatum> + '_ {
        self.raw.data.iter().map(decode_datum)
    }
}

/// `VehicleAlerts` for topic `alerts`.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleAlerts {
    raw: vehicle_alerts::VehicleAlerts,
}

impl VehicleAlerts {
    /// Prost message after timestamp normalization.
    #[must_use]
    pub fn raw(&self) -> &vehicle_alerts::VehicleAlerts {
        &self.raw
    }

    /// VIN after an optional certificate override.
    #[must_use]
    pub fn vin(&self) -> &str {
        &self.raw.vin
    }

    /// Collection timestamp.
    #[must_use]
    pub fn created_at(&self) -> Option<Timestamp> {
        self.raw.created_at.as_ref().map(Timestamp::from_prost)
    }

    /// Alerts in payload order.
    #[must_use]
    pub fn alerts(&self) -> Vec<VehicleAlert> {
        self.raw
            .alerts
            .iter()
            .map(|alert| VehicleAlert {
                name: alert.name.clone(),
                audiences: alert
                    .audiences
                    .iter()
                    .copied()
                    .map(TypedEnum::from_i32)
                    .collect(),
                started_at: alert.started_at.as_ref().map(Timestamp::from_prost),
                ended_at: alert.ended_at.as_ref().map(Timestamp::from_prost),
            })
            .collect()
    }
}

/// One alert. Active alerts have [`Self::ended_at`] equal to `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VehicleAlert {
    /// Alert name.
    pub name: String,
    /// Audiences Tesla addressed.
    pub audiences: Vec<TypedEnum<Audience>>,
    /// When the alert started.
    pub started_at: Option<Timestamp>,
    /// When the alert ended. `None` means it is still active.
    pub ended_at: Option<Timestamp>,
}

/// `VehicleErrors` for topic `errors`.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleErrors {
    raw: vehicle_error::VehicleErrors,
}

impl VehicleErrors {
    /// Prost message.
    #[must_use]
    pub fn raw(&self) -> &vehicle_error::VehicleErrors {
        &self.raw
    }

    /// VIN after an optional certificate override.
    #[must_use]
    pub fn vin(&self) -> &str {
        &self.raw.vin
    }

    /// Collection timestamp.
    #[must_use]
    pub fn created_at(&self) -> Option<Timestamp> {
        self.raw.created_at.as_ref().map(Timestamp::from_prost)
    }

    /// Errors in payload order.
    #[must_use]
    pub fn errors(&self) -> &[vehicle_error::VehicleError] {
        &self.raw.errors
    }
}

/// `VehicleConnectivity` for topic `connectivity`.
#[derive(Clone, Debug, PartialEq)]
pub struct VehicleConnectivity {
    raw: vehicle_connectivity::VehicleConnectivity,
}

impl VehicleConnectivity {
    /// Prost message.
    #[must_use]
    pub fn raw(&self) -> &vehicle_connectivity::VehicleConnectivity {
        &self.raw
    }

    /// VIN after an optional certificate override.
    #[must_use]
    pub fn vin(&self) -> &str {
        &self.raw.vin
    }

    /// Socket id assigned by the server that observed the transition.
    #[must_use]
    pub fn connection_id(&self) -> &str {
        &self.raw.connection_id
    }

    /// Connected or disconnected.
    #[must_use]
    pub fn status(&self) -> TypedEnum<ConnectivityEvent> {
        TypedEnum::from_i32(self.raw.status)
    }

    /// When the transition was recorded.
    #[must_use]
    pub fn created_at(&self) -> Option<Timestamp> {
        self.raw.created_at.as_ref().map(Timestamp::from_prost)
    }

    /// `X-Network-Interface` from the vehicle handshake, when the server copied it in.
    #[must_use]
    pub fn network_interface(&self) -> &str {
        &self.raw.network_interface
    }
}

/// A decoded telemetry body.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Decoded {
    /// Topic `V`.
    VehicleData(VehicleData),
    /// Topic `alerts`.
    Alerts(VehicleAlerts),
    /// Topic `errors`.
    Errors(VehicleErrors),
    /// Topic `connectivity`.
    Connectivity(VehicleConnectivity),
    /// Envelope topic this crate does not decode. The original bytes stay on the record.
    Unrecognized,
}

/// Decodes a `V` payload.
///
/// # Errors
///
/// Returns [`TelemetryError::Protobuf`] when `bytes` is not a `Payload`.
pub fn decode_vehicle_data(
    bytes: &[u8],
    options: &DecodeOptions,
) -> Result<VehicleData, TelemetryError> {
    let mut raw = Payload::decode(bytes).map_err(TelemetryError::Protobuf)?;
    apply_vin(&mut raw.vin, options);
    transform_location(&mut raw);
    transform_scientific_notation(&mut raw);
    transform_legacy_aeb(&mut raw, options.device_client_version.as_deref());
    Ok(VehicleData { raw })
}

/// Decodes an `alerts` payload.
///
/// # Errors
///
/// Returns [`TelemetryError::Protobuf`] when `bytes` is not `VehicleAlerts`.
pub fn decode_alerts(
    bytes: &[u8],
    options: &DecodeOptions,
) -> Result<VehicleAlerts, TelemetryError> {
    let mut raw = vehicle_alerts::VehicleAlerts::decode(bytes).map_err(TelemetryError::Protobuf)?;
    apply_vin(&mut raw.vin, options);
    for alert in &mut raw.alerts {
        alert.started_at = alert.started_at.take().map(normalize_timestamp);
        alert.ended_at = alert.ended_at.take().map(normalize_timestamp);
    }
    Ok(VehicleAlerts { raw })
}

/// Decodes an `errors` payload.
///
/// # Errors
///
/// Returns [`TelemetryError::Protobuf`] when `bytes` is not `VehicleErrors`.
pub fn decode_errors(
    bytes: &[u8],
    options: &DecodeOptions,
) -> Result<VehicleErrors, TelemetryError> {
    let mut raw = vehicle_error::VehicleErrors::decode(bytes).map_err(TelemetryError::Protobuf)?;
    apply_vin(&mut raw.vin, options);
    Ok(VehicleErrors { raw })
}

/// Decodes a `connectivity` payload.
///
/// # Errors
///
/// Returns [`TelemetryError::Protobuf`] when `bytes` is not `VehicleConnectivity`.
pub fn decode_connectivity(
    bytes: &[u8],
    options: &DecodeOptions,
) -> Result<VehicleConnectivity, TelemetryError> {
    let mut raw = vehicle_connectivity::VehicleConnectivity::decode(bytes)
        .map_err(TelemetryError::Protobuf)?;
    apply_vin(&mut raw.vin, options);
    Ok(VehicleConnectivity { raw })
}

/// Decodes a payload for one of the four reference-server topics.
///
/// # Errors
///
/// Returns [`TelemetryError::InvalidConfig`] for an unsupported topic and
/// [`TelemetryError::Protobuf`] when the body does not match that topic.
pub fn decode_topic(
    topic: &str,
    bytes: &[u8],
    options: &DecodeOptions,
) -> Result<Decoded, TelemetryError> {
    match topic {
        "V" => Ok(Decoded::VehicleData(decode_vehicle_data(bytes, options)?)),
        "alerts" => Ok(Decoded::Alerts(decode_alerts(bytes, options)?)),
        "errors" => Ok(Decoded::Errors(decode_errors(bytes, options)?)),
        "connectivity" => Ok(Decoded::Connectivity(decode_connectivity(bytes, options)?)),
        _ => Err(TelemetryError::InvalidConfig(
            "topic is not V, alerts, errors, or connectivity",
        )),
    }
}

fn apply_vin(vin: &mut String, options: &DecodeOptions) {
    if let Some(override_vin) = &options.vin_override {
        vin.clone_from(override_vin);
    }
}

fn decode_datum(datum: &vehicle_data::Datum) -> DecodedDatum {
    DecodedDatum {
        field: FieldKey::from_i32(datum.key),
        value: datum
            .value
            .as_ref()
            .and_then(|value| value.value.as_ref())
            .map_or(DatumValue::Absent, datum_value),
    }
}

fn datum_value(value: &RawOneof) -> DatumValue {
    match value {
        RawOneof::StringValue(value) => DatumValue::String(value.clone()),
        RawOneof::IntValue(value) => DatumValue::Int(*value),
        RawOneof::LongValue(value) => DatumValue::Long(*value),
        RawOneof::FloatValue(value) => DatumValue::Float(*value),
        RawOneof::DoubleValue(value) => DatumValue::Double(*value),
        RawOneof::BooleanValue(value) => DatumValue::Boolean(*value),
        RawOneof::LocationValue(value) => DatumValue::Location(*value),
        RawOneof::ChargingValue(value) => DatumValue::ChargingState(TypedEnum::from_i32(*value)),
        RawOneof::ShiftStateValue(value) => DatumValue::ShiftState(TypedEnum::from_i32(*value)),
        RawOneof::Invalid(value) => DatumValue::Invalid(*value),
        RawOneof::LaneAssistLevelValue(value) => {
            DatumValue::LaneAssistLevel(TypedEnum::from_i32(*value))
        }
        RawOneof::ScheduledChargingModeValue(value) => {
            DatumValue::ScheduledChargingMode(TypedEnum::from_i32(*value))
        }
        RawOneof::SentryModeStateValue(value) => {
            DatumValue::SentryModeState(TypedEnum::from_i32(*value))
        }
        RawOneof::SpeedAssistLevelValue(value) => {
            DatumValue::SpeedAssistLevel(TypedEnum::from_i32(*value))
        }
        RawOneof::BmsStateValue(value) => DatumValue::BmsState(TypedEnum::from_i32(*value)),
        RawOneof::BuckleStatusValue(value) => DatumValue::BuckleStatus(TypedEnum::from_i32(*value)),
        RawOneof::CarTypeValue(value) => DatumValue::CarType(TypedEnum::from_i32(*value)),
        RawOneof::ChargePortValue(value) => DatumValue::ChargePort(TypedEnum::from_i32(*value)),
        RawOneof::ChargePortLatchValue(value) => {
            DatumValue::ChargePortLatch(TypedEnum::from_i32(*value))
        }
        RawOneof::DoorValue(value) => DatumValue::Doors(*value),
        RawOneof::DriveInverterStateValue(value) => {
            DatumValue::DriveInverterState(TypedEnum::from_i32(*value))
        }
        RawOneof::HvilStatusValue(value) => DatumValue::HvilStatus(TypedEnum::from_i32(*value)),
        RawOneof::WindowStateValue(value) => DatumValue::WindowState(TypedEnum::from_i32(*value)),
        RawOneof::SeatFoldPositionValue(value) => {
            DatumValue::SeatFoldPosition(TypedEnum::from_i32(*value))
        }
        RawOneof::TractorAirStatusValue(value) => {
            DatumValue::TractorAirStatus(TypedEnum::from_i32(*value))
        }
        RawOneof::FollowDistanceValue(value) => {
            DatumValue::FollowDistance(TypedEnum::from_i32(*value))
        }
        RawOneof::ForwardCollisionSensitivityValue(value) => {
            DatumValue::ForwardCollisionSensitivity(TypedEnum::from_i32(*value))
        }
        RawOneof::GuestModeMobileAccessValue(value) => {
            DatumValue::GuestModeMobileAccess(TypedEnum::from_i32(*value))
        }
        RawOneof::TrailerAirStatusValue(value) => {
            DatumValue::TrailerAirStatus(TypedEnum::from_i32(*value))
        }
        RawOneof::TimeValue(value) => DatumValue::Time(*value),
        RawOneof::DetailedChargeStateValue(value) => {
            DatumValue::DetailedChargeState(TypedEnum::from_i32(*value))
        }
        RawOneof::HvacAutoModeValue(value) => DatumValue::HvacAutoMode(TypedEnum::from_i32(*value)),
        RawOneof::CabinOverheatProtectionModeValue(value) => {
            DatumValue::CabinOverheatProtectionMode(TypedEnum::from_i32(*value))
        }
        RawOneof::CabinOverheatProtectionTemperatureLimitValue(value) => {
            DatumValue::CabinOverheatProtectionTempLimit(TypedEnum::from_i32(*value))
        }
        RawOneof::DefrostModeValue(value) => DatumValue::DefrostMode(TypedEnum::from_i32(*value)),
        RawOneof::ClimateKeeperModeValue(value) => {
            DatumValue::ClimateKeeperMode(TypedEnum::from_i32(*value))
        }
        RawOneof::HvacPowerValue(value) => DatumValue::HvacPower(TypedEnum::from_i32(*value)),
        RawOneof::TireLocationValue(value) => DatumValue::TireLocation(*value),
        RawOneof::FastChargerValue(value) => DatumValue::FastCharger(TypedEnum::from_i32(*value)),
        RawOneof::CableTypeValue(value) => DatumValue::CableType(TypedEnum::from_i32(*value)),
        RawOneof::TonneauTentModeValue(value) => {
            DatumValue::TonneauTentMode(TypedEnum::from_i32(*value))
        }
        RawOneof::TonneauPositionValue(value) => {
            DatumValue::TonneauPosition(TypedEnum::from_i32(*value))
        }
        RawOneof::PowershareTypeValue(value) => {
            DatumValue::PowershareType(TypedEnum::from_i32(*value))
        }
        RawOneof::PowershareStateValue(value) => {
            DatumValue::PowershareState(TypedEnum::from_i32(*value))
        }
        RawOneof::PowershareStopReasonValue(value) => {
            DatumValue::PowershareStopReason(TypedEnum::from_i32(*value))
        }
        RawOneof::DisplayStateValue(value) => DatumValue::DisplayState(TypedEnum::from_i32(*value)),
        RawOneof::DistanceUnitValue(value) => DatumValue::DistanceUnit(TypedEnum::from_i32(*value)),
        RawOneof::TemperatureUnitValue(value) => {
            DatumValue::TemperatureUnit(TypedEnum::from_i32(*value))
        }
        RawOneof::PressureUnitValue(value) => DatumValue::PressureUnit(TypedEnum::from_i32(*value)),
        RawOneof::ChargeUnitPreferenceValue(value) => {
            DatumValue::ChargeUnitPreference(TypedEnum::from_i32(*value))
        }
        RawOneof::TurnSignalStateValue(value) => {
            DatumValue::TurnSignalState(TypedEnum::from_i32(*value))
        }
        RawOneof::MediaStatusValue(value) => DatumValue::MediaStatus(TypedEnum::from_i32(*value)),
        RawOneof::SunroofInstalledStateValue(value) => {
            DatumValue::SunroofInstalledState(TypedEnum::from_i32(*value))
        }
        RawOneof::CabinPortKeepOnValue(value) => {
            DatumValue::CabinPortKeepOn(TypedEnum::from_i32(*value))
        }
        RawOneof::PowerTransferStatusValue(value) => {
            DatumValue::PowerTransferStatus(TypedEnum::from_i32(*value))
        }
        RawOneof::AutomaticEmergencyBrakingStateValue(value) => {
            DatumValue::AutomaticEmergencyBrakingState(TypedEnum::from_i32(*value))
        }
    }
}

fn transform_location(payload: &mut Payload) {
    for datum in &mut payload.data {
        if datum.key != i32::from(Field::Location) {
            continue;
        }
        let Some(value) = datum.value.as_mut() else {
            return;
        };
        let Some(RawOneof::StringValue(text)) = value.value.as_ref() else {
            return;
        };
        let text = text.clone();
        if let Some((latitude, longitude)) = parse_location(&text) {
            value.value = Some(RawOneof::LocationValue(LocationValue {
                latitude,
                longitude,
            }));
        }
        return;
    }
}

fn transform_scientific_notation(payload: &mut Payload) {
    for datum in &mut payload.data {
        let Some(value) = datum.value.as_mut() else {
            continue;
        };
        let Some(RawOneof::StringValue(text)) = value.value.as_mut() else {
            continue;
        };
        if let Some(rewritten) = scientific_decimal(text) {
            *text = rewritten;
        }
    }
}

fn transform_legacy_aeb(payload: &mut Payload, version: Option<&str>) {
    if version != Some("1.3.0") {
        return;
    }
    for datum in &mut payload.data {
        if datum.key != i32::from(Field::AutomaticEmergencyBrakingState) {
            continue;
        }
        let Some(value) = datum.value.as_mut() else {
            continue;
        };
        let Some(RawOneof::AutomaticEmergencyBrakingStateValue(state)) = value.value.as_mut()
        else {
            continue;
        };
        if (0..=4).contains(state) {
            *state = state.wrapping_add(1) % 5;
        }
    }
}

fn parse_location(input: &str) -> Option<(f64, f64)> {
    let inner = input.strip_prefix('(')?.strip_suffix(')')?;
    let (latitude, longitude) = inner.split_once(", ")?;
    let (latitude, latitude_hemisphere) = latitude.rsplit_once(' ')?;
    let (longitude, longitude_hemisphere) = longitude.rsplit_once(' ')?;
    if latitude_hemisphere.len() != 1 || longitude_hemisphere.len() != 1 {
        return None;
    }
    let mut latitude = latitude.parse::<f64>().ok()?;
    let mut longitude = longitude.parse::<f64>().ok()?;
    match latitude_hemisphere {
        "N" => {}
        "S" => latitude = -latitude,
        _ => return None,
    }
    match longitude_hemisphere {
        "E" => {}
        "W" => longitude = -longitude,
        _ => return None,
    }
    Some((latitude, longitude))
}

fn scientific_decimal(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut index = 0;
    if matches!(bytes.first(), Some(b'+' | b'-')) {
        index = 1;
    }
    let mantissa_start = index;
    let mut dot_at = None;
    while index < bytes.len() && (bytes[index].is_ascii_digit() || bytes[index] == b'.') {
        if bytes[index] == b'.' {
            if dot_at.is_some() {
                return None;
            }
            dot_at = Some(index);
        }
        index += 1;
    }
    let dot_at = dot_at?;
    if dot_at == mantissa_start && index == dot_at + 1 {
        return None;
    }
    if index >= bytes.len() || !matches!(bytes[index], b'e' | b'E') {
        return None;
    }
    index += 1;
    if index < bytes.len() && matches!(bytes[index], b'+' | b'-') {
        index += 1;
    }
    let exponent_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    if index != bytes.len() || exponent_start == index {
        return None;
    }
    let value = input.parse::<f32>().ok()?;
    Some(format!("{value:.5}"))
}

fn normalize_timestamp(value: prost_types::Timestamp) -> prost_types::Timestamp {
    if value.seconds < MAX_SECONDS_IN_DURATION {
        return value;
    }
    let millis = value.seconds;
    prost_types::Timestamp {
        seconds: millis.div_euclid(1000),
        nanos: i32::try_from(millis.rem_euclid(1000))
            .unwrap_or(0)
            .saturating_mul(1_000_000),
    }
}
