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
use tesla_fleet_telemetry::proto::vehicle_data::{ChargingState, DistanceUnit, Field, ShiftState};
use tesla_fleet_telemetry::{
    ClientCa, DatumValue, Decoded, FieldKey, IncomingRecord, ServerConfig, TelemetryServer,
    TypedEnum, VehicleData,
};
use tokio::sync::watch;

use crate::config::Config;
use crate::error::AppError;
use crate::ingest::{Archive, apply_for_vin};
use crate::persist::{VehicleIdentity, resting_observation};
use crate::state::{Observation, Presence};
use crate::units::{miles_to_km, speed_to_kmh};

pub(crate) async fn run(
    config: &Config,
    archive: Arc<Archive>,
    mut shutdown: watch::Receiver<bool>,
) {
    if !config.telemetry_enabled {
        return;
    }
    let (Some(cert_path), Some(key_path)) = (
        config.telemetry_cert_path.as_ref(),
        config.telemetry_key_path.as_ref(),
    ) else {
        tracing::error!(
            "telemetry is enabled but TELEMETRY_CERT_PATH or TELEMETRY_KEY_PATH is missing"
        );
        return;
    };
    let certificate_pem = match std::fs::read_to_string(cert_path) {
        Ok(pem) => pem,
        Err(err) => {
            tracing::error!(error = %err, "could not read the telemetry certificate");
            return;
        }
    };
    let private_key_pem = match std::fs::read_to_string(key_path) {
        Ok(pem) => pem,
        Err(err) => {
            tracing::error!(error = %err, "could not read the telemetry private key");
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
            return;
        }
    };
    tracing::info!(addr = %server.local_addr(), "fleet telemetry listening");
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
                let mut guard = archive.runtimes.lock().await;
                if let Some(runtime) = guard.get_mut(&vin) {
                    runtime.last_telemetry = None;
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
        Field::PackVoltage => obs.charger_voltage = number(value),
        Field::InsideTemp => obs.inside_temp_c = number(value),
        Field::OutsideTemp => obs.outside_temp_c = number(value),
        Field::Locked => obs.locked = boolean(value),
        Field::SentryMode => obs.sentry_mode = boolean(value),
        Field::Version => obs.software_version = text(value),
        Field::CarType => obs.model = text(value),
        Field::Trim => obs.trim_badging = text(value),
        Field::ExteriorColor => obs.exterior_color = text(value),
        Field::FastChargerPresent => obs.fast_charger = boolean(value),
        _ => {}
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
}
