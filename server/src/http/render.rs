// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Official Tesla vehicle renders for the overview and fleet cards.
//!
//! The picture comes from Tesla's public configurator compositor
//! (`static-assets.tesla.com`) and depends only on the model, the paint and the
//! wheels. The server fetches it once per combination, keeps it in memory and in
//! a cache directory, and serves it from `/v/{id}/car.png` for a signed-in
//! browser and from `GET /api/v1/vehicles/{id}/car.png` for a read API token.
//! The browser never contacts Tesla, so the CSP keeps `img-src 'self' data:`.
//! The URL sent to Tesla holds the option codes only: no VIN, no vehicle id,
//! no coordinates.
//!
//! Only combinations known to render are requested. The compositor answers an
//! unknown paint with HTTP 200 and a car without a body, so unknown paints,
//! Cybertruck, and any failed fetch are not shown. The card uses a text placeholder.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::extract::{Path, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use tokio::sync::Mutex;

use super::car::Body;
use crate::app::AppState;
use crate::config::Config;

const BASE_URL: &str = "https://static-assets.tesla.com";
const COMPOSITOR_PATH: &str = "/v1/compositor/";
const MAX_BYTES: usize = 2 * 1024 * 1024;
const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const MEMORY_SLOTS: usize = 32;
const RETRY_AFTER_REJECTION: Duration = Duration::from_secs(24 * 3600);
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(300);

/// Paints that render on each body, by Tesla option code. Names are the Fleet API
/// `exterior_color` values with case and punctuation removed, plus a few aliases.
const PAINTS: &[(&[&str], &str, &[Body])] = &[
    (
        &["pearlwhite", "pearlwhitemulticoat", "white"],
        "PPSW",
        &[Body::Model3, Body::ModelY, Body::ModelS, Body::ModelX],
    ),
    (
        &["solidblack", "black"],
        "PBSB",
        &[Body::Model3, Body::ModelY, Body::ModelS, Body::ModelX],
    ),
    (
        &["midnightsilver", "midnightsilvermetallic"],
        "PMNG",
        &[Body::Model3, Body::ModelY, Body::ModelS, Body::ModelX],
    ),
    (
        &["deepblue", "deepbluemetallic", "blue"],
        "PPSB",
        &[Body::Model3, Body::ModelY, Body::ModelS, Body::ModelX],
    ),
    (
        &["redmulticoat", "red"],
        "PPMR",
        &[Body::Model3, Body::ModelY, Body::ModelS, Body::ModelX],
    ),
    (
        &["silver", "silvermetallic"],
        "PMSS",
        &[Body::Model3, Body::ModelS, Body::ModelX],
    ),
    (
        &["obsidian", "obsidianblack", "blackmetallic"],
        "PMBL",
        &[Body::Model3, Body::ModelS, Body::ModelX],
    ),
    (
        &["titanium", "titaniummetallic"],
        "PPTI",
        &[Body::ModelS, Body::ModelX],
    ),
    (&["signaturered"], "PPSR", &[Body::ModelS, Body::ModelX]),
    (&["dolphingrey"], "PMTG", &[Body::ModelS]),
    (&["midnightblue"], "PMMB", &[Body::ModelS]),
    (&["brown", "brownmetallic"], "PMAB", &[Body::ModelS]),
    (&["green", "greenmetallic"], "PMSG", &[Body::ModelS]),
];

/// Wheel option code by rim diameter in inches. The first entry is the default
/// used when the wheel name does not carry a size.
const WHEELS_MODEL_3: &[(u32, &str)] = &[(18, "W38B"), (19, "W39B"), (20, "W32P")];
const WHEELS_MODEL_Y: &[(u32, &str)] =
    &[(19, "WY19B"), (18, "WY18B"), (20, "WY20P"), (21, "WY21P")];
const WHEELS_MODEL_S: &[(u32, &str)] = &[(19, "WT19"), (21, "WT21")];
const WHEELS_MODEL_X: &[(u32, &str)] = &[(20, "WT20"), (22, "WT22")];

/// What to ask the compositor for. Built from vehicle data only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Spec {
    model: &'static str,
    trim: Option<&'static str>,
    paint: &'static str,
    wheel: &'static str,
}

impl Spec {
    /// `None` when the body has no render or the paint is not one that renders.
    pub(crate) fn from_vehicle(
        body: Body,
        color: Option<&str>,
        wheel: Option<&str>,
    ) -> Option<Self> {
        let (model, trim, wheels) = match body {
            Body::Model3 => ("m3", Some("MT347"), WHEELS_MODEL_3),
            Body::ModelY => ("my", Some("MTY46"), WHEELS_MODEL_Y),
            Body::ModelS => ("ms", None, WHEELS_MODEL_S),
            Body::ModelX => ("mx", None, WHEELS_MODEL_X),
            Body::Cybertruck => return None,
        };
        let key = normalise(color?);
        let paint = PAINTS
            .iter()
            .find(|(names, _, bodies)| bodies.contains(&body) && names.contains(&key.as_str()))
            .map(|(_, code, _)| *code)?;
        let diameter = wheel.and_then(rim_diameter);
        let wheel = diameter
            .and_then(|inches| wheels.iter().find(|(size, _)| *size == inches))
            .or_else(|| wheels.first())
            .map(|(_, code)| *code)?;
        Some(Self {
            model,
            trim,
            paint,
            wheel,
        })
    }

    /// Cache key and file stem. Letters, digits and dashes only.
    pub(crate) fn key(&self) -> String {
        format!(
            "{}-{}-{}-{}",
            self.model,
            self.trim.unwrap_or("x"),
            self.paint,
            self.wheel
        )
    }

    /// Short stable token for the image URL, so a changed paint or wheel is not
    /// served from the browser cache.
    pub(crate) fn version(&self) -> String {
        crate::crypto::sha256_hex(&self.key())[..10].to_owned()
    }

    fn options(&self) -> String {
        let mut codes = Vec::new();
        if let Some(trim) = self.trim {
            codes.push(format!("${trim}"));
        }
        codes.push(format!("${}", self.paint));
        codes.push(format!("${}", self.wheel));
        codes.join(",")
    }
}

fn normalise(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// The two-digit rim size inside a wheel name such as `Pinwheel18` or `Stiletto20`.
fn rim_diameter(wheel: &str) -> Option<u32> {
    let digits: String = wheel
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits
        .parse()
        .ok()
        .filter(|inches| (17..=23).contains(inches))
}

enum Slot {
    Ready(Arc<Vec<u8>>),
    Missing(Instant),
}

/// Fetches renders from Tesla once and serves them from memory or disk.
pub(crate) struct CarRenders {
    enabled: bool,
    base: String,
    client: reqwest::Client,
    dir: Option<PathBuf>,
    memory: Mutex<HashMap<String, Slot>>,
    fetch_gate: Mutex<()>,
}

impl CarRenders {
    pub(crate) fn from_config(config: &Config) -> Self {
        let dir = config
            .token_store_path
            .parent()
            .map(|parent| parent.join("car-renders"));
        Self::new(config.car_renders_enabled, BASE_URL.to_owned(), dir)
    }

    pub(crate) fn new(enabled: bool, base: String, dir: Option<PathBuf>) -> Self {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(10))
            .user_agent(concat!("tesla-logger/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        Self {
            enabled,
            base,
            client,
            dir,
            memory: Mutex::new(HashMap::new()),
            fetch_gate: Mutex::new(()),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.enabled
    }

    /// The PNG for `spec`, or `None` when it cannot be had right now.
    pub(crate) async fn get(&self, spec: &Spec) -> Option<Arc<Vec<u8>>> {
        if !self.enabled {
            return None;
        }
        let key = spec.key();
        if let Some(found) = self.remembered(&key).await {
            return found;
        }
        if let Some(bytes) = self.read_disk(&key).await {
            return Some(self.remember_ready(key, bytes).await);
        }
        let _gate = self.fetch_gate.lock().await;
        if let Some(found) = self.remembered(&key).await {
            return found;
        }
        match self.fetch(spec).await {
            Ok(bytes) => {
                self.write_disk(&key, &bytes).await;
                Some(self.remember_ready(key, bytes).await)
            }
            Err(retry) => {
                self.remember_missing(key, retry).await;
                None
            }
        }
    }

    /// `Some(None)` is a recent failure that should not be retried yet.
    async fn remembered(&self, key: &str) -> Option<Option<Arc<Vec<u8>>>> {
        let mut memory = self.memory.lock().await;
        match memory.get(key) {
            Some(Slot::Ready(bytes)) => Some(Some(Arc::clone(bytes))),
            Some(Slot::Missing(until)) if *until > Instant::now() => Some(None),
            Some(Slot::Missing(_)) => {
                memory.remove(key);
                None
            }
            None => None,
        }
    }

    async fn remember_ready(&self, key: String, bytes: Vec<u8>) -> Arc<Vec<u8>> {
        let bytes = Arc::new(bytes);
        let mut memory = self.memory.lock().await;
        if memory.len() >= MEMORY_SLOTS {
            memory.clear();
        }
        memory.insert(key, Slot::Ready(Arc::clone(&bytes)));
        bytes
    }

    async fn remember_missing(&self, key: String, retry: Duration) {
        let mut memory = self.memory.lock().await;
        if memory.len() >= MEMORY_SLOTS {
            memory.clear();
        }
        memory.insert(key, Slot::Missing(Instant::now() + retry));
    }

    async fn read_disk(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.dir.as_ref()?.join(format!("{key}.png"));
        let bytes = tokio::fs::read(path).await.ok()?;
        valid_png(&bytes).then_some(bytes)
    }

    async fn write_disk(&self, key: &str, bytes: &[u8]) {
        let Some(dir) = &self.dir else {
            return;
        };
        let target = dir.join(format!("{key}.png"));
        let staging = dir.join(format!("{key}.png.part"));
        let written = async {
            tokio::fs::create_dir_all(dir).await?;
            tokio::fs::write(&staging, bytes).await?;
            tokio::fs::rename(&staging, &target).await
        }
        .await;
        if let Err(err) = written {
            tracing::warn!(error = %err, "car render cache is not writable; keeping it in memory");
        }
    }

    /// `Err` carries how long to wait before asking Tesla again.
    async fn fetch(&self, spec: &Spec) -> Result<Vec<u8>, Duration> {
        let url = format!("{}{COMPOSITOR_PATH}", self.base.trim_end_matches('/'));
        let response = self
            .client
            .get(url)
            .query(&[
                ("model", spec.model),
                ("view", "STUD_3QTR"),
                ("size", "800"),
                ("options", spec.options().as_str()),
                ("bkba_opt", "1"),
                ("context", "design_studio_2"),
            ])
            .send()
            .await
            .map_err(|err| {
                tracing::warn!(error = %crate::error::redact_secrets(&err.without_url().to_string()), "car render fetch failed");
                RETRY_AFTER_FAILURE
            })?;
        let status = response.status();
        if !status.is_success() {
            tracing::warn!(key = %spec.key(), status = status.as_u16(), "car render refused");
            return Err(if status.is_client_error() {
                RETRY_AFTER_REJECTION
            } else {
                RETRY_AFTER_FAILURE
            });
        }
        let bytes = response.bytes().await.map_err(|_| RETRY_AFTER_FAILURE)?;
        if bytes.len() > MAX_BYTES || !valid_png(&bytes) {
            tracing::warn!(key = %spec.key(), "car render is not a usable PNG");
            return Err(RETRY_AFTER_REJECTION);
        }
        Ok(bytes.to_vec())
    }
}

fn valid_png(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_BYTES && bytes.starts_with(PNG_MAGIC)
}

/// `GET /v/{id}/car.png`: the cached render for a car the session may see.
pub(super) async fn car_png(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    let Some(page) = super::ui::signed_page(&app, &headers).await else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if !page.vehicles.iter().any(|vehicle| vehicle.id == id) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(spec) = super::overview::render_spec(&app, id).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(bytes) = app.inner.car_renders.get(&spec).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let mut response = (*bytes).clone().into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=604800"),
    );
    response
}

/// `GET /api/v1/vehicles/{id}/car.png`: the same cached render, for a read token.
///
/// `{id}` is the numeric vehicle id. A missing picture, a disabled render
/// service, or a failed fetch is the JSON `404` used by the other API routes.
pub(super) async fn api_car_png(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Result<Response, super::ApiError> {
    if !super::vehicle_visible(id) {
        return Err(super::ApiError::NotFound);
    }
    let Some(spec) = super::overview::render_spec(&app, id).await else {
        return Err(super::ApiError::NotFound);
    };
    let Some(bytes) = app.inner.car_renders.get(&spec).await else {
        return Err(super::ApiError::NotFound);
    };
    let etag = entity_tag(&bytes);
    if not_modified(&headers, &etag) {
        return Ok((
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CACHE_CONTROL, cache_control()),
            ],
        )
            .into_response());
    }
    let mut response = (*bytes).clone().into_response();
    let out = response.headers_mut();
    out.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
    out.insert(header::ETAG, etag);
    out.insert(header::CACHE_CONTROL, cache_control());
    Ok(response)
}

fn cache_control() -> HeaderValue {
    HeaderValue::from_static("private, max-age=86400")
}

/// Strong entity tag over the PNG bytes. Quoted, never weak.
fn entity_tag(bytes: &[u8]) -> HeaderValue {
    use sha2::{Digest, Sha256};
    let quoted = format!("\"{}\"", hex::encode(Sha256::digest(bytes)));
    HeaderValue::from_str(&quoted).unwrap_or_else(|_| HeaderValue::from_static("\"0\""))
}

/// `If-None-Match` uses weak comparison: `W/"abc"` matches `"abc"`, and `*` matches.
fn not_modified(headers: &HeaderMap, etag: &HeaderValue) -> bool {
    let Some(raw) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
    else {
        return false;
    };
    let Some(current) = etag.to_str().ok() else {
        return false;
    };
    let current = opaque_tag(current);
    raw.split(',').any(|part| {
        let part = part.trim();
        part == "*" || opaque_tag(part) == current
    })
}

fn opaque_tag(value: &str) -> &str {
    let value = value.trim();
    value
        .strip_prefix("W/")
        .unwrap_or(value)
        .trim()
        .trim_matches('"')
}

#[cfg(test)]
mod tests {
    use axum::body::Body as HttpBody;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{CarRenders, Spec, rim_diameter};
    use crate::http::car::Body;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrender-bytes";
    const VIN: &str = "5YJYTESTVIN000042";

    fn spec() -> Spec {
        Spec::from_vehicle(Body::ModelY, Some("PearlWhite"), Some("Gemini19")).unwrap()
    }

    async fn compositor(server: &MockServer, response: ResponseTemplate, hits: u64) {
        Mock::given(method("GET"))
            .and(path("/v1/compositor/"))
            .and(query_param("model", "my"))
            .and(query_param("options", "$MTY46,$PPSW,$WY19B"))
            .respond_with(response)
            .expect(hits)
            .mount(server)
            .await;
    }

    fn scratch() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "tl-render-{}",
            crate::crypto::random_token().unwrap()
        ))
    }

    #[tokio::test]
    async fn a_render_is_fetched_once_and_then_read_from_disk() {
        let server = MockServer::start().await;
        compositor(
            &server,
            ResponseTemplate::new(200).set_body_raw(PNG, "image/png"),
            1,
        )
        .await;
        let dir = scratch();
        let renders = CarRenders::new(true, server.uri(), Some(dir.clone()));
        assert_eq!(renders.get(&spec()).await.as_deref(), Some(&PNG.to_vec()));
        assert_eq!(renders.get(&spec()).await.as_deref(), Some(&PNG.to_vec()));
        assert!(dir.join("my-MTY46-PPSW-WY19B.png").exists());
        let restarted = CarRenders::new(true, server.uri(), Some(dir));
        assert_eq!(restarted.get(&spec()).await.as_deref(), Some(&PNG.to_vec()));
    }

    #[tokio::test]
    async fn a_refusal_is_remembered_instead_of_retried() {
        let server = MockServer::start().await;
        compositor(&server, ResponseTemplate::new(412), 1).await;
        let renders = CarRenders::new(true, server.uri(), None);
        assert!(renders.get(&spec()).await.is_none());
        assert!(renders.get(&spec()).await.is_none());
    }

    #[tokio::test]
    async fn bytes_that_are_not_a_png_are_refused() {
        let server = MockServer::start().await;
        compositor(
            &server,
            ResponseTemplate::new(200).set_body_raw(b"<html>nope</html>".as_slice(), "text/html"),
            1,
        )
        .await;
        let renders = CarRenders::new(true, server.uri(), None);
        assert!(renders.get(&spec()).await.is_none());
    }

    #[tokio::test]
    async fn a_redirect_is_not_followed() {
        let server = MockServer::start().await;
        compositor(
            &server,
            ResponseTemplate::new(302).insert_header("location", "http://127.0.0.1:9/x.png"),
            1,
        )
        .await;
        let renders = CarRenders::new(true, server.uri(), None);
        assert!(renders.get(&spec()).await.is_none());
    }

    #[tokio::test]
    async fn disabled_renders_never_call_out() {
        let server = MockServer::start().await;
        compositor(&server, ResponseTemplate::new(200), 0).await;
        let renders = CarRenders::new(false, server.uri(), None);
        assert!(renders.get(&spec()).await.is_none());
    }

    async fn fetch(
        state: &crate::app::AppState,
        uri: &str,
        cookie: &str,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let response = crate::http::router(state.clone())
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(header::COOKIE, cookie)
                    .body(HttpBody::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, headers, body)
    }

    #[tokio::test]
    async fn route_serves_the_cached_render_and_the_page_links_it() {
        let Some(db) = crate::test_util::TestDb::migrated("car-render-route").await else {
            return;
        };
        let server = MockServer::start().await;
        compositor(
            &server,
            ResponseTemplate::new(200).set_body_raw(PNG, "image/png"),
            1,
        )
        .await;
        let renders = CarRenders::new(true, server.uri(), Some(scratch()));
        let state = crate::app::test_state_renders(db.pool.clone(), renders).await;
        let known: (i64,) = sqlx::query_as(
            "INSERT INTO vehicles (vin, name, model, exterior_color, wheel_type)
             VALUES ($1, 'Rendu', 'modely', 'PearlWhite', 'Gemini19') RETURNING id",
        )
        .bind(VIN)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let plain: (i64,) = sqlx::query_as(
            "INSERT INTO vehicles (vin, name, model, exterior_color)
             VALUES ('5YJYTESTVIN000043', 'Dessin', 'modely', 'StealthGrey') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let crate::identity::LoginResult::Session(issued) =
            crate::identity::login(&state, "admin", "test-password", "", "127.0.0.1")
                .await
                .unwrap()
        else {
            panic!("session");
        };
        sqlx::query("UPDATE users SET must_change_password = false, totp_enabled = true")
            .execute(&db.pool)
            .await
            .unwrap();
        let cookie = format!("__Host-tl_session={}", issued.id);

        let (status, headers, body) =
            fetch(&state, &format!("/v/{}/car.png", known.0), &cookie).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "image/png");
        assert_eq!(body, PNG);
        let (again, _, _) = fetch(&state, &format!("/v/{}/car.png", known.0), &cookie).await;
        assert_eq!(again, StatusCode::OK);

        let (status, _, _) = fetch(&state, &format!("/v/{}/car.png", plain.0), &cookie).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _, _) = fetch(&state, &format!("/v/{}/car.png", known.0), "").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (_, _, page) = fetch(&state, &format!("/v/{}", known.0), &cookie).await;
        let page = String::from_utf8(page).unwrap();
        assert!(page.contains(&format!("image-url=\"/v/{}/car.png?v=", known.0)));
        assert!(!page.contains("<svg class=\"car\""));
        assert!(page.contains("Image du véhicule indisponible"));
        assert!(!page.contains(VIN));
        assert!(!page.contains("tesla.com/v1/compositor"));
        let (_, _, page) = fetch(&state, &format!("/v/{}", plain.0), &cookie).await;
        let page = String::from_utf8(page).unwrap();
        assert!(!page.contains("car.png"));
        assert!(page.contains("Image du véhicule indisponible"));
        assert!(!page.contains("<svg class=\"car\""));

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let url = requests[0].url.as_str();
        assert!(
            !url.contains(VIN) && !url.to_ascii_lowercase().contains("vin"),
            "{url}"
        );
    }

    #[tokio::test]
    async fn api_serves_the_cached_render_with_token_auth_etag_and_json_404() {
        let Some(db) = crate::test_util::TestDb::migrated("car-render-api").await else {
            return;
        };
        let server = MockServer::start().await;
        compositor(
            &server,
            ResponseTemplate::new(200).set_body_raw(PNG, "image/png"),
            1,
        )
        .await;
        let renders = CarRenders::new(true, server.uri(), Some(scratch()));
        let state = crate::app::test_state_renders(db.pool.clone(), renders).await;
        let known: i64 = sqlx::query_scalar(
            "INSERT INTO vehicles (vin, name, model, exterior_color, wheel_type)
             VALUES ($1, 'Rendu', 'modely', 'PearlWhite', 'Gemini19') RETURNING id",
        )
        .bind(VIN)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let plain: i64 = sqlx::query_scalar(
            "INSERT INTO vehicles (vin, name, model, exterior_color)
             VALUES ('5YJYTESTVIN000043', 'Dessin', 'modely', 'StealthGrey') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let read = "tl_render-read";
        sqlx::query("INSERT INTO api_tokens (name, token_hash, scopes) VALUES ('read', $1, $2)")
            .bind(crate::crypto::sha256_hex(read))
            .bind(&["read".to_owned()])
            .execute(&db.pool)
            .await
            .unwrap();
        let live_only = "tl_render-live";
        sqlx::query("INSERT INTO api_tokens (name, token_hash, scopes) VALUES ('live', $1, $2)")
            .bind(crate::crypto::sha256_hex(live_only))
            .bind(&["read:live".to_owned()])
            .execute(&db.pool)
            .await
            .unwrap();

        let path = format!("/api/v1/vehicles/{known}/car.png");
        let (status, _, body) = api(&state, &path, None, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, br#"{"error":"unauthorized"}"#);

        let (status, _, body) = api(&state, &path, Some(live_only), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, br#"{"error":"unauthorized"}"#);

        let (status, headers, body) = api(&state, &path, Some(read), None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(headers.get(header::CONTENT_TYPE).unwrap(), "image/png");
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            "private, max-age=86400"
        );
        let etag = headers
            .get(header::ETAG)
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert!(
            etag.starts_with('"') && etag.ends_with('"') && !etag.starts_with("W/"),
            "{etag}"
        );
        assert_eq!(body, PNG);

        let (status, headers, body) = api(&state, &path, Some(read), Some(etag.as_str())).await;
        assert_eq!(status, StatusCode::NOT_MODIFIED);
        assert_eq!(body, b"");
        assert_eq!(headers.get(header::ETAG).unwrap(), etag.as_str());
        assert_eq!(
            headers.get(header::CACHE_CONTROL).unwrap(),
            "private, max-age=86400"
        );

        let (status, _, body) = api(&state, &path, Some(read), Some("\"other\"")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, PNG);

        let (status, headers, body) = api(
            &state,
            &format!("/api/v1/vehicles/{plain}/car.png"),
            Some(read),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        assert_eq!(body, br#"{"error":"not found"}"#);

        let (status, _, body) = api(
            &state,
            &format!("/api/v1/vehicles/{VIN}/car.png"),
            Some(read),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_ne!(body, PNG);

        let one = api(
            &state,
            &format!("/api/v1/vehicles/{known}"),
            Some(read),
            None,
        )
        .await;
        let one = String::from_utf8(one.2).unwrap();
        assert!(one.contains("\"car_render\":true"), "{one}");

        let vehicles = api(&state, "/api/v1/vehicles", Some(read), None).await;
        let listed = String::from_utf8(vehicles.2).unwrap();
        assert!(listed.contains("\"car_render\":true"), "{listed}");
        assert!(listed.contains("\"car_render\":false"), "{listed}");
        assert!(!listed.contains("static-assets.tesla.com"));

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let url = requests[0].url.as_str();
        assert!(
            !url.contains(VIN) && !url.to_ascii_lowercase().contains("vin"),
            "{url}"
        );
    }

    #[tokio::test]
    async fn api_render_is_404_when_renders_are_disabled() {
        let Some(db) = crate::test_util::TestDb::migrated("car-render-off").await else {
            return;
        };
        let state = crate::app::test_state(db.pool.clone()).await;
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO vehicles (vin, name, model, exterior_color, wheel_type)
             VALUES ('5YJYTESTVIN000044', 'Off', 'modely', 'PearlWhite', 'Gemini19') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let token = "tl_render-off";
        sqlx::query("INSERT INTO api_tokens (name, token_hash, scopes) VALUES ('off', $1, $2)")
            .bind(crate::crypto::sha256_hex(token))
            .bind(&["read".to_owned()])
            .execute(&db.pool)
            .await
            .unwrap();
        let (status, headers, body) = api(
            &state,
            &format!("/api/v1/vehicles/{id}/car.png"),
            Some(token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body, br#"{"error":"not found"}"#);
        assert_eq!(
            headers.get(header::CONTENT_TYPE).unwrap(),
            "application/json"
        );
        let listed =
            String::from_utf8(api(&state, "/api/v1/vehicles", Some(token), None).await.2).unwrap();
        assert!(listed.contains("\"car_render\":false"), "{listed}");
        assert!(!listed.contains("\"car_render\":true"));
    }

    async fn api(
        state: &crate::app::AppState,
        uri: &str,
        token: Option<&str>,
        if_none_match: Option<&str>,
    ) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
        let mut builder = Request::builder().uri(uri);
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        if let Some(etag) = if_none_match {
            builder = builder.header(header::IF_NONE_MATCH, etag);
        }
        let response = crate::http::router(state.clone())
            .oneshot(builder.body(HttpBody::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec();
        (status, headers, body)
    }

    #[test]
    fn spec_follows_model_paint_and_wheel() {
        let spec = Spec::from_vehicle(Body::ModelY, Some("PearlWhite"), Some("Induction20"))
            .expect("known car");
        assert_eq!(spec.key(), "my-MTY46-PPSW-WY20P");
        assert_eq!(spec.options(), "$MTY46,$PPSW,$WY20P");
        let spec = Spec::from_vehicle(Body::Model3, Some("deep_blue"), Some("Pinwheel18"))
            .expect("known car");
        assert_eq!(spec.options(), "$MT347,$PPSB,$W38B");
        let spec = Spec::from_vehicle(Body::ModelS, Some("Titanium"), None).expect("known car");
        assert_eq!(spec.options(), "$PPTI,$WT19");
    }

    #[test]
    fn unknown_wheel_size_uses_the_model_default() {
        let spec = Spec::from_vehicle(Body::ModelY, Some("SolidBlack"), Some("Mystery")).unwrap();
        assert_eq!(spec.wheel, "WY19B");
        let spec = Spec::from_vehicle(Body::ModelY, Some("SolidBlack"), None).unwrap();
        assert_eq!(spec.wheel, "WY19B");
    }

    #[test]
    fn paints_without_a_render_fall_back() {
        for color in [
            "Quicksilver",
            "StealthGrey",
            "UltraRed",
            "UltraWhite",
            "",
            "SteelGrey",
        ] {
            assert!(
                Spec::from_vehicle(Body::ModelY, Some(color), Some("Gemini19")).is_none(),
                "{color}"
            );
        }
        assert!(Spec::from_vehicle(Body::ModelY, None, None).is_none());
        assert!(Spec::from_vehicle(Body::Cybertruck, Some("PearlWhite"), None).is_none());
        assert!(Spec::from_vehicle(Body::ModelY, Some("Titanium"), None).is_none());
    }

    #[test]
    fn rim_diameter_reads_the_size_in_the_name() {
        assert_eq!(rim_diameter("Stiletto19"), Some(19));
        assert_eq!(rim_diameter("Uberturbine21"), Some(21));
        assert_eq!(rim_diameter("Gemini"), None);
        assert_eq!(rim_diameter("Wheel2"), None);
    }

    #[test]
    fn if_none_match_uses_weak_comparison() {
        let etag = super::entity_tag(b"\x89PNG\r\n\x1a\nrender-bytes");
        let mut headers = axum::http::HeaderMap::new();
        assert!(!super::not_modified(&headers, &etag));
        headers.insert(header::IF_NONE_MATCH, etag.clone());
        assert!(super::not_modified(&headers, &etag));
        let weak = format!("W/{etag}", etag = etag.to_str().unwrap());
        headers.insert(header::IF_NONE_MATCH, weak.parse().unwrap());
        assert!(super::not_modified(&headers, &etag));
        headers.insert(header::IF_NONE_MATCH, "\"other\", *".parse().unwrap());
        assert!(super::not_modified(&headers, &etag));
        headers.insert(header::IF_NONE_MATCH, "\"other\"".parse().unwrap());
        assert!(!super::not_modified(&headers, &etag));
    }

    #[test]
    fn key_and_options_hold_no_identifier() {
        let spec = Spec::from_vehicle(Body::ModelX, Some("SolidBlack"), Some("Turbine22")).unwrap();
        for text in [spec.key(), spec.options(), spec.version()] {
            assert!(
                text.chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-$,".contains(c))
            );
        }
    }
}
