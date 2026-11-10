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
    FailureClass, OwnerClient, OwnerError, OwnerRegion, OwnerTokens, PumpEnd, SecretString,
    StreamEvent, plan_reconnect, pump_stream,
};
use tokio::sync::{Mutex, broadcast, mpsc, watch};
use tokio::task::JoinSet;
use tokio::time::sleep;

use crate::config::Config;
use crate::crypto::TokenFile;
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
        let updated = self
            .client
            .refresh(&refresh)
            .await
            .map_err(|err| session_err(&err))?;
        let saved = TokenSet::bearer(
            updated.access_token().clone(),
            updated.refresh_token().cloned(),
            updated.expires_at(),
            owner_scopes(),
        );
        self.store
            .save(&saved)
            .await
            .map_err(|err| SessionError::Other(err.to_string()))?;
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
    mut tokens: watch::Receiver<u64>,
    mut shutdown: watch::Receiver<bool>,
) {
    if !config.owner_stream_enabled {
        board.set_mode(false, false).await;
        return;
    }
    loop {
        if *shutdown.borrow() {
            break;
        }
        match tokens_ready(config).await {
            TokenFileState::Absent => {
                board.set_mode(true, false).await;
                if !wait_signal(&mut tokens, &mut shutdown, None).await {
                    break;
                }
                continue;
            }
            TokenFileState::Unreadable(message) => {
                tracing::warn!(error = %message, "owner token file could not be read");
                board.set_mode(true, true).await;
                if !wait_signal(&mut tokens, &mut shutdown, Some(Duration::from_secs(900))).await {
                    break;
                }
                continue;
            }
            TokenFileState::Ready => board.set_mode(true, true).await,
        }
        let (stop_tx, stop_rx) = watch::channel(false);
        let task = tokio::spawn(supervise(
            config.region,
            config.owner_token_store_path.clone(),
            config.token_encryption_key.clone(),
            Arc::clone(&archive),
            board.clone(),
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
    let session = Arc::new(OwnerSession {
        client: client.clone(),
        store,
        lock: Mutex::new(()),
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
        tasks.spawn(vehicle_loop(
            target,
            Arc::clone(&session),
            Arc::clone(&archive),
            board.clone(),
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
                        tracing::warn!(vin = %target.vin, error = %message, "owner token was not usable");
                        FailureClass::Auth
                    }
                    SessionError::Missing => return,
                };
                if !backoff(
                    &mut consecutive,
                    class,
                    &target,
                    &board,
                    &mut kick,
                    &mut stop,
                    true,
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
        let (tx, mut rx) = mpsc::channel(8);
        let pump = pump_stream(&url, &access, target.stream_tag, tx);
        tokio::pin!(pump);
        let mut saw = false;
        let end = loop {
            tokio::select! {
                biased;
                _ = stop.changed() => return,
                notice = rx.recv() => {
                    let Some(notice) = notice else {
                        break PumpEnd::Failed(FailureClass::Transport);
                    };
                    saw = true;
                    accept_notice(&board, &archive, &target, &mut carry, notice).await;
                }
                end = &mut pump => {
                    while let Ok(notice) = rx.try_recv() {
                        saw = true;
                        accept_notice(&board, &archive, &target, &mut carry, notice).await;
                    }
                    break end;
                }
            }
        };
        let PumpEnd::Failed(class) = end;
        if saw {
            consecutive = 0;
        }
        let refresh = plan_reconnect(class, consecutive.saturating_add(1)).refresh_token;
        if refresh && session.force_refresh().await.is_err() {
            tracing::warn!(vin = %target.vin, class = "auth", "owner token refresh failed");
        }
        if !backoff(
            &mut consecutive,
            class,
            &target,
            &board,
            &mut kick,
            &mut stop,
            true,
        )
        .await
        {
            return;
        }
    }
}

async fn accept_notice(
    board: &StreamBoard,
    archive: &Archive,
    target: &Target,
    carry: &mut Carry,
    notice: StreamEvent,
) {
    match notice {
        StreamEvent::Subscribed => {
            board.set_vehicle(target.db_id, "connected", None).await;
        }
        StreamEvent::Sample(sample) => {
            board.set_vehicle(target.db_id, "connected", None).await;
            store_sample(archive, target, carry, &sample).await;
        }
    }
}

async fn store_sample(
    archive: &Archive,
    target: &Target,
    carry: &mut Carry,
    sample: &tesla_owner_api::StreamSample,
) {
    let Some(fix) = prepare(carry, sample) else {
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
        tracing::warn!(vin = %target.vin, error = %err, "owner stream sample was not stored");
    }
}

async fn backoff(
    consecutive: &mut u32,
    class: FailureClass,
    target: &Target,
    board: &StreamBoard,
    kick: &mut broadcast::Receiver<String>,
    stop: &mut watch::Receiver<bool>,
    count_failure: bool,
) -> bool {
    if count_failure {
        *consecutive = consecutive.saturating_add(1);
    }
    let plan = plan_reconnect(class, (*consecutive).max(1));
    board
        .set_vehicle(target.db_id, "error", Some(class.as_str().to_owned()))
        .await;
    tracing::warn!(
        vin = %target.vin,
        status = "error",
        class = class.as_str(),
        retry_secs = plan.after.as_secs(),
        "owner stream status"
    );
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

fn prepare(carry: &mut Carry, sample: &tesla_owner_api::StreamSample) -> Option<StreamFix> {
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
    use super::{Carry, OwnerSession, StreamBoard, Target, prepare, vehicle_loop};
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
            }),
            archive,
            board.clone(),
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
}
