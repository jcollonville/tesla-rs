// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

pub(crate) mod api;
mod assets;
mod edits;
mod markup;
mod owner;
pub(crate) mod stats;
mod trips;
mod ui;

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, Request, StatusCode, header};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;

use crate::app::AppState;
use crate::crypto::random_token;
use crate::error::redact_secrets;

pub(crate) fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/api/v1/vehicles", get(api::list_vehicles))
        .route("/api/v1/vehicles/{id}", get(api::vehicle))
        .route("/api/v1/vehicles/{id}/state", get(api::vehicle_state))
        .route("/api/v1/vehicles/{id}/stats/series", get(stats::series))
        .route(
            "/api/v1/vehicles/{id}/stats/consumption_by_temp",
            get(stats::consumption_by_temp),
        )
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
        .merge(assets::routes())
        .merge(ui::routes())
        .merge(api)
        .layer(from_fn_with_state(state.clone(), security_headers))
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

async fn security_headers(
    State(app): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let path_for_setup = request.uri().path().to_owned();
    let headers_for_setup = request.headers().clone();
    if needs_setup(&app, &headers_for_setup, &path_for_setup).await {
        return axum::response::Redirect::to("/security/setup").into_response();
    }
    if !crate::identity::origin_allowed(
        request.method(),
        request.headers(),
        &app.inner.config.public_base_url,
    ) {
        return (StatusCode::FORBIDDEN, "cross-origin request rejected").into_response();
    }
    let path = request.uri().path().to_owned();
    let method = request.method().clone();
    let https = request_is_https(&request, &app.inner.config.public_base_url);
    let nonce = random_token().unwrap_or_else(|_| "nonce".to_owned());
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
    if let Ok(csp) = HeaderValue::from_str(&format!(
        "default-src 'self'; script-src 'self' 'nonce-{nonce}'; style-src 'self'; img-src 'self' data:; connect-src 'self'; font-src 'self'; base-uri 'self'; form-action 'self'; object-src 'none'; frame-ancestors 'none'"
    )) {
        headers.insert("content-security-policy", csp);
    }
    headers.insert(
        "permissions-policy",
        HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=(), usb=()"),
    );
    if https {
        headers.insert(
            "strict-transport-security",
            HeaderValue::from_static("max-age=31536000; includeSubDomains"),
        );
    }
    if private_page(&path) && !headers.contains_key(header::CACHE_CONTROL) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    }
    if path != "/healthz" && !path.ends_with("public-key.pem") && !assets::is_static(&path) {
        tracing::info!(%method, %path, status = %response.status(), "request");
    }
    response
}

async fn needs_setup(app: &AppState, headers: &HeaderMap, path: &str) -> bool {
    if path.starts_with("/api/")
        || path.starts_with("/static/")
        || path == "/security/setup"
        || path == "/security/password"
        || path == "/security/totp"
        || path == "/security/totp/confirm"
        || path == "/logout"
        || path == "/healthz"
        || assets::is_static(path)
    {
        return false;
    }
    let Some(id) = cookie(headers, crate::identity::SESSION_COOKIE).map(str::to_owned) else {
        return false;
    };
    crate::identity::load_session(&app.inner.db, &id)
        .await
        .ok()
        .flatten()
        .is_some_and(|principal| principal.needs_setup())
}

fn request_is_https(request: &Request<Body>, public_base: &str) -> bool {
    if public_base.starts_with("https://") || request.uri().scheme_str() == Some("https") {
        return true;
    }
    request
        .headers()
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.eq_ignore_ascii_case("https"))
}

fn private_page(path: &str) -> bool {
    !path.starts_with("/api/")
        && !path.starts_with("/static/")
        && path != "/healthz"
        && path != "/api/openapi.yaml"
        && !path.starts_with("/.well-known/")
        && !assets::is_static(path)
}

async fn require_bearer(
    State(app): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, ApiError> {
    let path = request.uri().path().to_owned();
    let Some((token, from_query)) = bearer_token(&request) else {
        return Err(ApiError::Unauthorized);
    };
    if from_query && path != "/api/v1/ws" {
        return Err(ApiError::Unauthorized);
    }
    let Some(record) = crate::identity::authenticate_token(&app.inner.db, &token)
        .await
        .map_err(|err| {
            tracing::error!(error = %redact_secrets(&err.to_string()), "token lookup failed");
            ApiError::Internal
        })?
    else {
        return Err(ApiError::Unauthorized);
    };
    let scopes = &record.scopes;
    let allowed = if from_query {
        scope_has(scopes, "read:live")
    } else if path == "/api/v1/ws" {
        scope_has(scopes, "read") || scope_has(scopes, "read:live")
    } else {
        scope_has(scopes, "read")
    };
    if !allowed {
        return Err(ApiError::Unauthorized);
    }
    let vehicles = match record.user_id {
        None => None,
        Some(user_id) => {
            let mut ids = crate::identity::accessible_vehicles(&app.inner.db, user_id)
                .await
                .map_err(|_| ApiError::Internal)?;
            if let Some(limit) = record.vehicle_ids {
                ids.retain(|id| limit.contains(id));
            }
            Some(ids)
        }
    };
    let grant = ApiGrant {
        user_id: record.user_id,
        vehicles,
    };
    Ok(API_GRANT.scope(grant, next.run(request)).await)
}

fn scope_has(scopes: &[String], needed: &str) -> bool {
    scopes
        .iter()
        .any(|scope| scope == needed || scope == "admin")
}

fn bearer_token(request: &Request<Body>) -> Option<(String, bool)> {
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
            return Some((token.to_owned(), false));
        }
    }
    let query = request.uri().query()?;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key == "access_token" && !value.is_empty() {
            return Some((value.into_owned(), true));
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

tokio::task_local! {
    static API_GRANT: ApiGrant;
}

struct ApiGrant {
    user_id: Option<i64>,
    vehicles: Option<Vec<i64>>,
}

pub(crate) fn api_user_id() -> Option<i64> {
    API_GRANT.try_with(|grant| grant.user_id).ok().flatten()
}

pub(crate) fn vehicle_visible(id: i64) -> bool {
    API_GRANT
        .try_with(|grant| grant.vehicles.as_ref().is_none_or(|ids| ids.contains(&id)))
        .unwrap_or(true)
}

pub(crate) async fn ui_session(
    app: &AppState,
    headers: &HeaderMap,
) -> Option<crate::identity::Principal> {
    let id = cookie(headers, crate::identity::SESSION_COOKIE)?;
    crate::identity::load_session(&app.inner.db, id)
        .await
        .ok()
        .flatten()
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

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::router;
    use crate::app::test_state;

    #[tokio::test]
    async fn csp_uses_a_nonce_and_hsts_follows_https() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .unwrap();
        let state = test_state(pool).await;
        let plain = router(state.clone())
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let csp = plain
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(csp.contains("default-src 'self'"), "{csp}");
        assert!(csp.contains("script-src 'self' 'nonce-"), "{csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
        assert!(csp.contains("object-src 'none'"), "{csp}");
        assert!(!csp.contains("unsafe-inline"), "{csp}");
        assert!(plain.headers().get("permissions-policy").is_some());
        assert!(plain.headers().get("strict-transport-security").is_none());
        assert_eq!(
            plain.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        assert_eq!(plain.status(), StatusCode::OK);

        let secure = router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/settings")
                    .header("x-forwarded-proto", "https")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let hsts = secure
            .headers()
            .get("strict-transport-security")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(hsts.contains("max-age=31536000"), "{hsts}");
        let again = secure
            .headers()
            .get("content-security-policy")
            .unwrap()
            .to_str()
            .unwrap();
        assert_ne!(csp, again, "each response gets its own nonce");

        let css = router(state)
            .oneshot(
                Request::builder()
                    .uri("/static/ui.css")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let cache = css.headers().get(header::CACHE_CONTROL).unwrap();
        assert_eq!(cache, "public, max-age=86400");
        let body = css.into_body().collect().await.unwrap().to_bytes();
        assert!(String::from_utf8_lossy(&body).contains("color-scheme"));
    }
}
