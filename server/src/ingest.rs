// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Pull ingestion from the Fleet API.
//!
//! [`IngestSource`] is the plug-in used by the poller. Fleet Telemetry does not
//! poll: [`crate::telemetry`] pushes decoded records through [`apply_for_vin`],
//! which is the same archive path. Neither path calls `wake_up`.
//!
//! A live `vehicle_data` call resets the car's sleep timer. Tesla's vehicle
//! endpoint page describes that route as a live call to the vehicle, including
//! when `endpoints` selects a subset, so the subset is still billed and can
//! wake the car. `vehicle_data_cached` does not wake the car, but the billing
//! page counts every response below HTTP 500 and does not list that route as
//! free, so the idle path does not use it. Idle presence checks are
//! `GET /api/1/vehicles` (the list) and, only immediately before a live read,
//! `GET /api/1/vehicles/{id}`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
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
#[derive(Clone, Copy, Debug)]
pub(crate) struct PollPolicy {
    /// `vehicle_data` while driving without fresh telemetry.
    pub drive_fallback: Duration,
    pub charge: Duration,
    pub parked: Duration,
    pub asleep: Duration,
    pub backoff: Duration,
    pub parked_live_limit: u32,
    /// How often a fallback drive stores a `source=poll` position.
    pub gps_fallback: Duration,
    pub daily_cap: u64,
    pub monthly_cap: u64,
    /// How long to wait between probes while Tesla billing is blocked.
    pub blocked_probe: Duration,
    /// Billable read when Fleet Telemetry is the source but has gone quiet.
    pub telemetry_fallback: Duration,
    /// When set, charge and parked polls do not call `vehicle_data`.
    pub telemetry_mode: bool,
    pub dynamic_budget: bool,
    pub month_seed: u64,
    pub month_seed_month: Option<chrono::NaiveDate>,
    /// Budget clock. Production leaves this unset and uses the wall clock.
    pub now: Option<DateTime<Utc>>,
}

fn policy_now(policy: &PollPolicy) -> DateTime<Utc> {
    policy.now.unwrap_or_else(Utc::now)
}

/// Facts the next cycle needs. Grace and Hold keep [`Activity::Driving`] without
/// [`PollFacts::movement_recent`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct PollFacts {
    pub presence: Presence,
    pub activity: Activity,
    pub parked_live_polls: u32,
    pub telemetry_fresh: bool,
    pub movement_recent: bool,
    /// Open drive, vehicle online, and no telemetry message for 90 seconds.
    pub telemetry_drive_silent: bool,
}

/// Whether the next cycle should pay for `vehicle_data`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PollPlan {
    pub observe: bool,
    pub wait: Duration,
}

/// Free vehicle-list cadence while fresh telemetry covers the car.
const LIST_WAIT: Duration = Duration::from_secs(60);

/// Movement evidence older than this is grace or Hold, not a live drive poll.
const MOVEMENT_FRESH: chrono::Duration = chrono::Duration::seconds(60);

pub(crate) fn movement_is_recent(
    last_driving_at: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    last_driving_at.is_some_and(|at| now.signed_duration_since(at) < MOVEMENT_FRESH)
}

pub(crate) fn poll_plan(facts: &PollFacts, policy: &PollPolicy) -> PollPlan {
    if facts.presence != Presence::Online {
        return PollPlan {
            observe: false,
            wait: policy.asleep,
        };
    }
    // Configured telemetry replaces polling. Stale telemetry still pays for one
    // rare `vehicle_data` read; that read is counted by the caller.
    if policy.telemetry_mode {
        if facts.telemetry_drive_silent {
            return PollPlan {
                observe: true,
                wait: policy.drive_fallback,
            };
        }
        if facts.telemetry_fresh {
            return PollPlan {
                observe: false,
                wait: policy.backoff,
            };
        }
        return PollPlan {
            observe: true,
            wait: policy.telemetry_fallback,
        };
    }
    if facts.telemetry_fresh {
        return PollPlan {
            observe: false,
            wait: LIST_WAIT,
        };
    }
    if facts.activity == Activity::Driving {
        if !facts.movement_recent {
            return parked_plan(facts.parked_live_polls, policy);
        }
        return PollPlan {
            observe: true,
            wait: policy.drive_fallback,
        };
    }
    if facts.activity == Activity::Charging {
        return PollPlan {
            observe: true,
            wait: policy.charge,
        };
    }
    if facts.activity == Activity::Updating {
        return parked_plan(facts.parked_live_polls, policy);
    }
    parked_plan(facts.parked_live_polls, policy)
}

/// Keep the parked snapshot counter while the car stays online and idle.
///
/// Resetting it on the idle cycle is what made the poller take another
/// `vehicle_data` pair after every backoff and stop the car from sleeping.
pub(crate) fn keep_parked_snapshot_count(presence: Presence, activity: Activity) -> bool {
    presence == Presence::Online && !matches!(activity, Activity::Driving | Activity::Charging)
}

fn budget_query(policy: &PollPolicy, now: DateTime<Utc>) -> crate::budget::BudgetQuery {
    crate::budget::BudgetQuery {
        daily_cap: policy.daily_cap,
        monthly_cap: policy.monthly_cap,
        now,
        dynamic: policy.dynamic_budget,
        seed_calls: policy.month_seed,
        seed_month: policy.month_seed_month,
    }
}

fn parked_plan(parked_live_polls: u32, policy: &PollPolicy) -> PollPlan {
    // Space the allowed snapshot and every later presence check by the idle
    // backoff. `POLL_PARKED_SECS` is not this gap: a short gap would call
    // `vehicle_data` again and reset the sleep timer.
    let wait = policy.backoff;
    if parked_live_polls < policy.parked_live_limit {
        PollPlan {
            observe: true,
            wait,
        }
    } else {
        PollPlan {
            observe: false,
            wait,
        }
    }
}

/// A pull source. Fleet Telemetry implements the push side separately.
pub(crate) trait IngestSource: Send + Sync {
    /// Account vehicles, without waking them.
    fn discover(&self) -> impl Future<Output = Result<Vec<Summary>, AppError>> + Send;

    /// Live data when the vehicle is already online, otherwise a resting presence.
    fn observe(&self, vin: &str) -> impl Future<Output = Result<ObserveResult, AppError>> + Send;

    /// Push a signed Fleet Telemetry config. The default reports every VIN as configured.
    fn configure_telemetry(
        &self,
        vins: &[String],
        host: &str,
        port: u16,
        ca_pem: &str,
    ) -> impl Future<Output = Result<crate::telemetry_config::PushOutcome, AppError>> + Send {
        let _ = (host, port, ca_pem);
        std::future::ready(Ok(crate::telemetry_config::PushOutcome {
            configured: vins.to_vec(),
            skipped: Vec::new(),
        }))
    }

    /// Whether Tesla still has our config for this VIN. The default does not call Tesla.
    fn telemetry_installed(
        &self,
        vin: &str,
        expected_host: &str,
    ) -> impl Future<Output = Result<bool, AppError>> + Send {
        let _ = (vin, expected_host);
        std::future::ready(Ok(true))
    }
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
    /// HTTP 402, or 403 `EXCEEDED_LIMIT`. `vehicle_data` pauses until the next UTC month.
    ///
    /// `counted` is set when this was a `vehicle_data` call and must be stored.
    BillingLimit {
        status: u16,
        counted: bool,
    },
    /// HTTP 403 that is not a billing limit. Use the 429 backoff, not the short error retry.
    ///
    /// `counted` is set when this was a `vehicle_data` call.
    Backoff {
        counted: bool,
    },
    /// `vehicle_data` returned this status and no live body. The call is billed.
    DataFault {
        status: u16,
    },
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
    /// Spaces polled drive points and wakes the poller.
    pub gps: crate::drive_gps::GpsBoard,
    /// Fleet account that discovers these vehicles. `None` in tests that do not poll.
    pub fleet_account_id: Option<i64>,
    /// 429 pause deadlines keyed by Tesla account id.
    pub account_backoff: Arc<Mutex<HashMap<i64, Instant>>>,
    /// Consecutive Fleet API 429s. A successful `vehicle_data` resets this.
    pub fleet_strikes: Arc<Mutex<crate::budget::StrikeBook>>,
    pub fences: crate::persist::FenceCache,
    /// How long an open drive survives a gap or a confirmed park.
    pub drive_grace: crate::state::DriveGrace,
    /// Hot-reloadable settings. Absent in unit tests that construct an archive alone.
    pub runtime: Option<crate::settings::RuntimeConfig>,
    /// Set when Fleet Telemetry should be subscribed on the cars.
    pub telemetry: Option<TelemetryTarget>,
    /// A message newer than this is `data_source=telemetry`.
    pub telemetry_fresh_for: Duration,
    /// Process start. Signal-gap closes wait out the continuity window from here.
    pub started_at: DateTime<Utc>,
    /// Set when the telemetry listener fails to start. Empty when it is listening.
    pub listener_fault: std::sync::Mutex<Option<String>>,
}

impl Archive {
    /// Configured mode. The live API field uses [`Self::reported_data_source`].
    pub(crate) fn data_source(&self) -> &'static str {
        if self.telemetry.is_some() {
            "telemetry"
        } else {
            "polling"
        }
    }

    pub(crate) async fn reported_data_source(&self, vin: &str) -> &'static str {
        if self.telemetry.is_none() {
            return "polling";
        }
        crate::telemetry_config::live_data_source(
            self.telemetry_age(vin).await,
            self.telemetry_fresh_for,
        )
    }

    pub(crate) async fn any_fresh_telemetry(&self) -> bool {
        if self.telemetry.is_none() {
            return false;
        }
        let slots = self.runtimes.lock().await;
        for slot in slots.values() {
            let runtime = slot.lock().await;
            if runtime
                .last_telemetry
                .is_some_and(|at| at.elapsed() <= self.telemetry_fresh_for)
            {
                return true;
            }
        }
        false
    }

    pub(crate) async fn telemetry_age(&self, vin: &str) -> Option<Duration> {
        let slots = self.runtimes.lock().await;
        let slot = slots.get(vin)?;
        let runtime = slot.lock().await;
        runtime.last_telemetry.map(|at| at.elapsed())
    }
}

/// Where the car should send Fleet Telemetry, plus the VINs Tesla accepted.
pub(crate) struct TelemetryTarget {
    pub host: String,
    pub port: u16,
    pub ca: String,
    pub check_every: Duration,
    pub retry_base: Duration,
    pub retry_cap: Duration,
    book: Mutex<TelemetryBook>,
}

struct TelemetryBook {
    configured: std::collections::BTreeSet<String>,
    skipped: std::collections::BTreeMap<String, String>,
    failures: u32,
    next_attempt: Instant,
    force: bool,
    last_error: Option<String>,
}

impl TelemetryTarget {
    pub(crate) fn new(
        host: String,
        port: u16,
        ca: String,
        check_every: Duration,
        retry_base: Duration,
    ) -> Self {
        Self {
            host,
            port,
            ca,
            check_every,
            retry_base,
            retry_cap: Duration::from_secs(6 * 3600),
            book: Mutex::new(TelemetryBook {
                configured: std::collections::BTreeSet::new(),
                skipped: std::collections::BTreeMap::new(),
                failures: 0,
                next_attempt: Instant::now(),
                force: false,
                last_error: None,
            }),
        }
    }

    pub(crate) async fn force(&self) {
        let mut book = self.book.lock().await;
        book.force = true;
        book.next_attempt = Instant::now();
    }

    pub(crate) async fn skip_reason(&self, vin: &str) -> Option<String> {
        self.book.lock().await.skipped.get(vin).cloned()
    }

    pub(crate) async fn is_configured(&self, vin: &str) -> bool {
        self.book.lock().await.configured.contains(vin)
    }

    pub(crate) async fn last_error(&self) -> Option<String> {
        self.book.lock().await.last_error.clone()
    }

    pub(crate) async fn retry_scheduled(&self) -> bool {
        let book = self.book.lock().await;
        book.failures > 0 && book.last_error.is_some()
    }

    pub(crate) async fn forget(&self, vins: &[String]) {
        self.drop_configured(vins).await;
        let mut book = self.book.lock().await;
        for vin in vins {
            book.skipped.remove(vin);
        }
    }

    async fn due(&self) -> (bool, bool, std::collections::BTreeSet<String>) {
        let book = self.book.lock().await;
        (
            Instant::now() >= book.next_attempt,
            book.force,
            book.configured.clone(),
        )
    }

    async fn drop_configured(&self, vins: &[String]) {
        let mut book = self.book.lock().await;
        for vin in vins {
            book.configured.remove(vin);
        }
    }

    async fn apply(&self, outcome: &crate::telemetry_config::PushOutcome) {
        let mut book = self.book.lock().await;
        let next = Instant::now() + self.check_every;
        let TelemetryBook {
            configured,
            skipped,
            failures,
            force,
            last_error,
            next_attempt,
        } = &mut *book;
        crate::telemetry_config::apply_outcome(configured, skipped, outcome);
        *failures = 0;
        *force = false;
        *last_error = None;
        *next_attempt = next;
    }

    async fn checked(&self) {
        let mut book = self.book.lock().await;
        book.failures = 0;
        book.force = false;
        book.last_error = None;
        book.next_attempt = Instant::now() + self.check_every;
    }

    async fn fail(&self, err: &AppError) {
        let mut book = self.book.lock().await;
        book.failures = book.failures.saturating_add(1);
        book.force = false;
        book.last_error = Some(crate::logs::redact_line(&err.to_string(), &[]));
        book.next_attempt = Instant::now()
            + crate::telemetry_config::retry_delay(book.failures, self.retry_base, self.retry_cap);
    }
}

impl Archive {
    pub(crate) fn new(
        db: PgPool,
        geocoder: Geocoder,
        events: broadcast::Sender<LiveEvent>,
        gps: crate::drive_gps::GpsBoard,
    ) -> Self {
        Self {
            db,
            geocoder,
            events,
            runtimes: Arc::new(Mutex::new(HashMap::new())),
            gps,
            fleet_account_id: None,
            account_backoff: Arc::new(Mutex::new(HashMap::new())),
            fleet_strikes: Arc::new(Mutex::new(crate::budget::StrikeBook::default())),
            fences: crate::persist::FenceCache::default(),
            drive_grace: crate::state::DriveGrace::standard(),
            runtime: None,
            telemetry: None,
            telemetry_fresh_for: Duration::from_secs(900),
            started_at: Utc::now(),
            listener_fault: std::sync::Mutex::new(None),
        }
    }

    pub(crate) async fn vehicle_health(
        &self,
        vin: &str,
        asleep: bool,
        online: bool,
    ) -> crate::telemetry_health::TelemetryHealth {
        use crate::telemetry_health::{HealthFacts, classify};
        let enabled = self.telemetry.is_some();
        let (configured, skip_reason, retry_scheduled) = if let Some(target) = &self.telemetry {
            (
                target.is_configured(vin).await,
                target.skip_reason(vin).await,
                target.retry_scheduled().await,
            )
        } else {
            (false, None, false)
        };
        let listener_error = self
            .listener_fault
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        classify(&HealthFacts {
            enabled,
            asleep,
            online,
            message_age: self.telemetry_age(vin).await,
            configured,
            skip_reason: skip_reason.as_deref(),
            retry_scheduled,
            listener_error: listener_error.as_deref(),
            tls_rejected: tesla_fleet_telemetry::tls_handshake_rejections(),
        })
    }

    pub(crate) fn live_drive_grace(&self) -> crate::state::DriveGrace {
        self.runtime.as_ref().map_or(self.drive_grace, |runtime| {
            let snap = runtime.snapshot();
            crate::state::DriveGrace::from_config(
                snap.drive_end_grace.as_secs(),
                snap.drive_park_grace.as_secs(),
                snap.drive_p_confirm.as_secs(),
                snap.drive_stop_in_gear.as_secs(),
            )
        })
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
    let feed = if from_telemetry {
        Feed::Telemetry {
            fresh_location: true,
        }
    } else {
        Feed::Poll
    };
    write_observation(archive, &slot, &obs, live, true, feed).await
}

/// Writes one folded telemetry observation.
///
/// `fresh_location` says whether this payload carried a `Location`. When it did
/// not, the stored point keeps speed and gear but no coordinates, so a stale
/// location is never drawn at the new timestamp.
pub(crate) async fn apply_telemetry_for_vin(
    archive: &Archive,
    vin: &str,
    identity: VehicleIdentity<'_>,
    obs: Observation,
    fresh_location: bool,
) -> Result<(), AppError> {
    let vehicle_id = upsert_vehicle(&archive.db, identity, archive.fleet_account_id).await?;
    let slot = ensure_runtime(archive, vin, vehicle_id).await?;
    write_observation(
        archive,
        &slot,
        &obs,
        true,
        true,
        Feed::Telemetry { fresh_location },
    )
    .await
}

/// Where an observation written by [`write_observation`] comes from.
#[derive(Clone, Copy)]
enum Feed {
    Poll,
    Telemetry { fresh_location: bool },
}

async fn ensure_runtime(
    archive: &Archive,
    vin: &str,
    vehicle_id: i64,
) -> Result<SharedRuntime, AppError> {
    if let Some(slot) = archive.runtimes.lock().await.get(vin).cloned() {
        return Ok(slot);
    }
    let mut loaded = load_runtime(&archive.db, vehicle_id, vin).await?;
    loaded.drive_grace = archive.live_drive_grace();
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
    feed: Feed,
) -> Result<(), AppError> {
    let from_telemetry = matches!(feed, Feed::Telemetry { .. });
    let fresh_location = !matches!(
        feed,
        Feed::Telemetry {
            fresh_location: false
        }
    );
    let fences = archive.fences.get(&archive.db).await?;
    let jobs = {
        let mut runtime = slot.lock().await;
        if from_telemetry {
            runtime.last_telemetry = Some(tokio::time::Instant::now());
        }
        let filled;
        let obs = match known_odometer(from_telemetry, obs, runtime.published.as_ref()) {
            Some(km) => {
                filled = Observation {
                    odometer_km: Some(km),
                    ..obs.clone()
                };
                &filled
            }
            None => obs,
        };
        let store_position = store_position
            && !(from_telemetry
                && crate::state::telemetry_point_repeats(runtime.published.as_ref(), obs));
        apply_observation(
            &archive.db,
            &mut runtime,
            obs,
            &fences,
            &archive.events,
            crate::persist::ApplyOptions {
                live,
                store_position,
                source: if from_telemetry { "telemetry" } else { "poll" },
                fresh_location,
            },
        )
        .await?
    };
    crate::persist::settle_addresses(&archive.db, &archive.geocoder, jobs).await;
    Ok(())
}

/// Last known odometer for a telemetry observation that does not carry one.
///
/// The car only publishes a changed odometer, so the first messages after a
/// reconnect or a restart have none. Poll values are merged into
/// `published`, which holds the latest reading.
fn known_odometer(
    from_telemetry: bool,
    obs: &Observation,
    published: Option<&Observation>,
) -> Option<f64> {
    if !from_telemetry || obs.odometer_km.is_some() {
        return None;
    }
    published
        .and_then(|last| last.odometer_km)
        .filter(|km| km.is_finite())
}

/// Closes open drives whose grace elapsed with no new driving evidence.
///
/// The clock comes from stored positions, so this still runs after a restart.
pub(crate) async fn expire_drives(
    archive: &Archive,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), AppError> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT d.vehicle_id, v.vin
         FROM drives d
         JOIN vehicles v ON v.id = d.vehicle_id
         WHERE d.end_at IS NULL
           AND d.deleted_at IS NULL
           AND d.source IN ('poll', 'stream', 'telemetry')",
        // 'stream' stays: a drive opened by the removed Owner stream must still close.
    )
    .fetch_all(&archive.db)
    .await?;
    let fences = archive.fences.get(&archive.db).await?;
    let mut jobs = Vec::new();
    for (vehicle_id, vin) in rows {
        let slot = ensure_runtime(archive, &vin, vehicle_id).await?;
        let mut runtime = slot.lock().await;
        runtime.drive_grace = archive.live_drive_grace();
        let due = crate::persist::close_drive_if_due(
            &archive.db,
            &mut runtime,
            now,
            archive.started_at,
            &fences,
            &archive.events,
        )
        .await?;
        let wake = archive.telemetry.is_some()
            && crate::state::telemetry_drive_needs_fallback(
                runtime.open_drive_id.is_some(),
                runtime.tracker.activity,
                runtime.last_telemetry.map(|at| at.elapsed()),
            );
        jobs.extend(due);
        drop(runtime);
        if wake {
            archive.gps.notify();
        }
    }
    drop(fences);
    crate::persist::settle_addresses(&archive.db, &archive.geocoder, jobs).await;
    Ok(())
}

/// Fleet API poller. It never calls `wake_up`.
pub(crate) struct FleetPoll<T> {
    client: FleetClient<T>,
    debug: DebugBoard,
    signing: Option<crate::telemetry_config::SigningMaterial>,
}

impl<T: AccessTokenProvider> FleetPoll<T> {
    #[allow(dead_code)]
    pub(crate) fn new(client: FleetClient<T>) -> Self {
        Self::with_debug(client, DebugBoard::new(crate::logs::LogBuffer::new(8)))
    }

    pub(crate) fn with_debug(client: FleetClient<T>, debug: DebugBoard) -> Self {
        Self {
            client,
            debug,
            signing: None,
        }
    }

    pub(crate) fn with_signing(
        mut self,
        signing: crate::telemetry_config::SigningMaterial,
    ) -> Self {
        self.signing = Some(signing);
        self
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
        Ok(Self {
            client,
            debug,
            signing: None,
        })
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
                    self.debug.token_failed(&err.to_string());
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
                return classify_fleet(err);
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
                    Some(408),
                    started,
                    "vehicle unavailable",
                );
                return Ok(ObserveResult::DataFault { status: 408 });
            }
            Err(err) => {
                let status = fleet_http_status(&err);
                self.note_http(
                    "GET",
                    "/api/1/vehicles/{id}/vehicle_data",
                    status,
                    started,
                    &err.to_string(),
                );
                if matches!(err, FleetError::PaymentRequired) {
                    return Ok(ObserveResult::BillingLimit {
                        status: 402,
                        counted: true,
                    });
                }
                if err.is_exceeded_limit() {
                    return Ok(ObserveResult::BillingLimit {
                        status: status.unwrap_or(403),
                        counted: true,
                    });
                }
                if err.is_forbidden() {
                    return Ok(ObserveResult::Backoff { counted: true });
                }
                if err.is_device_unavailable() {
                    return Ok(ObserveResult::DataFault {
                        status: status.unwrap_or(408),
                    });
                }
                if matches!(err, FleetError::RateLimited(_)) {
                    return map_rate(err);
                }
                if let Some(status) = status {
                    return Ok(ObserveResult::DataFault { status });
                }
                return Err(fleet_error(err));
            }
        };
        Ok(ObserveResult::Live(observation_from_vehicle_data(data)))
    }

    async fn configure_telemetry(
        &self,
        vins: &[String],
        host: &str,
        port: u16,
        ca_pem: &str,
    ) -> Result<crate::telemetry_config::PushOutcome, AppError> {
        let Some(signing) = self.signing.as_ref() else {
            return Err(AppError::message("telemetry signing is not configured"));
        };
        let path = if signing.proxy_endpoint.is_some() {
            "/api/1/vehicles/fleet_telemetry_config"
        } else {
            "/api/1/vehicles/fleet_telemetry_config_jws"
        };
        match crate::telemetry_config::push_config(&self.client, signing, host, port, ca_pem, vins)
            .await
        {
            Ok(outcome) => {
                self.note_http(
                    "POST",
                    path,
                    Some(200),
                    Instant::now(),
                    "telemetry config applied",
                );
                Ok(outcome)
            }
            Err(err) => {
                self.note_http(
                    "POST",
                    path,
                    None,
                    Instant::now(),
                    "telemetry config was not applied",
                );
                Err(err)
            }
        }
    }

    async fn telemetry_installed(&self, vin: &str, expected_host: &str) -> Result<bool, AppError> {
        let tag = tesla_fleet_api::VehicleTag::new(vin)
            .map_err(|_| AppError::message("vehicle vin is invalid"))?;
        match self.client.fleet_telemetry_config(&tag).await {
            Ok(found) => {
                let host = found.config.as_ref().map(|cfg| cfg.hostname.as_str());
                Ok(!crate::telemetry_config::config_needs_resend(
                    host,
                    expected_host,
                ))
            }
            Err(FleetError::NotFound { .. }) => Ok(false),
            Err(err) => {
                self.note_http(
                    "GET",
                    "/api/1/vehicles/{vin}/fleet_telemetry_config",
                    fleet_http_status(&err),
                    Instant::now(),
                    "telemetry config was not read",
                );
                Err(fleet_error(err))
            }
        }
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
    classify_fleet(err)
}

/// Turn a Fleet error into a poll result. 403 `EXCEEDED_LIMIT` pauses like 402.
/// Any other 403 is a backoff, not a short error retry.
fn classify_fleet(err: FleetError) -> Result<ObserveResult, AppError> {
    if matches!(err, FleetError::PaymentRequired) {
        return Ok(ObserveResult::BillingLimit {
            status: 402,
            counted: false,
        });
    }
    if err.is_exceeded_limit() {
        return Ok(ObserveResult::BillingLimit {
            status: fleet_http_status(&err).unwrap_or(403),
            counted: false,
        });
    }
    if err.is_forbidden() {
        return Ok(ObserveResult::Backoff { counted: false });
    }
    match fleet_error(err) {
        AppError::RateLimited { retry_after } => Ok(ObserveResult::RateLimited(retry_after)),
        AppError::PaymentRequired => Ok(ObserveResult::BillingLimit {
            status: 402,
            counted: false,
        }),
        AppError::ExceededLimit => Ok(ObserveResult::BillingLimit {
            status: 403,
            counted: false,
        }),
        AppError::Forbidden => Ok(ObserveResult::Backoff { counted: false }),
        other => Err(other),
    }
}

pub(crate) async fn subscribe_telemetry(
    source: &impl IngestSource,
    archive: &Archive,
    vehicles: &[Summary],
    billing_blocked: bool,
) {
    let Some(target) = archive.telemetry.as_ref() else {
        return;
    };
    let known = vehicles
        .iter()
        .map(|summary| summary.vin.clone())
        .collect::<Vec<_>>();
    let (due, force, configured) = target.due().await;
    let Some(plan) =
        crate::telemetry_config::plan_attempt(due, billing_blocked, &known, &configured, force)
    else {
        if billing_blocked {
            tracing::info!("fleet telemetry config check skipped while the data budget is paused");
        }
        return;
    };
    let mut missing = Vec::new();
    for vin in &plan.check {
        match source.telemetry_installed(vin, &target.host).await {
            Ok(true) => {}
            Ok(false) => missing.push(vin.clone()),
            Err(err) => {
                target.fail(&err).await;
                let message = crate::logs::redact_line(&err.to_string(), &[]);
                tracing::warn!(error = %message, "fleet telemetry config could not be read");
                return;
            }
        }
    }
    if !missing.is_empty() {
        target.drop_configured(&missing).await;
        tracing::info!(
            cars = missing.len(),
            "fleet telemetry config is gone and will be sent again"
        );
    }
    let mut push = plan.push;
    push.extend(missing);
    if push.is_empty() {
        target.checked().await;
        return;
    }
    match source
        .configure_telemetry(&push, &target.host, target.port, &target.ca)
        .await
    {
        Ok(outcome) => {
            for (_vin, reason) in &outcome.skipped {
                tracing::warn!(reason = %reason, "fleet telemetry config skipped a vehicle");
            }
            tracing::info!(
                configured = outcome.configured.len(),
                skipped = outcome.skipped.len(),
                "fleet telemetry config applied"
            );
            target.apply(&outcome).await;
        }
        Err(err) => {
            target.fail(&err).await;
            let message = crate::logs::redact_line(&err.to_string(), &[]);
            tracing::warn!(error = %message, cars = push.len(), "fleet telemetry config was not applied");
        }
    }
}

async fn pause_for_billing_limit(
    archive: &Archive,
    probe_list: bool,
    interval: Duration,
) -> CycleControl {
    if let Err(err) =
        crate::budget::enter_block(&archive.db, Utc::now(), interval, probe_list).await
    {
        tracing::warn!(error = %err, "could not store the fleet billing block");
    }
    CycleControl::Wait(interval)
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
        obs.scheduled_charging_mode = charge
            .scheduled_charging_mode
            .as_deref()
            .and_then(crate::state::normalize_charging_mode)
            .map(str::to_owned);
        obs.scheduled_departure_time = minutes_after_midnight(charge.scheduled_departure_time)
            .or_else(|| minutes_after_midnight(charge.scheduled_departure_time_minutes));
        obs.off_peak_charging_enabled = charge.off_peak_charging_enabled;
        obs.off_peak_hours_end_time = minutes_after_midnight(charge.off_peak_hours_end_time);
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
        let mut policy = policy;
        // Prefer the runtime snapshot (DB overrides) when the archive carries one;
        // otherwise fall back to per-key reads for unit tests.
        if let Some(runtime) = archive.runtime.as_ref() {
            let snap = runtime.snapshot();
            policy.drive_fallback = snap.drive_fallback;
            policy.charge = snap.charge;
            policy.parked = snap.parked;
            policy.asleep = snap.asleep;
            policy.backoff = snap.backoff;
            policy.gps_fallback = snap.gps_fallback;
            policy.parked_live_limit = snap.parked_live_limit;
            policy.daily_cap = snap.daily_cap;
            policy.monthly_cap = snap.monthly_cap;
            policy.blocked_probe = snap.blocked_probe;
            policy.telemetry_fallback = snap.telemetry_fallback;
            policy.dynamic_budget = snap.dynamic_budget;
            policy.month_seed = snap.month_seed;
            policy.month_seed_month = snap.month_seed_month;
        } else {
            policy.drive_fallback = crate::settings::bounded_secs(
                &archive.db,
                "POLL_DRIVE_FALLBACK_SECS",
                policy.drive_fallback,
                crate::budget::DRIVE_FALLBACK_MIN_SECS,
                crate::budget::DRIVE_FALLBACK_MAX_SECS,
            )
            .await;
            policy.charge = crate::settings::bounded_secs(
                &archive.db,
                "POLL_CHARGE_SECS",
                policy.charge,
                crate::budget::CHARGE_POLL_MIN_SECS,
                crate::budget::CHARGE_POLL_MAX_SECS,
            )
            .await;
            policy.parked =
                crate::settings::secs_or(&archive.db, "POLL_PARKED_SECS", policy.parked).await;
            policy.asleep =
                crate::settings::secs_or(&archive.db, "POLL_ASLEEP_SECS", policy.asleep).await;
            policy.backoff =
                crate::settings::secs_or(&archive.db, "POLL_BACKOFF_SECS", policy.backoff).await;
            policy.gps_fallback = crate::settings::secs_or(
                &archive.db,
                "POLL_GPS_FALLBACK_SECS",
                policy.gps_fallback,
            )
            .await;
            policy.parked_live_limit = u32::try_from(
                crate::settings::bounded_u64(
                    &archive.db,
                    "POLL_PARKED_LIVE_LIMIT",
                    u64::from(policy.parked_live_limit),
                    0,
                    100,
                )
                .await,
            )
            .unwrap_or(policy.parked_live_limit);
            policy.daily_cap = crate::settings::bounded_u64(
                &archive.db,
                "FLEET_DATA_DAILY_CAP",
                policy.daily_cap,
                crate::budget::DAILY_CAP_MIN,
                crate::budget::DAILY_CAP_MAX,
            )
            .await;
            policy.monthly_cap = crate::settings::bounded_u64(
                &archive.db,
                "FLEET_DATA_MONTHLY_CAP",
                policy.monthly_cap,
                crate::budget::MONTHLY_CAP_MIN,
                crate::budget::MONTHLY_CAP_MAX,
            )
            .await;
            if policy.monthly_cap < policy.daily_cap {
                policy.monthly_cap = policy.daily_cap;
            }
            policy.blocked_probe = crate::settings::bounded_secs(
                &archive.db,
                "FLEET_BLOCKED_PROBE_SECS",
                policy.blocked_probe,
                crate::budget::BLOCKED_PROBE_MIN_SECS,
                crate::budget::BLOCKED_PROBE_MAX_SECS,
            )
            .await;
        }
        // Telemetry mode is active only after the receiver certificate was read.
        // A missing file keeps the sleep-safe polling schedule.
        policy.telemetry_mode = archive.telemetry.is_some();
        let control = run_cycle(source, &archive, &policy, account_id).await;
        let gate = crate::budget::gate(&archive.db, budget_query(&policy, Utc::now())).await;
        source.debug.set_budget_banner(gate.debug_line());
        let wait = match control {
            Ok(CycleControl::Wait(wait) | CycleControl::Pause(wait)) => wait,
            Err(err) => {
                let message = crate::logs::redact_line(&err.to_string(), &[]);
                tracing::warn!(error = %message, "ingest cycle failed");
                if let Some(account_id) = account_id {
                    let _ =
                        crate::accounts::set_last_error(&archive.db, account_id, &message).await;
                }
                let backoff = match account_id {
                    Some(id) => account_pause(&archive, id).await,
                    None => None,
                };
                crate::budget::error_retry(backoff, gate.blocks_vehicle_data())
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
    if let Some(account_id) = account_id
        && let Some(wait) = disconnected_account_wait(archive, account_id).await
    {
        return Ok(CycleControl::Wait(wait));
    }
    let now_utc = policy_now(policy);
    let gate = crate::budget::gate(&archive.db, budget_query(policy, now_utc)).await;
    if let Some(wait) = gate.list_wait(now_utc) {
        return Ok(CycleControl::Wait(wait));
    }
    let list_probe = gate
        .block
        .is_some_and(|block| block.probe_list && block.due(now_utc));
    let vehicles = match source.discover().await {
        Ok(vehicles) => vehicles,
        Err(AppError::RateLimited { retry_after }) => {
            let wait = note_fleet_limit(archive, Some(retry_after)).await;
            note_rate_limit(archive, account_id, wait).await;
            return Ok(CycleControl::Pause(wait));
        }
        Err(AppError::PaymentRequired | AppError::ExceededLimit) => {
            return Ok(pause_for_billing_limit(archive, true, policy.blocked_probe).await);
        }
        Err(AppError::Forbidden) => {
            let wait = note_fleet_limit(archive, None).await;
            note_rate_limit(archive, account_id, wait).await;
            return Ok(CycleControl::Pause(wait));
        }
        Err(err) => return Err(err),
    };
    if list_probe {
        crate::budget::resume_billing(&archive.db).await?;
    }
    if policy.telemetry_mode {
        let blocked = gate.blocks_vehicle_data() || gate.cap_reached();
        subscribe_telemetry(source, archive, &vehicles, blocked).await;
    }
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
    let data_probe = gate
        .block
        .is_some_and(|block| !block.probe_list && block.due(now_utc));
    if data_probe {
        let online = vehicles
            .iter()
            .find(|summary| summary.presence == Presence::Online)
            .cloned();
        if gate.cap_reached() || online.is_none() {
            let _ =
                crate::budget::reschedule_probe(&archive.db, now_utc, policy.blocked_probe).await;
        } else if let Some(summary) = online {
            let control = handle_summary(source, archive, policy, &summary, true).await?;
            let still = crate::budget::gate(&archive.db, budget_query(policy, now_utc))
                .await
                .block
                .is_some();
            if still {
                return Ok(control);
            }
            return Ok(CycleControl::Wait(Duration::from_secs(1)));
        }
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
        match handle_summary(source, archive, policy, &summary, false).await {
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

async fn disconnected_account_wait(archive: &Archive, account_id: i64) -> Option<Duration> {
    let row: Option<(String,)> = sqlx::query_as("SELECT status FROM tesla_accounts WHERE id = $1")
        .bind(account_id)
        .fetch_optional(&archive.db)
        .await
        .ok()
        .flatten();
    row.as_ref()
        .and_then(|row| crate::connection::disconnected_poll_wait(&row.0))
}

pub(crate) async fn account_pause(archive: &Archive, account_id: i64) -> Option<Duration> {
    {
        let guard = archive.account_backoff.lock().await;
        if let Some(until) = guard.get(&account_id).copied() {
            let now = Instant::now();
            if until > now {
                return Some(until.saturating_duration_since(now));
            }
        }
    }
    let row: Option<(chrono::DateTime<chrono::Utc>,)> = sqlx::query_as(
        "SELECT paused_until FROM tesla_accounts WHERE id = $1 AND paused_until > now()",
    )
    .bind(account_id)
    .fetch_optional(&archive.db)
    .await
    .ok()
    .flatten();
    let until = row?.0;
    let wait = (until - chrono::Utc::now()).to_std().ok()?;
    note_account_pause_memory(archive, account_id, wait).await;
    Some(wait)
}

pub(crate) async fn note_account_pause(archive: &Archive, account_id: i64, wait: Duration) {
    note_account_pause_memory(archive, account_id, wait).await;
    let until = chrono::Utc::now() + chrono::Duration::from_std(wait).unwrap_or_default();
    let _ = sqlx::query(
        "UPDATE tesla_accounts
         SET paused_until = $2
         WHERE id = $1 AND (paused_until IS NULL OR paused_until < $2)",
    )
    .bind(account_id)
    .bind(until)
    .execute(&archive.db)
    .await;
}

async fn note_account_pause_memory(archive: &Archive, account_id: i64, wait: Duration) {
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

async fn note_fleet_limit(archive: &Archive, retry_after: Option<Duration>) -> Duration {
    archive
        .fleet_strikes
        .lock()
        .await
        .on_limited_now(retry_after)
}

async fn note_vehicle_data(archive: &Archive, status: u16) {
    if let Err(err) = crate::budget::record(
        &archive.db,
        crate::budget::VEHICLE_DATA,
        i32::from(status),
        Utc::now(),
    )
    .await
    {
        tracing::warn!(error = %err, "could not record fleet usage");
    }
}

async fn handle_summary(
    source: &impl IngestSource,
    archive: &Archive,
    policy: &PollPolicy,
    summary: &Summary,
    probe: bool,
) -> Result<CycleControl, AppError> {
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
    let now_utc = policy_now(policy);
    let (
        activity,
        telemetry_fresh,
        telemetry_age,
        parked_live_polls,
        movement_recent,
        open_drive_id,
    ) = {
        let runtime = slot.lock().await;
        (
            runtime.tracker.activity,
            runtime
                .last_telemetry
                .is_some_and(|at| at.elapsed() <= archive.telemetry_fresh_for),
            runtime.last_telemetry.map(|at| at.elapsed()),
            runtime.parked_live_polls,
            movement_is_recent(
                runtime.drive_clock.map(|clock| clock.last_driving_at),
                now_utc,
            ),
            runtime.open_drive_id,
        )
    };
    let facts = PollFacts {
        presence: summary.presence,
        activity,
        parked_live_polls,
        telemetry_fresh,
        movement_recent,
        telemetry_drive_silent: summary.presence == Presence::Online
            && crate::state::telemetry_drive_needs_fallback(
                open_drive_id.is_some(),
                activity,
                telemetry_age,
            ),
    };
    let mut plan = poll_plan(&facts, policy);
    if !plan.observe
        && crate::drive_gps::fallback_observe(
            summary.presence,
            telemetry_fresh,
            plan.observe,
            movement_recent,
        )
    {
        plan.observe = true;
        plan.wait = policy.drive_fallback;
    }
    let gate = crate::budget::gate(&archive.db, budget_query(policy, now_utc)).await;
    let blocked = gate.cap_reached() || (gate.block.is_some() && !probe);
    if blocked {
        plan.observe = false;
    }
    if summary.presence != Presence::Online || !plan.observe {
        return skip_live(archive, &slot, summary, vehicle_id, plan).await;
    }
    match source.observe(&summary.vin).await? {
        ObserveResult::RateLimited(retry_after) => {
            note_vehicle_data(archive, 429).await;
            let wait = note_fleet_limit(archive, Some(retry_after)).await;
            Ok(CycleControl::Pause(wait))
        }
        ObserveResult::BillingLimit { status, counted } => {
            if counted {
                note_vehicle_data(archive, status).await;
            }
            Ok(pause_for_billing_limit(archive, false, policy.blocked_probe).await)
        }
        ObserveResult::Backoff { counted } => {
            if counted {
                note_vehicle_data(archive, 403).await;
            }
            let wait = note_fleet_limit(archive, None).await;
            slot.lock().await.next_due = Instant::now() + wait;
            Ok(CycleControl::Pause(wait))
        }
        ObserveResult::DataFault { status } => {
            note_vehicle_data(archive, status).await;
            if probe {
                crate::budget::resume_billing(&archive.db).await?;
            }
            if status == 408 {
                {
                    let mut runtime = slot.lock().await;
                    runtime.parked_live_polls = 0;
                    runtime.next_due = Instant::now() + policy.asleep;
                }
                let obs = resting_observation(Utc::now(), Presence::Asleep);
                apply_existing(archive, &summary.vin, obs, false, false).await?;
                Ok(CycleControl::Wait(policy.asleep))
            } else {
                slot.lock().await.next_due = Instant::now() + policy.backoff;
                Ok(CycleControl::Wait(policy.backoff))
            }
        }
        ObserveResult::Resting(presence) => {
            if probe {
                let _ =
                    crate::budget::reschedule_probe(&archive.db, Utc::now(), policy.blocked_probe)
                        .await;
            }
            {
                let mut runtime = slot.lock().await;
                runtime.parked_live_polls = 0;
                runtime.next_due = Instant::now() + policy.asleep;
            }
            let obs = resting_observation(Utc::now(), presence);
            apply_existing(archive, &summary.vin, obs, false, false).await?;
            let wait = if probe {
                policy.blocked_probe
            } else {
                policy.asleep
            };
            Ok(CycleControl::Wait(wait))
        }
        ObserveResult::Live(obs) => {
            note_vehicle_data(archive, 200).await;
            archive.fleet_strikes.lock().await.on_success();
            let driving = crate::drive_gps::sample_is_driving(
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
            if probe {
                crate::budget::resume_billing(&archive.db).await?;
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

async fn skip_live(
    archive: &Archive,
    slot: &SharedRuntime,
    summary: &Summary,
    vehicle_id: i64,
    plan: PollPlan,
) -> Result<CycleControl, AppError> {
    let (preserve, activity) = {
        let runtime = slot.lock().await;
        (
            summary.presence == Presence::Online
                && (runtime.tracker.drive_open || runtime.tracker.activity == Activity::Driving),
            runtime.tracker.activity,
        )
    };
    {
        let mut runtime = slot.lock().await;
        if !keep_parked_snapshot_count(summary.presence, activity) {
            runtime.parked_live_polls = 0;
        }
        runtime.next_due = Instant::now() + plan.wait;
    }
    if preserve {
        return Ok(CycleControl::Wait(plan.wait));
    }
    let obs = resting_observation(Utc::now(), summary.presence);
    if let Err(err) = apply_existing(archive, &summary.vin, obs, false, false).await {
        tracing::warn!(
            vehicle_id = summary.tesla_id,
            error = %crate::logs::redact_line(&err.to_string(), &[]),
            "could not record resting state"
        );
        let _ = record_error(&archive.db, vehicle_id, &err.to_string()).await;
    }
    Ok(CycleControl::Wait(plan.wait))
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
    write_observation(archive, &slot, &obs, live, store_position, Feed::Poll).await
}

#[cfg(test)]
mod tests {
    use super::{Activity, FleetPoll, IngestSource, PollFacts, PollPolicy, Presence, poll_plan};
    use std::time::Duration;
    use tesla_fleet_api::{BearerToken, FleetClient};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn policy() -> PollPolicy {
        PollPolicy {
            drive_fallback: Duration::from_secs(120),
            charge: Duration::from_secs(900),
            parked: Duration::from_secs(180),
            asleep: Duration::from_secs(300),
            backoff: Duration::from_secs(720),
            parked_live_limit: 2,
            gps_fallback: Duration::from_secs(300),
            daily_cap: 160,
            monthly_cap: 5_000,
            blocked_probe: Duration::from_secs(3_600),
            telemetry_fallback: Duration::from_secs(3_600),
            telemetry_mode: false,
            dynamic_budget: true,
            month_seed: 0,
            month_seed_month: None,
            now: None,
        }
    }

    fn facts(
        presence: Presence,
        activity: Activity,
        parked_live_polls: u32,
        telemetry_fresh: bool,
        movement_recent: bool,
    ) -> PollFacts {
        PollFacts {
            presence,
            activity,
            parked_live_polls,
            telemetry_fresh,
            movement_recent,
            telemetry_drive_silent: false,
        }
    }

    #[test]
    fn asleep_and_backoff_do_not_request_live_data() {
        let policy = policy();
        let asleep = poll_plan(
            &facts(Presence::Asleep, Activity::Asleep, 0, false, false),
            &policy,
        );
        assert!(!asleep.observe);
        assert_eq!(asleep.wait, Duration::from_secs(300));
        let early = poll_plan(
            &facts(Presence::Online, Activity::Online, 0, false, false),
            &policy,
        );
        assert!(early.observe);
        let later = poll_plan(
            &facts(Presence::Online, Activity::Online, 2, false, false),
            &policy,
        );
        assert!(!later.observe);
        assert_eq!(later.wait, Duration::from_secs(720));
    }

    #[test]
    fn parked_takes_one_snapshot_then_waits_out_the_sleep_timer() {
        // Tesla resets the vehicle sleep timer on a live `vehicle_data` call.
        // After the configured snapshots, the next wait is the idle backoff and
        // the plan must not ask for another live read.
        let mut policy = policy();
        policy.parked_live_limit = 1;
        policy.backoff = Duration::from_secs(1_800);
        let first = poll_plan(
            &facts(Presence::Online, Activity::Online, 0, false, false),
            &policy,
        );
        assert!(first.observe);
        assert_eq!(first.wait, Duration::from_secs(1_800));
        let idle = poll_plan(
            &facts(Presence::Online, Activity::Online, 1, false, false),
            &policy,
        );
        assert!(!idle.observe);
        assert!(idle.wait >= Duration::from_secs(1_800));
        let telemetry_drive = poll_plan(
            &facts(Presence::Online, Activity::Driving, 0, true, true),
            &policy,
        );
        assert!(!telemetry_drive.observe);
    }

    #[test]
    fn telemetry_mode_skips_charge_polls_until_telemetry_is_stale() {
        let mut policy = policy();
        policy.telemetry_mode = true;
        policy.charge = Duration::from_secs(900);
        policy.telemetry_fallback = Duration::from_secs(3_600);
        let fresh = poll_plan(
            &facts(Presence::Online, Activity::Charging, 0, true, false),
            &policy,
        );
        assert!(!fresh.observe);
        let stale = poll_plan(
            &facts(Presence::Online, Activity::Charging, 0, false, false),
            &policy,
        );
        assert!(stale.observe);
        assert_eq!(stale.wait, Duration::from_secs(3_600));
        let mut driving = facts(Presence::Online, Activity::Driving, 0, false, true);
        driving.telemetry_drive_silent = true;
        let fallback = poll_plan(&driving, &policy);
        assert!(fallback.observe);
        assert_eq!(fallback.wait, policy.drive_fallback);
        let asleep = poll_plan(
            &facts(Presence::Asleep, Activity::Asleep, 0, false, false),
            &policy,
        );
        assert!(!asleep.observe);
    }

    #[test]
    fn charging_without_telemetry_uses_the_charge_interval() {
        let mut policy = policy();
        policy.charge = Duration::from_secs(900);
        let plan = poll_plan(
            &facts(Presence::Online, Activity::Charging, 9, false, false),
            &policy,
        );
        assert!(plan.observe);
        assert_eq!(plan.wait, Duration::from_secs(900));
    }

    #[test]
    fn idle_online_keeps_the_parked_snapshot_count() {
        assert!(super::keep_parked_snapshot_count(
            Presence::Online,
            Activity::Online
        ));
        assert!(!super::keep_parked_snapshot_count(
            Presence::Asleep,
            Activity::Online
        ));
        assert!(!super::keep_parked_snapshot_count(
            Presence::Online,
            Activity::Driving
        ));
        assert!(!super::keep_parked_snapshot_count(
            Presence::Online,
            Activity::Charging
        ));
    }

    #[test]
    fn driving_without_telemetry_uses_the_fallback_interval() {
        let policy = policy();
        let plan = poll_plan(
            &facts(Presence::Online, Activity::Driving, 0, false, true),
            &policy,
        );
        assert!(plan.observe);
        assert_eq!(plan.wait, Duration::from_secs(120));
        assert_ne!(plan.wait, Duration::from_secs(15));
    }

    #[test]
    fn grace_and_hold_do_not_use_a_fast_drive_poll() {
        let policy = policy();
        let grace = poll_plan(
            &facts(Presence::Online, Activity::Driving, 0, false, false),
            &policy,
        );
        assert_eq!(grace.wait, Duration::from_secs(720));
        assert_ne!(grace.wait, Duration::from_secs(15));
        let held = poll_plan(
            &facts(Presence::Online, Activity::Driving, 2, false, false),
            &policy,
        );
        assert!(!held.observe);
        assert_eq!(held.wait, Duration::from_secs(720));
        assert_ne!(held.wait, Duration::from_secs(15));
    }

    #[test]
    fn charging_uses_the_slower_default() {
        let policy = policy();
        let charging = poll_plan(
            &facts(Presence::Online, Activity::Charging, 0, false, false),
            &policy,
        );
        assert!(charging.observe);
        assert_eq!(charging.wait, Duration::from_secs(900));
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
    fn stopped_with_the_door_shut_stays_plugged_and_keeps_the_schedule() {
        let data: tesla_fleet_api::VehicleData = serde_json::from_str(
            r#"{
                "id": 1,
                "vin": "V",
                "charge_state": {
                    "charging_state": "Stopped",
                    "charge_port_latch": "Engaged",
                    "charge_port_door_open": false,
                    "charger_power": 0,
                    "conn_charge_cable": "<invalid>",
                    "scheduled_charging_mode": "StartAt",
                    "scheduled_charging_start_time": 1320,
                    "scheduled_departure_time": 2500,
                    "scheduled_departure_time_minutes": 420,
                    "off_peak_charging_enabled": true,
                    "off_peak_hours_end_time": 360
                }
            }"#,
        )
        .unwrap();
        let obs = super::observation_from_vehicle_data(data);
        assert_eq!(obs.charging_state.as_deref(), Some("Stopped"));
        assert_eq!(obs.plugged_in, Some(true));
        assert_eq!(obs.scheduled_charging_mode.as_deref(), Some("StartAt"));
        assert_eq!(obs.scheduled_charging_start_time, Some(1320));
        assert_eq!(obs.scheduled_departure_time, Some(420));
        assert_eq!(obs.off_peak_charging_enabled, Some(true));
        assert_eq!(obs.off_peak_hours_end_time, Some(360));
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
            crate::drive_gps::GpsBoard::default(),
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
            crate::drive_gps::GpsBoard::default(),
        );
        super::note_account_pause(&archive, 7, Duration::from_secs(40)).await;
        assert!(super::account_pause(&archive, 7).await.is_some());
        assert!(super::account_pause(&archive, 8).await.is_none());
    }

    #[tokio::test]
    async fn fresh_telemetry_drive_does_not_call_vehicle_data_in_an_hour() {
        let Some(db) = crate::test_util::TestDb::migrated("telemetrydrive").await else {
            return;
        };
        let vin = "TEST00000000VIN01";
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/1/vehicles"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"response":[{"id":1,"vin":"TEST00000000VIN01","state":"online","display_name":"Test"}],"pagination":{"next":null}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/1/vehicles/{vin}")))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"response":{"id":1,"vin":"TEST00000000VIN01","state":"online"}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/1/vehicles/{vin}/vehicle_data")))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"response":{"id":1,"vin":"TEST00000000VIN01","drive_state":{"shift_state":"D","speed":30},"climate_state":{"inside_temp":21,"outside_temp":9},"vehicle_state":{"odometer":1000,"tpms_pressure_fl":2.9}}}"#,
            ))
            .mount(&server)
            .await;
        sqlx::query("INSERT INTO vehicles (vin, name) VALUES ($1, 'Test')")
            .bind(vin)
            .execute(&db.pool)
            .await
            .unwrap();
        let vehicle_id: i64 = sqlx::query_scalar("SELECT id FROM vehicles WHERE vin = $1")
            .bind(vin)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO drives (vehicle_id, start_at, source) VALUES ($1, now(), 'telemetry')",
        )
        .bind(vehicle_id)
        .execute(&db.pool)
        .await
        .unwrap();
        let drive_id: i64 =
            sqlx::query_scalar("SELECT id FROM drives WHERE vehicle_id = $1 AND end_at IS NULL")
                .bind(vehicle_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO positions (vehicle_id, recorded_at, speed_kmh, shift_state, drive_id, source)
             VALUES ($1, now(), 40, 'D', $2, 'telemetry')",
        )
        .bind(vehicle_id)
        .bind(drive_id)
        .execute(&db.pool)
        .await
        .unwrap();
        let client = FleetClient::builder(&server.uri(), BearerToken::new("token"))
            .unwrap()
            .build()
            .unwrap();
        let source = FleetPoll::new(client);
        let archive = super::Archive::new(
            db.pool.clone(),
            crate::geocode::Geocoder::disabled(),
            tokio::sync::broadcast::channel(8).0,
            crate::drive_gps::GpsBoard::default(),
        );
        let mut policy = policy();
        policy.telemetry_mode = true;
        let mut elapsed = Duration::ZERO;
        let mut cycles = 0_u32;
        while elapsed < Duration::from_secs(3_600) && cycles < 500 {
            if let Some(slot) = archive.runtimes.lock().await.get(vin).cloned() {
                let mut runtime = slot.lock().await;
                runtime.next_due = tokio::time::Instant::now();
                runtime.tracker.activity = Activity::Driving;
                runtime.tracker.drive_open = true;
                runtime.drive_clock = Some(crate::state::DriveClock::moving_at(chrono::Utc::now()));
                runtime.last_telemetry = Some(tokio::time::Instant::now());
            }
            let control = super::run_cycle(&source, &archive, &policy, None)
                .await
                .expect("cycle");
            let wait = match control {
                super::CycleControl::Wait(wait) | super::CycleControl::Pause(wait) => wait,
            };
            elapsed += wait;
            cycles += 1;
        }
        let requests = server.received_requests().await.unwrap();
        let data_calls = requests
            .iter()
            .filter(|request| request.url.path().ends_with("/vehicle_data"))
            .count();
        let list_calls = requests
            .iter()
            .filter(|request| request.url.path() == "/api/1/vehicles")
            .count();
        let wakes = requests
            .iter()
            .filter(|request| request.url.path().contains("wake_up"))
            .count();
        assert_eq!(wakes, 0);
        assert!(
            data_calls <= 1,
            "vehicle_data calls {data_calls} over {cycles} cycles ({elapsed:?})"
        );
        assert!(
            list_calls >= 4,
            "list calls {list_calls}; the free list should keep running"
        );
        assert!(elapsed >= Duration::from_secs(3_600), "{elapsed:?}");
    }

    /// Vehicle list that always reports the car online and never expects a
    /// `vehicle_data` call: telemetry is fresh, so the poller only lists.
    struct ListOnly {
        vin: &'static str,
        observed: std::sync::atomic::AtomicUsize,
    }

    impl IngestSource for ListOnly {
        async fn discover(&self) -> Result<Vec<super::Summary>, crate::error::AppError> {
            std::future::ready(Ok(vec![summary(self.vin, 1)])).await
        }

        async fn observe(
            &self,
            _vin: &str,
        ) -> Result<super::ObserveResult, crate::error::AppError> {
            self.observed
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            std::future::ready(Err(crate::error::AppError::message(
                "unexpected vehicle_data",
            )))
            .await
        }
    }

    /// Replay of the night of 27/11/2026: an AC home charge at 7 kW, 59 -> 80 %,
    /// streamed by Fleet Telemetry only. A parked car sends no `Location`, no
    /// charge energy counter is subscribed, and the free vehicle-list cycle
    /// keeps running while telemetry is fresh.
    #[tokio::test]
    async fn telemetry_charge_with_list_polls_is_one_closed_located_session() {
        use crate::ingest::apply_telemetry_for_vin;
        use crate::persist::VehicleIdentity;

        let Some(db) = crate::test_util::TestDb::migrated("telcharge").await else {
            return;
        };
        let vin = "5YJ3E1EA7KF000127";
        let identity = || VehicleIdentity {
            vin,
            tesla_id: Some(1),
            legacy_vehicle_id: None,
            name: Some(vin),
        };
        let (home,): (i64,) = sqlx::query_as(
            "INSERT INTO geofences (name, latitude, longitude, radius_m)
             VALUES ('Home', 10.0, 20.0, 50) RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let archive = super::Archive::new(
            db.pool.clone(),
            crate::geocode::Geocoder::disabled(),
            tokio::sync::broadcast::channel(64).0,
            crate::drive_gps::GpsBoard::default(),
        );
        let start = chrono::Utc::now() - chrono::Duration::hours(3);
        // The car parked at home before the logger last restarted.
        let mut parked =
            crate::state::Observation::at(start - chrono::Duration::minutes(30), Presence::Online);
        parked.shift_state = Some("P".to_owned());
        parked.speed_kmh = Some(0.0);
        parked.latitude = Some(10.0);
        parked.longitude = Some(20.0);
        parked.battery_level = Some(59.0);
        super::apply_for_vin(&archive, vin, identity(), parked, true, false)
            .await
            .unwrap();
        archive.runtimes.lock().await.clear();

        let source = ListOnly {
            vin,
            observed: std::sync::atomic::AtomicUsize::new(0),
        };
        let minutes = 138_i64;
        for minute in 0..=minutes {
            let mut obs = crate::state::Observation::at(
                start + chrono::Duration::minutes(minute),
                Presence::Online,
            );
            let done = minute == minutes;
            obs.charging_state = Some(if done { "Complete" } else { "Charging" }.to_owned());
            obs.charger_power_kw = Some(if done { 0.0 } else { 7.0 });
            obs.shift_state = Some("P".to_owned());
            let step = f64::from(u32::try_from(minute).unwrap());
            obs.battery_level = Some((59.0 + 21.0 * step / 138.0).round());
            apply_telemetry_for_vin(&archive, vin, identity(), obs, false)
                .await
                .unwrap();
            if minute % 12 == 6 {
                if let Some(slot) = archive.runtimes.lock().await.get(vin).cloned() {
                    slot.lock().await.next_due = tokio::time::Instant::now();
                }
                super::run_cycle(&source, &archive, &policy(), None)
                    .await
                    .expect("list cycle");
            }
        }
        assert_eq!(
            source.observed.load(std::sync::atomic::Ordering::Relaxed),
            0,
            "fresh telemetry skips vehicle_data"
        );
        type ChargeRow = (
            chrono::DateTime<chrono::Utc>,
            Option<chrono::DateTime<chrono::Utc>>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<f64>,
            Option<i64>,
        );
        let rows: Vec<ChargeRow> = sqlx::query_as(
            "SELECT c.start_at, c.end_at, c.energy_added_kwh, c.start_battery_level,
                    c.end_battery_level, c.latitude, c.geofence_id
             FROM charges c JOIN vehicles v ON v.id = c.vehicle_id
             WHERE v.vin = $1 AND c.deleted_at IS NULL
             ORDER BY c.start_at",
        )
        .bind(vin)
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 1, "one continuous charge, got {rows:?}");
        let (start_at, end_at, energy, start_soc, end_soc, latitude, fence) = rows[0];
        let ms = |at: chrono::DateTime<chrono::Utc>| at.timestamp_millis();
        assert_eq!(ms(start_at), ms(start));
        let end_at = end_at.expect("closed on Complete");
        assert_eq!(ms(end_at), ms(start + chrono::Duration::minutes(minutes)));
        let kwh = energy.expect("energy from ACChargingPower");
        assert!((kwh - 7.0 * 138.0 / 60.0).abs() < 0.05, "{kwh}");
        assert_eq!((start_soc, end_soc), (Some(59.0), Some(80.0)));
        assert_eq!(latitude, Some(10.0), "last known car position");
        assert_eq!(fence, Some(home));
        let sources: Vec<(String,)> = sqlx::query_as(
            "SELECT DISTINCT c.source FROM charges c
             JOIN vehicles v ON v.id = c.vehicle_id WHERE v.vin = $1
             UNION
             SELECT DISTINCT s.source FROM charge_samples s
             JOIN charges c ON c.id = s.charge_id
             JOIN vehicles v ON v.id = c.vehicle_id WHERE v.vin = $1",
        )
        .bind(vin)
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(sources, vec![("telemetry".to_owned(),)]);
        let (is_dc,): (Option<bool>,) = sqlx::query_as(
            "SELECT c.is_dc FROM charges c JOIN vehicles v ON v.id = c.vehicle_id WHERE v.vin = $1",
        )
        .bind(vin)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(is_dc, Some(false), "a 7 kW telemetry charge is AC");
    }

    type DriveEnd = (
        Option<chrono::DateTime<chrono::Utc>>,
        Option<f64>,
        Option<f64>,
    );

    #[tokio::test]
    async fn telemetry_drive_closes_when_gear_and_speed_go_quiet_and_keeps_its_power() {
        use crate::ingest::apply_telemetry_for_vin;
        use crate::persist::VehicleIdentity;

        let Some(db) = crate::test_util::TestDb::migrated("teldriveend").await else {
            return;
        };
        let vin = "5YJ3E1EA7KF000128";
        let identity = || VehicleIdentity {
            vin,
            tesla_id: Some(1),
            legacy_vehicle_id: None,
            name: Some(vin),
        };
        let archive = super::Archive::new(
            db.pool.clone(),
            crate::geocode::Geocoder::disabled(),
            tokio::sync::broadcast::channel(64).0,
            crate::drive_gps::GpsBoard::default(),
        );
        // Only a fresh telemetry sample may open a drive.
        let start = chrono::Utc::now();
        // Ten minutes of driving, one folded telemetry message a minute.
        for minute in 0..=10_i32 {
            let mut obs = crate::state::Observation::at(
                start + chrono::Duration::minutes(i64::from(minute)),
                Presence::Online,
            );
            obs.shift_state = Some("D".to_owned());
            obs.speed_kmh = Some(50.0);
            obs.power_kw = Some(12.0);
            obs.odometer_km = Some(1_000.0 + f64::from(minute) * 0.8);
            obs.latitude = Some(10.0 + f64::from(minute) * 0.001);
            obs.longitude = Some(20.0);
            apply_telemetry_for_vin(&archive, vin, identity(), obs, true)
                .await
                .unwrap();
        }
        // Parked and awake: Gear and VehicleSpeed come back invalid, Soc keeps coming.
        for minute in 11..=40_i64 {
            let mut obs = crate::state::Observation::at(
                start + chrono::Duration::minutes(minute),
                Presence::Online,
            );
            obs.battery_level = Some(70.0);
            obs.odometer_km = Some(1_008.0);
            apply_telemetry_for_vin(&archive, vin, identity(), obs, false)
                .await
                .unwrap();
        }
        let drives: Vec<DriveEnd> = sqlx::query_as(
            "SELECT d.end_at, d.max_power_kw, d.energy_used_kwh FROM drives d
             JOIN vehicles v ON v.id = d.vehicle_id
             WHERE v.vin = $1 AND d.deleted_at IS NULL",
        )
        .bind(vin)
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(drives.len(), 1, "{drives:?}");
        let (end_at, max_power, energy) = drives[0];
        let energy = energy.expect("energy from the pack power");
        assert!((energy - 12.0 * 10.0 / 60.0).abs() < 1e-6, "{energy}");
        let end_at = end_at.expect("the drive is closed");
        assert_eq!(
            end_at.timestamp_millis(),
            (start + chrono::Duration::minutes(10)).timestamp_millis(),
            "ends at the last driving sample, not at the parked messages"
        );
        assert_eq!(max_power, Some(12.0));
    }

    #[tokio::test]
    async fn sentry_pings_after_a_rolling_park_do_not_keep_the_drive_open() {
        // Production sequence of drive 5145 (27/11, Paris time), neutral coordinates:
        // driving from 11:47:29, the only Gear=P arrives with VehicleSpeed 5 km/h at
        // 11:53:30, speed 1 mph at 11:53:35, then Sentry sends Location pings with no
        // Gear and no speed; the folded observation still holds P and the last speed.
        use crate::ingest::apply_telemetry_for_vin;
        use crate::persist::VehicleIdentity;

        let Some(db) = crate::test_util::TestDb::migrated("telsentry").await else {
            return;
        };
        let vin = "5YJ3E1EA7KF000129";
        let identity = || VehicleIdentity {
            vin,
            tesla_id: Some(1),
            legacy_vehicle_id: None,
            name: Some(vin),
        };
        let archive = super::Archive::new(
            db.pool.clone(),
            crate::geocode::Geocoder::disabled(),
            tokio::sync::broadcast::channel(64).0,
            crate::drive_gps::GpsBoard::default(),
        );
        let start = chrono::Utc::now();
        let secs = |n: i64| start + chrono::Duration::seconds(n);
        let send = |at, shift: &str, speed: f64, step: f64| {
            let mut obs = crate::state::Observation::at(at, Presence::Online);
            obs.shift_state = Some(shift.to_owned());
            obs.speed_kmh = Some(speed);
            obs.odometer_km = Some(1_000.0 + step * 0.05);
            obs.latitude = Some(10.0 + step * 0.0005);
            obs.longitude = Some(20.0);
            obs
        };
        // 11:47:29 → 11:53:29, one fix every 10 s.
        for step in 0..=36_i32 {
            let obs = send(secs(i64::from(step) * 10), "D", 30.0, f64::from(step));
            apply_telemetry_for_vin(&archive, vin, identity(), obs, true)
                .await
                .unwrap();
        }
        let park_at = secs(361); // 11:53:30
        for (at, speed) in [(park_at, 5.0), (secs(366), 1.609)] {
            let obs = send(at, "P", speed, 37.0);
            apply_telemetry_for_vin(&archive, vin, identity(), obs, true)
                .await
                .unwrap();
        }
        // Sentry pings every 3 min 33 s until 13:27:30; P and 1 mph are held.
        let mut at = secs(366);
        while at < secs(5_999) {
            at += chrono::Duration::seconds(213);
            let mut obs = send(at, "P", 1.609, 37.0);
            obs.longitude = Some(20.00001);
            apply_telemetry_for_vin(&archive, vin, identity(), obs, true)
                .await
                .unwrap();
        }
        let drives: Vec<(Option<chrono::DateTime<chrono::Utc>>,)> = sqlx::query_as(
            "SELECT d.end_at FROM drives d JOIN vehicles v ON v.id = d.vehicle_id
             WHERE v.vin = $1 AND d.deleted_at IS NULL",
        )
        .bind(vin)
        .fetch_all(&db.pool)
        .await
        .unwrap();
        assert_eq!(drives.len(), 1, "{drives:?}");
        let end_at = drives[0].0.expect("the drive is closed");
        assert_eq!(end_at.timestamp_millis(), park_at.timestamp_millis());
        let (attached_after,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM positions p JOIN drives d ON d.id = p.drive_id
             JOIN vehicles v ON v.id = d.vehicle_id
             WHERE v.vin = $1 AND p.recorded_at > d.end_at",
        )
        .bind(vin)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(attached_after, 0, "the ping tail is detached");
    }

    #[tokio::test]
    async fn a_drive_left_open_by_sentry_pings_closes_after_a_restart() {
        // Drive 5145 as stored by 8d9d175: P at 5 km/h then 1 mph, followed by
        // gearless, speedless Sentry pings still attached. The new binary must
        // close it on its own at the first P and detach the pings.
        use crate::ingest::apply_telemetry_for_vin;
        use crate::persist::VehicleIdentity;

        let Some(db) = crate::test_util::TestDb::migrated("telrestart").await else {
            return;
        };
        let vin = "5YJ3E1EA7KF000130";
        let identity = || VehicleIdentity {
            vin,
            tesla_id: Some(1),
            legacy_vehicle_id: None,
            name: Some(vin),
        };
        let archive = super::Archive::new(
            db.pool.clone(),
            crate::geocode::Geocoder::disabled(),
            tokio::sync::broadcast::channel(64).0,
            crate::drive_gps::GpsBoard::default(),
        );
        let start = chrono::Utc::now();
        let secs = |n: i64| start + chrono::Duration::seconds(n);
        for (step, shift, speed) in (0..=36_i32)
            .map(|step| (step, "D", 30.0))
            .chain([(37, "P", 5.0)])
        {
            let mut obs = crate::state::Observation::at(
                if step == 37 {
                    secs(361)
                } else {
                    secs(i64::from(step) * 10)
                },
                Presence::Online,
            );
            obs.shift_state = Some(shift.to_owned());
            obs.speed_kmh = Some(speed);
            obs.odometer_km = Some(1_000.0 + f64::from(step) * 0.05);
            obs.latitude = Some(10.0 + f64::from(step) * 0.0005);
            obs.longitude = Some(20.0);
            apply_telemetry_for_vin(&archive, vin, identity(), obs, true)
                .await
                .unwrap();
        }
        let (vehicle_id, drive_id): (i64, i64) = sqlx::query_as(
            "SELECT d.vehicle_id, d.id FROM drives d JOIN vehicles v ON v.id = d.vehicle_id
             WHERE v.vin = $1 AND d.end_at IS NULL",
        )
        .bind(vin)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        // What 8d9d175 stored after the park: 1 mph in P, then bare pings.
        let mut rows = vec![(secs(366), Some("P"), Some(1.609))];
        rows.extend((1..=27_i64).map(|n| (secs(366 + n * 213), None, None)));
        for (at, shift, speed) in rows {
            sqlx::query(
                "INSERT INTO positions (vehicle_id, recorded_at, latitude, longitude, speed_kmh,
                                        shift_state, drive_id, source)
                 VALUES ($1, $2, 10.0185, 20.0, $3, $4, $5, 'telemetry')",
            )
            .bind(vehicle_id)
            .bind(at)
            .bind(speed)
            .bind(shift)
            .bind(drive_id)
            .execute(&db.pool)
            .await
            .unwrap();
        }
        // Restart: no runtime in memory, then the expiry tick.
        archive.runtimes.lock().await.clear();
        super::expire_drives(&archive, secs(6_100)).await.unwrap();
        let (end_at,): (Option<chrono::DateTime<chrono::Utc>>,) =
            sqlx::query_as("SELECT end_at FROM drives WHERE id = $1")
                .bind(drive_id)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        let end_at = end_at.expect("closed by the expiry tick");
        assert_eq!(end_at.timestamp_millis(), secs(361).timestamp_millis());
        let (attached_after,): (i64,) = sqlx::query_as(
            "SELECT count(*) FROM positions WHERE drive_id = $1 AND recorded_at > $2",
        )
        .bind(drive_id)
        .bind(end_at)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(attached_after, 0, "the ping tail is detached");
    }

    const EXCEEDED_LIMIT_BODY: &str =
        r#"{"response":null,"error":"account disabled: EXCEEDED_LIMIT","error_description":""}"#;

    fn count_paths(requests: &[wiremock::Request], suffix: &str) -> usize {
        requests
            .iter()
            .filter(|request| request.url.path().ends_with(suffix))
            .count()
    }

    #[tokio::test]
    async fn exceeded_limit_403_on_the_list_pauses_until_resume_and_does_not_call_again() {
        let Some(db) = crate::test_util::TestDb::migrated("exceeded").await else {
            return;
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/1/vehicles"))
            .respond_with(ResponseTemplate::new(403).set_body_string(EXCEEDED_LIMIT_BODY))
            .mount(&server)
            .await;
        let client = FleetClient::builder(&server.uri(), BearerToken::new("token"))
            .unwrap()
            .build()
            .unwrap();
        let source = FleetPoll::new(client);
        let archive = super::Archive::new(
            db.pool.clone(),
            crate::geocode::Geocoder::disabled(),
            tokio::sync::broadcast::channel(4).0,
            crate::drive_gps::GpsBoard::default(),
        );
        let first = super::run_cycle(&source, &archive, &policy(), None)
            .await
            .expect("cycle");
        let wait = match first {
            super::CycleControl::Wait(wait) => wait,
            super::CycleControl::Pause(wait) => {
                panic!("billing limit must not use the 30s retry: {wait:?}")
            }
        };
        assert_eq!(wait, Duration::from_secs(3_600), "{wait:?}");
        let second = super::run_cycle(&source, &archive, &policy(), None)
            .await
            .expect("paused cycle");
        assert!(matches!(second, super::CycleControl::Wait(_)));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(count_paths(&requests, "/vehicles"), 1, "the list must stop");
        assert_eq!(count_paths(&requests, "/vehicle_data"), 0);
        crate::budget::resume_billing(&db.pool).await.unwrap();
        let _ = super::run_cycle(&source, &archive, &policy(), None)
            .await
            .expect("resumed cycle");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            count_paths(&requests, "/vehicles"),
            2,
            "resume calls the list again"
        );
        assert_eq!(count_paths(&requests, "/vehicle_data"), 0);
    }

    fn fleet_archive(pool: sqlx::PgPool) -> super::Archive {
        super::Archive::new(
            pool,
            crate::geocode::Geocoder::disabled(),
            tokio::sync::broadcast::channel(4).0,
            crate::drive_gps::GpsBoard::default(),
        )
    }

    fn online_list() -> &'static str {
        r#"{"response":[{"id":1,"vin":"TEST00000000VIN01","state":"online","display_name":"Test"}],"pagination":{"next":null}}"#
    }

    #[tokio::test]
    async fn billing_probe_waits_for_the_interval_and_a_403_reschedules_it() {
        let Some(db) = crate::test_util::TestDb::migrated("probewait").await else {
            return;
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/1/vehicles"))
            .respond_with(ResponseTemplate::new(403).set_body_string(EXCEEDED_LIMIT_BODY))
            .mount(&server)
            .await;
        let client = FleetClient::builder(&server.uri(), BearerToken::new("token"))
            .unwrap()
            .build()
            .unwrap();
        let source = FleetPoll::new(client);
        let archive = fleet_archive(db.pool.clone());
        let mut plan = policy();
        plan.blocked_probe = Duration::from_secs(3_600);
        let first = super::run_cycle(&source, &archive, &plan, None)
            .await
            .expect("block");
        assert!(matches!(
            first,
            super::CycleControl::Wait(wait) if wait == Duration::from_secs(3_600)
        ));
        let _ = super::run_cycle(&source, &archive, &plan, None)
            .await
            .expect("not due");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        sqlx::query("UPDATE fleet_control SET next_probe_at = now() - interval '1 second'")
            .execute(&db.pool)
            .await
            .unwrap();
        let again = super::run_cycle(&source, &archive, &plan, None)
            .await
            .expect("probe");
        assert!(matches!(
            again,
            super::CycleControl::Wait(wait) if wait == Duration::from_secs(3_600)
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        let _ = super::run_cycle(&source, &archive, &plan, None)
            .await
            .expect("rescheduled");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            2,
            "a 403 probe waits another interval"
        );
    }

    #[tokio::test]
    async fn successful_list_probe_resumes_polling() {
        let Some(db) = crate::test_util::TestDb::migrated("proberesume").await else {
            return;
        };
        let vin = "TEST00000000VIN01";
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/1/vehicles"))
            .respond_with(ResponseTemplate::new(403).set_body_string(EXCEEDED_LIMIT_BODY))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/1/vehicles"))
            .respond_with(ResponseTemplate::new(200).set_body_string(online_list()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/1/vehicles/{vin}")))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"response":{"id":1,"vin":"TEST00000000VIN01","state":"online"}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/1/vehicles/{vin}/vehicle_data")))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"response":{"id":1,"vin":"TEST00000000VIN01","drive_state":{"shift_state":"P"}}}"#,
            ))
            .mount(&server)
            .await;
        let client = FleetClient::builder(&server.uri(), BearerToken::new("token"))
            .unwrap()
            .build()
            .unwrap();
        let source = FleetPoll::new(client);
        let archive = fleet_archive(db.pool.clone());
        let _ = super::run_cycle(&source, &archive, &policy(), None)
            .await
            .expect("block");
        sqlx::query("UPDATE fleet_control SET next_probe_at = now() - interval '1 second'")
            .execute(&db.pool)
            .await
            .unwrap();
        let _ = super::run_cycle(&source, &archive, &policy(), None)
            .await
            .expect("probe succeeds");
        let gate = crate::budget::gate(
            &db.pool,
            crate::budget::BudgetQuery::caps(160, 5_000, chrono::Utc::now()),
        )
        .await;
        assert!(gate.block.is_none(), "a successful probe clears the block");
        let requests = server.received_requests().await.unwrap();
        assert!(
            count_paths(&requests, "/vehicle_data") >= 1,
            "normal polling resumes on the successful probe"
        );
    }

    #[tokio::test]
    async fn vehicle_data_probe_respects_the_cap() {
        let Some(db) = crate::test_util::TestDb::migrated("probecap").await else {
            return;
        };
        let vin = "TEST00000000VIN01";
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/1/vehicles"))
            .respond_with(ResponseTemplate::new(200).set_body_string(online_list()))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/1/vehicles/{vin}/vehicle_data")))
            .respond_with(ResponseTemplate::new(403).set_body_string(EXCEEDED_LIMIT_BODY))
            .mount(&server)
            .await;
        // Mid-month so the dynamic share does not depend on which day CI runs.
        // A full configured day is always at or over the effective cap, because
        // that cap is never higher than the configured one.
        let now = chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 6, 15, 12, 0, 0)
            .single()
            .expect("date");
        let mut plan = policy();
        plan.now = Some(now);
        plan.daily_cap = 160;
        plan.monthly_cap = 5_000;
        plan.dynamic_budget = true;
        plan.blocked_probe = Duration::from_secs(3_600);
        sqlx::query(
            "INSERT INTO fleet_usage (day, endpoint, status, count)
             VALUES ($1, 'vehicle_data', 200, $2)",
        )
        .bind(now.date_naive())
        .bind(i64::try_from(plan.daily_cap).unwrap_or(i64::MAX))
        .execute(&db.pool)
        .await
        .unwrap();
        crate::budget::enter_block(&db.pool, now, plan.blocked_probe, false)
            .await
            .unwrap();
        sqlx::query("UPDATE fleet_control SET next_probe_at = $1")
            .bind(now - chrono::Duration::seconds(1))
            .execute(&db.pool)
            .await
            .unwrap();
        let client = FleetClient::builder(&server.uri(), BearerToken::new("token"))
            .unwrap()
            .build()
            .unwrap();
        let source = FleetPoll::new(client);
        let archive = fleet_archive(db.pool.clone());
        let _ = super::run_cycle(&source, &archive, &plan, None)
            .await
            .expect("capped probe");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(count_paths(&requests, "/vehicles"), 1);
        assert_eq!(count_paths(&requests, "/vehicle_data"), 0);
        let gate = crate::budget::gate(&db.pool, super::budget_query(&plan, now)).await;
        let block = gate.block.expect("still blocked");
        assert!(block.next_probe > now);
        assert!(gate.cap_reached());
        let _ = super::run_cycle(&source, &archive, &plan, None)
            .await
            .expect("not due");
        assert_eq!(
            count_paths(&server.received_requests().await.unwrap(), "/vehicle_data"),
            0
        );
    }

    #[tokio::test]
    async fn other_403_uses_the_rate_limit_backoff_not_a_30s_retry() {
        let Some(db) = crate::test_util::TestDb::migrated("forbid").await else {
            return;
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/1/vehicles"))
            .respond_with(ResponseTemplate::new(403).set_body_string(
                r#"{"error":"invalid_scope","error_description":"missing vehicle_device_data"}"#,
            ))
            .mount(&server)
            .await;
        let client = FleetClient::builder(&server.uri(), BearerToken::new("token"))
            .unwrap()
            .build()
            .unwrap();
        let source = FleetPoll::new(client);
        let archive = super::Archive::new(
            db.pool.clone(),
            crate::geocode::Geocoder::disabled(),
            tokio::sync::broadcast::channel(4).0,
            crate::drive_gps::GpsBoard::default(),
        );
        let first = super::run_cycle(&source, &archive, &policy(), Some(1))
            .await
            .expect("cycle");
        let wait = match first {
            super::CycleControl::Pause(wait) => wait,
            super::CycleControl::Wait(wait) => panic!("other 403 must back off, not wait {wait:?}"),
        };
        assert!(wait >= Duration::from_secs(60), "{wait:?}");
        assert_ne!(wait, Duration::from_secs(30));
        let _ = super::run_cycle(&source, &archive, &policy(), Some(1))
            .await
            .expect("backoff holds");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests.len(),
            1,
            "the backoff must not call again immediately"
        );
        assert_eq!(count_paths(&requests, "/vehicle_data"), 0);
    }

    #[tokio::test]
    async fn paused_error_retry_does_not_call_vehicle_data() {
        let Some(db) = crate::test_util::TestDb::migrated("pausedata").await else {
            return;
        };
        let vin = "TEST00000000VIN01";
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/1/vehicles"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"response":[{"id":1,"vin":"TEST00000000VIN01","state":"online","display_name":"Test"}],"pagination":{"next":null}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/1/vehicles/{vin}")))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"response":{"id":1,"vin":"TEST00000000VIN01","state":"online"}}"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/1/vehicles/{vin}/vehicle_data")))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"response":{"id":1,"vin":"TEST00000000VIN01","drive_state":{"shift_state":"P"}}}"#,
            ))
            .mount(&server)
            .await;
        crate::budget::enter_block(
            &db.pool,
            chrono::Utc::now(),
            Duration::from_secs(3_600),
            false,
        )
        .await
        .unwrap();
        let client = FleetClient::builder(&server.uri(), BearerToken::new("token"))
            .unwrap()
            .build()
            .unwrap();
        let source = FleetPoll::new(client);
        let archive = super::Archive::new(
            db.pool.clone(),
            crate::geocode::Geocoder::disabled(),
            tokio::sync::broadcast::channel(4).0,
            crate::drive_gps::GpsBoard::default(),
        );
        let _ = super::run_cycle(&source, &archive, &policy(), None)
            .await
            .expect("paused cycle");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(count_paths(&requests, "/vehicles"), 1);
        assert_eq!(
            count_paths(&requests, "/vehicle_data"),
            0,
            "a paused retry must not call vehicle_data"
        );
    }
}
