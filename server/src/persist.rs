// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use tokio::sync::{Mutex, broadcast};

use crate::error::{AppError, redact_secrets};
use crate::geocode::{Geocoder, cache_key};
use crate::geofence::{Fence, matching_geofence};
use crate::state::{
    Activity, ClosedDrive, DriveSummary, Observation, Presence, Sample, Tracker, Transition,
    gate_stream_sample, keep_closed_drive, soc_level, summarize_charge, summarize_drive,
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
    /// Last merged status published to MQTT and the websocket.
    pub published: Option<Observation>,
    /// Time of the last Fleet poll. Stream frames keep this instead of their own timestamp.
    pub last_poll_at: Option<DateTime<Utc>>,
    pub efficiency_kwh_per_km: Option<f64>,
    pub distance_unit: Option<String>,
    pub name: Option<String>,
    pub model: Option<String>,
    pub next_due: tokio::time::Instant,
    /// Newest drive this vehicle has finished. Late stream samples join it.
    pub last_closed_drive: Option<ClosedDrive>,
}

/// Push payload for the read API websocket and the MQTT publisher.
#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct LiveEvent {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub vehicle_id: i64,
    pub vin: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub activity: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_level: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rated_range_km: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub odometer_km: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_kmh: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub power_kw: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charger_power_kw: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inside_temp_c: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outside_temp_c: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub longitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usable_battery_level: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ideal_range_km: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub est_range_km: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub heading_deg: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub climate_on: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locked: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sentry_mode: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charging_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_energy_added_kwh: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_limit_soc: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shift_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_dc: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub software_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update_status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_poll_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_data_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charger_voltage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charger_current: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_current_request: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_port_door_open: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charge_port_latch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plugged_in: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_full_charge_h: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scheduled_charging_start_time: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpms_pressure_fl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpms_pressure_fr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpms_pressure_rl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tpms_pressure_rr: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub df: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dr: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pf: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fd_window: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fp_window: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rd_window: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rp_window: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ft: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rt: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_user_present: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elevation_m: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery_heater_on: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_preconditioning: Option<bool>,
    pub at: DateTime<Utc>,
}

impl LiveEvent {
    /// A state frame with no measurements. Callers fill the fields they have.
    pub(crate) fn skeleton(vin: impl Into<String>, activity: &'static str) -> Self {
        let at = Utc::now();
        Self::from_observation(
            0,
            vin.into(),
            None,
            None,
            activity,
            &Observation::at(at, Presence::Offline),
            at,
        )
    }

    fn from_observation(
        vehicle_id: i64,
        vin: String,
        name: Option<String>,
        model: Option<String>,
        activity: &'static str,
        obs: &Observation,
        at: DateTime<Utc>,
    ) -> Self {
        Self {
            kind: "state",
            vehicle_id,
            vin,
            name,
            model,
            activity,
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
            usable_battery_level: obs.usable_battery_level,
            ideal_range_km: obs.ideal_range_km,
            est_range_km: obs.est_range_km,
            heading_deg: obs.heading_deg,
            climate_on: obs.climate_on,
            locked: obs.locked,
            sentry_mode: obs.sentry_mode,
            charging_state: obs.charging_state.clone(),
            charge_energy_added_kwh: obs.charge_energy_added_kwh,
            charge_limit_soc: obs.charge_limit_soc,
            shift_state: obs.shift_state.clone(),
            is_dc: obs.fast_charger,
            software_version: obs.software_version.clone(),
            update_status: obs.update_status.clone(),
            last_poll_at: Some(at),
            last_data_at: Some(at),
            last_error: None,
            updated_at: Some(at),
            charger_voltage: obs.charger_voltage,
            charger_current: obs.charger_current,
            charge_current_request: obs.charge_current_request,
            charge_port_door_open: obs.charge_port_door_open,
            charge_port_latch: obs.charge_port_latch.clone(),
            plugged_in: obs.plugged_in,
            time_to_full_charge_h: obs.time_to_full_charge_h,
            scheduled_charging_start_time: obs.scheduled_charging_start_time,
            tpms_pressure_fl: obs.tpms_pressure_fl,
            tpms_pressure_fr: obs.tpms_pressure_fr,
            tpms_pressure_rl: obs.tpms_pressure_rl,
            tpms_pressure_rr: obs.tpms_pressure_rr,
            df: obs.df,
            dr: obs.dr,
            pf: obs.pf,
            pr: obs.pr,
            fd_window: obs.fd_window,
            fp_window: obs.fp_window,
            rd_window: obs.rd_window,
            rp_window: obs.rp_window,
            ft: obs.ft,
            rt: obs.rt,
            is_user_present: obs.is_user_present,
            elevation_m: obs.elevation_m,
            battery_heater_on: obs.battery_heater_on,
            is_preconditioning: obs.is_preconditioning,
            at,
        }
    }
}

pub(crate) async fn upsert_vehicle(
    pool: &PgPool,
    identity: VehicleIdentity<'_>,
    account_id: Option<i64>,
) -> Result<i64, AppError> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO vehicles (vin, tesla_id, legacy_vehicle_id, name, source, external_id, account_id)
         VALUES ($1, $2, $3, $4, 'fleet', $1, $5)
         ON CONFLICT (vin) DO UPDATE SET
            tesla_id = COALESCE(EXCLUDED.tesla_id, vehicles.tesla_id),
            legacy_vehicle_id = COALESCE(EXCLUDED.legacy_vehicle_id, vehicles.legacy_vehicle_id),
            name = COALESCE(EXCLUDED.name, vehicles.name),
            account_id = COALESCE(vehicles.account_id, EXCLUDED.account_id),
            updated_at = now()
         RETURNING id",
    )
    .bind(identity.vin)
    .bind(identity.tesla_id)
    .bind(identity.legacy_vehicle_id)
    .bind(identity.name)
    .bind(account_id)
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
         LEFT JOIN drives d ON d.vehicle_id = v.id AND d.end_at IS NULL AND d.source IN ('poll', 'stream')
         LEFT JOIN charges c ON c.vehicle_id = v.id AND c.end_at IS NULL AND c.source = 'poll'
         WHERE v.id = $1",
    )
    .bind(vehicle_id)
    .fetch_one(pool)
    .await?;
    let closed: Option<(i64, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
        "SELECT id, start_at, end_at FROM drives
         WHERE vehicle_id = $1 AND end_at IS NOT NULL
         ORDER BY end_at DESC
         LIMIT 1",
    )
    .bind(vehicle_id)
    .fetch_optional(pool)
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
        published: None,
        last_poll_at: None,
        efficiency_kwh_per_km: row.efficiency_kwh_per_km,
        distance_unit: row.distance_unit,
        name: row.name,
        model: row.model,
        next_due: tokio::time::Instant::now(),
        last_closed_drive: closed.map(|(id, start_at, end_at)| ClosedDrive {
            id,
            start_at,
            end_at,
        }),
    })
}

/// Geofences change rarely. Stream batches reuse this instead of reading them per sample.
#[derive(Default)]
pub(crate) struct FenceCache {
    inner: Mutex<Option<(Instant, Vec<Fence>)>>,
}

impl FenceCache {
    pub(crate) async fn get(&self, pool: &PgPool) -> Result<Vec<Fence>, AppError> {
        const TTL: Duration = Duration::from_secs(60);
        let cached = {
            let guard = self.inner.lock().await;
            guard
                .as_ref()
                .and_then(|(loaded, fences)| (loaded.elapsed() < TTL).then(|| fences.clone()))
        };
        if let Some(fences) = cached {
            return Ok(fences);
        }
        let fences = fetch_fences(pool).await?;
        *self.inner.lock().await = Some((Instant::now(), fences.clone()));
        Ok(fences)
    }
}

async fn fetch_fences(pool: &PgPool) -> Result<Vec<Fence>, AppError> {
    let rows: Vec<(i64, f64, f64, f64)> =
        sqlx::query_as("SELECT id, latitude, longitude, radius_m FROM geofences")
            .fetch_all(pool)
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
///
/// Address lookups are returned so the caller can geocode after releasing the
/// runtime lock. The network call must not run while that lock is held.
pub(crate) async fn apply_observation(
    pool: &PgPool,
    runtime: &mut Runtime,
    obs: &Observation,
    fences: &[Fence],
    events: &broadcast::Sender<LiveEvent>,
    live: bool,
    store_position: bool,
) -> Result<Vec<AddressJob>, AppError> {
    let previous = runtime.tracker.clone();
    let (next, transitions) = crate::state::reduce(&previous, obs);
    let starting_drive = transitions.contains(&Transition::DriveStarted);
    let ending_drive = transitions.contains(&Transition::DriveEnded);
    let starting_charge = transitions.contains(&Transition::ChargeStarted);
    let ending_charge = transitions.contains(&Transition::ChargeEnded);

    let mut tx = pool.begin().await?;
    let fence_id = obs
        .latitude
        .zip(obs.longitude)
        .and_then(|(latitude, longitude)| matching_geofence(latitude, longitude, fences));

    let mut created_drive = false;
    if starting_drive {
        let (drive_id, created) =
            insert_drive(&mut tx, runtime.vehicle_id, obs, fence_id, "poll").await?;
        runtime.open_drive_id = Some(drive_id);
        created_drive = created;
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
    if live && store_position && obs.has_sample() {
        insert_position(
            &mut tx,
            runtime.vehicle_id,
            obs,
            attach_drive,
            attach_charge,
        )
        .await?;
    }
    if let Some(drive_id) = attach_drive
        && let Some(level) = soc_level(obs.battery_level, obs.usable_battery_level)
    {
        sqlx::query(
            "UPDATE drives SET start_battery_level = COALESCE(start_battery_level, $2) WHERE id = $1",
        )
        .bind(drive_id)
        .bind(level)
        .execute(&mut *tx)
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
    if created_drive && let Some(drive_id) = runtime.open_drive_id {
        push_address(&mut address_jobs, Some(drive_id), None, "start", obs);
    }
    if ending_drive && let Some(drive_id) = runtime.open_drive_id {
        if let Some(closed) = close_drive(
            &mut tx,
            runtime.vehicle_id,
            drive_id,
            obs,
            runtime.efficiency_kwh_per_km,
            fence_id,
        )
        .await?
        {
            push_address(&mut address_jobs, Some(drive_id), None, "end", obs);
            remember_closed(&mut runtime.last_closed_drive, closed);
        }
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

    runtime.last_poll_at = Some(obs.at);
    if live {
        let merged = crate::status::merge_poll(runtime.published.as_ref(), obs);
        runtime.published = Some(merged);
    }
    let event_obs = runtime.published.clone().unwrap_or_else(|| obs.clone());
    let _ = events.send(live_event(runtime, &event_obs));
    Ok(address_jobs)
}

/// Reverse-geocode drive and charge endpoints. Safe to call without the runtime lock.
pub(crate) async fn settle_addresses(pool: &PgPool, geocoder: &Geocoder, jobs: Vec<AddressJob>) {
    for job in jobs {
        if let Err(err) = attach_address(pool, geocoder, &job).await {
            tracing::warn!(error = %err, "reverse geocode failed");
        }
    }
}

/// One Owner API streaming fix, already converted to kilometres and metres.
pub(crate) struct StreamFix {
    pub at: DateTime<Utc>,
    pub shift_state: Option<String>,
    pub speed_kmh: Option<f64>,
    pub power_kw: Option<f64>,
    pub odometer_km: Option<f64>,
    pub battery_level: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub elevation_m: Option<f64>,
    pub heading_deg: Option<f64>,
    pub rated_range_km: Option<f64>,
    pub est_range_km: Option<f64>,
}

/// Records stream samples in one transaction.
///
/// `synchronous_commit` is off for this transaction: losing the last second of
/// positions is acceptable, waiting for a disk flush on every sample is not.
/// Geocoding is left to the caller so it can drop the runtime lock first.
pub(crate) async fn apply_stream_samples(
    pool: &PgPool,
    runtime: &mut Runtime,
    samples: &[StreamFix],
    fences: &[Fence],
    events: &broadcast::Sender<LiveEvent>,
) -> Result<Vec<AddressJob>, AppError> {
    if samples.is_empty() {
        return Ok(Vec::new());
    }
    let mut open_charge_id = runtime.open_charge_id;
    let mut open_drive_id = runtime.open_drive_id;
    let mut drive_open = runtime.tracker.drive_open;
    let mut charge_open = runtime.tracker.charge_open;
    let mut activity = runtime.tracker.activity;
    let mut last_closed = runtime.last_closed_drive;
    let mut published = runtime.published.clone();
    let mut address_jobs = Vec::new();
    let mut refresh_closed: Option<i64> = None;
    let mut last_touch: Option<(Activity, Observation)> = None;
    let now = Utc::now();

    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL synchronous_commit = off")
        .execute(&mut *tx)
        .await?;

    for sample in samples {
        let gate = gate_stream_sample(sample.at, now, last_closed);
        let tracker = Tracker {
            activity,
            drive_open,
            charge_open,
        };
        let mut decision = crate::state::stream_drive_decision(
            &tracker,
            sample.shift_state.as_deref(),
            sample.speed_kmh,
        );
        if !gate.allow_open {
            decision.start_drive = false;
        }
        let obs = observation_from_stream(sample);
        let fence_id = obs
            .latitude
            .zip(obs.longitude)
            .and_then(|(latitude, longitude)| matching_geofence(latitude, longitude, fences));

        if let Some(closed_id) = gate.attach_closed {
            if crate::state::should_store_stream_position(true, false, obs.latitude, obs.longitude)
            {
                let external_id = format!("{}:{}", runtime.vehicle_id, obs.at.timestamp_millis());
                insert_stream_position(
                    &mut tx,
                    runtime.vehicle_id,
                    &obs,
                    Some(closed_id),
                    &external_id,
                )
                .await?;
                refresh_closed = Some(closed_id);
            }
            continue;
        }
        if !gate.allow_open && !drive_open {
            continue;
        }

        if decision.end_charge
            && let Some(charge_id) = open_charge_id
        {
            close_charge(&mut tx, charge_id, &obs, fence_id).await?;
            push_address(&mut address_jobs, None, Some(charge_id), "end", &obs);
            open_charge_id = None;
            charge_open = false;
        }
        if decision.start_drive {
            let (drive_id, created) =
                insert_drive(&mut tx, runtime.vehicle_id, &obs, fence_id, "stream").await?;
            open_drive_id = Some(drive_id);
            drive_open = true;
            if created {
                push_address(&mut address_jobs, open_drive_id, None, "start", &obs);
            }
        }
        let attach_drive = if drive_open { open_drive_id } else { None };
        let store = crate::state::should_store_stream_position(
            decision.driving,
            decision.end_drive,
            sample.latitude,
            sample.longitude,
        );
        if store {
            let external_id = format!("{}:{}", runtime.vehicle_id, obs.at.timestamp_millis());
            insert_stream_position(
                &mut tx,
                runtime.vehicle_id,
                &obs,
                attach_drive,
                &external_id,
            )
            .await?;
        }
        if let Some(drive_id) = attach_drive
            && let Some(level) = soc_level(obs.battery_level, obs.usable_battery_level)
        {
            sqlx::query(
                "UPDATE drives SET start_battery_level = COALESCE(start_battery_level, $2) WHERE id = $1",
            )
            .bind(drive_id)
            .bind(level)
            .execute(&mut *tx)
            .await?;
        }
        if decision.end_drive
            && let Some(drive_id) = open_drive_id
        {
            if let Some(closed) = close_drive(
                &mut tx,
                runtime.vehicle_id,
                drive_id,
                &obs,
                runtime.efficiency_kwh_per_km,
                fence_id,
            )
            .await?
            {
                push_address(&mut address_jobs, Some(drive_id), None, "end", &obs);
                remember_closed(&mut last_closed, closed);
            }
            open_drive_id = None;
            drive_open = false;
        }
        activity = decision.activity;
        published = Some(crate::status::merge_stream(published.as_ref(), &obs));
        last_touch = Some((decision.activity, obs));
    }

    if let Some(drive_id) = refresh_closed
        && let Some(closed) =
            refresh_closed_drive(&mut tx, drive_id, runtime.efficiency_kwh_per_km).await?
    {
        remember_closed(&mut last_closed, closed);
    }
    if let Some((activity, obs)) = &last_touch {
        save_stream_state(&mut tx, runtime.vehicle_id, *activity, obs).await?;
    }
    tx.commit().await?;

    runtime.open_drive_id = open_drive_id;
    runtime.open_charge_id = open_charge_id;
    runtime.tracker.drive_open = drive_open;
    runtime.tracker.charge_open = charge_open;
    runtime.tracker.activity = activity;
    runtime.last_closed_drive = last_closed;
    if let Some(merged) = published {
        runtime.published = Some(merged.clone());
        if last_touch.is_some() {
            let _ = events.send(live_event(runtime, &merged));
        }
    }
    Ok(address_jobs)
}

fn observation_from_stream(sample: &StreamFix) -> Observation {
    let mut obs = Observation::at(sample.at, crate::state::Presence::Online);
    obs.shift_state.clone_from(&sample.shift_state);
    obs.speed_kmh = sample.speed_kmh;
    obs.power_kw = sample.power_kw;
    obs.odometer_km = sample.odometer_km;
    obs.battery_level = sample.battery_level;
    obs.latitude = sample.latitude;
    obs.longitude = sample.longitude;
    obs.elevation_m = sample.elevation_m;
    obs.heading_deg = sample.heading_deg;
    obs.rated_range_km = sample.rated_range_km;
    obs.est_range_km = sample.est_range_km;
    obs
}

async fn insert_stream_position(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: i64,
    obs: &Observation,
    drive_id: Option<i64>,
    external_id: &str,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO positions (
            vehicle_id, recorded_at, latitude, longitude, speed_kmh, power_kw, odometer_km,
            elevation_m, battery_level, usable_battery_level, ideal_range_km, rated_range_km,
            est_range_km, inside_temp_c, outside_temp_c, heading_deg, drive_id, charge_id,
            source, external_id
         )
         SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9, NULL, NULL, $10, $11, NULL, NULL, $12, $13,
                NULL, 'stream', $14
         WHERE NOT EXISTS (
            SELECT 1 FROM positions
            WHERE vehicle_id = $1
              AND source = 'poll'
              AND recorded_at BETWEEN $2 - interval '2 seconds' AND $2 + interval '2 seconds'
         )
         ON CONFLICT (source, external_id, recorded_at) WHERE external_id IS NOT NULL DO NOTHING",
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
    .bind(obs.rated_range_km)
    .bind(obs.est_range_km)
    .bind(obs.heading_deg)
    .bind(drive_id)
    .bind(external_id)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn save_stream_state(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: i64,
    activity: crate::state::Activity,
    obs: &Observation,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO vehicle_state (
            vehicle_id, activity, battery_level, rated_range_km, est_range_km, odometer_km,
            latitude, longitude, heading_deg, speed_kmh, power_kw, shift_state, elevation_m,
            last_data_at, updated_at
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, now()
         )
         ON CONFLICT (vehicle_id) DO UPDATE SET
            activity = EXCLUDED.activity,
            battery_level = COALESCE(EXCLUDED.battery_level, vehicle_state.battery_level),
            rated_range_km = COALESCE(EXCLUDED.rated_range_km, vehicle_state.rated_range_km),
            est_range_km = COALESCE(EXCLUDED.est_range_km, vehicle_state.est_range_km),
            odometer_km = COALESCE(EXCLUDED.odometer_km, vehicle_state.odometer_km),
            latitude = COALESCE(EXCLUDED.latitude, vehicle_state.latitude),
            longitude = COALESCE(EXCLUDED.longitude, vehicle_state.longitude),
            heading_deg = COALESCE(EXCLUDED.heading_deg, vehicle_state.heading_deg),
            speed_kmh = COALESCE(EXCLUDED.speed_kmh, vehicle_state.speed_kmh),
            power_kw = COALESCE(EXCLUDED.power_kw, vehicle_state.power_kw),
            shift_state = COALESCE(EXCLUDED.shift_state, vehicle_state.shift_state),
            elevation_m = COALESCE(EXCLUDED.elevation_m, vehicle_state.elevation_m),
            last_data_at = EXCLUDED.last_data_at,
            updated_at = now()",
    )
    .bind(vehicle_id)
    .bind(activity.as_str())
    .bind(obs.battery_level)
    .bind(obs.rated_range_km)
    .bind(obs.est_range_km)
    .bind(obs.odometer_km)
    .bind(obs.latitude)
    .bind(obs.longitude)
    .bind(obs.heading_deg)
    .bind(obs.speed_kmh)
    .bind(obs.power_kw)
    .bind(&obs.shift_state)
    .bind(obs.elevation_m)
    .bind(obs.at)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

pub(crate) struct AddressJob {
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

async fn insert_drive(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: i64,
    obs: &Observation,
    geofence_id: Option<i64>,
    source: &str,
) -> Result<(i64, bool), AppError> {
    let row: (i64, bool) = sqlx::query_as(
        "WITH inserted AS (
            INSERT INTO drives (
                vehicle_id, start_at, start_latitude, start_longitude, start_odometer_km,
                start_battery_level, start_rated_range_km, start_ideal_range_km,
                start_geofence_id, source
            )
            SELECT $1, $2, $3, $4, $5,
                   COALESCE($6, s.battery_level, s.usable_battery_level),
                   $7, $8, $9, $10
            FROM (SELECT 1) AS seed
            LEFT JOIN vehicle_state s ON s.vehicle_id = $1
            ON CONFLICT (vehicle_id) WHERE end_at IS NULL DO NOTHING
            RETURNING id
         )
         SELECT id, TRUE AS created FROM inserted
         UNION ALL
         SELECT d.id, FALSE AS created FROM drives d
         WHERE d.vehicle_id = $1
           AND d.end_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM inserted)
         LIMIT 1",
    )
    .bind(vehicle_id)
    .bind(obs.at)
    .bind(obs.latitude)
    .bind(obs.longitude)
    .bind(obs.odometer_km)
    .bind(soc_level(obs.battery_level, obs.usable_battery_level))
    .bind(obs.rated_range_km)
    .bind(obs.ideal_range_km)
    .bind(geofence_id)
    .bind(source)
    .fetch_one(&mut **tx)
    .await?;
    Ok(row)
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
    // The poll row is the richer sample. Drop streamed fixes from the same moment
    // so a drive does not keep two points for one poll. The window matches
    // `stream_collides_with_poll` (two seconds).
    sqlx::query(
        "DELETE FROM positions
         WHERE vehicle_id = $1
           AND source = 'stream'
           AND recorded_at BETWEEN $2 - interval '2 seconds' AND $2 + interval '2 seconds'",
    )
    .bind(vehicle_id)
    .bind(obs.at)
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

/// Closes `drive_id`. Returns the kept span.
///
/// A drive with fewer than two positions, or an odometer delta that is NULL or
/// below 0.01 km, is deleted in this transaction. Its positions stay, with
/// `drive_id` set to NULL before the delete. Callers must not read the row
/// when this returns [`None`].
async fn close_drive(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: i64,
    drive_id: i64,
    obs: &Observation,
    efficiency: Option<f64>,
    geofence_id: Option<i64>,
) -> Result<Option<ClosedDrive>, AppError> {
    let rows = sqlx::query_as::<_, PositionSample>(
        "SELECT recorded_at, latitude, longitude, speed_kmh, power_kw, odometer_km, elevation_m,
                battery_level, rated_range_km, ideal_range_km, inside_temp_c, outside_temp_c
         FROM positions
         WHERE drive_id = $1
         ORDER BY recorded_at, id",
    )
    .bind(drive_id)
    .fetch_all(&mut **tx)
    .await?;
    let samples: Vec<Sample> = rows.into_iter().map(PositionSample::into_sample).collect();
    let Some(summary) =
        summarize_drive(&samples, efficiency).filter(|_| keep_closed_drive(&samples))
    else {
        discard_drive(tx, vehicle_id, drive_id).await?;
        return Ok(None);
    };
    let end_battery = summary
        .end_battery_level
        .or(soc_level(obs.battery_level, obs.usable_battery_level));
    update_drive(tx, drive_id, &summary, end_battery, geofence_id).await?;
    Ok(Some(ClosedDrive {
        id: drive_id,
        start_at: summary.start,
        end_at: summary.end,
    }))
}

/// Recomputes a drive that was already closed after late positions were attached.
///
/// Unlike [`close_drive`], a drive that no longer meets the keep rule is left in
/// place. A late sample must not delete a drive the poll already accepted.
async fn refresh_closed_drive(
    tx: &mut Transaction<'_, Postgres>,
    drive_id: i64,
    efficiency: Option<f64>,
) -> Result<Option<ClosedDrive>, AppError> {
    let rows = sqlx::query_as::<_, PositionSample>(
        "SELECT recorded_at, latitude, longitude, speed_kmh, power_kw, odometer_km, elevation_m,
                battery_level, rated_range_km, ideal_range_km, inside_temp_c, outside_temp_c
         FROM positions
         WHERE drive_id = $1
         ORDER BY recorded_at, id",
    )
    .bind(drive_id)
    .fetch_all(&mut **tx)
    .await?;
    let samples: Vec<Sample> = rows.into_iter().map(PositionSample::into_sample).collect();
    let Some(summary) =
        summarize_drive(&samples, efficiency).filter(|_| keep_closed_drive(&samples))
    else {
        return Ok(None);
    };
    update_drive(tx, drive_id, &summary, summary.end_battery_level, None).await?;
    Ok(Some(ClosedDrive {
        id: drive_id,
        start_at: summary.start,
        end_at: summary.end,
    }))
}

fn remember_closed(slot: &mut Option<ClosedDrive>, closed: ClosedDrive) {
    let replace = slot.is_none_or(|current| closed.end_at >= current.end_at);
    if replace {
        *slot = Some(closed);
    }
}

async fn discard_drive(
    tx: &mut Transaction<'_, Postgres>,
    vehicle_id: i64,
    drive_id: i64,
) -> Result<(), AppError> {
    sqlx::query("UPDATE positions SET drive_id = NULL WHERE drive_id = $1")
        .bind(drive_id)
        .execute(&mut **tx)
        .await?;
    sqlx::query("DELETE FROM drives WHERE id = $1")
        .bind(drive_id)
        .execute(&mut **tx)
        .await?;
    tracing::info!(vehicle_id, drive_id, "discarded drive");
    Ok(())
}

async fn update_drive(
    tx: &mut Transaction<'_, Postgres>,
    drive_id: i64,
    summary: &DriveSummary,
    end_battery: Option<f64>,
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
            start_battery_level = COALESCE(start_battery_level, $12),
            end_battery_level = COALESCE($13, end_battery_level),
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
    .bind(end_battery)
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
            charger_type = COALESCE(
                CASE
                    WHEN btrim(charger_type) = ''
                        OR lower(btrim(charger_type)) IN ('<invalid>', 'none', 'null')
                        OR btrim(charger_type) ~ '^<.*>$'
                    THEN NULL
                    ELSE charger_type
                END,
                $14
            ),
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
                last_poll_at, last_data_at, last_error, updated_at,
                charger_voltage, charger_current, charge_current_request, charge_port_door_open,
                charge_port_latch, plugged_in, time_to_full_charge_h, scheduled_charging_start_time,
                tpms_pressure_fl, tpms_pressure_fr, tpms_pressure_rl, tpms_pressure_rr,
                df, dr, pf, pr, fd_window, fp_window, rd_window, rp_window, ft, rt,
                is_user_present, elevation_m, battery_heater_on, is_preconditioning
             ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $27, NULL, now(),
                $28, $29, $30, $31, $32, $33, $34, $35, $36, $37, $38, $39,
                $40, $41, $42, $43, $44, $45, $46, $47, $48, $49, $50, $51, $52, $53
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
                updated_at = now(),
                charger_voltage = COALESCE(EXCLUDED.charger_voltage, vehicle_state.charger_voltage),
                charger_current = COALESCE(EXCLUDED.charger_current, vehicle_state.charger_current),
                charge_current_request = COALESCE(EXCLUDED.charge_current_request, vehicle_state.charge_current_request),
                charge_port_door_open = COALESCE(EXCLUDED.charge_port_door_open, vehicle_state.charge_port_door_open),
                charge_port_latch = COALESCE(EXCLUDED.charge_port_latch, vehicle_state.charge_port_latch),
                plugged_in = COALESCE(EXCLUDED.plugged_in, vehicle_state.plugged_in),
                time_to_full_charge_h = COALESCE(EXCLUDED.time_to_full_charge_h, vehicle_state.time_to_full_charge_h),
                scheduled_charging_start_time = COALESCE(EXCLUDED.scheduled_charging_start_time, vehicle_state.scheduled_charging_start_time),
                tpms_pressure_fl = COALESCE(EXCLUDED.tpms_pressure_fl, vehicle_state.tpms_pressure_fl),
                tpms_pressure_fr = COALESCE(EXCLUDED.tpms_pressure_fr, vehicle_state.tpms_pressure_fr),
                tpms_pressure_rl = COALESCE(EXCLUDED.tpms_pressure_rl, vehicle_state.tpms_pressure_rl),
                tpms_pressure_rr = COALESCE(EXCLUDED.tpms_pressure_rr, vehicle_state.tpms_pressure_rr),
                df = COALESCE(EXCLUDED.df, vehicle_state.df),
                dr = COALESCE(EXCLUDED.dr, vehicle_state.dr),
                pf = COALESCE(EXCLUDED.pf, vehicle_state.pf),
                pr = COALESCE(EXCLUDED.pr, vehicle_state.pr),
                fd_window = COALESCE(EXCLUDED.fd_window, vehicle_state.fd_window),
                fp_window = COALESCE(EXCLUDED.fp_window, vehicle_state.fp_window),
                rd_window = COALESCE(EXCLUDED.rd_window, vehicle_state.rd_window),
                rp_window = COALESCE(EXCLUDED.rp_window, vehicle_state.rp_window),
                ft = COALESCE(EXCLUDED.ft, vehicle_state.ft),
                rt = COALESCE(EXCLUDED.rt, vehicle_state.rt),
                is_user_present = COALESCE(EXCLUDED.is_user_present, vehicle_state.is_user_present),
                elevation_m = COALESCE(EXCLUDED.elevation_m, vehicle_state.elevation_m),
                battery_heater_on = COALESCE(EXCLUDED.battery_heater_on, vehicle_state.battery_heater_on),
                is_preconditioning = COALESCE(EXCLUDED.is_preconditioning, vehicle_state.is_preconditioning)",
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
        .bind(obs.charger_voltage)
        .bind(obs.charger_current)
        .bind(obs.charge_current_request)
        .bind(obs.charge_port_door_open)
        .bind(&obs.charge_port_latch)
        .bind(obs.plugged_in)
        .bind(obs.time_to_full_charge_h)
        .bind(obs.scheduled_charging_start_time)
        .bind(obs.tpms_pressure_fl)
        .bind(obs.tpms_pressure_fr)
        .bind(obs.tpms_pressure_rl)
        .bind(obs.tpms_pressure_rr)
        .bind(obs.df)
        .bind(obs.dr)
        .bind(obs.pf)
        .bind(obs.pr)
        .bind(obs.fd_window)
        .bind(obs.fp_window)
        .bind(obs.rd_window)
        .bind(obs.rp_window)
        .bind(obs.ft)
        .bind(obs.rt)
        .bind(obs.is_user_present)
        .bind(obs.elevation_m)
        .bind(obs.battery_heater_on)
        .bind(obs.is_preconditioning)
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
    let mut event = LiveEvent::from_observation(
        runtime.vehicle_id,
        runtime.vin.clone(),
        runtime.name.clone(),
        runtime.model.clone(),
        runtime.tracker.activity.as_str(),
        obs,
        obs.at,
    );
    event.last_poll_at = runtime.last_poll_at;
    event
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

#[cfg(test)]
mod tests {
    use super::LiveEvent;
    use crate::state::{Observation, Presence};
    use crate::status::{at_secs, merge_stream};

    fn frame(
        obs: &Observation,
        last_poll_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> serde_json::Value {
        let mut event = LiveEvent::from_observation(
            1,
            "vehicle".to_owned(),
            None,
            None,
            "driving",
            obs,
            obs.at,
        );
        event.last_poll_at = last_poll_at;
        serde_json::to_value(&event).expect("event json")
    }

    fn polled() -> Observation {
        let mut obs = Observation::at(at_secs(0), Presence::Online);
        obs.inside_temp_c = Some(21.5);
        obs.outside_temp_c = Some(9.0);
        obs.charge_energy_added_kwh = Some(3.5);
        obs.charge_limit_soc = Some(80.0);
        obs.charging_state = Some("Disconnected".to_owned());
        obs.plugged_in = Some(false);
        obs.latitude = Some(48.1);
        obs.longitude = Some(2.2);
        obs
    }

    fn streamed() -> Observation {
        let mut obs = Observation::at(at_secs(5), Presence::Online);
        obs.latitude = Some(48.2);
        obs.longitude = Some(2.3);
        obs.speed_kmh = Some(42.0);
        obs.power_kw = Some(11.0);
        obs.odometer_km = Some(100.4);
        obs.shift_state = Some("D".to_owned());
        obs
    }

    #[test]
    fn stream_frame_keeps_polled_temps_and_does_not_emit_null() {
        let poll = polled();
        let merged = merge_stream(Some(&poll), &streamed());
        let value = frame(&merged, Some(poll.at));
        let text = value.to_string();
        assert_eq!(value["inside_temp_c"], 21.5);
        assert_eq!(value["outside_temp_c"], 9.0);
        assert_eq!(value["charge_energy_added_kwh"], 3.5);
        assert_eq!(value["charge_limit_soc"], 80.0);
        assert_eq!(value["charging_state"], "Disconnected");
        assert_eq!(value["plugged_in"], false);
        assert_eq!(value["latitude"], 48.2);
        assert_eq!(value["speed_kmh"], 42.0);
        assert!(value.get("charger_voltage").is_none(), "{text}");
        assert!(!text.contains("null"), "{text}");
        assert!(!text.contains("5YJ"), "{text}");
    }

    #[test]
    fn stream_frame_without_a_poll_omits_temperature() {
        let value = frame(&streamed(), None);
        let text = value.to_string();
        assert!(value.get("inside_temp_c").is_none(), "{text}");
        assert!(value.get("outside_temp_c").is_none(), "{text}");
        assert!(!text.contains("null"), "{text}");
        assert_eq!(value["speed_kmh"], 42.0);
    }
}
