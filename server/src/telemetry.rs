// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Fleet Telemetry push ingestion.
//!
//! This is off unless `TELEMETRY_ENABLED` is set. Records are folded into the
//! last observation for the VIN and archived by [`crate::ingest::apply_for_vin`].
//! A fresh stream suppresses billable `vehicle_data` polls so the two sources
//! do not fight over the same drive. Polling remains the path that notices sleep.

use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use tesla_fleet_telemetry::proto::vehicle_connectivity::ConnectivityEvent;
use tesla_fleet_telemetry::proto::vehicle_data::{
    CableType, ChargePortLatchValue, ChargingState, DetailedChargeStateValue, DistanceUnit, Field,
    HvacPowerState, ScheduledChargingModeValue, ShiftState, WindowState,
};
use tesla_fleet_telemetry::{
    ClientCa, DatumValue, Decoded, DecodedDatum, FieldKey, IncomingRecord, ServerConfig,
    TelemetryServer, TypedEnum, VehicleData,
};
use tokio::sync::watch;

use crate::config::Config;
use crate::debug::DebugBoard;
use crate::error::AppError;
use crate::ingest::{Archive, apply_for_vin};
use crate::persist::{VehicleIdentity, resting_observation};
use crate::state::{Observation, Presence, refresh_plugged_in};
use crate::units::{miles_to_km, speed_to_kmh};

/// Signals the logger asks the car to stream when telemetry is configured.
///
/// Numeric fields carry a `minimum_delta` so a parked car does not publish on
/// every sample. `DetailedChargeState` and the charge-schedule fields from the
/// plugged-in status are included. `ChargeState` stays for older firmware.
/// `Odometer` and `Soc` are re-sent every five minutes: the car only publishes a
/// change, so a reconnect or a logger restart would otherwise leave them empty.
/// `DCChargingEnergyIn` is the battery-side session counter, the telemetry
/// equivalent of `charge_energy_added`, for AC and DC charging alike.
pub(crate) fn subscription(
    hostname: &str,
    port: u16,
    ca_pem: &str,
    vins: &[String],
) -> Result<tesla_fleet_api::FleetTelemetryConfigRequest, AppError> {
    use tesla_fleet_telemetry::FleetTelemetryConfig;
    let ca = crate::telemetry_config::ca_chain_pem(ca_pem);
    let config = FleetTelemetryConfig::builder(hostname, port, ca)
        .field_with("DetailedChargeState", pace(60, None, Some(900)))
        .field_with("ChargeState", pace(60, None, Some(900)))
        .field_with("Gear", pace(5, None, Some(60)))
        .field_with("Location", pace(3, Some(5.0), None))
        .field_with("Odometer", pace(60, Some(0.1), Some(300)))
        .field_with("Soc", pace(60, Some(1.0), Some(300)))
        .field_with("VehicleSpeed", pace(10, Some(1.0), None))
        .field_with("ChargeLimitSoc", pace(300, Some(1.0), None))
        .field_with("ScheduledChargingStartTime", pace(300, None, Some(3600)))
        .field_with("ScheduledChargingMode", pace(300, None, Some(3600)))
        .field_with("ScheduledDepartureTime", pace(300, None, Some(3600)))
        .field_with("ChargePortLatch", pace(60, None, Some(900)))
        .field_with("ChargingCableType", pace(60, None, Some(900)))
        .field_with("ChargePortDoorOpen", pace(60, None, Some(900)))
        .field_with("TimeToFullCharge", pace(300, Some(0.1), None))
        .field_with("ACChargingPower", pace(60, Some(0.5), None))
        .field_with("DCChargingPower", pace(60, Some(0.5), None))
        .field_with("DCChargingEnergyIn", pace(60, Some(0.1), Some(300)))
        .field_with("PackVoltage", pace(60, Some(2.0), None))
        .field_with("PackCurrent", pace(10, Some(5.0), None))
        .build()
        .map_err(|err| AppError::message(err.to_string()))?;
    let value = config
        .to_value()
        .map_err(|err| AppError::message(err.to_string()))?;
    let parsed: tesla_fleet_api::TelemetryConfig = serde_json::from_value(value)
        .map_err(|_| AppError::message("telemetry config could not be encoded"))?;
    Ok(tesla_fleet_api::FleetTelemetryConfigRequest {
        vins: vins.to_vec(),
        config: parsed,
    })
}

fn pace(
    interval_seconds: u32,
    minimum_delta: Option<f64>,
    resend: Option<u32>,
) -> tesla_fleet_telemetry::FieldConfig {
    tesla_fleet_telemetry::FieldConfig {
        interval_seconds,
        minimum_delta,
        resend_interval_seconds: resend,
        include_fields: Vec::new(),
    }
}

pub(crate) async fn run(
    config: &Config,
    archive: Arc<Archive>,
    debug: DebugBoard,
    mut shutdown: watch::Receiver<bool>,
) {
    if !config.telemetry_enabled {
        debug.set_telemetry("disabled", None);
        return;
    }
    let (Some(cert_path), Some(key_path)) = (
        config.telemetry_cert_path.as_ref(),
        config.telemetry_key_path.as_ref(),
    ) else {
        tracing::error!(
            "telemetry is enabled but TELEMETRY_CERT_PATH or TELEMETRY_KEY_PATH is missing"
        );
        debug.set_telemetry(
            "error",
            Some("telemetry certificate or key path is missing".to_owned()),
        );
        return;
    };
    let certificate_pem = match std::fs::read_to_string(cert_path) {
        Ok(pem) => pem,
        Err(err) => {
            tracing::error!(error = %err, "could not read the telemetry certificate");
            debug.set_telemetry(
                "error",
                Some("telemetry certificate could not be read".to_owned()),
            );
            return;
        }
    };
    let private_key_pem = match std::fs::read_to_string(key_path) {
        Ok(pem) => pem,
        Err(err) => {
            tracing::error!(error = %err, "could not read the telemetry private key");
            debug.set_telemetry(
                "error",
                Some("telemetry private key could not be read".to_owned()),
            );
            return;
        }
    };
    let mut server_config =
        ServerConfig::new(config.telemetry_bind, certificate_pem, private_key_pem);
    server_config.client_ca = ClientCa::TeslaProd;
    let mut server = match TelemetryServer::bind(server_config).await {
        Ok(server) => server,
        Err(err) => {
            tracing::error!(error = %err, "telemetry listener failed");
            debug.set_telemetry("error", Some("telemetry listener failed".to_owned()));
            if let Ok(mut fault) = archive.listener_fault.lock() {
                *fault = Some("écoute impossible".to_owned());
            }
            return;
        }
    };
    if let Ok(mut fault) = archive.listener_fault.lock() {
        *fault = None;
    }
    tracing::info!(addr = %server.local_addr(), "fleet telemetry listening");
    debug.set_telemetry("listening", None);
    let mut folded: HashMap<String, Folded> = HashMap::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            record = server.incoming().recv() => {
                let Some(record) = record else { break };
                let mut batch = vec![record];
                while let Ok(next) = server.incoming().try_recv() {
                    batch.push(next);
                }
                batch.sort_by_key(|record| record.created_at);
                for record in batch {
                    if let Err(err) = handle_record(&archive, &mut folded, record).await {
                        tracing::warn!(error = %err, "telemetry record was not archived");
                    }
                }
            }
        }
    }
}

struct Folded {
    obs: Observation,
    connection_id: String,
}

async fn handle_record(
    archive: &Archive,
    folded: &mut HashMap<String, Folded>,
    record: IncomingRecord,
) -> Result<(), AppError> {
    match record.decoded {
        Decoded::VehicleData(data) => {
            let vin = data.vin().to_owned();
            if vin.is_empty() {
                return Ok(());
            }
            let slot = folded.entry(vin.clone()).or_insert_with(|| Folded {
                obs: Observation::at(Utc::now(), Presence::Online),
                connection_id: record.connection_id.clone(),
            });
            if slot.connection_id.is_empty() {
                slot.connection_id.clone_from(&record.connection_id);
            }
            let fresh_location = carries_location(&data);
            fold_vehicle_data(&mut slot.obs, &data);
            let snapshot = slot.obs.clone();
            crate::ingest::apply_telemetry_for_vin(
                archive,
                &vin,
                VehicleIdentity {
                    vin: &vin,
                    tesla_id: None,
                    legacy_vehicle_id: None,
                    name: None,
                },
                snapshot,
                fresh_location,
            )
            .await
        }
        Decoded::Connectivity(event) => {
            let vin = event.vin().to_owned();
            if vin.is_empty() {
                return Ok(());
            }
            let presence = match event.status() {
                TypedEnum::Known(ConnectivityEvent::Disconnected) => Presence::Offline,
                TypedEnum::Known(ConnectivityEvent::Connected) => Presence::Online,
                _ => return Ok(()),
            };
            if presence == Presence::Online {
                let connection_id = event.connection_id().to_owned();
                match folded.get_mut(&vin) {
                    Some(slot) => slot.connection_id.clone_from(&connection_id),
                    None => {
                        folded.insert(
                            vin,
                            Folded {
                                obs: Observation::at(Utc::now(), Presence::Online),
                                connection_id,
                            },
                        );
                    }
                }
                return Ok(());
            }
            let active = folded.get(&vin).map(|slot| slot.connection_id.as_str());
            if !crate::telemetry_health::disconnect_is_current(active, event.connection_id()) {
                return Ok(());
            }
            folded.remove(&vin);
            let slot = archive.runtimes.lock().await.get(&vin).cloned();
            if let Some(slot) = slot {
                slot.lock().await.last_telemetry = None;
            }
            apply_for_vin(
                archive,
                &vin,
                VehicleIdentity {
                    vin: &vin,
                    tesla_id: None,
                    legacy_vehicle_id: None,
                    name: None,
                },
                resting_observation(Utc::now(), presence),
                false,
                false,
            )
            .await
        }
        Decoded::Alerts(alerts) => {
            let count = alerts.alerts().len();
            tracing::info!(count, "telemetry alerts");
            for alert in alerts.alerts() {
                tracing::info!(alert = %clip_label(&alert.name), "telemetry alert");
            }
            Ok(())
        }
        Decoded::Errors(errors) => {
            tracing::warn!(count = errors.errors().len(), "telemetry vehicle errors");
            for error in errors.errors() {
                tracing::warn!(name = %clip_label(&error.name), "telemetry vehicle error");
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn clip_label(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars().take(64) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.') {
            out.push(ch);
        }
    }
    if out.is_empty() {
        "unknown".to_owned()
    } else {
        out
    }
}

/// The payload itself carries a `Location`. The folded observation keeps the
/// last one, which is stale on a speed- or gear-only message.
fn carries_location(data: &VehicleData) -> bool {
    data.iter().any(|datum| {
        datum.field == FieldKey::Known(Field::Location)
            && matches!(datum.value, DatumValue::Location(_))
    })
}

pub(crate) fn fold_vehicle_data(obs: &mut Observation, data: &VehicleData) {
    obs.presence = Presence::Online;
    if let Some(time) = data.created_at().and_then(|stamp| stamp.to_system_time()) {
        obs.at = DateTime::<Utc>::from(time);
    }
    let mut metric = crate::units::units_are_metric(obs.distance_unit.as_deref());
    let datums: Vec<DecodedDatum> = data.iter().collect();
    for datum in &datums {
        if let DatumValue::DistanceUnit(TypedEnum::Known(unit)) = &datum.value {
            metric = matches!(unit, DistanceUnit::Kilometers);
            obs.distance_unit = Some(if metric { "km/hr" } else { "mi/hr" }.to_owned());
        }
    }
    fold_datums(obs, &datums, metric);
}

/// Applies one telemetry payload. `DetailedChargeState` replaces the deprecated
/// `ChargeState` when both are present, whichever order the car sent them.
fn fold_datums(obs: &mut Observation, data: &[DecodedDatum], metric: bool) {
    for datum in data {
        apply_field(obs, datum.field, &datum.value, metric);
    }
    if let Some(state) = data.iter().find_map(|datum| {
        (datum.field == FieldKey::Known(Field::DetailedChargeState))
            .then(|| charge_label(&datum.value))
            .flatten()
    }) {
        obs.charging_state = Some(state.to_owned());
    }
    // AC and DC power can arrive together; the charger power is the larger of
    // the two in this payload, not the largest value ever folded.
    let ac = payload_power(data, Field::AcChargingPower);
    let dc = payload_power(data, Field::DcChargingPower);
    let power = match (ac, dc) {
        (Some(ac), Some(dc)) => Some(ac.max(dc)),
        (ac, dc) => ac.or(dc),
    };
    // Power through the on-board charger is AC; DC power is a fast charger.
    if dc.is_some_and(|kw| kw > 0.0) {
        obs.fast_charger = Some(true);
    } else if ac.is_some_and(|kw| kw > 0.0) {
        obs.fast_charger = Some(false);
    }
    if let Some(power) = power {
        obs.charger_power_kw = Some(power);
    } else if !charging_label(obs.charging_state.as_deref()) {
        // Power is published on change only. Once the car reports it is no
        // longer charging, a held value would be stale.
        if obs.charger_power_kw.is_some() {
            obs.charger_power_kw = Some(0.0);
        }
    }
    if data
        .iter()
        .any(|datum| datum.field == FieldKey::Known(Field::PackCurrent))
    {
        obs.power_kw = pack_power_kw(obs.pack_voltage, payload_power(data, Field::PackCurrent));
    }
    if obs.charging_state.as_deref() == Some("Disconnected") {
        // The session counter belongs to the plug-in that just ended.
        obs.charge_energy_added_kwh = None;
    }
    refresh_plugged_in(obs);
}

/// Pack power in kW: positive while discharging, negative on regeneration or charge.
fn pack_power_kw(voltage: Option<f64>, current: Option<f64>) -> Option<f64> {
    let power = voltage? * current? / 1_000.0;
    power.is_finite().then_some(power)
}

/// Largest finite value of one power field in this payload.
fn payload_power(data: &[DecodedDatum], field: Field) -> Option<f64> {
    data.iter()
        .filter(|datum| datum.field == FieldKey::Known(field))
        .filter_map(|datum| number(&datum.value))
        .reduce(f64::max)
}

fn charging_label(state: Option<&str>) -> bool {
    matches!(state, None | Some("Charging" | "Starting"))
}

fn apply_field(obs: &mut Observation, field: FieldKey, value: &DatumValue, metric: bool) {
    let FieldKey::Known(field) = field else {
        return;
    };
    let units = if metric { Some("km/hr") } else { None };
    match field {
        Field::Soc | Field::BatteryLevel => {
            if let Some(level) = number(value) {
                obs.battery_level = Some(level);
            }
        }
        Field::RatedRange => obs.rated_range_km = number(value).and_then(miles_to_km),
        Field::IdealBatteryRange => obs.ideal_range_km = number(value).and_then(miles_to_km),
        Field::EstBatteryRange => obs.est_range_km = number(value).and_then(miles_to_km),
        Field::Odometer => obs.odometer_km = number(value).and_then(miles_to_km),
        Field::VehicleSpeed => {
            obs.speed_kmh = number(value).and_then(|speed| speed_to_kmh(speed, units));
        }
        Field::GpsHeading => obs.heading_deg = number(value),
        Field::Location => {
            if let DatumValue::Location(location) = value {
                obs.latitude = Some(location.latitude);
                obs.longitude = Some(location.longitude);
            }
        }
        Field::Gear => obs.shift_state = shift_label(value).map(str::to_owned),
        Field::ChargeState | Field::DetailedChargeState => {
            if let Some(label) = charge_label(value) {
                obs.charging_state = Some(label.to_owned());
            }
        }
        // AC energy is measured before the charger losses; only the battery-side
        // counter matches `charge_energy_added`.
        Field::DcChargingEnergyIn => {
            if let Some(energy) = nonneg(number(value)) {
                obs.charge_energy_added_kwh = Some(energy);
            }
        }
        Field::ChargeLimitSoc => obs.charge_limit_soc = number(value),
        Field::ChargeAmps => obs.charger_current = number(value),
        Field::ChargerVoltage => obs.charger_voltage = number(value),
        Field::PackVoltage => obs.pack_voltage = number(value),
        Field::InsideTemp => obs.inside_temp_c = number(value),
        Field::OutsideTemp => obs.outside_temp_c = number(value),
        Field::Locked => obs.locked = boolean(value),
        Field::SentryMode => obs.sentry_mode = boolean(value),
        Field::Version => obs.software_version = text(value),
        Field::CarType => obs.model = text(value),
        Field::Trim => obs.trim_badging = text(value),
        Field::ExteriorColor => obs.exterior_color = text(value),
        Field::FastChargerPresent => obs.fast_charger = boolean(value),
        Field::ChargeCurrentRequest => obs.charge_current_request = number(value),
        Field::ChargePortDoorOpen => obs.charge_port_door_open = boolean(value),
        Field::ChargePortLatch => {
            if let Some(label) = latch_label(value) {
                obs.charge_port_latch = Some(label);
            }
        }
        Field::ChargingCableType => {
            if let Some(label) = cable_label(value) {
                obs.conn_charge_cable = Some(label);
            }
        }
        Field::TimeToFullCharge => obs.time_to_full_charge_h = nonneg(number(value)),
        Field::ScheduledChargingStartTime => {
            if let Some(minutes) = scheduled_minutes(value) {
                obs.scheduled_charging_start_time = Some(minutes);
            }
        }
        Field::ScheduledDepartureTime => {
            if let Some(minutes) = scheduled_minutes(value) {
                obs.scheduled_departure_time = Some(minutes);
            }
        }
        Field::ScheduledChargingMode => {
            if let Some(mode) = charging_mode_label(value) {
                obs.scheduled_charging_mode = Some(mode);
            }
        }
        Field::TpmsPressureFl => obs.tpms_pressure_fl = nonneg(number(value)),
        Field::TpmsPressureFr => obs.tpms_pressure_fr = nonneg(number(value)),
        Field::TpmsPressureRl => obs.tpms_pressure_rl = nonneg(number(value)),
        Field::TpmsPressureRr => obs.tpms_pressure_rr = nonneg(number(value)),
        Field::DoorState => apply_doors(obs, value),
        Field::FdWindow => obs.fd_window = window_code(value).or(obs.fd_window),
        Field::FpWindow => obs.fp_window = window_code(value).or(obs.fp_window),
        Field::RdWindow => obs.rd_window = window_code(value).or(obs.rd_window),
        Field::RpWindow => obs.rp_window = window_code(value).or(obs.rp_window),
        Field::BatteryHeaterOn => obs.battery_heater_on = boolean(value),
        Field::PreconditioningEnabled => obs.is_preconditioning = boolean(value),
        Field::HvacPower => apply_hvac(obs, value),
        _ => {}
    }
}

fn nonneg(value: Option<f64>) -> Option<f64> {
    value.filter(|number| number.is_finite() && *number >= 0.0)
}

fn apply_doors(obs: &mut Observation, value: &DatumValue) {
    let DatumValue::Doors(doors) = value else {
        return;
    };
    obs.df = Some(i32::from(doors.driver_front));
    obs.dr = Some(i32::from(doors.driver_rear));
    obs.pf = Some(i32::from(doors.passenger_front));
    obs.pr = Some(i32::from(doors.passenger_rear));
    obs.ft = Some(i32::from(doors.trunk_front));
    obs.rt = Some(i32::from(doors.trunk_rear));
}

fn window_code(value: &DatumValue) -> Option<i32> {
    match value {
        DatumValue::WindowState(TypedEnum::Known(state)) => match state {
            WindowState::Closed => Some(0),
            WindowState::PartiallyOpen => Some(1),
            WindowState::Opened => Some(2),
            WindowState::Unknown => None,
        },
        DatumValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "closed" | "close" | "windowstateclosed" => Some(0),
            "partiallyopen" | "partial" | "windowstatepartiallyopen" => Some(1),
            "opened" | "open" | "windowstateopened" => Some(2),
            _ => None,
        },
        _ => None,
    }
}

fn latch_label(value: &DatumValue) -> Option<String> {
    match value {
        DatumValue::ChargePortLatch(TypedEnum::Known(state)) => match state {
            ChargePortLatchValue::ChargePortLatchEngaged => Some("Engaged".to_owned()),
            ChargePortLatchValue::ChargePortLatchDisengaged => Some("Disengaged".to_owned()),
            ChargePortLatchValue::ChargePortLatchBlocking => Some("Blocking".to_owned()),
            _ => None,
        },
        DatumValue::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "engaged" => Some("Engaged".to_owned()),
            "disengaged" => Some("Disengaged".to_owned()),
            "blocking" => Some("Blocking".to_owned()),
            _ => None,
        },
        _ => None,
    }
}

fn cable_label(value: &DatumValue) -> Option<String> {
    match value {
        DatumValue::CableType(TypedEnum::Known(kind)) => match kind {
            CableType::Iec => Some("IEC".to_owned()),
            CableType::Sae => Some("SAE".to_owned()),
            CableType::GbAc => Some("GB_AC".to_owned()),
            CableType::GbDc => Some("GB_DC".to_owned()),
            _ => None,
        },
        DatumValue::String(text) if !text.trim().is_empty() => Some(text.trim().to_owned()),
        _ => None,
    }
}

fn scheduled_minutes(value: &DatumValue) -> Option<i32> {
    if let DatumValue::Time(time) = value {
        if (0..24).contains(&time.hour) && (0..60).contains(&time.minute) {
            return Some(time.hour * 60 + time.minute);
        }
        return None;
    }
    crate::units::minutes_after_midnight(number(value)?)
}

fn apply_hvac(obs: &mut Observation, value: &DatumValue) {
    let DatumValue::HvacPower(TypedEnum::Known(state)) = value else {
        return;
    };
    match state {
        HvacPowerState::Off => obs.climate_on = Some(false),
        HvacPowerState::On | HvacPowerState::OverheatProtect => obs.climate_on = Some(true),
        HvacPowerState::Precondition => {
            obs.climate_on = Some(true);
            obs.is_preconditioning = Some(true);
        }
        HvacPowerState::Unknown => {}
    }
}

fn number(value: &DatumValue) -> Option<f64> {
    match value {
        DatumValue::Double(number) => finite(*number),
        DatumValue::Float(number) => finite(f64::from(*number)),
        DatumValue::Int(number) => Some(f64::from(*number)),
        DatumValue::Long(number) => i32::try_from(*number).ok().map(f64::from),
        _ => None,
    }
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn boolean(value: &DatumValue) -> Option<bool> {
    match value {
        DatumValue::Boolean(value) => Some(*value),
        _ => None,
    }
}

fn text(value: &DatumValue) -> Option<String> {
    match value {
        DatumValue::String(value) if !value.is_empty() => Some(value.clone()),
        _ => None,
    }
}

fn shift_label(value: &DatumValue) -> Option<&'static str> {
    let DatumValue::ShiftState(TypedEnum::Known(shift)) = value else {
        return None;
    };
    Some(match shift {
        ShiftState::D => "D",
        ShiftState::R => "R",
        ShiftState::N => "N",
        ShiftState::P => "P",
        _ => return None,
    })
}

fn charge_label(value: &DatumValue) -> Option<&'static str> {
    match value {
        DatumValue::ChargingState(TypedEnum::Known(state)) => match state {
            ChargingState::ChargeStateCharging => Some("Charging"),
            ChargingState::ChargeStateStarting => Some("Starting"),
            ChargingState::ChargeStateDisconnected => Some("Disconnected"),
            ChargingState::ChargeStateComplete => Some("Complete"),
            ChargingState::ChargeStateStopped => Some("Stopped"),
            ChargingState::ChargeStateNoPower => Some("NoPower"),
            ChargingState::ChargeStateUnknown => None,
        },
        DatumValue::DetailedChargeState(TypedEnum::Known(state)) => match state {
            DetailedChargeStateValue::DetailedChargeStateCharging => Some("Charging"),
            DetailedChargeStateValue::DetailedChargeStateStarting => Some("Starting"),
            DetailedChargeStateValue::DetailedChargeStateDisconnected => Some("Disconnected"),
            DetailedChargeStateValue::DetailedChargeStateComplete => Some("Complete"),
            DetailedChargeStateValue::DetailedChargeStateStopped => Some("Stopped"),
            DetailedChargeStateValue::DetailedChargeStateNoPower => Some("NoPower"),
            DetailedChargeStateValue::DetailedChargeStateUnknown => None,
        },
        _ => None,
    }
}

fn charging_mode_label(value: &DatumValue) -> Option<String> {
    let mode = match value {
        DatumValue::ScheduledChargingMode(TypedEnum::Known(mode)) => match mode {
            ScheduledChargingModeValue::ScheduledChargingModeOff => "Off",
            ScheduledChargingModeValue::ScheduledChargingModeStartAt => "StartAt",
            ScheduledChargingModeValue::ScheduledChargingModeDepartBy => "DepartBy",
            ScheduledChargingModeValue::ScheduledChargingModeUnknown => return None,
        },
        DatumValue::String(text) => {
            return crate::state::normalize_charging_mode(text).map(str::to_owned);
        }
        _ => return None,
    };
    Some(mode.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{fold_vehicle_data, subscription};

    #[test]
    fn telemetry_subscription_includes_charge_gear_and_location_deltas() {
        let request = subscription(
            "logger.example",
            4443,
            "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
            &["TEST00000000VIN01".to_owned()],
        )
        .unwrap();
        let fields = &request.config.fields;
        assert!(fields.contains_key("DetailedChargeState"));
        assert!(fields.contains_key("ChargeState"));
        assert!(fields.contains_key("Gear"));
        assert!(fields.contains_key("ScheduledChargingMode"));
        assert!(fields.contains_key("ScheduledDepartureTime"));
        // About one fix every 3 s while driving; nothing from a parked car.
        assert_eq!(fields["Location"].interval_seconds, 3);
        assert_eq!(fields["Location"].minimum_delta, Some(5.0));
        assert_eq!(fields["Soc"].minimum_delta, Some(1.0));
        // Re-sent so a reconnect or a logger restart gets them back without a change.
        assert_eq!(fields["Odometer"].resend_interval_seconds, Some(300));
        assert_eq!(fields["Soc"].resend_interval_seconds, Some(300));
        // A reconnect or restart learns the gear again well within the drive grace.
        assert!(
            fields["Gear"]
                .resend_interval_seconds
                .is_some_and(|secs| secs <= 60)
        );
        // Drive power is pack voltage times pack current.
        assert!(fields.contains_key("PackVoltage"));
        assert!(fields.contains_key("PackCurrent"));
        assert!(fields.values().all(|field| field.interval_seconds > 0));
        assert_eq!(request.config.hostname, "logger.example");
        assert!(request.config.ca.contains("MIIB"));
        let chained = subscription(
            "logger.example",
            4443,
            "-----BEGIN CERTIFICATE-----\nLEAFDATA\n-----END CERTIFICATE-----\n\
             -----BEGIN CERTIFICATE-----\nROOTDATA\n-----END CERTIFICATE-----\n",
            &["TEST00000000VIN01".to_owned()],
        )
        .unwrap();
        assert!(chained.config.ca.contains("ROOTDATA"));
        assert!(!chained.config.ca.contains("LEAFDATA"));
    }

    use crate::state::{Observation, Presence};
    use chrono::Utc;
    use tesla_fleet_telemetry::DecodeOptions;

    #[test]
    fn folds_a_reference_vehicle_payload() {
        let bytes =
            include_bytes!("../../crates/tesla-fleet-telemetry/tests/fixtures/vehicle_data.pb");
        let data = tesla_fleet_telemetry::decode_vehicle_data(
            bytes,
            &DecodeOptions {
                vin_override: Some("5YJ3E1EA7KF000001".to_owned()),
                device_client_version: None,
            },
        )
        .unwrap();
        let mut obs = Observation::at(Utc::now(), Presence::Offline);
        fold_vehicle_data(&mut obs, &data);
        assert_eq!(obs.presence, Presence::Online);
        assert_eq!(obs.battery_level, Some(81.5));
        let latitude = obs.latitude.unwrap();
        assert!((latitude - 37.412_374).abs() < 1e-6);
        assert!(obs.shift_state.is_some());
    }

    #[test]
    fn keeps_fields_that_a_later_sparse_payload_omits() {
        let bytes =
            include_bytes!("../../crates/tesla-fleet-telemetry/tests/fixtures/vehicle_data.pb");
        let data =
            tesla_fleet_telemetry::decode_vehicle_data(bytes, &DecodeOptions::default()).unwrap();
        let mut obs = Observation::at(Utc::now(), Presence::Online);
        fold_vehicle_data(&mut obs, &data);
        let shift = obs.shift_state.clone();
        obs.battery_level = Some(10.0);
        fold_vehicle_data(&mut obs, &data);
        assert_eq!(obs.shift_state, shift);
        assert_eq!(obs.battery_level, Some(81.5));
    }

    #[test]
    fn folds_charge_closures_and_tire_pressure() {
        use super::{apply_field, refresh_plugged_in};
        use tesla_fleet_telemetry::proto::vehicle_data::{
            ChargePortLatchValue, Doors, Field, HvacPowerState, WindowState,
        };
        use tesla_fleet_telemetry::{DatumValue, FieldKey, TypedEnum};

        let mut obs = Observation::at(Utc::now(), Presence::Online);
        let known = FieldKey::Known;
        apply_field(
            &mut obs,
            known(Field::TpmsPressureFl),
            &DatumValue::Double(2.9),
            true,
        );
        apply_field(
            &mut obs,
            known(Field::DoorState),
            &DatumValue::Doors(Doors {
                driver_front: true,
                trunk_rear: true,
                ..Doors::default()
            }),
            true,
        );
        apply_field(
            &mut obs,
            known(Field::FdWindow),
            &DatumValue::WindowState(TypedEnum::Known(WindowState::Closed)),
            true,
        );
        apply_field(
            &mut obs,
            known(Field::ChargerVoltage),
            &DatumValue::Double(240.0),
            true,
        );
        apply_field(
            &mut obs,
            known(Field::ChargePortLatch),
            &DatumValue::ChargePortLatch(TypedEnum::Known(
                ChargePortLatchValue::ChargePortLatchEngaged,
            )),
            true,
        );
        apply_field(
            &mut obs,
            known(Field::TimeToFullCharge),
            &DatumValue::Double(1.25),
            true,
        );
        apply_field(
            &mut obs,
            known(Field::BatteryHeaterOn),
            &DatumValue::Boolean(true),
            true,
        );
        apply_field(
            &mut obs,
            known(Field::HvacPower),
            &DatumValue::HvacPower(TypedEnum::Known(HvacPowerState::Precondition)),
            true,
        );
        apply_field(
            &mut obs,
            known(Field::ScheduledChargingStartTime),
            &DatumValue::Int(1320),
            true,
        );
        refresh_plugged_in(&mut obs);
        assert_eq!(obs.tpms_pressure_fl, Some(2.9));
        assert_eq!(obs.df, Some(1));
        assert_eq!(obs.dr, Some(0));
        assert_eq!(obs.rt, Some(1));
        assert_eq!(obs.fd_window, Some(0));
        assert_eq!(obs.charger_voltage, Some(240.0));
        assert_eq!(obs.charge_port_latch.as_deref(), Some("Engaged"));
        assert_eq!(obs.plugged_in, Some(true));
        assert_eq!(obs.time_to_full_charge_h, Some(1.25));
        assert_eq!(obs.battery_heater_on, Some(true));
        assert_eq!(obs.climate_on, Some(true));
        assert_eq!(obs.is_preconditioning, Some(true));
        assert_eq!(obs.scheduled_charging_start_time, Some(1320));
    }

    #[test]
    fn detailed_charge_state_marks_a_stopped_cable_as_plugged() {
        use super::{apply_field, refresh_plugged_in};
        use tesla_fleet_telemetry::proto::vehicle_data::{DetailedChargeStateValue, Field};
        use tesla_fleet_telemetry::{DatumValue, FieldKey, TypedEnum};

        let mut obs = Observation::at(Utc::now(), Presence::Online);
        apply_field(
            &mut obs,
            FieldKey::Known(Field::DetailedChargeState),
            &DatumValue::DetailedChargeState(TypedEnum::Known(
                DetailedChargeStateValue::DetailedChargeStateStopped,
            )),
            true,
        );
        obs.charge_port_latch = Some("Engaged".to_owned());
        obs.charge_port_door_open = Some(false);
        obs.charger_power_kw = Some(0.0);
        refresh_plugged_in(&mut obs);
        assert_eq!(obs.charging_state.as_deref(), Some("Stopped"));
        assert_eq!(obs.plugged_in, Some(true));
    }

    fn charge_payload(
        state: tesla_fleet_telemetry::proto::vehicle_data::DetailedChargeStateValue,
        values: &[(tesla_fleet_telemetry::proto::vehicle_data::Field, f64)],
    ) -> Vec<super::DecodedDatum> {
        use tesla_fleet_telemetry::proto::vehicle_data::Field;
        use tesla_fleet_telemetry::{DatumValue, FieldKey, TypedEnum};
        let mut data = vec![super::DecodedDatum {
            field: FieldKey::Known(Field::DetailedChargeState),
            value: DatumValue::DetailedChargeState(TypedEnum::Known(state)),
        }];
        data.extend(values.iter().map(|(field, value)| super::DecodedDatum {
            field: FieldKey::Known(*field),
            value: DatumValue::Double(*value),
        }));
        data
    }

    #[test]
    fn charger_power_follows_the_latest_payload_instead_of_the_highest_ever() {
        use super::fold_datums;
        use tesla_fleet_telemetry::proto::vehicle_data::{DetailedChargeStateValue, Field};

        let mut obs = Observation::at(Utc::now(), Presence::Online);
        fold_datums(
            &mut obs,
            &charge_payload(
                DetailedChargeStateValue::DetailedChargeStateCharging,
                &[(Field::AcChargingPower, 7.2), (Field::DcChargingPower, 0.0)],
            ),
            true,
        );
        assert_eq!(obs.charger_power_kw, Some(7.2));
        fold_datums(
            &mut obs,
            &charge_payload(
                DetailedChargeStateValue::DetailedChargeStateComplete,
                &[(Field::AcChargingPower, 0.0)],
            ),
            true,
        );
        assert_eq!(
            obs.charger_power_kw,
            Some(0.0),
            "power drops when charging stops"
        );
    }

    #[test]
    fn pack_voltage_and_current_give_the_drive_power() {
        use super::{DecodedDatum, fold_datums};
        use tesla_fleet_telemetry::proto::vehicle_data::Field;
        use tesla_fleet_telemetry::{DatumValue, FieldKey};

        let payload = |values: &[(Field, f64)]| -> Vec<DecodedDatum> {
            values
                .iter()
                .map(|(field, value)| DecodedDatum {
                    field: FieldKey::Known(*field),
                    value: DatumValue::Double(*value),
                })
                .collect()
        };
        let mut obs = Observation::at(Utc::now(), Presence::Online);
        fold_datums(
            &mut obs,
            &payload(&[(Field::PackVoltage, 400.0), (Field::PackCurrent, 50.0)]),
            true,
        );
        assert_eq!(obs.power_kw, Some(20.0), "discharging: positive kW");
        assert_eq!(
            obs.charger_voltage, None,
            "pack voltage is not the charger voltage"
        );
        // Current changes far more often than voltage: the held voltage is reused.
        fold_datums(&mut obs, &payload(&[(Field::PackCurrent, -25.0)]), true);
        assert_eq!(obs.power_kw, Some(-10.0), "regeneration: negative kW");
        // A payload without pack data keeps the last power.
        fold_datums(&mut obs, &payload(&[(Field::Soc, 60.0)]), true);
        assert_eq!(obs.power_kw, Some(-10.0));
    }

    #[test]
    fn charger_power_tells_ac_from_dc() {
        use super::fold_datums;
        use tesla_fleet_telemetry::proto::vehicle_data::{DetailedChargeStateValue, Field};

        let mut obs = Observation::at(Utc::now(), Presence::Online);
        fold_datums(
            &mut obs,
            &charge_payload(
                DetailedChargeStateValue::DetailedChargeStateCharging,
                &[(Field::AcChargingPower, 7.0)],
            ),
            true,
        );
        assert_eq!(obs.fast_charger, Some(false), "on-board charger");
        fold_datums(
            &mut obs,
            &charge_payload(
                DetailedChargeStateValue::DetailedChargeStateCharging,
                &[
                    (Field::AcChargingPower, 0.0),
                    (Field::DcChargingPower, 95.0),
                ],
            ),
            true,
        );
        assert_eq!(obs.fast_charger, Some(true), "fast charger");
        fold_datums(
            &mut obs,
            &charge_payload(
                DetailedChargeStateValue::DetailedChargeStateCharging,
                &[
                    (Field::AcChargingPower, 11.0),
                    (Field::DcChargingPower, 0.0),
                ],
            ),
            true,
        );
        assert_eq!(obs.fast_charger, Some(false), "back on AC");
    }

    #[test]
    fn charge_energy_comes_from_the_battery_side_counter() {
        use super::fold_datums;
        use tesla_fleet_telemetry::proto::vehicle_data::{DetailedChargeStateValue, Field};

        let mut obs = Observation::at(Utc::now(), Presence::Online);
        fold_datums(
            &mut obs,
            &charge_payload(
                DetailedChargeStateValue::DetailedChargeStateCharging,
                &[
                    (Field::AcChargingEnergyIn, 17.4),
                    (Field::DcChargingEnergyIn, 16.1),
                ],
            ),
            true,
        );
        // DCChargingEnergyIn is the vehicle_data charge_energy_added equivalent.
        assert_eq!(obs.charge_energy_added_kwh, Some(16.1));
        fold_datums(
            &mut obs,
            &charge_payload(
                DetailedChargeStateValue::DetailedChargeStateDisconnected,
                &[],
            ),
            true,
        );
        assert_eq!(
            obs.charge_energy_added_kwh, None,
            "an unplugged cable ends the session counter"
        );
        fold_datums(
            &mut obs,
            &charge_payload(
                DetailedChargeStateValue::DetailedChargeStateCharging,
                &[(Field::DcChargingEnergyIn, 0.4)],
            ),
            true,
        );
        assert_eq!(obs.charge_energy_added_kwh, Some(0.4));
    }

    #[test]
    fn subscription_streams_the_charge_energy_counter() {
        let request = subscription(
            "logger.example",
            4443,
            "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----\n",
            &["TEST00000000VIN01".to_owned()],
        )
        .unwrap();
        let fields = &request.config.fields;
        let energy = &fields["DCChargingEnergyIn"];
        assert_eq!(energy.minimum_delta, Some(0.1));
        assert_eq!(energy.resend_interval_seconds, Some(300));
    }

    #[test]
    fn detailed_charge_state_wins_when_the_deprecated_field_says_disconnected() {
        use super::{DecodedDatum, fold_datums};
        use tesla_fleet_telemetry::proto::vehicle_data::{
            ChargingState, DetailedChargeStateValue, Field,
        };
        use tesla_fleet_telemetry::{DatumValue, FieldKey, TypedEnum};

        let known = FieldKey::Known;
        let datums = [
            DecodedDatum {
                field: known(Field::DetailedChargeState),
                value: DatumValue::DetailedChargeState(TypedEnum::Known(
                    DetailedChargeStateValue::DetailedChargeStateStopped,
                )),
            },
            DecodedDatum {
                field: known(Field::ChargeState),
                value: DatumValue::ChargingState(TypedEnum::Known(
                    ChargingState::ChargeStateDisconnected,
                )),
            },
        ];
        let mut obs = Observation::at(Utc::now(), Presence::Online);
        fold_datums(&mut obs, &datums, true);
        assert_eq!(obs.charging_state.as_deref(), Some("Stopped"));
        assert_eq!(obs.plugged_in, Some(true));
    }

    #[test]
    fn folds_a_departure_schedule_in_local_minutes() {
        use super::apply_field;
        use tesla_fleet_telemetry::proto::vehicle_data::{Field, ScheduledChargingModeValue, Time};
        use tesla_fleet_telemetry::{DatumValue, FieldKey, TypedEnum};

        let mut obs = Observation::at(Utc::now(), Presence::Online);
        let known = FieldKey::Known;
        apply_field(
            &mut obs,
            known(Field::ScheduledChargingMode),
            &DatumValue::ScheduledChargingMode(TypedEnum::Known(
                ScheduledChargingModeValue::ScheduledChargingModeDepartBy,
            )),
            true,
        );
        apply_field(
            &mut obs,
            known(Field::ScheduledDepartureTime),
            &DatumValue::Time(Time {
                hour: 7,
                minute: 30,
                second: 0,
            }),
            true,
        );
        assert_eq!(obs.scheduled_charging_mode.as_deref(), Some("DepartBy"));
        assert_eq!(obs.scheduled_departure_time, Some(7 * 60 + 30));
    }
}

/// Replays Fleet Telemetry records through [`handle_record`] the way the receiver
/// loop does: sparse payloads that only carry fields whose value changed.
#[cfg(test)]
mod replay_tests {
    use std::collections::HashMap;

    use chrono::{DateTime, Duration, Utc};
    use prost::Message;
    use tesla_fleet_telemetry::proto::vehicle_connectivity::{
        ConnectivityEvent, VehicleConnectivity,
    };
    use tesla_fleet_telemetry::proto::vehicle_data::{
        Datum, Field, LocationValue, Payload, ShiftState, Value, value::Value as Raw,
    };
    use tesla_fleet_telemetry::{DecodeOptions, Decoded, IncomingRecord};

    use super::{Folded, handle_record};
    use crate::ingest::Archive;
    use crate::persist::VehicleIdentity;
    use crate::test_util::TestDb;

    const VIN: &str = "TEST00000000VIN02";
    const SOCKET: &str = "test-socket-1";

    enum Signal {
        Gear(ShiftState),
        Speed(f64),
        Location(f64, f64),
        OdometerMiles(f64),
    }

    fn archive(pool: sqlx::PgPool) -> Archive {
        let (events, _) = tokio::sync::broadcast::channel(64);
        Archive::new(
            pool,
            crate::geocode::Geocoder::disabled(),
            events,
            crate::drive_gps::GpsBoard::default(),
        )
    }

    fn identity() -> VehicleIdentity<'static> {
        VehicleIdentity {
            vin: VIN,
            tesla_id: Some(31),
            legacy_vehicle_id: Some(41),
            name: Some("Replay"),
        }
    }

    fn stamp(at: DateTime<Utc>) -> prost_types::Timestamp {
        prost_types::Timestamp {
            seconds: at.timestamp(),
            nanos: i32::try_from(at.timestamp_subsec_nanos()).unwrap(),
        }
    }

    fn options() -> DecodeOptions {
        DecodeOptions {
            vin_override: Some(VIN.to_owned()),
            device_client_version: None,
        }
    }

    fn record(
        at: DateTime<Utc>,
        topic: &str,
        payload: Vec<u8>,
        decoded: Decoded,
    ) -> IncomingRecord {
        IncomingRecord {
            topic: topic.to_owned(),
            txid: Vec::new(),
            message_id: Vec::new(),
            sender_id: String::new(),
            device_type: String::new(),
            device_id: VIN.to_owned(),
            cert_device_type: String::new(),
            cert_device_id: VIN.to_owned(),
            created_at: u32::try_from(at.timestamp()).unwrap(),
            delivered_at_epoch_ms: 0,
            received_at_epoch_ms: 0,
            device_client_version: String::new(),
            network_interface: String::new(),
            connection_id: SOCKET.to_owned(),
            payload,
            decoded,
        }
    }

    fn vehicle_data(at: DateTime<Utc>, signals: &[Signal]) -> IncomingRecord {
        let data = signals
            .iter()
            .map(|signal| {
                let (key, value) = match signal {
                    Signal::Gear(shift) => (Field::Gear, Raw::ShiftStateValue(*shift as i32)),
                    Signal::Speed(kmh) => (Field::VehicleSpeed, Raw::DoubleValue(*kmh)),
                    Signal::Location(latitude, longitude) => (
                        Field::Location,
                        Raw::LocationValue(LocationValue {
                            latitude: *latitude,
                            longitude: *longitude,
                        }),
                    ),
                    Signal::OdometerMiles(miles) => (Field::Odometer, Raw::DoubleValue(*miles)),
                };
                Datum {
                    key: key as i32,
                    value: Some(Value { value: Some(value) }),
                }
            })
            .collect();
        let bytes = Payload {
            data,
            created_at: Some(stamp(at)),
            vin: VIN.to_owned(),
            is_resend: false,
        }
        .encode_to_vec();
        let decoded = tesla_fleet_telemetry::decode_vehicle_data(&bytes, &options()).unwrap();
        record(at, "V", bytes, Decoded::VehicleData(decoded))
    }

    fn connected(at: DateTime<Utc>) -> IncomingRecord {
        let bytes = VehicleConnectivity {
            vin: VIN.to_owned(),
            connection_id: SOCKET.to_owned(),
            status: ConnectivityEvent::Connected as i32,
            created_at: Some(stamp(at)),
            network_interface: String::new(),
        }
        .encode_to_vec();
        let decoded = tesla_fleet_telemetry::decode_connectivity(&bytes, &options()).unwrap();
        record(at, "connectivity", bytes, Decoded::Connectivity(decoded))
    }

    struct Replay {
        archive: Archive,
        folded: HashMap<String, Folded>,
    }

    impl Replay {
        fn new(pool: sqlx::PgPool) -> Self {
            Self {
                archive: archive(pool),
                folded: HashMap::new(),
            }
        }

        async fn send(&mut self, record: IncomingRecord) {
            handle_record(&self.archive, &mut self.folded, record)
                .await
                .unwrap();
        }

        /// A Fleet API `vehicle_data` poll while the car is awake.
        async fn poll(&self, at: DateTime<Utc>, shift: Option<&str>, kmh: f64, km: f64, lat: f64) {
            let mut obs = crate::state::Observation::at(at, crate::state::Presence::Online);
            obs.shift_state = shift.map(str::to_owned);
            obs.speed_kmh = Some(kmh);
            obs.power_kw = Some(5.0);
            obs.odometer_km = Some(km);
            obs.battery_level = Some(80.0);
            obs.latitude = Some(lat);
            obs.longitude = Some(20.0);
            crate::ingest::apply_for_vin(&self.archive, VIN, identity(), obs, true, false)
                .await
                .unwrap();
        }

        /// Telemetry-only drive: D, ten minutes at about 40 km/h, 0 km/h, then P.
        /// Returns the instant of the P message.
        async fn telemetry_drive(&mut self, start: DateTime<Utc>, odometer: bool) -> DateTime<Utc> {
            let at = |secs: i64| start + Duration::seconds(secs);
            self.send(connected(at(0))).await;
            self.send(vehicle_data(
                at(0),
                &[
                    Signal::Gear(ShiftState::D),
                    Signal::Speed(8.0),
                    Signal::Location(10.0, 20.0),
                ],
            ))
            .await;
            let mut latitude = 10.0;
            let mut miles = 600.0;
            for step in 1..=60_i64 {
                let mut signals = vec![Signal::Speed(if step % 2 == 0 { 40.0 } else { 45.0 })];
                if step % 3 == 0 {
                    latitude += 0.003;
                    miles += 0.12;
                    signals.push(Signal::Location(latitude, 20.0));
                    if odometer {
                        signals.push(Signal::OdometerMiles(miles));
                    }
                }
                self.send(vehicle_data(at(step * 10), &signals)).await;
            }
            self.send(vehicle_data(at(605), &[Signal::Speed(0.0)]))
                .await;
            self.send(vehicle_data(at(608), &[Signal::Gear(ShiftState::P)]))
                .await;
            at(608)
        }
    }

    async fn drive_counts(pool: &sqlx::PgPool) -> (i64, i64) {
        sqlx::query_as(
            "SELECT COUNT(*) FILTER (WHERE end_at IS NULL), COUNT(*) FILTER (WHERE end_at IS NOT NULL)
             FROM drives",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn telemetry_opener_without_odometer_keeps_the_polled_drive() {
        let Some(db) = TestDb::migrated("tdrop").await else {
            return;
        };
        let mut replay = Replay::new(db.pool.clone());
        let start = Utc::now();
        let at = |secs: i64| start + Duration::seconds(secs);
        // A logger restart empties the fold. The car only re-sends what changed:
        // the gear, speed and location, not the odometer.
        replay.send(connected(at(0))).await;
        replay
            .send(vehicle_data(
                at(0),
                &[
                    Signal::Gear(ShiftState::D),
                    Signal::Speed(8.0),
                    Signal::Location(10.0, 20.0),
                ],
            ))
            .await;
        let mut km = 1000.0;
        let mut latitude = 10.0;
        for secs in (30..=600).step_by(30) {
            km += 0.33;
            latitude += 0.003;
            replay.poll(at(secs), Some("D"), 40.0, km, latitude).await;
        }
        for secs in (630..=720).step_by(30) {
            replay.poll(at(secs), Some("P"), 0.0, km, latitude).await;
        }
        assert_eq!(
            drive_counts(&db.pool).await,
            (0, 1),
            "the real drive is kept"
        );
        let distance: (Option<f64>,) = sqlx::query_as("SELECT distance_km FROM drives")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert!(distance.0.is_some_and(|km| km > 6.0), "{distance:?}");
    }

    /// Telemetry only, the car driving north: speed every 10 s, `Location`
    /// every 30 s. A speed-only message must not store the 30 s old
    /// `Location` at its own time.
    #[tokio::test]
    async fn telemetry_without_location_stores_no_stale_coordinates() {
        let Some(db) = TestDb::migrated("tstale").await else {
            return;
        };
        let mut replay = Replay::new(db.pool.clone());
        let start = Utc::now();
        let at = |secs: i64| start + Duration::seconds(secs);
        let latitude = |secs: i32| 10.0 + 0.0001 * f64::from(secs);
        replay.send(connected(at(0))).await;
        for secs in 0..=120_i32 {
            if secs % 10 == 0 {
                let mut signals = vec![Signal::Gear(ShiftState::D), Signal::Speed(25.0)];
                if secs % 30 == 0 {
                    signals.push(Signal::Location(latitude(secs), 20.0));
                }
                let half = at(secs.into()) + Duration::milliseconds(500);
                replay.send(vehicle_data(half, &signals)).await;
            }
        }
        let track: Vec<(Option<f64>, String)> = sqlx::query_as(
            "SELECT latitude, source FROM positions
             WHERE drive_id IS NOT NULL ORDER BY recorded_at, id",
        )
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(
            drive_counts(&db.pool).await,
            (1, 0),
            "telemetry opens the drive"
        );
        let located: Vec<f64> = track
            .iter()
            .filter(|(_, source)| source == "telemetry")
            .filter_map(|(lat, _)| *lat)
            .collect();
        let expected: Vec<f64> = (0..=4).map(|step| latitude(step * 30)).collect();
        assert_eq!(
            located, expected,
            "one point per Location message, none in between"
        );
        let mut previous = f64::MIN;
        for (lat, source) in &track {
            let Some(lat) = lat else { continue };
            assert!(*lat >= previous, "{source} point jumps back to {lat}");
            previous = *lat;
        }
        let located: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM positions WHERE source = 'telemetry' AND latitude IS NOT NULL",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert!(located.0 <= 5, "only Location messages carry coordinates");
    }

    #[tokio::test]
    async fn telemetry_only_confirmed_park_closes_after_the_p_window() {
        let Some(db) = TestDb::migrated("tpark").await else {
            return;
        };
        let mut replay = Replay::new(db.pool.clone());
        let park = replay.telemetry_drive(Utc::now(), true).await;
        crate::ingest::expire_drives(&replay.archive, park + Duration::seconds(75))
            .await
            .unwrap();
        assert_eq!(drive_counts(&db.pool).await, (0, 1), "closed 60 s after P");
        let end: (DateTime<Utc>,) = sqlx::query_as("SELECT end_at FROM drives")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert!(
            (end.0 - park).num_milliseconds().abs() <= 1,
            "end is the P message"
        );
    }

    #[tokio::test]
    async fn telemetry_without_odometer_reuses_the_last_known_odometer() {
        let Some(db) = TestDb::migrated("todo").await else {
            return;
        };
        let mut replay = Replay::new(db.pool.clone());
        let start = Utc::now();
        let mut polled = crate::state::Observation::at(start, crate::state::Presence::Online);
        polled.shift_state = Some("P".to_owned());
        polled.speed_kmh = Some(0.0);
        polled.odometer_km = Some(1000.0);
        polled.latitude = Some(10.0);
        polled.longitude = Some(20.0);
        crate::ingest::apply_for_vin(&replay.archive, VIN, identity(), polled, true, false)
            .await
            .unwrap();
        replay.send(connected(start)).await;
        replay
            .send(vehicle_data(
                start + Duration::seconds(5),
                &[
                    Signal::Gear(ShiftState::D),
                    Signal::Speed(8.0),
                    Signal::Location(10.001, 20.0),
                ],
            ))
            .await;
        let opener: (Option<f64>,) = sqlx::query_as(
            "SELECT odometer_km FROM positions WHERE source = 'telemetry' ORDER BY recorded_at LIMIT 1",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(opener.0, Some(1000.0));
    }

    #[tokio::test]
    async fn a_moving_drive_without_any_odometer_is_kept() {
        let Some(db) = TestDb::migrated("tnoodo").await else {
            return;
        };
        let mut replay = Replay::new(db.pool.clone());
        let park = replay.telemetry_drive(Utc::now(), false).await;
        crate::ingest::expire_drives(&replay.archive, park + Duration::seconds(75))
            .await
            .unwrap();
        assert_eq!(
            drive_counts(&db.pool).await,
            (0, 1),
            "40 km/h for ten minutes is a drive"
        );
    }

    #[tokio::test]
    async fn stale_telemetry_cannot_open_a_drive_but_keeps_its_positions() {
        let Some(db) = TestDb::migrated("tstale").await else {
            return;
        };
        let mut replay = Replay::new(db.pool.clone());
        // Every message is processed more than 30 s after the car stamped it.
        replay
            .telemetry_drive(Utc::now() - Duration::seconds(608 + 40), true)
            .await;
        assert_eq!(drive_counts(&db.pool).await, (0, 0));
        let kept: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM positions WHERE source = 'telemetry'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(kept.0 > 0, "late samples are stored, not dropped");
    }
}
