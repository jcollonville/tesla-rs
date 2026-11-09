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

use crate::error::{AppError, fleet_error};
use crate::geocode::Geocoder;
use crate::persist::{
    LiveEvent, Runtime, VehicleIdentity, apply_observation, load_runtime, record_error,
    resting_observation, upsert_vehicle,
};
use crate::state::{Activity, Observation, Presence};

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

/// Shared archive handles used by polling and telemetry.
pub(crate) struct Archive {
    pub db: PgPool,
    pub geocoder: Geocoder,
    pub events: broadcast::Sender<LiveEvent>,
    pub runtimes: Arc<Mutex<HashMap<String, Runtime>>>,
}

pub(crate) async fn apply_for_vin(
    archive: &Archive,
    vin: &str,
    identity: VehicleIdentity<'_>,
    obs: Observation,
    live: bool,
    from_telemetry: bool,
) -> Result<(), AppError> {
    let vehicle_id = upsert_vehicle(&archive.db, identity).await?;
    let mut guard = archive.runtimes.lock().await;
    if !guard.contains_key(vin) {
        let runtime = load_runtime(&archive.db, vehicle_id, vin).await?;
        guard.insert(vin.to_owned(), runtime);
    }
    let runtime = guard.get_mut(vin).expect("runtime inserted");
    if from_telemetry {
        runtime.last_telemetry = Some(Instant::now());
    }
    apply_observation(
        &archive.db,
        runtime,
        &obs,
        &archive.geocoder,
        &archive.events,
        live,
    )
    .await
}

/// Fleet API poller. It never calls `wake_up`.
pub(crate) struct FleetPoll<T> {
    client: FleetClient<T>,
}

impl<T: AccessTokenProvider> FleetPoll<T> {
    #[allow(dead_code)]
    pub(crate) fn new(client: FleetClient<T>) -> Self {
        Self { client }
    }
}

impl<S> FleetPoll<RefreshingToken<S>>
where
    S: TokenStore + 'static,
{
    pub(crate) fn connect(
        region: tesla_auth::Region,
        session: Arc<Session<S>>,
    ) -> Result<Self, AppError> {
        let client =
            FleetClient::for_region(region, RefreshingToken(session)).map_err(fleet_error)?;
        Ok(Self { client })
    }
}

/// Session wrapper that satisfies [`AccessTokenProvider`].
#[derive(Clone)]
pub(crate) struct RefreshingToken<S>(pub Arc<Session<S>>);

impl<S: TokenStore> AccessTokenProvider for RefreshingToken<S> {
    async fn access_token(&self) -> Result<secrecy::SecretString, FleetError> {
        Session::access_token(&self.0)
            .await
            .map_err(FleetError::from)
    }
}

impl<T: AccessTokenProvider> IngestSource for FleetPoll<T> {
    async fn discover(&self) -> Result<Vec<Summary>, AppError> {
        let vehicles = self
            .client
            .list_all_vehicles(100, 20)
            .await
            .map_err(fleet_error)?;
        Ok(vehicles.into_iter().map(summary_from_vehicle).collect())
    }

    async fn observe(&self, vin: &str) -> Result<ObserveResult, AppError> {
        let tag = VehicleTag::new(vin).map_err(|_| AppError::message("vehicle vin is invalid"))?;
        let vehicle = match self.client.vehicle(&tag).await {
            Ok(vehicle) => vehicle,
            Err(err) if err.is_device_unavailable() => {
                return Ok(ObserveResult::Resting(Presence::Asleep));
            }
            Err(err) => return map_rate(err),
        };
        if !vehicle.is_online() || vehicle.in_service == Some(true) {
            return Ok(ObserveResult::Resting(Presence::from_fleet(
                vehicle.state.as_deref(),
            )));
        }
        let data = match self.client.vehicle_data(&tag, LIVE_ENDPOINTS).await {
            Ok(data) => data,
            Err(err) if err.is_device_unavailable() => {
                return Ok(ObserveResult::Resting(Presence::Asleep));
            }
            Err(err) => return map_rate(err),
        };
        Ok(ObserveResult::Live(observation_from_vehicle_data(data)))
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
        obs.charger_voltage = charge.charger_voltage;
        obs.charger_current = charge.charger_actual_current;
        obs.fast_charger = charge.fast_charger_present;
        obs.fast_charger_type = charge.fast_charger_type;
        obs.charge_limit_soc = charge.charge_limit_soc;
    }
    if let Some(climate) = data.climate_state {
        obs.inside_temp_c = climate.inside_temp;
        obs.outside_temp_c = climate.outside_temp;
        obs.climate_on = climate.is_climate_on;
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
    obs
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
    global_error: Arc<Mutex<Option<String>>>,
    mut shutdown: watch::Receiver<bool>,
) {
    loop {
        if *shutdown.borrow() {
            break;
        }
        let control = run_cycle(source, &archive, &policy, &global_error).await;
        let wait = match control {
            Ok(CycleControl::Wait(wait) | CycleControl::Pause(wait)) => wait,
            Err(err) => {
                tracing::warn!(error = %err, "ingest cycle failed");
                set_error(&global_error, err.to_string()).await;
                Duration::from_secs(30)
            }
        };
        tokio::select! {
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
            () = sleep(wait) => {}
        }
    }
}

async fn run_cycle(
    source: &impl IngestSource,
    archive: &Archive,
    policy: &PollPolicy,
    global_error: &Mutex<Option<String>>,
) -> Result<CycleControl, AppError> {
    let vehicles = match source.discover().await {
        Ok(vehicles) => vehicles,
        Err(AppError::RateLimited { retry_after }) => return Ok(CycleControl::Pause(retry_after)),
        Err(err) => return Err(err),
    };
    if vehicles.is_empty() {
        set_error(global_error, "no vehicles on the Tesla account".to_owned()).await;
        return Ok(CycleControl::Wait(policy.asleep));
    }
    clear_error(global_error).await;
    let mut soonest = policy.backoff;
    let now = Instant::now();
    for summary in vehicles {
        let due = {
            let guard = archive.runtimes.lock().await;
            guard
                .get(&summary.vin)
                .is_none_or(|runtime| runtime.next_due <= now)
        };
        if !due {
            if let Some(runtime) = archive.runtimes.lock().await.get(&summary.vin) {
                let remaining = runtime.next_due.saturating_duration_since(now);
                if remaining < soonest {
                    soonest = remaining;
                }
            }
            continue;
        }
        match handle_summary(source, archive, policy, &summary).await? {
            CycleControl::Pause(wait) => return Ok(CycleControl::Pause(wait)),
            CycleControl::Wait(wait) => {
                if wait < soonest {
                    soonest = wait;
                }
            }
        }
    }
    Ok(CycleControl::Wait(soonest.max(Duration::from_secs(1))))
}

async fn handle_summary(
    source: &impl IngestSource,
    archive: &Archive,
    policy: &PollPolicy,
    summary: &Summary,
) -> Result<CycleControl, AppError> {
    let vehicle_id = upsert_vehicle(
        &archive.db,
        VehicleIdentity {
            vin: &summary.vin,
            tesla_id: Some(summary.tesla_id),
            legacy_vehicle_id: summary.legacy_vehicle_id,
            name: summary.name.as_deref(),
        },
    )
    .await?;
    let mut guard = archive.runtimes.lock().await;
    if !guard.contains_key(&summary.vin) {
        guard.insert(
            summary.vin.clone(),
            load_runtime(&archive.db, vehicle_id, &summary.vin).await?,
        );
    }
    let runtime = guard.get_mut(&summary.vin).expect("runtime");
    let telemetry_fresh = runtime
        .last_telemetry
        .is_some_and(|at| at.elapsed() < Duration::from_secs(120));
    let plan = poll_plan(
        summary.presence,
        runtime.tracker.activity,
        runtime.parked_live_polls,
        telemetry_fresh,
        policy,
    );
    if summary.presence != Presence::Online || !plan.observe {
        runtime.parked_live_polls = 0;
        runtime.next_due = Instant::now() + plan.wait;
        let vin = summary.vin.clone();
        let presence = summary.presence;
        drop(guard);
        let obs = resting_observation(Utc::now(), presence);
        if let Err(err) = apply_existing(archive, &vin, obs, false).await {
            tracing::warn!(vin = %vin, error = %err, "could not record resting state");
            let _ = record_error(&archive.db, vehicle_id, &err.to_string()).await;
        }
        return Ok(CycleControl::Wait(plan.wait));
    }
    drop(guard);
    match source.observe(&summary.vin).await? {
        ObserveResult::RateLimited(wait) => Ok(CycleControl::Pause(wait)),
        ObserveResult::Resting(presence) => {
            let mut guard = archive.runtimes.lock().await;
            if let Some(runtime) = guard.get_mut(&summary.vin) {
                runtime.parked_live_polls = 0;
                runtime.next_due = Instant::now() + policy.asleep;
            }
            drop(guard);
            let obs = resting_observation(Utc::now(), presence);
            apply_existing(archive, &summary.vin, obs, false).await?;
            Ok(CycleControl::Wait(policy.asleep))
        }
        ObserveResult::Live(obs) => {
            apply_existing(archive, &summary.vin, obs, true).await?;
            let mut guard = archive.runtimes.lock().await;
            if let Some(runtime) = guard.get_mut(&summary.vin) {
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
) -> Result<(), AppError> {
    let mut guard = archive.runtimes.lock().await;
    let Some(runtime) = guard.get_mut(vin) else {
        return Ok(());
    };
    apply_observation(
        &archive.db,
        runtime,
        &obs,
        &archive.geocoder,
        &archive.events,
        live,
    )
    .await
}

async fn set_error(slot: &Mutex<Option<String>>, message: String) {
    *slot.lock().await = Some(message);
}

async fn clear_error(slot: &Mutex<Option<String>>) {
    *slot.lock().await = None;
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
}
