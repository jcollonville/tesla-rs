// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Optional Owner API streaming.
//!
//! Fleet API polling stays the source for charge sessions, climate, and sleep.
//! This task adds dense positions, speed, power, and shift state while the car
//! is driving, and it opens a drive as soon as the stream reports a gear.
//! The legacy streaming socket is unofficial. Failures back off and leave polling
//! running.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use tesla_auth::{Scope, TokenSet};
use tesla_owner_api::{
    FailureClass, OwnerClient, OwnerError, OwnerRegion, OwnerTokens, PumpEnd, STREAM_COLUMNS,
    SecretString, StreamEvent, StreamSample, StreamStop, StreamTrace, plan_reconnect, pump_stream,
};
use tokio::sync::{Mutex, broadcast, mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::config::Config;
use crate::crypto::TokenFile;
use crate::debug::{DebugBoard, OwnerCount, OwnerNote};
use crate::ingest::{Archive, apply_stream_for_vin};
use crate::persist::{StreamFix, VehicleIdentity, upsert_vehicle};
use crate::units::{MILES_TO_KM, miles_to_km};

const FEET_TO_M: f64 = 0.3048;

/// In-memory streaming status for the state API. Tokens are never stored here.
#[derive(Clone, Default)]
pub(crate) struct StreamBoard {
    inner: Arc<Mutex<BoardState>>,
}

#[derive(Default)]
struct BoardState {
    enabled: bool,
    configured: bool,
    vehicles: HashMap<i64, Slot>,
    logged_disabled: bool,
}

struct Slot {
    status: String,
    detail: Option<String>,
}

/// Point-in-time view used by the status page and the state API.
#[derive(Clone, Debug)]
pub(crate) struct StreamSnapshot {
    enabled: bool,
    configured: bool,
    vehicles: HashMap<i64, (String, Option<String>)>,
}

impl StreamSnapshot {
    /// `connected`, `disabled`, or `error` for one vehicle.
    pub(crate) fn for_vehicle(&self, vehicle_id: i64) -> (String, Option<String>) {
        if !self.enabled || !self.configured {
            return ("disabled".to_owned(), None);
        }
        self.vehicles
            .get(&vehicle_id)
            .cloned()
            .unwrap_or_else(|| ("error".to_owned(), Some("connecting".to_owned())))
    }

    /// Page-level status. Per-vehicle state is on [`Self::for_vehicle`].
    pub(crate) fn summary(&self) -> &'static str {
        if !self.enabled || !self.configured {
            return "disabled";
        }
        if self.vehicles.is_empty() {
            return "error";
        }
        if self
            .vehicles
            .values()
            .all(|(status, _)| status == "connected")
        {
            "connected"
        } else {
            "error"
        }
    }
}

impl StreamBoard {
    pub(crate) async fn set_mode(&self, enabled: bool, configured: bool) {
        let mut guard = self.inner.lock().await;
        if !enabled || !configured {
            guard.vehicles.clear();
            if !guard.logged_disabled {
                tracing::info!(status = "disabled", "owner stream status");
                guard.logged_disabled = true;
            }
        } else {
            guard.logged_disabled = false;
        }
        guard.enabled = enabled;
        guard.configured = configured;
    }

    pub(crate) async fn set_vehicle(&self, vehicle_id: i64, status: &str, detail: Option<String>) {
        let mut guard = self.inner.lock().await;
        let changed = guard
            .vehicles
            .get(&vehicle_id)
            .is_none_or(|slot| slot.status != status);
        if changed {
            tracing::info!(vehicle_id, status, "owner stream status");
        }
        guard.vehicles.insert(
            vehicle_id,
            Slot {
                status: status.to_owned(),
                detail,
            },
        );
        guard.logged_disabled = false;
    }

    pub(crate) async fn snapshot(&self) -> StreamSnapshot {
        let guard = self.inner.lock().await;
        StreamSnapshot {
            enabled: guard.enabled,
            configured: guard.configured,
            vehicles: guard
                .vehicles
                .iter()
                .map(|(id, slot)| (*id, (slot.status.clone(), slot.detail.clone())))
                .collect(),
        }
    }
}

enum TokenFileState {
    Absent,
    Ready,
    Unreadable(String),
}

enum SessionError {
    Missing,
    LoginRequired,
    Other(String),
}

struct OwnerSession {
    client: OwnerClient,
    store: TokenFile,
    lock: Mutex<()>,
    debug: DebugBoard,
}

impl OwnerSession {
    async fn access_token(&self) -> Result<SecretString, SessionError> {
        let _guard = self.lock.lock().await;
        self.load_token(false).await
    }

    async fn force_refresh(&self) -> Result<(), SessionError> {
        let _guard = self.lock.lock().await;
        self.load_token(true).await.map(|_| ())
    }

    async fn load_token(&self, force: bool) -> Result<SecretString, SessionError> {
        let stored = self
            .store
            .load()
            .await
            .map_err(|err| SessionError::Other(err.to_string()))?
            .ok_or(SessionError::Missing)?;
        let current = OwnerTokens::from_parts(
            stored.access_token().clone(),
            stored.refresh_token().cloned(),
            stored.expires_at(),
        );
        if !force && !current.needs_refresh(Duration::from_secs(60), SystemTime::now()) {
            return Ok(current.access_token().clone());
        }
        let Some(refresh) = current.refresh_token().cloned() else {
            return Err(SessionError::LoginRequired);
        };
        let updated = match self.client.refresh(&refresh).await {
            Ok(updated) => updated,
            Err(err) => {
                self.debug.token_failed("owner", &err.to_string());
                return Err(session_err(&err));
            }
        };
        let saved = TokenSet::bearer(
            updated.access_token().clone(),
            updated.refresh_token().cloned(),
            updated.expires_at(),
            owner_scopes(),
        );
        if let Err(err) = self.store.save(&saved).await {
            self.debug
                .token_failed("owner", "owner token file could not be saved");
            return Err(SessionError::Other(err.to_string()));
        }
        Ok(updated.access_token().clone())
    }
}

struct Target {
    db_id: i64,
    vin: String,
    stream_tag: i64,
}

#[derive(Default)]
struct Carry {
    shift: Option<String>,
    speed_kmh: Option<f64>,
    power_kw: Option<f64>,
}

pub(crate) async fn run(
    config: &Config,
    archive: Arc<Archive>,
    board: StreamBoard,
    debug: DebugBoard,
    mut tokens: watch::Receiver<u64>,
    mut shutdown: watch::Receiver<bool>,
) {
    if !config.owner_stream_enabled {
        board.set_mode(false, false).await;
        debug.set_owner_overview("disabled", None, true);
        return;
    }
    loop {
        if *shutdown.borrow() {
            break;
        }
        match tokens_ready(config).await {
            TokenFileState::Absent => {
                board.set_mode(true, false).await;
                debug.set_owner_overview(
                    "disabled",
                    Some("owner token file is absent".to_owned()),
                    true,
                );
                if !wait_signal(&mut tokens, &mut shutdown, None).await {
                    break;
                }
                continue;
            }
            TokenFileState::Unreadable(message) => {
                let message = crate::logs::redact_line(&message, &[]);
                tracing::warn!(error = %message, "owner token file could not be read");
                board.set_mode(true, true).await;
                debug.set_owner_overview("error", Some(message), true);
                if !wait_signal(&mut tokens, &mut shutdown, Some(Duration::from_secs(900))).await {
                    break;
                }
                continue;
            }
            TokenFileState::Ready => {
                board.set_mode(true, true).await;
                debug.set_owner_overview("connecting", None, false);
            }
        }
        let (stop_tx, stop_rx) = watch::channel(false);
        let task = tokio::spawn(supervise(
            config.region,
            config.owner_token_store_path.clone(),
            config.token_encryption_key.clone(),
            Arc::clone(&archive),
            board.clone(),
            debug.clone(),
            stop_rx,
        ));
        let abort = task.abort_handle();
        tokio::pin!(task);
        enum Next {
            Stop,
            Reload,
            Ended,
        }
        let next = tokio::select! {
            _ = shutdown.changed() => {
                let _ = stop_tx.send(true);
                abort.abort();
                Next::Stop
            }
            changed = tokens.changed() => {
                let _ = stop_tx.send(true);
                abort.abort();
                if changed.is_err() { Next::Stop } else { Next::Reload }
            }
            _ = task.as_mut() => Next::Ended,
        };
        // Join before starting another session so two refreshes cannot rotate one token.
        if !matches!(next, Next::Ended) {
            let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        }
        match next {
            Next::Stop => break,
            Next::Reload => {}
            Next::Ended => {
                if !wait_signal(&mut tokens, &mut shutdown, Some(Duration::from_secs(5))).await {
                    break;
                }
            }
        }
    }
}

async fn supervise(
    region: tesla_auth::Region,
    token_path: std::path::PathBuf,
    encryption_key: Option<String>,
    archive: Arc<Archive>,
    board: StreamBoard,
    debug: DebugBoard,
    mut stop: watch::Receiver<bool>,
) {
    if *stop.borrow() {
        return;
    }
    let client = match OwnerClient::new(owner_region(region)) {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(error = %err, "owner stream client could not be built");
            return;
        }
    };
    let store = match TokenFile::open(&token_path, encryption_key.as_deref()) {
        Ok(store) => store,
        Err(err) => {
            tracing::warn!(error = %err, "owner token file could not be opened");
            return;
        }
    };
    let saved = debug.clone();
    store.on_saved(std::sync::Arc::new(move |expires| {
        saved.token_saved("owner", expires);
    }));
    let session = Arc::new(OwnerSession {
        client: client.clone(),
        store,
        lock: Mutex::new(()),
        debug: debug.clone(),
    });
    let found = match targets(&client, &session, &archive).await {
        Ok(found) => found,
        Err(SessionError::LoginRequired) => {
            tracing::warn!(status = "error", class = "auth", "owner stream status");
            let _ = wait_stop(&mut stop, Duration::from_secs(900)).await;
            return;
        }
        Err(SessionError::Missing) => return,
        Err(SessionError::Other(message)) => {
            tracing::warn!(error = %message, "owner vehicle list failed");
            let _ = wait_stop(&mut stop, Duration::from_secs(60)).await;
            return;
        }
    };
    if found.is_empty() {
        tracing::warn!("owner stream has no streaming vehicle id yet");
        let _ = wait_stop(&mut stop, Duration::from_secs(60)).await;
        return;
    }
    let mut tasks = JoinSet::new();
    for target in found {
        board
            .set_vehicle(target.db_id, "error", Some("connecting".to_owned()))
            .await;
        debug.set_owner_vehicle(target.db_id, target.stream_tag, "connecting");
        tasks.spawn(vehicle_loop(
            target,
            Arc::clone(&session),
            Arc::clone(&archive),
            board.clone(),
            debug.clone(),
            client.stream_url().to_owned(),
            stop.clone(),
        ));
    }
    tokio::select! {
        _ = stop.changed() => {
            tasks.abort_all();
        }
        joined = tasks.join_next() => {
            if joined.is_some() {
                tasks.abort_all();
            }
        }
    }
}

async fn vehicle_loop(
    target: Target,
    session: Arc<OwnerSession>,
    archive: Arc<Archive>,
    board: StreamBoard,
    debug: DebugBoard,
    url: String,
    mut stop: watch::Receiver<bool>,
) {
    let mut consecutive = 0_u32;
    let mut carry = Carry::default();
    let mut kick = archive.stream_kick.subscribe();
    loop {
        if *stop.borrow() {
            return;
        }
        let access = match session.access_token().await {
            Ok(token) => token,
            Err(SessionError::Missing) => return,
            Err(err) => {
                let class = match err {
                    SessionError::LoginRequired => FailureClass::Auth,
                    SessionError::Other(message) => {
                        tracing::warn!(
                            vehicle_id = target.db_id,
                            stream_id = target.stream_tag,
                            error = %crate::logs::redact_line(&message, &[]),
                            "owner token was not usable"
                        );
                        FailureClass::Auth
                    }
                    SessionError::Missing => return,
                };
                if !backoff(
                    &mut consecutive,
                    class,
                    &target,
                    &board,
                    &debug,
                    &mut kick,
                    &mut stop,
                    true,
                    None,
                )
                .await
                {
                    return;
                }
                continue;
            }
        };
        board
            .set_vehicle(target.db_id, "error", Some("connecting".to_owned()))
            .await;
        debug.set_owner_vehicle(target.db_id, target.stream_tag, "connecting");
        let (tx, mut rx) = mpsc::channel(32);
        let pump = pump_stream(&url, &access, target.stream_tag, tx);
        tokio::pin!(pump);
        let mut saw = false;
        let mut rx_open = true;
        let end = loop {
            tokio::select! {
                biased;
                _ = stop.changed() => return,
                notice = rx.recv(), if rx_open => {
                    match notice {
                        Some(notice) => {
                            saw = true;
                            accept_notice(&board, &debug, &archive, &target, &mut carry, notice).await;
                        }
                        None => rx_open = false,
                    }
                }
                end = &mut pump => {
                    while let Ok(notice) = rx.try_recv() {
                        saw = true;
                        accept_notice(&board, &debug, &archive, &target, &mut carry, notice).await;
                    }
                    break end;
                }
            }
        };
        let PumpEnd::Failed(stop_info) = end;
        if saw {
            consecutive = 0;
        }
        let class = stop_info.class;
        let refresh = plan_reconnect(class, consecutive.saturating_add(1)).refresh_token;
        if refresh && session.force_refresh().await.is_err() {
            tracing::warn!(
                vehicle_id = target.db_id,
                stream_id = target.stream_tag,
                class = "auth",
                "owner token refresh failed"
            );
        }
        if !backoff(
            &mut consecutive,
            class,
            &target,
            &board,
            &debug,
            &mut kick,
            &mut stop,
            true,
            Some(&stop_info),
        )
        .await
        {
            return;
        }
    }
}

async fn accept_notice(
    board: &StreamBoard,
    debug: &DebugBoard,
    archive: &Archive,
    target: &Target,
    carry: &mut Carry,
    notice: StreamEvent,
) {
    note_stream_event(debug, target, &notice);
    match notice {
        StreamEvent::Subscribed => {
            board.set_vehicle(target.db_id, "connected", None).await;
        }
        StreamEvent::Sample(sample) => {
            board.set_vehicle(target.db_id, "connected", None).await;
            store_sample(debug, archive, target, carry, &sample).await;
        }
        StreamEvent::Trace(_) => {}
    }
}

fn note_stream_event(debug: &DebugBoard, target: &Target, notice: &StreamEvent) {
    match notice {
        StreamEvent::Subscribed => {
            debug.set_owner_vehicle(target.db_id, target.stream_tag, "connected");
            debug.record_event(
                "out",
                "data:subscribe_oauth",
                false,
                &format!(
                    "msg_type=data:subscribe_oauth tag={} columns={STREAM_COLUMNS}",
                    target.stream_tag
                ),
            );
        }
        StreamEvent::Sample(sample) => {
            debug.set_owner_vehicle(target.db_id, target.stream_tag, "connected");
            debug.record_owner(OwnerNote {
                db_id: target.db_id,
                stream_tag: target.stream_tag,
                direction: "in",
                kind: "data:update",
                keepalive: false,
                incoming: true,
                text: &summarize_sample(sample),
                count: OwnerCount::Update,
            });
        }
        StreamEvent::Trace(trace) => note_trace(debug, target, trace),
    }
}

fn note_trace(debug: &DebugBoard, target: &Target, trace: &StreamTrace) {
    let (direction, kind, keepalive, incoming, text, count) = match trace {
        StreamTrace::Hello => (
            "in",
            "control:hello",
            true,
            true,
            "control:hello".to_owned(),
            OwnerCount::HelloPing,
        ),
        StreamTrace::Ping => (
            "in",
            "ping",
            true,
            true,
            "ping".to_owned(),
            OwnerCount::HelloPing,
        ),
        StreamTrace::Pong => (
            "out",
            "pong",
            true,
            false,
            "pong".to_owned(),
            OwnerCount::None,
        ),
        StreamTrace::PongReceived => (
            "in",
            "pong",
            true,
            true,
            "pong".to_owned(),
            OwnerCount::None,
        ),
        StreamTrace::Closed { detail } => {
            ("in", "close", false, true, detail.clone(), OwnerCount::None)
        }
        StreamTrace::Error { error_type, value } => {
            let class = tesla_owner_api::classify_stream_error(error_type, value);
            let text = format!("type={error_type} value={value}");
            debug.note_owner_error(target.db_id, target.stream_tag, class.as_str(), &text);
            (
                "in",
                "data:error",
                false,
                true,
                format!("data:error {text}"),
                OwnerCount::Error,
            )
        }
        StreamTrace::ParseFailed { reason, msg_type } => {
            let text = match msg_type {
                Some(kind) => {
                    format!("frame received but failed to parse: {reason} (msg_type={kind})")
                }
                None => format!("frame received but failed to parse: {reason}"),
            };
            ("in", "parse", false, true, text, OwnerCount::Parse)
        }
        StreamTrace::Ignored { msg_type } => (
            "in",
            msg_type.as_str(),
            false,
            true,
            format!("msg_type={msg_type}"),
            OwnerCount::None,
        ),
    };
    debug.record_owner(OwnerNote {
        db_id: target.db_id,
        stream_tag: target.stream_tag,
        direction,
        kind,
        keepalive,
        incoming,
        text: &text,
        count,
    });
}

fn summarize_sample(sample: &StreamSample) -> String {
    let location = match (sample.est_lat, sample.est_lng) {
        (Some(lat), Some(lng)) => format!("lat={lat:.6} lng={lng:.6}"),
        (Some(_), None) | (None, Some(_)) => "latlng=partial".to_owned(),
        (None, None) => "latlng=absent".to_owned(),
    };
    format!(
        "speed_mph={} soc={} shift={} power_kw={} {location}",
        fmt_num(sample.speed_mph),
        fmt_num(sample.soc),
        sample.shift_state.as_deref().unwrap_or("-"),
        fmt_num(sample.power_kw),
    )
}

fn fmt_num(value: Option<f64>) -> String {
    match value {
        Some(value) => {
            let text = format!("{value:.4}");
            text.trim_end_matches('0').trim_end_matches('.').to_owned()
        }
        None => "-".to_owned(),
    }
}

async fn store_sample(
    debug: &DebugBoard,
    archive: &Archive,
    target: &Target,
    carry: &mut Carry,
    sample: &StreamSample,
) {
    let Some(fix) = consider_sample(debug, target, carry, sample) else {
        return;
    };
    if let Err(err) = apply_stream_for_vin(
        archive,
        &target.vin,
        VehicleIdentity {
            vin: &target.vin,
            tesla_id: None,
            legacy_vehicle_id: None,
            name: None,
        },
        &fix,
    )
    .await
    {
        tracing::warn!(
            vehicle_id = target.db_id,
            stream_id = target.stream_tag,
            error = %crate::logs::redact_line(&err.to_string(), &[]),
            "owner stream sample was not stored"
        );
    }
}

#[allow(clippy::too_many_arguments)]
async fn backoff(
    consecutive: &mut u32,
    class: FailureClass,
    target: &Target,
    board: &StreamBoard,
    debug: &DebugBoard,
    kick: &mut broadcast::Receiver<String>,
    stop: &mut watch::Receiver<bool>,
    count_failure: bool,
    report: Option<&StreamStop>,
) -> bool {
    if count_failure {
        *consecutive = consecutive.saturating_add(1);
    }
    let plan = plan_reconnect(class, (*consecutive).max(1));
    board
        .set_vehicle(target.db_id, "error", Some(class.as_str().to_owned()))
        .await;
    debug.owner_down(target.db_id, target.stream_tag, class.as_str());
    if let Some(report) = report {
        let detail = crate::logs::redact_line(&report.detail, &[]);
        let since = report
            .stats
            .since_last_frame
            .map_or_else(|| "none".to_owned(), |delay| delay.as_millis().to_string());
        tracing::warn!(
            vehicle_id = target.db_id,
            stream_id = target.stream_tag,
            status = "error",
            class = class.as_str(),
            detail = %detail,
            connected_ms = u64::try_from(report.stats.connected.as_millis()).unwrap_or(u64::MAX),
            since_frame_ms = %since,
            hello = report.stats.hello,
            ping = report.stats.ping,
            updates = report.stats.updates,
            errors = report.stats.errors,
            parse_fails = report.stats.parse_failures,
            retry_secs = plan.after.as_secs(),
            "owner stream status"
        );
        debug.record_connection(target.db_id, target.stream_tag, report);
    } else {
        tracing::warn!(
            vehicle_id = target.db_id,
            stream_id = target.stream_tag,
            status = "error",
            class = class.as_str(),
            retry_secs = plan.after.as_secs(),
            "owner stream status"
        );
    }
    let kickable = !matches!(class, FailureClass::Auth | FailureClass::Unavailable);
    let deadline = Instant::now() + plan.after;
    loop {
        if *stop.borrow() {
            return false;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return true;
        }
        tokio::select! {
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return false;
                }
            }
            message = kick.recv() => {
                let ours = match message {
                    Ok(vin) => vin == target.vin,
                    Err(broadcast::error::RecvError::Lagged(_)) => true,
                    Err(broadcast::error::RecvError::Closed) => false,
                };
                if ours && kickable {
                    *consecutive = 0;
                    return true;
                }
            }
            () = sleep(remaining) => return true,
        }
    }
}

async fn targets(
    client: &OwnerClient,
    session: &OwnerSession,
    archive: &Archive,
) -> Result<Vec<Target>, SessionError> {
    let access = session.access_token().await?;
    let mut found = Vec::new();
    match client.list_vehicles(&access).await {
        Ok(vehicles) => {
            for vehicle in vehicles {
                let db_id = upsert_vehicle(
                    &archive.db,
                    VehicleIdentity {
                        vin: &vehicle.vin,
                        tesla_id: None,
                        legacy_vehicle_id: None,
                        name: None,
                    },
                )
                .await
                .map_err(|err| SessionError::Other(err.to_string()))?;
                sqlx::query(
                    "UPDATE vehicles SET legacy_vehicle_id = $2
                     WHERE id = $1 AND legacy_vehicle_id IS NULL",
                )
                .bind(db_id)
                .bind(vehicle.vehicle_id)
                .execute(&archive.db)
                .await
                .map_err(|err| SessionError::Other(err.to_string()))?;
                found.push(Target {
                    db_id,
                    vin: vehicle.vin,
                    stream_tag: vehicle.vehicle_id,
                });
            }
        }
        Err(err) if err.is_login_required() => return Err(SessionError::LoginRequired),
        Err(err) => {
            tracing::warn!(error = %err, "owner vehicle list failed");
        }
    }
    if !found.is_empty() {
        return Ok(found);
    }
    let rows: Vec<(i64, String, i64)> = sqlx::query_as(
        "SELECT id, vin, legacy_vehicle_id FROM vehicles WHERE legacy_vehicle_id IS NOT NULL",
    )
    .fetch_all(&archive.db)
    .await
    .map_err(|err| SessionError::Other(err.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|(db_id, vin, stream_tag)| Target {
            db_id,
            vin,
            stream_tag,
        })
        .collect())
}

fn consider_sample(
    debug: &DebugBoard,
    target: &Target,
    carry: &mut Carry,
    sample: &StreamSample,
) -> Option<StreamFix> {
    if !raw_signal(sample) {
        debug.record_owner(OwnerNote {
            db_id: target.db_id,
            stream_tag: target.stream_tag,
            direction: "note",
            kind: "drop",
            keepalive: false,
            incoming: false,
            text: "sample dropped: all fields empty",
            count: OwnerCount::Drop,
        });
        return None;
    }
    prepare(carry, sample)
}

fn prepare(carry: &mut Carry, sample: &StreamSample) -> Option<StreamFix> {
    if sample.shift_state.is_some() {
        carry.shift.clone_from(&sample.shift_state);
    }
    if let Some(speed) = sample.speed_mph.and_then(mph_to_kmh) {
        carry.speed_kmh = Some(speed);
    }
    if let Some(power) = sample.power_kw.filter(|power| power.is_finite()) {
        carry.power_kw = Some(power);
    }
    if !raw_signal(sample) {
        return None;
    }
    let at = DateTime::<Utc>::from_timestamp_millis(sample.recorded_at_ms)?;
    let (latitude, longitude) = match coordinates(sample.est_lat, sample.est_lng) {
        Some((latitude, longitude)) => (Some(latitude), Some(longitude)),
        None => (None, None),
    };
    Some(StreamFix {
        at,
        shift_state: carry.shift.clone(),
        speed_kmh: carry.speed_kmh,
        power_kw: carry.power_kw,
        odometer_km: sample.odometer_mi.and_then(miles_to_km),
        battery_level: sample
            .soc
            .filter(|level| level.is_finite() && (0.0..=100.0).contains(level)),
        latitude,
        longitude,
        elevation_m: sample.elevation_ft.and_then(feet_to_m),
        heading_deg: sample
            .est_heading
            .or(sample.heading)
            .filter(|heading| heading.is_finite()),
        rated_range_km: sample.range_mi.and_then(miles_to_km),
        est_range_km: sample.est_range_mi.and_then(miles_to_km),
    })
}

fn raw_signal(sample: &tesla_owner_api::StreamSample) -> bool {
    sample.speed_mph.is_some()
        || sample.odometer_mi.is_some()
        || sample.soc.is_some()
        || sample.elevation_ft.is_some()
        || sample.est_heading.is_some()
        || sample.est_lat.is_some()
        || sample.est_lng.is_some()
        || sample.power_kw.is_some()
        || sample.shift_state.is_some()
        || sample.range_mi.is_some()
        || sample.est_range_mi.is_some()
        || sample.heading.is_some()
}

fn coordinates(latitude: Option<f64>, longitude: Option<f64>) -> Option<(f64, f64)> {
    let latitude = latitude?;
    let longitude = longitude?;
    if !latitude.is_finite() || !longitude.is_finite() {
        return None;
    }
    if !(-90.0..=90.0).contains(&latitude) || !(-180.0..=180.0).contains(&longitude) {
        return None;
    }
    if latitude.abs() < 1e-7 && longitude.abs() < 1e-7 {
        return None;
    }
    Some((latitude, longitude))
}

fn mph_to_kmh(mph: f64) -> Option<f64> {
    if mph.is_finite() && mph >= 0.0 {
        Some(mph * MILES_TO_KM)
    } else {
        None
    }
}

fn feet_to_m(feet: f64) -> Option<f64> {
    feet.is_finite().then_some(feet * FEET_TO_M)
}

fn owner_region(region: tesla_auth::Region) -> OwnerRegion {
    match region {
        tesla_auth::Region::China => OwnerRegion::China,
        _ => OwnerRegion::Global,
    }
}

fn owner_scopes() -> Vec<Scope> {
    vec![Scope::OpenId, Scope::Email, Scope::OfflineAccess]
}

fn session_err(err: &OwnerError) -> SessionError {
    if err.is_login_required() {
        SessionError::LoginRequired
    } else {
        SessionError::Other(err.to_string())
    }
}

async fn tokens_ready(config: &Config) -> TokenFileState {
    let store = match TokenFile::open(
        &config.owner_token_store_path,
        config.token_encryption_key.as_deref(),
    ) {
        Ok(store) => store,
        Err(err) => return TokenFileState::Unreadable(err.to_string()),
    };
    match store.load().await {
        Ok(Some(_)) => TokenFileState::Ready,
        Ok(None) => TokenFileState::Absent,
        Err(err) => TokenFileState::Unreadable(err.to_string()),
    }
}

async fn wait_signal(
    tokens: &mut watch::Receiver<u64>,
    shutdown: &mut watch::Receiver<bool>,
    limit: Option<Duration>,
) -> bool {
    let sleep = async {
        if let Some(limit) = limit {
            sleep(limit).await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        _ = shutdown.changed() => false,
        _ = tokens.changed() => true,
        () = sleep => true,
    }
}

async fn wait_stop(stop: &mut watch::Receiver<bool>, limit: Duration) -> bool {
    tokio::select! {
        _ = stop.changed() => false,
        () = sleep(limit) => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Carry, OwnerSession, StreamBoard, Target, consider_sample, note_stream_event, prepare,
        vehicle_loop,
    };
    use tesla_owner_api::StreamSample;

    fn blank() -> StreamSample {
        StreamSample {
            recorded_at_ms: 1_710_000_000_000,
            speed_mph: None,
            odometer_mi: None,
            soc: None,
            elevation_ft: None,
            est_heading: None,
            est_lat: None,
            est_lng: None,
            power_kw: None,
            shift_state: None,
            range_mi: None,
            est_range_mi: None,
            heading: None,
        }
    }

    #[test]
    fn empty_shift_keeps_the_previous_gear_and_converts_imperial_units() {
        let mut carry = Carry::default();
        let mut first = blank();
        first.shift_state = Some("D".to_owned());
        first.speed_mph = Some(10.0);
        first.odometer_mi = Some(1.0);
        first.elevation_ft = Some(10.0);
        first.est_lat = Some(48.1);
        first.est_lng = Some(11.5);
        first.power_kw = Some(-3.0);
        first.range_mi = Some(200.0);
        let prepared = prepare(&mut carry, &first).unwrap();
        assert_eq!(prepared.shift_state.as_deref(), Some("D"));
        assert!((prepared.speed_kmh.unwrap() - 16.09344).abs() < 1e-4);
        assert!((prepared.odometer_km.unwrap() - 1.609_344).abs() < 1e-6);
        assert!((prepared.elevation_m.unwrap() - 3.048).abs() < 1e-6);
        assert_eq!(prepared.power_kw, Some(-3.0));
        assert!((prepared.rated_range_km.unwrap() - 321.8688).abs() < 1e-3);

        let mut held = blank();
        held.recorded_at_ms += 1_000;
        held.est_lat = Some(48.11);
        held.est_lng = Some(11.51);
        let prepared = prepare(&mut carry, &held).unwrap();
        assert_eq!(prepared.shift_state.as_deref(), Some("D"));
        assert!((prepared.speed_kmh.unwrap() - 16.09344).abs() < 1e-4);

        let mut parked = blank();
        parked.recorded_at_ms += 2_000;
        parked.shift_state = Some("P".to_owned());
        parked.speed_mph = Some(0.0);
        assert_eq!(
            prepare(&mut carry, &parked).unwrap().shift_state.as_deref(),
            Some("P")
        );
        assert!(prepare(&mut Carry::default(), &blank()).is_none());
        let mut zero = blank();
        zero.est_lat = Some(0.0);
        zero.est_lng = Some(0.0);
        zero.shift_state = Some("D".to_owned());
        assert!(
            prepare(&mut Carry::default(), &zero)
                .unwrap()
                .latitude
                .is_none()
        );
    }

    #[tokio::test]
    async fn parked_socket_is_connected_before_a_gps_sample() {
        use std::collections::HashMap;
        use std::sync::Arc;
        use std::time::{Duration, SystemTime};

        use futures_util::{SinkExt, StreamExt};
        use tesla_auth::{Scope, TokenSet};
        use tesla_owner_api::{OwnerClient, OwnerRegion, SecretString};
        use tokio::net::TcpListener;
        use tokio_tungstenite::accept_async;
        use tokio_tungstenite::tungstenite::Message;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = accept_async(stream).await.unwrap();
            let message = socket.next().await.unwrap().unwrap();
            let Message::Text(text) = message else {
                panic!("expected text");
            };
            let value: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
            assert_eq!(value["msg_type"], "data:subscribe_oauth");
            assert_eq!(value["tag"], "99");
            socket
                .send(Message::Text(r#"{"msg_type":"control:hello"}"#.into()))
                .await
                .unwrap();
            socket.send(Message::Ping(vec![1].into())).await.unwrap();
            let _ = release_rx.await;
        });

        let dir = std::env::temp_dir().join(format!(
            "tl-stream-{}-{}",
            std::process::id(),
            crate::crypto::random_token().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = crate::crypto::TokenFile::open(dir.join("owner.json"), None).unwrap();
        store
            .save(&TokenSet::bearer(
                SecretString::from("stream-access-token".to_owned()),
                Some(SecretString::from("stream-refresh-token".to_owned())),
                SystemTime::now() + Duration::from_secs(3_600),
                vec![Scope::OpenId, Scope::OfflineAccess],
            ))
            .await
            .unwrap();
        let (events, _) = tokio::sync::broadcast::channel(4);
        let (kick, _) = tokio::sync::broadcast::channel(4);
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .unwrap();
        let archive = Arc::new(crate::ingest::Archive {
            db: pool,
            geocoder: crate::geocode::Geocoder::disabled(),
            events,
            runtimes: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            stream_kick: kick,
        });
        let board = StreamBoard::default();
        board.set_mode(true, true).await;
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let debug = crate::debug::DebugBoard::new(crate::logs::LogBuffer::new(32));
        let task = tokio::spawn(vehicle_loop(
            Target {
                db_id: 1,
                vin: "5YJ3E1EA7KF000001".to_owned(),
                stream_tag: 99,
            },
            Arc::new(OwnerSession {
                client: OwnerClient::new(OwnerRegion::Global).unwrap(),
                store,
                lock: tokio::sync::Mutex::new(()),
                debug: debug.clone(),
            }),
            archive,
            board.clone(),
            debug.clone(),
            format!("ws://{addr}/streaming/"),
            stop_rx,
        ));

        let connected = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let snapshot = board.snapshot().await;
                let (status, detail) = snapshot.for_vehicle(1);
                if status == "connected" {
                    assert!(detail.is_none());
                    assert_eq!(snapshot.summary(), "connected");
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
        let console = debug.joined();
        assert!(console.contains("tag=99"), "{console}");
        assert!(console.contains("data:subscribe_oauth"), "{console}");
        assert!(!console.contains("stream-access-token"), "{console}");
        assert!(!console.contains("5YJ3E1EA7KF000001"), "{console}");
        if connected.is_err() {
            let snapshot = board.snapshot().await;
            let (status, detail) = snapshot.for_vehicle(1);
            panic!(
                "stream status {status} {detail:?} summary {}",
                snapshot.summary()
            );
        }
        let _ = release_tx.send(());
        let _ = stop_tx.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(2), task).await;
        server.abort();
    }

    #[test]
    fn empty_sample_emits_a_debug_drop() {
        let debug = crate::debug::DebugBoard::new(crate::logs::LogBuffer::new(4));
        let target = Target {
            db_id: 3,
            vin: "5YJ3E1EA7KF000001".to_owned(),
            stream_tag: 9,
        };
        assert!(consider_sample(&debug, &target, &mut Carry::default(), &blank()).is_none());
        let text = debug.joined();
        assert!(text.contains("sample dropped: all fields empty"), "{text}");
        assert!(!text.contains(&target.vin), "{text}");
    }

    #[test]
    fn parse_failure_emits_a_debug_event() {
        use tesla_owner_api::{StreamEvent, StreamTrace};

        let debug = crate::debug::DebugBoard::new(crate::logs::LogBuffer::new(4));
        let target = Target {
            db_id: 3,
            vin: "5YJ3E1EA7KF000001".to_owned(),
            stream_tag: 9,
        };
        note_stream_event(
            &debug,
            &target,
            &StreamEvent::Trace(StreamTrace::ParseFailed {
                reason: "too many columns",
                msg_type: Some("data:update".to_owned()),
            }),
        );
        let text = debug.joined();
        assert!(
            text.contains("frame received but failed to parse"),
            "{text}"
        );
        assert!(text.contains("too many columns"), "{text}");
        assert!(text.contains("data:update"), "{text}");
        assert!(!text.contains(&target.vin), "{text}");
    }
}
