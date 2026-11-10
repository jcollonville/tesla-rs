// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Pull ingestion from the Fleet API.
//!
//! [`IngestSource`] is the plug-in used by the poller. Fleet Telemetry does not
//! poll: [`crate::telemetry`] pushes decoded records through [`apply_for_vin`],
//! which is the same archive path. Neither path calls `wake_up`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::PgPool;
use tesla_auth::{Session, TokenStore};
use tesla_fleet_api::{
    AccessTokenProvider, FleetClient, FleetError, Vehicle, VehicleData, VehicleDataEndpoint,
    VehicleTag,
};
use tokio::sync::{Mutex, broadcast, watch};
use tokio::time::{Instant, sleep};

use crate::debug::DebugBoard;
use crate::error::{AppError, fleet_error};
use crate::geocode::Geocoder;
use crate::persist::{
    LiveEvent, Runtime, VehicleIdentity, apply_observation, load_runtime, record_error,
    resting_observation, upsert_vehicle,
};
use crate::state::{Activity, Observation, Presence, refresh_plugged_in, sanitize_charger_type};

const LIVE_ENDPOINTS: &[VehicleDataEndpoint] = &[
    VehicleDataEndpoint::ChargeState,
    VehicleDataEndpoint::ClimateState,
    VehicleDataEndpoint::DriveState,
    VehicleDataEndpoint::LocationData,
    VehicleDataEndpoint::VehicleState,
    VehicleDataEndpoint::VehicleConfig,
    VehicleDataEndpoint::GuiSettings,
];

/// How often to call the Fleet API. Live reads happen only while the car is already online.
#[derive(Clone, Debug)]
pub(crate) struct PollPolicy {
    pub drive: Duration,
    pub charge: Duration,
    pub parked: Duration,
    pub asleep: Duration,
    pub backoff: Duration,
    pub parked_live_limit: u32,
    /// Backup drive-position interval used when the Owner stream is not healthy.
    pub gps_fallback: Duration,
}

/// Whether the next cycle should pay for `vehicle_data`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PollPlan {
    pub observe: bool,
    pub wait: Duration,
}

pub(crate) fn poll_plan(
    presence: Presence,
    activity: Activity,
    parked_live_polls: u32,
    telemetry_fresh: bool,
    policy: &PollPolicy,
) -> PollPlan {
    if presence != Presence::Online {
        return PollPlan {
            observe: false,
            wait: policy.asleep,
        };
    }
    if telemetry_fresh {
        return PollPlan {
            observe: false,
            wait: Duration::from_secs(60),
        };
    }
    match activity {
        Activity::Driving => PollPlan {
            observe: true,
            wait: policy.drive,
        },
        Activity::Charging => PollPlan {
            observe: true,
            wait: policy.charge,
        },
        Activity::Updating => PollPlan {
            observe: true,
            wait: policy.parked,
        },
        Activity::Online | Activity::Asleep | Activity::Offline => {
            if parked_live_polls < policy.parked_live_limit {
                PollPlan {
                    observe: true,
                    wait: policy.parked,
                }
            } else {
                PollPlan {
                    observe: false,
                    wait: policy.backoff,
                }
            }
        }
    }
}

/// A pull source. Fleet Telemetry implements the push side separately.
pub(crate) trait IngestSource: Send + Sync {
    /// Account vehicles, without waking them.
    fn discover(&self) -> impl Future<Output = Result<Vec<Summary>, AppError>> + Send;

    /// Live data when the vehicle is already online, otherwise a resting presence.
    fn observe(&self, vin: &str) -> impl Future<Output = Result<ObserveResult, AppError>> + Send;
}

/// List-endpoint summary.
#[derive(Clone, Debug)]
pub(crate) struct Summary {
    pub vin: String,
    pub tesla_id: i64,
    pub legacy_vehicle_id: Option<i64>,
    pub name: Option<String>,
    pub presence: Presence,
}

/// Result of a live read that refused to wake the car.
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum ObserveResult {
    Live(Observation),
    Resting(Presence),
    RateLimited(Duration),
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum CycleControl {
    Wait(Duration),
    Pause(Duration),
}

/// One vehicle's runtime. The map lock is only held to look this up.
pub(crate) type SharedRuntime = Arc<Mutex<Runtime>>;

/// Shared archive handles used by polling and telemetry.
pub(crate) struct Archive {
    pub db: PgPool,
    pub geocoder: Geocoder,
    pub events: broadcast::Sender<LiveEvent>,
    pub runtimes: Arc<Mutex<HashMap<String, SharedRuntime>>>,
    /// Wakes a backing-off Owner API stream when this VIN is seen online.
    pub stream_kick: broadcast::Sender<String>,
    /// Owner-stream GPS mode. Fallback wakes the poller through this board.
    pub gps: crate::position_mode::GpsBoard,
    /// Fleet account that discovers these vehicles. `None` in tests that do not poll.
    pub fleet_account_id: Option<i64>,
    /// Owner-stream account. `None` when streaming is not wired.
    pub owner_account_id: Option<i64>,
    /// 429 pause deadlines keyed by Tesla account id.
    pub account_backoff: Arc<Mutex<HashMap<i64, Instant>>>,
    pub fences: crate::persist::FenceCache,
}

impl Archive {
    pub(crate) fn new(
        db: PgPool,
        geocoder: Geocoder,
        events: broadcast::Sender<LiveEvent>,
        stream_kick: broadcast::Sender<String>,
        gps: crate::position_mode::GpsBoard,
    ) -> Self {
        Self {
            db,
            geocoder,
            events,
            runtimes: Arc::new(Mutex::new(HashMap::new())),
            stream_kick,
            gps,
            fleet_account_id: None,
            owner_account_id: None,
            account_backoff: Arc::new(Mutex::new(HashMap::new())),
            fences: crate::persist::FenceCache::default(),
        }
    }
}

pub(crate) async fn apply_for_vin(
    archive: &Archive,
    vin: &str,
    identity: VehicleIdentity<'_>,
    obs: Observation,
    live: bool,
    from_telemetry: bool,
) -> Result<(), AppError> {
    let vehicle_id = upsert_vehicle(&archive.db, identity, archive.fleet_account_id).await?;
    let slot = ensure_runtime(archive, vin, vehicle_id).await?;
    write_observation(archive, &slot, &obs, live, true, from_telemetry).await
}

/// Writes one Owner API streaming sample onto the current drive.
///
/// The vehicle id is read from the cached runtime. `upsert_vehicle` runs only
/// until that runtime exists. The socket writer uses [`apply_stream_batch`];
/// integration tests call this directly.
#[cfg(test)]
pub(crate) async fn apply_stream_for_vin(
    archive: &Archive,
    vin: &str,
    identity: VehicleIdentity<'_>,
    sample: &crate::persist::StreamFix,
) -> Result<(), AppError> {
    let existing = archive.runtimes.lock().await.get(vin).cloned();
    let slot = if let Some(slot) = existing {
        slot
    } else {
        let vehicle_id = upsert_vehicle(&archive.db, identity, archive.fleet_account_id).await?;
        ensure_runtime(archive, vin, vehicle_id).await?
    };
    write_stream(archive, &slot, std::slice::from_ref(sample)).await
}

/// Writes a batch of stream samples for a vehicle whose row id is already known.
pub(crate) async fn apply_stream_batch(
    archive: &Archive,
    vin: &str,
    vehicle_id: i64,
    samples: &[crate::persist::StreamFix],
) -> Result<(), AppError> {
    let slot = ensure_runtime(archive, vin, vehicle_id).await?;
    write_stream(archive, &slot, samples).await
}

async fn ensure_runtime(
    archive: &Archive,
    vin: &str,
    vehicle_id: i64,
) -> Result<SharedRuntime, AppError> {
    if let Some(slot) = archive.runtimes.lock().await.get(vin).cloned() {
        return Ok(slot);
    }
    let loaded = load_runtime(&archive.db, vehicle_id, vin).await?;
    let mut guard = archive.runtimes.lock().await;
    Ok(Arc::clone(
        guard
            .entry(vin.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(loaded))),
    ))
}

async fn write_observation(
    archive: &Archive,
    slot: &SharedRuntime,
    obs: &Observation,
    live: bool,
    store_position: bool,
    from_telemetry: bool,
) -> Result<(), AppError> {
    let fences = archive.fences.get(&archive.db).await?;
    let jobs = {
        let mut runtime = slot.lock().await;
        if from_telemetry {
            runtime.last_telemetry = Some(Instant::now());
        }
        apply_observation(
            &archive.db,
            &mut runtime,
            obs,
            &fences,
            &archive.events,
            live,
            store_position,
        )
        .await?
    };
    crate::persist::settle_addresses(&archive.db, &archive.geocoder, jobs).await;
    Ok(())
}

async fn write_stream(
    archive: &Archive,
    slot: &SharedRuntime,
    samples: &[crate::persist::StreamFix],
) -> Result<(), AppError> {
    if samples.is_empty() {
        return Ok(());
    }
    let fences = archive.fences.get(&archive.db).await?;
    let jobs = {
        let mut runtime = slot.lock().await;
        crate::persist::apply_stream_samples(
            &archive.db,
            &mut runtime,
            samples,
            &fences,
            &archive.events,
        )
        .await?
    };
    crate::persist::settle_addresses(&archive.db, &archive.geocoder, jobs).await;
    Ok(())
}

/// Fleet API poller. It never calls `wake_up`.
pub(crate) struct FleetPoll<T> {
    client: FleetClient<T>,
    debug: DebugBoard,
}

impl<T: AccessTokenProvider> FleetPoll<T> {
    #[allow(dead_code)]
    pub(crate) fn new(client: FleetClient<T>) -> Self {
        Self::with_debug(client, DebugBoard::new(crate::logs::LogBuffer::new(8)))
    }

    pub(crate) fn with_debug(client: FleetClient<T>, debug: DebugBoard) -> Self {
        Self { client, debug }
    }

    fn note_http(
        &self,
        method: &str,
        path: &str,
        status: Option<u16>,
        started: Instant,
        summary: &str,
    ) {
        self.debug
            .record_poll(method, path, status, started.elapsed(), summary);
    }
}

impl<S> FleetPoll<RefreshingToken<S>>
where
    S: TokenStore + 'static,
{
    pub(crate) fn connect(
        region: tesla_auth::Region,
        session: Arc<Session<S>>,
        debug: DebugBoard,
    ) -> Result<Self, AppError> {
        let client = FleetClient::for_region(
            region,
            RefreshingToken {
                session,
                debug: debug.clone(),
            },
        )
        .map_err(fleet_error)?;
        Ok(Self { client, debug })
    }
}

/// Session wrapper that satisfies [`AccessTokenProvider`].
#[derive(Clone)]
pub(crate) struct RefreshingToken<S> {
    session: Arc<Session<S>>,
    debug: DebugBoard,
}

impl<S: TokenStore> AccessTokenProvider for RefreshingToken<S> {
    async fn access_token(&self) -> Result<secrecy::SecretString, FleetError> {
        match Session::access_token(&self.session).await {
            Ok(token) => Ok(token),
            Err(err) => {
                if !matches!(err, tesla_auth::AuthError::NotAuthenticated) {
                    self.debug.token_failed("fleet", &err.to_string());
                }
                Err(FleetError::from(err))
            }
        }
    }
}

impl<T: AccessTokenProvider> IngestSource for FleetPoll<T> {
    async fn discover(&self) -> Result<Vec<Summary>, AppError> {
        let started = Instant::now();
        match self.client.list_all_vehicles(100, 20).await {
            Ok(vehicles) => {
                let count = vehicles.len();
                self.note_http(
                    "GET",
                    "/api/1/vehicles",
                    Some(200),
                    started,
                    &format!("vehicles={count}"),
                );
                Ok(vehicles.into_iter().map(summary_from_vehicle).collect())
            }
            Err(err) => {
                self.note_http(
                    "GET",
                    "/api/1/vehicles",
                    fleet_http_status(&err),
                    started,
                    &err.to_string(),
                );
                Err(fleet_error(err))
            }
        }
    }

    async fn observe(&self, vin: &str) -> Result<ObserveResult, AppError> {
        let tag = VehicleTag::new(vin).map_err(|_| AppError::message("vehicle vin is invalid"))?;
        let started = Instant::now();
        let vehicle = match self.client.vehicle(&tag).await {
            Ok(vehicle) => {
                let state = vehicle
                    .state
                    .clone()
                    .unwrap_or_else(|| "unknown".to_owned());
                self.note_http(
                    "GET",
                    "/api/1/vehicles/{id}",
                    Some(200),
                    started,
                    &format!("state={state}"),
                );
                vehicle
            }
            Err(err) if err.is_device_unavailable() => {
                self.note_http(
                    "GET",
                    "/api/1/vehicles/{id}",
                    fleet_http_status(&err),
                    started,
                    "vehicle unavailable",
                );
                return Ok(ObserveResult::Resting(Presence::Asleep));
            }
            Err(err) => {
                self.note_http(
                    "GET",
                    "/api/1/vehicles/{id}",
                    fleet_http_status(&err),
                    started,
                    &err.to_string(),
                );
                return map_rate(err);
            }
        };
        if !vehicle.is_online() || vehicle.in_service == Some(true) {
            return Ok(ObserveResult::Resting(Presence::from_fleet(
                vehicle.state.as_deref(),
            )));
        }
        let started = Instant::now();
        let data = match self.client.vehicle_data(&tag, LIVE_ENDPOINTS).await {
            Ok(data) => {
                let drive = if data.drive_state.is_some() {
                    "yes"
                } else {
                    "no"
                };
                self.note_http(
                    "GET",
                    "/api/1/vehicles/{id}/vehicle_data",
                    Some(200),
                    started,
                    &format!("drive_state={drive}"),
                );
                data
            }
            Err(err) if err.is_device_unavailable() => {
                self.note_http(
                    "GET",
                    "/api/1/vehicles/{id}/vehicle_data",
                    fleet_http_status(&err),
                    started,
                    "vehicle unavailable",
                );
                return Ok(ObserveResult::Resting(Presence::Asleep));
            }
            Err(err) => {
                self.note_http(
                    "GET",
                    "/api/1/vehicles/{id}/vehicle_data",
                    fleet_http_status(&err),
                    started,
                    &err.to_string(),
                );
                return map_rate(err);
            }
        };
        Ok(ObserveResult::Live(observation_from_vehicle_data(data)))
    }
}

fn fleet_http_status(err: &FleetError) -> Option<u16> {
    match err {
        FleetError::Unauthorized { .. } => Some(401),
        FleetError::PaymentRequired => Some(402),
        FleetError::Forbidden { .. } => Some(403),
        FleetError::NotFound { .. } => Some(404),
        FleetError::DeviceUnavailable { .. } => Some(408),
        FleetError::PreconditionFailed { .. } => Some(412),
        FleetError::IncorrectRegion { .. } => Some(421),
        FleetError::InvalidResource { .. } => Some(422),
        FleetError::Locked => Some(423),
        FleetError::RateLimited(_) => Some(429),
        FleetError::UnavailableForLegalReasons { .. } => Some(451),
        FleetError::Api { status, .. } => Some(*status),
        _ => None,
    }
}

fn map_rate(err: FleetError) -> Result<ObserveResult, AppError> {
    match fleet_error(err) {
        AppError::RateLimited { retry_after } => Ok(ObserveResult::RateLimited(retry_after)),
        other => Err(other),
    }
}

fn summary_from_vehicle(vehicle: Vehicle) -> Summary {
    let presence = if vehicle.in_service == Some(true) {
        Presence::Offline
    } else {
        Presence::from_fleet(vehicle.state.as_deref())
    };
    Summary {
        vin: vehicle.vin,
        tesla_id: vehicle.id,
        legacy_vehicle_id: vehicle.vehicle_id,
        name: vehicle.display_name,
        presence,
    }
}

pub(crate) fn observation_from_vehicle_data(data: VehicleData) -> Observation {
    use crate::units::{miles_to_km, speed_to_kmh};
    let units = data
        .gui_settings
        .as_ref()
        .and_then(|gui| gui.gui_distance_units.clone());
    let mut obs = Observation::at(Utc::now(), Presence::Online);
    obs.distance_unit.clone_from(&units);
    if let Some(charge) = data.charge_state {
        obs.battery_level = charge.battery_level;
        obs.usable_battery_level = charge.usable_battery_level;
        obs.rated_range_km = charge.battery_range.and_then(miles_to_km);
        obs.ideal_range_km = charge.ideal_battery_range.and_then(miles_to_km);
        obs.est_range_km = charge.est_battery_range.and_then(miles_to_km);
        obs.charging_state = charge.charging_state;
        obs.charge_energy_added_kwh = charge.charge_energy_added;
        obs.charger_power_kw = charge.charger_power;
        obs.charger_voltage = finite(charge.charger_voltage);
        obs.charger_current = finite(charge.charger_actual_current);
        obs.charge_current_request = finite(charge.charge_current_request);
        obs.fast_charger = charge.fast_charger_present;
        obs.fast_charger_type = sanitize_charger_type(charge.fast_charger_type.as_deref());
        obs.charge_limit_soc = charge.charge_limit_soc;
        obs.charge_port_door_open = charge.charge_port_door_open;
        obs.charge_port_latch = charge.charge_port_latch;
        obs.conn_charge_cable = charge.conn_charge_cable;
        obs.time_to_full_charge_h = finite(charge.time_to_full_charge)
            .or_else(|| finite(charge.minutes_to_full_charge).map(|minutes| minutes / 60.0));
        obs.scheduled_charging_start_time =
            minutes_after_midnight(charge.scheduled_charging_start_time);
        obs.battery_heater_on = json_bool(charge.additional.get("battery_heater_on"));
    }
    if let Some(climate) = data.climate_state {
        obs.inside_temp_c = climate.inside_temp;
        obs.outside_temp_c = climate.outside_temp;
        obs.climate_on = climate.is_climate_on;
        obs.is_preconditioning = climate.is_preconditioning;
        obs.battery_heater_on = climate
            .battery_heater
            .or(obs.battery_heater_on)
            .or_else(|| json_bool(climate.additional.get("battery_heater_on")));
    }
    if let Some(drive) = data.drive_state {
        obs.latitude = drive.latitude;
        obs.longitude = drive.longitude;
        obs.heading_deg = drive.heading;
        obs.speed_kmh = drive
            .speed
            .and_then(|speed| speed_to_kmh(speed, units.as_deref()));
        obs.power_kw = drive.power;
        obs.shift_state = drive.shift_state;
        obs.elevation_m = json_number(drive.additional.get("elevation"))
            .or_else(|| json_number(drive.additional.get("gps_elevation")));
    }
    if let Some(state) = data.vehicle_state {
        obs.odometer_km = state.odometer.and_then(miles_to_km);
        obs.locked = state.locked;
        obs.sentry_mode = state.sentry_mode;
        obs.software_version = state.car_version;
        obs.is_user_present = state.is_user_present;
        obs.df = closure_code(state.df);
        obs.dr = closure_code(state.dr);
        obs.pf = closure_code(state.pf);
        obs.pr = closure_code(state.pr);
        obs.ft = closure_code(state.ft);
        obs.rt = closure_code(state.rt);
        obs.fd_window = closure_code(state.fd_window);
        obs.fp_window = closure_code(state.fp_window);
        obs.rd_window = closure_code(state.rd_window);
        obs.rp_window = closure_code(state.rp_window);
        obs.tpms_pressure_fl = bar(state.tpms_pressure_fl);
        obs.tpms_pressure_fr = bar(state.tpms_pressure_fr);
        obs.tpms_pressure_rl = bar(state.tpms_pressure_rl);
        obs.tpms_pressure_rr = bar(state.tpms_pressure_rr);
        if let Some(update) = state.software_update {
            obs.update_status = update.status;
        }
    }
    if let Some(config) = data.vehicle_config {
        obs.model = config.car_type;
        obs.trim_badging = config.trim_badging;
        obs.exterior_color = config.exterior_color;
        obs.wheel_type = config.wheel_type;
    }
    refresh_plugged_in(&mut obs);
    obs
}

fn finite(value: Option<f64>) -> Option<f64> {
    value.filter(|number| number.is_finite())
}

fn bar(value: Option<f64>) -> Option<f64> {
    finite(value).filter(|pressure| *pressure >= 0.0)
}

fn closure_code(value: Option<i64>) -> Option<i32> {
    i32::try_from(value?).ok()
}

fn minutes_after_midnight(value: Option<f64>) -> Option<i32> {
    let value = finite(value)?;
    crate::units::minutes_after_midnight(value)
}

fn json_bool(value: Option<&serde_json::Value>) -> Option<bool> {
    match value? {
        serde_json::Value::Bool(flag) => Some(*flag),
        serde_json::Value::Number(number) => number.as_i64().map(|number| number != 0),
        serde_json::Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "on" | "yes" => Some(true),
            "false" | "0" | "off" | "no" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn json_number(value: Option<&serde_json::Value>) -> Option<f64> {
    let value = value?;
    value
        .as_f64()
        .filter(|number| number.is_finite())
        .or_else(|| {
            value.as_i64().and_then(|number| {
                let number = i32::try_from(number).ok()?;
                Some(f64::from(number))
            })
        })
}

pub(crate) async fn run_poll<T: AccessTokenProvider>(
    source: &FleetPoll<T>,
    archive: Arc<Archive>,
    policy: PollPolicy,
    account_id: Option<i64>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            break;
        }
        let control = run_cycle(source, &archive, &policy, account_id).await;
        let wait = match control {
            Ok(CycleControl::Wait(wait) | CycleControl::Pause(wait)) => wait,
            Err(err) => {
                let message = crate::logs::redact_line(&err.to_string(), &[]);
                tracing::warn!(error = %message, "ingest cycle failed");
                if let Some(account_id) = account_id {
                    let _ =
                        crate::accounts::set_last_error(&archive.db, account_id, &message).await;
                }
                Duration::from_secs(30)
            }
        };
        let wake = archive.gps.notified();
        tokio::pin!(wake);
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            () = sleep(wait) => {}
            () = wake => {}
        }
    }
}

async fn run_cycle(
    source: &impl IngestSource,
    archive: &Archive,
    policy: &PollPolicy,
    account_id: Option<i64>,
) -> Result<CycleControl, AppError> {
    if let Some(account_id) = account_id
        && let Some(wait) = account_pause(archive, account_id).await
    {
        return Ok(CycleControl::Pause(wait));
    }
    let vehicles = match source.discover().await {
        Ok(vehicles) => vehicles,
        Err(AppError::RateLimited { retry_after }) => {
            note_rate_limit(archive, account_id, retry_after).await;
            return Ok(CycleControl::Pause(retry_after));
        }
        Err(err) => return Err(err),
    };
    if vehicles.is_empty() {
        if let Some(account_id) = account_id {
            let _ = crate::accounts::set_last_error(
                &archive.db,
                account_id,
                "no vehicles on the Tesla account",
            )
            .await;
        }
        return Ok(CycleControl::Wait(policy.asleep));
    }
    if let Some(account_id) = account_id {
        let _ = crate::accounts::clear_last_error(&archive.db, account_id).await;
    }
    let mut soonest = policy.backoff;
    let now = Instant::now();
    for summary in vehicles {
        let slot = archive.runtimes.lock().await.get(&summary.vin).cloned();
        let due = match &slot {
            None => true,
            Some(slot) => slot.lock().await.next_due <= now,
        };
        if !due {
            if let Some(slot) = slot {
                let remaining = slot.lock().await.next_due.saturating_duration_since(now);
                if remaining < soonest {
                    soonest = remaining;
                }
            }
            continue;
        }
        match handle_summary(source, archive, policy, &summary).await {
            Ok(CycleControl::Pause(wait)) => {
                note_rate_limit(archive, account_id, wait).await;
                return Ok(CycleControl::Pause(wait));
            }
            Ok(CycleControl::Wait(wait)) => {
                if wait < soonest {
                    soonest = wait;
                }
            }
            Err(err) => {
                let message = crate::logs::redact_line(&err.to_string(), &[]);
                tracing::warn!(error = %message, "vehicle poll failed");
                if let Some(vehicle_id) = vehicle_id_by_vin(&archive.db, &summary.vin).await {
                    let _ = record_error(&archive.db, vehicle_id, &message).await;
                } else if let Some(account_id) = account_id {
                    let _ =
                        crate::accounts::set_last_error(&archive.db, account_id, &message).await;
                }
                let slot = archive.runtimes.lock().await.get(&summary.vin).cloned();
                if let Some(slot) = slot {
                    slot.lock().await.next_due = Instant::now() + policy.backoff;
                }
                if policy.backoff < soonest {
                    soonest = policy.backoff;
                }
            }
        }
    }
    Ok(CycleControl::Wait(soonest.max(Duration::from_secs(1))))
}

async fn note_rate_limit(archive: &Archive, account_id: Option<i64>, wait: Duration) {
    let Some(account_id) = account_id else {
        return;
    };
    note_account_pause(archive, account_id, wait).await;
    let _ = crate::accounts::set_last_error(
        &archive.db,
        account_id,
        &format!("rate limited; retry after {}s", wait.as_secs()),
    )
    .await;
}

pub(crate) async fn account_pause(archive: &Archive, account_id: i64) -> Option<Duration> {
    let guard = archive.account_backoff.lock().await;
    let until = *guard.get(&account_id)?;
    let now = Instant::now();
    if until <= now {
        None
    } else {
        Some(until.saturating_duration_since(now))
    }
}

pub(crate) async fn note_account_pause(archive: &Archive, account_id: i64, wait: Duration) {
    let until = Instant::now() + wait;
    let mut guard = archive.account_backoff.lock().await;
    guard
        .entry(account_id)
        .and_modify(|existing| {
            if *existing < until {
                *existing = until;
            }
        })
        .or_insert(until);
}

async fn vehicle_id_by_vin(pool: &PgPool, vin: &str) -> Option<i64> {
    sqlx::query_as::<_, (i64,)>("SELECT id FROM vehicles WHERE vin = $1")
        .bind(vin)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .map(|row| row.0)
}

async fn handle_summary(
    source: &impl IngestSource,
    archive: &Archive,
    policy: &PollPolicy,
    summary: &Summary,
) -> Result<CycleControl, AppError> {
    if summary.presence == Presence::Online {
        let _ = archive.stream_kick.send(summary.vin.clone());
    }
    let vehicle_id = upsert_vehicle(
        &archive.db,
        VehicleIdentity {
            vin: &summary.vin,
            tesla_id: Some(summary.tesla_id),
            legacy_vehicle_id: summary.legacy_vehicle_id,
            name: summary.name.as_deref(),
        },
        archive.fleet_account_id,
    )
    .await?;
    let slot = ensure_runtime(archive, &summary.vin, vehicle_id).await?;
    let (activity, telemetry_fresh, parked_live_polls) = {
        let runtime = slot.lock().await;
        (
            runtime.tracker.activity,
            runtime
                .last_telemetry
                .is_some_and(|at| at.elapsed() < Duration::from_secs(120)),
            runtime.parked_live_polls,
        )
    };
    // Owner stream health is not `telemetry_fresh`. A live socket must not
    // suspend `vehicle_data`; that flag is only Fleet Telemetry.
    let mode = archive
        .gps
        .reconcile(vehicle_id, summary.presence, activity)
        .await
        .mode;
    let mut plan = poll_plan(
        summary.presence,
        activity,
        parked_live_polls,
        telemetry_fresh,
        policy,
    );
    if crate::position_mode::fallback_observe(mode, summary.presence, telemetry_fresh, plan.observe)
    {
        plan.observe = true;
        plan.wait = policy.gps_fallback;
    }
    if summary.presence != Presence::Online || !plan.observe {
        {
            let mut runtime = slot.lock().await;
            runtime.parked_live_polls = 0;
            runtime.next_due = Instant::now() + plan.wait;
        }
        let vin = summary.vin.clone();
        let presence = summary.presence;
        let obs = resting_observation(Utc::now(), presence);
        if let Err(err) = apply_existing(archive, &vin, obs, false, false).await {
            tracing::warn!(
                vehicle_id = summary.tesla_id,
                error = %crate::logs::redact_line(&err.to_string(), &[]),
                "could not record resting state"
            );
            let _ = record_error(&archive.db, vehicle_id, &err.to_string()).await;
        }
        return Ok(CycleControl::Wait(plan.wait));
    }
    match source.observe(&summary.vin).await? {
        ObserveResult::RateLimited(wait) => Ok(CycleControl::Pause(wait)),
        ObserveResult::Resting(presence) => {
            {
                let mut runtime = slot.lock().await;
                runtime.parked_live_polls = 0;
                runtime.next_due = Instant::now() + policy.asleep;
            }
            let obs = resting_observation(Utc::now(), presence);
            apply_existing(archive, &summary.vin, obs, false, false).await?;
            Ok(CycleControl::Wait(policy.asleep))
        }
        ObserveResult::Live(obs) => {
            let driving = crate::position_mode::sample_is_driving(
                activity,
                obs.shift_state.as_deref(),
                obs.speed_kmh,
            );
            let now = std::time::Instant::now();
            let store_position = archive
                .gps
                .allow_poll_position(vehicle_id, driving, now, policy.gps_fallback)
                .await;
            apply_existing(archive, &summary.vin, obs, true, store_position).await?;
            if store_position && driving {
                archive.gps.mark_drive_position(vehicle_id, now).await;
            }
            {
                let mut runtime = slot.lock().await;
                if matches!(
                    runtime.tracker.activity,
                    Activity::Driving | Activity::Charging
                ) {
                    runtime.parked_live_polls = 0;
                } else {
                    runtime.parked_live_polls = runtime.parked_live_polls.saturating_add(1);
                }
                runtime.next_due = Instant::now() + plan.wait;
            }
            Ok(CycleControl::Wait(plan.wait))
        }
    }
}

async fn apply_existing(
    archive: &Archive,
    vin: &str,
    obs: Observation,
    live: bool,
    store_position: bool,
) -> Result<(), AppError> {
    let Some(slot) = archive.runtimes.lock().await.get(vin).cloned() else {
        return Ok(());
    };
    write_observation(archive, &slot, &obs, live, store_position, false).await
}

#[cfg(test)]
mod tests {
    use super::{Activity, FleetPoll, IngestSource, PollPolicy, Presence, poll_plan};
    use std::time::Duration;
    use tesla_fleet_api::{BearerToken, FleetClient};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn policy() -> PollPolicy {
        PollPolicy {
            drive: Duration::from_secs(15),
            charge: Duration::from_secs(60),
            parked: Duration::from_secs(180),
            asleep: Duration::from_secs(300),
            backoff: Duration::from_secs(720),
            parked_live_limit: 2,
            gps_fallback: Duration::from_secs(300),
        }
    }

    #[test]
    fn asleep_and_backoff_do_not_request_live_data() {
        let policy = policy();
        let asleep = poll_plan(Presence::Asleep, Activity::Asleep, 0, false, &policy);
        assert!(!asleep.observe);
        assert_eq!(asleep.wait, Duration::from_secs(300));
        let early = poll_plan(Presence::Online, Activity::Online, 0, false, &policy);
        assert!(early.observe);
        let later = poll_plan(Presence::Online, Activity::Online, 2, false, &policy);
        assert!(!later.observe);
        assert_eq!(later.wait, Duration::from_secs(720));
        let driving = poll_plan(Presence::Online, Activity::Driving, 9, false, &policy);
        assert!(driving.observe);
        assert_eq!(driving.wait, Duration::from_secs(15));
        let streamed = poll_plan(Presence::Online, Activity::Driving, 0, true, &policy);
        assert!(!streamed.observe);
    }

    #[test]
    fn owner_stream_does_not_suspend_vehicle_data() {
        let policy = policy();
        let driving = poll_plan(Presence::Online, Activity::Driving, 0, false, &policy);
        assert!(driving.observe);
        assert_eq!(driving.wait, policy.drive);
        let charging = poll_plan(Presence::Online, Activity::Charging, 0, false, &policy);
        assert!(charging.observe);
    }

    #[tokio::test]
    async fn asleep_list_does_not_call_wake_or_vehicle_data() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/1/vehicles"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"response":[{"id":1,"vin":"TEST00000000VIN01","state":"asleep","display_name":"Nap"}],"pagination":{"next":null}}"#,
            ))
            .mount(&server)
            .await;
        let client = FleetClient::builder(&server.uri(), BearerToken::new("token"))
            .unwrap()
            .build()
            .unwrap();
        let found = FleetPoll::new(client).discover().await.unwrap();
        assert_eq!(found[0].presence, Presence::Asleep);
        let requests = server.received_requests().await.unwrap();
        assert!(requests.iter().all(|request| {
            let path = request.url.path();
            !path.contains("wake_up") && !path.contains("vehicle_data")
        }));
    }

    #[tokio::test]
    async fn observe_stops_when_the_vehicle_endpoint_says_asleep() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/1/vehicles/TEST00000000VIN01"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"response":{"id":1,"vin":"TEST00000000VIN01","state":"asleep"}}"#,
            ))
            .mount(&server)
            .await;
        let client = FleetClient::builder(&server.uri(), BearerToken::new("token"))
            .unwrap()
            .build()
            .unwrap();
        let observed = FleetPoll::new(client)
            .observe("TEST00000000VIN01")
            .await
            .unwrap();
        assert!(matches!(
            observed,
            super::ObserveResult::Resting(Presence::Asleep)
        ));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].url.path().contains("wake_up"));
        assert!(!requests[0].url.path().contains("vehicle_data"));
    }

    #[tokio::test]
    async fn vehicle_data_poll_is_logged_without_the_vin_or_body() {
        let vin = "TEST00000000VIN01";
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!("/api/1/vehicles/{vin}")))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"response":{{"id":1,"vehicle_id":99,"vin":"{vin}","state":"online"}}}}"#
            )))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/1/vehicles/{vin}/vehicle_data")))
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"response":{{"id":1,"vin":"{vin}","drive_state":{{"shift_state":"D","latitude":48.1,"longitude":2.3}}}}}}"#
            )))
            .mount(&server)
            .await;
        let client = FleetClient::builder(&server.uri(), BearerToken::new("fleet-secret-token"))
            .unwrap()
            .build()
            .unwrap();
        let debug = crate::debug::DebugBoard::new(crate::logs::LogBuffer::new(8));
        let observed = FleetPoll::with_debug(client, debug.clone())
            .observe(vin)
            .await
            .unwrap();
        assert!(matches!(observed, super::ObserveResult::Live(_)));
        let text = debug.joined();
        assert!(
            text.contains("GET /api/1/vehicles/{id}/vehicle_data"),
            "{text}"
        );
        assert!(text.contains("status=200"), "{text}");
        assert!(text.contains("drive_state=yes"), "{text}");
        assert!(!text.contains(vin), "{text}");
        assert!(!text.contains("fleet-secret-token"), "{text}");
        assert!(!text.contains("latitude"), "{text}");
    }

    #[test]
    fn vehicle_data_maps_home_assistant_fields_in_metric_units() {
        let data: tesla_fleet_api::VehicleData = serde_json::from_str(
            r#"{
                "id": 1,
                "vin": "TEST00000000VIN01",
                "gui_settings": {"gui_distance_units": "mi/hr"},
                "charge_state": {
                    "battery_level": 80,
                    "battery_range": 100,
                    "charger_voltage": 240,
                    "charger_actual_current": 32,
                    "charge_current_request": 16,
                    "charging_state": "Charging",
                    "charge_port_door_open": true,
                    "charge_port_latch": "Engaged",
                    "conn_charge_cable": "IEC",
                    "time_to_full_charge": 1.5,
                    "scheduled_charging_start_time": 1320,
                    "charger_power": 7.6
                },
                "climate_state": {
                    "inside_temp": 21.5,
                    "outside_temp": 9,
                    "is_climate_on": true,
                    "is_preconditioning": true,
                    "battery_heater": true
                },
                "drive_state": {
                    "latitude": 48.85,
                    "longitude": 2.35,
                    "heading": 90,
                    "speed": 10,
                    "power": -5,
                    "shift_state": "D",
                    "elevation": 35
                },
                "vehicle_state": {
                    "odometer": 10,
                    "locked": false,
                    "sentry_mode": true,
                    "is_user_present": true,
                    "car_version": "2026.4",
                    "df": 1,
                    "dr": 0,
                    "pf": 0,
                    "pr": 0,
                    "ft": 0,
                    "rt": 1,
                    "fd_window": 0,
                    "fp_window": 1,
                    "rd_window": 0,
                    "rp_window": 0,
                    "tpms_pressure_fl": 2.9,
                    "tpms_pressure_fr": 2.8,
                    "tpms_pressure_rl": 2.7,
                    "tpms_pressure_rr": 2.6
                }
            }"#,
        )
        .unwrap();
        let obs = super::observation_from_vehicle_data(data);
        assert!((obs.rated_range_km.unwrap() - 160.9344).abs() < 1e-3);
        assert!((obs.odometer_km.unwrap() - 16.09344).abs() < 1e-3);
        assert!((obs.speed_kmh.unwrap() - 16.09344).abs() < 1e-3);
        assert_eq!(obs.charger_voltage, Some(240.0));
        assert_eq!(obs.charger_current, Some(32.0));
        assert_eq!(obs.charge_current_request, Some(16.0));
        assert_eq!(obs.time_to_full_charge_h, Some(1.5));
        assert_eq!(obs.scheduled_charging_start_time, Some(1320));
        assert_eq!(obs.plugged_in, Some(true));
        assert_eq!(obs.charge_port_door_open, Some(true));
        assert_eq!(obs.charge_port_latch.as_deref(), Some("Engaged"));
        assert_eq!(obs.inside_temp_c, Some(21.5));
        assert_eq!(obs.battery_heater_on, Some(true));
        assert_eq!(obs.is_preconditioning, Some(true));
        assert_eq!(obs.elevation_m, Some(35.0));
        assert_eq!(obs.power_kw, Some(-5.0));
        assert_eq!(obs.df, Some(1));
        assert_eq!(obs.dr, Some(0));
        assert_eq!(obs.rt, Some(1));
        assert_eq!(obs.fd_window, Some(0));
        assert_eq!(obs.fp_window, Some(1));
        assert_eq!(obs.tpms_pressure_fl, Some(2.9));
        assert_eq!(obs.is_user_present, Some(true));
        assert_eq!(obs.locked, Some(false));
        assert_eq!(obs.software_version.as_deref(), Some("2026.4"));
    }

    #[test]
    fn minutes_to_full_charge_becomes_hours() {
        let data: tesla_fleet_api::VehicleData = serde_json::from_str(
            r#"{"id":1,"vin":"V","charge_state":{"minutes_to_full_charge":90,"charging_state":"Disconnected","charge_port_latch":"Disengaged","conn_charge_cable":"<invalid>"}}"#,
        )
        .unwrap();
        let obs = super::observation_from_vehicle_data(data);
        assert_eq!(obs.time_to_full_charge_h, Some(1.5));
        assert_eq!(obs.plugged_in, Some(false));
    }

    #[test]
    fn invalid_fast_charger_type_is_not_stored() {
        let invalid: tesla_fleet_api::VehicleData = serde_json::from_str(
            r#"{"id":1,"vin":"V","charge_state":{"fast_charger_type":"<invalid>","charging_state":"Charging"}}"#,
        )
        .unwrap();
        assert_eq!(
            super::observation_from_vehicle_data(invalid).fast_charger_type,
            None
        );

        let combo: tesla_fleet_api::VehicleData = serde_json::from_str(
            r#"{"id":1,"vin":"V","charge_state":{"fast_charger_type":" Combo ","fast_charger_present":true}}"#,
        )
        .unwrap();
        assert_eq!(
            super::observation_from_vehicle_data(combo)
                .fast_charger_type
                .as_deref(),
            Some("Combo")
        );
    }

    struct SplitSource {
        fail_vin: &'static str,
        ok_vin: &'static str,
    }

    impl IngestSource for SplitSource {
        async fn discover(&self) -> Result<Vec<super::Summary>, crate::error::AppError> {
            std::future::ready(Ok(vec![summary(self.fail_vin, 1), summary(self.ok_vin, 2)])).await
        }

        async fn observe(&self, vin: &str) -> Result<super::ObserveResult, crate::error::AppError> {
            let result = if vin == self.fail_vin {
                Err(crate::error::AppError::message("vehicle data failed"))
            } else {
                Ok(super::ObserveResult::Resting(Presence::Asleep))
            };
            std::future::ready(result).await
        }
    }

    fn summary(vin: &'static str, tesla_id: i64) -> super::Summary {
        super::Summary {
            vin: vin.to_owned(),
            tesla_id,
            legacy_vehicle_id: None,
            name: Some(vin.to_owned()),
            presence: Presence::Online,
        }
    }

    #[tokio::test]
    async fn one_vehicle_error_does_not_fail_the_cycle() {
        let Some(db) = crate::test_util::TestDb::migrated("isolate").await else {
            return;
        };
        let archive = super::Archive::new(
            db.pool.clone(),
            crate::geocode::Geocoder::disabled(),
            tokio::sync::broadcast::channel(4).0,
            tokio::sync::broadcast::channel(4).0,
            crate::position_mode::GpsBoard::default(),
        );
        let fail_vin = "5YJ3E1EA7KF000001";
        let ok_vin = "5YJ3E1EA7KF000002";
        let control = super::run_cycle(
            &SplitSource { fail_vin, ok_vin },
            &archive,
            &policy(),
            Some(1),
        )
        .await
        .expect("cycle stays up");
        assert!(matches!(control, super::CycleControl::Wait(_)));
        let failed: (Option<String>,) = sqlx::query_as(
            "SELECT s.last_error FROM vehicle_state s
                 JOIN vehicles v ON v.id = s.vehicle_id WHERE v.vin = $1",
        )
        .bind(fail_vin)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(failed.0.as_deref(), Some("vehicle data failed"));
        let ok: (Option<String>,) = sqlx::query_as(
            "SELECT s.last_error FROM vehicle_state s
             JOIN vehicles v ON v.id = s.vehicle_id WHERE v.vin = $1",
        )
        .bind(ok_vin)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(ok.0, None);
    }

    #[tokio::test]
    async fn rate_limit_backoff_is_per_account() {
        let archive = super::Archive::new(
            sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://localhost/unused")
                .unwrap(),
            crate::geocode::Geocoder::disabled(),
            tokio::sync::broadcast::channel(1).0,
            tokio::sync::broadcast::channel(1).0,
            crate::position_mode::GpsBoard::default(),
        );
        super::note_account_pause(&archive, 7, Duration::from_secs(40)).await;
        assert!(super::account_pause(&archive, 7).await.is_some());
        assert!(super::account_pause(&archive, 8).await.is_none());
    }
}
