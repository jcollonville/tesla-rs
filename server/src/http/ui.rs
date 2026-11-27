// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use chrono::{DateTime, Datelike, TimeZone, Utc};
use serde::Deserialize;
use tesla_auth::{CallbackParams, ExposeSecret, TokenStore};
use zeroize::Zeroize;

use super::markup::{
    AccessLine, AuditLine, Chrome, ConnectionView, FleetCard, GlobalTab, NavVehicle, Overview,
    SessionLine, SettingRow, SetupStep, TokenLine, VehicleTab, connection_page, debug_page,
    fleet_page, login_page, missing_page, overview_page, security_page, settings_page, setup_page,
    system_page,
};
use super::{cookie, ui_session};
use crate::accounts::{self, TokenPublic};
use crate::app::{AppState, PendingLogin};
use crate::crypto::{public_key_fingerprint, random_token, sha256_hex};

pub(crate) fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(home).post(login))
        .route("/logout", post(logout))
        .route("/v/{id}", get(overview))
        .route("/v/{id}/car.png", get(super::render::car_png))
        .route("/v/{id}/drives", get(super::trips::drives))
        .route(
            "/v/{id}/drives/split-candidates",
            get(super::trips::split_candidates_page),
        )
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
        .route("/connection/partner", post(register_partner))
        .route("/connection/telemetry", post(resend_telemetry))
        .route("/v/{id}/delete", post(delete_vehicle))
        .route("/settings", get(settings))
        .route("/settings/override", post(save_setting))
        .route("/debug", get(debug_page_handler))
        .route("/system", get(system))
        .route("/system/fleet/resume", post(resume_fleet))
        .route("/system/backup", get(backup))
        .route("/security/passkey/start", post(passkey_start))
        .route("/security/passkey/finish", post(passkey_finish))
        .route("/accounts", post(add_account))
        .route("/accounts/{id}/disconnect", post(disconnect_account))
        .route("/security", get(security))
        .route("/security/setup", get(security_setup))
        .route("/security/setup/finish", post(finish_setup))
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
        .route("/auth/callback/complete", get(oauth_complete))
        .route("/auth/disconnect", post(disconnect))
        .route("/auth/refresh", post(fleet_refresh))
        .route("/tokens", post(create_token))
        .route("/tokens/{id}/revoke", post(revoke_token))
        .route("/static/ui.css", get(ui_css))
        .route("/static/debug.js", get(debug_js))
        .route("/static/tabs.js", get(tabs_js))
        .route("/static/setup.js", get(setup_js))
        .route("/static/charts.js", get(charts_js))
        .route("/static/tesla-logger-ui.js", get(ui_js))
        .route("/static/map.js", get(map_js))
        .route("/static/maplibre-gl.css", get(maplibre_css))
        .route("/static/maplibre-gl-csp.js", get(maplibre_js))
        .route("/static/maplibre-gl-csp-worker.js", get(maplibre_worker))
        .route("/tiles/p/{*path}", get(super::tiles::proxy))
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
    #[serde(default)]
    pub(super) confirm: String,
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
    pub(crate) sudo: bool,
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
        let mut card = FleetCard {
            id: vehicle.id,
            name: vehicle.name.clone(),
            activity: vehicle
                .activity
                .clone()
                .unwrap_or_else(|| "inconnu".to_owned()),
            battery: fmt_battery(vehicle.battery_level),
            last_seen: super::format::ago(
                vehicle.last_data_at.or(vehicle.last_poll_at),
                Utc::now(),
            ),
            last_error: vehicle.last_error.clone(),
            month: format!(
                "{} · {}",
                super::format::km(Some(km)),
                super::format::kwh(Some(kwh))
            ),
            ..FleetCard::default()
        };
        super::overview::decorate_fleet_card(&app, &mut card).await;
        cards.push(card);
    }
    let chrome = chrome(&page, None, None, GlobalTab::None, false);
    html_page(
        fleet_page(
            &chrome,
            &cards,
            &format!(
                "{} parcourus · {} chargés",
                super::format::km(Some(total_km)),
                super::format::kwh(Some(total_kwh))
            ),
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
    let tz = super::format::zone(&app, Some(id)).await;
    let (last_drive, last_charge, car, month) = tokio::join!(
        super::overview::recent_drive(&app, id, tz),
        super::overview::recent_charge(&app, id, tz),
        super::overview::car_card(&app, id),
        super::overview::month(&app, id, tz),
    );
    let view = Overview {
        name: vehicle.name.clone(),
        activity: vehicle
            .activity
            .clone()
            .unwrap_or_else(|| "inconnu".to_owned()),
        battery: fmt_battery(vehicle.battery_level),
        last_poll: fmt_time(vehicle.last_poll_at),
        last_data: fmt_time(vehicle.last_data_at),
        last_error: vehicle.last_error.clone().unwrap_or_else(|| "—".to_owned()),
        last_drive,
        last_charge,
        telemetry_notice: super::overview::telemetry_notice(&app, id).await,
        car,
        month,
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
    let fleet_token = app.inner.fleet_store.public_view().await;
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
    if let Some(id) = vehicle_id {
        remember_vehicle(app, &page.session_id, id).await;
    }
    let section = vehicle_id.map(|_| VehicleTab::Connection);
    let chrome = chrome(&page, vehicle_id, section, GlobalTab::None, false);
    let view = ConnectionView {
        admin: page.admin,
        sudo: page.sudo,
        shared_names: shared_names.clone().unwrap_or_default(),
        shared_ok: shared_names.is_ok(),
        csrf: &page.csrf,
        vehicle_id,
        error,
        notice,
        report: connection_report(app, &page, account_id, &fleet_token, fleet_account.as_ref())
            .await,
    };
    html_page(connection_page(&chrome, &view), page.cookie)
}

async fn connection_report(
    app: &AppState,
    page: &Page,
    account_id: i64,
    fleet_token: &TokenPublic,
    fleet_account: Option<&accounts::TeslaAccount>,
) -> crate::connection::Report {
    refresh_partner(app).await;
    let rows = named_vins(&app.inner.db, account_id).await;
    let vins = rows.iter().map(|row| row.vin.clone()).collect::<Vec<_>>();
    if fleet_token.present {
        refresh_pairing(app, &vins).await;
    }
    let (partner, pairing) = {
        let cache = app.inner.links.lock().await;
        (cache.partner.clone(), cache.pairing.clone())
    };
    let archive = &app.inner.archive;
    let mut vehicles = Vec::new();
    for row in rows {
        let (skip_reason, config_pushed) = if let Some(target) = archive.telemetry.as_ref() {
            (
                target.skip_reason(&row.vin).await,
                target.is_configured(&row.vin).await,
            )
        } else {
            (None, false)
        };
        let activity: Option<String> =
            sqlx::query_scalar("SELECT activity FROM vehicle_state WHERE vehicle_id = $1")
                .bind(row.id)
                .fetch_optional(&app.inner.db)
                .await
                .ok()
                .flatten();
        let asleep = activity.as_deref() == Some("asleep");
        let online = matches!(
            activity.as_deref(),
            Some("online" | "driving" | "charging" | "updating")
        );
        let (health, health_tone) = if archive.telemetry.is_some() {
            let state = archive.vehicle_health(&row.vin, asleep, online).await;
            let (label, tone) = state.chip();
            (label.to_owned(), tone)
        } else {
            (String::new(), "")
        };
        vehicles.push(crate::connection::VehicleInput {
            id: row.id,
            name: row.name,
            key: pairing
                .get(&row.vin)
                .copied()
                .unwrap_or(crate::connection::KeyState::Unknown),
            skip_reason,
            config_pushed,
            last_message_age: archive.telemetry_age(&row.vin).await,
            health,
            health_tone,
        });
    }
    let secret_set = !tesla_auth::ExposeSecret::expose_secret(&app.inner.config.client_secret)
        .trim()
        .is_empty();
    crate::connection::build(&crate::connection::Input {
        now: Utc::now(),
        fleet_present: fleet_token.present,
        fleet_readable: fleet_token.readable,
        fleet_expires: fleet_token.expires_at,
        scopes: fleet_token.scopes.clone(),
        refresh_failed: fleet_account.is_some_and(|account| account.status == "error"),
        partner,
        public_key_published: app.inner.public_key_pem.contains("BEGIN PUBLIC KEY"),
        domain: app.inner.config.domain.clone(),
        redirect_uri: app.inner.config.redirect_uri.clone(),
        client_configured: !app.inner.config.client_id.trim().is_empty() && secret_set,
        client_id: app.inner.config.client_id.clone(),
        region: app.inner.config.region,
        fingerprint: public_key_fingerprint(&app.inner.public_key_pem),
        telemetry_enabled: app.inner.config.telemetry_enabled,
        telemetry_port: app.inner.config.telemetry_port,
        fresh_for: archive.telemetry_fresh_for,
        is_admin: page.admin,
        vehicles,
    })
}

#[derive(sqlx::FromRow)]
struct NamedVin {
    id: i64,
    name: String,
    vin: String,
}

async fn named_vins(pool: &sqlx::PgPool, account_id: i64) -> Vec<NamedVin> {
    sqlx::query_as(
        "SELECT id, COALESCE(NULLIF(name, ''), 'Véhicule') AS name, vin
         FROM vehicles WHERE account_id = $1 ORDER BY id",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}

async fn refresh_partner(app: &AppState) {
    if app
        .inner
        .links
        .lock()
        .await
        .partner_fresh(Duration::from_secs(300))
    {
        return;
    }
    let state = match tokio::time::timeout(
        Duration::from_secs(3),
        crate::partner::lookup(&app.inner.config),
    )
    .await
    {
        Ok(Ok(state)) => state,
        Ok(Err(err)) => {
            tracing::warn!(
                error = %crate::logs::redact_line(&err.to_string(), &[]),
                "partner domain could not be checked"
            );
            crate::connection::PartnerState::Unknown {
                message: "La vérification du domaine n'a pas abouti.".to_owned(),
            }
        }
        Err(_) => crate::connection::PartnerState::Unknown {
            message: "La vérification du domaine a expiré.".to_owned(),
        },
    };
    let mut cache = app.inner.links.lock().await;
    cache.partner = state;
    cache.partner_at = Some(Instant::now());
}

async fn refresh_pairing(app: &AppState, vins: &[String]) {
    if vins.is_empty()
        || app
            .inner
            .links
            .lock()
            .await
            .pairing_fresh(Duration::from_secs(60))
    {
        return;
    }
    let token = match app.inner.session.access_token().await {
        Ok(token) => token,
        Err(err) => {
            tracing::warn!(
                error = %crate::logs::redact_line(&err.to_string(), &[]),
                "fleet token was not available for virtual-key status"
            );
            return;
        }
    };
    let Ok(client) = tesla_fleet_api::FleetClient::for_region(
        app.inner.config.region,
        tesla_fleet_api::BearerToken::new(tesla_auth::ExposeSecret::expose_secret(&token)),
    ) else {
        return;
    };
    let Ok(Ok(status)) =
        tokio::time::timeout(Duration::from_secs(3), client.fleet_status(vins)).await
    else {
        tracing::warn!("virtual-key status could not be read");
        return;
    };
    let mut pairing = std::collections::BTreeMap::new();
    for vin in vins {
        let state = if status.key_paired_vins.iter().any(|item| item == vin) {
            crate::connection::KeyState::Paired
        } else if status.unpaired_vins.iter().any(|item| item == vin) {
            crate::connection::KeyState::Unpaired
        } else {
            crate::connection::KeyState::Unknown
        };
        pairing.insert(vin.clone(), state);
    }
    let mut cache = app.inner.links.lock().await;
    cache.pairing = pairing;
    cache.pairing_at = Some(Instant::now());
}

async fn register_partner(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    if !page.admin {
        return render_connection(
            &app,
            &headers,
            form.vehicle_id,
            Some("Un administrateur doit enregistrer le domaine."),
            None,
        )
        .await;
    }
    match crate::partner::register(&app.inner.config).await {
        Ok(()) => {
            let mut cache = app.inner.links.lock().await;
            cache.partner = crate::connection::PartnerState::Registered;
            cache.partner_at = Some(Instant::now());
            Redirect::to(&connection_location(form.vehicle_id)).into_response()
        }
        Err(err) => {
            tracing::warn!(
                error = %crate::logs::redact_line(&err.to_string(), &[]),
                "partner domain was not registered"
            );
            render_connection(
                &app,
                &headers,
                form.vehicle_id,
                Some("L'enregistrement du domaine a été refusé."),
                None,
            )
            .await
        }
    }
}

async fn resend_telemetry(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    if signed_page(&app, &headers).await.is_none() || !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    if !app.inner.config.telemetry_enabled {
        return render_connection(
            &app,
            &headers,
            form.vehicle_id,
            Some("La télémétrie n'est pas activée."),
            None,
        )
        .await;
    }
    let gate = crate::budget::gate(&app.inner.db, super::api::budget_query(&app, Utc::now())).await;
    if gate.blocks_vehicle_data() || gate.cap_reached() {
        return render_connection(
            &app,
            &headers,
            form.vehicle_id,
            Some("Le budget Fleet est en pause. La configuration sera renvoyée quand il reprend."),
            None,
        )
        .await;
    }
    let Some(target) = app.inner.archive.telemetry.as_ref() else {
        return render_connection(
            &app,
            &headers,
            form.vehicle_id,
            Some("Le certificat de télémétrie n'est pas chargé."),
            None,
        )
        .await;
    };
    target.force().await;
    let Ok(fleet) = crate::ingest::FleetPoll::connect(
        app.inner.config.region,
        std::sync::Arc::clone(&app.inner.session),
        app.inner.debug.clone(),
    )
    .map(|poll| poll.with_signing(crate::app::signing_from(&app.inner.config))) else {
        return render_connection(
            &app,
            &headers,
            form.vehicle_id,
            Some("Le client Fleet n'a pas pu être ouvert."),
            None,
        )
        .await;
    };
    let rows = named_vins(&app.inner.db, app.inner.fleet_account_id).await;
    let vehicles = rows
        .into_iter()
        .map(|row| crate::ingest::Summary {
            vin: row.vin,
            tesla_id: 0,
            legacy_vehicle_id: None,
            name: Some(row.name),
            presence: crate::state::Presence::Online,
        })
        .collect::<Vec<_>>();
    crate::ingest::subscribe_telemetry(&fleet, &app.inner.archive, &vehicles, false).await;
    if target.last_error().await.is_some() {
        return render_connection(
            &app,
            &headers,
            form.vehicle_id,
            Some("La configuration n'a pas été envoyée."),
            None,
        )
        .await;
    }
    Redirect::to(&connection_location(form.vehicle_id)).into_response()
}

#[derive(Deserialize)]
struct SetupQuery {
    #[serde(default)]
    step: String,
}

async fn security_setup(
    State(app): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<SetupQuery>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if principal.must_change_password {
        return html_page(
            setup_page(
                &page.csrf,
                None,
                SetupStep::Password {
                    require_current: true,
                },
            ),
            page.cookie,
        );
    }
    if principal.totp_enabled {
        return html_page(
            setup_page(&page.csrf, None, SetupStep::Finished),
            page.cookie,
        );
    }
    if query.step == "verify" {
        return html_page(setup_page(&page.csrf, None, SetupStep::Verify), page.cookie);
    }
    let Ok(secret) =
        crate::identity::ensure_pending_totp(&app, &page.session_id, principal.user_id, false)
            .await
    else {
        return html_page(setup_page(&page.csrf, None, SetupStep::Verify), page.cookie);
    };
    let uri = crate::identity::otpauth_uri(&page.username, &secret);
    let groups = crate::identity::grouped_secret(&secret);
    let qr = crate::qr::svg(&uri).unwrap_or_default();
    html_page(
        setup_page(
            &page.csrf,
            None,
            SetupStep::Authenticator {
                secret: &secret,
                groups: &groups,
                qr: &qr,
            },
        ),
        page.cookie,
    )
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
    if let Some(message) = password_form_error(&form) {
        crate::identity::audit(
            &app.inner.db,
            Some(principal.user_id),
            "password.change",
            "failure",
            r#"{"reason":"rejected"}"#,
        )
        .await;
        return setup_error(&app, &headers, message).await;
    }
    if !crate::identity::password_matches(&app.inner.db, principal.user_id, &form.current)
        .await
        .unwrap_or(false)
    {
        crate::identity::audit(
            &app.inner.db,
            Some(principal.user_id),
            "password.change",
            "failure",
            r#"{"reason":"current"}"#,
        )
        .await;
        return setup_error(&app, &headers, "Le mot de passe actuel est incorrect.").await;
    }
    if let Err(err) =
        crate::identity::change_password(&app.inner.db, principal.user_id, &form.password).await
    {
        return setup_error(&app, &headers, &err.to_string()).await;
    }
    Redirect::to("/security/setup").into_response()
}

fn password_form_error(form: &PasswordForm) -> Option<&'static str> {
    if form.password != form.confirm {
        return Some("Les mots de passe ne correspondent pas.");
    }
    if form.password.chars().count() < 12 {
        return Some("Le mot de passe doit contenir au moins 12 caractères.");
    }
    if form.current.trim().is_empty() {
        return Some("Le mot de passe actuel est incorrect.");
    }
    None
}

async fn setup_error(app: &AppState, headers: &HeaderMap, message: &str) -> Response {
    let Some(page) = signed_page(app, headers).await else {
        return Redirect::to("/").into_response();
    };
    let mut response = html_page(
        setup_page(
            &page.csrf,
            Some(message),
            SetupStep::Password {
                require_current: true,
            },
        ),
        page.cookie,
    );
    *response.status_mut() = StatusCode::BAD_REQUEST;
    response
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
    let _ = crate::identity::ensure_pending_totp(&app, &principal.raw_id, principal.user_id, true)
        .await;
    Redirect::to("/security/setup").into_response()
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
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    match crate::identity::confirm_totp(&app, &principal.raw_id, principal.user_id, &form.code)
        .await
    {
        Ok(crate::identity::TotpConfirm::Accepted(codes)) => html_page(
            setup_page(&page.csrf, None, SetupStep::Recovery { codes: &codes }),
            page.cookie,
        ),
        Ok(crate::identity::TotpConfirm::Rejected) => {
            let mut response = html_page(
                setup_page(
                    &page.csrf,
                    Some("Code incorrect. Réessayez avec le même secret."),
                    SetupStep::Verify,
                ),
                page.cookie,
            );
            *response.status_mut() = StatusCode::BAD_REQUEST;
            response
        }
        _ => {
            let mut response = html_page(
                setup_page(
                    &page.csrf,
                    Some("Le secret a expiré. Scannez un nouveau code."),
                    SetupStep::Verify,
                ),
                page.cookie,
            );
            *response.status_mut() = StatusCode::BAD_REQUEST;
            response
        }
    }
}

#[derive(Deserialize)]
struct FinishForm {
    csrf: String,
    #[serde(default)]
    saved: String,
}

async fn finish_setup(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<FinishForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    if form.saved == "yes" {
        crate::identity::audit(
            &app.inner.db,
            Some(principal.user_id),
            "2fa.recovery_ack",
            "ok",
            "{}",
        )
        .await;
        Redirect::to("/").into_response()
    } else {
        let Some(page) = signed_page(&app, &headers).await else {
            return Redirect::to("/").into_response();
        };
        let mut response = html_page(
            setup_page(
                &page.csrf,
                Some("Cochez la case pour confirmer que vous avez sauvegardé les codes."),
                SetupStep::Finished,
            ),
            page.cookie,
        );
        *response.status_mut() = StatusCode::BAD_REQUEST;
        response
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
    confirm: String,
    #[serde(default)]
    current: String,
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
    let snap = app.inner.runtime.snapshot();
    let rows = app
        .inner
        .config
        .public_settings(page.admin)
        .into_iter()
        .map(|row| {
            let key = setting_key_for_label(row.label).unwrap_or("");
            let effective = if key.is_empty() {
                crate::settings::EffectiveSetting {
                    key: String::new(),
                    label: row.label,
                    value: row.value,
                    source: row.source,
                }
            } else {
                crate::settings::effective_row(&app.inner.config, &snap, key, row.label, row.value)
            };
            SettingRow {
                label: effective.label,
                value: effective.value,
                source: effective.source,
            }
        })
        .collect::<Vec<_>>();
    let override_form = page.admin.then(|| settings_override_form(&page.csrf));
    let body = settings_page(
        &chrome(&page, None, None, GlobalTab::Settings, false),
        &rows,
        override_form.as_deref(),
    );
    html_page(body, page.cookie)
}

fn setting_key_for_label(label: &str) -> Option<&'static str> {
    match label {
        "Poll conduite repli (60–3600 s)" => Some("POLL_DRIVE_FALLBACK_SECS"),
        "Délai fin de trajet (s)" => Some("DRIVE_END_GRACE_SECS"),
        "Délai à 0 km/h (s)" => Some("DRIVE_PARK_GRACE_SECS"),
        "Confirmation parking (s)" => Some("DRIVE_P_CONFIRM_SECS"),
        "Arrêt en prise (s)" => Some("DRIVE_STOP_IN_GEAR_SECS"),
        "Poll charge (60–86400 s)" => Some("POLL_CHARGE_SECS"),
        "Poll à l'arrêt (s)" => Some("POLL_PARKED_SECS"),
        "Poll endormi (s)" => Some("POLL_ASLEEP_SECS"),
        "Poll repli (s)" => Some("POLL_BACKOFF_SECS"),
        "Lectures à l'arrêt" => Some("POLL_PARKED_LIVE_LIMIT"),
        "Repli GPS (s)" => Some("POLL_GPS_FALLBACK_SECS"),
        "Plafond vehicle_data / jour (0–100000)" => Some("FLEET_DATA_DAILY_CAP"),
        "Plafond vehicle_data / mois (0–1000000)" => Some("FLEET_DATA_MONTHLY_CAP"),
        "Sonde blocage Tesla (300–86400 s)" => Some("FLEET_BLOCKED_PROBE_SECS"),
        "Fuseau" => Some("TESLA_TIMEZONE"),
        "Prix du kWh" => Some("ENERGY_PRICE_PER_KWH"),
        "Nominatim" => Some("NOMINATIM_ENABLED"),
        "Nominatim URL" => Some("NOMINATIM_URL"),
        "Nominatim intervalle (ms)" => Some("NOMINATIM_MIN_INTERVAL_MS"),
        "MQTT" => Some("MQTT_ENABLED"),
        "MQTT hôte" => Some("MQTT_HOST"),
        "MQTT port" => Some("MQTT_PORT"),
        "MQTT préfixe" => Some("MQTT_TOPIC_PREFIX"),
        _ => None,
    }
}

fn settings_override_form(csrf: &str) -> String {
    maud::html! {
        section class="card" {
            h2 { "Modifier un réglage" }
            p class="muted" {
                "La valeur est appliquée tout de suite sans redémarrage. Un champ vide revient à l'environnement. Les secrets restent dans l'environnement."
            }
            form class="stack" method="post" action="/settings/override" {
                input type="hidden" name="csrf" value=(csrf);
                label class="field" for="setting-key" {
                    "Réglage"
                    select id="setting-key" name="key" {
                        @for group in [
                            crate::settings::SettingGroup::Polling,
                            crate::settings::SettingGroup::Fleet,
                            crate::settings::SettingGroup::Nominatim,
                            crate::settings::SettingGroup::Mqtt,
                            crate::settings::SettingGroup::Misc,
                        ] {
                            optgroup label=(group.label()) {
                                @for entry in crate::settings::REGISTRY.iter().filter(|entry| entry.group == group) {
                                    option value=(entry.key) { (entry.label) " — " (entry.key) }
                                }
                            }
                        }
                    }
                }
                label class="field" for="setting-value" { "Valeur" input id="setting-value" name="value"; }
                label class="field" for="setting-vehicle" {
                    "Véhicule (id), vide pour tous"
                    input id="setting-vehicle" name="vehicle_id";
                }
                button type="submit" { "Enregistrer" }
            }
        }
    }
    .into_string()
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
    if !csrf_ok(&headers, &form.csrf) || !principal.is_admin() {
        return (StatusCode::FORBIDDEN, "admin required").into_response();
    }
    let vehicle = form.vehicle_id.trim().parse::<i64>().ok();
    if let Err(err) = crate::settings::set_and_publish(
        &app.inner.db,
        Some(&app.inner.runtime),
        vehicle,
        &form.key,
        &form.value,
    )
    .await
    {
        return (StatusCode::BAD_REQUEST, err.to_string()).into_response();
    }
    crate::identity::audit(
        &app.inner.db,
        Some(principal.user_id),
        "settings.change",
        "ok",
        &serde_json::json!({ "key": form.key }).to_string(),
    )
    .await;
    Redirect::to("/settings").into_response()
}

async fn system(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    let status = system_status(&app, &page.csrf, page.admin).await;
    let body = system_page(
        &chrome(&page, None, None, GlobalTab::System, false),
        &status,
    );
    html_page(body, page.cookie)
}

#[allow(clippy::format_push_string)]
async fn system_status(app: &AppState, csrf: &str, admin: bool) -> String {
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
    let gate = crate::budget::gate(
        &app.inner.db,
        super::api::budget_query(app, chrono::Utc::now()),
    )
    .await;
    app.inner.debug.set_budget_banner(gate.debug_line());
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
    text.push_str("<h2>Fleet API</h2>");
    if let Some(kind) = gate.pause {
        text.push_str(&format!(
            "<p class=\"banner bad\" role=\"alert\">{}</p>",
            escape_html(kind.system_banner())
        ));
    }
    let today = super::format::number(crate::budget::as_display_f64(gate.today_calls), 0);
    let month = super::format::number(crate::budget::as_display_f64(gate.month_calls), 0);
    let today_cost = super::format::number(gate.today_cost_usd(), 3);
    let month_cost = super::format::number(gate.month_cost_usd(), 3);
    let daily_label = super::format::number(crate::budget::as_display_f64(gate.daily_cap), 0);
    let monthly_label = super::format::number(crate::budget::as_display_f64(gate.monthly_cap), 0);
    text.push_str(&format!(
        "<p>Aujourd'hui {today} appels ({today_cost} USD). Mois {month} appels ({month_cost} USD). Plafonds {daily_label} / jour et {monthly_label} / mois.</p>"
    ));
    text.push_str(
        "<p class=\"muted\">Estimation : 1 USD pour 500 appels vehicle_data, avant le crédit mensuel de 10 USD. Les réponses 408 comptent.</p>",
    );
    if let Some(block) = gate.block {
        let tz = app.inner.config.timezone.parse().unwrap_or(chrono_tz::UTC);
        let since = escape_html(&super::format::moment(block.since, tz));
        let next = escape_html(&super::format::moment(block.next_probe, tz));
        text.push_str(&format!(
            "<p>Bloqué par Tesla depuis {since}, prochain test à {next}.</p>"
        ));
    }
    if admin && gate.pause == Some(crate::budget::PauseKind::Billing) {
        let csrf = escape_html(csrf);
        text.push_str(&format!(
            "<form method=\"post\" action=\"/system/fleet/resume\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\"><button type=\"submit\">Reprendre vehicle_data</button></form>"
        ));
    }
    text.push_str("<h2>Comptes Tesla</h2><ul class=\"list\">");
    for (id, label, status) in &accounts {
        let label = escape_html(label);
        let status = escape_html(status);
        text.push_str(&format!("<li><span>{label} · {status}</span>"));
        if admin {
            let csrf = escape_html(csrf);
            text.push_str(&format!(
                "<form method=\"post\" action=\"/accounts/{id}/disconnect\" class=\"inline\">\
                 <input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
                 <button class=\"danger\" type=\"submit\">Déconnecter</button></form>"
            ));
        }
        text.push_str("</li>");
    }
    text.push_str("</ul>");
    if admin {
        let csrf = escape_html(csrf);
        text.push_str(&format!(
            "<form class=\"stack\" method=\"post\" action=\"/accounts\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
             <label class=\"field\">Libellé <input name=\"label\" required maxlength=\"80\"></label>\
             <button type=\"submit\">Ajouter un compte Fleet</button>\
             <p class=\"muted\">Nécessite sudo actif.</p></form>"
        ));
    }
    text.push_str("<h2>Curseurs d'import</h2><ul class=\"list\">");
    for (source, cursor) in cursors {
        let source = escape_html(&source);
        text.push_str(&format!("<li><span>{source} · {cursor}</span></li>"));
    }
    text.push_str(
        "</ul><p>Sauvegarde : <a href=\"/system/backup\">/system/backup</a> (admin et sudo).</p></section>",
    );
    text
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

async fn resume_fleet(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) || !principal.is_admin() {
        return (StatusCode::FORBIDDEN, "admin required").into_response();
    }
    if let Err(err) = crate::budget::resume_billing(&app.inner.db).await {
        tracing::warn!(error = %err, "fleet billing pause was not cleared");
        return (StatusCode::BAD_GATEWAY, "resume failed").into_response();
    }
    let gate = crate::budget::gate(
        &app.inner.db,
        super::api::budget_query(&app, chrono::Utc::now()),
    )
    .await;
    app.inner.debug.set_budget_banner(gate.debug_line());
    Redirect::to("/system").into_response()
}

async fn backup(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !principal.is_admin() || !principal.sudo_active() {
        return (StatusCode::FORBIDDEN, "sudo required").into_response();
    }
    let Ok(mut command) = crate::system::pg_dump_command(&app.inner.config.database_url) else {
        return (StatusCode::BAD_GATEWAY, "pg_dump failed").into_response();
    };
    let output = command.output();
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
    Redirect::to("/system").into_response()
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
    Redirect::to("/system").into_response()
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

const OAUTH_PENDING_TTL: Duration = Duration::from_secs(10 * 60);
const OAUTH_PENDING_LIMIT: usize = 32;
const OAUTH_STATE_MAX: usize = 256;
const OAUTH_CODE_MAX: usize = 2_048;

async fn oauth_start(State(app): State<AppState>, headers: HeaderMap) -> Response {
    let Some(principal) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
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
        Err(err) => {
            tracing::warn!(
                error = %crate::logs::redact_line(&err.to_string(), &[]),
                "tesla login could not start"
            );
            return oauth_text(StatusCode::BAD_REQUEST, "Tesla sign-in could not start");
        }
    };
    let url = pending.url().to_string();
    let state = pending.state().to_owned();
    let mut guard = app.inner.pending.lock().await;
    purge_expired(&mut guard);
    if guard.len() >= OAUTH_PENDING_LIMIT {
        return oauth_text(StatusCode::SERVICE_UNAVAILABLE, "too many sign-in attempts");
    }
    guard.insert(
        state,
        PendingLogin {
            authorization: pending,
            session_hash: sha256_hex(&principal.raw_id),
            created: Instant::now(),
        },
    );
    drop(guard);
    Redirect::to(&url).into_response()
}

/// Cross-site return from `auth.tesla.com`. The browser does not send the
/// `SameSite=Strict` session cookie on this navigation, so the login is finished
/// by a same-origin hop that does.
async fn oauth_callback(request: axum::extract::Request) -> Response {
    let query = request.uri().query().unwrap_or("");
    let Some(target) = hop_target(query) else {
        return oauth_text(StatusCode::BAD_REQUEST, "missing oauth state");
    };
    let target = escape_attr(&target);
    let html = format!(
        "<!DOCTYPE html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\"><meta http-equiv=\"refresh\" content=\"0; url={target}\"><title>Continue</title></head><body><p><a href=\"{target}\">Continue</a></p></body></html>"
    );
    let mut response = (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        )],
        html,
    )
        .into_response();
    stamp_oauth(&mut response);
    response
}

async fn oauth_complete(State(app): State<AppState>, request: axum::extract::Request) -> Response {
    let headers = request.headers().clone();
    let Some(principal) = ui_session(&app, &headers).await else {
        return oauth_text(StatusCode::UNAUTHORIZED, "sign in to the logger first");
    };
    let query = request.uri().query().unwrap_or("");
    let params = CallbackParams::from_query(query);
    let Some(state) = params.state.clone() else {
        return oauth_text(StatusCode::BAD_REQUEST, "missing oauth state");
    };
    if state.is_empty() || state.len() > OAUTH_STATE_MAX {
        return oauth_text(StatusCode::BAD_REQUEST, "missing oauth state");
    }
    let session_hash = sha256_hex(&principal.raw_id);
    let pending = {
        let mut guard = app.inner.pending.lock().await;
        let Some(entry) = guard.get(&state) else {
            return oauth_text(StatusCode::BAD_REQUEST, "unknown oauth state");
        };
        if !crate::crypto::constant_time_eq(entry.session_hash.as_bytes(), session_hash.as_bytes())
        {
            return oauth_text(StatusCode::FORBIDDEN, "sign-in does not match this session");
        }
        if Instant::now().saturating_duration_since(entry.created) > OAUTH_PENDING_TTL {
            guard.remove(&state);
            return oauth_text(StatusCode::BAD_REQUEST, "oauth state expired");
        }
        guard.remove(&state)
    };
    let Some(pending) = pending else {
        return oauth_text(StatusCode::BAD_REQUEST, "unknown oauth state");
    };
    finish_login(&app, pending, params, &state).await
}

async fn finish_login(
    app: &AppState,
    pending: PendingLogin,
    params: CallbackParams,
    state: &str,
) -> Response {
    let mut code = params
        .code()
        .map(|value| value.expose_secret().to_owned())
        .unwrap_or_default();
    let result = app
        .inner
        .session
        .complete_login(&pending.authorization, params)
        .await;
    let response = match result {
        Ok(_) => {
            let mut response = Redirect::to("/connection").into_response();
            stamp_oauth(&mut response);
            response
        }
        Err(err) => {
            let mut secrets = Vec::new();
            if code.len() >= 4 {
                secrets.push(code.clone());
            }
            if state.len() >= 4 {
                secrets.push(state.to_owned());
            }
            let rendered = crate::logs::redact_line(&err.to_string(), &secrets);
            for secret in &mut secrets {
                secret.zeroize();
            }
            tracing::warn!(error = %rendered, "tesla login failed");
            oauth_text(StatusCode::BAD_GATEWAY, "Tesla sign-in failed")
        }
    };
    code.zeroize();
    response
}

fn hop_target(query: &str) -> Option<String> {
    let params = CallbackParams::from_query(query);
    let state = params.state.as_deref()?;
    if !oauth_token_ok(state, OAUTH_STATE_MAX) {
        return None;
    }
    if let Some(code) = params.code()
        && !oauth_token_ok(code.expose_secret(), OAUTH_CODE_MAX)
    {
        return None;
    }
    let mut serializer =
        url::form_urlencoded::Serializer::new(String::from("/auth/callback/complete?"));
    if let Some(code) = params.code() {
        serializer.append_pair("code", code.expose_secret());
    }
    serializer.append_pair("state", state);
    if let Some(error) = params.error.as_deref()
        && oauth_token_ok(error, 128)
    {
        serializer.append_pair("error", error);
    }
    if let Some(description) = params.error_description.as_deref() {
        let description: String = description.chars().take(200).collect();
        if !description.is_empty() && !description.chars().any(|ch| ch.is_control()) {
            serializer.append_pair("error_description", &description);
        }
    }
    Some(serializer.finish())
}

fn oauth_token_ok(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}

fn purge_expired(pending: &mut HashMap<String, PendingLogin>) {
    let now = Instant::now();
    pending.retain(|_, entry| now.saturating_duration_since(entry.created) <= OAUTH_PENDING_TTL);
}

fn stamp_oauth(response: &mut Response) {
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
}

fn oauth_text(status: StatusCode, message: &'static str) -> Response {
    let mut response = (status, message).into_response();
    stamp_oauth(&mut response);
    response
}

fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

async fn disconnect(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    if ui_session(&app, &headers).await.is_none() || !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    if form.confirm != "disconnect" {
        return render_connection(
            &app,
            &headers,
            form.vehicle_id,
            Some(
                "Confirmez la déconnexion pour effacer les jetons. Les données des véhicules sont conservées.",
            ),
            None,
        )
        .await;
    }
    let rows = named_vins(&app.inner.db, app.inner.fleet_account_id).await;
    stop_telemetry(&app, &rows).await;
    if let Some(target) = app.inner.archive.telemetry.as_ref() {
        let vins = rows.iter().map(|row| row.vin.clone()).collect::<Vec<_>>();
        target.forget(&vins).await;
    }
    let fleet_cleared = match app.inner.fleet_store.clear().await {
        Ok(()) => true,
        Err(err) => {
            tracing::warn!(
                error = %crate::logs::redact_line(&err.to_string(), &[]),
                "could not clear fleet tokens"
            );
            false
        }
    };
    app.inner.archive.gps.notify();
    {
        let mut cache = app.inner.links.lock().await;
        cache.pairing.clear();
        cache.pairing_at = None;
    }
    if !fleet_cleared {
        return render_connection(
            &app,
            &headers,
            form.vehicle_id,
            Some(
                "Les jetons n'ont pas tous été effacés. Les données des véhicules sont toujours là.",
            ),
            None,
        )
        .await;
    }
    Redirect::to(&connection_location(form.vehicle_id)).into_response()
}

async fn stop_telemetry(app: &AppState, rows: &[NamedVin]) {
    if rows.is_empty() {
        return;
    }
    let Ok(token) = app.inner.session.access_token().await else {
        return;
    };
    let exposed = tesla_auth::ExposeSecret::expose_secret(&token);
    let Ok(client) = tesla_fleet_api::FleetClient::for_region(
        app.inner.config.region,
        tesla_fleet_api::BearerToken::new(exposed),
    ) else {
        return;
    };
    for row in rows {
        let Ok(tag) = tesla_fleet_api::VehicleTag::new(&row.vin) else {
            continue;
        };
        if let Err(err) = client.delete_fleet_telemetry_config(&tag).await
            && !matches!(err, tesla_fleet_api::FleetError::NotFound { .. })
        {
            tracing::warn!(
                error = %crate::logs::redact_line(&err.to_string(), &[]),
                "telemetry config was not removed"
            );
        }
    }
}

#[derive(Deserialize)]
struct DeleteForm {
    csrf: String,
    #[serde(default)]
    confirm: String,
}

async fn delete_vehicle(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Form(form): Form<DeleteForm>,
) -> Response {
    let Some(page) = signed_page(&app, &headers).await else {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    if !page.admin || !page.sudo {
        return (StatusCode::FORBIDDEN, "sudo required").into_response();
    }
    if id <= 0 || !page.vehicles.iter().any(|vehicle| vehicle.id == id) {
        return html_page(
            missing_page(&chrome(&page, None, None, GlobalTab::None, false)),
            page.cookie,
        );
    }
    let typed = form.confirm.trim();
    let stored: Option<(String,)> =
        sqlx::query_as("SELECT COALESCE(NULLIF(name, ''), 'Véhicule') FROM vehicles WHERE id = $1")
            .bind(id)
            .fetch_optional(&app.inner.db)
            .await
            .ok()
            .flatten();
    let Some((name,)) = stored else {
        return html_page(
            missing_page(&chrome(&page, None, None, GlobalTab::None, false)),
            page.cookie,
        );
    };
    if typed.is_empty() || typed.len() > 120 || name.trim() != typed {
        return render_connection(
            &app,
            &headers,
            Some(id),
            Some("Le nom ne correspond pas. Aucune donnée n'a été supprimée."),
            None,
        )
        .await;
    }
    match crate::persist::delete_vehicle_data(&app.inner.db, id).await {
        Ok(true) => Redirect::to("/connection").into_response(),
        Ok(false) => html_page(
            missing_page(&chrome(&page, None, None, GlobalTab::None, false)),
            page.cookie,
        ),
        Err(err) => {
            tracing::warn!(
                error = %crate::logs::redact_line(&err.to_string(), &[]),
                "vehicle data was not deleted"
            );
            render_connection(
                &app,
                &headers,
                Some(id),
                Some("La suppression a échoué. Les autres véhicules n'ont pas été touchés."),
                None,
            )
            .await
        }
    }
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
    let Some(session) = ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf) {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    if !session.is_admin() || !session.sudo_active() {
        return (StatusCode::FORBIDDEN, "sudo required").into_response();
    }
    let _ = sqlx::query(
        "UPDATE api_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(id)
    .execute(&app.inner.db)
    .await;
    crate::identity::audit(
        &app.inner.db,
        Some(session.user_id),
        "token.revoke",
        "ok",
        &serde_json::json!({ "id": id }).to_string(),
    )
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
    cookie(headers, "tl_csrf").is_some_and(|cookie| {
        !form.is_empty() && crate::crypto::constant_time_eq(cookie.as_bytes(), form.as_bytes())
    })
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
    let sudo = principal.sudo_active();
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
        sudo,
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
    super::format::percent(value)
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

async fn setup_js() -> Response {
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
        include_str!("setup.js"),
    )
        .into_response()
}

async fn charts_js() -> Response {
    script(include_str!("charts.js"))
}

async fn map_js() -> Response {
    script(include_str!("map.js"))
}

async fn maplibre_js() -> Response {
    script(include_str!("vendor/maplibre-gl-csp.js"))
}

async fn maplibre_worker() -> Response {
    script(include_str!("vendor/maplibre-gl-csp-worker.js"))
}

async fn maplibre_css() -> Response {
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
        include_str!("vendor/maplibre-gl.css"),
    )
        .into_response()
}

fn script(body: &'static str) -> Response {
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
        body,
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

async fn ui_js() -> Response {
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
        include_str!("../../../ui/dist/tesla-logger-ui.js"),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    use super::super::router;
    use crate::app::{AppState, OauthDouble, test_state, test_state_oauth};

    async fn body_string(response: axum::response::Response) -> String {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn set_cookie(headers: &axum::http::HeaderMap, name: &str) -> String {
        headers
            .get_all(header::SET_COOKIE)
            .iter()
            .find_map(|value| {
                let text = value.to_str().ok()?;
                let pair = text.split(';').next()?;
                let (key, value) = pair.split_once('=')?;
                (key.trim() == name).then(|| value.trim().to_owned())
            })
            .unwrap_or_else(|| panic!("missing {name}"))
    }

    async fn lazy_app() -> AppState {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://localhost/unused")
            .unwrap();
        test_state(pool).await
    }

    async fn get(app: &AppState, uri: &str, session: Option<&str>) -> axum::response::Response {
        let mut builder = Request::builder().uri(uri);
        if let Some(session) = session {
            builder = builder.header(header::COOKIE, format!("__Host-tl_session={session}"));
        }
        router(app.clone())
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    fn assert_unlogged(response: &axum::response::Response) {
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        assert_eq!(
            response.headers().get(header::REFERRER_POLICY).unwrap(),
            "no-referrer"
        );
    }

    async fn ready_session(app: &AppState, username: &str) -> String {
        if username != "admin" {
            sqlx::query(
                "INSERT INTO users (username, password_hash, role, must_change_password, totp_enabled)
                 SELECT $1, password_hash, 'user', false, false FROM users WHERE username = 'admin'",
            )
            .bind(username)
            .execute(&app.inner.db)
            .await
            .unwrap();
        }
        let login = get(app, "/", None).await;
        let csrf = set_cookie(login.headers(), "tl_csrf");
        let form = if username == "admin" {
            format!("password=test-password&csrf={csrf}")
        } else {
            format!("username={username}&password=test-password&csrf={csrf}")
        };
        let posted = router(app.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/")
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header(header::COOKIE, format!("tl_csrf={csrf}"))
                    .body(Body::from(form))
                    .unwrap(),
            )
            .await
            .unwrap();
        let posted_status = posted.status();
        let session_header = posted
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let session = set_cookie(posted.headers(), "__Host-tl_session");
        assert!(
            posted_status.is_redirection(),
            "{}",
            body_string(posted).await
        );
        let session_header = session_header.expect("session cookie");
        assert!(
            session_header.contains("SameSite=Strict"),
            "{session_header}"
        );
        sqlx::query(
            "UPDATE users SET must_change_password = false, totp_enabled = true WHERE username = $1",
        )
        .bind(username)
        .execute(&app.inner.db)
        .await
        .unwrap();
        session
    }

    async fn start_oauth(app: &AppState, session: &str) -> String {
        let response = get(app, "/auth/login", Some(session)).await;
        let status = response.status();
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        assert!(status.is_redirection(), "{}", body_string(response).await);
        let location = location.expect("location");
        assert!(
            location.starts_with("https://auth.tesla.com/"),
            "{location}"
        );
        url::Url::parse(&location)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "state")
            .map(|(_, value)| value.into_owned())
            .unwrap()
    }

    async fn oauth_app(
        prefix: &str,
    ) -> Option<(crate::test_util::TestDb, AppState, wiremock::MockServer)> {
        let db = crate::test_util::TestDb::migrated(prefix).await?;
        let server = wiremock::MockServer::start().await;
        let account_id: i64 = sqlx::query_scalar(
            "INSERT INTO tesla_accounts (label, kind) VALUES ('fleet', 'fleet') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let token_endpoint = format!("{}/oauth2/v3/token", server.uri()).parse().unwrap();
        let app = test_state_oauth(
            db.pool.clone(),
            OauthDouble {
                token_endpoint,
                fleet_account_id: account_id,
            },
        )
        .await;
        Some((db, app, server))
    }

    #[tokio::test]
    async fn callback_without_cookie_returns_same_origin_hop() {
        let app = lazy_app().await;
        let state = "state-value";
        let response = get(
            &app,
            &format!("/auth/callback?code=code%3Cscript&state={state}&next=https://evil.example"),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_unlogged(&response);
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(content_type.starts_with("text/html"), "{content_type}");
        let body = body_string(response).await;
        assert!(body.contains("http-equiv=\"refresh\""), "{body}");
        assert!(body.contains("/auth/callback/complete?"), "{body}");
        assert!(body.contains("state=state-value"), "{body}");
        assert!(
            body.contains("%3Cscript") || body.contains("%3cscript"),
            "{body}"
        );
        assert!(!body.contains("<script"), "{body}");
        assert!(!body.contains("https://evil.example"), "{body}");
        assert!(!body.contains("sign in to the logger first"), "{body}");
    }

    #[tokio::test]
    async fn completion_without_session_is_rejected_and_does_not_echo_secrets() {
        let app = lazy_app().await;
        let code = "super-secret-code";
        let state = "super-secret-state";
        let response = get(
            &app,
            &format!("/auth/callback/complete?code={code}&state={state}"),
            None,
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_unlogged(&response);
        let body = body_string(response).await;
        assert!(!body.contains(code), "{body}");
        assert!(!body.contains(state), "{body}");
    }

    #[tokio::test]
    async fn completion_with_unknown_state_is_rejected() {
        let Some(db) = crate::test_util::TestDb::migrated("oauth-unknown").await else {
            return;
        };
        let app = test_state(db.pool.clone()).await;
        let session = ready_session(&app, "admin").await;
        let code = "unused-code";
        let state = "missing-state-value";
        let response = get(
            &app,
            &format!("/auth/callback/complete?code={code}&state={state}"),
            Some(&session),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_unlogged(&response);
        let body = body_string(response).await;
        assert!(body.contains("unknown"), "{body}");
        assert!(!body.contains(code), "{body}");
        assert!(!body.contains(state), "{body}");
    }

    #[tokio::test]
    async fn completion_with_a_different_session_is_rejected() {
        let Some((_db, app, server)) = oauth_app("oauth-other").await else {
            return;
        };
        let owner = ready_session(&app, "admin").await;
        let other = ready_session(&app, "other").await;
        let state = start_oauth(&app, &owner).await;
        let code = "auth-code-value";
        let response = get(
            &app,
            &format!("/auth/callback/complete?code={code}&state={state}"),
            Some(&other),
        )
        .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_unlogged(&response);
        let body = body_string(response).await;
        assert!(!body.contains(code), "{body}");
        assert!(!body.contains(&state), "{body}");
        assert!(app.inner.pending.lock().await.contains_key(&state));
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn completion_happy_path_redirects_to_connection_once() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};

        let Some((_db, app, server)) = oauth_app("oauth-happy").await else {
            return;
        };
        Mock::given(method("POST"))
            .and(path("/oauth2/v3/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "access-secret",
                "refresh_token": "refresh-secret",
                "token_type": "Bearer",
                "expires_in": 3600
            })))
            .mount(&server)
            .await;
        let session = ready_session(&app, "admin").await;
        let state = start_oauth(&app, &session).await;
        let code = "auth-code-value";
        let hop = get(
            &app,
            &format!("/auth/callback?code={code}&state={state}"),
            None,
        )
        .await;
        assert_eq!(hop.status(), StatusCode::OK);
        assert_unlogged(&hop);
        let hop_body = body_string(hop).await;
        assert!(hop_body.contains("/auth/callback/complete?"), "{hop_body}");
        assert!(app.inner.pending.lock().await.contains_key(&state));
        assert!(server.received_requests().await.unwrap().is_empty());

        let done = get(
            &app,
            &format!("/auth/callback/complete?code={code}&state={state}"),
            Some(&session),
        )
        .await;
        let done_status = done.status();
        let location = done
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        assert_unlogged(&done);
        assert!(done_status.is_redirection(), "{}", body_string(done).await);
        assert_eq!(location.as_deref(), Some("/connection"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        let stored: String =
            sqlx::query_scalar("SELECT status FROM tesla_accounts WHERE kind = 'fleet'")
                .fetch_one(&app.inner.db)
                .await
                .unwrap();
        assert_eq!(stored, "connected");

        let reused = get(
            &app,
            &format!("/auth/callback/complete?code={code}&state={state}"),
            Some(&session),
        )
        .await;
        assert_eq!(reused.status(), StatusCode::BAD_REQUEST);
        assert_unlogged(&reused);
        let body = body_string(reused).await;
        assert!(body.contains("unknown"), "{body}");
        assert!(!body.contains(code), "{body}");
        assert!(!body.contains(&state), "{body}");
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn completion_with_expired_state_is_rejected() {
        let Some((_db, app, server)) = oauth_app("oauth-expired").await else {
            return;
        };
        let session = ready_session(&app, "admin").await;
        let state = start_oauth(&app, &session).await;
        {
            let mut pending = app.inner.pending.lock().await;
            let entry = pending.get_mut(&state).expect("pending login");
            entry.created = Instant::now()
                .checked_sub(super::OAUTH_PENDING_TTL + Duration::from_secs(1))
                .expect("ttl");
        }
        let code = "auth-code-value";
        let response = get(
            &app,
            &format!("/auth/callback/complete?code={code}&state={state}"),
            Some(&session),
        )
        .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_unlogged(&response);
        let body = body_string(response).await;
        assert!(body.contains("expired"), "{body}");
        assert!(!body.contains(code), "{body}");
        assert!(!body.contains(&state), "{body}");
        assert!(server.received_requests().await.unwrap().is_empty());
        assert!(!app.inner.pending.lock().await.contains_key(&state));
    }

    #[tokio::test]
    async fn pending_oauth_logins_are_bounded() {
        let Some(db) = crate::test_util::TestDb::migrated("oauth-bound").await else {
            return;
        };
        let app = test_state(db.pool.clone()).await;
        let session = ready_session(&app, "admin").await;
        let mut blocked_at = None;
        for n in 1..=40 {
            let response = get(&app, "/auth/login", Some(&session)).await;
            if response.status().is_redirection() {
                continue;
            }
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            let body = body_string(response).await;
            assert!(!body.contains("client_secret"), "{body}");
            blocked_at = Some(n);
            break;
        }
        assert_eq!(
            blocked_at,
            Some(33),
            "32 pending logins fit; the next is refused"
        );
        let expired = {
            let mut pending = app.inner.pending.lock().await;
            let state = pending.keys().next().expect("pending").clone();
            let entry = pending.get_mut(&state).expect("entry");
            entry.created = Instant::now()
                .checked_sub(super::OAUTH_PENDING_TTL + Duration::from_secs(1))
                .expect("ttl");
            state
        };
        let response = get(&app, "/auth/login", Some(&session)).await;
        assert!(response.status().is_redirection());
        assert!(!app.inner.pending.lock().await.contains_key(&expired));
    }

    async fn post_form(
        app: &AppState,
        uri: &str,
        cookie: &str,
        body: &str,
    ) -> axum::response::Response {
        router(app.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header(header::COOKIE, cookie)
                    .body(Body::from(body.to_owned()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn delete_requires_csrf_and_sudo_and_disconnect_keeps_the_other_car() {
        let Some(db) = crate::test_util::TestDb::migrated("conn-guard").await else {
            return;
        };
        let app = test_state(db.pool.clone()).await;
        let login = get(&app, "/", None).await;
        let csrf = set_cookie(login.headers(), "tl_csrf");
        let posted = post_form(
            &app,
            "/",
            &format!("tl_csrf={csrf}"),
            &format!("password=test-password&csrf={csrf}"),
        )
        .await;
        let session = set_cookie(posted.headers(), "__Host-tl_session");
        sqlx::query("UPDATE users SET must_change_password = false, totp_enabled = true")
            .execute(&db.pool)
            .await
            .unwrap();
        let cookie = format!("__Host-tl_session={session}; tl_csrf={csrf}");
        for (vin, name) in [
            ("5YJ30000000000D01", "Faucon"),
            ("5YJ30000000000D02", "Millenium"),
        ] {
            let id: (i64,) =
                sqlx::query_as("INSERT INTO vehicles (vin, name) VALUES ($1, $2) RETURNING id")
                    .bind(vin)
                    .bind(name)
                    .fetch_one(&db.pool)
                    .await
                    .unwrap();
            sqlx::query("INSERT INTO drives (vehicle_id, start_at) VALUES ($1, now())")
                .bind(id.0)
                .execute(&db.pool)
                .await
                .unwrap();
        }
        let first: (i64,) = sqlx::query_as("SELECT id FROM vehicles WHERE name = 'Faucon'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let denied = post_form(
            &app,
            &format!("/v/{}/delete", first.0),
            &cookie,
            "csrf=not-the-cookie&confirm=Faucon",
        )
        .await;
        assert_eq!(denied.status(), StatusCode::BAD_REQUEST);
        let forbidden = post_form(
            &app,
            &format!("/v/{}/delete", first.0),
            &cookie,
            &format!("csrf={csrf}&confirm=Faucon"),
        )
        .await;
        assert_eq!(forbidden.status(), StatusCode::FORBIDDEN);
        let count: (i64,) = sqlx::query_as("SELECT count(*) FROM drives")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(count.0, 2);

        sqlx::query("UPDATE sessions SET sudo_until = now() + interval '5 minutes'")
            .execute(&db.pool)
            .await
            .unwrap();
        let mismatch = post_form(
            &app,
            &format!("/v/{}/delete", first.0),
            &cookie,
            &format!("csrf={csrf}&confirm=Millenium"),
        )
        .await;
        assert_eq!(mismatch.status(), StatusCode::OK);
        let mismatch_body = body_string(mismatch).await;
        assert!(mismatch_body.contains("ne correspond pas"));
        let still: (i64,) = sqlx::query_as("SELECT count(*) FROM vehicles")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(still.0, 2);

        let removed = post_form(
            &app,
            &format!("/v/{}/delete", first.0),
            &cookie,
            &format!("csrf={csrf}&confirm=Faucon"),
        )
        .await;
        assert!(removed.status().is_redirection());
        let names: Vec<(String,)> = sqlx::query_as("SELECT name FROM vehicles ORDER BY name")
            .fetch_all(&db.pool)
            .await
            .unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].0, "Millenium");
        let drives: (i64,) = sqlx::query_as("SELECT count(*) FROM drives")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(drives.0, 1);

        let unconfirmed =
            post_form(&app, "/auth/disconnect", &cookie, &format!("csrf={csrf}")).await;
        assert_eq!(unconfirmed.status(), StatusCode::OK);
        let kept: (i64,) = sqlx::query_as("SELECT count(*) FROM vehicles")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(kept.0, 1);
        let confirmed = post_form(
            &app,
            "/auth/disconnect",
            &cookie,
            &format!("csrf={csrf}&confirm=disconnect"),
        )
        .await;
        assert!(confirmed.status().is_redirection());
        let after: (i64,) = sqlx::query_as("SELECT count(*) FROM vehicles")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(after.0, 1);
    }
}
