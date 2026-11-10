// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use axum::Router;
use std::fmt::Write;

use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use serde::Deserialize;

use super::{cookie, ui_session};
use crate::app::{AppState, session_ttl};
use crate::crypto::{password_matches, random_token, sha256_hex};
use tesla_auth::{CallbackParams, ExposeSecret};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(status).post(login))
        .route("/logout", post(logout))
        .route("/auth/login", get(oauth_start))
        .route("/auth/callback", get(oauth_callback))
        .route("/auth/disconnect", post(disconnect))
        .route("/auth/owner", get(super::owner::page))
        .route("/auth/owner/start", post(super::owner::start))
        .route("/auth/owner/callback", post(super::owner::callback))
        .route("/auth/owner/refresh", post(super::owner::refresh))
        .route("/auth/owner/disconnect", post(super::owner::disconnect))
        .route("/tokens", post(create_token))
        .route("/tokens/{id}/revoke", post(revoke_token))
        .route("/logs", get(logs))
        .route("/logs/events", get(logs_events))
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
    csrf: String,
}

#[derive(Deserialize)]
struct CsrfForm {
    csrf: String,
}

#[derive(Deserialize)]
struct TokenForm {
    csrf: String,
    name: String,
}

async fn logs(State(app): State<AppState>, headers: HeaderMap) -> Response {
    if ui_session(&app, &headers).await.is_none() {
        return Redirect::to("/").into_response();
    }
    crate::debug::probe(&app).await;
    let console = app.inner.debug.console(&app.inner.logs.lines());
    let body = format!(
        "<div id=\"debug-root\" data-frames=\"{}\" data-logs=\"{}\">\
         <header class=\"toolbar\"><div><p class=\"kicker\"><a href=\"/\">tesla-logger</a></p>\
         <h1 class=\"title\">{}Debug</h1></div>\
         <div class=\"toolbar-actions\">\
         <span class=\"chip\" id=\"live\">connecting</span>\
         <button class=\"ghost\" type=\"button\" id=\"pause\" aria-pressed=\"false\">Pause</button>\
         <a class=\"button ghost\" href=\"/auth/owner\">{}Streaming</a>\
         <a class=\"button ghost\" href=\"/\">{}Dashboard</a></div></header>\
         <p class=\"muted\">Processes, websocket frames, and recent log lines from this process. They stay in memory and reset on restart. Tokens, passwords, and VINs are removed.</p>\
         {}</div>",
        console.frames_cursor,
        console.logs_cursor,
        icon(ICON_LOGS),
        icon(ICON_STREAM),
        icon(ICON_ARROW),
        console.body
    );
    let mut response = Html(page_wide(&body)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

async fn logs_events(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(cursor): Query<LogCursor>,
) -> Response {
    if ui_session(&app, &headers).await.is_none() {
        return Redirect::to("/").into_response();
    }
    let stream = crate::debug::event_stream(app, cursor.frames, cursor.logs);
    let mut response = axum::response::sse::Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(std::time::Duration::from_secs(15))
                .text("ping"),
        )
        .into_response();
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert("x-accel-buffering", HeaderValue::from_static("no"));
    response
}

#[derive(Deserialize)]
struct LogCursor {
    #[serde(default)]
    frames: u64,
    #[serde(default)]
    logs: u64,
}

async fn status(State(app): State<AppState>, headers: HeaderMap) -> Response {
    if ui_session(&app, &headers).await.is_none() {
        return login_page(&app, &headers, None);
    }
    let csrf = match ensure_csrf(&app, &headers) {
        Ok(value) => value,
        Err(status) => return (status, "could not start a session").into_response(),
    };
    let logged_in = tesla_connected(&app).await;
    let vehicles = match vehicle_rows(&app).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "status query failed");
            Vec::new()
        }
    };
    let tokens = token_rows(&app).await.unwrap_or_default();
    let revealed = if let Some(session) = cookie(&headers, "tl_session") {
        app.inner.revealed.lock().await.remove(session)
    } else {
        None
    };
    let global = app.global_error().await;
    let mut streaming = app.inner.stream_board.snapshot().await;
    streaming.modes = app.inner.archive.gps.modes().await;
    let body = render_status(
        &csrf.0,
        logged_in,
        &vehicles,
        &tokens,
        revealed.as_deref(),
        global.as_deref(),
        &streaming,
    );
    let mut response = Html(body).into_response();
    if let Some(cookie) = csrf.1 {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

async fn tesla_connected(app: &AppState) -> bool {
    let Ok(store) = crate::crypto::TokenFile::open(
        &app.inner.config.token_store_path,
        app.inner.config.token_encryption_key.as_deref(),
    ) else {
        return false;
    };
    matches!(store.load().await, Ok(Some(_)))
}

async fn login(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    let expected = app.inner.config.ui_password.expose_secret();
    if !password_matches(expected, &form.password) {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        return login_page(&app, &headers, Some("Incorrect password"));
    }
    let Ok(id) = random_token() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "entropy failed").into_response();
    };
    app.inner.ui_sessions.lock().await.insert(
        id.clone(),
        crate::app::UiSession {
            expires: std::time::Instant::now() + session_ttl(),
        },
    );
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&session_cookie(&app, &id))
            .unwrap_or(HeaderValue::from_static("tl_session=; HttpOnly")),
    );
    response
}

async fn logout(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    if let Some(id) = cookie(&headers, "tl_session") {
        app.inner.ui_sessions.lock().await.remove(id);
    }
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static("tl_session=; HttpOnly; Path=/; Max-Age=0; SameSite=Lax"),
    );
    response
}

async fn oauth_start(State(app): State<AppState>, headers: HeaderMap) -> Response {
    if ui_session(&app, &headers).await.is_none() {
        return Redirect::to("/").into_response();
    }
    let pending = match app
        .inner
        .session
        .client()
        .begin_login(&tesla_auth::AuthorizeOptions {
            prompt: Some("login".to_owned()),
            locale: Some("en-US".to_owned()),
            ..tesla_auth::AuthorizeOptions::default()
        }) {
        Ok(pending) => pending,
        Err(err) => return (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    };
    let url = pending.url().to_string();
    app.inner
        .pending
        .lock()
        .await
        .insert(pending.state().to_owned(), pending);
    Redirect::to(&url).into_response()
}

async fn oauth_callback(State(app): State<AppState>, request: axum::extract::Request) -> Response {
    let headers = request.headers().clone();
    if ui_session(&app, &headers).await.is_none() {
        return (StatusCode::UNAUTHORIZED, "sign in to the logger first").into_response();
    }
    let query = request.uri().query().unwrap_or("");
    let params = CallbackParams::from_query(query);
    let Some(state) = params.state.clone() else {
        return (StatusCode::BAD_REQUEST, "missing oauth state").into_response();
    };
    let Some(pending) = app.inner.pending.lock().await.remove(&state) else {
        return (StatusCode::BAD_REQUEST, "unknown oauth state").into_response();
    };
    match app.inner.session.complete_login(&pending, params).await {
        Ok(_) => Redirect::to("/").into_response(),
        Err(err) => {
            tracing::warn!(error = %err, "tesla login failed");
            (StatusCode::BAD_GATEWAY, "Tesla sign-in failed").into_response()
        }
    }
}

async fn disconnect(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    if ui_session(&app, &headers).await.is_none() || !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    if let Err(err) = clear_tokens(&app).await {
        tracing::warn!(error = %err, "could not clear tesla tokens");
    }
    Redirect::to("/").into_response()
}

async fn create_token(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    let Some(session) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    let name = form.name.trim();
    if name.is_empty() || name.len() > 80 {
        return (StatusCode::BAD_REQUEST, "token name is required").into_response();
    }
    let Ok(token) = random_token() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "entropy failed").into_response();
    };
    let stored = format!("tl_{token}");
    let hash = sha256_hex(&stored);
    if let Err(err) = sqlx::query("INSERT INTO api_tokens (name, token_hash) VALUES ($1, $2)")
        .bind(name)
        .bind(hash)
        .execute(&app.inner.db)
        .await
    {
        tracing::warn!(error = %err, "could not store api token");
        return (StatusCode::INTERNAL_SERVER_ERROR, "could not store token").into_response();
    }
    app.inner.revealed.lock().await.insert(session, stored);
    Redirect::to("/").into_response()
}

async fn revoke_token(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Response {
    if ui_session(&app, &headers).await.is_none() || !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    let _ = sqlx::query(
        "UPDATE api_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(id)
    .execute(&app.inner.db)
    .await;
    Redirect::to("/").into_response()
}

pub(super) struct CsrfIssued(pub(super) String, pub(super) Option<HeaderValue>);

pub(super) fn ensure_csrf(app: &AppState, headers: &HeaderMap) -> Result<CsrfIssued, StatusCode> {
    if let Some(existing) = cookie(headers, "tl_csrf") {
        return Ok(CsrfIssued(existing.to_owned(), None));
    }
    let Ok(value) = random_token() else {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    };
    let cookie = format!(
        "tl_csrf={value}; HttpOnly; Path=/; Max-Age=43200; SameSite=Lax{}",
        secure_flag(app)
    );
    let header = HeaderValue::from_str(&cookie).map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(CsrfIssued(value, Some(header)))
}

fn login_page(app: &AppState, headers: &HeaderMap, error: Option<&str>) -> Response {
    let csrf = cookie(headers, "tl_csrf").unwrap_or("").to_owned();
    let mut set_cookie = None;
    let csrf = if csrf.is_empty() {
        let Ok(value) = random_token() else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "entropy failed").into_response();
        };
        let cookie = format!(
            "tl_csrf={value}; HttpOnly; Path=/; Max-Age=43200; SameSite=Lax{}",
            secure_flag(app)
        );
        set_cookie = HeaderValue::from_str(&cookie).ok();
        value
    } else {
        csrf
    };
    let body = login_markup(&csrf, error);
    let mut response = Html(body).into_response();
    if let Some(cookie) = set_cookie {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
}

fn login_markup(csrf: &str, error: Option<&str>) -> String {
    let error = error.map(|text| banner("bad", text)).unwrap_or_default();
    page(&format!(
        "<section class=\"card auth-card\">\
         <p class=\"kicker\">tesla-logger</p>\
         <h1>Sign in</h1>\
         <p class=\"muted\">Password for this logger. This is not the Tesla account.</p>\
         {error}\
         <form class=\"stack\" method=\"post\" action=\"/\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
         <label class=\"field\">Password <input type=\"password\" name=\"password\" autocomplete=\"current-password\" required></label>\
         <button type=\"submit\">{}Sign in</button></form></section>",
        icon(ICON_KEY)
    ))
}

fn render_status(
    csrf: &str,
    connected: bool,
    vehicles: &[VehicleRow],
    tokens: &[TokenRow],
    revealed: Option<&str>,
    global: Option<&str>,
    streaming: &crate::owner_stream::StreamSnapshot,
) -> String {
    let global = global.map(|text| banner("bad", text)).unwrap_or_default();
    let account = if connected {
        chip("Tesla connected", "ok")
    } else {
        chip("Tesla not connected", "neutral")
    };
    let stream_summary = streaming.summary();
    let stream = chip(
        &format!("Stream {stream_summary}"),
        tone_for_stream(stream_summary),
    );
    let mut cards = String::new();
    if vehicles.is_empty() {
        cards.push_str("<p class=\"muted\">No vehicles yet.</p>");
    }
    for vehicle in vehicles {
        let _ = write!(cards, "{}", vehicle_card(vehicle, streaming));
    }
    let mut token_rows = String::new();
    for token in tokens {
        let (label, tone) = if token.revoked_at.is_some() {
            ("revoked", "neutral")
        } else {
            ("active", "ok")
        };
        let _ = write!(
            token_rows,
            "<li><span>{}</span><span class=\"row\">{}<form method=\"post\" action=\"/tokens/{}/revoke\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><button class=\"ghost\" type=\"submit\">Revoke</button></form></span></li>",
            escape(&token.name),
            chip(label, tone),
            token.id
        );
    }
    let revealed = revealed
        .map(|token| {
            format!(
                "<div class=\"banner note\" role=\"status\"><p>New API token, shown once.</p><code class=\"secret\">{}</code></div>",
                escape(token)
            )
        })
        .unwrap_or_default();
    let connect = if connected {
        format!(
            "<form method=\"post\" action=\"/auth/disconnect\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><button class=\"ghost\" type=\"submit\">{}Disconnect Tesla</button></form>",
            icon(ICON_UNLINK)
        )
    } else {
        format!(
            "<a class=\"button\" href=\"/auth/login\">{}Connect Tesla</a>",
            icon(ICON_KEY)
        )
    };
    page(&format!(
        "<header class=\"toolbar\">\
         <div><p class=\"kicker\">tesla-logger</p><h1>Dashboard</h1></div>\
         <div class=\"toolbar-actions\">{connect}\
         <form method=\"post\" action=\"/logout\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><button class=\"ghost\" type=\"submit\">{}Sign out</button></form></div></header>\
         {global}\
         <div class=\"meta\">{account}{stream}<a class=\"button ghost\" href=\"/auth/owner\">{}Set up streaming</a><a class=\"button ghost\" href=\"/logs\">Debug</a></div>\
         <section><h2>Vehicles</h2><div class=\"vehicles\">{cards}</div></section>\
         <section class=\"card\"><h2>API tokens</h2>{revealed}\
         <form class=\"stack\" method=\"post\" action=\"/tokens\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
         <label class=\"field\">Name <input name=\"name\" required maxlength=\"80\" autocomplete=\"off\"></label>\
         <button type=\"submit\">{}Create token</button></form>\
         <ul class=\"list\">{token_rows}</ul></section>",
        icon(ICON_LOGOUT),
        icon(ICON_STREAM),
        icon(ICON_KEY)
    ))
}

pub(super) fn csrf_ok(headers: &HeaderMap, form: &str) -> bool {
    cookie(headers, "tl_csrf").is_some_and(|cookie| cookie == form && !form.is_empty())
}

fn session_cookie(app: &AppState, id: &str) -> String {
    format!(
        "tl_session={id}; HttpOnly; Path=/; Max-Age=43200; SameSite=Lax{}",
        secure_flag(app)
    )
}

pub(super) fn secure_flag(app: &AppState) -> &'static str {
    if app.inner.config.public_base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    }
}

async fn clear_tokens(app: &AppState) -> Result<(), crate::error::AppError> {
    let store = crate::crypto::TokenFile::open(
        &app.inner.config.token_store_path,
        app.inner.config.token_encryption_key.as_deref(),
    )?;
    store.clear().await.map_err(crate::error::AppError::from)
}

async fn vehicle_rows(app: &AppState) -> Result<Vec<VehicleRow>, sqlx::Error> {
    sqlx::query_as(
        "SELECT v.id, v.vin, COALESCE(v.name, v.vin) AS name, s.activity, s.battery_level, s.last_poll_at, s.last_error
         FROM vehicles v
         LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
         ORDER BY v.id",
    )
    .fetch_all(&app.inner.db)
    .await
}

async fn token_rows(app: &AppState) -> Result<Vec<TokenRow>, sqlx::Error> {
    sqlx::query_as("SELECT id, name, revoked_at FROM api_tokens ORDER BY id DESC LIMIT 20")
        .fetch_all(&app.inner.db)
        .await
}

#[derive(sqlx::FromRow)]
struct VehicleRow {
    id: i64,
    vin: String,
    name: Option<String>,
    activity: Option<String>,
    battery_level: Option<f64>,
    last_poll_at: Option<chrono::DateTime<chrono::Utc>>,
    last_error: Option<String>,
}

#[derive(sqlx::FromRow)]
struct TokenRow {
    id: i64,
    name: String,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn fmt_battery(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_owned(), |value| format!("{value:.0}%"))
}

fn vehicle_card(vehicle: &VehicleRow, streaming: &crate::owner_stream::StreamSnapshot) -> String {
    let (status, detail) = streaming.for_vehicle(vehicle.id);
    let detail = detail.unwrap_or_else(|| "—".to_owned());
    format!(
        "<article class=\"vehicle\"><div class=\"vehicle-head\"><h3>{}</h3>{}</div>\
         <dl class=\"facts\">\
         <div><dt>VIN</dt><dd>{}</dd></div>\
         <div><dt>State</dt><dd>{}</dd></div>\
         <div><dt>Battery</dt><dd>{}</dd></div>\
         <div><dt>Last poll</dt><dd>{}</dd></div>\
         <div><dt>Error</dt><dd>{}</dd></div>\
         <div><dt>Stream</dt><dd>{}</dd></div>\
         <div><dt>GPS</dt><dd>{}</dd></div>\
         </dl></article>",
        escape(vehicle.name.as_deref().unwrap_or(vehicle.vin.as_str())),
        chip(&status, tone_for_stream(&status)),
        escape(&vehicle.vin),
        escape(vehicle.activity.as_deref().unwrap_or("unknown")),
        fmt_battery(vehicle.battery_level),
        fmt_time(vehicle.last_poll_at),
        escape(vehicle.last_error.as_deref().unwrap_or("—")),
        escape(&detail),
        escape(streaming.position_mode(vehicle.id))
    )
}

fn tone_for_stream(status: &str) -> &'static str {
    match status {
        "connected" => "ok",
        "error" => "bad",
        _ => "neutral",
    }
}

pub(super) fn chip(label: &str, tone: &str) -> String {
    let tone = match tone {
        "ok" | "bad" => tone,
        _ => "neutral",
    };
    format!("<span class=\"chip {tone}\">{}</span>", escape(label))
}

pub(super) fn banner(tone: &str, text: &str) -> String {
    let role = if tone == "bad" { "alert" } else { "status" };
    format!(
        "<div class=\"banner {tone}\" role=\"{role}\"><p>{}</p></div>",
        escape(text)
    )
}

pub(super) fn icon(path: &str) -> String {
    format!(
        "<svg class=\"icon\" viewBox=\"0 0 24 24\" aria-hidden=\"true\" focusable=\"false\"><path fill=\"currentColor\" d=\"{path}\"/></svg>"
    )
}

// Paths from Material Design Icons (Pictogrammers), Apache-2.0.
pub(super) const ICON_KEY: &str = "M22,18V22H18V19H15V16H12L9.74,13.74C9.19,13.91 8.61,14 8,14A6,6 0 0,1 2,8A6,6 0 0,1 8,2A6,6 0 0,1 14,8C14,8.61 13.91,9.19 13.74,9.74L22,18M7,5A2,2 0 0,0 5,7A2,2 0 0,0 7,9A2,2 0 0,0 9,7A2,2 0 0,0 7,5Z";
pub(super) const ICON_LOGOUT: &str = "M17 7L15.59 8.41L18.17 11H8V13H18.17L15.59 15.58L17 17L22 12M4 5H12V3H4C2.9 3 2 3.9 2 5V19C2 20.1 2.9 21 4 21H12V19H4V5Z";
pub(super) const ICON_STREAM: &str = "M4.93,4.93C3.12,6.74 2,9.24 2,12C2,14.76 3.12,17.26 4.93,19.07L6.34,17.66C4.89,16.22 4,14.22 4,12C4,9.79 4.89,7.78 6.34,6.34L4.93,4.93M19.07,4.93L17.66,6.34C19.11,7.78 20,9.79 20,12C20,14.22 19.11,16.22 17.66,17.66L19.07,19.07C20.88,17.26 22,14.76 22,12C22,9.24 20.88,6.74 19.07,4.93M7.76,7.76C6.67,8.85 6,10.35 6,12C6,13.65 6.67,15.15 7.76,16.24L9.17,14.83C8.45,14.11 8,13.11 8,12C8,10.89 8.45,9.89 9.17,9.17L7.76,7.76M16.24,7.76L14.83,9.17C15.55,9.89 16,10.89 16,12C16,13.11 15.55,14.11 14.83,14.83L16.24,16.24C17.33,15.15 18,13.65 18,12C18,10.35 17.33,8.85 16.24,7.76M12,10A2,2 0 0,0 10,12A2,2 0 0,0 12,14A2,2 0 0,0 14,12A2,2 0 0,0 12,10Z";
pub(super) const ICON_UNLINK: &str = "M17,7H13V8.9H17C18.71,8.9 20.1,10.29 20.1,12C20.1,13.43 19.12,14.63 17.79,15L19.25,16.44C20.88,15.61 22,13.95 22,12A5,5 0 0,0 17,7M16,11H13.81L15.81,13H16V11M2,4.27L5.11,7.38C3.29,8.12 2,9.91 2,12A5,5 0 0,0 7,17H11V15.1H7C5.29,15.1 3.9,13.71 3.9,12C3.9,10.41 5.11,9.1 6.66,8.93L8.73,11H8V13H10.73L13,15.27V17H14.73L18.74,21L20,19.74L3.27,3L2,4.27Z";
pub(super) const ICON_ARROW: &str =
    "M20,11V13H8L13.5,18.5L12.08,19.92L4.16,12L12.08,4.08L13.5,5.5L8,11H20Z";
pub(super) const ICON_LOGS: &str = "M5,3C3.89,3 3,3.89 3,5V19A2,2 0 0,0 5,21H19A2,2 0 0,0 21,19V5C21,3.89 20.1,3 19,3H5M5,5H19V19H5V5M7,7V9H17V7H7M7,11V13H17V11H7M7,15V17H14V15H7Z";
pub(super) const ICON_CAR: &str = "M5,11L6.5,6.5H17.5L19,11M17.5,16A1.5,1.5 0 0,1 16,14.5A1.5,1.5 0 0,1 17.5,13A1.5,1.5 0 0,1 19,14.5A1.5,1.5 0 0,1 17.5,16M6.5,16A1.5,1.5 0 0,1 5,14.5A1.5,1.5 0 0,1 6.5,13A1.5,1.5 0 0,1 8,14.5A1.5,1.5 0 0,1 6.5,16M18.92,6C18.72,5.42 18.16,5 17.5,5H6.5C5.84,5 5.28,5.42 5.08,6L3,12V20A1,1 0 0,0 4,21H5A1,1 0 0,0 6,20V19H18V20A1,1 0 0,0 19,21H20A1,1 0 0,0 21,20V12L18.92,6Z";

fn fmt_time(value: Option<chrono::DateTime<chrono::Utc>>) -> String {
    value.map_or_else(
        || "—".to_owned(),
        |value| value.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    )
}

pub(super) fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

const HEAD: &str = concat!(
    "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\">",
    "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
    "<meta name=\"robots\" content=\"noindex\"><title>tesla-logger</title><style>",
    include_str!("ui.css"),
    "</style></head><body><main class=\"shell\">"
);
const FOOT: &str = "</main><footer><p>tesla-logger is free software under the GNU AGPL-3.0-only. <a href=\"https://github.com/jcollonville/tesla-rs\">Source code</a>.</p></footer></body></html>";

pub(super) fn page(body: &str) -> String {
    let mut html = String::with_capacity(HEAD.len() + body.len() + FOOT.len());
    html.push_str(HEAD);
    html.push_str(body);
    html.push_str(FOOT);
    html
}

fn page_wide(body: &str) -> String {
    page(body).replacen("<main class=\"shell\">", "<main class=\"shell wide\">", 1)
}

#[cfg(test)]
pub(super) fn preview(name: &str, html: &str) {
    if let Ok(dir) = std::env::var("UI_PREVIEW_DIR") {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(format!("{dir}/{name}"), html);
    }
}

#[cfg(test)]
mod tests {
    use super::{TokenRow, VehicleRow, login_markup, preview, render_status};

    #[test]
    fn login_form_posts_the_password() {
        let html = login_markup("csrf-token", Some("Incorrect password"));
        assert!(html.contains("action=\"/\""));
        assert!(html.contains("name=\"password\""));
        assert!(html.contains("name=\"csrf\" value=\"csrf-token\""));
        assert!(html.contains("Incorrect password"));
        assert!(html.contains("role=\"alert\""));
        preview("login.html", &html);
    }
    use crate::owner_stream::StreamBoard;

    #[tokio::test]
    async fn dashboard_keeps_actions_and_shows_stream_chips() {
        let board = StreamBoard::default();
        board.set_mode(true, true).await;
        board.set_vehicle(7, "connected", None).await;
        let snapshot = board.snapshot().await;
        let vehicles = [VehicleRow {
            id: 7,
            vin: "5YJ3E1EA7KF000001".to_owned(),
            name: Some("Red".to_owned()),
            activity: Some("driving".to_owned()),
            battery_level: Some(72.4),
            last_poll_at: None,
            last_error: None,
        }];
        let tokens = [TokenRow {
            id: 3,
            name: "home".to_owned(),
            revoked_at: None,
        }];
        let html = render_status(
            "csrf-token",
            true,
            &vehicles,
            &tokens,
            Some("tl_secret"),
            Some("poll failed"),
            &snapshot,
        );
        assert!(html.contains("action=\"/logout\""));
        assert!(html.contains("action=\"/auth/disconnect\""));
        assert!(html.contains("href=\"/auth/owner\""));
        assert!(html.contains("href=\"/logs\""));
        assert!(html.contains("action=\"/tokens\""));
        assert!(html.contains("action=\"/tokens/3/revoke\""));
        assert!(html.contains("name=\"csrf\" value=\"csrf-token\""));
        assert!(html.contains("Stream connected"));
        assert!(html.contains("class=\"chip ok\""));
        assert!(html.contains("72%"));
        assert!(html.contains("Red"));
        assert!(html.contains("tl_secret"));
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains("poll failed"));
        assert!(html.contains("viewport"));
        assert!(!html.contains("void/callback"));
        preview("dashboard.html", &html);
    }
}
