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
        .field_with("Gear", pace(5, None, None))
        .field_with("Location", pace(30, Some(25.0), None))
        .field_with("Odometer", pace(60, Some(0.1), None))
        .field_with("Soc", pace(60, Some(1.0), None))
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
            fold_vehicle_data(&mut slot.obs, &data);
            let snapshot = slot.obs.clone();
            apply_for_vin(
                archive,
                &vin,
                VehicleIdentity {
                    vin: &vin,
                    tesla_id: None,
                    legacy_vehicle_id: None,
                    name: None,
                },
                snapshot,
                true,
                true,
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
    refresh_plugged_in(obs);
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
        Field::AcChargingPower | Field::DcChargingPower => {
            if let Some(power) = number(value) {
                obs.charger_power_kw = Some(
                    obs.charger_power_kw
                        .map_or(power, |current| current.max(power)),
                );
                if matches!(field, Field::DcChargingPower) && power > 0.0 {
                    obs.fast_charger = Some(true);
                }
            }
        }
        Field::AcChargingEnergyIn | Field::DcChargingEnergyIn => {
            if let Some(energy) = number(value) {
                obs.charge_energy_added_kwh = Some(
                    obs.charge_energy_added_kwh
                        .map_or(energy, |current| current.max(energy)),
                );
            }
        }
        Field::ChargeLimitSoc => obs.charge_limit_soc = number(value),
        Field::ChargeAmps => obs.charger_current = number(value),
        Field::PackVoltage | Field::ChargerVoltage => obs.charger_voltage = number(value),
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
        assert_eq!(fields["Location"].minimum_delta, Some(25.0));
        assert_eq!(fields["Soc"].minimum_delta, Some(1.0));
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
