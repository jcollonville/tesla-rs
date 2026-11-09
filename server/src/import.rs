// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Idempotent, resumable import from a TeslaMate 4.x Postgres database.
//!
//! The source connection is read-only and pinned to UTC. Rows are copied in id
//! order and the cursor is committed with each batch, so a crash resumes at the
//! last id. TeslaMate already stores kilometres, km/h, and Celsius. Those values
//! are not converted again.
//!
//! Column types were checked against TeslaMate v4.1.1
//! (`teslamate-org/teslamate` tag `v4.1.1`, `priv/repo/migrations`). Every
//! selected value is cast in SQL: `smallint` to `integer`, `numeric` and `real`
//! to `float8`, `varchar` and the `billing_type` enum to `text`, `json` to
//! `jsonb`, and `timestamp` without time zone to `timestamptz` with
//! `AT TIME ZONE 'UTC'`. TeslaMate stores UTC in `timestamp(6)`, not
//! `timestamptz` (the positions BRIN index uses `timestamp_minmax_multi_ops`).
//! `drives.ascent` / `drives.descent` are optional smallints added by migration
//! `20250613133700`. Drive state of charge is not a drives column in TeslaMate.
//! It is copied from `positions.battery_level`, falling back to
//! `usable_battery_level`, through `drives.start_position_id` and
//! `drives.end_position_id`. [`backfill_drive_battery`] runs at the end of every
//! import, after positions are copied, and only fills levels that are still
//! null. A resume that finishes the positions therefore fills drives whose
//! rows arrived after migration `0003_backfill_drive_battery` already ran.
//!
//! Each table is read with a keyset on its primary key. TeslaMate 4.1.1 stores
//! `cars.id` as `smallint` and every other imported id as `integer`. The
//! comparison casts the bound cursor (`$1::int2` or `$1::int4`), not the
//! column, and `ORDER BY` names the table column (`positions.id`, `drives.id`,
//! and the other imported tables). `ORDER BY id` would sort the `id::bigint`
//! output alias, and Postgres would sequential-scan and sort. Cursors stay
//! `bigint` in `import_cursors`, so a positions cursor such as 17000 still
//! resumes.
//!
//! Positions are read 5000 rows at a time. One page is a few megabytes and is
//! dropped after it is inserted. A position row binds 18 parameters, so each
//! insert is capped under Postgres' 65535-parameter limit. Other tables use
//! `batch`, which is at most 2000. Progress is logged when a table starts and
//! then about every 5 seconds (`done`, `total`, `percent`, `rows_per_sec`).
//! Re-running continues from the committed cursor.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, QueryBuilder, Transaction};

use crate::error::AppError;
use crate::state::range_energy;

const SOURCE: &str = "teslamate";

#[derive(Debug, Default)]
pub(crate) struct ImportStats {
    pub cars: u64,
    pub addresses: u64,
    pub geofences: u64,
    pub drives: u64,
    pub charges: u64,
    pub charge_samples: u64,
    pub positions: u64,
}

pub(crate) async fn import_url(
    dest_url: &str,
    source_url: &str,
    batch: i64,
) -> Result<ImportStats, AppError> {
    let batch = normalize_batch(batch)?;
    let dest = PgPool::connect(dest_url).await?;
    sqlx::migrate!("./migrations").run(&dest).await?;
    let source = read_only_pool(source_url).await?;
    import_pools(&dest, &source, batch).await
}

async fn read_only_pool(url: &str) -> Result<PgPool, AppError> {
    use sqlx::postgres::PgPoolOptions;
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("SET TIME ZONE 'UTC'")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("SET default_transaction_read_only = on")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(url)
        .await
        .map_err(AppError::from)
}

pub(crate) async fn import_pools(
    dest: &PgPool,
    source: &PgPool,
    batch: i64,
) -> Result<ImportStats, AppError> {
    let batch = normalize_batch(batch)?;
    verify_schema(source).await?;
    let mut stats = ImportStats::default();
    let ascent = column_exists(source, "drives", "ascent").await?;
    stats.cars = copy_cars(dest, source, batch).await?;
    stats.addresses = copy_addresses(dest, source, batch).await?;
    stats.geofences = copy_geofences(dest, source, batch).await?;
    let cars = load_links(dest, "car").await?;
    let addresses = load_links(dest, "address").await?;
    let fences = load_links(dest, "geofence").await?;
    let efficiency = load_efficiency(dest).await?;
    stats.drives = copy_drives(
        dest,
        source,
        batch,
        ascent,
        &cars,
        &addresses,
        &fences,
        &efficiency,
    )
    .await?;
    stats.charges = copy_charges(dest, source, batch, &cars, &addresses, &fences).await?;
    let drives = load_links(dest, "drive").await?;
    let charges = load_links(dest, "charge").await?;
    stats.charge_samples = copy_charge_samples(dest, source, batch, &charges).await?;
    stats.positions = copy_positions(dest, source, &cars, &drives).await?;
    // Runs after positions, including a resume that only copies the tail.
    // Migration 0003 already ran at startup, before those rows existed.
    backfill_drive_battery(dest).await?;
    tracing::info!(
        cars = stats.cars,
        addresses = stats.addresses,
        geofences = stats.geofences,
        drives = stats.drives,
        charges = stats.charges,
        charge_samples = stats.charge_samples,
        positions = stats.positions,
        "teslamate import finished"
    );
    Ok(stats)
}

fn normalize_batch(batch: i64) -> Result<i64, AppError> {
    // Charge samples bind 13 parameters per row. 2000 × 13 stays under the
    // 65535 protocol limit. Positions use [`POSITIONS_PAGE`] and chunk inserts.
    if (1..=2_000).contains(&batch) {
        Ok(batch)
    } else {
        Err(AppError::message("batch size must be between 1 and 2000"))
    }
}

async fn verify_schema(source: &PgPool) -> Result<(), AppError> {
    let present = columns(source).await?;
    let mut missing = Vec::new();
    for (table, column) in REQUIRED {
        if !present.contains(&(*table, *column)) {
            missing.push(format!("{table}.{column}"));
        }
    }
    if missing.is_empty() {
        Ok(())
    } else {
        missing.truncate(30);
        Err(AppError::message(format!(
            "TeslaMate schema is missing columns: {}",
            missing.join(", ")
        )))
    }
}

async fn columns(source: &PgPool) -> Result<HashSet<(&str, &str)>, AppError> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_name, column_name FROM information_schema.columns WHERE table_schema = 'public'",
    )
    .fetch_all(source)
    .await?;
    let mut set = HashSet::new();
    for (table, column) in &rows {
        for (expected_table, expected_column) in REQUIRED {
            if table == expected_table && column == expected_column {
                set.insert((*expected_table, *expected_column));
            }
        }
        if table == "drives" && column == "ascent" {
            set.insert(("drives", "ascent"));
        }
    }
    Ok(set)
}

async fn column_exists(source: &PgPool, table: &str, column: &str) -> Result<bool, AppError> {
    let row: (bool,) = sqlx::query_as(
        "SELECT EXISTS (
            SELECT 1 FROM information_schema.columns
            WHERE table_schema = 'public' AND table_name = $1 AND column_name = $2
         )",
    )
    .bind(table)
    .bind(column)
    .fetch_one(source)
    .await?;
    Ok(row.0)
}

const REQUIRED: &[(&str, &str)] = &[
    ("cars", "id"),
    ("cars", "eid"),
    ("cars", "vid"),
    ("cars", "vin"),
    ("cars", "name"),
    ("cars", "model"),
    ("cars", "efficiency"),
    ("cars", "trim_badging"),
    ("cars", "marketing_name"),
    ("cars", "exterior_color"),
    ("cars", "wheel_type"),
    ("cars", "spoiler_type"),
    ("addresses", "id"),
    ("addresses", "display_name"),
    ("addresses", "latitude"),
    ("addresses", "longitude"),
    ("addresses", "city"),
    ("addresses", "country"),
    ("addresses", "raw"),
    ("geofences", "id"),
    ("geofences", "name"),
    ("geofences", "latitude"),
    ("geofences", "longitude"),
    ("geofences", "radius"),
    ("geofences", "cost_per_unit"),
    ("geofences", "session_fee"),
    ("geofences", "billing_type"),
    ("drives", "id"),
    ("drives", "car_id"),
    ("drives", "start_date"),
    ("drives", "end_date"),
    ("drives", "distance"),
    ("drives", "duration_min"),
    ("drives", "start_km"),
    ("drives", "end_km"),
    ("drives", "speed_max"),
    ("drives", "power_max"),
    ("drives", "power_min"),
    ("drives", "start_rated_range_km"),
    ("drives", "end_rated_range_km"),
    ("drives", "start_ideal_range_km"),
    ("drives", "end_ideal_range_km"),
    ("drives", "start_address_id"),
    ("drives", "end_address_id"),
    ("drives", "start_geofence_id"),
    ("drives", "end_geofence_id"),
    ("drives", "start_position_id"),
    ("drives", "end_position_id"),
    ("positions", "id"),
    ("positions", "car_id"),
    ("positions", "drive_id"),
    ("positions", "date"),
    ("positions", "latitude"),
    ("positions", "longitude"),
    ("positions", "speed"),
    ("positions", "power"),
    ("positions", "odometer"),
    ("positions", "elevation"),
    ("positions", "battery_level"),
    ("positions", "usable_battery_level"),
    ("positions", "ideal_battery_range_km"),
    ("positions", "rated_battery_range_km"),
    ("positions", "est_battery_range_km"),
    ("positions", "inside_temp"),
    ("positions", "outside_temp"),
    ("charging_processes", "id"),
    ("charging_processes", "car_id"),
    ("charging_processes", "start_date"),
    ("charging_processes", "end_date"),
    ("charging_processes", "charge_energy_added"),
    ("charging_processes", "charge_energy_used"),
    ("charging_processes", "start_battery_level"),
    ("charging_processes", "end_battery_level"),
    ("charging_processes", "position_id"),
    ("charging_processes", "address_id"),
    ("charging_processes", "geofence_id"),
    ("charging_processes", "cost"),
    ("charges", "id"),
    ("charges", "charging_process_id"),
    ("charges", "date"),
    ("charges", "battery_level"),
    ("charges", "charge_energy_added"),
    ("charges", "charger_power"),
    ("charges", "charger_voltage"),
    ("charges", "charger_actual_current"),
    ("charges", "fast_charger_present"),
    ("charges", "fast_charger_type"),
];

async fn copy_cars(dest: &PgPool, source: &PgPool, batch: i64) -> Result<u64, AppError> {
    let mut copied = 0u64;
    let mut progress =
        TableProgress::open(source, "cars", cursor(dest, "teslamate.cars").await?).await?;
    loop {
        let cursor = cursor(dest, "teslamate.cars").await?;
        let rows: Vec<TmCar> = sqlx::query_as(
            "SELECT id::bigint AS id, eid::bigint AS eid, vid::bigint AS vid, vin::text AS vin,
                    name::text AS name, model::text AS model, efficiency::float8 AS efficiency,
                    trim_badging::text AS trim_badging, marketing_name::text AS marketing_name,
                    exterior_color::text AS exterior_color, wheel_type::text AS wheel_type,
                    spoiler_type::text AS spoiler_type, display_priority::integer AS display_priority
             FROM cars WHERE cars.id > $1::int2 ORDER BY cars.id LIMIT $2",
        )
        .bind(cursor)
        .bind(batch)
        .fetch_all(source)
        .await?;
        if rows.is_empty() {
            break;
        }
        let last = rows.last().map_or(cursor, |row| row.id);
        let mut tx = dest.begin().await?;
        for row in &rows {
            let local: (i64,) = sqlx::query_as(
                "INSERT INTO vehicles (
                    vin, tesla_id, legacy_vehicle_id, name, model, efficiency_kwh_per_km,
                    trim_badging, marketing_name, exterior_color, wheel_type, spoiler_type,
                    display_priority, source, external_id
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
                 ON CONFLICT (vin) DO UPDATE SET
                    tesla_id = COALESCE(vehicles.tesla_id, EXCLUDED.tesla_id),
                    legacy_vehicle_id = COALESCE(vehicles.legacy_vehicle_id, EXCLUDED.legacy_vehicle_id),
                    name = COALESCE(vehicles.name, EXCLUDED.name),
                    model = COALESCE(vehicles.model, EXCLUDED.model),
                    efficiency_kwh_per_km = COALESCE(EXCLUDED.efficiency_kwh_per_km, vehicles.efficiency_kwh_per_km),
                    trim_badging = COALESCE(vehicles.trim_badging, EXCLUDED.trim_badging),
                    marketing_name = COALESCE(vehicles.marketing_name, EXCLUDED.marketing_name),
                    exterior_color = COALESCE(vehicles.exterior_color, EXCLUDED.exterior_color),
                    wheel_type = COALESCE(vehicles.wheel_type, EXCLUDED.wheel_type),
                    spoiler_type = COALESCE(vehicles.spoiler_type, EXCLUDED.spoiler_type),
                    display_priority = COALESCE(vehicles.display_priority, EXCLUDED.display_priority),
                    updated_at = now()
                 RETURNING id",
            )
            .bind(&row.vin)
            .bind(row.eid)
            .bind(row.vid)
            .bind(&row.name)
            .bind(&row.model)
            .bind(row.efficiency)
            .bind(&row.trim_badging)
            .bind(&row.marketing_name)
            .bind(&row.exterior_color)
            .bind(&row.wheel_type)
            .bind(&row.spoiler_type)
            .bind(row.display_priority)
            .bind(SOURCE)
            .bind(row.id.to_string())
            .fetch_one(&mut *tx)
            .await?;
            save_link(&mut tx, "car", &row.id.to_string(), local.0).await?;
            copied += 1;
        }
        save_cursor(&mut tx, "teslamate.cars", last).await?;
        tx.commit().await?;
        progress.advance(rows.len() as u64);
    }
    progress.finish();
    Ok(copied)
}

#[derive(sqlx::FromRow)]
struct TmCar {
    id: i64,
    eid: i64,
    vid: i64,
    vin: String,
    name: Option<String>,
    model: Option<String>,
    efficiency: Option<f64>,
    trim_badging: Option<String>,
    marketing_name: Option<String>,
    exterior_color: Option<String>,
    wheel_type: Option<String>,
    spoiler_type: Option<String>,
    display_priority: Option<i32>,
}

async fn copy_addresses(dest: &PgPool, source: &PgPool, batch: i64) -> Result<u64, AppError> {
    let mut copied = 0u64;
    let mut progress = TableProgress::open(
        source,
        "addresses",
        cursor(dest, "teslamate.addresses").await?,
    )
    .await?;
    loop {
        let cursor = cursor(dest, "teslamate.addresses").await?;
        let rows: Vec<TmAddress> = sqlx::query_as(
            "SELECT id::bigint AS id, display_name::text AS display_name, name::text AS name,
                    house_number::text AS house_number, road::text AS road,
                    neighbourhood::text AS neighbourhood, city::text AS city, county::text AS county,
                    state::text AS state, state_district::text AS state_district,
                    postcode::text AS postcode, country::text AS country,
                    latitude::float8 AS latitude, longitude::float8 AS longitude,
                    osm_id::bigint AS osm_id, osm_type::text AS osm_type, raw::jsonb AS raw
             FROM addresses WHERE addresses.id > $1::int4 ORDER BY addresses.id LIMIT $2",
        )
        .bind(cursor)
        .bind(batch)
        .fetch_all(source)
        .await?;
        if rows.is_empty() {
            break;
        }
        let last = rows.last().map_or(cursor, |row| row.id);
        let mut tx = dest.begin().await?;
        for row in &rows {
            let local: (i64,) = sqlx::query_as(
                "INSERT INTO addresses (
                    latitude, longitude, display_name, name, house_number, road, neighbourhood,
                    city, county, state, state_district, postcode, country, osm_id, osm_type, raw,
                    source, external_id
                 ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18
                 )
                 ON CONFLICT (source, external_id) WHERE external_id IS NOT NULL DO UPDATE SET
                    display_name = EXCLUDED.display_name
                 RETURNING id",
            )
            .bind(row.latitude)
            .bind(row.longitude)
            .bind(&row.display_name)
            .bind(&row.name)
            .bind(&row.house_number)
            .bind(&row.road)
            .bind(&row.neighbourhood)
            .bind(&row.city)
            .bind(&row.county)
            .bind(&row.state)
            .bind(&row.state_district)
            .bind(&row.postcode)
            .bind(&row.country)
            .bind(row.osm_id)
            .bind(&row.osm_type)
            .bind(&row.raw)
            .bind(SOURCE)
            .bind(row.id.to_string())
            .fetch_one(&mut *tx)
            .await?;
            save_link(&mut tx, "address", &row.id.to_string(), local.0).await?;
            copied += 1;
        }
        save_cursor(&mut tx, "teslamate.addresses", last).await?;
        tx.commit().await?;
        progress.advance(rows.len() as u64);
    }
    progress.finish();
    Ok(copied)
}

#[derive(sqlx::FromRow)]
struct TmAddress {
    id: i64,
    display_name: Option<String>,
    name: Option<String>,
    house_number: Option<String>,
    road: Option<String>,
    neighbourhood: Option<String>,
    city: Option<String>,
    county: Option<String>,
    state: Option<String>,
    state_district: Option<String>,
    postcode: Option<String>,
    country: Option<String>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    osm_id: Option<i64>,
    osm_type: Option<String>,
    raw: Option<serde_json::Value>,
}

async fn copy_geofences(dest: &PgPool, source: &PgPool, batch: i64) -> Result<u64, AppError> {
    let mut copied = 0u64;
    let mut progress = TableProgress::open(
        source,
        "geofences",
        cursor(dest, "teslamate.geofences").await?,
    )
    .await?;
    loop {
        let cursor = cursor(dest, "teslamate.geofences").await?;
        let rows: Vec<TmFence> = sqlx::query_as(
            "SELECT id::bigint AS id, name::text AS name, latitude::float8 AS latitude,
                    longitude::float8 AS longitude, radius::float8 AS radius,
                    cost_per_unit::float8 AS cost_per_unit, session_fee::float8 AS session_fee,
                    billing_type::text AS billing_type
             FROM geofences WHERE geofences.id > $1::int4 ORDER BY geofences.id LIMIT $2",
        )
        .bind(cursor)
        .bind(batch)
        .fetch_all(source)
        .await?;
        if rows.is_empty() {
            break;
        }
        let last = rows.last().map_or(cursor, |row| row.id);
        let mut tx = dest.begin().await?;
        for row in &rows {
            let local: (i64,) = sqlx::query_as(
                "INSERT INTO geofences (
                    name, latitude, longitude, radius_m, billing_type, cost_per_unit, session_fee,
                    source, external_id
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                 ON CONFLICT (source, external_id) WHERE external_id IS NOT NULL DO UPDATE SET
                    name = EXCLUDED.name,
                    radius_m = EXCLUDED.radius_m,
                    cost_per_unit = EXCLUDED.cost_per_unit,
                    session_fee = EXCLUDED.session_fee,
                    billing_type = EXCLUDED.billing_type,
                    updated_at = now()
                 RETURNING id",
            )
            .bind(&row.name)
            .bind(row.latitude)
            .bind(row.longitude)
            .bind(row.radius.max(1.0))
            .bind(&row.billing_type)
            .bind(row.cost_per_unit)
            .bind(row.session_fee)
            .bind(SOURCE)
            .bind(row.id.to_string())
            .fetch_one(&mut *tx)
            .await?;
            save_link(&mut tx, "geofence", &row.id.to_string(), local.0).await?;
            copied += 1;
        }
        save_cursor(&mut tx, "teslamate.geofences", last).await?;
        tx.commit().await?;
        progress.advance(rows.len() as u64);
    }
    progress.finish();
    Ok(copied)
}

#[derive(sqlx::FromRow)]
struct TmFence {
    id: i64,
    name: String,
    latitude: f64,
    longitude: f64,
    radius: f64,
    cost_per_unit: Option<f64>,
    session_fee: Option<f64>,
    billing_type: Option<String>,
}

#[allow(clippy::too_many_arguments)]
async fn copy_drives(
    dest: &PgPool,
    source: &PgPool,
    batch: i64,
    ascent: bool,
    cars: &HashMap<i64, i64>,
    addresses: &HashMap<i64, i64>,
    fences: &HashMap<i64, i64>,
    efficiency: &HashMap<i64, Option<f64>>,
) -> Result<u64, AppError> {
    let ascent_sql = if ascent {
        "d.ascent::integer"
    } else {
        "NULL::integer"
    };
    let descent_sql = if ascent {
        "d.descent::integer"
    } else {
        "NULL::integer"
    };
    let sql = format!(
        "SELECT d.id::bigint AS id, d.car_id::bigint AS car_id,
                (d.start_date AT TIME ZONE 'UTC') AS start_date,
                (d.end_date AT TIME ZONE 'UTC') AS end_date,
                d.distance::float8 AS distance, d.duration_min::integer AS duration_min,
                d.start_km::float8 AS start_km, d.end_km::float8 AS end_km,
                d.speed_max::integer AS speed_max, d.power_max::integer AS power_max,
                d.power_min::integer AS power_min,
                d.start_ideal_range_km::float8 AS start_ideal_range_km,
                d.end_ideal_range_km::float8 AS end_ideal_range_km,
                d.start_rated_range_km::float8 AS start_rated_range_km,
                d.end_rated_range_km::float8 AS end_rated_range_km,
                d.inside_temp_avg::float8 AS inside_temp_avg,
                d.outside_temp_avg::float8 AS outside_temp_avg,
                d.start_address_id::bigint AS start_address_id,
                d.end_address_id::bigint AS end_address_id,
                d.start_geofence_id::bigint AS start_geofence_id,
                d.end_geofence_id::bigint AS end_geofence_id,
                sp.latitude::float8 AS start_latitude, sp.longitude::float8 AS start_longitude,
                ep.latitude::float8 AS end_latitude, ep.longitude::float8 AS end_longitude,
                sp.battery_level::integer AS start_battery_level,
                sp.usable_battery_level::integer AS start_usable_battery_level,
                ep.battery_level::integer AS end_battery_level,
                ep.usable_battery_level::integer AS end_usable_battery_level,
                {ascent_sql} AS ascent, {descent_sql} AS descent
         FROM drives d
         LEFT JOIN positions sp ON sp.id = d.start_position_id
         LEFT JOIN positions ep ON ep.id = d.end_position_id
         WHERE d.id > $1::int4 ORDER BY d.id LIMIT $2"
    );
    let mut copied = 0u64;
    let mut progress =
        TableProgress::open(source, "drives", cursor(dest, "teslamate.drives").await?).await?;
    loop {
        let cursor = cursor(dest, "teslamate.drives").await?;
        let rows: Vec<TmDrive> = sqlx::query_as(&sql)
            .bind(cursor)
            .bind(batch)
            .fetch_all(source)
            .await?;
        if rows.is_empty() {
            break;
        }
        let last = rows.last().map_or(cursor, |row| row.id);
        let mut tx = dest.begin().await?;
        for row in &rows {
            let Some(vehicle_id) = cars.get(&row.car_id).copied() else {
                return Err(AppError::message(format!(
                    "drive {} references unknown car {}",
                    row.id, row.car_id
                )));
            };
            let distance = row.distance.filter(|value| value.is_finite()).or_else(|| {
                let (Some(start), Some(end)) = (row.start_km, row.end_km) else {
                    return None;
                };
                (end >= start).then_some(end - start)
            });
            let duration = duration_s(row.start_date, row.end_date, row.duration_min);
            let energy = range_energy(
                row.start_rated_range_km,
                row.end_rated_range_km,
                efficiency.get(&row.car_id).copied().flatten(),
            );
            let avg = match (distance, duration) {
                (Some(distance), Some(seconds)) if seconds > 0 => {
                    Some(distance / (f64::from(seconds) / 3_600.0))
                }
                _ => None,
            };
            let local: (i64,) = sqlx::query_as(
                "INSERT INTO drives (
                    vehicle_id, start_at, end_at, start_latitude, start_longitude, end_latitude,
                    end_longitude, start_odometer_km, end_odometer_km, distance_km, duration_s,
                    start_rated_range_km, end_rated_range_km, start_ideal_range_km, end_ideal_range_km,
                    energy_used_kwh, max_speed_kmh, avg_speed_kmh, max_power_kw, min_power_kw,
                    elevation_gain_m, elevation_loss_m, inside_temp_avg_c, outside_temp_avg_c,
                    start_address_id, end_address_id, start_geofence_id, end_geofence_id,
                    source, external_id, start_battery_level, end_battery_level
                 ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17,
                    $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30, $31, $32
                 )
                 ON CONFLICT (source, external_id) WHERE external_id IS NOT NULL DO UPDATE SET
                    end_at = EXCLUDED.end_at,
                    distance_km = EXCLUDED.distance_km,
                    duration_s = EXCLUDED.duration_s,
                    energy_used_kwh = EXCLUDED.energy_used_kwh,
                    elevation_gain_m = EXCLUDED.elevation_gain_m,
                    elevation_loss_m = EXCLUDED.elevation_loss_m,
                    start_battery_level = COALESCE(drives.start_battery_level, EXCLUDED.start_battery_level),
                    end_battery_level = COALESCE(drives.end_battery_level, EXCLUDED.end_battery_level)
                 RETURNING id",
            )
            .bind(vehicle_id)
            .bind(row.start_date)
            .bind(row.end_date)
            .bind(row.start_latitude)
            .bind(row.start_longitude)
            .bind(row.end_latitude)
            .bind(row.end_longitude)
            .bind(row.start_km)
            .bind(row.end_km)
            .bind(distance)
            .bind(duration)
            .bind(row.start_rated_range_km)
            .bind(row.end_rated_range_km)
            .bind(row.start_ideal_range_km)
            .bind(row.end_ideal_range_km)
            .bind(energy)
            .bind(row.speed_max.map(f64::from))
            .bind(avg)
            .bind(row.power_max.map(f64::from))
            .bind(row.power_min.map(f64::from))
            .bind(row.ascent.map(f64::from))
            .bind(row.descent.map(f64::from))
            .bind(row.inside_temp_avg)
            .bind(row.outside_temp_avg)
            .bind(row.start_address_id.and_then(|id| addresses.get(&id).copied()))
            .bind(row.end_address_id.and_then(|id| addresses.get(&id).copied()))
            .bind(row.start_geofence_id.and_then(|id| fences.get(&id).copied()))
            .bind(row.end_geofence_id.and_then(|id| fences.get(&id).copied()))
            .bind(SOURCE)
            .bind(row.id.to_string())
            .bind(position_soc(
                row.start_battery_level,
                row.start_usable_battery_level,
            ))
            .bind(position_soc(
                row.end_battery_level,
                row.end_usable_battery_level,
            ))
            .fetch_one(&mut *tx)
            .await?;
            save_link(&mut tx, "drive", &row.id.to_string(), local.0).await?;
            copied += 1;
        }
        save_cursor(&mut tx, "teslamate.drives", last).await?;
        tx.commit().await?;
        progress.advance(rows.len() as u64);
    }
    progress.finish();
    Ok(copied)
}

#[derive(sqlx::FromRow)]
struct TmDrive {
    id: i64,
    car_id: i64,
    start_date: DateTime<Utc>,
    end_date: Option<DateTime<Utc>>,
    distance: Option<f64>,
    duration_min: Option<i32>,
    start_km: Option<f64>,
    end_km: Option<f64>,
    speed_max: Option<i32>,
    power_max: Option<i32>,
    power_min: Option<i32>,
    start_ideal_range_km: Option<f64>,
    end_ideal_range_km: Option<f64>,
    start_rated_range_km: Option<f64>,
    end_rated_range_km: Option<f64>,
    inside_temp_avg: Option<f64>,
    outside_temp_avg: Option<f64>,
    start_address_id: Option<i64>,
    end_address_id: Option<i64>,
    start_geofence_id: Option<i64>,
    end_geofence_id: Option<i64>,
    start_latitude: Option<f64>,
    start_longitude: Option<f64>,
    end_latitude: Option<f64>,
    end_longitude: Option<f64>,
    ascent: Option<i32>,
    descent: Option<i32>,
    start_battery_level: Option<i32>,
    start_usable_battery_level: Option<i32>,
    end_battery_level: Option<i32>,
    end_usable_battery_level: Option<i32>,
}

fn position_soc(level: Option<i32>, usable: Option<i32>) -> Option<f64> {
    level.or(usable).map(f64::from)
}

async fn copy_charges(
    dest: &PgPool,
    source: &PgPool,
    batch: i64,
    cars: &HashMap<i64, i64>,
    addresses: &HashMap<i64, i64>,
    fences: &HashMap<i64, i64>,
) -> Result<u64, AppError> {
    let mut copied = 0u64;
    let mut progress = TableProgress::open(
        source,
        "charging_processes",
        cursor(dest, "teslamate.charges").await?,
    )
    .await?;
    loop {
        let cursor = cursor(dest, "teslamate.charges").await?;
        let rows: Vec<TmProcess> = sqlx::query_as(
            "SELECT c.id::bigint AS id, c.car_id::bigint AS car_id,
                    (c.start_date AT TIME ZONE 'UTC') AS start_date,
                    (c.end_date AT TIME ZONE 'UTC') AS end_date,
                    c.charge_energy_added::float8 AS charge_energy_added,
                    c.charge_energy_used::float8 AS charge_energy_used,
                    c.start_battery_level::integer AS start_battery_level,
                    c.end_battery_level::integer AS end_battery_level,
                    c.start_rated_range_km::float8 AS start_rated_range_km,
                    c.end_rated_range_km::float8 AS end_rated_range_km,
                    c.start_ideal_range_km::float8 AS start_ideal_range_km,
                    c.end_ideal_range_km::float8 AS end_ideal_range_km,
                    c.duration_min::integer AS duration_min,
                    c.address_id::bigint AS address_id,
                    c.geofence_id::bigint AS geofence_id, c.cost::float8 AS cost,
                    c.outside_temp_avg::float8 AS outside_temp_avg,
                    p.latitude::float8 AS latitude, p.longitude::float8 AS longitude
             FROM charging_processes c
             LEFT JOIN positions p ON p.id = c.position_id
             WHERE c.id > $1::int4 ORDER BY c.id LIMIT $2",
        )
        .bind(cursor)
        .bind(batch)
        .fetch_all(source)
        .await?;
        if rows.is_empty() {
            break;
        }
        let last = rows.last().map_or(cursor, |row| row.id);
        let mut tx = dest.begin().await?;
        for row in &rows {
            let Some(vehicle_id) = cars.get(&row.car_id).copied() else {
                return Err(AppError::message(format!(
                    "charge {} references unknown car {}",
                    row.id, row.car_id
                )));
            };
            let local: (i64,) = sqlx::query_as(
                "INSERT INTO charges (
                    vehicle_id, start_at, end_at, energy_added_kwh, energy_used_kwh,
                    start_battery_level, end_battery_level, start_rated_range_km, end_rated_range_km,
                    start_ideal_range_km, end_ideal_range_km, latitude, longitude, address_id,
                    geofence_id, duration_s, cost, outside_temp_avg_c, source, external_id
                 ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20
                 )
                 ON CONFLICT (source, external_id) WHERE external_id IS NOT NULL DO UPDATE SET
                    end_at = EXCLUDED.end_at,
                    energy_added_kwh = EXCLUDED.energy_added_kwh,
                    energy_used_kwh = EXCLUDED.energy_used_kwh,
                    cost = EXCLUDED.cost
                 RETURNING id",
            )
            .bind(vehicle_id)
            .bind(row.start_date)
            .bind(row.end_date)
            .bind(row.charge_energy_added)
            .bind(row.charge_energy_used)
            .bind(row.start_battery_level.map(f64::from))
            .bind(row.end_battery_level.map(f64::from))
            .bind(row.start_rated_range_km)
            .bind(row.end_rated_range_km)
            .bind(row.start_ideal_range_km)
            .bind(row.end_ideal_range_km)
            .bind(row.latitude)
            .bind(row.longitude)
            .bind(row.address_id.and_then(|id| addresses.get(&id).copied()))
            .bind(row.geofence_id.and_then(|id| fences.get(&id).copied()))
            .bind(duration_s(row.start_date, row.end_date, row.duration_min))
            .bind(row.cost)
            .bind(row.outside_temp_avg)
            .bind(SOURCE)
            .bind(row.id.to_string())
            .fetch_one(&mut *tx)
            .await?;
            save_link(&mut tx, "charge", &row.id.to_string(), local.0).await?;
            copied += 1;
        }
        save_cursor(&mut tx, "teslamate.charges", last).await?;
        tx.commit().await?;
        progress.advance(rows.len() as u64);
    }
    progress.finish();
    Ok(copied)
}

#[derive(sqlx::FromRow)]
struct TmProcess {
    id: i64,
    car_id: i64,
    start_date: DateTime<Utc>,
    end_date: Option<DateTime<Utc>>,
    charge_energy_added: Option<f64>,
    charge_energy_used: Option<f64>,
    start_battery_level: Option<i32>,
    end_battery_level: Option<i32>,
    start_rated_range_km: Option<f64>,
    end_rated_range_km: Option<f64>,
    start_ideal_range_km: Option<f64>,
    end_ideal_range_km: Option<f64>,
    duration_min: Option<i32>,
    address_id: Option<i64>,
    geofence_id: Option<i64>,
    cost: Option<f64>,
    outside_temp_avg: Option<f64>,
    latitude: Option<f64>,
    longitude: Option<f64>,
}

async fn copy_charge_samples(
    dest: &PgPool,
    source: &PgPool,
    batch: i64,
    charges: &HashMap<i64, i64>,
) -> Result<u64, AppError> {
    let mut copied = 0u64;
    let mut progress = TableProgress::open(
        source,
        "charges",
        cursor(dest, "teslamate.charge_samples").await?,
    )
    .await?;
    loop {
        let cursor = cursor(dest, "teslamate.charge_samples").await?;
        let rows: Vec<TmSample> = sqlx::query_as(
            "SELECT id::bigint AS id, charging_process_id::bigint AS charging_process_id,
                    (date AT TIME ZONE 'UTC') AS date,
                    battery_level::integer AS battery_level,
                    usable_battery_level::integer AS usable_battery_level,
                    charge_energy_added::float8 AS charge_energy_added,
                    charger_power::integer AS charger_power,
                    charger_voltage::integer AS charger_voltage,
                    charger_actual_current::integer AS charger_actual_current,
                    ideal_battery_range_km::float8 AS ideal_battery_range_km,
                    rated_battery_range_km::float8 AS rated_battery_range_km,
                    outside_temp::float8 AS outside_temp,
                    fast_charger_present, fast_charger_type::text AS fast_charger_type
             FROM charges WHERE charges.id > $1::int4 ORDER BY charges.id LIMIT $2",
        )
        .bind(cursor)
        .bind(batch)
        .fetch_all(source)
        .await?;
        if rows.is_empty() {
            break;
        }
        let last = rows.last().map_or(cursor, |row| row.id);
        let mut prepared = Vec::with_capacity(rows.len());
        // First DC sample wins, matching the previous per-row COALESCE.
        let mut dc_type: HashMap<i64, Option<String>> = HashMap::new();
        for row in &rows {
            let Some(charge_id) = charges.get(&row.charging_process_id).copied() else {
                continue;
            };
            if row.fast_charger_present == Some(true) {
                dc_type
                    .entry(charge_id)
                    .or_insert_with(|| row.fast_charger_type.clone());
            }
            prepared.push(SampleInsert {
                charge_id,
                recorded_at: row.date,
                battery_level: row.battery_level.map(f64::from),
                usable_battery_level: row.usable_battery_level.map(f64::from),
                charger_power_kw: row.charger_power.map(f64::from),
                charger_voltage: row.charger_voltage.map(f64::from),
                charger_current: row.charger_actual_current.map(f64::from),
                charge_energy_added_kwh: row.charge_energy_added,
                ideal_range_km: row.ideal_battery_range_km,
                rated_range_km: row.rated_battery_range_km,
                outside_temp_c: row.outside_temp,
                external_id: row.id.to_string(),
            });
        }
        let mut tx = dest.begin().await?;
        if !prepared.is_empty() {
            let mut builder: QueryBuilder<Postgres> = QueryBuilder::new(
                "INSERT INTO charge_samples (
                    charge_id, recorded_at, battery_level, usable_battery_level, charger_power_kw,
                    charger_voltage, charger_current, charge_energy_added_kwh, ideal_range_km,
                    rated_range_km, outside_temp_c, source, external_id
                ) ",
            );
            builder.push_values(prepared.iter(), |mut row, item| {
                row.push_bind(item.charge_id)
                    .push_bind(item.recorded_at)
                    .push_bind(item.battery_level)
                    .push_bind(item.usable_battery_level)
                    .push_bind(item.charger_power_kw)
                    .push_bind(item.charger_voltage)
                    .push_bind(item.charger_current)
                    .push_bind(item.charge_energy_added_kwh)
                    .push_bind(item.ideal_range_km)
                    .push_bind(item.rated_range_km)
                    .push_bind(item.outside_temp_c)
                    .push_bind(SOURCE)
                    .push_bind(&item.external_id);
            });
            builder.push(
                " ON CONFLICT (source, external_id) WHERE external_id IS NOT NULL DO NOTHING",
            );
            builder.build().execute(&mut *tx).await?;
        }
        if !dc_type.is_empty() {
            let pairs: Vec<(i64, Option<String>)> = dc_type.into_iter().collect();
            let mut builder: QueryBuilder<Postgres> = QueryBuilder::new(
                "UPDATE charges AS c SET is_dc = TRUE, charger_type = COALESCE(c.charger_type, v.ty) FROM (",
            );
            builder.push_values(pairs.iter(), |mut row, (id, ty)| {
                row.push_bind(*id).push_bind(ty);
            });
            builder.push(") AS v(id, ty) WHERE c.id = v.id");
            builder.build().execute(&mut *tx).await?;
        }
        save_cursor(&mut tx, "teslamate.charge_samples", last).await?;
        tx.commit().await?;
        copied += prepared.len() as u64;
        progress.advance(rows.len() as u64);
    }
    progress.finish();
    Ok(copied)
}

struct SampleInsert {
    charge_id: i64,
    recorded_at: DateTime<Utc>,
    battery_level: Option<f64>,
    usable_battery_level: Option<f64>,
    charger_power_kw: Option<f64>,
    charger_voltage: Option<f64>,
    charger_current: Option<f64>,
    charge_energy_added_kwh: Option<f64>,
    ideal_range_km: Option<f64>,
    rated_range_km: Option<f64>,
    outside_temp_c: Option<f64>,
    external_id: String,
}

#[derive(sqlx::FromRow)]
struct TmSample {
    id: i64,
    charging_process_id: i64,
    date: DateTime<Utc>,
    battery_level: Option<i32>,
    usable_battery_level: Option<i32>,
    charge_energy_added: Option<f64>,
    charger_power: Option<i32>,
    charger_voltage: Option<i32>,
    charger_actual_current: Option<i32>,
    ideal_battery_range_km: Option<f64>,
    rated_battery_range_km: Option<f64>,
    outside_temp: Option<f64>,
    fast_charger_present: Option<bool>,
    fast_charger_type: Option<String>,
}

/// Rows fetched per positions keyset page. A page is a few megabytes.
pub(crate) const POSITIONS_PAGE: i64 = 5_000;

/// Bind parameters in one positions `INSERT` row (see [`insert_positions`]).
const POSITION_INSERT_PARAMS: usize = 18;

/// Postgres rejects a statement with more than 65535 parameters.
const POSITION_INSERT_CHUNK: usize = 65_535 / POSITION_INSERT_PARAMS;

const _: () = assert!(POSITION_INSERT_CHUNK >= 1);
const _: () = assert!(POSITION_INSERT_CHUNK * POSITION_INSERT_PARAMS <= 65_535);

/// TeslaMate `positions.id` is `integer` (int4). The cursor is stored as bigint
/// and compared by casting the parameter, not the column. `ORDER BY` names the
/// table column so it does not resolve to the `id::bigint` output alias.
pub(crate) const POSITIONS_PAGE_SQL: &str =
    "SELECT id::bigint AS id, car_id::bigint AS car_id, drive_id::bigint AS drive_id,
                    (date AT TIME ZONE 'UTC') AS date,
                    latitude::float8 AS latitude, longitude::float8 AS longitude,
                    speed::integer AS speed, power::integer AS power,
                    odometer::float8 AS odometer, elevation::integer AS elevation,
                    battery_level::integer AS battery_level,
                    usable_battery_level::integer AS usable_battery_level,
                    ideal_battery_range_km::float8 AS ideal_battery_range_km,
                    rated_battery_range_km::float8 AS rated_battery_range_km,
                    est_battery_range_km::float8 AS est_battery_range_km,
                    inside_temp::float8 AS inside_temp, outside_temp::float8 AS outside_temp
             FROM positions WHERE positions.id > $1::int4 ORDER BY positions.id LIMIT $2";

struct PositionInsert {
    vehicle_id: i64,
    recorded_at: DateTime<Utc>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    speed_kmh: Option<f64>,
    power_kw: Option<f64>,
    odometer_km: Option<f64>,
    elevation_m: Option<f64>,
    battery_level: Option<f64>,
    usable_battery_level: Option<f64>,
    ideal_range_km: Option<f64>,
    rated_range_km: Option<f64>,
    est_range_km: Option<f64>,
    inside_temp_c: Option<f64>,
    outside_temp_c: Option<f64>,
    drive_id: Option<i64>,
    external_id: String,
}

async fn copy_positions(
    dest: &PgPool,
    source: &PgPool,
    cars: &HashMap<i64, i64>,
    drives: &HashMap<i64, i64>,
) -> Result<u64, AppError> {
    let mut copied = 0u64;
    let mut progress = TableProgress::open(
        source,
        "positions",
        cursor(dest, "teslamate.positions").await?,
    )
    .await?;
    loop {
        let cursor = cursor(dest, "teslamate.positions").await?;
        // One page only. The page is dropped at the end of this iteration.
        let rows: Vec<TmPosition> = sqlx::query_as(POSITIONS_PAGE_SQL)
            .bind(cursor)
            .bind(POSITIONS_PAGE)
            .fetch_all(source)
            .await?;
        if rows.is_empty() {
            break;
        }
        let last = rows.last().map_or(cursor, |row| row.id);
        let mut prepared = Vec::with_capacity(rows.len());
        for row in &rows {
            let Some(vehicle_id) = cars.get(&row.car_id).copied() else {
                return Err(AppError::message(format!(
                    "position {} references unknown car {}",
                    row.id, row.car_id
                )));
            };
            prepared.push(PositionInsert {
                vehicle_id,
                recorded_at: row.date,
                latitude: row.latitude,
                longitude: row.longitude,
                speed_kmh: row.speed.map(f64::from),
                power_kw: row.power.map(f64::from),
                odometer_km: row.odometer,
                elevation_m: row.elevation.map(f64::from),
                battery_level: row.battery_level.map(f64::from),
                usable_battery_level: row.usable_battery_level.map(f64::from),
                ideal_range_km: row.ideal_battery_range_km,
                rated_range_km: row.rated_battery_range_km,
                est_range_km: row.est_battery_range_km,
                inside_temp_c: row.inside_temp,
                outside_temp_c: row.outside_temp,
                drive_id: row.drive_id.and_then(|id| drives.get(&id).copied()),
                external_id: row.id.to_string(),
            });
        }
        let mut tx = dest.begin().await?;
        insert_positions(&mut tx, &prepared).await?;
        save_cursor(&mut tx, "teslamate.positions", last).await?;
        tx.commit().await?;
        copied += prepared.len() as u64;
        progress.advance(rows.len() as u64);
    }
    progress.finish();
    Ok(copied)
}

async fn insert_positions(
    tx: &mut Transaction<'_, Postgres>,
    prepared: &[PositionInsert],
) -> Result<(), AppError> {
    for chunk in prepared.chunks(POSITION_INSERT_CHUNK) {
        let mut builder: QueryBuilder<Postgres> = QueryBuilder::new(
            "INSERT INTO positions (
                vehicle_id, recorded_at, latitude, longitude, speed_kmh, power_kw, odometer_km,
                elevation_m, battery_level, usable_battery_level, ideal_range_km, rated_range_km,
                est_range_km, inside_temp_c, outside_temp_c, drive_id, source, external_id
            ) ",
        );
        builder.push_values(chunk, |mut row, item| {
            // POSITION_INSERT_PARAMS must match this bind list.
            row.push_bind(item.vehicle_id)
                .push_bind(item.recorded_at)
                .push_bind(item.latitude)
                .push_bind(item.longitude)
                .push_bind(item.speed_kmh)
                .push_bind(item.power_kw)
                .push_bind(item.odometer_km)
                .push_bind(item.elevation_m)
                .push_bind(item.battery_level)
                .push_bind(item.usable_battery_level)
                .push_bind(item.ideal_range_km)
                .push_bind(item.rated_range_km)
                .push_bind(item.est_range_km)
                .push_bind(item.inside_temp_c)
                .push_bind(item.outside_temp_c)
                .push_bind(item.drive_id)
                .push_bind(SOURCE)
                .push_bind(&item.external_id);
        });
        builder.push(
            " ON CONFLICT (source, external_id, recorded_at) WHERE external_id IS NOT NULL DO NOTHING",
        );
        builder.build().execute(pg(tx)).await?;
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct TmPosition {
    id: i64,
    car_id: i64,
    drive_id: Option<i64>,
    date: DateTime<Utc>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    speed: Option<i32>,
    power: Option<i32>,
    odometer: Option<f64>,
    elevation: Option<i32>,
    battery_level: Option<i32>,
    usable_battery_level: Option<i32>,
    ideal_battery_range_km: Option<f64>,
    rated_battery_range_km: Option<f64>,
    est_battery_range_km: Option<f64>,
    inside_temp: Option<f64>,
    outside_temp: Option<f64>,
}

fn duration_s(
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
    fallback_min: Option<i32>,
) -> Option<i32> {
    if let Some(end) = end {
        let seconds = (end - start).num_seconds().max(0);
        return Some(i32::try_from(seconds).unwrap_or(i32::MAX));
    }
    fallback_min.and_then(|minutes| minutes.checked_mul(60))
}

/// Fills missing drive battery levels from the earliest and latest stored positions.
///
/// Already-set values are left alone. Closed drives get an end level. Open drives
/// only get a start level.
pub(crate) async fn backfill_drive_battery(pool: &PgPool) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE drives AS d
         SET start_battery_level = src.level
         FROM (
             SELECT DISTINCT ON (drive_id)
                 drive_id,
                 COALESCE(battery_level, usable_battery_level) AS level
             FROM positions
             WHERE drive_id IS NOT NULL
             ORDER BY drive_id, recorded_at ASC, id ASC
         ) AS src
         WHERE d.id = src.drive_id
           AND d.start_battery_level IS NULL
           AND src.level IS NOT NULL",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "UPDATE drives AS d
         SET end_battery_level = src.level
         FROM (
             SELECT DISTINCT ON (drive_id)
                 drive_id,
                 COALESCE(battery_level, usable_battery_level) AS level
             FROM positions
             WHERE drive_id IS NOT NULL
             ORDER BY drive_id, recorded_at DESC, id DESC
         ) AS src
         WHERE d.id = src.drive_id
           AND d.end_at IS NOT NULL
           AND d.end_battery_level IS NULL
           AND src.level IS NOT NULL",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn cursor(pool: &PgPool, key: &str) -> Result<i64, AppError> {
    let row: Option<(i64,)> =
        sqlx::query_as("SELECT cursor_id FROM import_cursors WHERE source = $1")
            .bind(key)
            .fetch_optional(pool)
            .await?;
    Ok(row.map_or(0, |row| row.0))
}

async fn save_cursor(
    tx: &mut Transaction<'_, Postgres>,
    key: &str,
    id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO import_cursors (source, cursor_id) VALUES ($1, $2)
         ON CONFLICT (source) DO UPDATE SET cursor_id = EXCLUDED.cursor_id, updated_at = now()",
    )
    .bind(key)
    .bind(id)
    .execute(pg(tx))
    .await?;
    Ok(())
}

async fn save_link(
    tx: &mut Transaction<'_, Postgres>,
    entity: &str,
    external_id: &str,
    local_id: i64,
) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO import_links (entity, external_id, local_id) VALUES ($1, $2, $3)
         ON CONFLICT (entity, external_id) DO UPDATE SET local_id = EXCLUDED.local_id",
    )
    .bind(entity)
    .bind(external_id)
    .bind(local_id)
    .execute(pg(tx))
    .await?;
    Ok(())
}

fn pg<'a>(tx: &'a mut Transaction<'_, Postgres>) -> &'a mut sqlx::PgConnection {
    tx
}

async fn load_links(pool: &PgPool, entity: &str) -> Result<HashMap<i64, i64>, AppError> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT external_id, local_id FROM import_links WHERE entity = $1")
            .bind(entity)
            .fetch_all(pool)
            .await?;
    let mut map = HashMap::with_capacity(rows.len());
    for (external, local) in rows {
        if let Ok(id) = external.parse::<i64>() {
            map.insert(id, local);
        }
    }
    Ok(map)
}

fn approx_f64(value: u64) -> f64 {
    u32::try_from(value).map_or(f64::from(u32::MAX), f64::from)
}

struct TableProgress {
    label: &'static str,
    total: u64,
    done: u64,
    copied: u64,
    started: std::time::Instant,
    last_log: std::time::Instant,
}

impl TableProgress {
    async fn open(source: &PgPool, table: &'static str, cursor: i64) -> Result<Self, AppError> {
        let sql = match table {
            "cars" => {
                "SELECT COUNT(*)::bigint, COUNT(*) FILTER (WHERE cars.id <= $1::int2)::bigint FROM cars"
            }
            "addresses" => {
                "SELECT COUNT(*)::bigint, COUNT(*) FILTER (WHERE addresses.id <= $1::int4)::bigint FROM addresses"
            }
            "geofences" => {
                "SELECT COUNT(*)::bigint, COUNT(*) FILTER (WHERE geofences.id <= $1::int4)::bigint FROM geofences"
            }
            "drives" => {
                "SELECT COUNT(*)::bigint, COUNT(*) FILTER (WHERE drives.id <= $1::int4)::bigint FROM drives"
            }
            "charging_processes" => {
                "SELECT COUNT(*)::bigint, COUNT(*) FILTER (WHERE charging_processes.id <= $1::int4)::bigint FROM charging_processes"
            }
            "charges" => {
                "SELECT COUNT(*)::bigint, COUNT(*) FILTER (WHERE charges.id <= $1::int4)::bigint FROM charges"
            }
            "positions" => {
                "SELECT COUNT(*)::bigint, COUNT(*) FILTER (WHERE positions.id <= $1::int4)::bigint FROM positions"
            }
            other => {
                return Err(AppError::message(format!(
                    "internal error: no progress query for {other}"
                )));
            }
        };
        let (total, already): (i64, i64) =
            sqlx::query_as(sql).bind(cursor).fetch_one(source).await?;
        let total = u64::try_from(total).unwrap_or(0);
        let done = u64::try_from(already).unwrap_or(0);
        tracing::info!(table, total, already = done, "teslamate import table");
        let now = std::time::Instant::now();
        Ok(Self {
            label: table,
            total,
            done,
            copied: 0,
            started: now,
            last_log: now,
        })
    }

    fn advance(&mut self, rows: u64) {
        self.copied += rows;
        self.done = self.done.saturating_add(rows);
        if self.last_log.elapsed() >= std::time::Duration::from_secs(5) {
            self.log();
        }
    }

    fn finish(&mut self) {
        self.log();
    }

    fn log(&mut self) {
        self.last_log = std::time::Instant::now();
        let secs = self.started.elapsed().as_secs_f64();
        let rows_per_sec = if secs < 0.05 {
            0.0
        } else {
            approx_f64(self.copied) / secs
        };
        let percent = if self.total == 0 {
            100.0
        } else {
            (approx_f64(self.done) / approx_f64(self.total) * 100.0).min(100.0)
        };
        tracing::info!(
            table = self.label,
            done = self.done,
            total = self.total,
            percent,
            rows_per_sec,
            "teslamate import progress"
        );
    }
}

async fn load_efficiency(pool: &PgPool) -> Result<HashMap<i64, Option<f64>>, AppError> {
    let rows: Vec<(String, Option<f64>)> = sqlx::query_as(
        "SELECT l.external_id, v.efficiency_kwh_per_km
         FROM import_links l
         JOIN vehicles v ON v.id = l.local_id
         WHERE l.entity = 'car'",
    )
    .fetch_all(pool)
    .await?;
    let mut map = HashMap::new();
    for (external, efficiency) in rows {
        if let Ok(id) = external.parse::<i64>() {
            map.insert(id, efficiency);
        }
    }
    Ok(map)
}
