// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Read-only time series for the Home Assistant charts.
//!
//! Buckets are calendar weeks (Monday), months, or years in [`crate::config::Config::timezone`].
//! Drive energy prefers `drives.energy_used_kwh` when it is already stored (native trips and
//! TeslaMate imports). Otherwise it uses the same basis as [`crate::state::range_energy`]:
//! rated range lost times vehicle efficiency, then ideal range lost times efficiency.

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, FixedOffset, Utc};
use serde::{Deserialize, Serialize};

use super::ApiError;
use crate::app::AppState;
use crate::error::redact_secrets;

const MAX_BUCKETS: i64 = 120;

pub(crate) async fn series(
    State(app): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<PeriodQuery>,
) -> Result<Json<SeriesResponse>, ApiError> {
    let (period, count) = resolve_period(query.period.as_deref(), query.count)?;
    let rows = sqlx::query_as::<_, SeriesRow>(&series_sql())
        .bind(id)
        .bind(app.inner.config.timezone.as_str())
        .bind(period)
        .bind(count)
        .bind(app.inner.config.energy_price_per_kwh)
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
    Ok(Json(SeriesResponse {
        period: period.to_owned(),
        buckets,
    }))
}

pub(crate) async fn consumption_by_temp(
    State(app): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<PeriodQuery>,
) -> Result<Json<Vec<TempBin>>, ApiError> {
    let (period, count) = resolve_period(query.period.as_deref(), query.count)?;
    let rows = sqlx::query_as::<_, TempRow>(&consumption_sql())
        .bind(id)
        .bind(app.inner.config.timezone.as_str())
        .bind(period)
        .bind(count)
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

/// Shared `params` / `buckets` prefix. `$1` vehicle, `$2` timezone, `$3` unit, `$4` count.
fn buckets_cte() -> &'static str {
    "params AS (
        SELECT
            $1::bigint AS vehicle_id,
            $2::text AS tz,
            $3::text AS unit,
            $4::int AS bucket_count,
            CASE $3
                WHEN 'week' THEN interval '1 week'
                WHEN 'month' THEN interval '1 month'
                ELSE interval '1 year'
            END AS step
    ),
    buckets AS (
        SELECT
            gs AS local_start,
            (gs AT TIME ZONE p.tz) AS start_at,
            ((gs + p.step) AT TIME ZONE p.tz) AS end_at,
            EXTRACT(EPOCH FROM (
                gs - ((gs AT TIME ZONE p.tz) AT TIME ZONE 'UTC')
            ))::integer AS offset_seconds,
            p.tz,
            p.vehicle_id
        FROM params p
        CROSS JOIN LATERAL generate_series(
            date_trunc(p.unit, timezone(p.tz, now())) - (p.bucket_count - 1) * p.step,
            date_trunc(p.unit, timezone(p.tz, now())),
            p.step
        ) AS gs
    )"
}

fn series_sql() -> String {
    let energy = drive_energy_expr("d", "v");
    let fast = charge_is_fast_expr("c");
    format!(
        "WITH {buckets},
        drive_rows AS (
            SELECT
                d.id,
                d.start_at,
                COALESCE(d.distance_km, 0) AS distance_km,
                COALESCE(d.duration_s, 0) AS duration_s,
                {energy} AS energy_kwh
            FROM drives d
            JOIN vehicles v ON v.id = d.vehicle_id
            WHERE d.vehicle_id = $1
              AND d.end_at IS NOT NULL
              AND d.start_at >= (SELECT min(start_at) FROM buckets)
              AND d.start_at < (SELECT max(end_at) FROM buckets)
        ),
        drive_agg AS (
            SELECT
                b.local_start,
                COUNT(d.id)::bigint AS drive_count,
                COALESCE(SUM(d.distance_km), 0)::float8 AS distance_km,
                COALESCE(SUM(d.duration_s), 0)::bigint AS drive_duration_s,
                CASE
                    WHEN COUNT(d.id) = 0 THEN 0::float8
                    ELSE SUM(d.energy_kwh)::float8
                END AS energy_used_kwh
            FROM buckets b
            LEFT JOIN drive_rows d
              ON d.start_at >= b.start_at AND d.start_at < b.end_at
            GROUP BY b.local_start
        ),
        charge_rows AS (
            SELECT
                c.start_at,
                c.energy_added_kwh,
                c.cost,
                {fast} AS fast
            FROM charges c
            WHERE c.vehicle_id = $1
              AND c.end_at IS NOT NULL
              AND c.start_at >= (SELECT min(start_at) FROM buckets)
              AND c.start_at < (SELECT max(end_at) FROM buckets)
        ),
        charge_agg AS (
            SELECT
                b.local_start,
                COALESCE(SUM(c.energy_added_kwh) FILTER (WHERE NOT c.fast), 0)::float8 AS ac_energy_kwh,
                COALESCE(SUM(c.energy_added_kwh) FILTER (WHERE c.fast), 0)::float8 AS dc_energy_kwh,
                SUM(
                    COALESCE(
                        c.cost,
                        CASE
                            WHEN $5::float8 IS NOT NULL AND c.energy_added_kwh IS NOT NULL
                            THEN c.energy_added_kwh * $5::float8
                            ELSE NULL
                        END
                    )
                )::float8 AS charge_cost
            FROM buckets b
            LEFT JOIN charge_rows c
              ON c.start_at >= b.start_at AND c.start_at < b.end_at
            GROUP BY b.local_start
        ),
        soc_rows AS (
            SELECT recorded_at, battery_level, rated_range_km
            FROM positions
            WHERE vehicle_id = $1
              AND recorded_at >= (SELECT min(start_at) FROM buckets)
              AND recorded_at < (SELECT max(end_at) FROM buckets)
              AND battery_level > 10 AND battery_level <= 100
              AND rated_range_km > 0
            UNION ALL
            SELECT cs.recorded_at, cs.battery_level, cs.rated_range_km
            FROM charge_samples cs
            JOIN charges ch ON ch.id = cs.charge_id
            WHERE ch.vehicle_id = $1
              AND cs.recorded_at >= (SELECT min(start_at) FROM buckets)
              AND cs.recorded_at < (SELECT max(end_at) FROM buckets)
              AND cs.battery_level > 10 AND cs.battery_level <= 100
              AND cs.rated_range_km > 0
            UNION ALL
            SELECT c.start_at, c.start_battery_level, c.start_rated_range_km
            FROM charges c
            WHERE c.vehicle_id = $1
              AND c.end_at IS NOT NULL
              AND c.start_at >= (SELECT min(start_at) FROM buckets)
              AND c.start_at < (SELECT max(end_at) FROM buckets)
              AND c.start_battery_level > 10 AND c.start_battery_level <= 100
              AND c.start_rated_range_km > 0
            UNION ALL
            SELECT c.end_at, c.end_battery_level, c.end_rated_range_km
            FROM charges c
            WHERE c.vehicle_id = $1
              AND c.end_at IS NOT NULL
              AND c.end_at >= (SELECT min(start_at) FROM buckets)
              AND c.end_at < (SELECT max(end_at) FROM buckets)
              AND c.end_battery_level > 10 AND c.end_battery_level <= 100
              AND c.end_rated_range_km > 0
        ),
        soc_agg AS (
            SELECT
                b.local_start,
                AVG(s.rated_range_km / s.battery_level * 100.0)::float8 AS range_at_100_km
            FROM buckets b
            LEFT JOIN soc_rows s
              ON s.recorded_at >= b.start_at AND s.recorded_at < b.end_at
            GROUP BY b.local_start
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
              AND end_at IS NOT NULL
              AND end_at >= (SELECT min(start_at) FROM buckets)
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
              AND end_at IS NOT NULL
              AND end_at >= (SELECT min(start_at) FROM buckets)
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
                b.local_start,
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
            FROM buckets b
            LEFT JOIN idle_rows i
              ON i.parked_start >= b.start_at AND i.parked_start < b.end_at
            GROUP BY b.local_start
        )
        SELECT
            (SELECT EXISTS (SELECT 1 FROM vehicles WHERE id = $1)) AS vehicle_exists,
            b.start_at,
            b.offset_seconds,
            d.distance_km,
            d.energy_used_kwh,
            CASE
                WHEN d.distance_km > 0 AND d.energy_used_kwh IS NOT NULL
                THEN d.energy_used_kwh * 1000.0 / d.distance_km
                ELSE NULL
            END AS wh_per_km,
            (c.ac_energy_kwh + c.dc_energy_kwh) AS energy_charged_kwh,
            d.drive_duration_s,
            d.drive_count,
            s.range_at_100_km,
            c.ac_energy_kwh,
            c.dc_energy_kwh,
            CASE
                WHEN (c.ac_energy_kwh + c.dc_energy_kwh) > 0
                THEN c.dc_energy_kwh / (c.ac_energy_kwh + c.dc_energy_kwh) * 100.0
                ELSE NULL
            END AS dc_share_pct,
            c.charge_cost,
            CASE
                WHEN c.charge_cost IS NOT NULL AND d.distance_km > 0
                THEN c.charge_cost / d.distance_km
                ELSE NULL
            END AS cost_per_km,
            i.idle_range_loss_km_per_day,
            i.idle_energy_loss_kwh_per_day
        FROM buckets b
        JOIN drive_agg d ON d.local_start = b.local_start
        JOIN charge_agg c ON c.local_start = b.local_start
        JOIN soc_agg s ON s.local_start = b.local_start
        JOIN idle_agg i ON i.local_start = b.local_start
        ORDER BY b.start_at",
        buckets = buckets_cte(),
        energy = energy,
        fast = fast,
    )
}

fn consumption_sql() -> String {
    let energy = drive_energy_expr("d", "v");
    format!(
        "WITH {buckets},
        bins AS (
            SELECT
                (floor(d.outside_temp_avg_c / 5.0) * 5)::integer AS temp_c_bucket,
                SUM(d.distance_km)::float8 AS distance_km,
                (SUM({energy}) * 1000.0 / SUM(d.distance_km))::float8 AS wh_per_km
            FROM drives d
            JOIN vehicles v ON v.id = d.vehicle_id
            WHERE d.vehicle_id = $1
              AND d.end_at IS NOT NULL
              AND d.outside_temp_avg_c IS NOT NULL
              AND d.distance_km > 0
              AND {energy} IS NOT NULL
              AND d.start_at >= (SELECT min(start_at) FROM buckets)
              AND d.start_at < (SELECT max(end_at) FROM buckets)
            GROUP BY 1
        )
        SELECT
            present.ok AS vehicle_exists,
            bins.temp_c_bucket,
            bins.wh_per_km,
            bins.distance_km
        FROM (SELECT EXISTS (SELECT 1 FROM vehicles WHERE id = $1) AS ok) AS present
        LEFT JOIN bins ON present.ok
        ORDER BY bins.temp_c_bucket",
        buckets = buckets_cte(),
        energy = energy,
    )
}

#[derive(Deserialize)]
pub(crate) struct PeriodQuery {
    period: Option<String>,
    count: Option<i64>,
}

#[derive(Serialize)]
pub(crate) struct SeriesResponse {
    pub period: String,
    pub buckets: Vec<Bucket>,
}

#[derive(Serialize)]
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

#[cfg(test)]
mod tests {
    use super::resolve_period;

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
}
