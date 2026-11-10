// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Read-only time series for the Home Assistant charts.
//!
//! Buckets are calendar weeks (Monday), months, or years in [`crate::config::Config::timezone`].
//! Drive energy prefers `drives.energy_used_kwh` when it is already stored (native trips and
//! TeslaMate imports). Otherwise it uses the same basis as [`crate::state::range_energy`]:
//! rated range lost times vehicle efficiency, then ideal range lost times efficiency.
//!
//! The series statement does not read `positions` or `charge_samples`. Those tables are
//! range-partitioned by month (hundreds of partitions). Window bounds are timestamp
//! literals, computed before the query, so the planner sees constants. Rows are grouped
//! with `date_trunc` and joined to buckets on that key.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{
    DateTime, Datelike, FixedOffset, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeZone, Utc,
    Weekday,
};
use chrono_tz::Tz;
use serde::{Deserialize, Serialize};

use super::ApiError;
use crate::app::AppState;
use crate::error::redact_secrets;

const MAX_BUCKETS: i64 = 120;
const SERIES_CACHE_TTL: Duration = Duration::from_secs(5 * 60);

pub(crate) async fn series(
    State(app): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<PeriodQuery>,
) -> Result<Json<SeriesResponse>, ApiError> {
    if !super::vehicle_visible(id) {
        return Err(super::ApiError::NotFound);
    }
    let (period, count) = resolve_period(query.period.as_deref(), query.count)?;
    Ok(Json(load_series(&app, id, period, count).await?))
}

pub(crate) async fn load_series(
    app: &AppState,
    id: i64,
    period: &str,
    count: i32,
) -> Result<SeriesResponse, ApiError> {
    let timezone = crate::settings::text_or(
        &app.inner.db,
        Some(id),
        "TESLA_TIMEZONE",
        &app.inner.config.timezone,
    )
    .await;
    let price = crate::settings::f64_or(
        &app.inner.db,
        Some(id),
        "ENERGY_PRICE_PER_KWH",
        app.inner.config.energy_price_per_kwh,
    )
    .await;
    let window = stats_window(period, count, &timezone, Utc::now())?;
    let cache_key = SeriesCacheKey {
        state: std::sync::Arc::as_ptr(&app.inner) as usize,
        vehicle_id: id,
        period: period.to_owned(),
        count,
        timezone: timezone.clone(),
        price_bits: price.map(f64::to_bits),
        window_start_ms: window.start.timestamp_millis(),
    };
    if let Some(hit) = cache_get(&cache_key, Instant::now()) {
        return Ok(hit);
    }
    let rows = sqlx::query_as::<_, SeriesRow>(&series_query_sql(&window))
        .bind(id)
        .bind(price)
        .fetch_all(&app.inner.db)
        .await
        .map_err(map_query)?;
    let Some(first) = rows.first() else {
        return Err(ApiError::NotFound);
    };
    if !first.vehicle_exists {
        return Err(ApiError::NotFound);
    }
    let mut buckets = Vec::with_capacity(rows.len());
    for row in rows {
        buckets.push(row.into_bucket()?);
    }
    let response = SeriesResponse {
        period: period.to_owned(),
        buckets,
    };
    cache_insert(cache_key, response.clone(), Instant::now());
    Ok(response)
}

pub(crate) async fn consumption_by_temp(
    State(app): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<PeriodQuery>,
) -> Result<Json<Vec<TempBin>>, ApiError> {
    if !super::vehicle_visible(id) {
        return Err(super::ApiError::NotFound);
    }
    let (period, count) = resolve_period(query.period.as_deref(), query.count)?;
    let window = stats_window(period, count, &app.inner.config.timezone, Utc::now())?;
    let rows = sqlx::query_as::<_, TempRow>(&consumption_query_sql(&window))
        .bind(id)
        .fetch_all(&app.inner.db)
        .await
        .map_err(map_query)?;
    let Some(first) = rows.first() else {
        return Err(ApiError::NotFound);
    };
    if !first.vehicle_exists {
        return Err(ApiError::NotFound);
    }
    Ok(Json(
        rows.into_iter()
            .filter_map(|row| {
                Some(TempBin {
                    temp_c_bucket: row.temp_c_bucket?,
                    wh_per_km: row.wh_per_km,
                    distance_km: row.distance_km?,
                })
            })
            .collect(),
    ))
}

fn resolve_period(
    period: Option<&str>,
    count: Option<i64>,
) -> Result<(&'static str, i32), ApiError> {
    let period = match period {
        Some("week") => "week",
        Some("month") => "month",
        Some("year") => "year",
        _ => return Err(ApiError::BadRequest("period must be week, month, or year")),
    };
    let default = match period {
        "year" => 5,
        _ => 12,
    };
    let count = count.unwrap_or(default);
    if !(1..=MAX_BUCKETS).contains(&count) {
        return Err(ApiError::BadRequest("count must be between 1 and 120"));
    }
    let Ok(count) = i32::try_from(count) else {
        return Err(ApiError::BadRequest("count must be between 1 and 120"));
    };
    Ok((period, count))
}

#[allow(clippy::needless_pass_by_value)]
fn map_query(err: sqlx::Error) -> ApiError {
    let text = err.to_string();
    let redacted = redact_secrets(&text);
    if text.contains("time zone") || text.contains("TimeZone") {
        tracing::error!(error = %redacted, "stats timezone rejected");
        return ApiError::BadRequest("timezone is not recognized");
    }
    tracing::error!(error = %redacted, "query failed");
    ApiError::Internal
}

/// Bucket edges for `[now - (count-1) periods, now]`, oldest first.
/// `start` is inclusive and `end` is exclusive, both in UTC.
pub(crate) struct StatsWindow {
    unit: &'static str,
    tz: String,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    buckets: Vec<BucketEdge>,
}

struct BucketEdge {
    start_at: DateTime<Utc>,
    local_start: NaiveDateTime,
    offset_seconds: i32,
}

pub(crate) fn stats_window(
    period: &str,
    count: i32,
    timezone: &str,
    now: DateTime<Utc>,
) -> Result<StatsWindow, ApiError> {
    let tz: Tz = timezone
        .parse()
        .map_err(|_| ApiError::BadRequest("timezone is not recognized"))?;
    let unit = match period {
        "week" => "week",
        "month" => "month",
        "year" => "year",
        _ => return Err(ApiError::BadRequest("period must be week, month, or year")),
    };
    let local_now = now.with_timezone(&tz);
    let current = match unit {
        "week" => {
            let iso = local_now.date_naive().iso_week();
            NaiveDate::from_isoywd_opt(iso.year(), iso.week(), Weekday::Mon)
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .ok_or(ApiError::Internal)?
        }
        "month" => NaiveDate::from_ymd_opt(local_now.year(), local_now.month(), 1)
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .ok_or(ApiError::Internal)?,
        _ => NaiveDate::from_ymd_opt(local_now.year(), 1, 1)
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .ok_or(ApiError::Internal)?,
    };
    let mut locals = Vec::with_capacity(usize::try_from(count).unwrap_or(1));
    for index in 0..count {
        let back = count - 1 - index;
        locals.push(shift_local(unit, current, back)?);
    }
    let mut buckets = Vec::with_capacity(locals.len());
    for local_start in locals {
        let (start_at, offset_seconds) = localize(tz, local_start)?;
        buckets.push(BucketEdge {
            start_at,
            local_start,
            offset_seconds,
        });
    }
    let next_local = shift_local(unit, current, -1)?;
    let (end, _) = localize(tz, next_local)?;
    let start = buckets.first().ok_or(ApiError::Internal)?.start_at;
    Ok(StatsWindow {
        unit,
        tz: timezone.to_owned(),
        start,
        end,
        buckets,
    })
}

fn shift_local(unit: &str, current: NaiveDateTime, back: i32) -> Result<NaiveDateTime, ApiError> {
    match unit {
        "week" => Ok(current - chrono::Duration::days(i64::from(back) * 7)),
        "month" => shift_months(current, -back),
        _ => {
            let year = current.year() - back;
            NaiveDate::from_ymd_opt(year, 1, 1)
                .and_then(|date| date.and_hms_opt(0, 0, 0))
                .ok_or(ApiError::Internal)
        }
    }
}

fn shift_months(current: NaiveDateTime, delta: i32) -> Result<NaiveDateTime, ApiError> {
    let month = i32::try_from(current.month()).map_err(|_| ApiError::Internal)?;
    let total = current.year() * 12 + (month - 1) + delta;
    let year = total.div_euclid(12);
    let month = total.rem_euclid(12) + 1;
    NaiveDate::from_ymd_opt(year, u32::try_from(month).unwrap_or(1), 1)
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .ok_or(ApiError::Internal)
}

fn localize(tz: Tz, local_start: NaiveDateTime) -> Result<(DateTime<Utc>, i32), ApiError> {
    let zoned = match tz.from_local_datetime(&local_start) {
        LocalResult::Single(dt) => dt,
        LocalResult::Ambiguous(earliest, _) => earliest,
        LocalResult::None => {
            let shifted = local_start + chrono::Duration::hours(1);
            match tz.from_local_datetime(&shifted) {
                LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => dt,
                LocalResult::None => {
                    return Err(ApiError::BadRequest("timezone is not recognized"));
                }
            }
        }
    };
    Ok((
        zoned.with_timezone(&Utc),
        zoned.offset().fix().local_minus_utc(),
    ))
}

fn drive_energy_expr(drive: &str, vehicle: &str) -> String {
    format!(
        "COALESCE({drive}.energy_used_kwh, \
         CASE \
           WHEN {vehicle}.efficiency_kwh_per_km > 0 \
            AND {drive}.start_rated_range_km >= {drive}.end_rated_range_km \
           THEN ({drive}.start_rated_range_km - {drive}.end_rated_range_km) \
                * {vehicle}.efficiency_kwh_per_km \
           WHEN {vehicle}.efficiency_kwh_per_km > 0 \
            AND {drive}.start_ideal_range_km >= {drive}.end_ideal_range_km \
           THEN ({drive}.start_ideal_range_km - {drive}.end_ideal_range_km) \
                * {vehicle}.efficiency_kwh_per_km \
           ELSE NULL \
         END)"
    )
}

fn charge_is_fast_expr(charge: &str) -> String {
    format!(
        "({charge}.is_dc IS TRUE OR ({charge}.is_dc IS NULL AND lower(COALESCE({charge}.charger_type, '')) \
         IN ('ccs', 'chademo', 'combo', 'supercharger', 'tesla', 'gb/t', 'nacs')))"
    )
}

fn endpoint_proj(range: &str, soc: &str) -> String {
    format!(
        "CASE WHEN {soc} > 10 AND {soc} <= 100 AND {range} > 0 \
         THEN {range} / {soc} * 100.0 ELSE NULL END"
    )
}

fn bucket_of(window: &StatsWindow, timestamp_sql: &str) -> String {
    format!(
        "date_trunc('{unit}', timezone('{tz}', {timestamp_sql}))",
        unit = window.unit,
        tz = window.tz,
    )
}

fn timestamptz_literal(ts: DateTime<Utc>) -> String {
    format!("TIMESTAMPTZ '{}'", ts.format("%Y-%m-%d %H:%M:%S+00"))
}

fn timestamp_literal(ts: NaiveDateTime) -> String {
    format!("TIMESTAMP '{}'", ts.format("%Y-%m-%d %H:%M:%S"))
}

fn bucket_values(window: &StatsWindow) -> String {
    window
        .buckets
        .iter()
        .map(|bucket| {
            format!(
                "({}, {}, {}::integer)",
                timestamptz_literal(bucket.start_at),
                timestamp_literal(bucket.local_start),
                bucket.offset_seconds,
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Series statement. `$1` is the vehicle id and `$2` is the optional energy price.
/// Time bounds are literals so a prepared plan can prune at plan time.
pub(crate) fn series_query_sql(window: &StatsWindow) -> String {
    let energy = drive_energy_expr("d", "v");
    let fast = charge_is_fast_expr("c");
    let drive_bucket = bucket_of(window, "d.start_at");
    let charge_bucket = bucket_of(window, "c.start_at");
    let idle_bucket = bucket_of(window, "i.parked_start");
    let start = timestamptz_literal(window.start);
    let end = timestamptz_literal(window.end);
    let drive_start = endpoint_proj("d.start_rated_range_km", "d.start_battery_level");
    let drive_end = endpoint_proj("d.end_rated_range_km", "d.end_battery_level");
    let charge_end = endpoint_proj("c.end_rated_range_km", "c.end_battery_level");
    let charge_start = endpoint_proj("c.start_rated_range_km", "c.start_battery_level");
    format!(
        "WITH buckets AS (
            SELECT start_at, local_start, offset_seconds
            FROM (VALUES {values}) AS v(start_at, local_start, offset_seconds)
        ),
        drive_rows AS (
            SELECT
                {drive_bucket} AS local_start,
                d.id,
                COALESCE(d.distance_km, 0) AS distance_km,
                COALESCE(d.duration_s, 0) AS duration_s,
                {energy} AS energy_kwh,
                CASE
                    WHEN ({drive_start}) IS NOT NULL AND ({drive_end}) IS NOT NULL
                    THEN (({drive_start}) + ({drive_end})) / 2.0
                    ELSE COALESCE({drive_end}, {drive_start})
                END AS range_at_100
            FROM drives d
            JOIN vehicles v ON v.id = d.vehicle_id
            WHERE d.vehicle_id = $1
              AND d.deleted_at IS NULL
              AND d.end_at IS NOT NULL
              AND d.start_at >= {start}
              AND d.start_at < {end}
        ),
        drive_agg AS (
            SELECT
                local_start,
                COUNT(id)::bigint AS drive_count,
                COALESCE(SUM(distance_km), 0)::float8 AS distance_km,
                COALESCE(SUM(duration_s), 0)::bigint AS drive_duration_s,
                SUM(energy_kwh)::float8 AS energy_used_kwh
            FROM drive_rows
            GROUP BY local_start
        ),
        charge_rows AS (
            SELECT
                {charge_bucket} AS local_start,
                c.energy_added_kwh,
                c.cost,
                {fast} AS fast,
                COALESCE({charge_end}, {charge_start}) AS range_at_100
            FROM charges c
            WHERE c.vehicle_id = $1
              AND c.deleted_at IS NULL
              AND c.end_at IS NOT NULL
              AND c.start_at >= {start}
              AND c.start_at < {end}
        ),
        charge_agg AS (
            SELECT
                local_start,
                COALESCE(SUM(energy_added_kwh) FILTER (WHERE NOT fast), 0)::float8 AS ac_energy_kwh,
                COALESCE(SUM(energy_added_kwh) FILTER (WHERE fast), 0)::float8 AS dc_energy_kwh,
                SUM(
                    COALESCE(
                        cost,
                        CASE
                            WHEN $2::float8 IS NOT NULL AND energy_added_kwh IS NOT NULL
                            THEN energy_added_kwh * $2::float8
                            ELSE NULL
                        END
                    )
                )::float8 AS charge_cost
            FROM charge_rows
            GROUP BY local_start
        ),
        soc_agg AS (
            SELECT local_start, AVG(range_at_100)::float8 AS range_at_100_km
            FROM (
                SELECT local_start, range_at_100 FROM drive_rows WHERE range_at_100 IS NOT NULL
                UNION ALL
                SELECT local_start, range_at_100 FROM charge_rows WHERE range_at_100 IS NOT NULL
            ) points
            GROUP BY local_start
        ),
        activities AS (
            SELECT
                start_at,
                end_at,
                start_rated_range_km,
                end_rated_range_km,
                start_odometer_km,
                end_odometer_km
            FROM drives
            WHERE vehicle_id = $1
              AND deleted_at IS NULL
              AND end_at IS NOT NULL
              AND end_at >= {start}
            UNION ALL
            SELECT
                start_at,
                end_at,
                start_rated_range_km,
                end_rated_range_km,
                NULL::float8,
                NULL::float8
            FROM charges
            WHERE vehicle_id = $1
              AND deleted_at IS NULL
              AND end_at IS NOT NULL
              AND end_at >= {start}
        ),
        parked AS (
            SELECT
                lag(end_at) OVER w AS parked_start,
                start_at AS parked_end,
                lag(end_rated_range_km) OVER w AS range_before,
                start_rated_range_km AS range_after,
                lag(end_odometer_km) OVER w AS odo_before,
                start_odometer_km AS odo_after
            FROM activities
            WINDOW w AS (ORDER BY start_at ASC, end_at ASC)
        ),
        idle_rows AS (
            SELECT
                parked_start,
                EXTRACT(EPOCH FROM (parked_end - parked_start)) AS seconds,
                (range_before - range_after) AS range_loss_km
            FROM parked
            WHERE parked_start IS NOT NULL
              AND parked_end > parked_start
              AND parked_start >= {start}
              AND parked_start < {end}
              AND range_before IS NOT NULL
              AND range_after IS NOT NULL
              AND range_before >= range_after
              AND (
                    odo_before IS NULL
                    OR odo_after IS NULL
                    OR odo_after - odo_before < 1
              )
        ),
        idle_agg AS (
            SELECT
                {idle_bucket} AS local_start,
                CASE
                    WHEN COALESCE(SUM(i.seconds), 0) > 0
                    THEN (SUM(i.range_loss_km) / (SUM(i.seconds) / 86400.0))::float8
                    ELSE NULL
                END AS idle_range_loss_km_per_day,
                CASE
                    WHEN COALESCE(SUM(i.seconds), 0) > 0
                     AND (SELECT efficiency_kwh_per_km FROM vehicles WHERE id = $1) > 0
                    THEN (
                        SUM(i.range_loss_km)
                        * (SELECT efficiency_kwh_per_km FROM vehicles WHERE id = $1)
                        / (SUM(i.seconds) / 86400.0)
                    )::float8
                    ELSE NULL
                END AS idle_energy_loss_kwh_per_day
            FROM idle_rows i
            GROUP BY 1
        )
        SELECT
            (SELECT EXISTS (SELECT 1 FROM vehicles WHERE id = $1)) AS vehicle_exists,
            b.start_at,
            b.offset_seconds,
            COALESCE(d.distance_km, 0)::float8 AS distance_km,
            CASE
                WHEN COALESCE(d.drive_count, 0) = 0 THEN 0::float8
                ELSE d.energy_used_kwh
            END AS energy_used_kwh,
            CASE
                WHEN COALESCE(d.distance_km, 0) > 0 AND d.energy_used_kwh IS NOT NULL
                THEN d.energy_used_kwh * 1000.0 / d.distance_km
                ELSE NULL
            END AS wh_per_km,
            (COALESCE(c.ac_energy_kwh, 0) + COALESCE(c.dc_energy_kwh, 0)) AS energy_charged_kwh,
            COALESCE(d.drive_duration_s, 0)::bigint AS drive_duration_s,
            COALESCE(d.drive_count, 0)::bigint AS drive_count,
            s.range_at_100_km,
            COALESCE(c.ac_energy_kwh, 0)::float8 AS ac_energy_kwh,
            COALESCE(c.dc_energy_kwh, 0)::float8 AS dc_energy_kwh,
            CASE
                WHEN (COALESCE(c.ac_energy_kwh, 0) + COALESCE(c.dc_energy_kwh, 0)) > 0
                THEN COALESCE(c.dc_energy_kwh, 0)
                     / (COALESCE(c.ac_energy_kwh, 0) + COALESCE(c.dc_energy_kwh, 0)) * 100.0
                ELSE NULL
            END AS dc_share_pct,
            c.charge_cost,
            CASE
                WHEN c.charge_cost IS NOT NULL AND COALESCE(d.distance_km, 0) > 0
                THEN c.charge_cost / d.distance_km
                ELSE NULL
            END AS cost_per_km,
            i.idle_range_loss_km_per_day,
            i.idle_energy_loss_kwh_per_day
        FROM buckets b
        LEFT JOIN drive_agg d ON d.local_start = b.local_start
        LEFT JOIN charge_agg c ON c.local_start = b.local_start
        LEFT JOIN soc_agg s ON s.local_start = b.local_start
        LEFT JOIN idle_agg i ON i.local_start = b.local_start
        ORDER BY b.start_at",
        values = bucket_values(window),
    )
}

pub(crate) fn consumption_query_sql(window: &StatsWindow) -> String {
    let energy = drive_energy_expr("d", "v");
    let start = timestamptz_literal(window.start);
    let end = timestamptz_literal(window.end);
    format!(
        "WITH bins AS (
            SELECT
                (floor(d.outside_temp_avg_c / 5.0) * 5)::integer AS temp_c_bucket,
                SUM(d.distance_km)::float8 AS distance_km,
                (SUM({energy}) * 1000.0 / SUM(d.distance_km))::float8 AS wh_per_km
            FROM drives d
            JOIN vehicles v ON v.id = d.vehicle_id
            WHERE d.vehicle_id = $1
              AND d.deleted_at IS NULL
              AND d.end_at IS NOT NULL
              AND d.outside_temp_avg_c IS NOT NULL
              AND d.distance_km > 0
              AND {energy} IS NOT NULL
              AND d.start_at >= {start}
              AND d.start_at < {end}
            GROUP BY 1
        )
        SELECT
            present.ok AS vehicle_exists,
            bins.temp_c_bucket,
            bins.wh_per_km,
            bins.distance_km
        FROM (SELECT EXISTS (SELECT 1 FROM vehicles WHERE id = $1) AS ok) AS present
        LEFT JOIN bins ON present.ok
        ORDER BY bins.temp_c_bucket"
    )
}

#[derive(Clone, Eq, PartialEq, Hash)]
struct SeriesCacheKey {
    state: usize,
    vehicle_id: i64,
    period: String,
    count: i32,
    timezone: String,
    price_bits: Option<u64>,
    window_start_ms: i64,
}

static SERIES_CACHE: Mutex<Option<HashMap<SeriesCacheKey, (Instant, SeriesResponse)>>> =
    Mutex::new(None);

fn cache_map()
-> std::sync::MutexGuard<'static, Option<HashMap<SeriesCacheKey, (Instant, SeriesResponse)>>> {
    SERIES_CACHE
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

fn cache_get(key: &SeriesCacheKey, now: Instant) -> Option<SeriesResponse> {
    let mut guard = cache_map();
    let map = guard.get_or_insert_with(HashMap::new);
    let fresh = map.get(key).and_then(|(stored, value)| {
        (now.saturating_duration_since(*stored) < SERIES_CACHE_TTL).then(|| value.clone())
    });
    if fresh.is_none() {
        map.remove(key);
    }
    fresh
}

fn cache_insert(key: SeriesCacheKey, value: SeriesResponse, now: Instant) {
    let mut guard = cache_map();
    let map = guard.get_or_insert_with(HashMap::new);
    map.retain(|_, (stored, _)| now.saturating_duration_since(*stored) < SERIES_CACHE_TTL);
    map.insert(key, (now, value));
}

#[derive(Deserialize)]
pub(crate) struct PeriodQuery {
    period: Option<String>,
    count: Option<i64>,
}

#[derive(Clone, Serialize)]
pub(crate) struct SeriesResponse {
    pub period: String,
    pub buckets: Vec<Bucket>,
}

#[derive(Clone, Serialize)]
pub(crate) struct Bucket {
    pub start: DateTime<FixedOffset>,
    pub distance_km: f64,
    pub energy_used_kwh: Option<f64>,
    pub wh_per_km: Option<f64>,
    pub energy_charged_kwh: f64,
    pub drive_duration_s: i64,
    pub drive_count: i64,
    pub range_at_100_km: Option<f64>,
    pub ac_energy_kwh: f64,
    pub dc_energy_kwh: f64,
    pub dc_share_pct: Option<f64>,
    pub charge_cost: Option<f64>,
    pub cost_per_km: Option<f64>,
    pub idle_range_loss_km_per_day: Option<f64>,
    pub idle_energy_loss_kwh_per_day: Option<f64>,
}

#[derive(Serialize)]
pub(crate) struct TempBin {
    pub temp_c_bucket: i32,
    pub wh_per_km: Option<f64>,
    pub distance_km: f64,
}

#[derive(sqlx::FromRow)]
struct SeriesRow {
    vehicle_exists: bool,
    start_at: DateTime<Utc>,
    offset_seconds: i32,
    distance_km: f64,
    energy_used_kwh: Option<f64>,
    wh_per_km: Option<f64>,
    energy_charged_kwh: f64,
    drive_duration_s: i64,
    drive_count: i64,
    range_at_100_km: Option<f64>,
    ac_energy_kwh: f64,
    dc_energy_kwh: f64,
    dc_share_pct: Option<f64>,
    charge_cost: Option<f64>,
    cost_per_km: Option<f64>,
    idle_range_loss_km_per_day: Option<f64>,
    idle_energy_loss_kwh_per_day: Option<f64>,
}

impl SeriesRow {
    fn into_bucket(self) -> Result<Bucket, ApiError> {
        let Some(offset) = FixedOffset::east_opt(self.offset_seconds) else {
            return Err(ApiError::Internal);
        };
        Ok(Bucket {
            start: self.start_at.with_timezone(&offset),
            distance_km: self.distance_km,
            energy_used_kwh: self.energy_used_kwh,
            wh_per_km: self.wh_per_km,
            energy_charged_kwh: self.energy_charged_kwh,
            drive_duration_s: self.drive_duration_s,
            drive_count: self.drive_count,
            range_at_100_km: self.range_at_100_km,
            ac_energy_kwh: self.ac_energy_kwh,
            dc_energy_kwh: self.dc_energy_kwh,
            dc_share_pct: self.dc_share_pct,
            charge_cost: self.charge_cost,
            cost_per_km: self.cost_per_km,
            idle_range_loss_km_per_day: self.idle_range_loss_km_per_day,
            idle_energy_loss_kwh_per_day: self.idle_energy_loss_kwh_per_day,
        })
    }
}

#[derive(sqlx::FromRow)]
struct TempRow {
    vehicle_exists: bool,
    temp_c_bucket: Option<i32>,
    wh_per_km: Option<f64>,
    distance_km: Option<f64>,
}

pub(super) async fn console_page(
    app: &AppState,
    chrome: &super::markup::Chrome,
    id: i64,
    period: &str,
    count: i32,
) -> String {
    use super::markup::{document, toolbar, vehicle_tabs};
    use maud::html;
    let series = load_series(app, id, period, count).await.ok();
    let bins = load_bins(app, id, period, count).await.unwrap_or_default();
    let quality = quality(app, id).await;
    let body = html! {
        (toolbar(chrome, "Stats"))
        (vehicle_tabs(chrome))
        div role="tabpanel" {
            form class="filters" method="get" action=(format!("/v/{id}/stats")) {
                label class="field" for="period" {
                    "Période"
                    select id="period" name="period" {
                        option value="week" { "semaine" }
                        option value="month" { "mois" }
                        option value="year" { "année" }
                    }
                }
                label class="field" for="count" { "Nombre" input id="count" name="count" value=(count.to_string()); }
                button class="tonal" type="submit" { "Afficher" }
                a href=(format!("/v/{id}/stats.csv?period={period}&count={count}")) { "CSV" }
            }
            @if let Some(series) = series {
                p { "Total distance " (sum_distance(&series)) " km · coût/km " (first_cost(&series)) }
                (bars(id, "Wh/km", &series, |bucket| bucket.wh_per_km.unwrap_or(0.0)))
                (bars(id, "Distance", &series, |bucket| bucket.distance_km))
                (bars(id, "Énergie chargée", &series, |bucket| bucket.energy_charged_kwh))
                (bars(id, "Autonomie à 100 %", &series, |bucket| bucket.range_at_100_km.unwrap_or(0.0)))
                (bars(id, "Part DC", &series, |bucket| bucket.dc_share_pct.unwrap_or(0.0)))
                (bars(id, "Coût/km", &series, |bucket| bucket.cost_per_km.unwrap_or(0.0)))
                (bars(id, "Perte à l'arrêt", &series, |bucket| bucket.idle_range_loss_km_per_day.unwrap_or(0.0)))
                section class="card" {
                    h2 { "Tableau" }
                    table class="settings" {
                        thead { tr { th { "Début" } th { "km" } th { "Wh/km" } th { "Coût/km" } } }
                        tbody {
                            @for bucket in &series.buckets {
                                tr {
                                    td { a href=(bucket_href(id, &series.period, bucket.start)) { (bucket.start.format("%Y-%m-%d").to_string()) } }
                                    td { (format!("{:.1}", bucket.distance_km)) }
                                    td { (bucket.wh_per_km.map_or_else(|| "—".into(), |value| format!("{value:.0}"))) }
                                    td { (bucket.cost_per_km.map_or_else(|| "—".into(), |value| format!("{value:.3}"))) }
                                }
                            }
                        }
                    }
                }
            }
            section class="card" {
                h2 { "Consommation par température" }
                (temp_bars(id, &bins))
            }
            section class="card" {
                h2 { "Qualité" }
                dl class="facts" {
                    div { dt { "Trajets sans énergie" } dd { (quality.drives_without_energy) } }
                    div { dt { "Charges sans énergie utilisée" } dd { (quality.charges_without_energy) } }
                    div { dt { "Jours sans point GPS" } dd { (quality.gap_days) } }
                    @for (source, count) in &quality.points {
                        div { dt { (source) } dd { (count) } }
                    }
                }
            }
        }
    };
    document(chrome.debug_script, body)
}

struct Quality {
    drives_without_energy: i64,
    charges_without_energy: i64,
    gap_days: i64,
    points: Vec<(String, i64)>,
}

async fn quality(app: &AppState, id: i64) -> Quality {
    let drives_without_energy: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM drives WHERE vehicle_id = $1 AND deleted_at IS NULL AND end_at IS NOT NULL AND energy_used_kwh IS NULL",
    ).bind(id).fetch_one(&app.inner.db).await.unwrap_or(0);
    let charges_without_energy: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM charges WHERE vehicle_id = $1 AND deleted_at IS NULL AND end_at IS NOT NULL AND energy_used_kwh IS NULL",
    ).bind(id).fetch_one(&app.inner.db).await.unwrap_or(0);
    let gap_days: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM generate_series((now() - interval '30 days')::date, now()::date, '1 day') AS day
         WHERE NOT EXISTS (
            SELECT 1 FROM positions
            WHERE vehicle_id = $1 AND recorded_at >= day AND recorded_at < day + interval '1 day'
         )",
    ).bind(id).fetch_one(&app.inner.db).await.unwrap_or(0);
    let points = sqlx::query_as(
        "SELECT source, COUNT(*)::bigint FROM positions
         WHERE vehicle_id = $1 AND recorded_at >= now() - interval '30 days'
         GROUP BY source ORDER BY source",
    )
    .bind(id)
    .fetch_all(&app.inner.db)
    .await
    .unwrap_or_default();
    Quality {
        drives_without_energy,
        charges_without_energy,
        gap_days,
        points,
    }
}

async fn load_bins(
    app: &AppState,
    id: i64,
    period: &str,
    count: i32,
) -> Result<Vec<TempBin>, ApiError> {
    let timezone = crate::settings::text_or(
        &app.inner.db,
        Some(id),
        "TESLA_TIMEZONE",
        &app.inner.config.timezone,
    )
    .await;
    let window = stats_window(period, count, &timezone, Utc::now())?;
    let rows = sqlx::query_as::<_, TempRow>(&consumption_query_sql(&window))
        .bind(id)
        .fetch_all(&app.inner.db)
        .await
        .map_err(map_query)?;
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(TempBin {
                temp_c_bucket: row.temp_c_bucket?,
                wh_per_km: row.wh_per_km,
                distance_km: row.distance_km.unwrap_or(0.0),
            })
        })
        .collect())
}

fn bars(
    id: i64,
    title: &str,
    series: &SeriesResponse,
    value: impl Fn(&Bucket) -> f64,
) -> maud::Markup {
    use maud::html;
    let max = series
        .buckets
        .iter()
        .map(&value)
        .fold(0.0_f64, f64::max)
        .max(1e-9);
    let width = u32::try_from(series.buckets.len().max(1)).unwrap_or(1) * 8;
    html! {
        section class="card" {
            h2 { (title) }
            svg class="chart" viewBox=(format!("0 0 {width} 40")) width="640" height="180" role="img" aria-label=(title) {
                @for (index, bucket) in series.buckets.iter().enumerate() {
                    @let height = (value(bucket) / max) * 36.0;
                    @let x = u32::try_from(index).unwrap_or(0) * 8;
                    a href=(bucket_href(id, &series.period, bucket.start)) {
                        rect x=(x) y=(format!("{:.2}", 38.0 - height)) width="6" height=(format!("{height:.2}")) fill="currentColor" {}
                    }
                }
            }
            ol class="chart-labels" {
                @for bucket in &series.buckets {
                    li { a href=(bucket_href(id, &series.period, bucket.start)) { (bucket.start.format("%d/%m").to_string()) } }
                }
            }
        }
    }
}

fn temp_bars(id: i64, bins: &[TempBin]) -> maud::Markup {
    use maud::html;
    let max = bins
        .iter()
        .filter_map(|bin| bin.wh_per_km)
        .fold(0.0_f64, f64::max)
        .max(1e-9);
    html! {
        svg class="chart" viewBox=(format!("0 0 {} 40", bins.len().max(1) * 8)) width="640" height="180" role="img" aria-label="Consommation par température" {
            @for (index, bin) in bins.iter().enumerate() {
                @let height = bin.wh_per_km.unwrap_or(0.0) / max * 36.0;
                a href=(format!("/v/{id}/drives")) {
                    rect x=(u32::try_from(index).unwrap_or(0) * 8) y=(format!("{:.2}", 38.0 - height)) width="6" height=(format!("{height:.2}")) fill="currentColor" {}
                }
            }
        }
        ol class="chart-labels" {
            @for bin in bins {
                li { (bin.temp_c_bucket) " °C" }
            }
        }
    }
}

fn bucket_href(id: i64, period: &str, start: DateTime<FixedOffset>) -> String {
    let days = match period {
        "year" => 366,
        "week" => 7,
        _ => 31,
    };
    let end = start + chrono::Duration::days(days);
    format!(
        "/v/{id}/drives?from={}&to={}",
        start.format("%Y-%m-%d"),
        end.format("%Y-%m-%d")
    )
}

fn sum_distance(series: &SeriesResponse) -> String {
    format!(
        "{:.1}",
        series
            .buckets
            .iter()
            .map(|bucket| bucket.distance_km)
            .sum::<f64>()
    )
}

fn first_cost(series: &SeriesResponse) -> String {
    series
        .buckets
        .iter()
        .find_map(|bucket| bucket.cost_per_km)
        .map_or_else(|| "—".into(), |value| format!("{value:.3}"))
}

#[cfg(test)]
mod tests {
    use super::{
        SeriesCacheKey, SeriesResponse, cache_get, cache_insert, resolve_period, stats_window,
    };
    use chrono::{DateTime, Utc};
    use std::time::{Duration, Instant};

    #[test]
    fn period_defaults_and_rejects_bad_input() {
        assert_eq!(resolve_period(Some("week"), None).unwrap(), ("week", 12));
        assert_eq!(resolve_period(Some("month"), None).unwrap(), ("month", 12));
        assert_eq!(resolve_period(Some("year"), None).unwrap(), ("year", 5));
        assert_eq!(resolve_period(Some("year"), Some(2)).unwrap(), ("year", 2));
        assert!(resolve_period(Some("day"), None).is_err());
        assert!(resolve_period(None, None).is_err());
        assert!(resolve_period(Some("month"), Some(0)).is_err());
        assert!(resolve_period(Some("month"), Some(121)).is_err());
    }

    #[test]
    fn paris_buckets_match_local_midnights_across_dst() {
        let now = DateTime::parse_from_rfc3339("2026-11-10T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let window = stats_window("month", 2, "Europe/Paris", now).unwrap();
        assert_eq!(window.buckets.len(), 2);
        assert_eq!(
            window.buckets[0].local_start.to_string(),
            "2026-10-01 00:00:00"
        );
        assert_eq!(window.buckets[0].offset_seconds, 2 * 3600);
        assert_eq!(
            window.buckets[1].local_start.to_string(),
            "2026-11-01 00:00:00"
        );
        assert_eq!(window.buckets[1].offset_seconds, 3600);
        assert_eq!(
            window.buckets[1].start_at,
            DateTime::parse_from_rfc3339("2026-10-31T23:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        );
        let week = stats_window("week", 1, "Europe/Paris", now).unwrap();
        assert_eq!(
            week.buckets[0].local_start.to_string(),
            "2026-11-09 00:00:00"
        );
        assert!(stats_window("month", 1, "Not/AZone", now).is_err());
    }

    #[test]
    fn series_cache_keeps_a_hit_for_five_minutes() {
        let key = SeriesCacheKey {
            state: usize::MAX,
            vehicle_id: -1,
            period: "month".to_owned(),
            count: 1,
            timezone: "Europe/Paris".to_owned(),
            price_bits: None,
            window_start_ms: 0,
        };
        let value = SeriesResponse {
            period: "month".to_owned(),
            buckets: Vec::new(),
        };
        let now = Instant::now();
        cache_insert(key.clone(), value, now);
        assert!(cache_get(&key, now + Duration::from_secs(299)).is_some());
        assert!(cache_get(&key, now + Duration::from_secs(301)).is_none());
    }

    #[tokio::test]
    async fn stored_kwh_price_changes_cost_per_km() {
        let Some(db) = crate::test_util::TestDb::migrated("price").await else {
            return;
        };
        let state = crate::app::test_state(db.pool.clone()).await;
        let vehicle: i64 = sqlx::query_scalar(
            "INSERT INTO vehicles (vin, name) VALUES ('5YJ30000000000055', 'Prix') RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let start = chrono::Utc::now() - chrono::Duration::hours(2);
        sqlx::query(
            "INSERT INTO drives (vehicle_id, start_at, end_at, distance_km, energy_used_kwh, source)
             VALUES ($1, $2, $3, 100, 10, 'poll')",
        )
        .bind(vehicle)
        .bind(start)
        .bind(start + chrono::Duration::minutes(30))
        .execute(&db.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO charges (vehicle_id, start_at, end_at, energy_added_kwh, source)
             VALUES ($1, $2, $3, 10, 'poll')",
        )
        .bind(vehicle)
        .bind(start)
        .bind(start + chrono::Duration::hours(1))
        .execute(&db.pool)
        .await
        .unwrap();
        crate::settings::set(&db.pool, None, "ENERGY_PRICE_PER_KWH", "0.20")
            .await
            .unwrap();
        let series = super::load_series(&state, vehicle, "month", 1)
            .await
            .unwrap();
        let cost = series
            .buckets
            .iter()
            .find_map(|bucket| bucket.cost_per_km)
            .expect("cost");
        assert!((cost - 0.02).abs() < 0.0001, "{cost}");
        let page = super::console_page(
            &state,
            &crate::http::ui::chrome(
                &crate::http::ui::Page {
                    session_id: "s".into(),
                    csrf: "c".into(),
                    cookie: None,
                    vehicles: Vec::new(),
                    remembered: None,
                    admin: true,
                    username: "admin".into(),
                },
                Some(vehicle),
                Some(crate::http::markup::VehicleTab::Stats),
                crate::http::markup::GlobalTab::None,
                false,
            ),
            vehicle,
            "month",
            1,
        )
        .await;
        assert!(page.contains("class=\"chart\""));
        assert!(page.contains("height=\"180\""));
        assert!(page.contains("chart-labels"));
        assert!(!page.contains("<text"));
    }
}
