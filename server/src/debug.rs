// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Signed-in operator console: subsystem status, websocket frames, and app logs.
//!
//! Events are redacted before they are stored. Access tokens, refresh tokens, and VINs are not
//! kept in the ring buffer.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::response::sse::Event;
use chrono::{DateTime, Utc};
use maud::{PreEscaped, html};
use tokio::sync::{broadcast, mpsc};
use tokio_stream::StreamExt;
use tokio_stream::wrappers::ReceiverStream;

use crate::app::AppState;
use crate::config::Config;
use crate::logs::{LogBuffer, LogLine, redact_debug_text};

const EVENT_CAP: usize = 400;

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
    fleet: FleetSlot,
    postgres: ProbeSlot,
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
    text: String,
}

struct FleetSlot {
    last_poll_at: Option<DateTime<Utc>>,
    last_status: Option<u16>,
    last_error: Option<String>,
    drive: Duration,
    charge: Duration,
    parked: Duration,
    asleep: Duration,
    budget_banner: Option<String>,
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

impl Default for DebugState {
    fn default() -> Self {
        Self {
            next_id: 1,
            events: VecDeque::new(),
            fleet: FleetSlot {
                last_poll_at: None,
                last_status: None,
                last_error: None,
                drive: Duration::ZERO,
                charge: Duration::ZERO,
                parked: Duration::ZERO,
                asleep: Duration::ZERO,
                budget_banner: None,
            },
            postgres: ProbeSlot {
                reachable: None,
                checked_at: None,
            },
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
            guard.fleet.drive = config.drive_fallback_poll;
            guard.fleet.charge = config.charge_poll;
            guard.fleet.parked = config.parked_poll;
            guard.fleet.asleep = config.asleep_poll;
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
            push_event(&mut guard, "poll", "poll", text);
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

    pub(crate) fn set_token_probe(&self, probe: TokenProbe) {
        {
            let mut guard = self.lock();
            let slot = &mut guard.fleet_token;
            slot.present = Some(probe.present);
            slot.readable = probe.readable;
            slot.expires_at = probe.expires_at;
        }
        self.notify();
    }

    pub(crate) fn token_saved(&self, expires: SystemTime) {
        {
            let mut guard = self.lock();
            let slot = &mut guard.fleet_token;
            slot.present = Some(true);
            slot.readable = true;
            slot.expires_at = system_to_utc(expires);
            slot.last_refresh_at = Some(Utc::now());
            slot.last_refresh_ok = Some(true);
            slot.last_refresh_error = None;
        }
        self.notify();
    }

    pub(crate) fn token_failed(&self, message: &str) {
        let message = self.scrub(message);
        {
            let mut guard = self.lock();
            let slot = &mut guard.fleet_token;
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

    /// Operator banner when `vehicle_data` is stopped. `None` clears it.
    pub(crate) fn set_budget_banner(&self, text: Option<&str>) {
        let text = text.map(str::to_owned);
        {
            let mut guard = self.lock();
            if guard.fleet.budget_banner == text {
                return;
            }
            guard.fleet.budget_banner = text;
        }
        self.notify();
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
        if let Some(banner) = guard.fleet.budget_banner.clone() {
            body.push_str(&budget_banner_html(&banner));
        }
        body.push_str(&render_fleet(&guard.fleet, now));
        body.push_str(&render_postgres(&guard.postgres, now));
        body.push_str(&render_token("Fleet token store", &guard.fleet_token, now));
        body.push_str(&render_service("MQTT", &guard.mqtt));
        body.push_str(&render_service("Fleet telemetry", &guard.telemetry));
        body.push_str("</div>");
        body.push_str(&render_frames(&guard.events));
        body.push_str(&render_logs(lines));
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
    let fleet = token_probe(&app.inner.fleet_store).await;
    app.inner.debug.set_token_probe(fleet);
}

async fn token_probe(store: &crate::accounts::DbTokenStore) -> TokenProbe {
    let view = store.public_view().await;
    TokenProbe {
        present: view.readable && view.present,
        readable: view.readable,
        expires_at: view.expires_at,
    }
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
    if let Some(banner) = guard.fleet.budget_banner.clone() {
        html.push_str(&budget_banner_html(&banner));
    }
    html.push_str(&render_fleet(&guard.fleet, now));
    html.push_str(&render_postgres(&guard.postgres, now));
    html.push_str(&render_token("Fleet token store", &guard.fleet_token, now));
    html.push_str(&render_service("MQTT", &guard.mqtt));
    html.push_str(&render_service("Fleet telemetry", &guard.telemetry));
    html
}

fn push_event(guard: &mut DebugState, direction: &str, kind: &str, text: String) {
    while guard.events.len() >= EVENT_CAP {
        guard.events.pop_front();
    }
    let id = guard.next_id;
    guard.next_id = guard.next_id.saturating_add(1);
    guard.events.push_back(DebugEvent {
        id,
        at: Utc::now(),
        direction: direction.to_owned(),
        kind: kind.to_owned(),
        text,
    });
}

fn budget_banner_html(text: &str) -> String {
    let text = text
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<div class=\"banner bad\" role=\"alert\"><p>{text}</p></div>")
}

fn system_to_utc(time: SystemTime) -> Option<DateTime<Utc>> {
    let Ok(duration) = time.duration_since(UNIX_EPOCH) else {
        return None;
    };
    let secs = i64::try_from(duration.as_secs()).ok()?;
    DateTime::<Utc>::from_timestamp(secs, duration.subsec_nanos())
}

fn render_fleet(slot: &FleetSlot, now: DateTime<Utc>) -> String {
    let intervals = format!(
        "drive fallback {} · charge {} · parked {} · asleep {}",
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

fn render_frames(events: &VecDeque<DebugEvent>) -> String {
    html! {
        section class="card" id="frames-panel" {
            div class="toolbar" {
                h2 { "Frames" }
                div class="filters" {
                    select id="kind" aria-label="Frame type" {
                        option value="" { "All types" }
                        option { "poll" }
                    }
                }
            }
            @if events.is_empty() {
                p class="muted" id="frames-empty" { "No frames yet." }
            }
            div class="console" id="frames-scroll" {
                ol id="frames" class="logs" {
                    @for event in events {
                        (PreEscaped(render_frame(event)))
                    }
                }
            }
        }
    }
    .into_string()
}

fn render_logs(lines: &[LogLine]) -> String {
    html! {
        section class="card" id="logs-panel" {
            div class="toolbar" {
                h2 { "Application logs" }
            }
            @if lines.is_empty() {
                p class="muted" { "No log lines yet." }
            }
            div class="filters" {
                select id="level" aria-label="Log level" {
                    option value="" { "All levels" }
                    option { "ERROR" }
                    option { "WARN" }
                    option { "INFO" }
                    option { "DEBUG" }
                    option { "TRACE" }
                }
            }
            div class="console" id="logs-scroll" {
                ol id="app-logs" class="logs" {
                    @for line in lines {
                        (PreEscaped(render_log(line)))
                    }
                }
            }
        }
    }
    .into_string()
}

fn render_frame(event: &DebugEvent) -> String {
    let direction_class = format!("dir {}", event.direction);
    html! {
        li data-stream="frame" data-seq=(event.id) data-dir=(&event.direction) data-type=(&event.kind) {
            time datetime=(event.at.to_rfc3339()) title=(event.at.format("%Y-%m-%d %H:%M:%S UTC").to_string()) {
                (event.at.format("%H:%M:%S").to_string())
            }
            " "
            span class=(direction_class) { (direction_label(&event.direction)) }
            " "
            span class=(tone_class(&event.kind)) { (&event.kind) }
            " "
            (event.text)
        }
    }
    .into_string()
}

fn render_log(line: &LogLine) -> String {
    html! {
        li data-stream="log" data-seq=(line.seq) data-level=(&line.level) {
            time datetime=(line.at.to_rfc3339()) { (line.at.format("%H:%M:%S").to_string()) }
            " "
            span class=(level_class(&line.level)) { (&line.level) }
            " "
            (line.text)
        }
    }
    .into_string()
}

fn card(title: &str, chip_label: &str, tone_name: &str, rows: &[(&str, String)]) -> String {
    html! {
        article class="card" {
            div class="vehicle-head" {
                h3 { (title) }
                (chip(chip_label, tone_name))
            }
            dl class="facts" {
                @for (label, value) in rows {
                    div {
                        dt { (label) }
                        dd { (value) }
                    }
                }
            }
        }
    }
    .into_string()
}

fn chip(label: &str, tone_name: &str) -> maud::Markup {
    let tone_name = match tone_name {
        "ok" | "bad" => tone_name,
        _ => "neutral",
    };
    let class = format!("chip {tone_name}");
    html! { span class=(class) { (label) } }
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

#[cfg(test)]
mod tests {
    use super::DebugBoard;
    use crate::logs::LogBuffer;

    #[test]
    fn debug_events_redact_authorization_and_vin() {
        let board = DebugBoard::new(LogBuffer::new(4));
        board.record_poll(
            "GET",
            "/api/1/vehicles",
            Some(401),
            std::time::Duration::from_millis(12),
            "Authorization: Bearer bearer-secret-value vin=5YJ3E1EA7KF000001",
        );
        let text = board.joined();
        assert!(!text.contains("bearer-secret-value"), "{text}");
        assert!(!text.contains("5YJ3E1EA7KF000001"), "{text}");
    }

    #[test]
    fn console_shows_the_budget_banner() {
        let board = DebugBoard::new(LogBuffer::new(4));
        board.set_budget_banner(Some(
            "vehicle_data is paused: the daily Fleet API cap is reached. The vehicle list keeps running.",
        ));
        let html = board.console(&[]).body;
        assert!(html.contains("class=\"banner bad\""), "{html}");
        assert!(html.contains("daily Fleet API cap"), "{html}");
        board.set_budget_banner(None);
        assert!(!board.console(&[]).body.contains("banner bad"));
    }
}
