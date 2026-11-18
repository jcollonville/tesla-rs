// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Drive and charge lists and detail pages. Pagination is keyset on the sort column.
//! Coordinates are drawn, not written to logs or to the page.

use std::fmt::Write as _;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, header};
use axum::response::{IntoResponse, Redirect, Response};
use chrono::{DateTime, Datelike, NaiveDate, Offset, Utc};
use chrono_tz::Tz;
use maud::{Markup, html};
use serde::Deserialize;

use super::format::{self, Place};
use super::markup::{GlobalTab, VehicleTab, document, missing_page, toolbar, vehicle_tabs};
use super::ui::{chrome, html_page, remember_vehicle, signed_page};
use super::viz::{self, Axis, LineChart, Series};
use crate::app::AppState;

const PAGE: usize = 50;
const CHART_POINTS: usize = 720;

#[derive(Clone, Copy, PartialEq, Eq)]
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
    energy_kwh: Option<f64>,
    start_battery_level: Option<f64>,
    end_battery_level: Option<f64>,
    source: String,
    start_address: Option<String>,
    end_address: Option<String>,
    start_geofence: Option<String>,
    end_geofence: Option<String>,
}

#[derive(Clone, sqlx::FromRow)]
struct ChargeRow {
    id: i64,
    start_at: DateTime<Utc>,
    end_at: Option<DateTime<Utc>>,
    energy_added_kwh: Option<f64>,
    duration_s: Option<i32>,
    cost: Option<f64>,
    is_dc: Option<bool>,
    charger_type: Option<String>,
    max_power_kw: Option<f64>,
    start_battery_level: Option<f64>,
    end_battery_level: Option<f64>,
    source: String,
    address: Option<String>,
    geofence: Option<String>,
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Drives,
    Charges,
}

impl Kind {
    fn path(self) -> &'static str {
        match self {
            Self::Drives => "drives",
            Self::Charges => "charges",
        }
    }
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
    let tz = format::zone(&app, Some(id)).await;
    let bounds = bounds_from(&query);
    let rows = match drive_page(&app, id, &bounds, i64::try_from(PAGE).unwrap_or(50) + 1).await {
        Ok(rows) => rows,
        Err(err) => {
            tracing::warn!(error = %err, "drive list failed");
            Vec::new()
        }
    };
    let totals = drive_totals(&app, id, &bounds).await;
    let next = (rows.len() > PAGE).then(|| {
        let last = &rows[PAGE - 1];
        let before = match bounds.sort {
            Sort::Start => last.start_at.to_rfc3339(),
            Sort::Distance => fmt_num(last.distance_km),
            Sort::Duration => last.duration_s.unwrap_or(0).to_string(),
        };
        page_href(id, Kind::Drives, &query, bounds.sort, &before, last.id)
    });
    let shown = &rows[..rows.len().min(PAGE)];
    let chrome = chrome(
        &page,
        Some(id),
        Some(VehicleTab::Drives),
        GlobalTab::None,
        false,
    );
    let today = format::local_date(Utc::now(), tz);
    let groups = group_by_day(shown, |row| row.start_at, tz, bounds.sort == Sort::Start);
    let body = html! {
        (toolbar(&chrome, "Trajets"))
        (vehicle_tabs(&chrome))
        div id="panel-drives" role="tabpanel" aria-labelledby="tab-drives" {
            (list_toolbar(id, Kind::Drives, &query, tz))
            (summary_strip(&totals))
            @if shown.is_empty() {
                section class="card empty-state" {
                    h2 { "Aucun trajet" }
                    p class="muted" { "Rien ne correspond à ces filtres." }
                }
            }
            @for (day, items) in &groups {
                section class="day" aria-label=(day_title(*day, today, bounds.sort)) {
                    header class="day-head" {
                        h2 { (day_title(*day, today, bounds.sort)) }
                        p class="muted" { (drive_day_totals(items)) }
                    }
                    ul class="trips" role="list" {
                        @for row in items {
                            li { (drive_item(id, row, tz)) }
                        }
                    }
                }
            }
            (pager(id, Kind::Drives, &query, next.as_deref()))
        }
    };
    html_page(document(false, body), page.cookie)
}

fn day_title(day: Option<NaiveDate>, today: NaiveDate, sort: Sort) -> String {
    match (day, sort) {
        (Some(day), _) => format::day_label(day, today),
        (None, Sort::Distance) => "Du plus long au plus court".to_owned(),
        (None, _) => "De la plus longue à la plus courte durée".to_owned(),
    }
}

/// Consecutive rows that start on the same local day. `None` when the list is not by date.
fn group_by_day<T>(
    rows: &[T],
    start: impl Fn(&T) -> DateTime<Utc>,
    tz: Tz,
    by_day: bool,
) -> Vec<(Option<NaiveDate>, Vec<&T>)> {
    let mut groups: Vec<(Option<NaiveDate>, Vec<&T>)> = Vec::new();
    for row in rows {
        let day = by_day.then(|| format::local_date(start(row), tz));
        match groups.last_mut() {
            Some((current, items)) if *current == day => items.push(row),
            _ => groups.push((day, vec![row])),
        }
    }
    groups
}

fn drive_day_totals(items: &[&DriveRow]) -> String {
    let distance: f64 = items.iter().filter_map(|row| row.distance_km).sum();
    let seconds: i64 = items
        .iter()
        .filter_map(|row| row.duration_s)
        .map(i64::from)
        .sum();
    let count = items.len();
    let label = if count > 1 { "trajets" } else { "trajet" };
    format!(
        "{count} {label} · {} · {}",
        format::km(Some(distance)),
        format::duration(Some(seconds))
    )
}

fn drive_item(id: i64, row: &DriveRow, tz: Tz) -> Markup {
    let from = Place::from(row.start_address.as_deref(), row.start_geofence.as_deref());
    let to = Place::from(row.end_address.as_deref(), row.end_geofence.as_deref());
    let from_name = Place::name_or_unknown(from.as_ref());
    let to_name = if row.end_at.is_none() {
        "En route".to_owned()
    } else {
        Place::name_or_unknown(to.as_ref())
    };
    let efficiency = format::efficiency(row.energy_kwh, row.distance_km);
    let label = format!(
        "{} à {}, de {from_name} à {to_name}, {}",
        format::capitalize(&format::long_date(
            format::local_date(row.start_at, tz),
            false
        )),
        format::clock(row.start_at, tz),
        format::km(row.distance_km)
    );
    html! {
        a class="trip" href=(format!("/v/{id}/drives/{}", row.id)) aria-label=(label) {
            span class="trip-times" {
                time datetime=(row.start_at.to_rfc3339()) { (format::clock(row.start_at, tz)) }
                @if let Some(end) = row.end_at {
                    span { (format::clock(end, tz)) }
                } @else {
                    span class="live" { span class="live-dot" {} "en cours" }
                }
            }
            span class="route" {
                span class="route-stop start" title=[from.as_ref().and_then(|place| place.detail.clone())] { (from_name) }
                span class="route-stop end" title=[to.as_ref().and_then(|place| place.detail.clone())] { (to_name) }
            }
            span class="trip-figures" {
                span class="figure" { (format::km(row.distance_km)) }
                span class="muted" { (format::duration_i32(row.duration_s)) }
            }
            span class="trip-stats" {
                span { span class="stat-label" { "Énergie" } (energy_label(row.energy_kwh, row.energy_used_kwh.is_none())) }
                span { span class="stat-label" { "Efficacité" } (format::unit(efficiency, 0, "Wh/km")) }
                span { span class="stat-label" { "Batterie" } (battery_span(row.start_battery_level, row.end_battery_level)) }
            }
            @if row.source != "poll" && row.source != "stream" {
                span class="chip source" { (row.source) }
            }
        }
    }
}

/// Stored duration, else measured from the bounds; open sessions run until now.
fn elapsed(start: DateTime<Utc>, end: Option<DateTime<Utc>>, stored: Option<i32>) -> Option<i64> {
    stored
        .map(i64::from)
        .or_else(|| Some((end.unwrap_or_else(Utc::now) - start).num_seconds()))
}

fn energy_label(value: Option<f64>, estimated: bool) -> String {
    match value {
        Some(kwh) if estimated => format!("≈{}", format::kwh(Some(kwh))),
        other => format::kwh(other),
    }
}

fn battery_span(start: Option<f64>, end: Option<f64>) -> String {
    match (start, end) {
        (Some(start), Some(end)) => format!(
            "{} → {}",
            format::number(start, 0),
            format::percent(Some(end))
        ),
        (Some(start), None) => format::percent(Some(start)),
        _ => "—".to_owned(),
    }
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

#[derive(sqlx::FromRow)]
struct DriveDetail {
    id: i64,
    start_at: DateTime<Utc>,
    end_at: Option<DateTime<Utc>>,
    distance_km: Option<f64>,
    duration_s: Option<i32>,
    energy_used_kwh: Option<f64>,
    energy_kwh: Option<f64>,
    start_battery_level: Option<f64>,
    end_battery_level: Option<f64>,
    max_speed_kmh: Option<f64>,
    avg_speed_kmh: Option<f64>,
    max_power_kw: Option<f64>,
    min_power_kw: Option<f64>,
    elevation_gain_m: Option<f64>,
    elevation_loss_m: Option<f64>,
    outside_temp_avg_c: Option<f64>,
    start_odometer_km: Option<f64>,
    source: String,
    note: Option<String>,
    tag: Option<String>,
    start_address: Option<String>,
    end_address: Option<String>,
    start_geofence: Option<String>,
    end_geofence: Option<String>,
}

type TrackPoint = (
    DateTime<Utc>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
);

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
    let energy = super::stats::drive_energy_sql("d", "v");
    let drive = sqlx::query_as::<_, DriveDetail>(&format!(
        "SELECT d.id, d.start_at, d.end_at, d.distance_km, d.duration_s, d.energy_used_kwh,
                {energy} AS energy_kwh, d.start_battery_level, d.end_battery_level,
                d.max_speed_kmh, d.avg_speed_kmh, d.max_power_kw, d.min_power_kw,
                d.elevation_gain_m, d.elevation_loss_m, d.outside_temp_avg_c, d.start_odometer_km,
                d.source, d.note, d.tag,
                start_addr.display_name AS start_address,
                end_addr.display_name AS end_address,
                start_fence.name AS start_geofence,
                end_fence.name AS end_geofence
         FROM drives d
         JOIN vehicles v ON v.id = d.vehicle_id
         LEFT JOIN addresses start_addr ON start_addr.id = d.start_address_id
         LEFT JOIN addresses end_addr ON end_addr.id = d.end_address_id
         LEFT JOIN geofences start_fence ON start_fence.id = d.start_geofence_id
         LEFT JOIN geofences end_fence ON end_fence.id = d.end_geofence_id
         WHERE d.id = $1 AND d.vehicle_id = $2 AND d.deleted_at IS NULL"
    ))
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
    let tz = format::zone(&app, Some(id)).await;
    let end = drive.end_at.unwrap_or_else(Utc::now);
    let points: Vec<TrackPoint> = sqlx::query_as(
        "SELECT recorded_at, latitude, longitude, speed_kmh, power_kw, elevation_m, battery_level
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
    let located: Vec<(f64, f64)> = points
        .iter()
        .filter_map(|point| Some((point.1?, point.2?)))
        .collect();
    let kept = viz::stride(points.len(), CHART_POINTS);
    let sampled: Vec<&TrackPoint> = kept.iter().filter_map(|index| points.get(*index)).collect();
    let sync = format!("drive-{drive_id}");
    let map = viz::route_map(
        &located,
        &format!("route-{drive_id}"),
        &sync,
        &sampled
            .iter()
            .map(|point| Some((point.1?, point.2?)))
            .collect::<Vec<_>>(),
    );
    let axis = time_axis(drive.start_at, tz);
    let t: Vec<f64> = sampled
        .iter()
        .map(|point| format::seconds_since(point.0, drive.start_at))
        .collect();
    let pick = |get: fn(&TrackPoint) -> Option<f64>| -> Vec<Option<f64>> {
        sampled.iter().map(|point| get(point)).collect()
    };
    let speed = pick(|point| point.3);
    let power = pick(|point| point.4);
    let elevation = pick(|point| point.5);
    let battery = pick(|point| point.6);
    let charts = [
        (
            "Vitesse",
            speed_summary(&speed, drive.avg_speed_kmh),
            series("Vitesse", "km/h", 0, "accent", speed, true, false),
        ),
        (
            "Puissance",
            power_summary(&power),
            series("Puissance", "kW", 0, "violet", power, true, true),
        ),
        (
            "Altitude",
            range_summary(&elevation, "m", 0),
            series("Altitude", "m", 0, "amber", elevation, true, false),
        ),
        (
            "Batterie",
            range_summary(&battery, "%", 0),
            series("Batterie", "%", 0, "green", battery, false, false),
        ),
    ];
    let from = Place::from(
        drive.start_address.as_deref(),
        drive.start_geofence.as_deref(),
    );
    let to = Place::from(drive.end_address.as_deref(), drive.end_geofence.as_deref());
    let title = format!(
        "{} → {}",
        Place::name_or_unknown(from.as_ref()),
        if drive.end_at.is_none() {
            "En route".to_owned()
        } else {
            Place::name_or_unknown(to.as_ref())
        }
    );
    let efficiency = format::efficiency(drive.energy_kwh, drive.distance_km);
    let mut metrics = vec![
        ("Distance", format::km(drive.distance_km)),
        ("Durée", format::duration_i32(drive.duration_s)),
        (
            "Énergie",
            energy_label(drive.energy_kwh, drive.energy_used_kwh.is_none()),
        ),
        ("Efficacité", format::unit(efficiency, 0, "Wh/km")),
        (
            "Batterie",
            battery_span(drive.start_battery_level, drive.end_battery_level),
        ),
        (
            "Vitesse moyenne",
            format::unit(drive.avg_speed_kmh, 0, "km/h"),
        ),
        ("Vitesse max", format::unit(drive.max_speed_kmh, 0, "km/h")),
        ("Puissance max", format::unit(drive.max_power_kw, 0, "kW")),
    ];
    if let Some(regen) = drive.min_power_kw.filter(|kw| *kw < 0.0) {
        metrics.push(("Récupération max", format::unit(Some(-regen), 0, "kW")));
    }
    if drive.elevation_gain_m.is_some() || drive.elevation_loss_m.is_some() {
        metrics.push((
            "Dénivelé",
            format!(
                "+{} / −{}",
                format::unit(drive.elevation_gain_m, 0, "m"),
                format::unit(drive.elevation_loss_m, 0, "m")
            ),
        ));
    }
    if let Some(outside) = drive.outside_temp_avg_c {
        metrics.push((
            "Extérieur",
            format!("{}{}°C", format::number(outside, 1), format::NNBSP),
        ));
    }
    if drive.start_odometer_km.is_some() {
        metrics.push(("Compteur", format::unit(drive.start_odometer_km, 0, "km")));
    }
    let chrome = chrome(
        &page,
        Some(id),
        Some(VehicleTab::Drives),
        GlobalTab::None,
        false,
    );
    let when = match drive.end_at {
        Some(end) => format!(
            "{} – {}",
            format::moment(drive.start_at, tz),
            format::clock(end, tz)
        ),
        None => format!("{} · en cours", format::moment(drive.start_at, tz)),
    };
    let csrf = &page.csrf;
    let body = html! {
        (toolbar(&chrome, &title))
        (vehicle_tabs(&chrome))
        div class="detail" role="tabpanel" aria-labelledby="tab-drives" {
            div class="detail-head" {
                a class="text-link back-inline" href=(format!("/v/{id}/drives")) { "← Retour à la liste" }
                p class="eyebrow" { (when) }
            }
            section class="card route-card" aria-label="Tracé" {
                @if let Some(map) = &map {
                    div class="map-frame wide" { (map) }
                } @else {
                    p class="muted" { "Pas de points GPS." }
                }
                ol class="route-ends" {
                    li class="route-stop start" {
                        span class="stop-name" { (Place::name_or_unknown(from.as_ref())) }
                        @if let Some(detail) = from.as_ref().and_then(|place| place.detail.as_ref()) {
                            span class="muted" { (detail) }
                        }
                        span class="muted" { (format::clock(drive.start_at, tz)) }
                    }
                    li class="route-stop end" {
                        span class="stop-name" { (if drive.end_at.is_some() { Place::name_or_unknown(to.as_ref()) } else { "En route".to_owned() }) }
                        @if let Some(detail) = to.as_ref().and_then(|place| place.detail.as_ref()) {
                            span class="muted" { (detail) }
                        }
                        @if let Some(end) = drive.end_at {
                            span class="muted" { (format::clock(end, tz)) }
                        }
                    }
                }
            }
            section class="card" aria-labelledby="drive-metrics" {
                h2 id="drive-metrics" class="visually-hidden" { "Mesures" }
                dl class="metrics" {
                    @for (label, value) in &metrics {
                        div { dt { (label) } dd { (value) } }
                    }
                }
                @if drive.note.is_some() || drive.tag.is_some() {
                    p class="note" {
                        @if let Some(tag) = &drive.tag { span class="chip" { (tag) } " " }
                        @if let Some(note) = &drive.note { (note) }
                    }
                }
            }
            section class="card charts" aria-labelledby="drive-charts" data-sync-root=(sync) {
                div class="card-head" {
                    h2 id="drive-charts" { "Télémétrie" }
                    p class="hint chart-hint" { "Survolez pour lire, faites glisser pour zoomer." }
                }
                @if sampled.len() < 2 {
                    p class="muted" { "Pas assez de points." }
                } @else {
                    @for (name, summary, data) in charts {
                        @if data.values.iter().filter(|value| value.is_some()).count() >= 2 {
                            (viz::line_figure(name, &summary, Some(&sync), &LineChart { kind: "line", x: &axis, t: &t, series: vec![data] }))
                        }
                    }
                }
            }
            details class="card edit" {
                summary { "Modifier ce trajet" }
                div class="edit-grid" {
                    form class="stack" method="post" action=(format!("/v/{id}/drives/{drive_id}/edit")) {
                        h3 { "Notes et lieux" }
                        input type="hidden" name="csrf" value=(csrf);
                        label class="field" for="note" { "Note" input id="note" name="note" value=[drive.note.as_deref()]; }
                        label class="field" for="tag" { "Tag" input id="tag" name="tag" value=[drive.tag.as_deref()]; }
                        label class="field" for="start_geofence_id" { "Géofence départ (id, 0 pour vider)" input id="start_geofence_id" name="start_geofence_id" inputmode="numeric"; }
                        label class="field" for="end_geofence_id" { "Géofence arrivée" input id="end_geofence_id" name="end_geofence_id" inputmode="numeric"; }
                        label class="field" for="start_address_id" { "Adresse départ" input id="start_address_id" name="start_address_id" inputmode="numeric"; }
                        label class="field" for="end_address_id" { "Adresse arrivée" input id="end_address_id" name="end_address_id" inputmode="numeric"; }
                        button type="submit" { "Enregistrer" }
                    }
                    div class="stack" {
                        h3 { "Découpage" }
                        form method="post" action=(format!("/v/{id}/drives/{drive_id}/recompute")) {
                            input type="hidden" name="csrf" value=(csrf);
                            button class="tonal" type="submit" { "Recalculer" }
                        }
                        form class="stack" method="post" action=(format!("/v/{id}/drives/{drive_id}/split")) {
                            input type="hidden" name="csrf" value=(csrf);
                            label class="field" for="split_at" { "Couper à" input id="split_at" name="at" required placeholder=(drive.start_at.to_rfc3339()); }
                            button class="tonal" type="submit" { "Couper" }
                        }
                        form class="stack" method="post" action=(format!("/v/{id}/drives/{drive_id}/merge")) {
                            input type="hidden" name="csrf" value=(csrf);
                            label class="field" for="other" { "Fusionner avec l'id" input id="other" name="other" required inputmode="numeric"; }
                            button class="tonal" type="submit" { "Fusionner" }
                        }
                    }
                    div class="stack danger-zone" {
                        h3 { "Zone sensible" }
                        p class="hint" { "Trajet n° " (drive.id) " · source " (drive.source) }
                        form method="post" action=(format!("/v/{id}/drives/{drive_id}/trash")) {
                            input type="hidden" name="csrf" value=(csrf);
                            button class="danger" type="submit" { "Mettre à la corbeille" }
                        }
                        form class="stack" method="post" action=(format!("/v/{id}/drives/{drive_id}/purge")) {
                            input type="hidden" name="csrf" value=(csrf);
                            label class="field" for="confirm_id" { "Retaper l'id pour purger" input id="confirm_id" name="confirm_id" required inputmode="numeric"; }
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
                }
            }
        }
        script src="/static/charts.js" defer {}
    };
    html_page(document(false, body), page.cookie)
}

fn time_axis(start: DateTime<Utc>, tz: Tz) -> Axis {
    let offset = start.with_timezone(&tz).offset().fix().local_minus_utc();
    Axis::Time {
        origin_ms: start.timestamp_millis(),
        offset_min: offset / 60,
    }
}

fn series(
    name: &str,
    unit: &str,
    decimals: u8,
    tone: &'static str,
    values: Vec<Option<f64>>,
    area: bool,
    signed: bool,
) -> Series {
    Series {
        name: name.to_owned(),
        unit: unit.to_owned(),
        decimals,
        tone,
        values,
        area,
        signed,
    }
}

fn extent(values: &[Option<f64>]) -> Option<(f64, f64)> {
    values.iter().flatten().fold(None, |acc, value| match acc {
        None => Some((*value, *value)),
        Some((low, high)) => Some((low.min(*value), high.max(*value))),
    })
}

fn speed_summary(values: &[Option<f64>], average: Option<f64>) -> String {
    let Some((_, high)) = extent(values) else {
        return String::new();
    };
    match average {
        Some(average) => format!(
            "Max {} · moyenne {}",
            format::unit(Some(high), 0, "km/h"),
            format::unit(Some(average), 0, "km/h")
        ),
        None => format!("Max {}", format::unit(Some(high), 0, "km/h")),
    }
}

fn power_summary(values: &[Option<f64>]) -> String {
    let Some((low, high)) = extent(values) else {
        return String::new();
    };
    if low < 0.0 {
        format!(
            "Max {} · récupération {}",
            format::unit(Some(high), 0, "kW"),
            format::unit(Some(-low), 0, "kW")
        )
    } else {
        format!("Max {}", format::unit(Some(high), 0, "kW"))
    }
}

fn range_summary(values: &[Option<f64>], unit: &str, decimals: usize) -> String {
    let first = values.iter().flatten().next().copied();
    let last = values.iter().flatten().last().copied();
    let Some((low, high)) = extent(values) else {
        return String::new();
    };
    if unit == "%" {
        return format!(
            "{} → {}",
            format::number(first.unwrap_or(low), 0),
            format::percent(last)
        );
    }
    format!(
        "Min {} · max {}",
        format::unit(Some(low), decimals, unit),
        format::unit(Some(high), decimals, unit)
    )
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
    let tz = format::zone(&app, Some(id)).await;
    let bounds = bounds_from(&query);
    let rows = charge_page(&app, id, &bounds, i64::try_from(PAGE).unwrap_or(50) + 1)
        .await
        .unwrap_or_default();
    let totals = charge_totals(&app, id, &bounds).await;
    let next = (rows.len() > PAGE).then(|| {
        let last = &rows[PAGE - 1];
        let before = match bounds.sort {
            Sort::Start => last.start_at.to_rfc3339(),
            Sort::Distance => fmt_num(last.energy_added_kwh),
            Sort::Duration => last.duration_s.unwrap_or(0).to_string(),
        };
        page_href(id, Kind::Charges, &query, bounds.sort, &before, last.id)
    });
    let shown = &rows[..rows.len().min(PAGE)];
    let chrome = chrome(
        &page,
        Some(id),
        Some(VehicleTab::Charges),
        GlobalTab::None,
        false,
    );
    let today = format::local_date(Utc::now(), tz);
    let groups = group_by_day(shown, |row| row.start_at, tz, bounds.sort == Sort::Start);
    let body = html! {
        (toolbar(&chrome, "Charges"))
        (vehicle_tabs(&chrome))
        div id="panel-charges" role="tabpanel" aria-labelledby="tab-charges" {
            (list_toolbar(id, Kind::Charges, &query, tz))
            (summary_strip(&totals))
            @if shown.is_empty() {
                section class="card empty-state" {
                    h2 { "Aucune charge" }
                    p class="muted" { "Rien ne correspond à ces filtres." }
                }
            }
            @for (day, items) in &groups {
                section class="day" aria-label=(charge_day_title(*day, today, bounds.sort)) {
                    header class="day-head" {
                        h2 { (charge_day_title(*day, today, bounds.sort)) }
                        p class="muted" { (charge_day_totals(items)) }
                    }
                    ul class="trips" role="list" {
                        @for row in items {
                            li { (charge_item(id, row, tz)) }
                        }
                    }
                }
            }
            (pager(id, Kind::Charges, &query, next.as_deref()))
        }
    };
    html_page(document(false, body), page.cookie)
}

fn charge_day_title(day: Option<NaiveDate>, today: NaiveDate, sort: Sort) -> String {
    match (day, sort) {
        (Some(day), _) => format::day_label(day, today),
        (None, Sort::Distance) => "De la plus grande à la plus petite énergie".to_owned(),
        (None, _) => "De la plus longue à la plus courte durée".to_owned(),
    }
}

fn charge_day_totals(items: &[&ChargeRow]) -> String {
    let energy: f64 = items.iter().filter_map(|row| row.energy_added_kwh).sum();
    let count = items.len();
    let label = if count > 1 { "charges" } else { "charge" };
    format!("{count} {label} · +{}", format::kwh(Some(energy)))
}

fn charge_kind(row_dc: Option<bool>, charger: Option<&str>) -> String {
    let current = match row_dc {
        Some(true) => "DC",
        Some(false) => "AC",
        None => "",
    };
    match (current, charger.filter(|text| !text.is_empty())) {
        ("", Some(charger)) => charger.to_owned(),
        ("", None) => "Charge".to_owned(),
        (current, Some(charger)) => format!("{current} · {charger}"),
        (current, None) => current.to_owned(),
    }
}

fn charge_item(id: i64, row: &ChargeRow, tz: Tz) -> Markup {
    let place = Place::from(row.address.as_deref(), row.geofence.as_deref());
    let name = Place::name_or_unknown(place.as_ref());
    let duration = elapsed(row.start_at, row.end_at, row.duration_s);
    let label = format!(
        "{} à {}, {name}, {}",
        format::capitalize(&format::long_date(
            format::local_date(row.start_at, tz),
            false
        )),
        format::clock(row.start_at, tz),
        row.energy_added_kwh.map_or_else(
            || "en cours".to_owned(),
            |kwh| format!("+{}", format::kwh(Some(kwh)))
        )
    );
    let dc = row.is_dc == Some(true);
    html! {
        a class=(if dc { "trip charge dc" } else { "trip charge" }) href=(format!("/v/{id}/charges/{}", row.id)) aria-label=(label) {
            span class="trip-times" {
                time datetime=(row.start_at.to_rfc3339()) { (format::clock(row.start_at, tz)) }
                @if let Some(end) = row.end_at {
                    span { (format::clock(end, tz)) }
                } @else {
                    span class="live" { span class="live-dot" {} "en cours" }
                }
            }
            span class="route single" {
                span class="route-stop charge-stop" title=[place.as_ref().and_then(|place| place.detail.clone())] { (name) }
                span class="muted kind" {
                    @if dc { span class="chip dc" { "DC" } " " }
                    (row.charger_type.as_deref().filter(|text| !text.is_empty()).unwrap_or(match row.is_dc {
                        Some(true) => "",
                        Some(false) => "AC",
                        None => "Charge",
                    }))
                }
            }
            span class="trip-figures" {
                span class="figure" {
                    (row.energy_added_kwh.map_or_else(|| "En cours".to_owned(), |kwh| format!("+{}", format::kwh(Some(kwh)))))
                }
                span class="muted" { (format::duration(duration)) }
            }
            span class="trip-stats" {
                span class="soc" {
                    span class="stat-label" { "Batterie" }
                    (battery_span(row.start_battery_level, row.end_battery_level))
                    (viz::soc_bar(row.start_battery_level, row.end_battery_level))
                }
                span { span class="stat-label" { "Puissance max" } (format::unit(row.max_power_kw, 0, "kW")) }
                span { span class="stat-label" { "Coût" } (format::money(row.cost)) }
            }
            @if row.source != "poll" && row.source != "stream" {
                span class="chip source" { (row.source) }
            }
        }
    }
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

#[derive(sqlx::FromRow)]
struct ChargeDetail {
    id: i64,
    start_at: DateTime<Utc>,
    end_at: Option<DateTime<Utc>>,
    energy_added_kwh: Option<f64>,
    energy_used_kwh: Option<f64>,
    duration_s: Option<i32>,
    cost: Option<f64>,
    is_dc: Option<bool>,
    charger_type: Option<String>,
    max_power_kw: Option<f64>,
    start_battery_level: Option<f64>,
    end_battery_level: Option<f64>,
    start_rated_range_km: Option<f64>,
    end_rated_range_km: Option<f64>,
    outside_temp_avg_c: Option<f64>,
    source: String,
    note: Option<String>,
    address: Option<String>,
    geofence: Option<String>,
}

type ChargeSample = (
    DateTime<Utc>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
    Option<f64>,
);

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
    let charge = sqlx::query_as::<_, ChargeDetail>(
        "SELECT c.id, c.start_at, c.end_at, c.energy_added_kwh, c.energy_used_kwh, c.duration_s,
                c.cost, c.is_dc, c.charger_type, c.max_power_kw, c.start_battery_level,
                c.end_battery_level, c.start_rated_range_km, c.end_rated_range_km,
                c.outside_temp_avg_c, c.source, c.note,
                addr.display_name AS address,
                fence.name AS geofence
         FROM charges c
         LEFT JOIN addresses addr ON addr.id = c.address_id
         LEFT JOIN geofences fence ON fence.id = c.geofence_id
         WHERE c.id = $1 AND c.vehicle_id = $2 AND c.deleted_at IS NULL",
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
    let tz = format::zone(&app, Some(id)).await;
    let samples: Vec<ChargeSample> = sqlx::query_as(
        "SELECT recorded_at, charger_power_kw, battery_level, charger_voltage, charge_energy_added_kwh
         FROM charge_samples
         WHERE charge_id = $1 ORDER BY recorded_at",
    )
    .bind(charge_id)
    .fetch_all(&app.inner.db)
    .await
    .unwrap_or_default();
    let kept = viz::stride(samples.len(), CHART_POINTS);
    let sampled: Vec<&ChargeSample> = kept
        .iter()
        .filter_map(|index| samples.get(*index))
        .collect();
    let axis = time_axis(charge.start_at, tz);
    let t: Vec<f64> = sampled
        .iter()
        .map(|sample| format::seconds_since(sample.0, charge.start_at))
        .collect();
    let power: Vec<Option<f64>> = sampled.iter().map(|sample| sample.1).collect();
    let soc: Vec<Option<f64>> = sampled.iter().map(|sample| sample.2).collect();
    let voltage: Vec<Option<f64>> = sampled.iter().map(|sample| sample.3).collect();
    let curve: Vec<(f64, f64)> = sampled
        .iter()
        .filter_map(|sample| Some((sample.2?, sample.1?)))
        .collect();
    let curve_axis = Axis::Number {
        unit: "%".to_owned(),
        label: "Batterie".to_owned(),
        decimals: 0,
    };
    let curve_t: Vec<f64> = curve.iter().map(|point| point.0).collect();
    let sync = format!("charge-{charge_id}");
    let place = Place::from(charge.address.as_deref(), charge.geofence.as_deref());
    let title = Place::name_or_unknown(place.as_ref());
    let dc = charge.is_dc == Some(true);
    let duration = elapsed(charge.start_at, charge.end_at, charge.duration_s);
    let average_kw = match (charge.energy_added_kwh, duration) {
        (Some(kwh), Some(secs)) if secs > 0 => Some(kwh / (format::seconds_f64(secs) / 3600.0)),
        _ => None,
    };
    let mut metrics = vec![
        (
            "Énergie ajoutée",
            charge.energy_added_kwh.map_or_else(
                || "—".to_owned(),
                |kwh| format!("+{}", format::kwh(Some(kwh))),
            ),
        ),
        ("Durée", format::duration(duration)),
        ("Puissance max", format::unit(charge.max_power_kw, 0, "kW")),
        ("Puissance moyenne", format::unit(average_kw, 1, "kW")),
        (
            "Type",
            charge_kind(charge.is_dc, charge.charger_type.as_deref()),
        ),
    ];
    if let (Some(start), Some(end)) = (charge.start_rated_range_km, charge.end_rated_range_km) {
        metrics.push((
            "Autonomie gagnée",
            format!("+{}", format::km(Some(end - start))),
        ));
    }
    if charge.energy_used_kwh.is_some() {
        metrics.push(("Énergie consommée", format::kwh(charge.energy_used_kwh)));
    }
    if let (Some(added), Some(used)) = (charge.energy_added_kwh, charge.energy_used_kwh)
        && used > 0.0
    {
        metrics.push(("Rendement", format::percent(Some(added / used * 100.0))));
    }
    metrics.push(("Coût", format::money(charge.cost)));
    if let (Some(cost), Some(added)) = (charge.cost, charge.energy_added_kwh)
        && added > 0.0
    {
        metrics.push(("Prix du kWh", format::unit(Some(cost / added), 3, "€")));
    }
    if let Some(outside) = charge.outside_temp_avg_c {
        metrics.push((
            "Extérieur",
            format!("{}{}°C", format::number(outside, 1), format::NNBSP),
        ));
    }
    let when = match charge.end_at {
        Some(end) => format!(
            "{} – {}",
            format::moment(charge.start_at, tz),
            format::clock(end, tz)
        ),
        None => format!("{} · en cours", format::moment(charge.start_at, tz)),
    };
    let chrome = chrome(
        &page,
        Some(id),
        Some(VehicleTab::Charges),
        GlobalTab::None,
        false,
    );
    let csrf = &page.csrf;
    let body = html! {
        (toolbar(&chrome, &title))
        (vehicle_tabs(&chrome))
        div class="detail" role="tabpanel" aria-labelledby="tab-charges" {
            div class="detail-head" {
                a class="text-link back-inline" href=(format!("/v/{id}/charges")) { "← Retour à la liste" }
                p class="eyebrow" {
                    (when)
                    @if dc { " · " span class="chip dc" { "DC" } }
                }
                @if let Some(detail) = place.as_ref().and_then(|place| place.detail.as_ref()) {
                    p class="muted" { (detail) }
                }
            }
            section class="card charge-hero" aria-labelledby="charge-metrics" {
                h2 id="charge-metrics" class="visually-hidden" { "Mesures" }
                div class="charge-soc" {
                    span class="hero-figure" { (battery_span(charge.start_battery_level, charge.end_battery_level)) }
                    (viz::soc_bar(charge.start_battery_level, charge.end_battery_level))
                }
                dl class="metrics" {
                    @for (label, value) in &metrics {
                        div { dt { (label) } dd { (value) } }
                    }
                }
                @if let Some(note) = &charge.note {
                    p class="note" { (note) }
                }
            }
            section class="card charts" aria-labelledby="charge-charts" data-sync-root=(sync) {
                div class="card-head" {
                    h2 id="charge-charts" { "Courbes" }
                    p class="hint chart-hint" { "Survolez pour lire, faites glisser pour zoomer." }
                }
                @if sampled.len() < 2 {
                    p class="muted" { "Pas assez de points." }
                } @else {
                    @if curve.len() >= 2 {
                        (viz::line_figure(
                            "Courbe de charge",
                            "Puissance selon le niveau de batterie",
                            None,
                            &LineChart {
                                kind: "line",
                                x: &curve_axis,
                                t: &curve_t,
                                series: vec![series("Puissance", "kW", 1, "green", curve.iter().map(|point| Some(point.1)).collect(), true, false)],
                            },
                        ))
                    }
                    (viz::line_figure("Puissance", &power_summary(&power), Some(&sync), &LineChart { kind: "line", x: &axis, t: &t, series: vec![series("Puissance", "kW", 1, "green", power, true, false)] }))
                    (viz::line_figure("Niveau de batterie", &range_summary(&soc, "%", 0), Some(&sync), &LineChart { kind: "line", x: &axis, t: &t, series: vec![series("Batterie", "%", 0, "accent", soc, true, false)] }))
                    @if voltage.iter().filter(|value| value.is_some()).count() >= 2 {
                        (viz::line_figure("Tension", &range_summary(&voltage, "V", 0), Some(&sync), &LineChart { kind: "line", x: &axis, t: &t, series: vec![series("Tension", "V", 0, "amber", voltage, false, false)] }))
                    }
                }
            }
            details class="card edit" {
                summary { "Modifier cette charge" }
                div class="edit-grid" {
                    form class="stack" method="post" action=(format!("/v/{id}/charges/{charge_id}/edit")) {
                        h3 { "Détails" }
                        input type="hidden" name="csrf" value=(csrf);
                        label class="field" for="cost" { "Coût" input id="cost" name="cost" inputmode="decimal" value=[charge.cost.map(|cost| format!("{cost:.2}"))]; }
                        label class="field" for="is_dc" {
                            "Courant"
                            select id="is_dc" name="is_dc" {
                                option value="" selected[charge.is_dc.is_none()] { "Inconnu" }
                                option value="ac" selected[charge.is_dc == Some(false)] { "AC" }
                                option value="dc" selected[charge.is_dc == Some(true)] { "DC" }
                            }
                        }
                        label class="field" for="charger_type" { "Borne" input id="charger_type" name="charger_type" value=[charge.charger_type.as_deref()]; }
                        label class="field" for="geofence_id" { "Géofence (id, 0 pour vider)" input id="geofence_id" name="geofence_id" inputmode="numeric"; }
                        label class="field" for="charge-note" { "Note" input id="charge-note" name="note" value=[charge.note.as_deref()]; }
                        button type="submit" { "Enregistrer" }
                    }
                    div class="stack danger-zone" {
                        h3 { "Zone sensible" }
                        p class="hint" { "Charge n° " (charge.id) " · source " (charge.source) }
                        form method="post" action=(format!("/v/{id}/charges/{charge_id}/trash")) {
                            input type="hidden" name="csrf" value=(csrf);
                            button class="danger" type="submit" { "Mettre à la corbeille" }
                        }
                    }
                }
            }
        }
        script src="/static/charts.js" defer {}
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

const DRIVE_FILTER: &str = "d.vehicle_id = $1
           AND d.deleted_at IS NULL
           AND ($2::timestamptz IS NULL OR d.start_at >= $2)
           AND ($3::timestamptz IS NULL OR d.start_at < $3)
           AND ($4::text IS NULL OR d.source = $4)
           AND ($5::float8 IS NULL OR COALESCE(d.distance_km, 0) >= $5)
           AND ($6::int IS NULL OR COALESCE(d.duration_s, 0) >= $6)
           AND ($7::text IS NULL OR (
                ($7 = 'zero_km' AND COALESCE(d.distance_km, 0) = 0)
             OR ($7 = 'no_energy' AND COALESCE(d.energy_used_kwh, 0) = 0)
             OR ($7 = 'open' AND d.end_at IS NULL AND d.start_at < now() - interval '24 hours')))";

const CHARGE_FILTER: &str = "c.vehicle_id = $1
           AND c.deleted_at IS NULL
           AND ($2::timestamptz IS NULL OR c.start_at >= $2)
           AND ($3::timestamptz IS NULL OR c.start_at < $3)
           AND ($4::text IS NULL OR c.source = $4)
           AND ($5::float8 IS NULL OR COALESCE(c.energy_added_kwh, 0) >= $5)
           AND ($6::int IS NULL OR COALESCE(c.duration_s, 0) >= $6)
           AND ($7::text IS NULL OR (
                ($7 = 'zero_km' AND COALESCE(c.energy_added_kwh, 0) = 0)
             OR ($7 = 'no_energy' AND COALESCE(c.energy_used_kwh, 0) = 0)
             OR ($7 = 'open' AND c.end_at IS NULL AND c.start_at < now() - interval '24 hours')))";

async fn drive_page(
    app: &AppState,
    vehicle_id: i64,
    bounds: &Bounds<'_>,
    limit: i64,
) -> Result<Vec<DriveRow>, sqlx::Error> {
    let sql = match bounds.sort {
        Sort::Start => drive_sql(
            "($8::timestamptz IS NULL OR (d.start_at, d.id) < ($8, $9))",
            "d.start_at DESC, d.id DESC",
        ),
        Sort::Distance => drive_sql(
            "($8::float8 IS NULL OR (COALESCE(d.distance_km, -1e308), d.id) < ($8, $9))",
            "d.distance_km DESC NULLS LAST, d.id DESC",
        ),
        Sort::Duration => drive_sql(
            "($8::float8 IS NULL OR (COALESCE(d.duration_s::float8, -1), d.id) < ($8, $9))",
            "d.duration_s DESC NULLS LAST, d.id DESC",
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
    let energy = super::stats::drive_energy_sql("d", "v");
    format!(
        "SELECT d.id, d.start_at, d.end_at, d.distance_km, d.duration_s, d.energy_used_kwh, d.source,
                {energy} AS energy_kwh, d.start_battery_level, d.end_battery_level,
                start_addr.display_name AS start_address,
                end_addr.display_name AS end_address,
                start_fence.name AS start_geofence,
                end_fence.name AS end_geofence
         FROM drives d
         JOIN vehicles v ON v.id = d.vehicle_id
         LEFT JOIN addresses start_addr ON start_addr.id = d.start_address_id
         LEFT JOIN addresses end_addr ON end_addr.id = d.end_address_id
         LEFT JOIN geofences start_fence ON start_fence.id = d.start_geofence_id
         LEFT JOIN geofences end_fence ON end_fence.id = d.end_geofence_id
         WHERE {DRIVE_FILTER}
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
            "($8::timestamptz IS NULL OR (c.start_at, c.id) < ($8, $9))",
            "c.start_at DESC, c.id DESC",
        ),
        Sort::Distance => charge_sql(
            "($8::float8 IS NULL OR (COALESCE(c.energy_added_kwh, -1e308), c.id) < ($8, $9))",
            "c.energy_added_kwh DESC NULLS LAST, c.id DESC",
        ),
        Sort::Duration => charge_sql(
            "($8::float8 IS NULL OR (COALESCE(c.duration_s::float8, -1), c.id) < ($8, $9))",
            "c.duration_s DESC NULLS LAST, c.id DESC",
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
        "SELECT c.id, c.start_at, c.end_at, c.energy_added_kwh, c.energy_used_kwh, c.duration_s,
                c.cost, c.is_dc, c.charger_type, c.max_power_kw,
                c.start_battery_level, c.end_battery_level, c.source,
                addr.display_name AS address,
                fence.name AS geofence
         FROM charges c
         LEFT JOIN addresses addr ON addr.id = c.address_id
         LEFT JOIN geofences fence ON fence.id = c.geofence_id
         WHERE {CHARGE_FILTER}
           AND {cursor}
         ORDER BY {order}
         LIMIT $10"
    )
}

type DriveTotals = (i64, Option<f64>, Option<i64>, Option<f64>);
type ChargeTotals = (i64, Option<f64>, Option<i64>, Option<f64>, Option<f64>);

/// Aggregates over the whole filtered range, ignoring the page cursor.
async fn drive_totals(
    app: &AppState,
    vehicle_id: i64,
    bounds: &Bounds<'_>,
) -> Vec<(&'static str, String)> {
    let energy = super::stats::drive_energy_sql("d", "v");
    let row: Option<DriveTotals> = sqlx::query_as(&format!(
        "SELECT COUNT(*), SUM(d.distance_km), SUM(d.duration_s)::bigint, SUM({energy})
         FROM drives d JOIN vehicles v ON v.id = d.vehicle_id
         WHERE {DRIVE_FILTER}"
    ))
    .bind(vehicle_id)
    .bind(bounds.from)
    .bind(bounds.to)
    .bind(bounds.source)
    .bind(bounds.min_km)
    .bind(bounds.min_s)
    .bind(bounds.suspect)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten();
    let Some((count, distance, seconds, energy)) = row else {
        return Vec::new();
    };
    vec![
        ("Trajets", format::count(count)),
        ("Distance", format::km(Some(distance.unwrap_or(0.0)))),
        ("Au volant", format::duration(Some(seconds.unwrap_or(0)))),
        ("Énergie", format::kwh(energy)),
        (
            "Efficacité",
            format::unit(format::efficiency(energy, distance), 0, "Wh/km"),
        ),
    ]
}

async fn charge_totals(
    app: &AppState,
    vehicle_id: i64,
    bounds: &Bounds<'_>,
) -> Vec<(&'static str, String)> {
    let row: Option<ChargeTotals> = sqlx::query_as(&format!(
        "SELECT COUNT(*), SUM(c.energy_added_kwh), SUM(c.duration_s)::bigint, SUM(c.cost),
                    SUM(c.energy_added_kwh) FILTER (WHERE c.is_dc)
             FROM charges c
             WHERE {CHARGE_FILTER}"
    ))
    .bind(vehicle_id)
    .bind(bounds.from)
    .bind(bounds.to)
    .bind(bounds.source)
    .bind(bounds.min_km)
    .bind(bounds.min_s)
    .bind(bounds.suspect)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten();
    let Some((count, energy, seconds, cost, dc)) = row else {
        return Vec::new();
    };
    let share = match (energy, dc) {
        (Some(total), Some(dc)) if total > 0.0 => Some(dc / total * 100.0),
        (Some(total), None) if total > 0.0 => Some(0.0),
        _ => None,
    };
    vec![
        ("Charges", format::count(count)),
        ("Énergie ajoutée", format::kwh(Some(energy.unwrap_or(0.0)))),
        ("Branchée", format::duration(Some(seconds.unwrap_or(0)))),
        ("Coût", format::money(cost)),
        ("Part DC", format::percent(share)),
    ]
}

fn summary_strip(totals: &[(&'static str, String)]) -> Markup {
    html! {
        @if !totals.is_empty() {
            dl class="metrics summary" aria-label="Totaux de la sélection" {
                @for (label, value) in totals {
                    div { dt { (label) } dd { (value) } }
                }
            }
        }
    }
}

fn sort_name(sort: Sort) -> &'static str {
    match sort {
        Sort::Start => "start",
        Sort::Distance => "distance",
        Sort::Duration => "duration",
    }
}

/// Filter parameters of the current list, without the page cursor.
fn filter_pairs(query: &ListQuery) -> Vec<(&'static str, String)> {
    let mut pairs = Vec::new();
    let mut push = |key: &'static str, value: Option<String>| {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            pairs.push((key, value));
        }
    };
    push("from", query.from.clone());
    push("to", query.to.clone());
    push("source", query.source.clone());
    push("min_km", query.min_km.map(|value| value.to_string()));
    push("min_s", query.min_s.map(|value| value.to_string()));
    push("suspect", query.suspect.clone());
    push(
        "sort",
        query.sort.clone().filter(|sort| sort.as_str() != "start"),
    );
    pairs
}

fn encode(pairs: &[(&str, String)]) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in pairs {
        serializer.append_pair(key, value);
    }
    serializer.finish()
}

fn page_href(
    id: i64,
    kind: Kind,
    query: &ListQuery,
    sort: Sort,
    before: &str,
    last: i64,
) -> String {
    let mut pairs: Vec<(&str, String)> = filter_pairs(query)
        .into_iter()
        .filter(|(key, _)| *key != "sort")
        .collect();
    pairs.push(("sort", sort_name(sort).to_owned()));
    pairs.push(("before", before.to_owned()));
    pairs.push(("before_id", last.to_string()));
    format!("/v/{id}/{}?{}", kind.path(), encode(&pairs))
}

fn list_href(id: i64, kind: Kind, pairs: &[(&str, String)]) -> String {
    if pairs.is_empty() {
        format!("/v/{id}/{}", kind.path())
    } else {
        format!("/v/{id}/{}?{}", kind.path(), encode(pairs))
    }
}

fn pager(id: i64, kind: Kind, query: &ListQuery, next: Option<&str>) -> Markup {
    let paged = query.before.is_some();
    html! {
        @if paged || next.is_some() {
            nav class="pager" aria-label="Pagination" {
                @if paged {
                    a class="button ghost" href=(list_href(id, kind, &filter_pairs(query))) { "← Les plus récents" }
                }
                @if let Some(href) = next {
                    a class="button ghost" href=(href) rel="next" { "Plus anciens →" }
                }
            }
        }
    }
}

fn list_toolbar(id: i64, kind: Kind, query: &ListQuery, tz: Tz) -> Markup {
    let today = format::local_date(Utc::now(), tz);
    let ranges = [
        ("7 jours", today - chrono::Duration::days(6)),
        ("30 jours", today - chrono::Duration::days(29)),
        (
            "Cette année",
            NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap_or(today),
        ),
    ];
    let quick = query.to.is_none()
        && query.source.is_none()
        && query.min_km.is_none()
        && query.min_s.is_none()
        && query.suspect.as_deref().unwrap_or("").is_empty();
    let current_from = query.from.as_deref().unwrap_or("");
    let advanced = query.to.as_deref().is_some_and(|to| !to.is_empty())
        || query
            .source
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        || query.min_km.is_some()
        || query.min_s.is_some()
        || query
            .suspect
            .as_deref()
            .is_some_and(|value| !value.is_empty())
        || query.sort.as_deref().is_some_and(|sort| sort != "start");
    let sort = query.sort.as_deref().unwrap_or("start");
    let source = query.source.as_deref().unwrap_or("");
    let suspect = query.suspect.as_deref().unwrap_or("");
    let min_label = match kind {
        Kind::Drives => "Distance min (km)",
        Kind::Charges => "Énergie min (kWh)",
    };
    let size_label = match kind {
        Kind::Drives => "distance",
        Kind::Charges => "énergie",
    };
    let mut csv_pairs = filter_pairs(query);
    csv_pairs.retain(|(key, _)| *key != "sort");
    let csv = if csv_pairs.is_empty() {
        format!("/v/{id}/{}.csv", kind.path())
    } else {
        format!("/v/{id}/{}.csv?{}", kind.path(), encode(&csv_pairs))
    };
    html! {
        div class="list-toolbar" {
            nav class="segmented" aria-label="Période" {
                @for (label, from) in ranges {
                    @let value = from.format("%Y-%m-%d").to_string();
                    @if quick && current_from == value {
                        a href=(list_href(id, kind, &[("from", value.clone())])) aria-current="page" { (label) }
                    } @else {
                        a href=(list_href(id, kind, &[("from", value.clone())])) { (label) }
                    }
                }
                @if quick && current_from.is_empty() {
                    a href=(list_href(id, kind, &[])) aria-current="page" { "Tout" }
                } @else {
                    a href=(list_href(id, kind, &[])) { "Tout" }
                }
            }
            a class="button ghost small" href=(csv) download { "Exporter en CSV" }
        }
        details class="filters-panel" open[advanced] {
            summary { "Filtres et tri" }
            form class="filters" method="get" action=(format!("/v/{id}/{}", kind.path())) {
                label class="field" for="from" { "Du" input id="from" name="from" type="date" value=(date_value(query.from.as_deref())); }
                label class="field" for="to" { "Au" input id="to" name="to" type="date" value=(date_value(query.to.as_deref())); }
                label class="field" for="source" {
                    "Source"
                    select id="source" name="source" {
                        @for (value, label) in [("", "toutes"), ("poll", "poll"), ("stream", "stream"), ("teslamate", "teslamate"), ("manual", "manual")] {
                            option value=(value) selected[source == value] { (label) }
                        }
                    }
                }
                label class="field" for="min_km" { (min_label) input id="min_km" name="min_km" inputmode="decimal" value=(query.min_km.map(|value| value.to_string()).unwrap_or_default()); }
                label class="field" for="min_s" { "Durée min (s)" input id="min_s" name="min_s" inputmode="numeric" value=(query.min_s.map(|value| value.to_string()).unwrap_or_default()); }
                label class="field" for="suspect" {
                    "Suspects"
                    select id="suspect" name="suspect" {
                        @for (value, label) in [("", "tous"), ("zero_km", if kind == Kind::Drives { "0 km" } else { "0 kWh" }), ("no_energy", "sans énergie"), ("open", "ouverts > 24 h")] {
                            option value=(value) selected[suspect == value] { (label) }
                        }
                    }
                }
                label class="field" for="sort" {
                    "Tri"
                    select id="sort" name="sort" {
                        option value="start" selected[sort == "start"] { "date" }
                        option value="distance" selected[sort == "distance"] { (size_label) }
                        option value="duration" selected[sort == "duration"] { "durée" }
                    }
                }
                div class="filters-actions" {
                    button type="submit" { "Appliquer" }
                    a class="button ghost" href=(format!("/v/{id}/{}", kind.path())) { "Réinitialiser" }
                }
            }
        }
    }
}

/// `YYYY-MM-DD` for a date input, from either a date or an RFC 3339 value.
fn date_value(value: Option<&str>) -> String {
    let Some(value) = value else {
        return String::new();
    };
    parse_time(value).map_or_else(String::new, |time| time.format("%Y-%m-%d").to_string())
}

fn fmt_num(value: Option<f64>) -> String {
    value.map_or_else(String::new, |value| format!("{value:.3}"))
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
        assert!(first.contains("datetime=\"2024-06-01T11:00:00+00:00\""));
        assert!(first.contains("datetime=\"2024-06-01T10:00:00+00:00\""));
        assert!(first.contains("Samedi 1 juin 2024"));
        assert!(first.contains("<main class=\"shell\">"));
        let zero = get(
            &state,
            &format!("/v/{}/drives?suspect=zero_km", vehicle.0),
            &cookie,
        )
        .await;
        assert!(zero.contains("datetime=\"2024-06-01T10:00:00+00:00\""));
        assert!(!zero.contains("datetime=\"2024-06-01T11:00:00+00:00\""));
        assert!(zero.contains("<option value=\"zero_km\" selected>"));
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
             VALUES ($1, '2024-06-01T11:01:00Z', 0.011, 0.021, 30, $2, 'poll'),
                    ($1, '2024-06-01T11:02:00Z', 0.012, 0.022, 40, $2, 'poll')",
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
        assert!(detail.contains("class=\"map route-map\""));
        assert!(detail.contains("data-chart="));
        assert!(detail.contains("/static/charts.js"));
        assert!(detail.contains("Retour à la liste"));
        assert!(!detail.contains("0.011"), "coordinates leaked");
        assert!(!detail.contains("0.021"), "coordinates leaked");
    }

    #[tokio::test]
    async fn next_page_link_keeps_the_filters() {
        let query = super::ListQuery {
            source: Some("poll".to_owned()),
            min_km: Some(2.0),
            ..super::ListQuery::default()
        };
        let href = super::page_href(
            7,
            super::Kind::Drives,
            &query,
            super::Sort::Start,
            "2024-06-01T11:00:00+00:00",
            9,
        );
        assert!(href.starts_with("/v/7/drives?"), "{href}");
        assert!(href.contains("source=poll"), "{href}");
        assert!(href.contains("min_km=2"), "{href}");
        assert!(
            href.contains("before=2024-06-01T11%3A00%3A00%2B00%3A00"),
            "{href}"
        );
        assert!(href.contains("before_id=9"), "{href}");
    }

    #[tokio::test]
    async fn console_lists_show_fixture_address_names() {
        let Some(db) = crate::test_util::TestDb::migrated("tripaddr").await else {
            return;
        };
        let state = crate::app::test_state(db.pool.clone()).await;
        let vehicle: (i64,) = sqlx::query_as(
            "INSERT INTO vehicles (vin, name) VALUES ('5YJ30000000000A01', 'Lieux') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let origin: (i64,) = sqlx::query_as(
            "INSERT INTO addresses (latitude, longitude, display_name)
             VALUES (0.0, 0.0, 'Fixture Origin') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let bay: (i64,) = sqlx::query_as(
            "INSERT INTO geofences (name, latitude, longitude, radius_m)
             VALUES ('Fixture Bay', 0.0, 0.0, 10) RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let drive: (i64,) = sqlx::query_as(
            "INSERT INTO drives (
                vehicle_id, start_at, end_at, distance_km, duration_s, source,
                start_address_id, start_geofence_id
             ) VALUES ($1, '2024-07-02T08:00:00Z', '2024-07-02T08:10:00Z', 2, 600, 'poll', $2, $3)
             RETURNING id",
        )
        .bind(vehicle.0)
        .bind(origin.0)
        .bind(bay.0)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let charge: (i64,) = sqlx::query_as(
            "INSERT INTO charges (vehicle_id, start_at, end_at, energy_added_kwh, source, geofence_id)
             VALUES ($1, '2024-07-02T09:00:00Z', '2024-07-02T09:20:00Z', 5, 'poll', $2)
             RETURNING id",
        )
        .bind(vehicle.0)
        .bind(bay.0)
        .fetch_one(&db.pool)
        .await
        .unwrap();
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
        let drives = get(&state, &format!("/v/{}/drives", vehicle.0), &cookie).await;
        assert!(drives.contains(">Fixture Bay<"));
        assert!(drives.contains("title=\"Fixture Origin\""));
        let detail = get(
            &state,
            &format!("/v/{}/drives/{}", vehicle.0, drive.0),
            &cookie,
        )
        .await;
        assert!(detail.contains("Fixture Bay"));
        assert!(detail.contains("Fixture Origin"));
        let charges = get(&state, &format!("/v/{}/charges", vehicle.0), &cookie).await;
        assert!(charges.contains("Fixture Bay"));
        let charge_page = get(
            &state,
            &format!("/v/{}/charges/{}", vehicle.0, charge.0),
            &cookie,
        )
        .await;
        assert!(charge_page.contains("Fixture Bay"));
        assert!(charge_page.contains(&format!(
            "action=\"/v/{}/charges/{}/edit\"",
            vehicle.0, charge.0
        )));
        assert!(charge_page.contains(&format!(
            "action=\"/v/{}/charges/{}/trash\"",
            vehicle.0, charge.0
        )));
    }

    #[test]
    fn missing_place_is_unknown() {
        assert!(super::Place::from(None, None).is_none());
        assert_eq!(super::Place::name_or_unknown(None), "Lieu inconnu");
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
