// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Read-only drive and charge lists. Pagination is keyset on the sort column.
//! Coordinates are drawn, not written to logs.

use std::fmt::Write as _;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Redirect, Response};
use chrono::{DateTime, Utc};
use maud::html;
use serde::Deserialize;

use super::markup::{GlobalTab, VehicleTab, document, missing_page, toolbar, vehicle_tabs};
use super::ui::{chrome, html_page, remember_vehicle, signed_page};
use crate::app::AppState;
use crate::rdp::rdp_limit;

const PAGE: usize = 50;

type TrackPoint = (
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
);
type ChargeSample = (Option<f64>, Option<f64>);

#[derive(Clone, Copy)]
enum Sort {
    Start,
    Distance,
    Duration,
}

#[derive(Deserialize, Default)]
pub(crate) struct ListQuery {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    min_km: Option<f64>,
    #[serde(default)]
    min_s: Option<i32>,
    #[serde(default)]
    suspect: Option<String>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    before: Option<String>,
    #[serde(default)]
    before_id: Option<i64>,
}

#[derive(Clone, sqlx::FromRow)]
struct DriveRow {
    id: i64,
    start_at: DateTime<Utc>,
    end_at: Option<DateTime<Utc>>,
    distance_km: Option<f64>,
    duration_s: Option<i32>,
    energy_used_kwh: Option<f64>,
    source: String,
}

#[derive(Clone, sqlx::FromRow)]
struct ChargeRow {
    id: i64,
    start_at: DateTime<Utc>,
    end_at: Option<DateTime<Utc>>,
    energy_added_kwh: Option<f64>,
    energy_used_kwh: Option<f64>,
    duration_s: Option<i32>,
    cost: Option<f64>,
    is_dc: Option<bool>,
    source: String,
}

struct Bounds<'a> {
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    source: Option<&'a str>,
    min_km: Option<f64>,
    min_s: Option<i32>,
    suspect: Option<&'a str>,
    before: Option<DateTime<Utc>>,
    before_num: Option<f64>,
    before_id: Option<i64>,
    sort: Sort,
}

pub(super) async fn drives(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(query): Query<ListQuery>,
) -> Response {
    let Some(opened) = open_vehicle(&app, &headers, id).await else {
        return Redirect::to("/").into_response();
    };
    let page = match opened {
        Ok(page) => page,
        Err(response) => return response,
    };
    let bounds = bounds_from(&query);
    let rows = match drive_page(&app, id, &bounds, i64::try_from(PAGE).unwrap_or(50) + 1).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "drive list failed");
            Vec::new()
        }
    };
    let (shown, next) = split_page(&rows, bounds.sort);
    let chrome = chrome(
        &page,
        Some(id),
        Some(VehicleTab::Drives),
        GlobalTab::None,
        false,
    );
    let body = html! {
        (toolbar(&chrome, "Trajets"))
        (vehicle_tabs(&chrome))
        div role="tabpanel" {
            (filters(id, "drives", &query))
            section class="card" {
                h2 { "Trajets" }
                table class="settings" {
                    thead {
                        tr {
                            th { "Début" }
                            th { "Distance" }
                            th { "Durée" }
                            th { "Énergie" }
                            th { "Source" }
                        }
                    }
                    tbody {
                        @for row in &shown {
                            tr {
                                td { a href=(format!("/v/{id}/drives/{}", row.id)) { (row.start_at.format("%Y-%m-%d %H:%M UTC").to_string()) } }
                                td { (fmt_opt(row.distance_km, "km")) }
                                td { (fmt_duration(row.duration_s)) }
                                td { (fmt_opt(row.energy_used_kwh, "kWh")) }
                                td { (row.source) }
                            }
                        }
                    }
                }
                @if let Some(href) = next {
                    p { a href=(href) { "Page suivante" } }
                }
            }
        }
    };
    html_page(document(false, body), page.cookie)
}

pub(super) async fn drives_csv(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(query): Query<ListQuery>,
) -> Response {
    let Some(opened) = open_vehicle(&app, &headers, id).await else {
        return Redirect::to("/").into_response();
    };
    let page = match opened {
        Ok(page) => page,
        Err(response) => return response,
    };
    let _ = page;
    let mut bounds = bounds_from(&query);
    bounds.before = None;
    bounds.before_id = None;
    let rows = drive_page(&app, id, &bounds, 5_000)
        .await
        .unwrap_or_default();
    let mut csv =
        String::from("id,start_at,end_at,distance_km,duration_s,energy_used_kwh,source\n");
    for row in rows.into_iter().take(5_000) {
        let _ = writeln!(
            csv,
            "{},{},{},{},{},{},{}",
            row.id,
            row.start_at.to_rfc3339(),
            row.end_at.map(|at| at.to_rfc3339()).unwrap_or_default(),
            fmt_num(row.distance_km),
            row.duration_s
                .map(|value| value.to_string())
                .unwrap_or_default(),
            fmt_num(row.energy_used_kwh),
            row.source
        );
    }
    csv_response(csv)
}

pub(super) async fn drive_detail(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((id, drive_id)): Path<(i64, i64)>,
) -> Response {
    let Some(opened) = open_vehicle(&app, &headers, id).await else {
        return Redirect::to("/").into_response();
    };
    let page = match opened {
        Ok(page) => page,
        Err(response) => return response,
    };
    let drive = sqlx::query_as::<_, DriveRow>(
        "SELECT id, start_at, end_at, distance_km, duration_s, energy_used_kwh, source
         FROM drives WHERE id = $1 AND vehicle_id = $2 AND deleted_at IS NULL",
    )
    .bind(drive_id)
    .bind(id)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten();
    let Some(drive) = drive else {
        return html_page(
            missing_page(&chrome(&page, Some(id), None, GlobalTab::None, false)),
            page.cookie,
        );
    };
    let end = drive.end_at.unwrap_or_else(Utc::now);
    let points: Vec<TrackPoint> = sqlx::query_as(
        "SELECT latitude, longitude, speed_kmh, power_kw, elevation_m
         FROM positions
         WHERE drive_id = $1 AND recorded_at >= $2 AND recorded_at <= $3
         ORDER BY recorded_at",
    )
    .bind(drive_id)
    .bind(drive.start_at)
    .bind(end)
    .fetch_all(&app.inner.db)
    .await
    .unwrap_or_default();
    let located: Vec<[f64; 2]> = points
        .iter()
        .filter_map(|point| Some([point.0?, point.1?]))
        .collect();
    let kept = rdp_limit(&located, 200);
    let map = polyline(&located, &kept);
    let speed = series_polyline(&points, |point| point.2);
    let power = series_polyline(&points, |point| point.3);
    let altitude = series_polyline(&points, |point| point.4);
    let chrome = chrome(
        &page,
        Some(id),
        Some(VehicleTab::Drives),
        GlobalTab::None,
        false,
    );
    let body = html! {
        (toolbar(&chrome, "Trajet"))
        (vehicle_tabs(&chrome))
        section class="card" {
            h2 { (drive.start_at.format("%Y-%m-%d %H:%M UTC").to_string()) }
            p { (fmt_opt(drive.distance_km, "km")) " · " (fmt_duration(drive.duration_s)) " · " (fmt_opt(drive.energy_used_kwh, "kWh")) }
            p { a href=(format!("/v/{id}/drives")) { "Retour à la liste" } }
        }
        section class="card" {
            h2 { "Tracé" }
            (maud::PreEscaped(map))
        }
        section class="card" {
            h2 { "Vitesse" }
            (maud::PreEscaped(speed))
            h2 { "Puissance" }
            (maud::PreEscaped(power))
            h2 { "Altitude" }
            (maud::PreEscaped(altitude))
        }
        section class="card" {
            h2 { "Modifier" }
            form class="stack" method="post" action=(format!("/v/{id}/drives/{drive_id}/edit")) {
                input type="hidden" name="csrf" value=(&page.csrf);
                label class="field" for="note" { "Note" input id="note" name="note"; }
                label class="field" for="tag" { "Tag" input id="tag" name="tag"; }
                label class="field" for="start_geofence_id" { "Géofence départ (id, 0 pour vider)" input id="start_geofence_id" name="start_geofence_id"; }
                label class="field" for="end_geofence_id" { "Géofence arrivée" input id="end_geofence_id" name="end_geofence_id"; }
                label class="field" for="start_address_id" { "Adresse départ" input id="start_address_id" name="start_address_id"; }
                label class="field" for="end_address_id" { "Adresse arrivée" input id="end_address_id" name="end_address_id"; }
                button type="submit" { "Enregistrer" }
            }
            form method="post" action=(format!("/v/{id}/drives/{drive_id}/recompute")) {
                input type="hidden" name="csrf" value=(&page.csrf);
                button class="tonal" type="submit" { "Recalculer" }
            }
            form class="stack" method="post" action=(format!("/v/{id}/drives/{drive_id}/split")) {
                input type="hidden" name="csrf" value=(&page.csrf);
                label class="field" for="split_at" { "Couper à" input id="split_at" name="at" required; }
                button class="tonal" type="submit" { "Couper" }
            }
            form class="stack" method="post" action=(format!("/v/{id}/drives/{drive_id}/merge")) {
                input type="hidden" name="csrf" value=(&page.csrf);
                label class="field" for="other" { "Fusionner avec l'id" input id="other" name="other" required; }
                button class="tonal" type="submit" { "Fusionner" }
            }
            form method="post" action=(format!("/v/{id}/drives/{drive_id}/trash")) {
                input type="hidden" name="csrf" value=(&page.csrf);
                button class="danger" type="submit" { "Mettre à la corbeille" }
            }
            form class="stack" method="post" action=(format!("/v/{id}/drives/{drive_id}/purge")) {
                input type="hidden" name="csrf" value=(&page.csrf);
                label class="field" for="confirm_id" { "Retaper l'id pour purger" input id="confirm_id" name="confirm_id" required; }
                label class="field" for="positions" {
                    "Positions"
                    select id="positions" name="positions" {
                        option value="detach" { "Détacher" }
                        option value="delete" { "Supprimer sur la plage" }
                    }
                }
                button class="danger" type="submit" { "Purger" }
            }
        }
    };
    html_page(document(false, body), page.cookie)
}

pub(super) async fn charges(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(query): Query<ListQuery>,
) -> Response {
    let Some(opened) = open_vehicle(&app, &headers, id).await else {
        return Redirect::to("/").into_response();
    };
    let page = match opened {
        Ok(page) => page,
        Err(response) => return response,
    };
    let bounds = bounds_from(&query);
    let rows = charge_page(&app, id, &bounds, i64::try_from(PAGE).unwrap_or(50) + 1)
        .await
        .unwrap_or_default();
    let (shown, next) = split_charge(&rows, bounds.sort);
    let chrome = chrome(
        &page,
        Some(id),
        Some(VehicleTab::Charges),
        GlobalTab::None,
        false,
    );
    let body = html! {
        (toolbar(&chrome, "Charges"))
        (vehicle_tabs(&chrome))
        div role="tabpanel" {
            (filters(id, "charges", &query))
            section class="card" {
                h2 { "Charges" }
                table class="settings" {
                    thead {
                        tr {
                            th { "Début" }
                            th { "Énergie" }
                            th { "Durée" }
                            th { "Coût" }
                            th { "Source" }
                        }
                    }
                    tbody {
                        @for row in &shown {
                            tr {
                                td { a href=(format!("/v/{id}/charges/{}", row.id)) { (row.start_at.format("%Y-%m-%d %H:%M UTC").to_string()) } }
                                td { (fmt_opt(row.energy_added_kwh, "kWh")) }
                                td { (fmt_duration(row.duration_s)) }
                                td { (fmt_opt(row.cost, "")) }
                                td { (row.source) }
                            }
                        }
                    }
                }
                @if let Some(href) = next {
                    p { a href=(href) { "Page suivante" } }
                }
            }
        }
    };
    html_page(document(false, body), page.cookie)
}

pub(super) async fn charges_csv(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
    Query(query): Query<ListQuery>,
) -> Response {
    let Some(opened) = open_vehicle(&app, &headers, id).await else {
        return Redirect::to("/").into_response();
    };
    if let Err(response) = opened {
        return response;
    }
    let mut bounds = bounds_from(&query);
    bounds.before = None;
    bounds.before_id = None;
    let rows = charge_page(&app, id, &bounds, 5_000)
        .await
        .unwrap_or_default();
    let mut csv =
        String::from("id,start_at,end_at,energy_added_kwh,duration_s,cost,is_dc,source\n");
    for row in rows.into_iter().take(5_000) {
        let _ = writeln!(
            csv,
            "{},{},{},{},{},{},{},{}",
            row.id,
            row.start_at.to_rfc3339(),
            row.end_at.map(|at| at.to_rfc3339()).unwrap_or_default(),
            fmt_num(row.energy_added_kwh),
            row.duration_s
                .map(|value| value.to_string())
                .unwrap_or_default(),
            fmt_num(row.cost),
            row.is_dc.map(|value| value.to_string()).unwrap_or_default(),
            row.source
        );
    }
    csv_response(csv)
}

pub(super) async fn charge_detail(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((id, charge_id)): Path<(i64, i64)>,
) -> Response {
    let Some(opened) = open_vehicle(&app, &headers, id).await else {
        return Redirect::to("/").into_response();
    };
    let page = match opened {
        Ok(page) => page,
        Err(response) => return response,
    };
    let charge = sqlx::query_as::<_, ChargeRow>(
        "SELECT id, start_at, end_at, energy_added_kwh, energy_used_kwh, duration_s, cost, is_dc, source
         FROM charges WHERE id = $1 AND vehicle_id = $2 AND deleted_at IS NULL",
    )
    .bind(charge_id)
    .bind(id)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten();
    let Some(charge) = charge else {
        return html_page(
            missing_page(&chrome(&page, Some(id), None, GlobalTab::None, false)),
            page.cookie,
        );
    };
    let samples: Vec<ChargeSample> = sqlx::query_as(
        "SELECT charger_power_kw, battery_level FROM charge_samples
         WHERE charge_id = $1 ORDER BY recorded_at",
    )
    .bind(charge_id)
    .fetch_all(&app.inner.db)
    .await
    .unwrap_or_default();
    let power = sample_polyline(&samples, |sample| sample.0);
    let soc = sample_polyline(&samples, |sample| sample.1);
    let chrome = chrome(
        &page,
        Some(id),
        Some(VehicleTab::Charges),
        GlobalTab::None,
        false,
    );
    let body = html! {
        (toolbar(&chrome, "Charge"))
        (vehicle_tabs(&chrome))
        section class="card" {
            h2 { (charge.start_at.format("%Y-%m-%d %H:%M UTC").to_string()) }
            p { (fmt_opt(charge.energy_added_kwh, "kWh")) " · " (fmt_opt(charge.energy_used_kwh, "kWh utilisés")) }
            p { a href=(format!("/v/{id}/charges")) { "Retour à la liste" } }
        }
        section class="card" {
            h2 { "Puissance" }
            (maud::PreEscaped(power))
            h2 { "État de charge" }
            (maud::PreEscaped(soc))
        }
    };
    html_page(document(false, body), page.cookie)
}

async fn open_vehicle(
    app: &AppState,
    headers: &HeaderMap,
    id: i64,
) -> Option<Result<super::ui::Page, Response>> {
    let page = signed_page(app, headers).await?;
    if !page.vehicles.iter().any(|vehicle| vehicle.id == id) {
        let response = html_page(
            missing_page(&chrome(&page, None, None, GlobalTab::None, false)),
            page.cookie,
        );
        return Some(Err(response));
    }
    remember_vehicle(app, &page.session_id, id).await;
    Some(Ok(page))
}

fn bounds_from(query: &ListQuery) -> Bounds<'_> {
    let sort = match query.sort.as_deref() {
        Some("distance") => Sort::Distance,
        Some("duration") => Sort::Duration,
        _ => Sort::Start,
    };
    let before = query.before.as_deref().and_then(|value| value.parse().ok());
    let before_num = query.before.as_deref().and_then(|value| value.parse().ok());
    Bounds {
        from: query.from.as_deref().and_then(parse_time),
        to: query.to.as_deref().and_then(parse_time),
        source: allowed_source(query.source.as_deref()),
        min_km: query.min_km,
        min_s: query.min_s,
        suspect: allowed_suspect(query.suspect.as_deref()),
        before: if matches!(sort, Sort::Start) {
            before
        } else {
            None
        },
        before_num: if matches!(sort, Sort::Start) {
            None
        } else {
            before_num
        },
        before_id: query.before_id,
        sort,
    }
}

fn allowed_source(value: Option<&str>) -> Option<&str> {
    matches!(value, Some("poll" | "stream" | "teslamate" | "manual")).then_some(value)?
}

fn allowed_suspect(value: Option<&str>) -> Option<&str> {
    matches!(value, Some("zero_km" | "no_energy" | "open")).then_some(value)?
}

fn parse_time(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|time| time.with_timezone(&Utc))
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .map(|time| time.and_utc())
        })
}

async fn drive_page(
    app: &AppState,
    vehicle_id: i64,
    bounds: &Bounds<'_>,
    limit: i64,
) -> Result<Vec<DriveRow>, sqlx::Error> {
    let sql = match bounds.sort {
        Sort::Start => drive_sql(
            "($8::timestamptz IS NULL OR (start_at, id) < ($8, $9))",
            "start_at DESC, id DESC",
        ),
        Sort::Distance => drive_sql(
            "($8::float8 IS NULL OR (COALESCE(distance_km, -1e308), id) < ($8, $9))",
            "distance_km DESC NULLS LAST, id DESC",
        ),
        Sort::Duration => drive_sql(
            "($8::float8 IS NULL OR (COALESCE(duration_s::float8, -1), id) < ($8, $9))",
            "duration_s DESC NULLS LAST, id DESC",
        ),
    };
    let cursor_time = bounds.before;
    let cursor_num = bounds.before_num;
    let query = sqlx::query_as::<_, DriveRow>(&sql)
        .bind(vehicle_id)
        .bind(bounds.from)
        .bind(bounds.to)
        .bind(bounds.source)
        .bind(bounds.min_km)
        .bind(bounds.min_s)
        .bind(bounds.suspect);
    match bounds.sort {
        Sort::Start => {
            query
                .bind(cursor_time)
                .bind(bounds.before_id)
                .bind(limit)
                .fetch_all(&app.inner.db)
                .await
        }
        Sort::Distance | Sort::Duration => {
            query
                .bind(cursor_num)
                .bind(bounds.before_id)
                .bind(limit)
                .fetch_all(&app.inner.db)
                .await
        }
    }
}

fn drive_sql(cursor: &str, order: &str) -> String {
    format!(
        "SELECT id, start_at, end_at, distance_km, duration_s, energy_used_kwh, source
         FROM drives
         WHERE vehicle_id = $1
           AND deleted_at IS NULL
           AND ($2::timestamptz IS NULL OR start_at >= $2)
           AND ($3::timestamptz IS NULL OR start_at < $3)
           AND ($4::text IS NULL OR source = $4)
           AND ($5::float8 IS NULL OR COALESCE(distance_km, 0) >= $5)
           AND ($6::int IS NULL OR COALESCE(duration_s, 0) >= $6)
           AND ($7::text IS NULL OR (
                ($7 = 'zero_km' AND COALESCE(distance_km, 0) = 0)
             OR ($7 = 'no_energy' AND COALESCE(energy_used_kwh, 0) = 0)
             OR ($7 = 'open' AND end_at IS NULL AND start_at < now() - interval '24 hours')))
           AND {cursor}
         ORDER BY {order}
         LIMIT $10"
    )
}

async fn charge_page(
    app: &AppState,
    vehicle_id: i64,
    bounds: &Bounds<'_>,
    limit: i64,
) -> Result<Vec<ChargeRow>, sqlx::Error> {
    let sql = match bounds.sort {
        Sort::Start => charge_sql(
            "($8::timestamptz IS NULL OR (start_at, id) < ($8, $9))",
            "start_at DESC, id DESC",
        ),
        Sort::Distance => charge_sql(
            "($8::float8 IS NULL OR (COALESCE(energy_added_kwh, -1e308), id) < ($8, $9))",
            "energy_added_kwh DESC NULLS LAST, id DESC",
        ),
        Sort::Duration => charge_sql(
            "($8::float8 IS NULL OR (COALESCE(duration_s::float8, -1), id) < ($8, $9))",
            "duration_s DESC NULLS LAST, id DESC",
        ),
    };
    let query = sqlx::query_as::<_, ChargeRow>(&sql)
        .bind(vehicle_id)
        .bind(bounds.from)
        .bind(bounds.to)
        .bind(bounds.source)
        .bind(bounds.min_km)
        .bind(bounds.min_s)
        .bind(bounds.suspect);
    match bounds.sort {
        Sort::Start => {
            query
                .bind(bounds.before)
                .bind(bounds.before_id)
                .bind(limit)
                .fetch_all(&app.inner.db)
                .await
        }
        Sort::Distance | Sort::Duration => {
            query
                .bind(bounds.before_num)
                .bind(bounds.before_id)
                .bind(limit)
                .fetch_all(&app.inner.db)
                .await
        }
    }
}

fn charge_sql(cursor: &str, order: &str) -> String {
    format!(
        "SELECT id, start_at, end_at, energy_added_kwh, energy_used_kwh, duration_s, cost, is_dc, source
         FROM charges
         WHERE vehicle_id = $1
           AND deleted_at IS NULL
           AND ($2::timestamptz IS NULL OR start_at >= $2)
           AND ($3::timestamptz IS NULL OR start_at < $3)
           AND ($4::text IS NULL OR source = $4)
           AND ($5::float8 IS NULL OR COALESCE(energy_added_kwh, 0) >= $5)
           AND ($6::int IS NULL OR COALESCE(duration_s, 0) >= $6)
           AND ($7::text IS NULL OR (
                ($7 = 'zero_km' AND COALESCE(energy_added_kwh, 0) = 0)
             OR ($7 = 'no_energy' AND COALESCE(energy_used_kwh, 0) = 0)
             OR ($7 = 'open' AND end_at IS NULL AND start_at < now() - interval '24 hours')))
           AND {cursor}
         ORDER BY {order}
         LIMIT $10"
    )
}

fn split_page(rows: &[DriveRow], sort: Sort) -> (Vec<DriveRow>, Option<String>) {
    if rows.len() <= PAGE {
        return (rows.to_vec(), None);
    }
    let shown = rows[..PAGE].to_vec();
    let last = shown.last().unwrap();
    let before = match sort {
        Sort::Start => last.start_at.to_rfc3339(),
        Sort::Distance => fmt_num(last.distance_km),
        Sort::Duration => last.duration_s.unwrap_or(0).to_string(),
    };
    let href = format!(
        "?sort={}&before={}&before_id={}",
        sort_name(sort),
        before,
        last.id
    );
    (shown, Some(href))
}

fn split_charge(rows: &[ChargeRow], sort: Sort) -> (Vec<ChargeRow>, Option<String>) {
    if rows.len() <= PAGE {
        return (rows.to_vec(), None);
    }
    let shown = rows[..PAGE].to_vec();
    let last = shown.last().unwrap();
    let before = match sort {
        Sort::Start => last.start_at.to_rfc3339(),
        Sort::Distance => fmt_num(last.energy_added_kwh),
        Sort::Duration => last.duration_s.unwrap_or(0).to_string(),
    };
    let href = format!(
        "?sort={}&before={}&before_id={}",
        sort_name(sort),
        before,
        last.id
    );
    (shown, Some(href))
}

fn sort_name(sort: Sort) -> &'static str {
    match sort {
        Sort::Start => "start",
        Sort::Distance => "distance",
        Sort::Duration => "duration",
    }
}

fn filters(id: i64, kind: &str, query: &ListQuery) -> maud::Markup {
    html! {
        form class="filters" method="get" action=(format!("/v/{id}/{kind}")) {
            label class="field" for="from" { "Du" input id="from" name="from" value=(query.from.as_deref().unwrap_or("")); }
            label class="field" for="to" { "Au" input id="to" name="to" value=(query.to.as_deref().unwrap_or("")); }
            label class="field" for="source" {
                "Source"
                select id="source" name="source" {
                    option value="" { "toutes" }
                    option value="poll" { "poll" }
                    option value="stream" { "stream" }
                    option value="teslamate" { "teslamate" }
                    option value="manual" { "manual" }
                }
            }
            label class="field" for="min_km" { "Minimum km ou kWh" input id="min_km" name="min_km" value=(query.min_km.map(|value| value.to_string()).unwrap_or_default()); }
            label class="field" for="min_s" { "Durée min (s)" input id="min_s" name="min_s" value=(query.min_s.map(|value| value.to_string()).unwrap_or_default()); }
            label class="field" for="suspect" {
                "Suspects"
                select id="suspect" name="suspect" {
                    option value="" { "tous" }
                    option value="zero_km" { "0 km" }
                    option value="no_energy" { "sans énergie" }
                    option value="open" { "ouverts > 24 h" }
                }
            }
            label class="field" for="sort" {
                "Tri"
                select id="sort" name="sort" {
                    option value="start" { "début" }
                    option value="distance" { "distance / énergie" }
                    option value="duration" { "durée" }
                }
            }
            button class="tonal" type="submit" { "Filtrer" }
            a href=(format!("/v/{id}/{kind}.csv")) { "CSV" }
        }
    }
}

fn coord(index: usize) -> f64 {
    f64::from(u32::try_from(index).unwrap_or(u32::MAX))
}

fn polyline(points: &[[f64; 2]], kept: &[usize]) -> String {
    if kept.is_empty() {
        return "<p class=\"muted\">Pas de points GPS.</p>".to_owned();
    }
    let selected: Vec<[f64; 2]> = kept
        .iter()
        .filter_map(|index| points.get(*index).copied())
        .collect();
    chart_svg(
        &selected
            .iter()
            .enumerate()
            .map(|(index, point)| (coord(index), point[1]))
            .collect::<Vec<_>>(),
    )
}

fn series_polyline(points: &[TrackPoint], pick: impl Fn(&TrackPoint) -> Option<f64>) -> String {
    let series: Vec<(f64, f64)> = points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| Some((coord(index), pick(point)?)))
        .collect();
    chart_svg(&series)
}

fn sample_polyline(points: &[ChargeSample], pick: impl Fn(&ChargeSample) -> Option<f64>) -> String {
    let series: Vec<(f64, f64)> = points
        .iter()
        .enumerate()
        .filter_map(|(index, point)| Some((coord(index), pick(point)?)))
        .collect();
    chart_svg(&series)
}

fn chart_svg(series: &[(f64, f64)]) -> String {
    if series.len() < 2 {
        return "<p class=\"muted\">Pas assez de points.</p>".to_owned();
    }
    let min_y = series
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let max_y = series
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let span = (max_y - min_y).max(1e-9);
    let last_x = series.last().map_or(1.0, |point| point.0).max(1.0);
    let coords = series
        .iter()
        .map(|point| {
            let x = point.0 / last_x * 100.0;
            let y = 40.0 - ((point.1 - min_y) / span) * 36.0;
            format!("{x:.2},{y:.2}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "<svg class=\"chart\" viewBox=\"0 0 100 44\" width=\"640\" height=\"180\" role=\"img\" aria-label=\"graphique\"><polyline fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.2\" points=\"{coords}\"/></svg>"
    )
}

fn fmt_opt(value: Option<f64>, unit: &str) -> String {
    value.map_or_else(|| "—".to_owned(), |value| format!("{value:.1} {unit}"))
}

fn fmt_num(value: Option<f64>) -> String {
    value.map_or_else(String::new, |value| format!("{value:.3}"))
}

fn fmt_duration(value: Option<i32>) -> String {
    value.map_or_else(|| "—".to_owned(), |secs| format!("{} min", secs / 60))
}

fn csv_response(body: String) -> Response {
    let mut response = (
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/csv; charset=utf-8"),
        )],
        body,
    )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;
    use tower::ServiceExt;

    use super::chart_svg;

    #[test]
    fn chart_svg_has_no_style_attribute() {
        let svg = chart_svg(&[(0.0, 1.0), (1.0, 3.0)]);
        assert!(svg.contains("<polyline"));
        assert!(svg.contains("height=\"180\""));
        assert!(!svg.contains("style="));
        assert!(!svg.contains("<text"));
        assert!(!svg.contains("<script"));
    }

    #[tokio::test]
    async fn drive_list_is_keyset_and_csv_omits_coordinates() {
        let Some(db) = crate::test_util::TestDb::migrated("trips").await else {
            return;
        };
        let state = crate::app::test_state(db.pool.clone()).await;
        let vehicle: (i64,) = sqlx::query_as(
            "INSERT INTO vehicles (vin, name) VALUES ('5YJ30000000000099', 'Liste') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        for (hour, km) in [(10, 0.0), (11, 12.0)] {
            sqlx::query(
                "INSERT INTO drives (vehicle_id, start_at, end_at, distance_km, duration_s, energy_used_kwh, source)
                 VALUES ($1, $2, $3, $4, 600, 1.5, 'poll')",
            )
            .bind(vehicle.0)
            .bind(chrono::Utc.with_ymd_and_hms(2024, 6, 1, hour, 0, 0).unwrap())
            .bind(chrono::Utc.with_ymd_and_hms(2024, 6, 1, hour, 10, 0).unwrap())
            .bind(km)
            .execute(&db.pool)
            .await
            .unwrap();
        }
        let login = crate::identity::login(&state, "admin", "test-password", "", "127.0.0.1")
            .await
            .unwrap();
        let crate::identity::LoginResult::Session(issued) = login else {
            panic!("session");
        };
        sqlx::query("UPDATE users SET must_change_password = false, totp_enabled = true")
            .execute(&db.pool)
            .await
            .unwrap();
        let cookie = format!("__Host-tl_session={}", issued.id);
        let first = get(
            &state,
            &format!("/v/{}/drives?sort=start", vehicle.0),
            &cookie,
        )
        .await;
        assert!(first.contains("2024-06-01 11:00"));
        assert!(first.contains("2024-06-01 10:00"));
        assert!(first.contains("<main class=\"shell\">"));
        let zero = get(
            &state,
            &format!("/v/{}/drives?suspect=zero_km", vehicle.0),
            &cookie,
        )
        .await;
        assert!(zero.contains("2024-06-01 10:00"));
        assert!(!zero.contains("2024-06-01 11:00"));
        let later: (i64,) =
            sqlx::query_as("SELECT id FROM drives WHERE vehicle_id = $1 AND distance_km = 12")
                .bind(vehicle.0)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        let cursor = super::ListQuery {
            sort: Some("start".to_owned()),
            before: Some("2024-06-01T11:00:00+00:00".to_owned()),
            before_id: Some(later.0),
            ..super::ListQuery::default()
        };
        let older = super::drive_page(&state, vehicle.0, &super::bounds_from(&cursor), 50)
            .await
            .unwrap();
        assert_eq!(older.len(), 1);
        assert_eq!(older[0].distance_km, Some(0.0));
        let csv = get(&state, &format!("/v/{}/drives.csv", vehicle.0), &cookie).await;
        assert!(csv.starts_with("id,start_at"));
        assert!(!csv.to_ascii_lowercase().contains("latitude"));
        let drive: (i64,) = sqlx::query_as("SELECT id FROM drives WHERE distance_km = 12")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO positions (vehicle_id, recorded_at, latitude, longitude, speed_kmh, drive_id, source)
             VALUES ($1, '2024-06-01T11:01:00Z', 48.1, 2.1, 30, $2, 'poll'),
                    ($1, '2024-06-01T11:02:00Z', 48.2, 2.2, 40, $2, 'poll')",
        )
        .bind(vehicle.0)
        .bind(drive.0)
        .execute(&db.pool)
        .await
        .unwrap();
        let detail = get(
            &state,
            &format!("/v/{}/drives/{}", vehicle.0, drive.0),
            &cookie,
        )
        .await;
        assert!(detail.contains("<polyline"));
        assert!(detail.contains("Retour à la liste"));
    }

    async fn get(state: &crate::app::AppState, uri: &str, cookie: &str) -> String {
        let response = super::super::router(state.clone())
            .oneshot(
                axum::http::Request::builder()
                    .uri(uri)
                    .header(axum::http::header::COOKIE, cookie)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}
