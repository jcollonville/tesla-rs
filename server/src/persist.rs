// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use tokio::sync::broadcast;

use crate::error::{AppError, redact_secrets};
use crate::geocode::{Geocoder, cache_key};
use crate::geofence::{Fence, matching_geofence};
use crate::state::{
    Activity, DriveSummary, Observation, Presence, Sample, Tracker, Transition, summarize_charge,
    summarize_drive,
};

/// Identity fields learned from a vehicle list response.
pub(crate) struct VehicleIdentity<'a> {
    pub vin: &'a str,
    pub tesla_id: Option<i64>,
    pub legacy_vehicle_id: Option<i64>,
    pub name: Option<&'a str>,
}

/// In-memory poll bookkeeping for one vehicle.
pub(crate) struct Runtime {
    pub vehicle_id: i64,
    pub vin: String,
    pub tracker: Tracker,
    pub open_drive_id: Option<i64>,
    pub open_charge_id: Option<i64>,
    pub parked_live_polls: u32,
    pub last_telemetry: Option<tokio::time::Instant>,
    pub efficiency_kwh_per_km: Option<f64>,
    pub distance_unit: Option<String>,
    pub name: Option<String>,
    pub model: Option<String>,
    pub next_due: tokio::time::Instant,
}

/// Push payload for the read API websocket and the MQTT publisher.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct LiveEvent {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub vehicle_id: i64,
    pub vin: String,
    pub name: Option<String>,
    pub model: Option<String>,
    pub activity: &'static str,
    pub battery_level: Option<f64>,
    pub rated_range_km: Option<f64>,
    pub odometer_km: Option<f64>,
    pub speed_kmh: Option<f64>,
    pub power_kw: Option<f64>,
    pub charger_power_kw: Option<f64>,
    pub inside_temp_c: Option<f64>,
    pub outside_temp_c: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub at: DateTime<Utc>,
}

pub(crate) async fn upsert_vehicle(
    pool: &PgPool,
    identity: VehicleIdentity<'_>,
) -> Result<i64, AppError> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO vehicles (vin, tesla_id, legacy_vehicle_id, name, source, external_id)
         VALUES ($1, $2, $3, $4, 'fleet', $1)
         ON CONFLICT (vin) DO UPDATE SET
            tesla_id = COALESCE(EXCLUDED.tesla_id, vehicles.tesla_id),
            legacy_vehicle_id = COALESCE(EXCLUDED.legacy_vehicle_id, vehicles.legacy_vehicle_id),
            name = COALESCE(EXCLUDED.name, vehicles.name),
            updated_at = now()
         RETURNING id",
    )
    .bind(identity.vin)
    .bind(identity.tesla_id)
    .bind(identity.legacy_vehicle_id)
    .bind(identity.name)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

pub(crate) async fn load_runtime(
    pool: &PgPool,
    vehicle_id: i64,
    vin: &str,
) -> Result<Runtime, AppError> {
    let row = sqlx::query_as::<_, RuntimeRow>(
        "SELECT v.efficiency_kwh_per_km, v.distance_unit, v.name, v.model, s.activity,
                d.id AS drive_id, c.id AS charge_id
         FROM vehicles v
         LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
         LEFT JOIN drives d ON d.vehicle_id = v.id AND d.end_at IS NULL AND d.source = 'poll'
         LEFT JOIN charges c ON c.vehicle_id = v.id AND c.end_at IS NULL AND c.source = 'poll'
         WHERE v.id = $1",
    )
    .bind(vehicle_id)
    .fetch_one(pool)
    .await?;
    let drive_open = row.drive_id.is_some();
    let charge_open = row.charge_id.is_some();
    let mut activity = row
        .activity
        .as_deref()
        .map_or(Activity::Offline, Activity::parse);
    if drive_open {
        activity = Activity::Driving;
    } else if charge_open {
        activity = Activity::Charging;
    } else if matches!(activity, Activity::Driving | Activity::Charging) {
        activity = Activity::Online;
    }
    Ok(Runtime {
        vehicle_id,
        vin: vin.to_owned(),
        tracker: Tracker {
            activity,
            drive_open,
            charge_open,
        },
        open_drive_id: row.drive_id,
        open_charge_id: row.charge_id,
        parked_live_polls: 0,
        last_telemetry: None,
        efficiency_kwh_per_km: row.efficiency_kwh_per_km,
        distance_unit: row.distance_unit,
        name: row.name,
        model: row.model,
        next_due: tokio::time::Instant::now(),
    })
}

#[derive(sqlx::FromRow)]
struct RuntimeRow {
    efficiency_kwh_per_km: Option<f64>,
    distance_unit: Option<String>,
    name: Option<String>,
    model: Option<String>,
    activity: Option<String>,
    drive_id: Option<i64>,
    charge_id: Option<i64>,
}

pub(crate) async fn record_error(
    pool: &PgPool,
    vehicle_id: i64,
    message: &str,
) -> Result<(), AppError> {
    let message = clip_error(message);
    sqlx::query(
        "INSERT INTO vehicle_state (vehicle_id, activity, last_poll_at, last_error, updated_at)
         VALUES ($1, 'offline', now(), $2, now())
         ON CONFLICT (vehicle_id) DO UPDATE SET
            last_poll_at = now(),
            last_error = EXCLUDED.last_error,
            updated_at = now()",
    )
    .bind(vehicle_id)
    .bind(message)
    .execute(pool)
    .await?;
    Ok(())
}

/// Applies one observation: session boundaries, positions, and current state.
pub(crate) async fn apply_observation(
    pool: &PgPool,
    runtime: &mut Runtime,
    obs: &Observation,
    geocoder: &Geocoder,
    events: &broadcast::Sender<LiveEvent>,
    live: bool,
) -> Result<(), AppError> {
    let previous = runtime.tracker.clone();
    let (next, transitions) = crate::state::reduce(&previous, obs);
    let starting_drive = transitions.contains(&Transition::DriveStarted);
    let ending_drive = transitions.contains(&Transition::DriveEnded);
    let starting_charge = transitions.contains(&Transition::ChargeStarted);
    let ending_charge = transitions.contains(&Transition::ChargeEnded);

    let mut tx = pool.begin().await?;
    let fences = load_fences(&mut tx).await?;
    let fence_id = obs
        .latitude
        .zip(obs.longitude)
        .and_then(|(latitude, longitude)| matching_geofence(latitude, longitude, &fences));

    if starting_drive {
        runtime.open_drive_id =
            Some(insert_drive(&mut tx, runtime.vehicle_id, obs, fence_id).await?);
    }
    if starting_charge {
        runtime.open_charge_id =
            Some(insert_charge(&mut tx, runtime.vehicle_id, obs, fence_id).await?);
    }

    let attach_drive = if previous.drive_open || starting_drive {
        runtime.open_drive_id
    } else {
        None
    };
    let attach_charge = if previous.charge_open || starting_charge {
        runtime.open_charge_id
    } else {
        None
    };
    if live && obs.has_sample() {
        insert_position(
            &mut tx,
            runtime.vehicle_id,
            obs,
            attach_drive,
            attach_charge,
        )
        .await?;
    }
    if (previous.charge_open || starting_charge)
        && live
        && obs.charging_state.is_some()
        && let Some(charge_id) = runtime.open_charge_id
    {
        insert_charge_sample(&mut tx, charge_id, obs).await?;
        if obs.fast_charger == Some(true) {
            sqlx::query("UPDATE charges SET is_dc = TRUE WHERE id = $1")
                .bind(charge_id)
                .execute(&mut *tx)
                .await?;
        }
    }

    let mut address_jobs = Vec::new();
    if starting_drive && let Some(drive_id) = runtime.open_drive_id {
        push_address(&mut address_jobs, Some(drive_id), None, "start", obs);
    }
    if ending_drive && let Some(drive_id) = runtime.open_drive_id {
        close_drive(
            &mut tx,
            drive_id,
            obs,
            runtime.efficiency_kwh_per_km,
            fence_id,
        )
        .await?;
        push_address(&mut address_jobs, Some(drive_id), None, "end", obs);
        runtime.open_drive_id = None;
    }
    if ending_charge && let Some(charge_id) = runtime.open_charge_id {
        close_charge(&mut tx, charge_id, obs, fence_id).await?;
        push_address(&mut address_jobs, None, Some(charge_id), "end", obs);
        runtime.open_charge_id = None;
    }

    update_vehicle_meta(&mut tx, runtime.vehicle_id, obs).await?;
    save_state(&mut tx, runtime.vehicle_id, &next, obs, live).await?;
    if let Some(unit) = obs.distance_unit.clone() {
        runtime.distance_unit = Some(unit);
    }
    if let Some(efficiency) = obs.efficiency_kwh_per_km {
        runtime.efficiency_kwh_per_km = Some(efficiency);
    }
    if let Some(model) = obs.model.clone() {
        runtime.model = Some(model);
    }
    runtime.tracker = next;
    tx.commit().await?;

    for job in address_jobs {
        if let Err(err) = attach_address(pool, geocoder, &job).await {
            tracing::warn!(error = %err, "reverse geocode failed");
        }
    }
    let _ = events.send(live_event(runtime, obs));
    Ok(())
}

struct AddressJob {
    drive_id: Option<i64>,
    charge_id: Option<i64>,
    which: &'static str,
    latitude: f64,
    longitude: f64,
}

fn push_address(
    jobs: &mut Vec<AddressJob>,
    drive_id: Option<i64>,
    charge_id: Option<i64>,
    which: &'static str,
    obs: &Observation,
) {
    let Some(latitude) = obs.latitude else {
        return;
    };
    let Some(longitude) = obs.longitude else {
        return;
    };
    jobs.push(AddressJob {
        drive_id,
        charge_id,
        which,
        latitude,
        longitude,
    });
}

async fn load_fences(tx: &mut Transaction<'_, Postgres>) -> Result<Vec<Fence>, AppError> {
    let rows: Vec<(i64, f64, f64, f64)> =
        sqlx::query_as("SELECT id, latitude, longitude, radius_m FROM geofences")
            .fetch_all(&mut **tx)
            .await?;
    Ok(rows
        .into_iter()
        .map(|(id, latitude, longitude, radius_m)| Fence {
            id,
            latitude,
            longitude,
            radius_m,
        })
        .collect())
}

async fn insert_drive(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: i64,
    obs: &Observation,
    geofence_id: Option<i64>,
) -> Result<i64, AppError> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO drives (
            vehicle_id, start_at, start_latitude, start_longitude, start_odometer_km,
            start_battery_level, start_rated_range_km, start_ideal_range_km,
            start_geofence_id, source
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'poll')
         RETURNING id",
    )
    .bind(vehicle_id)
    .bind(obs.at)
    .bind(obs.latitude)
    .bind(obs.longitude)
    .bind(obs.odometer_km)
    .bind(obs.battery_level)
    .bind(obs.rated_range_km)
    .bind(obs.ideal_range_km)
    .bind(geofence_id)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.0)
}

async fn insert_charge(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: i64,
    obs: &Observation,
    geofence_id: Option<i64>,
) -> Result<i64, AppError> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO charges (
            vehicle_id, start_at, latitude, longitude, start_battery_level,
            start_rated_range_km, start_ideal_range_km, geofence_id, is_dc, charger_type, source
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, 'poll')
         RETURNING id",
    )
    .bind(vehicle_id)
    .bind(obs.at)
    .bind(obs.latitude)
    .bind(obs.longitude)
    .bind(obs.battery_level)
    .bind(obs.rated_range_km)
    .bind(obs.ideal_range_km)
    .bind(geofence_id)
    .bind(obs.fast_charger)
    .bind(&obs.fast_charger_type)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row.0)
}

async fn insert_position(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: i64,
    obs: &Observation,
    drive_id: Option<i64>,
    charge_id: Option<i64>,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO positions (
            vehicle_id, recorded_at, latitude, longitude, speed_kmh, power_kw, odometer_km,
            elevation_m, battery_level, usable_battery_level, ideal_range_km, rated_range_km,
            est_range_km, inside_temp_c, outside_temp_c, heading_deg, drive_id, charge_id, source
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, 'poll'
         )
         ON CONFLICT (vehicle_id, recorded_at, source) WHERE source = 'poll' DO NOTHING",
    )
    .bind(vehicle_id)
    .bind(obs.at)
    .bind(obs.latitude)
    .bind(obs.longitude)
    .bind(obs.speed_kmh)
    .bind(obs.power_kw)
    .bind(obs.odometer_km)
    .bind(obs.elevation_m)
    .bind(obs.battery_level)
    .bind(obs.usable_battery_level)
    .bind(obs.ideal_range_km)
    .bind(obs.rated_range_km)
    .bind(obs.est_range_km)
    .bind(obs.inside_temp_c)
    .bind(obs.outside_temp_c)
    .bind(obs.heading_deg)
    .bind(drive_id)
    .bind(charge_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn insert_charge_sample(
    tx: &mut Transaction<'_, Postgres>,
    charge_id: i64,
    obs: &Observation,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO charge_samples (
            charge_id, recorded_at, battery_level, usable_battery_level, charger_power_kw,
            charger_voltage, charger_current, charge_energy_added_kwh, ideal_range_km,
            rated_range_km, outside_temp_c, source
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, 'poll')
         ON CONFLICT (charge_id, recorded_at, source) DO NOTHING",
    )
    .bind(charge_id)
    .bind(obs.at)
    .bind(obs.battery_level)
    .bind(obs.usable_battery_level)
    .bind(obs.charger_power_kw)
    .bind(obs.charger_voltage)
    .bind(obs.charger_current)
    .bind(obs.charge_energy_added_kwh)
    .bind(obs.ideal_range_km)
    .bind(obs.rated_range_km)
    .bind(obs.outside_temp_c)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn close_drive(
    tx: &mut Transaction<'_, Postgres>,
    drive_id: i64,
    obs: &Observation,
    efficiency: Option<f64>,
    geofence_id: Option<i64>,
) -> Result<(), AppError> {
    let rows = sqlx::query_as::<_, PositionSample>(
        "SELECT recorded_at, latitude, longitude, speed_kmh, power_kw, odometer_km, elevation_m,
                battery_level, rated_range_km, ideal_range_km, inside_temp_c, outside_temp_c
         FROM positions
         WHERE drive_id = $1
         ORDER BY recorded_at",
    )
    .bind(drive_id)
    .fetch_all(&mut **tx)
    .await?;
    let samples: Vec<Sample> = rows.into_iter().map(PositionSample::into_sample).collect();
    let Some(summary) = summarize_drive(&samples, efficiency) else {
        sqlx::query("UPDATE drives SET end_at = $2 WHERE id = $1 AND end_at IS NULL")
            .bind(drive_id)
            .bind(obs.at)
            .execute(&mut **tx)
            .await?;
        return Ok(());
    };
    update_drive(tx, drive_id, &summary, geofence_id).await
}

async fn update_drive(
    tx: &mut Transaction<'_, Postgres>,
    drive_id: i64,
    summary: &DriveSummary,
    geofence_id: Option<i64>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE drives SET
            start_at = $2,
            end_at = $3,
            start_latitude = $4,
            start_longitude = $5,
            end_latitude = $6,
            end_longitude = $7,
            start_odometer_km = $8,
            end_odometer_km = $9,
            distance_km = $10,
            duration_s = $11,
            start_battery_level = $12,
            end_battery_level = $13,
            start_rated_range_km = $14,
            end_rated_range_km = $15,
            start_ideal_range_km = $16,
            end_ideal_range_km = $17,
            energy_used_kwh = $18,
            max_speed_kmh = $19,
            avg_speed_kmh = $20,
            max_power_kw = $21,
            min_power_kw = $22,
            elevation_gain_m = $23,
            elevation_loss_m = $24,
            inside_temp_avg_c = $25,
            outside_temp_avg_c = $26,
            end_geofence_id = COALESCE($27, end_geofence_id)
         WHERE id = $1",
    )
    .bind(drive_id)
    .bind(summary.start)
    .bind(summary.end)
    .bind(summary.start_latitude)
    .bind(summary.start_longitude)
    .bind(summary.end_latitude)
    .bind(summary.end_longitude)
    .bind(summary.start_odometer_km)
    .bind(summary.end_odometer_km)
    .bind(summary.distance_km)
    .bind(summary.duration_s)
    .bind(summary.start_battery_level)
    .bind(summary.end_battery_level)
    .bind(summary.start_rated_range_km)
    .bind(summary.end_rated_range_km)
    .bind(summary.start_ideal_range_km)
    .bind(summary.end_ideal_range_km)
    .bind(summary.energy_used_kwh)
    .bind(summary.max_speed_kmh)
    .bind(summary.avg_speed_kmh)
    .bind(summary.max_power_kw)
    .bind(summary.min_power_kw)
    .bind(summary.elevation_gain_m)
    .bind(summary.elevation_loss_m)
    .bind(summary.inside_temp_avg_c)
    .bind(summary.outside_temp_avg_c)
    .bind(geofence_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn close_charge(
    tx: &mut Transaction<'_, Postgres>,
    charge_id: i64,
    obs: &Observation,
    geofence_id: Option<i64>,
) -> Result<(), AppError> {
    let rows = sqlx::query_as::<_, ChargeSampleRow>(
        "SELECT recorded_at, battery_level, charger_power_kw, charge_energy_added_kwh,
                ideal_range_km, rated_range_km, outside_temp_c
         FROM charge_samples
         WHERE charge_id = $1
         ORDER BY recorded_at",
    )
    .bind(charge_id)
    .fetch_all(&mut **tx)
    .await?;
    let mut samples: Vec<Sample> = rows.into_iter().map(ChargeSampleRow::into_sample).collect();
    if samples.is_empty() {
        samples.push(obs.sample());
    } else if let Some(last) = samples.last_mut() {
        if last.fast_charger.is_none() {
            last.fast_charger = obs.fast_charger;
        }
        if last.fast_charger_type.is_none() {
            last.fast_charger_type.clone_from(&obs.fast_charger_type);
        }
        if last.latitude.is_none() {
            last.latitude = obs.latitude;
            last.longitude = obs.longitude;
        }
    }
    let Some(summary) = summarize_charge(&samples) else {
        sqlx::query("UPDATE charges SET end_at = $2 WHERE id = $1 AND end_at IS NULL")
            .bind(charge_id)
            .bind(obs.at)
            .execute(&mut **tx)
            .await?;
        return Ok(());
    };
    sqlx::query(
        "UPDATE charges SET
            start_at = $2,
            end_at = $3,
            energy_added_kwh = $4,
            start_battery_level = $5,
            end_battery_level = $6,
            start_rated_range_km = $7,
            end_rated_range_km = $8,
            start_ideal_range_km = $9,
            end_ideal_range_km = $10,
            latitude = COALESCE($11, latitude),
            longitude = COALESCE($12, longitude),
            is_dc = COALESCE(is_dc, $13),
            charger_type = COALESCE(charger_type, $14),
            max_power_kw = $15,
            duration_s = $16,
            outside_temp_avg_c = $17,
            geofence_id = COALESCE($18, geofence_id)
         WHERE id = $1",
    )
    .bind(charge_id)
    .bind(summary.start)
    .bind(summary.end)
    .bind(summary.energy_added_kwh)
    .bind(summary.start_battery_level)
    .bind(summary.end_battery_level)
    .bind(summary.start_rated_range_km)
    .bind(summary.end_rated_range_km)
    .bind(summary.start_ideal_range_km)
    .bind(summary.end_ideal_range_km)
    .bind(summary.latitude)
    .bind(summary.longitude)
    .bind(summary.is_dc)
    .bind(&summary.charger_type)
    .bind(summary.max_power_kw)
    .bind(summary.duration_s)
    .bind(summary.outside_temp_avg_c)
    .bind(geofence_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn update_vehicle_meta(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: i64,
    obs: &Observation,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE vehicles SET
            model = COALESCE($2, model),
            trim_badging = COALESCE($3, trim_badging),
            exterior_color = COALESCE($4, exterior_color),
            wheel_type = COALESCE($5, wheel_type),
            distance_unit = COALESCE($6, distance_unit),
            updated_at = now()
         WHERE id = $1",
    )
    .bind(vehicle_id)
    .bind(&obs.model)
    .bind(&obs.trim_badging)
    .bind(&obs.exterior_color)
    .bind(&obs.wheel_type)
    .bind(&obs.distance_unit)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn save_state(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: i64,
    tracker: &Tracker,
    obs: &Observation,
    live: bool,
) -> Result<(), AppError> {
    if live {
        sqlx::query(
            "INSERT INTO vehicle_state (
                vehicle_id, activity, battery_level, usable_battery_level, rated_range_km,
                ideal_range_km, est_range_km, odometer_km, latitude, longitude, heading_deg,
                speed_kmh, power_kw, inside_temp_c, outside_temp_c, climate_on, locked,
                sentry_mode, charging_state, charger_power_kw, charge_energy_added_kwh,
                charge_limit_soc, shift_state, is_dc, software_version, update_status,
                last_poll_at, last_data_at, last_error, updated_at
             ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $27, NULL, now()
             )
             ON CONFLICT (vehicle_id) DO UPDATE SET
                activity = EXCLUDED.activity,
                battery_level = COALESCE(EXCLUDED.battery_level, vehicle_state.battery_level),
                usable_battery_level = COALESCE(EXCLUDED.usable_battery_level, vehicle_state.usable_battery_level),
                rated_range_km = COALESCE(EXCLUDED.rated_range_km, vehicle_state.rated_range_km),
                ideal_range_km = COALESCE(EXCLUDED.ideal_range_km, vehicle_state.ideal_range_km),
                est_range_km = COALESCE(EXCLUDED.est_range_km, vehicle_state.est_range_km),
                odometer_km = COALESCE(EXCLUDED.odometer_km, vehicle_state.odometer_km),
                latitude = COALESCE(EXCLUDED.latitude, vehicle_state.latitude),
                longitude = COALESCE(EXCLUDED.longitude, vehicle_state.longitude),
                heading_deg = COALESCE(EXCLUDED.heading_deg, vehicle_state.heading_deg),
                speed_kmh = EXCLUDED.speed_kmh,
                power_kw = EXCLUDED.power_kw,
                inside_temp_c = COALESCE(EXCLUDED.inside_temp_c, vehicle_state.inside_temp_c),
                outside_temp_c = COALESCE(EXCLUDED.outside_temp_c, vehicle_state.outside_temp_c),
                climate_on = COALESCE(EXCLUDED.climate_on, vehicle_state.climate_on),
                locked = COALESCE(EXCLUDED.locked, vehicle_state.locked),
                sentry_mode = COALESCE(EXCLUDED.sentry_mode, vehicle_state.sentry_mode),
                charging_state = COALESCE(EXCLUDED.charging_state, vehicle_state.charging_state),
                charger_power_kw = EXCLUDED.charger_power_kw,
                charge_energy_added_kwh = COALESCE(EXCLUDED.charge_energy_added_kwh, vehicle_state.charge_energy_added_kwh),
                charge_limit_soc = COALESCE(EXCLUDED.charge_limit_soc, vehicle_state.charge_limit_soc),
                shift_state = EXCLUDED.shift_state,
                is_dc = COALESCE(EXCLUDED.is_dc, vehicle_state.is_dc),
                software_version = COALESCE(EXCLUDED.software_version, vehicle_state.software_version),
                update_status = EXCLUDED.update_status,
                last_poll_at = EXCLUDED.last_poll_at,
                last_data_at = EXCLUDED.last_data_at,
                last_error = NULL,
                updated_at = now()",
        )
        .bind(vehicle_id)
        .bind(tracker.activity.as_str())
        .bind(obs.battery_level)
        .bind(obs.usable_battery_level)
        .bind(obs.rated_range_km)
        .bind(obs.ideal_range_km)
        .bind(obs.est_range_km)
        .bind(obs.odometer_km)
        .bind(obs.latitude)
        .bind(obs.longitude)
        .bind(obs.heading_deg)
        .bind(obs.speed_kmh)
        .bind(obs.power_kw)
        .bind(obs.inside_temp_c)
        .bind(obs.outside_temp_c)
        .bind(obs.climate_on)
        .bind(obs.locked)
        .bind(obs.sentry_mode)
        .bind(&obs.charging_state)
        .bind(obs.charger_power_kw)
        .bind(obs.charge_energy_added_kwh)
        .bind(obs.charge_limit_soc)
        .bind(&obs.shift_state)
        .bind(obs.fast_charger)
        .bind(&obs.software_version)
        .bind(&obs.update_status)
        .bind(obs.at)
        .execute(&mut **tx)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO vehicle_state (vehicle_id, activity, last_poll_at, last_error, updated_at)
             VALUES ($1, $2, $3, NULL, now())
             ON CONFLICT (vehicle_id) DO UPDATE SET
                activity = EXCLUDED.activity,
                last_poll_at = EXCLUDED.last_poll_at,
                last_error = NULL,
                updated_at = now()",
        )
        .bind(vehicle_id)
        .bind(tracker.activity.as_str())
        .bind(obs.at)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}

async fn attach_address(
    pool: &PgPool,
    geocoder: &Geocoder,
    job: &AddressJob,
) -> Result<(), AppError> {
    let Some(address_id) = lookup_or_geocode(pool, geocoder, job.latitude, job.longitude).await?
    else {
        return Ok(());
    };
    if let Some(drive_id) = job.drive_id {
        let column = if job.which == "start" {
            "start_address_id"
        } else {
            "end_address_id"
        };
        let sql = format!("UPDATE drives SET {column} = $2 WHERE id = $1");
        sqlx::query(&sql)
            .bind(drive_id)
            .bind(address_id)
            .execute(pool)
            .await?;
    }
    if let Some(charge_id) = job.charge_id {
        sqlx::query("UPDATE charges SET address_id = $2 WHERE id = $1")
            .bind(charge_id)
            .bind(address_id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn lookup_or_geocode(
    pool: &PgPool,
    geocoder: &Geocoder,
    latitude: f64,
    longitude: f64,
) -> Result<Option<i64>, AppError> {
    let key = cache_key(latitude, longitude);
    if let Some(row) = sqlx::query_as::<_, (i64,)>("SELECT id FROM addresses WHERE cache_key = $1")
        .bind(&key)
        .fetch_optional(pool)
        .await?
    {
        return Ok(Some(row.0));
    }
    let Some(parsed) = geocoder.reverse(latitude, longitude).await? else {
        return Ok(None);
    };
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO addresses (
            latitude, longitude, display_name, name, house_number, road, neighbourhood, city,
            county, state, state_district, postcode, country, osm_id, osm_type, raw, cache_key, source
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, 'nominatim'
         )
         ON CONFLICT (cache_key) WHERE cache_key IS NOT NULL DO UPDATE SET
            display_name = COALESCE(EXCLUDED.display_name, addresses.display_name)
         RETURNING id",
    )
    .bind(latitude)
    .bind(longitude)
    .bind(&parsed.display_name)
    .bind(&parsed.name)
    .bind(&parsed.house_number)
    .bind(&parsed.road)
    .bind(&parsed.neighbourhood)
    .bind(&parsed.city)
    .bind(&parsed.county)
    .bind(&parsed.state)
    .bind(&parsed.state_district)
    .bind(&parsed.postcode)
    .bind(&parsed.country)
    .bind(parsed.osm_id)
    .bind(&parsed.osm_type)
    .bind(&parsed.raw)
    .bind(key)
    .fetch_one(pool)
    .await?;
    Ok(Some(row.0))
}

fn live_event(runtime: &Runtime, obs: &Observation) -> LiveEvent {
    LiveEvent {
        kind: "state",
        vehicle_id: runtime.vehicle_id,
        vin: runtime.vin.clone(),
        name: runtime.name.clone(),
        model: runtime.model.clone(),
        activity: runtime.tracker.activity.as_str(),
        battery_level: obs.battery_level,
        rated_range_km: obs.rated_range_km,
        odometer_km: obs.odometer_km,
        speed_kmh: obs.speed_kmh,
        power_kw: obs.power_kw,
        charger_power_kw: obs.charger_power_kw,
        inside_temp_c: obs.inside_temp_c,
        outside_temp_c: obs.outside_temp_c,
        latitude: obs.latitude,
        longitude: obs.longitude,
        at: obs.at,
    }
}

fn clip_error(text: &str) -> String {
    redact_secrets(text).chars().take(500).collect()
}

#[derive(sqlx::FromRow)]
struct PositionSample {
    recorded_at: DateTime<Utc>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    speed_kmh: Option<f64>,
    power_kw: Option<f64>,
    odometer_km: Option<f64>,
    elevation_m: Option<f64>,
    battery_level: Option<f64>,
    rated_range_km: Option<f64>,
    ideal_range_km: Option<f64>,
    inside_temp_c: Option<f64>,
    outside_temp_c: Option<f64>,
}

impl PositionSample {
    fn into_sample(self) -> Sample {
        Sample {
            at: self.recorded_at,
            latitude: self.latitude,
            longitude: self.longitude,
            speed_kmh: self.speed_kmh,
            power_kw: self.power_kw,
            odometer_km: self.odometer_km,
            elevation_m: self.elevation_m,
            battery_level: self.battery_level,
            rated_range_km: self.rated_range_km,
            ideal_range_km: self.ideal_range_km,
            inside_temp_c: self.inside_temp_c,
            outside_temp_c: self.outside_temp_c,
            charge_energy_added_kwh: None,
            charger_power_kw: None,
            fast_charger: None,
            fast_charger_type: None,
        }
    }
}

#[derive(sqlx::FromRow)]
struct ChargeSampleRow {
    recorded_at: DateTime<Utc>,
    battery_level: Option<f64>,
    charger_power_kw: Option<f64>,
    charge_energy_added_kwh: Option<f64>,
    ideal_range_km: Option<f64>,
    rated_range_km: Option<f64>,
    outside_temp_c: Option<f64>,
}

impl ChargeSampleRow {
    fn into_sample(self) -> Sample {
        Sample {
            at: self.recorded_at,
            latitude: None,
            longitude: None,
            speed_kmh: None,
            power_kw: None,
            odometer_km: None,
            elevation_m: None,
            battery_level: self.battery_level,
            rated_range_km: self.rated_range_km,
            ideal_range_km: self.ideal_range_km,
            inside_temp_c: None,
            outside_temp_c: self.outside_temp_c,
            charge_energy_added_kwh: self.charge_energy_added_kwh,
            charger_power_kw: self.charger_power_kw,
            fast_charger: None,
            fast_charger_type: None,
        }
    }
}

pub(crate) fn resting_observation(at: DateTime<Utc>, presence: Presence) -> Observation {
    Observation::at(at, presence)
}
