// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Signed-in operator console: subsystem status, websocket frames, and app logs.
//!
//! Events are redacted before they are stored. Access tokens, refresh tokens, and VINs are not
//! kept in the ring buffer.

use std::collections::{BTreeMap, VecDeque};
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::response::sse::Event;
use chrono::{DateTime, Utc};
use tesla_owner_api::StreamStop;
use tokio::sync::{broadcast, mpsc};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::app::AppState;
use crate::config::Config;
use crate::logs::{LogBuffer, LogLine, redact_debug_text};

const EVENT_CAP: usize = 400;
const KEEPALIVE_CAP: usize = 40;
const HISTORY_CAP: usize = 30;

/// What an owner-stream frame should add to the lifetime counters.
#[derive(Clone, Copy)]
pub(crate) enum OwnerCount {
    None,
    HelloPing,
    Update,
    Error,
    Parse,
    Drop,
}

/// One owner-stream frame or note.
#[derive(Clone, Copy)]
pub(crate) struct OwnerNote<'a> {
    pub db_id: i64,
    pub stream_tag: i64,
    pub direction: &'static str,
    pub kind: &'a str,
    pub keepalive: bool,
    pub incoming: bool,
    pub text: &'a str,
    pub count: OwnerCount,
}

/// Rendered debug page sections, without the toolbar.
pub(crate) struct ConsoleView {
    pub frames_cursor: u64,
    pub logs_cursor: u64,
    pub body: String,
}

#[derive(Clone)]
pub(crate) struct DebugBoard {
    inner: Arc<Mutex<DebugState>>,
    wake: broadcast::Sender<()>,
    logs: LogBuffer,
}

struct DebugState {
    next_id: u64,
    events: VecDeque<DebugEvent>,
    history: VecDeque<CloseRow>,
    owner: OwnerOverview,
    fleet: FleetSlot,
    postgres: ProbeSlot,
    owner_token: TokenSlot,
    fleet_token: TokenSlot,
    mqtt: ServiceSlot,
    telemetry: ServiceSlot,
}

#[derive(Clone)]
struct DebugEvent {
    id: u64,
    at: DateTime<Utc>,
    direction: String,
    kind: String,
    keepalive: bool,
    text: String,
}

struct CloseRow {
    at: DateTime<Utc>,
    text: String,
}

struct OwnerOverview {
    state: String,
    detail: Option<String>,
    vehicles: BTreeMap<i64, OwnerSlot>,
}

struct OwnerSlot {
    stream_tag: i64,
    state: String,
    since: DateTime<Utc>,
    last_error_class: Option<String>,
    last_error: Option<String>,
    last_close: Option<String>,
    reconnects: u64,
    last_frame_at: Option<DateTime<Utc>>,
    last_update_at: Option<DateTime<Utc>>,
    position_mode: String,
    hello_ping: u64,
    updates: u64,
    errors: u64,
    parse_failures: u64,
    empty_drops: u64,
}

struct FleetSlot {
    last_poll_at: Option<DateTime<Utc>>,
    last_status: Option<u16>,
    last_error: Option<String>,
    drive: Duration,
    charge: Duration,
    parked: Duration,
    asleep: Duration,
}

struct ProbeSlot {
    reachable: Option<bool>,
    checked_at: Option<DateTime<Utc>>,
}

struct TokenSlot {
    present: Option<bool>,
    readable: bool,
    expires_at: Option<DateTime<Utc>>,
    last_refresh_at: Option<DateTime<Utc>>,
    last_refresh_ok: Option<bool>,
    last_refresh_error: Option<String>,
}

struct ServiceSlot {
    state: String,
    detail: Option<String>,
    since: Option<DateTime<Utc>>,
}

/// Presence and expiry of one token file. Token strings are not copied.
#[derive(Clone, Copy)]
pub(crate) struct TokenProbe {
    pub present: bool,
    pub readable: bool,
    pub expires_at: Option<DateTime<Utc>>,
}

impl OwnerSlot {
    fn new(stream_tag: i64) -> Self {
        Self {
            stream_tag,
            state: "connecting".to_owned(),
            since: Utc::now(),
            last_error_class: None,
            last_error: None,
            last_close: None,
            reconnects: 0,
            last_frame_at: None,
            last_update_at: None,
            position_mode: "fallback".to_owned(),
            hello_ping: 0,
            updates: 0,
            errors: 0,
            parse_failures: 0,
            empty_drops: 0,
        }
    }
}

impl Default for DebugState {
    fn default() -> Self {
        Self {
            next_id: 1,
            events: VecDeque::new(),
            history: VecDeque::new(),
            owner: OwnerOverview {
                state: "connecting".to_owned(),
                detail: None,
                vehicles: BTreeMap::new(),
            },
            fleet: FleetSlot {
                last_poll_at: None,
                last_status: None,
                last_error: None,
                drive: Duration::ZERO,
                charge: Duration::ZERO,
                parked: Duration::ZERO,
                asleep: Duration::ZERO,
            },
            postgres: ProbeSlot {
                reachable: None,
                checked_at: None,
            },
            owner_token: TokenSlot::unknown(),
            fleet_token: TokenSlot::unknown(),
            mqtt: ServiceSlot::new("disabled"),
            telemetry: ServiceSlot::new("disabled"),
        }
    }
}

impl TokenSlot {
    fn unknown() -> Self {
        Self {
            present: None,
            readable: true,
            expires_at: None,
            last_refresh_at: None,
            last_refresh_ok: None,
            last_refresh_error: None,
        }
    }
}

impl ServiceSlot {
    fn new(state: &str) -> Self {
        Self {
            state: state.to_owned(),
            detail: None,
            since: None,
        }
    }
}

impl DebugBoard {
    pub(crate) fn new(logs: LogBuffer) -> Self {
        let (wake, _) = broadcast::channel(32);
        Self {
            inner: Arc::new(Mutex::new(DebugState::default())),
            wake,
            logs,
        }
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<()> {
        self.wake.subscribe()
    }

    pub(crate) fn apply_config(&self, config: &Config) {
        {
            let mut guard = self.lock();
            guard.fleet.drive = config.drive_poll;
            guard.fleet.charge = config.charge_poll;
            guard.fleet.parked = config.parked_poll;
            guard.fleet.asleep = config.asleep_poll;
        }
        if config.owner_stream_enabled {
            self.set_owner_overview("connecting", None, false);
        } else {
            self.set_owner_overview("disabled", None, true);
        }
        if config.mqtt_enabled {
            self.set_mqtt("connecting", None);
        } else {
            self.set_mqtt("disabled", None);
        }
        if config.telemetry_enabled {
            self.set_telemetry("connecting", None);
        } else {
            self.set_telemetry("disabled", None);
        }
    }

    pub(crate) fn set_owner_overview(&self, state: &str, detail: Option<String>, clear: bool) {
        let detail = detail.map(|text| self.scrub(&text));
        {
            let mut guard = self.lock();
            state.clone_into(&mut guard.owner.state);
            guard.owner.detail = detail;
            if clear {
                guard.owner.vehicles.clear();
            }
        }
        self.notify();
    }

    /// `hybrid` or `fallback`. The label is a fixed word, never a VIN.
    pub(crate) fn set_position_mode(&self, db_id: i64, mode: &str) {
        {
            let mut guard = self.lock();
            let Some(slot) = guard.owner.vehicles.get_mut(&db_id) else {
                return;
            };
            if slot.position_mode == mode {
                return;
            }
            mode.clone_into(&mut slot.position_mode);
        }
        self.notify();
    }

    pub(crate) fn set_owner_vehicle(&self, db_id: i64, stream_tag: i64, state: &str) {
        {
            let mut guard = self.lock();
            let slot = guard
                .owner
                .vehicles
                .entry(db_id)
                .or_insert_with(|| OwnerSlot::new(stream_tag));
            slot.stream_tag = stream_tag;
            if slot.state != state {
                state.clone_into(&mut slot.state);
                slot.since = Utc::now();
            }
        }
        self.notify();
    }

    pub(crate) fn owner_down(&self, db_id: i64, stream_tag: i64, class: &str) {
        let class = self.scrub(class);
        {
            let mut guard = self.lock();
            let slot = guard
                .owner
                .vehicles
                .entry(db_id)
                .or_insert_with(|| OwnerSlot::new(stream_tag));
            slot.stream_tag = stream_tag;
            if slot.state != "error" {
                "error".clone_into(&mut slot.state);
                slot.since = Utc::now();
            }
            slot.last_error_class = Some(class.clone());
            if slot.last_error.is_none() {
                slot.last_error = Some(class);
            }
            slot.reconnects = slot.reconnects.saturating_add(1);
        }
        self.notify();
    }

    pub(crate) fn note_owner_error(&self, db_id: i64, stream_tag: i64, class: &str, message: &str) {
        let class = self.scrub(class);
        let message = self.scrub(message);
        {
            let mut guard = self.lock();
            let slot = guard
                .owner
                .vehicles
                .entry(db_id)
                .or_insert_with(|| OwnerSlot::new(stream_tag));
            slot.stream_tag = stream_tag;
            slot.last_error_class = Some(class);
            slot.last_error = Some(message);
        }
        self.notify();
    }

    pub(crate) fn record_owner(&self, note: OwnerNote<'_>) {
        let text = self.scrub(note.text);
        let kind = kind_of(&text, note.kind);
        {
            let mut guard = self.lock();
            let slot = guard
                .owner
                .vehicles
                .entry(note.db_id)
                .or_insert_with(|| OwnerSlot::new(note.stream_tag));
            slot.stream_tag = note.stream_tag;
            let now = Utc::now();
            if note.incoming {
                slot.last_frame_at = Some(now);
            }
            match note.count {
                OwnerCount::None => {}
                OwnerCount::HelloPing => slot.hello_ping = slot.hello_ping.saturating_add(1),
                OwnerCount::Update => {
                    slot.updates = slot.updates.saturating_add(1);
                    slot.last_update_at = Some(now);
                }
                OwnerCount::Error => slot.errors = slot.errors.saturating_add(1),
                OwnerCount::Parse => slot.parse_failures = slot.parse_failures.saturating_add(1),
                OwnerCount::Drop => slot.empty_drops = slot.empty_drops.saturating_add(1),
            }
            push_event(&mut guard, note.direction, &kind, note.keepalive, text);
        }
        self.notify();
    }

    pub(crate) fn record_event(&self, direction: &str, kind: &str, keepalive: bool, text: &str) {
        let text = self.scrub(text);
        let kind = kind_of(&text, kind);
        {
            let mut guard = self.lock();
            push_event(&mut guard, direction, &kind, keepalive, text);
        }
        self.notify();
    }

    pub(crate) fn record_poll(
        &self,
        method: &str,
        path: &str,
        status: Option<u16>,
        elapsed: Duration,
        summary: &str,
    ) {
        let summary = self.scrub(summary);
        let status_text = status.map_or_else(|| "none".to_owned(), |code| code.to_string());
        let text = format!(
            "{method} {path} status={status_text} {} {summary}",
            fmt_elapsed(elapsed)
        );
        let failed = !matches!(status, Some(code) if (200..300).contains(&code));
        {
            let mut guard = self.lock();
            guard.fleet.last_poll_at = Some(Utc::now());
            guard.fleet.last_status = status;
            if failed {
                guard.fleet.last_error = Some(summary);
            }
            push_event(&mut guard, "poll", "poll", false, text);
        }
        self.notify();
    }

    pub(crate) fn record_connection(&self, db_id: i64, stream_id: i64, stop: &StreamStop) {
        let text = self.scrub(&connection_text(stream_id, stop));
        {
            let mut guard = self.lock();
            if let Some(slot) = guard.owner.vehicles.get_mut(&db_id) {
                slot.last_close = Some(text.clone());
            }
            if guard.history.len() >= HISTORY_CAP {
                guard.history.pop_front();
            }
            guard.history.push_back(CloseRow {
                at: Utc::now(),
                text: text.clone(),
            });
            push_event(&mut guard, "note", "session", false, text);
        }
        self.notify();
    }

    pub(crate) fn set_postgres(&self, reachable: bool) {
        {
            let mut guard = self.lock();
            guard.postgres.reachable = Some(reachable);
            guard.postgres.checked_at = Some(Utc::now());
        }
        self.notify();
    }

    pub(crate) fn set_token_probe(&self, kind: &str, probe: TokenProbe) {
        {
            let mut guard = self.lock();
            let slot = token_mut(&mut guard, kind);
            slot.present = Some(probe.present);
            slot.readable = probe.readable;
            slot.expires_at = probe.expires_at;
        }
        self.notify();
    }

    pub(crate) fn token_saved(&self, kind: &str, expires: SystemTime) {
        {
            let mut guard = self.lock();
            let slot = token_mut(&mut guard, kind);
            slot.present = Some(true);
            slot.readable = true;
            slot.expires_at = system_to_utc(expires);
            slot.last_refresh_at = Some(Utc::now());
            slot.last_refresh_ok = Some(true);
            slot.last_refresh_error = None;
        }
        self.notify();
    }

    pub(crate) fn token_failed(&self, kind: &str, message: &str) {
        let message = self.scrub(message);
        {
            let mut guard = self.lock();
            let slot = token_mut(&mut guard, kind);
            slot.last_refresh_at = Some(Utc::now());
            slot.last_refresh_ok = Some(false);
            slot.last_refresh_error = Some(message);
        }
        self.notify();
    }

    pub(crate) fn set_mqtt(&self, state: &str, detail: Option<String>) {
        self.set_service(true, state, detail);
    }

    pub(crate) fn set_telemetry(&self, state: &str, detail: Option<String>) {
        self.set_service(false, state, detail);
    }

    fn frames_after(&self, seq: u64) -> Vec<DebugEvent> {
        self.lock()
            .events
            .iter()
            .filter(|event| event.id > seq)
            .cloned()
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn joined(&self) -> String {
        self.lock()
            .events
            .iter()
            .map(|event| event.text.clone())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub(crate) fn console(&self, lines: &[LogLine]) -> ConsoleView {
        let guard = self.lock();
        let now = Utc::now();
        let frames_cursor = guard.events.back().map_or(0, |event| event.id);
        let logs_cursor = lines.last().map_or(0, |line| line.seq);
        let mut body = String::new();
        body.push_str("<div id=\"status\" class=\"status-grid\">");
        body.push_str(&render_owner(&guard, now));
        body.push_str(&render_fleet(&guard.fleet, now));
        body.push_str(&render_postgres(&guard.postgres, now));
        body.push_str(&render_token("Owner token store", &guard.owner_token, now));
        body.push_str(&render_token("Fleet token store", &guard.fleet_token, now));
        body.push_str(&render_service("MQTT", &guard.mqtt));
        body.push_str(&render_service("Fleet telemetry", &guard.telemetry));
        body.push_str("</div>");
        body.push_str(&render_history(&guard.history));
        body.push_str(&render_frames(&guard.events));
        body.push_str(&render_logs(lines));
        body.push_str(SCRIPT);
        ConsoleView {
            frames_cursor,
            logs_cursor,
            body,
        }
    }

    fn set_service(&self, mqtt: bool, state: &str, detail: Option<String>) {
        let detail = detail.map(|text| self.scrub(&text));
        {
            let mut guard = self.lock();
            let slot = if mqtt {
                &mut guard.mqtt
            } else {
                &mut guard.telemetry
            };
            if slot.state != state {
                slot.since = Some(Utc::now());
            }
            state.clone_into(&mut slot.state);
            slot.detail = detail;
        }
        self.notify();
    }

    fn scrub(&self, text: &str) -> String {
        let secrets = self.logs.secret_list();
        truncate(redact_debug_text(text, &secrets), 500)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, DebugState> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn notify(&self) {
        let _ = self.wake.send(());
    }
}

pub(crate) async fn probe(app: &AppState) {
    let reachable = matches!(
        tokio::time::timeout(
            Duration::from_millis(800),
            sqlx::query("SELECT 1").execute(&app.inner.db),
        )
        .await,
        Ok(Ok(_))
    );
    app.inner.debug.set_postgres(reachable);
    let key = app.inner.config.token_encryption_key.as_deref();
    let owner = read_token(&app.inner.config.owner_token_store_path, key).await;
    let fleet = read_token(&app.inner.config.token_store_path, key).await;
    app.inner.debug.set_token_probe("owner", owner);
    app.inner.debug.set_token_probe("fleet", fleet);
}

pub(crate) fn event_stream(
    app: AppState,
    frames: u64,
    logs: u64,
) -> impl tokio_stream::Stream<Item = Result<Event, Infallible>> + Send + 'static {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(feed(app, frames, logs, tx));
    ReceiverStream::new(rx).map(Ok)
}

async fn feed(app: AppState, mut frames: u64, mut logs: u64, tx: mpsc::Sender<Event>) {
    let mut wake = app.inner.debug.subscribe();
    let mut ticks = 0_u32;
    loop {
        if !send_status(&tx, &app).await || !send_pending(&tx, &app, &mut frames, &mut logs).await {
            break;
        }
        tokio::select! {
            alive = next_wake(&mut wake) => {
                if !alive {
                    break;
                }
            }
            () = tokio::time::sleep(Duration::from_secs(2)) => {
                ticks = ticks.wrapping_add(1);
                if ticks.is_multiple_of(3) {
                    probe(&app).await;
                }
            }
        }
    }
}

async fn next_wake(wake: &mut broadcast::Receiver<()>) -> bool {
    match wake.recv().await {
        Ok(()) | Err(broadcast::error::RecvError::Lagged(_)) => true,
        Err(broadcast::error::RecvError::Closed) => false,
    }
}

async fn send_status(tx: &mpsc::Sender<Event>, app: &AppState) -> bool {
    let html = status_html(app);
    tx.send(Event::default().event("status").data(html))
        .await
        .is_ok()
}

async fn send_pending(
    tx: &mpsc::Sender<Event>,
    app: &AppState,
    frames: &mut u64,
    logs: &mut u64,
) -> bool {
    for event in app.inner.debug.frames_after(*frames) {
        let id = event.id;
        let sse = Event::default()
            .id(format!("f-{id}"))
            .event("frame")
            .data(render_frame(&event));
        if tx.send(sse).await.is_err() {
            return false;
        }
        *frames = id;
    }
    for line in app.inner.logs.since(*logs) {
        let id = line.seq;
        let sse = Event::default()
            .id(format!("l-{id}"))
            .event("log")
            .data(render_log(&line));
        if tx.send(sse).await.is_err() {
            return false;
        }
        *logs = id;
    }
    true
}

fn status_html(app: &AppState) -> String {
    let guard = app.inner.debug.lock();
    let now = Utc::now();
    let mut html = String::new();
    html.push_str(&render_owner(&guard, now));
    html.push_str(&render_fleet(&guard.fleet, now));
    html.push_str(&render_postgres(&guard.postgres, now));
    html.push_str(&render_token("Owner token store", &guard.owner_token, now));
    html.push_str(&render_token("Fleet token store", &guard.fleet_token, now));
    html.push_str(&render_service("MQTT", &guard.mqtt));
    html.push_str(&render_service("Fleet telemetry", &guard.telemetry));
    html
}

fn push_event(guard: &mut DebugState, direction: &str, kind: &str, keepalive: bool, text: String) {
    make_room(&mut guard.events, keepalive);
    let id = guard.next_id;
    guard.next_id = guard.next_id.saturating_add(1);
    guard.events.push_back(DebugEvent {
        id,
        at: Utc::now(),
        direction: direction.to_owned(),
        kind: kind.to_owned(),
        keepalive,
        text,
    });
}

fn make_room(events: &mut VecDeque<DebugEvent>, keepalive: bool) {
    if keepalive {
        let keep_count = events.iter().filter(|event| event.keepalive).count();
        if keep_count >= KEEPALIVE_CAP
            && let Some(pos) = events.iter().position(|event| event.keepalive)
        {
            events.remove(pos);
        }
    }
    while events.len() >= EVENT_CAP {
        if let Some(pos) = events.iter().position(|event| event.keepalive) {
            events.remove(pos);
        } else {
            events.pop_front();
        }
    }
}

fn kind_of(text: &str, kind: &str) -> String {
    if text.contains("msg_type=data:subscribe_oauth") {
        "data:subscribe_oauth".to_owned()
    } else {
        kind.to_owned()
    }
}

fn token_mut<'a>(guard: &'a mut DebugState, kind: &str) -> &'a mut TokenSlot {
    if kind == "owner" {
        &mut guard.owner_token
    } else {
        &mut guard.fleet_token
    }
}

fn connection_text(stream_id: i64, stop: &StreamStop) -> String {
    let since = stop
        .stats
        .since_last_frame
        .map_or_else(|| "none".to_owned(), |delay| delay.as_millis().to_string());
    format!(
        "connection ended stream_id={stream_id} class={} {} connected_ms={} since_frame_ms={since} hello={} ping={} updates={} errors={} parse_fails={}",
        stop.class.as_str(),
        stop.detail,
        stop.stats.connected.as_millis(),
        stop.stats.hello,
        stop.stats.ping,
        stop.stats.updates,
        stop.stats.errors,
        stop.stats.parse_failures,
    )
}

async fn read_token(path: &std::path::Path, key: Option<&str>) -> TokenProbe {
    let Ok(store) = crate::crypto::TokenFile::open(path, key) else {
        return TokenProbe {
            present: false,
            readable: false,
            expires_at: None,
        };
    };
    match store.load().await {
        Ok(Some(tokens)) => {
            let expires_at = system_to_utc(tokens.expires_at());
            drop(tokens);
            TokenProbe {
                present: true,
                readable: true,
                expires_at,
            }
        }
        Ok(None) => TokenProbe {
            present: false,
            readable: true,
            expires_at: None,
        },
        Err(_) => TokenProbe {
            present: false,
            readable: false,
            expires_at: None,
        },
    }
}

fn system_to_utc(time: SystemTime) -> Option<DateTime<Utc>> {
    let Ok(duration) = time.duration_since(UNIX_EPOCH) else {
        return None;
    };
    let secs = i64::try_from(duration.as_secs()).ok()?;
    DateTime::<Utc>::from_timestamp(secs, duration.subsec_nanos())
}

fn render_owner(guard: &DebugState, now: DateTime<Utc>) -> String {
    if guard.owner.vehicles.is_empty() {
        let detail = guard
            .owner
            .detail
            .clone()
            .unwrap_or_else(|| "no streaming vehicle yet".to_owned());
        return card(
            "Owner stream",
            &guard.owner.state,
            tone(&guard.owner.state),
            &[("Detail", detail)],
        );
    }
    let mut html = String::new();
    for (db_id, slot) in &guard.owner.vehicles {
        let counts = format!(
            "hello/ping {} · updates {} · errors {} · parse failures {} · empty drops {}",
            slot.hello_ping, slot.updates, slot.errors, slot.parse_failures, slot.empty_drops
        );
        html.push_str(&card(
            "Owner stream",
            &slot.state,
            tone(&slot.state),
            &[
                ("Database id", db_id.to_string()),
                ("Stream id", slot.stream_tag.to_string()),
                ("Status since", fmt_since(slot.since, now)),
                (
                    "Last error class",
                    slot.last_error_class
                        .clone()
                        .unwrap_or_else(|| "—".to_owned()),
                ),
                (
                    "Last error",
                    slot.last_error.clone().unwrap_or_else(|| "—".to_owned()),
                ),
                ("Reconnects", slot.reconnects.to_string()),
                ("Last frame", fmt_ago(slot.last_frame_at, now)),
                ("Last data:update", fmt_ago(slot.last_update_at, now)),
                ("Position mode", slot.position_mode.clone()),
                ("Counts", counts),
                (
                    "Last close",
                    slot.last_close.clone().unwrap_or_else(|| "—".to_owned()),
                ),
            ],
        ));
    }
    html
}

fn render_fleet(slot: &FleetSlot, now: DateTime<Utc>) -> String {
    let intervals = format!(
        "drive {} · charge {} · parked {} · asleep {}",
        fmt_duration(slot.drive),
        fmt_duration(slot.charge),
        fmt_duration(slot.parked),
        fmt_duration(slot.asleep)
    );
    card(
        "Fleet poller",
        "poll",
        "neutral",
        &[
            ("Last poll", fmt_ago(slot.last_poll_at, now)),
            (
                "Last HTTP status",
                slot.last_status
                    .map_or_else(|| "—".to_owned(), |code| code.to_string()),
            ),
            ("Intervals", intervals),
            (
                "Last error",
                slot.last_error.clone().unwrap_or_else(|| "—".to_owned()),
            ),
        ],
    )
}

fn render_postgres(slot: &ProbeSlot, now: DateTime<Utc>) -> String {
    let (label, tone_name) = match slot.reachable {
        Some(true) => ("reachable", "ok"),
        Some(false) => ("unreachable", "bad"),
        None => ("unknown", "neutral"),
    };
    card(
        "Postgres",
        label,
        tone_name,
        &[("Checked", fmt_ago(slot.checked_at, now))],
    )
}

fn render_token(title: &str, slot: &TokenSlot, now: DateTime<Utc>) -> String {
    let present = if slot.readable {
        match slot.present {
            Some(true) => "yes",
            Some(false) => "no",
            None => "unknown",
        }
    } else {
        "unreadable"
    };
    let refresh = match (slot.last_refresh_at, slot.last_refresh_ok) {
        (None, _) => "not observed".to_owned(),
        (Some(at), Some(true)) => format!("{} ago, ok", fmt_span((now - at).num_seconds().max(0))),
        (Some(at), Some(false)) => format!(
            "{} ago, failed ({})",
            fmt_span((now - at).num_seconds().max(0)),
            slot.last_refresh_error.as_deref().unwrap_or("error")
        ),
        (Some(at), None) => format!("{} ago", fmt_span((now - at).num_seconds().max(0))),
    };
    let tone_name = if present == "unreadable" {
        "bad"
    } else {
        "neutral"
    };
    card(
        title,
        present,
        tone_name,
        &[
            ("Present", present.to_owned()),
            ("Access expires", fmt_expiry(slot.expires_at, now)),
            ("Last refresh", refresh),
        ],
    )
}

fn render_service(title: &str, slot: &ServiceSlot) -> String {
    let mut rows = vec![(
        "Since",
        slot.since.map_or_else(
            || "—".to_owned(),
            |at| at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        ),
    )];
    if let Some(detail) = &slot.detail {
        rows.push(("Detail", detail.clone()));
    }
    card(title, &slot.state, tone(&slot.state), &rows)
}

fn render_history(rows: &VecDeque<CloseRow>) -> String {
    let mut items = String::new();
    if rows.is_empty() {
        items.push_str("<p class=\"muted\">No connection has ended yet.</p>");
    } else {
        items.push_str("<ol class=\"logs\">");
        for row in rows {
            let _ = std::fmt::Write::write_fmt(
                &mut items,
                format_args!(
                    "<li><time>{}</time> <span class=\"dir note\">END</span> {}</li>",
                    escape(&row.at.format("%H:%M:%S").to_string()),
                    escape(&row.text)
                ),
            );
        }
        items.push_str("</ol>");
    }
    format!("<section class=\"card\"><h2>Connection history</h2>{items}</section>")
}

fn render_frames(events: &VecDeque<DebugEvent>) -> String {
    let mut items = String::new();
    if events.is_empty() {
        items.push_str("<p class=\"muted\" id=\"frames-empty\">No frames yet.</p>");
    }
    items.push_str(
        "<div class=\"console\" id=\"frames-scroll\"><ol id=\"frames\" class=\"logs hide-keep\">",
    );
    for event in events {
        items.push_str(&render_frame(event));
    }
    items.push_str("</ol></div>");
    format!(
        "<section class=\"card\" id=\"frames-panel\"><div class=\"toolbar\"><h2>Frames</h2>\
         <div class=\"filters\">\
         <label class=\"check\"><input id=\"keepalives\" type=\"checkbox\"> Show keepalives</label>\
         <select id=\"dir\" aria-label=\"Direction\"><option value=\"\">All directions</option>\
         <option value=\"in\">IN</option><option value=\"out\">OUT</option>\
         <option value=\"poll\">POLL</option><option value=\"note\">NOTE</option></select>\
         <select id=\"kind\" aria-label=\"Frame type\"><option value=\"\">All types</option>\
         <option>data:update</option><option>data:subscribe_oauth</option><option>data:error</option>\
         <option>parse</option><option>drop</option><option>poll</option><option>session</option>\
         <option>control:hello</option><option>ping</option><option>pong</option><option>close</option>\
         <option>unknown</option></select></div></div>{items}</section>"
    )
}

fn render_logs(lines: &[LogLine]) -> String {
    let mut items = String::new();
    if lines.is_empty() {
        items.push_str("<p class=\"muted\">No log lines yet.</p>");
    }
    items.push_str(
        "<div class=\"filters\"><select id=\"level\" aria-label=\"Log level\"><option value=\"\">All levels</option>\
         <option>ERROR</option><option>WARN</option><option>INFO</option><option>DEBUG</option><option>TRACE</option></select></div>",
    );
    items.push_str("<div class=\"console\" id=\"logs-scroll\"><ol id=\"app-logs\" class=\"logs\">");
    for line in lines {
        items.push_str(&render_log(line));
    }
    items.push_str("</ol></div>");
    format!(
        "<section class=\"card\" id=\"logs-panel\"><div class=\"toolbar\"><h2>Application logs</h2></div>{items}</section>"
    )
}

fn render_frame(event: &DebugEvent) -> String {
    let keep = if event.keepalive { "1" } else { "0" };
    format!(
        "<li data-stream=\"frame\" data-seq=\"{}\" data-dir=\"{}\" data-type=\"{}\" data-keep=\"{keep}\"><time datetime=\"{}\" title=\"{}\">{}</time> <span class=\"dir {}\">{}</span> <span class=\"{}\">{}</span> {}</li>",
        event.id,
        escape(&event.direction),
        escape(&event.kind),
        event.at.to_rfc3339(),
        escape(&event.at.format("%Y-%m-%d %H:%M:%S UTC").to_string()),
        escape(&event.at.format("%H:%M:%S").to_string()),
        escape(&event.direction),
        direction_label(&event.direction),
        tone_class(&event.kind),
        escape(&event.kind),
        escape(&event.text)
    )
}

fn render_log(line: &LogLine) -> String {
    format!(
        "<li data-stream=\"log\" data-seq=\"{}\" data-level=\"{}\"><time datetime=\"{}\">{}</time> <span class=\"{}\">{}</span> {}</li>",
        line.seq,
        escape(&line.level),
        line.at.to_rfc3339(),
        escape(&line.at.format("%H:%M:%S").to_string()),
        level_class(&line.level),
        escape(&line.level),
        escape(&line.text)
    )
}

fn card(title: &str, chip_label: &str, tone_name: &str, rows: &[(&str, String)]) -> String {
    let mut facts = String::new();
    for (label, value) in rows {
        let _ = std::fmt::Write::write_fmt(
            &mut facts,
            format_args!(
                "<div><dt>{}</dt><dd>{}</dd></div>",
                escape(label),
                escape(value)
            ),
        );
    }
    format!(
        "<article class=\"card\"><div class=\"vehicle-head\"><h3>{}</h3>{}</div><dl class=\"facts\">{facts}</dl></article>",
        escape(title),
        chip(chip_label, tone_name)
    )
}

fn chip(label: &str, tone_name: &str) -> String {
    let tone_name = match tone_name {
        "ok" | "bad" => tone_name,
        _ => "neutral",
    };
    format!("<span class=\"chip {tone_name}\">{}</span>", escape(label))
}

fn tone(state: &str) -> &'static str {
    match state {
        "connected" | "listening" | "reachable" => "ok",
        "error" | "unreachable" => "bad",
        _ => "neutral",
    }
}

fn tone_class(kind: &str) -> &'static str {
    match kind {
        "data:error" | "parse" | "drop" | "session" => "bad",
        "data:update" | "data:subscribe_oauth" => "ok",
        _ => "info",
    }
}

fn level_class(level: &str) -> &'static str {
    match level.to_ascii_uppercase().as_str() {
        "ERROR" => "bad",
        "WARN" | "WARNING" => "warn",
        _ => "info",
    }
}

fn direction_label(direction: &str) -> &'static str {
    match direction {
        "in" => "IN",
        "out" => "OUT",
        "poll" => "POLL",
        _ => "NOTE",
    }
}

fn fmt_ago(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(at) = at else {
        return "never".to_owned();
    };
    let secs = (now - at).num_seconds().max(0);
    format!("{} ago", fmt_span(secs))
}

fn fmt_since(at: DateTime<Utc>, now: DateTime<Utc>) -> String {
    format!(
        "{} ago ({})",
        fmt_span((now - at).num_seconds().max(0)),
        at.format("%H:%M:%S UTC")
    )
}

fn fmt_expiry(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(at) = at else {
        return "unknown".to_owned();
    };
    let abs = at.format("%Y-%m-%d %H:%M:%S UTC").to_string();
    let secs = (at - now).num_seconds();
    if secs >= 0 {
        format!("in {} ({abs})", fmt_span(secs))
    } else {
        format!("expired {} ago ({abs})", fmt_span(-secs))
    }
}

fn fmt_span(secs: i64) -> String {
    let secs = u64::try_from(secs).unwrap_or(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

fn fmt_duration(duration: Duration) -> String {
    fmt_span(i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
}

fn fmt_elapsed(duration: Duration) -> String {
    let ms = duration.as_millis();
    if ms < 10_000 {
        format!("{ms}ms")
    } else {
        format!("{}s", duration.as_secs())
    }
}

fn truncate(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const SCRIPT: &str = r#"<script>
(function () {
  var root = document.getElementById("debug-root");
  var frames = document.getElementById("frames");
  var logs = document.getElementById("app-logs");
  var status = document.getElementById("status");
  var pauseBtn = document.getElementById("pause");
  var live = document.getElementById("live");
  var framePane = document.getElementById("frames-scroll");
  var logPane = document.getElementById("logs-scroll");
  if (!root || !frames || !logs || !status) return;
  var paused = false;
  var queue = [];
  var stickFrames = true;
  var stickLogs = true;
  function nearBottom(el) {
    return el.scrollHeight - el.scrollTop - el.clientHeight < 48;
  }
  if (framePane) framePane.addEventListener("scroll", function () { stickFrames = nearBottom(framePane); });
  if (logPane) logPane.addEventListener("scroll", function () { stickLogs = nearBottom(logPane); });
  function applyFilters() {
    var dir = document.getElementById("dir").value;
    var kind = document.getElementById("kind").value;
    var level = document.getElementById("level").value;
    var showKeep = document.getElementById("keepalives").checked;
    frames.classList.toggle("hide-keep", !showKeep);
    frames.querySelectorAll("li").forEach(function (li) {
      var dirOk = !dir || li.dataset.dir === dir;
      var kindOk = !kind || li.dataset.type === kind;
      li.hidden = !(dirOk && kindOk);
    });
    logs.querySelectorAll("li").forEach(function (li) {
      var value = li.dataset.level || "";
      var levelOk = !level || value === level || (level === "WARN" && value === "WARNING");
      li.hidden = !levelOk;
    });
  }
  function trim(list) {
    while (list.children.length > 400) list.removeChild(list.firstElementChild);
  }
  function appendHtml(html) {
    var tpl = document.createElement("template");
    tpl.innerHTML = html.trim();
    var li = tpl.content.firstElementChild;
    if (!li) return;
    var list = li.dataset.stream === "log" ? logs : frames;
    var sel = 'li[data-stream="' + li.dataset.stream + '"][data-seq="' + li.dataset.seq + '"]';
    if (list.querySelector(sel)) return;
    list.appendChild(li);
    trim(list);
    var empty = document.getElementById("frames-empty");
    if (empty && list === frames) empty.hidden = true;
    applyFilters();
    if (!paused) {
      if (list === frames && framePane && stickFrames) framePane.scrollTop = framePane.scrollHeight;
      if (list === logs && logPane && stickLogs) logPane.scrollTop = logPane.scrollHeight;
    }
  }
  if (pauseBtn) {
    pauseBtn.addEventListener("click", function () {
      paused = !paused;
      pauseBtn.textContent = paused ? "Resume" : "Pause";
      pauseBtn.setAttribute("aria-pressed", paused ? "true" : "false");
      if (!paused) {
        queue.forEach(appendHtml);
        queue.length = 0;
        if (framePane && stickFrames) framePane.scrollTop = framePane.scrollHeight;
        if (logPane && stickLogs) logPane.scrollTop = logPane.scrollHeight;
      }
    });
  }
  ["dir", "kind", "level", "keepalives"].forEach(function (id) {
    var el = document.getElementById(id);
    if (el) el.addEventListener("change", applyFilters);
  });
  var src = new EventSource("/logs/events?frames=" + (root.dataset.frames || "0") + "&logs=" + (root.dataset.logs || "0"));
  src.addEventListener("status", function (event) {
    if (!paused) status.innerHTML = event.data;
  });
  function onItem(event) {
    if (paused) {
      queue.push(event.data);
      if (pauseBtn) pauseBtn.textContent = "Resume (" + queue.length + ")";
      return;
    }
    appendHtml(event.data);
  }
  src.addEventListener("frame", onItem);
  src.addEventListener("log", onItem);
  src.onopen = function () { if (live) live.textContent = "live"; };
  src.onerror = function () { if (live) live.textContent = "reconnecting"; };
  applyFilters();
  if (framePane) framePane.scrollTop = framePane.scrollHeight;
  if (logPane) logPane.scrollTop = logPane.scrollHeight;
})();
</script>"#;

#[cfg(test)]
mod tests {
    use super::DebugBoard;
    use crate::logs::LogBuffer;

    #[test]
    fn subscribe_payload_stored_in_the_debug_buffer_has_no_token() {
        let token = "subscribe-token-value";
        let frame = tesla_owner_api::subscribe_frame(token, 42);
        let board = DebugBoard::new(LogBuffer::new(8));
        board.record_event("out", "data:subscribe_oauth", false, &frame);
        let text = board.joined();
        assert!(!text.contains(token), "{text}");
        assert!(!text.contains("\"token\""), "{text}");
        assert!(text.contains("data:subscribe_oauth"), "{text}");
        assert!(text.contains("tag=42"), "{text}");
        assert!(text.contains("speed,odometer"), "{text}");
    }

    #[test]
    fn debug_events_redact_authorization_and_vin() {
        let board = DebugBoard::new(LogBuffer::new(4));
        board.record_event(
            "in",
            "data:error",
            false,
            "Authorization: Bearer bearer-secret-value vin=5YJ3E1EA7KF000001",
        );
        let text = board.joined();
        assert!(!text.contains("bearer-secret-value"), "{text}");
        assert!(!text.contains("5YJ3E1EA7KF000001"), "{text}");
    }

    #[test]
    fn keepalives_do_not_evict_a_parse_failure() {
        let board = DebugBoard::new(LogBuffer::new(8));
        board.record_event(
            "in",
            "parse",
            false,
            "frame received but failed to parse: too many columns",
        );
        for _ in 0..80 {
            board.record_event("in", "ping", true, "ping");
        }
        let text = board.joined();
        assert!(text.contains("failed to parse"), "{text}");
    }
}
