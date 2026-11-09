// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

pub(crate) mod api;
mod ui;

use std::time::Instant;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use axum::middleware::{Next, from_fn, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::app::AppState;
use crate::crypto::sha256_hex;
use crate::error::redact_secrets;

pub(crate) fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/api/v1/vehicles", get(api::list_vehicles))
        .route("/api/v1/vehicles/{id}", get(api::vehicle))
        .route("/api/v1/vehicles/{id}/state", get(api::vehicle_state))
        .route("/api/v1/drives", get(api::list_drives))
        .route("/api/v1/drives/{id}", get(api::drive_detail))
        .route("/api/v1/charges", get(api::list_charges))
        .route("/api/v1/charges/{id}", get(api::charge_detail))
        .route("/api/v1/stats", get(api::stats))
        .route("/api/v1/geofences", get(api::geofences))
        .route("/api/v1/ws", get(api::ws))
        .route_layer(from_fn_with_state(state.clone(), require_bearer));
    Router::new()
        .route("/healthz", get(health))
        .route(
            "/.well-known/appspecific/com.tesla.3p.public-key.pem",
            get(public_key),
        )
        .route("/api/openapi.yaml", get(openapi))
        .merge(ui::routes())
        .merge(api)
        .layer(from_fn(security_headers))
        .with_state(state)
}

async fn health(State(app): State<AppState>) -> Response {
    match sqlx::query("SELECT 1").execute(&app.inner.db).await {
        Ok(_) => (StatusCode::OK, "ok").into_response(),
        Err(err) => {
            tracing::warn!(error = %redact_secrets(&err.to_string()), "healthcheck failed");
            (StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response()
        }
    }
}

async fn public_key(State(app): State<AppState>) -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/x-pem-file; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=300"),
            ),
        ],
        app.inner.public_key_pem.clone(),
    )
        .into_response()
}

async fn openapi() -> Response {
    (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/yaml; charset=utf-8"),
        )],
        OPENAPI,
    )
        .into_response()
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert("x-frame-options", HeaderValue::from_static("DENY"));
    if path != "/healthz" && !path.ends_with("public-key.pem") {
        tracing::info!(%method, %path, status = %response.status(), "request");
    }
    response
}

async fn require_bearer(
    State(app): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let Some(token) = bearer_token(&request) else {
        return Err(ApiError::Unauthorized);
    };
    let hash = sha256_hex(&token);
    let row = sqlx::query_as::<_, (i64,)>(
        "SELECT id FROM api_tokens WHERE token_hash = $1 AND revoked_at IS NULL",
    )
    .bind(&hash)
    .fetch_optional(&app.inner.db)
    .await
    .map_err(|err| {
        tracing::error!(error = %redact_secrets(&err.to_string()), "token lookup failed");
        ApiError::Internal
    })?;
    let Some((id,)) = row else {
        return Err(ApiError::Unauthorized);
    };
    let _ = sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE id = $1")
        .bind(id)
        .execute(&app.inner.db)
        .await;
    Ok(next.run(request).await)
}

fn bearer_token(request: &Request<Body>) -> Option<String> {
    if let Some(header) = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        let token = header
            .strip_prefix("Bearer ")
            .or_else(|| header.strip_prefix("bearer "))?;
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_owned());
        }
    }
    let query = request.uri().query()?;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key == "access_token" && !value.is_empty() {
            return Some(value.into_owned());
        }
    }
    None
}

pub(crate) fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for part in raw.split(';') {
        let (key, value) = part.trim().split_once('=')?;
        if key.trim() == name {
            return Some(value.trim());
        }
    }
    None
}

pub(crate) async fn ui_session(app: &AppState, headers: &HeaderMap) -> Option<String> {
    let id = cookie(headers, "tl_session")?;
    let mut sessions = app.inner.ui_sessions.lock().await;
    let session = sessions.get(id)?;
    if session.expires <= Instant::now() {
        sessions.remove(id);
        return None;
    }
    Some(id.to_owned())
}

#[derive(Debug)]
pub(crate) enum ApiError {
    Unauthorized,
    NotFound,
    BadRequest(&'static str),
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not found"),
            Self::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            Self::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "internal"),
        };
        (status, Json(ErrorBody { error })).into_response()
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

pub(crate) const OPENAPI: &str = include_str!("../../openapi.yaml");
