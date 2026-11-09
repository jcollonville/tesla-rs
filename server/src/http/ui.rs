// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use axum::Router;
use std::fmt::Write;

use axum::extract::{Form, Path, State};
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
    let streaming = app.inner.stream_board.snapshot().await;
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
    let error = error
        .map(|text| format!("<p class=\"err\">{}</p>", escape(text)))
        .unwrap_or_default();
    let body = page(&format!(
        "<h1>tesla-logger</h1>{error}<form method=\"post\" action=\"/\">\
         <input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
         <label>Password <input type=\"password\" name=\"password\" autocomplete=\"current-password\" required></label>\
         <button type=\"submit\">Sign in</button></form>"
    ));
    let mut response = Html(body).into_response();
    if let Some(cookie) = set_cookie {
        response.headers_mut().insert(header::SET_COOKIE, cookie);
    }
    response
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
    let connected_text = if connected { "yes" } else { "no" };
    let global = global
        .map(|text| format!("<p class=\"err\">{}</p>", escape(text)))
        .unwrap_or_default();
    let mut rows = String::new();
    if vehicles.is_empty() {
        rows.push_str("<tr><td colspan=\"7\">No vehicles yet.</td></tr>");
    }
    for vehicle in vehicles {
        let (stream_status, stream_error) = streaming.for_vehicle(vehicle.id);
        let stream = match stream_error {
            Some(error) => format!("{stream_status} ({error})"),
            None => stream_status,
        };
        let _ = write!(
            rows,
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape(vehicle.name.as_deref().unwrap_or(vehicle.vin.as_str())),
            escape(&vehicle.vin),
            escape(vehicle.activity.as_deref().unwrap_or("unknown")),
            fmt_opt(vehicle.battery_level),
            fmt_time(vehicle.last_poll_at),
            escape(vehicle.last_error.as_deref().unwrap_or("")),
            escape(&stream)
        );
    }
    let mut token_rows = String::new();
    for token in tokens {
        let state = if token.revoked_at.is_some() {
            "revoked"
        } else {
            "active"
        };
        let _ = write!(
            token_rows,
            "<tr><td>{}</td><td>{state}</td><td><form class=\"inline\" method=\"post\" action=\"/tokens/{}/revoke\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><button type=\"submit\">Revoke</button></form></td></tr>",
            escape(&token.name),
            token.id
        );
    }
    let revealed = revealed
        .map(|token| {
            format!(
                "<p>New API token, shown once: <code>{}</code></p>",
                escape(token)
            )
        })
        .unwrap_or_default();
    let stream_summary = streaming.summary();
    let connect = if connected {
        format!(
            "<form class=\"inline\" method=\"post\" action=\"/auth/disconnect\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><button type=\"submit\">Disconnect Tesla</button></form>"
        )
    } else {
        "<a href=\"/auth/login\">Connect Tesla</a>".to_owned()
    };
    page(&format!(
        "<h1>tesla-logger</h1>{global}<p>Tesla account connected: {connected_text}. {connect}</p>\
         <form class=\"inline\" method=\"post\" action=\"/logout\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><button type=\"submit\">Sign out</button></form>\
         <h2>Vehicles</h2><p>Owner API stream: {stream_summary}. <a href=\"/auth/owner\">Set up streaming</a></p>\
         <table><thead><tr><th>Name</th><th>VIN</th><th>State</th><th>Battery %</th><th>Last poll</th><th>Error</th><th>Stream</th></tr></thead><tbody>{rows}</tbody></table>\
         <h2>API tokens</h2>{revealed}<form method=\"post\" action=\"/tokens\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
         <label>Name <input name=\"name\" required maxlength=\"80\"></label> <button type=\"submit\">Create token</button></form>\
         <table><thead><tr><th>Name</th><th>Status</th><th></th></tr></thead><tbody>{token_rows}</tbody></table>"
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

fn fmt_opt(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_owned(), |value| format!("{value:.0}"))
}

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

const HEAD: &str = "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><meta name=\"robots\" content=\"noindex\"><title>tesla-logger</title><style>:root{color-scheme:light dark}body{font:15px/1.45 system-ui,sans-serif;margin:2rem auto;max-width:52rem;padding:0 1rem}table{border-collapse:collapse;width:100%}th,td{text-align:left;padding:.35rem .5rem;border-bottom:1px solid #8884;vertical-align:top}code{font-family:ui-monospace,monospace}button,input{font:inherit}.err{color:#a33}form.inline{display:inline}footer{margin-top:2rem;color:#666}</style></head><body>";
const FOOT: &str = "<footer><p>tesla-logger is free software under the GNU AGPL-3.0-only. <a href=\"https://github.com/jcollonville/tesla-rs\">Source code</a>.</p></footer></body></html>";

pub(super) fn page(body: &str) -> String {
    let mut html = String::with_capacity(HEAD.len() + body.len() + FOOT.len());
    html.push_str(HEAD);
    html.push_str(body);
    html.push_str(FOOT);
    html
}
