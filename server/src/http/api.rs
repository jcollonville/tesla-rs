// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use axum::Json;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::ApiError;
use crate::app::AppState;
use crate::error::redact_secrets;
use crate::rdp::{rdp_limit, stride_indices};

pub(crate) async fn list_vehicles(
    State(app): State<AppState>,
) -> Result<Json<Vec<VehicleJson>>, ApiError> {
    let rows = sqlx::query_as::<_, VehicleJson>(
        "SELECT v.id, v.vin, v.name, v.model, v.exterior_color, s.activity,
                s.battery_level, s.rated_range_km, s.odometer_km, s.latitude, s.longitude,
                s.speed_kmh, s.charger_power_kw, s.last_poll_at, s.last_error
         FROM vehicles v
         LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
         ORDER BY v.id",
    )
    .fetch_all(&app.inner.db)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

pub(crate) async fn vehicle(
    State(app): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<VehicleJson>, ApiError> {
    let row = sqlx::query_as::<_, VehicleJson>(
        "SELECT v.id, v.vin, v.name, v.model, v.exterior_color, s.activity,
                s.battery_level, s.rated_range_km, s.odometer_km, s.latitude, s.longitude,
                s.speed_kmh, s.charger_power_kw, s.last_poll_at, s.last_error
         FROM vehicles v
         LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
         WHERE v.id = $1",
    )
    .bind(id)
    .fetch_optional(&app.inner.db)
    .await
    .map_err(internal)?
    .ok_or(ApiError::NotFound)?;
    Ok(Json(row))
}

pub(crate) async fn vehicle_state(
    State(app): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<StateJson>, ApiError> {
    let exists: Option<(i64,)> = sqlx::query_as("SELECT id FROM vehicles WHERE id = $1")
        .bind(id)
        .fetch_optional(&app.inner.db)
        .await
        .map_err(internal)?;
    if exists.is_none() {
        return Err(ApiError::NotFound);
    }
    let sql = format!(
        "SELECT vehicle_id, activity, {columns} FROM vehicle_state WHERE vehicle_id = $1",
        columns = STATE_FIELDS.join(", ")
    );
    let row = sqlx::query_as::<_, StateJson>(&sql)
        .bind(id)
        .fetch_optional(&app.inner.db)
        .await
        .map_err(internal)?;
    Ok(Json(row.unwrap_or_else(|| StateJson::offline(id))))
}

pub(crate) async fn list_drives(
    State(app): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Page<DriveJson>>, ApiError> {
    let page = page_params(query.page, query.per_page)?;
    let total: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM drives
         WHERE ($1::bigint IS NULL OR vehicle_id = $1)
           AND ($2::timestamptz IS NULL OR start_at >= $2)
           AND ($3::timestamptz IS NULL OR start_at < $3)",
    )
    .bind(query.vehicle_id)
    .bind(query.from)
    .bind(query.to)
    .fetch_one(&app.inner.db)
    .await
    .map_err(internal)?;
    let data = sqlx::query_as::<_, DriveJson>(&drive_select(
        "WHERE ($1::bigint IS NULL OR vehicle_id = $1)
           AND ($2::timestamptz IS NULL OR start_at >= $2)
           AND ($3::timestamptz IS NULL OR start_at < $3)
         ORDER BY start_at DESC
         LIMIT $4 OFFSET $5",
    ))
    .bind(query.vehicle_id)
    .bind(query.from)
    .bind(query.to)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&app.inner.db)
    .await
    .map_err(internal)?;
    Ok(Json(Page {
        data,
        page: page.page,
        per_page: page.per_page,
        total: total.0,
    }))
}

pub(crate) async fn drive_detail(
    State(app): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<DetailQuery>,
) -> Result<Json<DriveDetail>, ApiError> {
    let max_points = max_points(query.max_points)?;
    let drive = sqlx::query_as::<_, DriveJson>(&drive_select("WHERE id = $1"))
        .bind(id)
        .fetch_optional(&app.inner.db)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    let points = sqlx::query_as::<_, PointRow>(
        "SELECT recorded_at, latitude, longitude, speed_kmh, power_kw, battery_level,
                elevation_m, rated_range_km, odometer_km
         FROM positions
         WHERE drive_id = $1
           AND ($2::timestamptz IS NULL OR recorded_at >= $2)
           AND ($3::timestamptz IS NULL OR recorded_at <= $3)
         ORDER BY recorded_at",
    )
    .bind(id)
    .bind(drive.start_at)
    .bind(drive.end_at)
    .fetch_all(&app.inner.db)
    .await
    .map_err(internal)?;
    let coords: Vec<[f64; 2]> = points
        .iter()
        .filter_map(|point| Some([point.latitude?, point.longitude?]))
        .collect();
    let route_index = rdp_limit(&coords, max_points);
    let mut route = Vec::new();
    let mut coord_i = 0usize;
    for point in &points {
        if point.latitude.is_some() && point.longitude.is_some() {
            if route_index.binary_search(&coord_i).is_ok() {
                route.push(RoutePoint {
                    recorded_at: point.recorded_at,
                    latitude: point.latitude.unwrap_or(0.0),
                    longitude: point.longitude.unwrap_or(0.0),
                });
            }
            coord_i += 1;
        }
    }
    let series_index = stride_indices(points.len(), max_points);
    let series = series_index
        .into_iter()
        .filter_map(|index| points.get(index).map(PointRow::to_series))
        .collect();
    Ok(Json(DriveDetail {
        drive,
        route,
        series,
    }))
}

pub(crate) async fn list_charges(
    State(app): State<AppState>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Page<ChargeJson>>, ApiError> {
    let page = page_params(query.page, query.per_page)?;
    let total: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM charges
         WHERE ($1::bigint IS NULL OR vehicle_id = $1)
           AND ($2::timestamptz IS NULL OR start_at >= $2)
           AND ($3::timestamptz IS NULL OR start_at < $3)",
    )
    .bind(query.vehicle_id)
    .bind(query.from)
    .bind(query.to)
    .fetch_one(&app.inner.db)
    .await
    .map_err(internal)?;
    let data = sqlx::query_as::<_, ChargeJson>(&charge_select(
        "WHERE ($1::bigint IS NULL OR vehicle_id = $1)
           AND ($2::timestamptz IS NULL OR start_at >= $2)
           AND ($3::timestamptz IS NULL OR start_at < $3)
         ORDER BY start_at DESC LIMIT $4 OFFSET $5",
    ))
    .bind(query.vehicle_id)
    .bind(query.from)
    .bind(query.to)
    .bind(page.limit)
    .bind(page.offset)
    .fetch_all(&app.inner.db)
    .await
    .map_err(internal)?;
    Ok(Json(Page {
        data,
        page: page.page,
        per_page: page.per_page,
        total: total.0,
    }))
}

pub(crate) async fn charge_detail(
    State(app): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Json<ChargeDetail>, ApiError> {
    let charge = sqlx::query_as::<_, ChargeJson>(&charge_select("WHERE id = $1"))
        .bind(id)
        .fetch_optional(&app.inner.db)
        .await
        .map_err(internal)?
        .ok_or(ApiError::NotFound)?;
    let samples = sqlx::query_as::<_, ChargeSampleJson>(
        "SELECT recorded_at, battery_level, charger_power_kw, charger_voltage, charger_current,
                charge_energy_added_kwh, rated_range_km, ideal_range_km, outside_temp_c
         FROM charge_samples WHERE charge_id = $1 ORDER BY recorded_at",
    )
    .bind(id)
    .fetch_all(&app.inner.db)
    .await
    .map_err(internal)?;
    Ok(Json(ChargeDetail { charge, samples }))
}

pub(crate) async fn stats(
    State(app): State<AppState>,
    Query(query): Query<StatsQuery>,
) -> Result<Json<StatsJson>, ApiError> {
    let Some(vehicle_id) = query.vehicle_id else {
        return Err(ApiError::BadRequest("vehicle_id is required"));
    };
    let drives: (i64, f64, i64, f64) = sqlx::query_as(
        "SELECT COUNT(*)::bigint,
                COALESCE(SUM(distance_km), 0)::float8,
                COALESCE(SUM(duration_s), 0)::bigint,
                COALESCE(SUM(energy_used_kwh), 0)::float8
         FROM drives
         WHERE vehicle_id = $1 AND end_at IS NOT NULL
           AND ($2::timestamptz IS NULL OR start_at >= $2)
           AND ($3::timestamptz IS NULL OR start_at < $3)",
    )
    .bind(vehicle_id)
    .bind(query.from)
    .bind(query.to)
    .fetch_one(&app.inner.db)
    .await
    .map_err(internal)?;
    let charges: (i64, f64) = sqlx::query_as(
        "SELECT COUNT(*)::bigint, COALESCE(SUM(energy_added_kwh), 0)::float8
         FROM charges
         WHERE vehicle_id = $1 AND end_at IS NOT NULL
           AND ($2::timestamptz IS NULL OR start_at >= $2)
           AND ($3::timestamptz IS NULL OR start_at < $3)",
    )
    .bind(vehicle_id)
    .bind(query.from)
    .bind(query.to)
    .fetch_one(&app.inner.db)
    .await
    .map_err(internal)?;
    Ok(Json(StatsJson {
        vehicle_id,
        from: query.from,
        to: query.to,
        drive_count: drives.0,
        distance_km: drives.1,
        drive_duration_s: drives.2,
        energy_used_kwh: drives.3,
        charge_count: charges.0,
        energy_added_kwh: charges.1,
    }))
}

pub(crate) async fn geofences(
    State(app): State<AppState>,
) -> Result<Json<Vec<GeofenceJson>>, ApiError> {
    let rows = sqlx::query_as::<_, GeofenceJson>(
        "SELECT id, name, latitude, longitude, radius_m, billing_type, cost_per_unit, session_fee
         FROM geofences ORDER BY id",
    )
    .fetch_all(&app.inner.db)
    .await
    .map_err(internal)?;
    Ok(Json(rows))
}

pub(crate) async fn ws(
    State(app): State<AppState>,
    upgrade: WebSocketUpgrade,
) -> impl IntoResponse {
    let events = app.inner.events.subscribe();
    let db = app.inner.db.clone();
    upgrade.on_upgrade(move |socket| handle_socket(socket, db, events))
}

async fn handle_socket(
    mut socket: WebSocket,
    db: sqlx::PgPool,
    mut events: tokio::sync::broadcast::Receiver<crate::persist::LiveEvent>,
) {
    let snapshot = format!(
        "SELECT v.vin, v.name, v.model, v.id AS vehicle_id,
                COALESCE(s.activity, 'offline') AS activity, {prefixed}
         FROM vehicles v
         LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
         ORDER BY v.id",
        prefixed = state_columns("s")
    );
    if let Ok(rows) = sqlx::query_as::<_, WsSnapshot>(&snapshot)
        .fetch_all(&db)
        .await
    {
        for row in rows {
            let payload = serde_json::json!({
                "type": "state",
                "vin": row.vin,
                "name": row.name,
                "model": row.model,
            });
            let Ok(mut value) = serde_json::to_value(payload) else {
                return;
            };
            let Ok(state) = serde_json::to_value(&row.state) else {
                return;
            };
            if let (Some(object), Some(state)) = (value.as_object_mut(), state.as_object()) {
                for (key, item) in state {
                    object.insert(key.clone(), item.clone());
                }
            }
            if socket
                .send(Message::Text(value.to_string().into()))
                .await
                .is_err()
            {
                return;
            }
        }
    }
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            event = events.recv() => {
                match event {
                    Ok(event) => {
                        if let Ok(text) = serde_json::to_string(&event)
                            && socket.send(Message::Text(text.into())).await.is_err()
                        {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

#[allow(clippy::needless_pass_by_value)]
fn internal(err: sqlx::Error) -> ApiError {
    tracing::error!(error = %redact_secrets(&err.to_string()), "query failed");
    ApiError::Internal
}

struct PageParams {
    page: i64,
    per_page: i64,
    limit: i64,
    offset: i64,
}

fn page_params(page: Option<i64>, per_page: Option<i64>) -> Result<PageParams, ApiError> {
    let page = page.unwrap_or(1);
    let per_page = per_page.unwrap_or(50);
    if page < 1 || per_page < 1 {
        return Err(ApiError::BadRequest("page and per_page must be positive"));
    }
    let per_page = per_page.min(100);
    Ok(PageParams {
        page,
        per_page,
        limit: per_page,
        offset: page.saturating_sub(1).saturating_mul(per_page),
    })
}

fn max_points(value: Option<i64>) -> Result<usize, ApiError> {
    let value = value.unwrap_or(300);
    if value < 2 {
        return Err(ApiError::BadRequest("max_points must be at least 2"));
    }
    Ok(usize::try_from(value.min(2_000)).unwrap_or(2_000))
}

fn drive_select(tail: &str) -> String {
    format!(
        "SELECT id, vehicle_id, start_at, end_at, distance_km, duration_s,
                start_battery_level, end_battery_level, start_rated_range_km, end_rated_range_km,
                energy_used_kwh, max_speed_kmh, avg_speed_kmh, max_power_kw, min_power_kw,
                elevation_gain_m, elevation_loss_m, inside_temp_avg_c, outside_temp_avg_c,
                start_latitude, start_longitude, end_latitude, end_longitude,
                start_address_id, end_address_id, start_geofence_id, end_geofence_id
         FROM drives {tail}"
    )
}

fn charge_select(tail: &str) -> String {
    format!(
        "SELECT id, vehicle_id, start_at, end_at, energy_added_kwh, energy_used_kwh,
                start_battery_level, end_battery_level, start_rated_range_km, end_rated_range_km,
                latitude, longitude, is_dc, charger_type, max_power_kw, duration_s, cost,
                outside_temp_avg_c, address_id, geofence_id
         FROM charges {tail}"
    )
}

#[derive(Deserialize)]
pub(crate) struct ListQuery {
    pub vehicle_id: Option<i64>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub page: Option<i64>,
    pub per_page: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct DetailQuery {
    pub max_points: Option<i64>,
}

#[derive(Deserialize)]
pub(crate) struct StatsQuery {
    pub vehicle_id: Option<i64>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}

#[derive(Serialize)]
pub(crate) struct Page<T> {
    pub data: Vec<T>,
    pub page: i64,
    pub per_page: i64,
    pub total: i64,
}

#[derive(sqlx::FromRow, Serialize)]
pub(crate) struct VehicleJson {
    pub id: i64,
    pub vin: String,
    pub name: Option<String>,
    pub model: Option<String>,
    pub exterior_color: Option<String>,
    pub activity: Option<String>,
    pub battery_level: Option<f64>,
    pub rated_range_km: Option<f64>,
    pub odometer_km: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub speed_kmh: Option<f64>,
    pub charger_power_kw: Option<f64>,
    pub last_poll_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

const STATE_FIELDS: &[&str] = &[
    "battery_level",
    "usable_battery_level",
    "rated_range_km",
    "ideal_range_km",
    "est_range_km",
    "odometer_km",
    "latitude",
    "longitude",
    "heading_deg",
    "speed_kmh",
    "power_kw",
    "inside_temp_c",
    "outside_temp_c",
    "climate_on",
    "locked",
    "sentry_mode",
    "charging_state",
    "charger_power_kw",
    "charge_energy_added_kwh",
    "charge_limit_soc",
    "shift_state",
    "is_dc",
    "software_version",
    "update_status",
    "last_poll_at",
    "last_data_at",
    "last_error",
    "updated_at",
    "charger_voltage",
    "charger_current",
    "charge_current_request",
    "charge_port_door_open",
    "charge_port_latch",
    "plugged_in",
    "time_to_full_charge_h",
    "scheduled_charging_start_time",
    "tpms_pressure_fl",
    "tpms_pressure_fr",
    "tpms_pressure_rl",
    "tpms_pressure_rr",
    "df",
    "dr",
    "pf",
    "pr",
    "fd_window",
    "fp_window",
    "rd_window",
    "rp_window",
    "ft",
    "rt",
    "is_user_present",
    "elevation_m",
    "battery_heater_on",
    "is_preconditioning",
];

fn state_columns(prefix: &str) -> String {
    STATE_FIELDS
        .iter()
        .map(|field| format!("{prefix}.{field}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(sqlx::FromRow, Serialize)]
pub(crate) struct StateJson {
    pub vehicle_id: i64,
    pub activity: String,
    pub battery_level: Option<f64>,
    pub usable_battery_level: Option<f64>,
    pub rated_range_km: Option<f64>,
    pub ideal_range_km: Option<f64>,
    pub est_range_km: Option<f64>,
    pub odometer_km: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub heading_deg: Option<f64>,
    pub speed_kmh: Option<f64>,
    pub power_kw: Option<f64>,
    pub inside_temp_c: Option<f64>,
    pub outside_temp_c: Option<f64>,
    pub climate_on: Option<bool>,
    pub locked: Option<bool>,
    pub sentry_mode: Option<bool>,
    pub charging_state: Option<String>,
    pub charger_power_kw: Option<f64>,
    pub charge_energy_added_kwh: Option<f64>,
    pub charge_limit_soc: Option<f64>,
    pub shift_state: Option<String>,
    pub is_dc: Option<bool>,
    pub software_version: Option<String>,
    pub update_status: Option<String>,
    pub last_poll_at: Option<DateTime<Utc>>,
    pub last_data_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub updated_at: Option<DateTime<Utc>>,
    pub charger_voltage: Option<f64>,
    pub charger_current: Option<f64>,
    pub charge_current_request: Option<f64>,
    pub charge_port_door_open: Option<bool>,
    pub charge_port_latch: Option<String>,
    pub plugged_in: Option<bool>,
    pub time_to_full_charge_h: Option<f64>,
    pub scheduled_charging_start_time: Option<i32>,
    pub tpms_pressure_fl: Option<f64>,
    pub tpms_pressure_fr: Option<f64>,
    pub tpms_pressure_rl: Option<f64>,
    pub tpms_pressure_rr: Option<f64>,
    pub df: Option<i32>,
    pub dr: Option<i32>,
    pub pf: Option<i32>,
    pub pr: Option<i32>,
    pub fd_window: Option<i32>,
    pub fp_window: Option<i32>,
    pub rd_window: Option<i32>,
    pub rp_window: Option<i32>,
    pub ft: Option<i32>,
    pub rt: Option<i32>,
    pub is_user_present: Option<bool>,
    pub elevation_m: Option<f64>,
    pub battery_heater_on: Option<bool>,
    pub is_preconditioning: Option<bool>,
}

impl StateJson {
    fn offline(vehicle_id: i64) -> Self {
        Self {
            vehicle_id,
            activity: "offline".to_owned(),
            battery_level: None,
            usable_battery_level: None,
            rated_range_km: None,
            ideal_range_km: None,
            est_range_km: None,
            odometer_km: None,
            latitude: None,
            longitude: None,
            heading_deg: None,
            speed_kmh: None,
            power_kw: None,
            inside_temp_c: None,
            outside_temp_c: None,
            climate_on: None,
            locked: None,
            sentry_mode: None,
            charging_state: None,
            charger_power_kw: None,
            charge_energy_added_kwh: None,
            charge_limit_soc: None,
            shift_state: None,
            is_dc: None,
            software_version: None,
            update_status: None,
            last_poll_at: None,
            last_data_at: None,
            last_error: None,
            updated_at: None,
            charger_voltage: None,
            charger_current: None,
            charge_current_request: None,
            charge_port_door_open: None,
            charge_port_latch: None,
            plugged_in: None,
            time_to_full_charge_h: None,
            scheduled_charging_start_time: None,
            tpms_pressure_fl: None,
            tpms_pressure_fr: None,
            tpms_pressure_rl: None,
            tpms_pressure_rr: None,
            df: None,
            dr: None,
            pf: None,
            pr: None,
            fd_window: None,
            fp_window: None,
            rd_window: None,
            rp_window: None,
            ft: None,
            rt: None,
            is_user_present: None,
            elevation_m: None,
            battery_heater_on: None,
            is_preconditioning: None,
        }
    }
}

#[cfg(test)]
pub(crate) async fn snapshot_rows(
    db: &sqlx::PgPool,
) -> Result<Vec<(String, String, Option<f64>, Option<i32>)>, sqlx::Error> {
    let snapshot = format!(
        "SELECT v.vin, v.name, v.model, v.id AS vehicle_id,
                COALESCE(s.activity, 'offline') AS activity, {prefixed}
         FROM vehicles v
         LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
         ORDER BY v.id",
        prefixed = state_columns("s")
    );
    let rows = sqlx::query_as::<_, WsSnapshot>(&snapshot)
        .fetch_all(db)
        .await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            (
                row.vin,
                row.state.activity,
                row.state.battery_level,
                row.state.df,
            )
        })
        .collect())
}

#[derive(sqlx::FromRow)]
struct WsSnapshot {
    vin: String,
    name: Option<String>,
    model: Option<String>,
    #[sqlx(flatten)]
    state: StateJson,
}

#[derive(sqlx::FromRow, Serialize)]
pub(crate) struct DriveJson {
    pub id: i64,
    pub vehicle_id: i64,
    pub start_at: DateTime<Utc>,
    pub end_at: Option<DateTime<Utc>>,
    pub distance_km: Option<f64>,
    pub duration_s: Option<i32>,
    pub start_battery_level: Option<f64>,
    pub end_battery_level: Option<f64>,
    pub start_rated_range_km: Option<f64>,
    pub end_rated_range_km: Option<f64>,
    pub energy_used_kwh: Option<f64>,
    pub max_speed_kmh: Option<f64>,
    pub avg_speed_kmh: Option<f64>,
    pub max_power_kw: Option<f64>,
    pub min_power_kw: Option<f64>,
    pub elevation_gain_m: Option<f64>,
    pub elevation_loss_m: Option<f64>,
    pub inside_temp_avg_c: Option<f64>,
    pub outside_temp_avg_c: Option<f64>,
    pub start_latitude: Option<f64>,
    pub start_longitude: Option<f64>,
    pub end_latitude: Option<f64>,
    pub end_longitude: Option<f64>,
    pub start_address_id: Option<i64>,
    pub end_address_id: Option<i64>,
    pub start_geofence_id: Option<i64>,
    pub end_geofence_id: Option<i64>,
}

#[derive(Serialize)]
pub(crate) struct DriveDetail {
    #[serde(flatten)]
    pub drive: DriveJson,
    pub route: Vec<RoutePoint>,
    pub series: Vec<SeriesPoint>,
}

#[derive(Serialize)]
pub(crate) struct RoutePoint {
    pub recorded_at: DateTime<Utc>,
    pub latitude: f64,
    pub longitude: f64,
}

#[derive(Serialize)]
pub(crate) struct SeriesPoint {
    pub recorded_at: DateTime<Utc>,
    pub speed_kmh: Option<f64>,
    pub power_kw: Option<f64>,
    pub battery_level: Option<f64>,
    pub elevation_m: Option<f64>,
    pub rated_range_km: Option<f64>,
    pub odometer_km: Option<f64>,
}

#[derive(sqlx::FromRow)]
struct PointRow {
    recorded_at: DateTime<Utc>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    speed_kmh: Option<f64>,
    power_kw: Option<f64>,
    battery_level: Option<f64>,
    elevation_m: Option<f64>,
    rated_range_km: Option<f64>,
    odometer_km: Option<f64>,
}

impl PointRow {
    fn to_series(&self) -> SeriesPoint {
        SeriesPoint {
            recorded_at: self.recorded_at,
            speed_kmh: self.speed_kmh,
            power_kw: self.power_kw,
            battery_level: self.battery_level,
            elevation_m: self.elevation_m,
            rated_range_km: self.rated_range_km,
            odometer_km: self.odometer_km,
        }
    }
}

#[derive(sqlx::FromRow, Serialize)]
pub(crate) struct ChargeJson {
    pub id: i64,
    pub vehicle_id: i64,
    pub start_at: DateTime<Utc>,
    pub end_at: Option<DateTime<Utc>>,
    pub energy_added_kwh: Option<f64>,
    pub energy_used_kwh: Option<f64>,
    pub start_battery_level: Option<f64>,
    pub end_battery_level: Option<f64>,
    pub start_rated_range_km: Option<f64>,
    pub end_rated_range_km: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub is_dc: Option<bool>,
    pub charger_type: Option<String>,
    pub max_power_kw: Option<f64>,
    pub duration_s: Option<i32>,
    pub cost: Option<f64>,
    pub outside_temp_avg_c: Option<f64>,
    pub address_id: Option<i64>,
    pub geofence_id: Option<i64>,
}

#[derive(Serialize)]
pub(crate) struct ChargeDetail {
    #[serde(flatten)]
    pub charge: ChargeJson,
    pub samples: Vec<ChargeSampleJson>,
}

#[derive(sqlx::FromRow, Serialize)]
pub(crate) struct ChargeSampleJson {
    pub recorded_at: DateTime<Utc>,
    pub battery_level: Option<f64>,
    pub charger_power_kw: Option<f64>,
    pub charger_voltage: Option<f64>,
    pub charger_current: Option<f64>,
    pub charge_energy_added_kwh: Option<f64>,
    pub rated_range_km: Option<f64>,
    pub ideal_range_km: Option<f64>,
    pub outside_temp_c: Option<f64>,
}

#[derive(Serialize)]
pub(crate) struct StatsJson {
    pub vehicle_id: i64,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub drive_count: i64,
    pub distance_km: f64,
    pub drive_duration_s: i64,
    pub energy_used_kwh: f64,
    pub charge_count: i64,
    pub energy_added_kwh: f64,
}

#[derive(sqlx::FromRow, Serialize)]
pub(crate) struct GeofenceJson {
    pub id: i64,
    pub name: String,
    pub latitude: f64,
    pub longitude: f64,
    pub radius_m: f64,
    pub billing_type: Option<String>,
    pub cost_per_unit: Option<f64>,
    pub session_fee: Option<f64>,
}
