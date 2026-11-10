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
    CableType, ChargePortLatchValue, ChargingState, DistanceUnit, Field, HvacPowerState,
    ShiftState, WindowState,
};
use tesla_fleet_telemetry::{
    ClientCa, DatumValue, Decoded, FieldKey, IncomingRecord, ServerConfig, TelemetryServer,
    TypedEnum, VehicleData,
};
use tokio::sync::watch;

use crate::config::Config;
use crate::debug::DebugBoard;
use crate::error::AppError;
use crate::ingest::{Archive, apply_for_vin};
use crate::persist::{VehicleIdentity, resting_observation};
use crate::state::{Observation, Presence, refresh_plugged_in};
use crate::units::{miles_to_km, speed_to_kmh};

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
            return;
        }
    };
    tracing::info!(addr = %server.local_addr(), "fleet telemetry listening");
    debug.set_telemetry("listening", None);
    let mut folded: HashMap<String, Observation> = HashMap::new();
    loop {
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            record = server.incoming().recv() => {
                let Some(record) = record else { break };
                if let Err(err) = handle_record(&archive, &mut folded, record).await {
                    tracing::warn!(error = %err, "telemetry record was not archived");
                }
            }
        }
    }
}

async fn handle_record(
    archive: &Archive,
    folded: &mut HashMap<String, Observation>,
    record: IncomingRecord,
) -> Result<(), AppError> {
    match record.decoded {
        Decoded::VehicleData(data) => {
            let vin = data.vin().to_owned();
            if vin.is_empty() {
                return Ok(());
            }
            let obs = folded
                .entry(vin.clone())
                .or_insert_with(|| Observation::at(Utc::now(), Presence::Online));
            fold_vehicle_data(obs, &data);
            let snapshot = obs.clone();
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
            if presence == Presence::Offline {
                folded.remove(&vin);
                let slot = archive.runtimes.lock().await.get(&vin).cloned();
                if let Some(slot) = slot {
                    slot.lock().await.last_telemetry = None;
                }
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
        _ => Ok(()),
    }
}

pub(crate) fn fold_vehicle_data(obs: &mut Observation, data: &VehicleData) {
    obs.presence = Presence::Online;
    if let Some(time) = data.created_at().and_then(|stamp| stamp.to_system_time()) {
        obs.at = DateTime::<Utc>::from(time);
    }
    let mut metric = crate::units::units_are_metric(obs.distance_unit.as_deref());
    for datum in data.iter() {
        if let DatumValue::DistanceUnit(TypedEnum::Known(unit)) = &datum.value {
            metric = matches!(unit, DistanceUnit::Kilometers);
            obs.distance_unit = Some(if metric { "km/hr" } else { "mi/hr" }.to_owned());
        }
    }
    for datum in data.iter() {
        apply_field(obs, datum.field, &datum.value, metric);
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
        Field::ChargeState => obs.charging_state = charge_label(value).map(str::to_owned),
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
    let DatumValue::ChargingState(TypedEnum::Known(state)) = value else {
        return None;
    };
    Some(match state {
        ChargingState::ChargeStateCharging => "Charging",
        ChargingState::ChargeStateStarting => "Starting",
        ChargingState::ChargeStateDisconnected => "Disconnected",
        ChargingState::ChargeStateComplete => "Complete",
        ChargingState::ChargeStateStopped => "Stopped",
        ChargingState::ChargeStateNoPower => "NoPower",
        ChargingState::ChargeStateUnknown => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::fold_vehicle_data;
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
}
