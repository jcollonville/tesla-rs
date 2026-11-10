// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;

use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Datelike, TimeZone, Utc};
use serde::Deserialize;
use tesla_auth::{CallbackParams, TokenStore};

use super::markup::{
    AccessLine, AuditLine, Chrome, ConnectionView, FleetCard, GlobalTab, NavVehicle, Overview,
    Party, SessionLine, SettingRow, TokenLine, VehicleTab, connection_page, debug_page, fleet_page,
    login_page, missing_page, overview_page, security_page, settings_page, setup_page, system_page,
};
use super::{cookie, ui_session};
use crate::accounts::{self, TokenPublic};
use crate::app::AppState;
use crate::crypto::{public_key_fingerprint, random_token, sha256_hex};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(home).post(login))
        .route("/logout", post(logout))
        .route("/v/{id}", get(overview))
        .route("/v/{id}/drives", get(super::trips::drives))
        .route("/v/{id}/drives.csv", get(super::trips::drives_csv))
        .route("/v/{id}/drives/{drive_id}", get(super::trips::drive_detail))
        .route("/v/{id}/charges", get(super::trips::charges))
        .route("/v/{id}/charges.csv", get(super::trips::charges_csv))
        .route(
            "/v/{id}/charges/{charge_id}",
            get(super::trips::charge_detail),
        )
        .merge(super::edits::routes())
        .route("/v/{id}/stats", get(stats))
        .route("/v/{id}/connection", get(vehicle_connection))
        .route("/connection", get(account_connection))
        .route("/settings", get(settings))
        .route("/settings/override", post(save_setting))
        .route("/debug", get(debug_page_handler))
        .route("/system", get(system))
        .route("/system/backup", get(backup))
        .route("/security/passkey/start", post(passkey_start))
        .route("/security/passkey/finish", post(passkey_finish))
        .route("/accounts", post(add_account))
        .route("/accounts/{id}/disconnect", post(disconnect_account))
        .route("/security", get(security))
        .route("/security/setup", get(security_setup))
        .route("/security/password", post(change_password))
        .route("/security/totp", post(enroll_totp))
        .route("/security/totp/confirm", post(confirm_totp))
        .route("/sudo", post(sudo))
        .route("/users", post(create_user))
        .route("/vehicle-access", post(set_access))
        .route("/sessions/{id}/revoke", post(revoke_session))
        .route("/logs", get(logs_redirect))
        .route("/logs/events", get(logs_events))
        .route("/auth/login", get(oauth_start))
        .route("/auth/callback", get(oauth_callback))
        .route("/auth/disconnect", post(disconnect))
        .route("/auth/refresh", post(fleet_refresh))
        .route("/auth/owner", get(owner_redirect))
        .route("/auth/owner/start", post(super::owner::start))
        .route("/auth/owner/callback", post(super::owner::callback))
        .route("/auth/owner/refresh", post(super::owner::refresh))
        .route("/auth/owner/disconnect", post(super::owner::disconnect))
        .route("/tokens", post(create_token))
        .route("/tokens/{id}/revoke", post(revoke_token))
        .route("/static/ui.css", get(ui_css))
        .route("/static/debug.js", get(debug_js))
        .route("/static/tabs.js", get(tabs_js))
}

#[derive(Deserialize)]
struct LoginForm {
    password: String,
    csrf: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    totp: String,
}

#[derive(Deserialize)]
pub(super) struct CsrfForm {
    pub(super) csrf: String,
    #[serde(default)]
    pub(super) vehicle_id: Option<i64>,
}

#[derive(Deserialize)]
struct TokenForm {
    csrf: String,
    name: String,
    #[serde(default)]
    scopes: String,
    #[serde(default)]
    expires_days: String,
    #[serde(default)]
    vehicle_ids: String,
}

#[derive(Clone, sqlx::FromRow)]
pub(crate) struct VehicleRow {
    pub(crate) id: i64,
    name: String,
    account_id: Option<i64>,
    activity: Option<String>,
    battery_level: Option<f64>,
    last_poll_at: Option<DateTime<Utc>>,
    last_data_at: Option<DateTime<Utc>>,
    last_error: Option<String>,
}

pub(crate) struct Page {
    pub(crate) session_id: String,
    pub(crate) csrf: String,
    pub(crate) cookie: Option<HeaderValue>,
    pub(crate) vehicles: Vec<VehicleRow>,
    pub(crate) remembered: Option<i64>,
    pub(crate) admin: bool,
    pub(crate) username: String,
}

async fn home(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(page) = signed_page(&app, &headers).await else {
        return login_page_response(&app, &headers, None);
    };
    if page.vehicles.len() == 1 {
        return Redirect::to(&format!("/v/{}", page.vehicles[0].id)).into_response();
    }
    let month_start = month_start_utc(&app.inner.config.timezone);
    let months = month_by_vehicle(&app, month_start).await;
    let mut total_km = 0.0;
    let mut total_kwh = 0.0;
    let mut cards = Vec::new();
    for vehicle in &page.vehicles {
        let (km, kwh) = months.get(&vehicle.id).copied().unwrap_or((0.0, 0.0));
        total_km += km;
        total_kwh += kwh;
        let mode = app.inner.archive.gps.mode_name(vehicle.id).await;
        cards.push(FleetCard {
            id: vehicle.id,
            name: vehicle.name.clone(),
            activity: vehicle
                .activity
                .clone()
                .unwrap_or_else(|| "inconnu".to_owned()),
            battery: fmt_battery(vehicle.battery_level),
            position_mode: mode,
            last_seen: fmt_time(vehicle.last_data_at.or(vehicle.last_poll_at)),
            last_error: vehicle.last_error.clone(),
            month: format!("{km:.1} km · {kwh:.1} kWh"),
        });
    }
    let chrome = chrome(&page, None, None, GlobalTab::None, false);
    html_page(
        fleet_page(
            &chrome,
            &cards,
            &format!("{total_km:.1} km · {total_kwh:.1} kWh"),
        ),
        page.cookie,
    )
}

async fn overview(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    let Some(vehicle) = page.vehicles.iter().find(|vehicle| vehicle.id == id) else {
        return html_page(
            missing_page(&chrome(&page, None, None, GlobalTab::None, false)),
            page.cookie,
        );
    };
    remember_vehicle(&app, &page.session_id, id).await;
    let mode = app.inner.archive.gps.mode_name(id).await;
    let (latest_sample, queue_depth) = app.inner.stream_board.snapshot().await.ingestion(id);
    let drive = last_drive(&app, id).await;
    let charge = last_charge(&app, id).await;
    let view = Overview {
        name: vehicle.name.clone(),
        activity: vehicle
            .activity
            .clone()
            .unwrap_or_else(|| "inconnu".to_owned()),
        battery: fmt_battery(vehicle.battery_level),
        position_mode: mode,
        last_poll: fmt_time(vehicle.last_poll_at),
        last_data: fmt_time(vehicle.last_data_at),
        last_error: vehicle.last_error.clone().unwrap_or_else(|| "—".to_owned()),
        stream_lag: fmt_lag(latest_sample),
        stream_queue: queue_depth.to_string(),
        last_drive: drive,
        last_charge: charge,
    };
    html_page(
        overview_page(
            &chrome(
                &page,
                Some(id),
                Some(VehicleTab::Overview),
                GlobalTab::None,
                false,
            ),
            &view,
        ),
        page.cookie,
    )
}

/// Age of the newest Owner stream sample, `—` when none was seen.
fn fmt_lag(at: Option<DateTime<Utc>>) -> String {
    let Some(at) = at else {
        return "—".to_owned();
    };
    let secs = (Utc::now() - at).num_seconds().max(0);
    if secs < 60 {
        format!("{secs} s")
    } else if secs < 3_600 {
        format!("{} min", secs / 60)
    } else {
        format!("{} h", secs / 3_600)
    }
}

async fn stats(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(query): Query<StatsQuery>,
) -> Response {
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !page.vehicles.iter().any(|vehicle| vehicle.id == id) {
        return html_page(
            missing_page(&chrome(&page, None, None, GlobalTab::None, false)),
            page.cookie,
        );
    }
    remember_vehicle(&app, &page.session_id, id).await;
    let period = query.period.as_deref().unwrap_or("month");
    let count = i32::try_from(query.count.unwrap_or(6)).unwrap_or(6);
    let view = chrome(
        &page,
        Some(id),
        Some(VehicleTab::Stats),
        GlobalTab::None,
        false,
    );
    html_page(
        super::stats::console_page(&app, &view, id, period, count).await,
        page.cookie,
    )
}

#[derive(Deserialize)]
struct StatsQuery {
    #[serde(default)]
    period: Option<String>,
    #[serde(default)]
    count: Option<i64>,
}

async fn vehicle_connection(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    render_connection(&app, &headers, Some(id), None, None).await
}

async fn account_connection(State(app): State<AppState>, headers: HeaderMap) -> Response {
    render_connection(&app, &headers, None, None, None).await
}

pub(super) async fn render_connection(
    app: &AppState,
    headers: &HeaderMap,
    vehicle_id: Option<i64>,
    error: Option<&str>,
    notice: Option<&str>,
) -> Response {
    let Some(page) = signed_page(app, headers).await else {
        return Redirect::to("/").into_response();
    };
    if let Some(id) = vehicle_id
        && !page.vehicles.iter().any(|vehicle| vehicle.id == id)
    {
        return html_page(
            missing_page(&chrome(&page, None, None, GlobalTab::None, false)),
            page.cookie,
        );
    }
    let account_id = vehicle_id
        .and_then(|id| {
            page.vehicles
                .iter()
                .find(|vehicle| vehicle.id == id)
                .and_then(|vehicle| vehicle.account_id)
        })
        .unwrap_or(app.inner.fleet_account_id);
    let fleet_account = accounts::load_account(&app.inner.db, account_id)
        .await
        .ok()
        .flatten();
    let owner_account = accounts::load_account(&app.inner.db, app.inner.owner_account_id)
        .await
        .ok()
        .flatten();
    let fleet_token = app.inner.fleet_store.public_view().await;
    let owner_token = app.inner.owner_store.public_view().await;
    let shared = match accounts::vehicles_for_account(&app.inner.db, account_id).await {
        Ok(rows) => Ok(rows),
        Err(err) => {
            tracing::warn!(
                error = %crate::error::redact_secrets(&err.to_string()),
                "account vehicles could not be listed"
            );
            Err(())
        }
    };
    let shared_names = shared.as_ref().map(|rows| {
        rows.iter()
            .filter(|vehicle| Some(vehicle.id) != vehicle_id)
            .map(|vehicle| vehicle.name.clone())
            .collect::<Vec<_>>()
    });
    let snapshot = app.inner.stream_board.snapshot().await;
    let pending = app.inner.owner_pending.lock().await.clone();
    let authorize_url = pending.and_then(|(login, created)| {
        (created.elapsed() <= super::owner::PENDING_TTL).then(|| login.authorize_url().to_string())
    });
    if let Some(id) = vehicle_id {
        remember_vehicle(app, &page.session_id, id).await;
    }
    let section = vehicle_id.map(|_| VehicleTab::Connection);
    let chrome = chrome(&page, vehicle_id, section, GlobalTab::None, false);
    let view = ConnectionView {
        fleet: party("Compte Fleet", fleet_account.as_ref(), &fleet_token),
        owner_status: owner_account.as_ref().map_or_else(
            || {
                if owner_token.present {
                    "connecté".to_owned()
                } else {
                    "déconnecté".to_owned()
                }
            },
            |account| account.status.clone(),
        ),
        owner_tone: token_tone(
            &owner_token,
            owner_account
                .as_ref()
                .and_then(|account| account.last_error.as_deref()),
        ),
        owner_expires: fmt_time(owner_token.expires_at),
        owner_scopes: fmt_scopes(&owner_token.scopes),
        owner_last_refresh: fmt_time(
            owner_account
                .as_ref()
                .and_then(|account| account.last_refresh_at),
        ),
        owner_last_error: owner_account
            .as_ref()
            .and_then(|account| account.last_error.clone()),
        owner_connected: owner_token.present,
        fingerprint: public_key_fingerprint(&app.inner.public_key_pem),
        shared_names: shared_names.clone().unwrap_or_default(),
        shared_ok: shared_names.is_ok(),
        csrf: &page.csrf,
        vehicle_id,
        error,
        notice,
        stream_status: snapshot.summary(),
        streaming_enabled: app.inner.config.owner_stream_enabled,
        authorize_url: authorize_url.as_deref(),
    };
    html_page(connection_page(&chrome, &view), page.cookie)
}

fn party(fallback: &str, account: Option<&accounts::TeslaAccount>, token: &TokenPublic) -> Party {
    let status = if !token.readable {
        "illisible".to_owned()
    } else if token.present {
        account
            .map(|account| account.status.clone())
            .filter(|status| status != "unknown")
            .unwrap_or_else(|| "connecté".to_owned())
    } else {
        "déconnecté".to_owned()
    };
    let title = account.map_or_else(
        || fallback.to_owned(),
        |account| format!("{} · {} #{}", account.label, account.kind, account.id),
    );
    Party {
        title,
        tone: token_tone(
            token,
            account.and_then(|account| account.last_error.as_deref()),
        ),
        expires: fmt_time(token.expires_at),
        scopes: fmt_scopes(&token.scopes),
        last_refresh: fmt_time(account.and_then(|account| account.last_refresh_at)),
        last_error: account.and_then(|account| account.last_error.clone()),
        connected: token.present,
        status,
    }
}

fn token_tone(token: &TokenPublic, error: Option<&str>) -> &'static str {
    if !token.readable || error.is_some() {
        "bad"
    } else if token.present {
        "ok"
    } else {
        "neutral"
    }
}

fn fmt_scopes(scopes: &[String]) -> String {
    if scopes.is_empty() {
        "—".to_owned()
    } else {
        scopes.join(" ")
    }
}

async fn security_setup(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(_principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    html_page(setup_page(&page.csrf, None), page.cookie)
}

async fn change_password(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<PasswordForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    if let Err(err) =
        crate::identity::change_password(&app.inner.db, principal.user_id, &form.password).await
    {
        let Some(page) = signed_page(&app, &headers).await else {
            return Redirect::to("/").into_response();
        };
        return html_page(setup_page(&page.csrf, Some(&err.to_string())), page.cookie);
    }
    Redirect::to("/security/setup").into_response()
}

async fn enroll_totp(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    match crate::identity::enroll_totp(&app, principal.user_id).await {
        Ok(enrollment) => {
            let Some(page) = signed_page(&app, &headers).await else {
                return Redirect::to("/").into_response();
            };
            let mut body = setup_page(&page.csrf, None);
            let extra = format!(
                "<section class=\"card\"><h2>Secret TOTP</h2><code>{}</code><p>{}</p></section></main>",
                enrollment.secret,
                enrollment.recovery.join(" ")
            );
            body = body.replacen("</main>", &extra, 1);
            html_page(body, page.cookie)
        }
        Err(_) => Redirect::to("/security/setup").into_response(),
    }
}

async fn confirm_totp(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<TotpForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    let ok = crate::identity::confirm_totp(&app, principal.user_id, &form.code)
        .await
        .unwrap_or(false);
    if ok {
        Redirect::to("/").into_response()
    } else {
        let Some(page) = signed_page(&app, &headers).await else {
            return Redirect::to("/").into_response();
        };
        html_page(
            setup_page(&page.csrf, Some("Code TOTP refusé")),
            page.cookie,
        )
    }
}

async fn create_user(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<UserForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) || !principal.is_admin() || !principal.sudo_active() {
        return (StatusCode::FORBIDDEN, "sudo required").into_response();
    }
    let _ = crate::identity::create_user(
        &app.inner.db,
        principal.user_id,
        &form.username,
        &form.password,
        &form.role,
    )
    .await;
    Redirect::to("/security").into_response()
}

async fn set_access(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<AccessForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) || !principal.is_admin() || !principal.sudo_active() {
        return (StatusCode::FORBIDDEN, "sudo required").into_response();
    }
    let role = if form.role.is_empty() {
        None
    } else {
        Some(form.role.as_str())
    };
    let _ = crate::identity::set_vehicle_access(
        &app.inner.db,
        principal.user_id,
        form.user_id,
        form.vehicle_id,
        role,
    )
    .await;
    Redirect::to("/security").into_response()
}

async fn revoke_session(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) || !principal.is_admin() {
        return (StatusCode::FORBIDDEN, "admin required").into_response();
    }
    let _ = crate::identity::revoke_session(&app.inner.db, principal.user_id, &id).await;
    Redirect::to("/security").into_response()
}

#[derive(Deserialize)]
struct UserForm {
    csrf: String,
    username: String,
    password: String,
    role: String,
}

#[derive(Deserialize)]
struct AccessForm {
    csrf: String,
    user_id: i64,
    vehicle_id: i64,
    #[serde(default)]
    role: String,
}

async fn sudo(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<PasswordForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    let _ = crate::identity::grant_sudo(&app.inner.db, &principal, &form.password).await;
    Redirect::to("/security").into_response()
}

#[derive(Deserialize)]
struct PasswordForm {
    csrf: String,
    password: String,
}

#[derive(Deserialize)]
struct TotpForm {
    csrf: String,
    code: String,
}

async fn settings(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    let rows = app
        .inner
        .config
        .public_settings(page.admin)
        .into_iter()
        .map(|row| SettingRow {
            label: row.label,
            value: row.value,
            source: row.source,
        })
        .collect::<Vec<_>>();
    let mut body = settings_page(
        &chrome(&page, None, None, GlobalTab::Settings, false),
        &rows,
    );
    if page.admin {
        let form = maud::html! {
            section class="card" {
                h2 { "Remplacer un réglage" }
                p class="muted" { "La valeur est appliquée tout de suite. Un champ vide revient à l'environnement. Les secrets restent dans l'environnement." }
                form class="stack" method="post" action="/settings/override" {
                    input type="hidden" name="csrf" value=(&page.csrf);
                    label class="field" for="setting-key" {
                        "Clé"
                        select id="setting-key" name="key" {
                            @for key in crate::settings::KEYS {
                                option value=(key) { (key) }
                            }
                        }
                    }
                    label class="field" for="setting-value" { "Valeur" input id="setting-value" name="value"; }
                    label class="field" for="setting-vehicle" { "Véhicule, vide pour tous" input id="setting-vehicle" name="vehicle_id"; }
                    button type="submit" { "Enregistrer" }
                }
            }
        };
        let extra = form.into_string();
        body = body.replacen("</main>", &(extra + "</main>"), 1);
    }
    html_page(body, page.cookie)
}

#[derive(Deserialize)]
struct SettingForm {
    csrf: String,
    key: String,
    #[serde(default)]
    value: String,
    #[serde(default)]
    vehicle_id: String,
}

async fn save_setting(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<SettingForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) || !principal.is_admin() || !principal.sudo_active() {
        return (StatusCode::FORBIDDEN, "sudo required").into_response();
    }
    let vehicle = form.vehicle_id.trim().parse::<i64>().ok();
    if let Err(err) = crate::settings::set(&app.inner.db, vehicle, &form.key, &form.value).await {
        tracing::warn!(error = %err, "setting was not stored");
    } else {
        crate::identity::audit(
            &app.inner.db,
            Some(principal.user_id),
            "settings.change",
            "ok",
            &serde_json::json!({ "key": form.key }).to_string(),
        )
        .await;
    }
    Redirect::to("/settings").into_response()
}

async fn system(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    let mut body = system_page(&chrome(&page, None, None, GlobalTab::System, false));
    let extra = system_status(&app).await;
    body = body.replacen("</main>", &(extra + "</main>"), 1);
    html_page(body, page.cookie)
}

#[allow(clippy::format_push_string)]
async fn system_status(app: &AppState) -> String {
    let size: String =
        sqlx::query_scalar("SELECT pg_size_pretty(pg_database_size(current_database()))")
            .fetch_one(&app.inner.db)
            .await
            .unwrap_or_else(|_| "—".to_owned());
    let default_rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM positions_default")
        .fetch_one(&app.inner.db)
        .await
        .unwrap_or(0);
    let migrations: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(&app.inner.db)
        .await
        .unwrap_or(0);
    let cursors: Vec<(String, i64)> =
        sqlx::query_as("SELECT source, cursor_id FROM import_cursors ORDER BY source")
            .fetch_all(&app.inner.db)
            .await
            .unwrap_or_default();
    let accounts: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT id, label, status FROM tesla_accounts ORDER BY id")
            .fetch_all(&app.inner.db)
            .await
            .unwrap_or_default();
    let alert = crate::system::storage_alert(default_rows, chrono::Utc::now().year());
    let mode = escape_html(&app.inner.config.auth_mode);
    let oidc = if app.inner.config.oidc_issuer.is_some() {
        "configuré"
    } else {
        "absent"
    };
    let allow = app.inner.config.proxy_allowlist.len();
    let mut text = format!(
        "<section class=\"card\"><h2>Base</h2><p>Taille {size}. Migrations appliquées : {migrations}. Lignes dans positions_default : {default_rows}. Auth {mode}, OIDC {oidc}, allowlist {allow}.</p>"
    );
    if let Some(alert) = alert {
        text.push_str(&format!(
            "<p class=\"banner bad\" role=\"alert\">{alert}</p>"
        ));
    }
    text.push_str("<h2>Comptes Tesla</h2><ul class=\"list\">");
    for (_id, label, status) in accounts {
        let label = escape_html(&label);
        let status = escape_html(&status);
        text.push_str(&format!("<li><span>{label} · {status}</span></li>"));
    }
    text.push_str("</ul><h2>Curseurs d'import</h2><ul class=\"list\">");
    for (source, cursor) in cursors {
        let source = escape_html(&source);
        text.push_str(&format!("<li><span>{source} · {cursor}</span></li>"));
    }
    text.push_str(
        "</ul><p>Sauvegarde : envoi authentifié vers /system/backup (admin et sudo).</p></section>",
    );
    text
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn backup(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !principal.is_admin() || !principal.sudo_active() {
        return (StatusCode::FORBIDDEN, "sudo required").into_response();
    }
    let output = std::process::Command::new("pg_dump")
        .arg("--dbname")
        .arg(&app.inner.config.database_url)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let mut response = output.stdout.into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/sql"),
            );
            response.headers_mut().insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"tesla-logger.sql\""),
            );
            response
        }
        _ => (StatusCode::BAD_GATEWAY, "pg_dump failed").into_response(),
    }
}

async fn passkey_finish(
    State(app): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    let parsed = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
    match crate::passkeys::finish_registration(
        &app.inner.db,
        &app.inner.config,
        principal.user_id,
        &parsed,
    )
    .await
    {
        Ok(()) => Redirect::to("/security").into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    }
}

async fn passkey_start(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    match crate::passkeys::start_registration(
        &app.inner.db,
        &app.inner.config,
        principal.user_id,
        &principal.username,
    )
    .await
    {
        Ok(body) => axum::Json(body).into_response(),
        Err(err) => (StatusCode::BAD_REQUEST, err.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct AccountForm {
    csrf: String,
    label: String,
}

async fn add_account(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<AccountForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) || !principal.is_admin() || !principal.sudo_active() {
        return (StatusCode::FORBIDDEN, "sudo required").into_response();
    }
    let label = form.label.trim();
    if label.is_empty() {
        return (StatusCode::BAD_REQUEST, "label required").into_response();
    }
    let _ = sqlx::query("INSERT INTO tesla_accounts (label, kind) VALUES ($1, 'fleet')")
        .bind(label)
        .execute(&app.inner.db)
        .await;
    Redirect::to("/settings").into_response()
}

async fn disconnect_account(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) || !principal.is_admin() || !principal.sudo_active() {
        return (StatusCode::FORBIDDEN, "sudo required").into_response();
    }
    let _ = sqlx::query(
        "UPDATE tesla_accounts SET tokens = NULL, status = 'disconnected', last_error = NULL WHERE id = $1",
    )
    .bind(id)
    .execute(&app.inner.db)
    .await;
    Redirect::to("/settings").into_response()
}

async fn security(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    let tokens = token_rows(&app).await.unwrap_or_default();
    let revealed = if let Some(session) = cookie(&headers, crate::identity::SESSION_COOKIE) {
        app.inner.revealed.lock().await.remove(session)
    } else {
        None
    };
    let lines = tokens
        .iter()
        .map(|token| TokenLine {
            id: token.id,
            name: token.name.clone(),
            active: token.revoked_at.is_none(),
            scopes: token.scopes.join(" "),
        })
        .collect::<Vec<_>>();
    let access = access_lines(&app).await;
    let sessions = session_lines(&app).await;
    let audit = audit_lines(&app).await;
    html_page(
        security_page(
            &chrome(&page, None, None, GlobalTab::Security, false),
            &lines,
            revealed.as_deref(),
            &page.username,
            &access,
            &sessions,
            &audit,
        ),
        page.cookie,
    )
}

async fn debug_page_handler(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    crate::debug::probe(&app).await;
    let console = app.inner.debug.console(&app.inner.logs.lines());
    html_page(
        debug_page(
            &chrome(&page, None, None, GlobalTab::Debug, true),
            console.frames_cursor,
            console.logs_cursor,
            &console.body,
        ),
        page.cookie,
    )
}

async fn logs_redirect(State(app): State<AppState>, headers: HeaderMap) -> Response {
    if ui_session(&app, &headers).await.is_none() {
        return Redirect::to("/").into_response();
    }
    Redirect::to("/debug").into_response()
}

async fn owner_redirect(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    let vehicles = vehicle_rows(&app, principal.user_id, principal.is_admin())
        .await
        .unwrap_or_default();
    if vehicles.len() == 1 {
        Redirect::to(&format!("/v/{}/connection", vehicles[0].id)).into_response()
    } else {
        Redirect::to("/connection").into_response()
    }
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

async fn login(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    if app.inner.config.auth_mode == "proxy" {
        let ip = crate::identity::client_ip(&headers);
        if !crate::system::proxy_ip_allowed(&app.inner.config.proxy_allowlist, &ip) {
            return (StatusCode::FORBIDDEN, "proxy address is not allowed").into_response();
        }
    }
    let ip = crate::identity::client_ip(&headers);
    let outcome =
        match crate::identity::login(&app, &form.username, &form.password, &form.totp, &ip).await {
            Ok(outcome) => outcome,
            Err(err) => {
                tracing::warn!(error = %err, "login failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "login failed").into_response();
            }
        };
    let id = match outcome {
        crate::identity::LoginResult::Session(issued) => issued.id,
        crate::identity::LoginResult::Locked => {
            return login_page_response(&app, &headers, Some("Compte temporairement bloqué"));
        }
        crate::identity::LoginResult::Rejected => {
            return login_page_response(&app, &headers, Some("Mot de passe incorrect"));
        }
    };
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_str(&crate::identity::session_cookie(&id)).unwrap_or(
            HeaderValue::from_static(
                "__Host-tl_session=; HttpOnly; Secure; Path=/; Max-Age=0; SameSite=Strict",
            ),
        ),
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
    if let Some(principal) = ui_session(&app, &headers).await {
        let _ = crate::identity::revoke_session(
            &app.inner.db,
            principal.user_id,
            &crate::crypto::sha256_hex(&principal.raw_id),
        )
        .await;
    }
    let mut response = Redirect::to("/").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        HeaderValue::from_static(crate::identity::clear_session_cookie()),
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
        Ok(_) => Redirect::to("/connection").into_response(),
        Err(err) => {
            tracing::warn!(
                error = %crate::logs::redact_line(&err.to_string(), &[]),
                "tesla login failed"
            );
            (StatusCode::BAD_GATEWAY, "Tesla sign-in failed").into_response()
        }
    }
}

async fn disconnect(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) || !principal.sudo_active() {
        return (StatusCode::FORBIDDEN, "sudo required").into_response();
    }
    if let Err(err) = app.inner.fleet_store.clear().await {
        tracing::warn!(
            error = %crate::logs::redact_line(&err.to_string(), &[]),
            "could not clear tesla tokens"
        );
    }
    Redirect::to(&connection_location(form.vehicle_id)).into_response()
}

async fn fleet_refresh(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    if ui_session(&app, &headers).await.is_none() || !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    let failed = |message: &str| message.to_owned();
    let outcome = match app.inner.session.force_refresh().await {
        Ok(_) => None,
        Err(err) => {
            tracing::warn!(
                error = %crate::logs::redact_line(&err.to_string(), &[]),
                "fleet token refresh failed"
            );
            Some(failed("Le rafraîchissement Fleet a été refusé."))
        }
    };
    if let Some(message) = outcome {
        return render_connection(&app, &headers, form.vehicle_id, Some(&message), None).await;
    }
    Redirect::to(&connection_location(form.vehicle_id)).into_response()
}

async fn create_token(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    let Some(session) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !session.is_admin() || !session.sudo_active() {
        return (StatusCode::FORBIDDEN, "sudo required").into_response();
    }
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
    let scopes = form
        .scopes
        .split_whitespace()
        .filter(|scope| matches!(*scope, "read" | "read:live" | "write" | "admin"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let scopes = if scopes.is_empty() {
        vec!["read".to_owned(), "read:live".to_owned()]
    } else {
        scopes
    };
    let vehicle_ids = form
        .vehicle_ids
        .split(|ch: char| !ch.is_ascii_digit())
        .filter_map(|part| part.parse::<i64>().ok())
        .collect::<Vec<_>>();
    let vehicle_ids = if vehicle_ids.is_empty() {
        None
    } else {
        Some(vehicle_ids)
    };
    let expires_days = form
        .expires_days
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|days| *days > 0);
    if let Err(err) = sqlx::query(
        "INSERT INTO api_tokens (name, token_hash, user_id, scopes, expires_at, vehicle_ids)
         VALUES ($1, $2, $3, $4, CASE WHEN $5::bigint IS NULL THEN NULL ELSE now() + make_interval(days => $5::int) END, $6)",
    )
    .bind(name)
    .bind(hash)
    .bind(session.user_id)
    .bind(&scopes)
    .bind(expires_days)
    .bind(vehicle_ids.as_deref())
        .execute(&app.inner.db)
        .await
    {
        tracing::warn!(
            error = %crate::error::redact_secrets(&err.to_string()),
            "could not store api token"
        );
        return (StatusCode::INTERNAL_SERVER_ERROR, "could not store token").into_response();
    }
    app.inner
        .revealed
        .lock()
        .await
        .insert(session.raw_id, stored);
    crate::identity::audit(
        &app.inner.db,
        Some(session.user_id),
        "token.create",
        "ok",
        "{}",
    )
    .await;
    Redirect::to("/security").into_response()
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
    Redirect::to("/security").into_response()
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

fn login_page_response(app: &AppState, headers: &HeaderMap, error: Option<&str>) -> Response {
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
    html_page(login_page(&csrf, error), set_cookie)
}

pub(super) fn csrf_ok(headers: &HeaderMap, form: &str) -> bool {
    cookie(headers, "tl_csrf").is_some_and(|cookie| cookie == form && !form.is_empty())
}

pub(super) fn secure_flag(app: &AppState) -> &'static str {
    if app.inner.config.public_base_url.starts_with("https://") {
        "; Secure"
    } else {
        ""
    }
}

pub(super) fn connection_location(vehicle_id: Option<i64>) -> String {
    match vehicle_id {
        Some(id) if id > 0 => format!("/v/{id}/connection"),
        _ => "/connection".to_owned(),
    }
}

pub(crate) async fn signed_page(app: &AppState, headers: &HeaderMap) -> Option<Page> {
    let principal = ui_session(app, headers).await?;
    let session_id = principal.raw_id.clone();
    let remembered = principal.last_vehicle;
    let user_id = principal.user_id;
    let admin = principal.is_admin();
    let username = principal.username;
    let csrf = ensure_csrf(app, headers).ok()?;
    let vehicles = match vehicle_rows(app, user_id, admin).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(
                error = %crate::error::redact_secrets(&err.to_string()),
                "vehicle list failed"
            );
            Vec::new()
        }
    };
    Some(Page {
        session_id,
        csrf: csrf.0,
        cookie: csrf.1,
        vehicles,
        remembered,
        admin,
        username,
    })
}

pub(crate) async fn remember_vehicle(app: &AppState, session_id: &str, id: i64) {
    crate::identity::remember_vehicle(&app.inner.db, session_id, id).await;
}

pub(crate) fn chrome(
    page: &Page,
    current: Option<i64>,
    section: Option<VehicleTab>,
    global: GlobalTab,
    debug_script: bool,
) -> Chrome {
    Chrome {
        csrf: page.csrf.clone(),
        vehicles: nav_from(page, current),
        section,
        global,
        debug_script,
        remembered: page.remembered,
    }
}

pub(crate) fn html_page(body: String, set_cookie: Option<HeaderValue>) -> Response {
    let mut response = Html(body).into_response();
    if let Some(cookie) = set_cookie {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
}

async fn vehicle_rows(
    app: &AppState,
    user_id: i64,
    admin: bool,
) -> Result<Vec<VehicleRow>, sqlx::Error> {
    let see_all = admin
        && crate::settings::text_or(&app.inner.db, None, "ADMIN_SEES_ALL_TRIPS", "true").await
            != "false";
    if see_all {
        return sqlx::query_as(
            "SELECT v.id,
                    COALESCE(NULLIF(v.name, ''), 'Véhicule ' || v.id::text) AS name,
                    v.account_id,
                    s.activity,
                    s.battery_level,
                    s.last_poll_at,
                    s.last_data_at,
                    s.last_error
             FROM vehicles v
             LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
             ORDER BY v.id",
        )
        .fetch_all(&app.inner.db)
        .await;
    }
    sqlx::query_as(
        "SELECT v.id,
                COALESCE(NULLIF(v.name, ''), 'Véhicule ' || v.id::text) AS name,
                v.account_id,
                s.activity,
                s.battery_level,
                s.last_poll_at,
                s.last_data_at,
                s.last_error
         FROM vehicles v
         LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
         WHERE v.id IN (SELECT vehicle_id FROM vehicle_access WHERE user_id = $1)
         ORDER BY v.id",
    )
    .bind(user_id)
    .fetch_all(&app.inner.db)
    .await
}

async fn month_by_vehicle(app: &AppState, start: DateTime<Utc>) -> HashMap<i64, (f64, f64)> {
    let drives: Vec<(i64, Option<f64>)> = sqlx::query_as(
        "SELECT vehicle_id, COALESCE(SUM(distance_km), 0)
         FROM drives WHERE deleted_at IS NULL AND start_at >= $1 GROUP BY vehicle_id",
    )
    .bind(start)
    .fetch_all(&app.inner.db)
    .await
    .unwrap_or_default();
    let charges: Vec<(i64, Option<f64>)> = sqlx::query_as(
        "SELECT vehicle_id, COALESCE(SUM(energy_added_kwh), 0)
         FROM charges WHERE deleted_at IS NULL AND start_at >= $1 GROUP BY vehicle_id",
    )
    .bind(start)
    .fetch_all(&app.inner.db)
    .await
    .unwrap_or_default();
    let mut map = HashMap::new();
    for (id, km) in drives {
        map.insert(id, (km.unwrap_or(0.0), 0.0));
    }
    for (id, kwh) in charges {
        let entry = map.entry(id).or_insert((0.0, 0.0));
        entry.1 = kwh.unwrap_or(0.0);
    }
    map
}

type ActivityBrief = (DateTime<Utc>, Option<f64>, Option<DateTime<Utc>>);

async fn last_drive(app: &AppState, vehicle_id: i64) -> String {
    let row: Option<ActivityBrief> = sqlx::query_as(
        "SELECT start_at, distance_km, end_at FROM drives
         WHERE vehicle_id = $1 AND deleted_at IS NULL ORDER BY start_at DESC LIMIT 1",
    )
    .bind(vehicle_id)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten();
    match row {
        Some((start, distance, end)) => {
            let distance = distance.map_or_else(|| "—".to_owned(), |km| format!("{km:.1} km"));
            let state = if end.is_some() { "terminé" } else { "ouvert" };
            format!("{distance} · {state} · {}", fmt_time(Some(start)))
        }
        None => "Aucun trajet".to_owned(),
    }
}

async fn last_charge(app: &AppState, vehicle_id: i64) -> String {
    let row: Option<ActivityBrief> = sqlx::query_as(
        "SELECT start_at, energy_added_kwh, end_at FROM charges
         WHERE vehicle_id = $1 AND deleted_at IS NULL ORDER BY start_at DESC LIMIT 1",
    )
    .bind(vehicle_id)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten();
    match row {
        Some((start, energy, end)) => {
            let energy = energy.map_or_else(|| "—".to_owned(), |kwh| format!("{kwh:.1} kWh"));
            let state = if end.is_some() {
                "terminée"
            } else {
                "ouverte"
            };
            format!("{energy} · {state} · {}", fmt_time(Some(start)))
        }
        None => "Aucune charge".to_owned(),
    }
}

#[derive(sqlx::FromRow)]
struct TokenRow {
    id: i64,
    name: String,
    revoked_at: Option<DateTime<Utc>>,
    scopes: Vec<String>,
}

async fn token_rows(app: &AppState) -> Result<Vec<TokenRow>, sqlx::Error> {
    sqlx::query_as("SELECT id, name, revoked_at, scopes FROM api_tokens ORDER BY id DESC LIMIT 20")
        .fetch_all(&app.inner.db)
        .await
}

async fn access_lines(app: &AppState) -> Vec<AccessLine> {
    sqlx::query_as::<_, (String, String, String)>(
        "SELECT u.username, COALESCE(v.name, v.id::text), a.role
         FROM vehicle_access a
         JOIN users u ON u.id = a.user_id
         JOIN vehicles v ON v.id = a.vehicle_id
         ORDER BY u.username, v.id",
    )
    .fetch_all(&app.inner.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(user, vehicle, role)| AccessLine {
        user,
        vehicle,
        role,
    })
    .collect()
}

async fn session_lines(app: &AppState) -> Vec<SessionLine> {
    sqlx::query_as::<_, (String, String, DateTime<Utc>)>(
        "SELECT s.id_hash, u.username, s.last_seen_at
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.revoked_at IS NULL AND s.expires_at > now()
         ORDER BY s.last_seen_at DESC
         LIMIT 20",
    )
    .fetch_all(&app.inner.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(id_hash, user, seen)| SessionLine {
        id_hash,
        user,
        seen: seen.format("%Y-%m-%d %H:%M UTC").to_string(),
    })
    .collect()
}

async fn audit_lines(app: &AppState) -> Vec<AuditLine> {
    sqlx::query_as::<_, (DateTime<Utc>, String, String)>(
        "SELECT at, action, outcome FROM audit_log ORDER BY id DESC LIMIT 40",
    )
    .fetch_all(&app.inner.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(at, action, outcome)| AuditLine {
        at: at.format("%Y-%m-%d %H:%M UTC").to_string(),
        action,
        outcome,
    })
    .collect()
}

fn nav_from(page: &Page, current: Option<i64>) -> Vec<NavVehicle> {
    page.vehicles
        .iter()
        .map(|vehicle| NavVehicle {
            id: vehicle.id,
            name: vehicle.name.clone(),
            current: Some(vehicle.id) == current,
        })
        .collect()
}

fn fmt_battery(value: Option<f64>) -> String {
    value.map_or_else(|| "—".to_owned(), |value| format!("{value:.0}%"))
}

fn fmt_time(value: Option<DateTime<Utc>>) -> String {
    value.map_or_else(
        || "—".to_owned(),
        |value| value.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
    )
}

fn month_start_utc(timezone: &str) -> DateTime<Utc> {
    let tz: chrono_tz::Tz = timezone.parse().unwrap_or(chrono_tz::UTC);
    let local = Utc::now().with_timezone(&tz);
    tz.with_ymd_and_hms(local.year(), local.month(), 1, 0, 0, 0)
        .single()
        .map_or_else(Utc::now, |start| start.with_timezone(&Utc))
}

async fn ui_css() -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/css; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            ),
        ],
        include_str!("ui.css"),
    )
        .into_response()
}

async fn debug_js() -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/javascript; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            ),
        ],
        include_str!("debug.js"),
    )
        .into_response()
}

async fn tabs_js() -> Response {
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/javascript; charset=utf-8"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=86400"),
            ),
        ],
        include_str!("tabs.js"),
    )
        .into_response()
}
