// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Edits that do not rewrite measured drive columns by hand.
//!
//! Recompute uses the same keep rule as closing a drive: at least two positions
//! and 0.01 km of odometer. Audit JSON drops coordinates.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::AppError;
use crate::identity;
use crate::state::{Sample, keep_closed_drive, summarize_drive};

const RESTORE_DAYS: i64 = 30;

pub(crate) struct DriveEdit {
    pub note: Option<String>,
    pub tag: Option<String>,
    pub start_address_id: Option<i64>,
    pub end_address_id: Option<i64>,
    pub start_geofence_id: Option<i64>,
    pub end_geofence_id: Option<i64>,
}

pub(crate) struct ChargeEdit {
    pub cost: Option<f64>,
    pub is_dc: Option<bool>,
    pub charger_type: Option<String>,
    pub geofence_id: Option<i64>,
    pub note: Option<String>,
}

pub(crate) enum PurgePositions {
    Detach,
    DeleteRange,
}

pub(crate) async fn edit_drive(
    pool: &PgPool,
    actor: i64,
    vehicle_id: i64,
    drive_id: i64,
    edit: &DriveEdit,
) -> Result<(), AppError> {
    let before = drive_json(pool, drive_id).await?;
    ensure_closed_drive(pool, vehicle_id, drive_id).await?;
    let changed = sqlx::query(
        "UPDATE drives SET
            note = $3, tag = $4,
            start_address_id = $5, end_address_id = $6,
            start_geofence_id = $7, end_geofence_id = $8
         WHERE id = $1 AND vehicle_id = $2 AND end_at IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(drive_id)
    .bind(vehicle_id)
    .bind(&edit.note)
    .bind(&edit.tag)
    .bind(edit.start_address_id)
    .bind(edit.end_address_id)
    .bind(edit.start_geofence_id)
    .bind(edit.end_geofence_id)
    .execute(pool)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::message("drive cannot be edited"));
    }
    let after = drive_json(pool, drive_id).await?;
    identity::audit(
        pool,
        Some(actor),
        "drive.edit",
        "ok",
        &pair(&before, &after),
    )
    .await;
    Ok(())
}

pub(crate) async fn edit_charge(
    pool: &PgPool,
    actor: i64,
    vehicle_id: i64,
    charge_id: i64,
    edit: &ChargeEdit,
) -> Result<(), AppError> {
    let before = charge_json(pool, charge_id).await?;
    let changed = sqlx::query(
        "UPDATE charges SET cost = $3, is_dc = $4, charger_type = $5, geofence_id = $6, note = $7
         WHERE id = $1 AND vehicle_id = $2 AND end_at IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(charge_id)
    .bind(vehicle_id)
    .bind(edit.cost)
    .bind(edit.is_dc)
    .bind(&edit.charger_type)
    .bind(edit.geofence_id)
    .bind(&edit.note)
    .execute(pool)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::message("charge cannot be edited"));
    }
    let after = charge_json(pool, charge_id).await?;
    identity::audit(
        pool,
        Some(actor),
        "charge.edit",
        "ok",
        &pair(&before, &after),
    )
    .await;
    Ok(())
}

pub(crate) async fn create_manual_charge(
    pool: &PgPool,
    actor: i64,
    vehicle_id: i64,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    edit: &ChargeEdit,
) -> Result<i64, AppError> {
    if end <= start {
        return Err(AppError::message("charge end must be after the start"));
    }
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO charges (
            vehicle_id, start_at, end_at, source, cost, is_dc, charger_type, geofence_id, note
         ) VALUES ($1, $2, $3, 'manual', $4, $5, $6, $7, $8)
         RETURNING id",
    )
    .bind(vehicle_id)
    .bind(start)
    .bind(end)
    .bind(edit.cost)
    .bind(edit.is_dc)
    .bind(&edit.charger_type)
    .bind(edit.geofence_id)
    .bind(&edit.note)
    .fetch_one(pool)
    .await?;
    let after = charge_json(pool, id.0).await?;
    identity::audit(
        pool,
        Some(actor),
        "charge.create",
        "ok",
        &pair("null", &after),
    )
    .await;
    Ok(id.0)
}

pub(crate) async fn merge_drives(
    pool: &PgPool,
    actor: i64,
    vehicle_id: i64,
    earlier: i64,
    later: i64,
) -> Result<(), AppError> {
    if earlier == later {
        return Err(AppError::message("choose two drives"));
    }
    let rows: Vec<(i64, DateTime<Utc>, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT id, start_at, end_at FROM drives
         WHERE vehicle_id = $1 AND id IN ($2, $3) AND deleted_at IS NULL",
    )
    .bind(vehicle_id)
    .bind(earlier)
    .bind(later)
    .fetch_all(pool)
    .await?;
    if rows.len() != 2 || rows.iter().any(|row| row.2.is_none()) {
        return Err(AppError::message("only closed drives can be merged"));
    }
    let mut ordered = rows;
    ordered.sort_by_key(|row| row.1);
    let (keep, drop_id) = (ordered[0].0, ordered[1].0);
    let between: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM drives
         WHERE vehicle_id = $1 AND deleted_at IS NULL AND id <> $2 AND id <> $3
           AND start_at > $4 AND start_at < $5",
    )
    .bind(vehicle_id)
    .bind(keep)
    .bind(drop_id)
    .bind(ordered[0].1)
    .bind(ordered[1].1)
    .fetch_one(pool)
    .await?;
    if between.0 > 0 {
        return Err(AppError::message("drives are not consecutive"));
    }
    let before = drive_json(pool, keep).await?;
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE positions SET drive_id = $1 WHERE drive_id = $2")
        .bind(keep)
        .bind(drop_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM drives WHERE id = $1")
        .bind(drop_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    recompute_drive(pool, actor, vehicle_id, keep).await?;
    let after = drive_json(pool, keep).await?;
    identity::audit(
        pool,
        Some(actor),
        "drive.merge",
        "ok",
        &pair(&before, &after),
    )
    .await;
    Ok(())
}

pub(crate) async fn split_drive(
    pool: &PgPool,
    actor: i64,
    vehicle_id: i64,
    drive_id: i64,
    at: DateTime<Utc>,
) -> Result<i64, AppError> {
    let span: Option<(DateTime<Utc>, DateTime<Utc>, String)> = sqlx::query_as(
        "SELECT start_at, end_at, source FROM drives
         WHERE id = $1 AND vehicle_id = $2 AND end_at IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(drive_id)
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await?;
    let Some((start, end, source)) = span else {
        return Err(AppError::message("drive cannot be split"));
    };
    if at <= start || at >= end {
        return Err(AppError::message("split time must be inside the drive"));
    }
    let before = drive_json(pool, drive_id).await?;
    let mut tx = pool.begin().await?;
    let created: (i64,) = sqlx::query_as(
        "INSERT INTO drives (vehicle_id, start_at, end_at, source)
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(vehicle_id)
    .bind(at)
    .bind(end)
    .bind(&source)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE positions SET drive_id = $1
         WHERE drive_id = $2 AND recorded_at >= $3 AND recorded_at <= $4",
    )
    .bind(created.0)
    .bind(drive_id)
    .bind(at)
    .bind(end)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    recompute_drive(pool, actor, vehicle_id, drive_id).await?;
    recompute_drive(pool, actor, vehicle_id, created.0).await?;
    let after = drive_json(pool, drive_id).await?;
    identity::audit(
        pool,
        Some(actor),
        "drive.split",
        "ok",
        &pair(&before, &after),
    )
    .await;
    Ok(created.0)
}

pub(crate) async fn recompute_drive(
    pool: &PgPool,
    actor: i64,
    vehicle_id: i64,
    drive_id: i64,
) -> Result<bool, AppError> {
    let before = drive_json(pool, drive_id).await?;
    let samples = load_samples(pool, drive_id).await?;
    if !keep_closed_drive(&samples) {
        let mut tx = pool.begin().await?;
        sqlx::query("UPDATE positions SET drive_id = NULL WHERE drive_id = $1")
            .bind(drive_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM drives WHERE id = $1 AND vehicle_id = $2")
            .bind(drive_id)
            .bind(vehicle_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        identity::audit(
            pool,
            Some(actor),
            "drive.recompute",
            "discarded",
            &pair(&before, "null"),
        )
        .await;
        return Ok(false);
    }
    let Some(summary) = summarize_drive(&samples, None) else {
        return Err(AppError::message("drive could not be recomputed"));
    };
    sqlx::query(
        "UPDATE drives SET
            start_at = $3, end_at = $4,
            start_latitude = $5, start_longitude = $6, end_latitude = $7, end_longitude = $8,
            start_odometer_km = $9, end_odometer_km = $10, distance_km = $11, duration_s = $12,
            start_battery_level = $13, end_battery_level = $14,
            start_rated_range_km = $15, end_rated_range_km = $16,
            start_ideal_range_km = $17, end_ideal_range_km = $18,
            energy_used_kwh = $19, max_speed_kmh = $20, avg_speed_kmh = $21,
            max_power_kw = $22, min_power_kw = $23,
            elevation_gain_m = $24, elevation_loss_m = $25,
            inside_temp_avg_c = $26, outside_temp_avg_c = $27
         WHERE id = $1 AND vehicle_id = $2 AND deleted_at IS NULL",
    )
    .bind(drive_id)
    .bind(vehicle_id)
    .bind(summary.start)
    .bind(summary.end)
    .bind(summary.start_latitude)
    .bind(summary.start_longitude)
    .bind(summary.end_latitude)
    .bind(summary.end_longitude)
    .bind(summary.start_odometer_km)
    .bind(summary.end_odometer_km)
    .bind(summary.distance_km)
    .bind(summary.duration_s)
    .bind(summary.start_battery_level)
    .bind(summary.end_battery_level)
    .bind(summary.start_rated_range_km)
    .bind(summary.end_rated_range_km)
    .bind(summary.start_ideal_range_km)
    .bind(summary.end_ideal_range_km)
    .bind(summary.energy_used_kwh)
    .bind(summary.max_speed_kmh)
    .bind(summary.avg_speed_kmh)
    .bind(summary.max_power_kw)
    .bind(summary.min_power_kw)
    .bind(summary.elevation_gain_m)
    .bind(summary.elevation_loss_m)
    .bind(summary.inside_temp_avg_c)
    .bind(summary.outside_temp_avg_c)
    .execute(pool)
    .await?;
    let after = drive_json(pool, drive_id).await?;
    identity::audit(
        pool,
        Some(actor),
        "drive.recompute",
        "ok",
        &pair(&before, &after),
    )
    .await;
    Ok(true)
}

pub(crate) async fn recompute_vehicle(
    pool: &PgPool,
    actor: i64,
    vehicle_id: i64,
) -> Result<u64, AppError> {
    let ids: Vec<(i64,)> = sqlx::query_as(
        "SELECT id FROM drives
         WHERE vehicle_id = $1 AND end_at IS NOT NULL AND deleted_at IS NULL
         ORDER BY start_at",
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await?;
    let mut kept = 0_u64;
    for (id,) in ids {
        if recompute_drive(pool, actor, vehicle_id, id).await? {
            kept += 1;
        }
    }
    Ok(kept)
}

pub(crate) async fn trash_drive(
    pool: &PgPool,
    actor: i64,
    vehicle_id: i64,
    drive_id: i64,
) -> Result<(), AppError> {
    let before = drive_json(pool, drive_id).await?;
    let changed = sqlx::query(
        "UPDATE drives SET deleted_at = now()
         WHERE id = $1 AND vehicle_id = $2 AND end_at IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(drive_id)
    .bind(vehicle_id)
    .execute(pool)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::message("drive cannot be deleted"));
    }
    identity::audit(
        pool,
        Some(actor),
        "drive.trash",
        "ok",
        &pair(&before, &drive_json(pool, drive_id).await?),
    )
    .await;
    Ok(())
}

pub(crate) async fn restore_drive(
    pool: &PgPool,
    actor: i64,
    vehicle_id: i64,
    drive_id: i64,
) -> Result<(), AppError> {
    let changed = sqlx::query(
        "UPDATE drives SET deleted_at = NULL
         WHERE id = $1 AND vehicle_id = $2 AND deleted_at IS NOT NULL
           AND deleted_at > now() - make_interval(days => $3::int)",
    )
    .bind(drive_id)
    .bind(vehicle_id)
    .bind(RESTORE_DAYS)
    .execute(pool)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::message("drive can no longer be restored"));
    }
    identity::audit(
        pool,
        Some(actor),
        "drive.restore",
        "ok",
        &drive_json(pool, drive_id).await?,
    )
    .await;
    Ok(())
}

pub(crate) async fn purge_drive(
    pool: &PgPool,
    actor: i64,
    vehicle_id: i64,
    drive_id: i64,
    positions: PurgePositions,
) -> Result<(), AppError> {
    let before = drive_json(pool, drive_id).await?;
    let span: Option<(DateTime<Utc>, Option<DateTime<Utc>>)> =
        sqlx::query_as("SELECT start_at, end_at FROM drives WHERE id = $1 AND vehicle_id = $2")
            .bind(drive_id)
            .bind(vehicle_id)
            .fetch_optional(pool)
            .await?;
    let Some((start, end)) = span else {
        return Err(AppError::message("drive was not found"));
    };
    let mut tx = pool.begin().await?;
    match positions {
        PurgePositions::Detach => {
            sqlx::query("UPDATE positions SET drive_id = NULL WHERE drive_id = $1")
                .bind(drive_id)
                .execute(&mut *tx)
                .await?;
        }
        PurgePositions::DeleteRange => {
            let end = end.unwrap_or(start);
            sqlx::query(
                "DELETE FROM positions
                 WHERE drive_id = $1 AND recorded_at >= $2 AND recorded_at <= $3",
            )
            .bind(drive_id)
            .bind(start)
            .bind(end)
            .execute(&mut *tx)
            .await?;
        }
    }
    sqlx::query("DELETE FROM drives WHERE id = $1 AND vehicle_id = $2")
        .bind(drive_id)
        .bind(vehicle_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    identity::audit(
        pool,
        Some(actor),
        "drive.purge",
        "ok",
        &pair(&before, "null"),
    )
    .await;
    Ok(())
}

pub(crate) async fn trash_charge(
    pool: &PgPool,
    actor: i64,
    vehicle_id: i64,
    charge_id: i64,
) -> Result<(), AppError> {
    let changed = sqlx::query(
        "UPDATE charges SET deleted_at = now()
         WHERE id = $1 AND vehicle_id = $2 AND end_at IS NOT NULL AND deleted_at IS NULL",
    )
    .bind(charge_id)
    .bind(vehicle_id)
    .execute(pool)
    .await?
    .rows_affected();
    if changed == 0 {
        return Err(AppError::message("charge cannot be deleted"));
    }
    identity::audit(
        pool,
        Some(actor),
        "charge.trash",
        "ok",
        &charge_json(pool, charge_id).await?,
    )
    .await;
    Ok(())
}

async fn ensure_closed_drive(
    pool: &PgPool,
    vehicle_id: i64,
    drive_id: i64,
) -> Result<(), AppError> {
    let open: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM drives
         WHERE id = $1 AND vehicle_id = $2 AND end_at IS NULL AND deleted_at IS NULL",
    )
    .bind(drive_id)
    .bind(vehicle_id)
    .fetch_optional(pool)
    .await?;
    if open.is_some() {
        return Err(AppError::message("an open drive cannot be edited"));
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct StoredSample {
    recorded_at: DateTime<Utc>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    speed_kmh: Option<f64>,
    power_kw: Option<f64>,
    odometer_km: Option<f64>,
    elevation_m: Option<f64>,
    battery_level: Option<f64>,
    rated_range_km: Option<f64>,
    ideal_range_km: Option<f64>,
    inside_temp_c: Option<f64>,
    outside_temp_c: Option<f64>,
}

async fn load_samples(pool: &PgPool, drive_id: i64) -> Result<Vec<Sample>, AppError> {
    let rows: Vec<StoredSample> = sqlx::query_as(
        "SELECT recorded_at, latitude, longitude, speed_kmh, power_kw, odometer_km, elevation_m,
                battery_level, rated_range_km, ideal_range_km, inside_temp_c, outside_temp_c
         FROM positions WHERE drive_id = $1 ORDER BY recorded_at, id",
    )
    .bind(drive_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| Sample {
            at: row.recorded_at,
            latitude: row.latitude,
            longitude: row.longitude,
            speed_kmh: row.speed_kmh,
            power_kw: row.power_kw,
            odometer_km: row.odometer_km,
            elevation_m: row.elevation_m,
            battery_level: row.battery_level,
            rated_range_km: row.rated_range_km,
            ideal_range_km: row.ideal_range_km,
            inside_temp_c: row.inside_temp_c,
            outside_temp_c: row.outside_temp_c,
            charge_energy_added_kwh: None,
            charger_power_kw: None,
            fast_charger: None,
            fast_charger_type: None,
        })
        .collect())
}

async fn drive_json(pool: &PgPool, id: i64) -> Result<String, AppError> {
    let value: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(drives) - 'start_latitude' - 'start_longitude' - 'end_latitude' - 'end_longitude'
         FROM drives WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(value.map_or_else(|| "null".to_owned(), |value| value.to_string()))
}

async fn charge_json(pool: &PgPool, id: i64) -> Result<String, AppError> {
    let value: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT to_jsonb(charges) - 'latitude' - 'longitude' FROM charges WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(value.map_or_else(|| "null".to_owned(), |value| value.to_string()))
}

fn pair(before: &str, after: &str) -> String {
    serde_json::json!({ "before": json_or_raw(before), "after": json_or_raw(after) }).to_string()
}

fn json_or_raw(value: &str) -> serde_json::Value {
    serde_json::from_str(value).unwrap_or(serde_json::Value::String(value.to_owned()))
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::{ChargeEdit, DriveEdit, PurgePositions};

    fn at(hour: u32, minute: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 6, 2, hour, minute, 0).unwrap()
    }

    async fn vehicle(pool: &sqlx::PgPool) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO vehicles (vin, name) VALUES ('5YJ30000000000077', 'Edit') RETURNING id",
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn drive(pool: &sqlx::PgPool, vehicle: i64, start_h: u32, end_h: u32) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO drives (vehicle_id, start_at, end_at, distance_km, source)
             VALUES ($1, $2, $3, 1, 'poll') RETURNING id",
        )
        .bind(vehicle)
        .bind(at(start_h, 0))
        .bind(at(end_h, 0))
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn point(pool: &sqlx::PgPool, vehicle: i64, drive: i64, minute: u32, odo: f64) {
        sqlx::query(
            "INSERT INTO positions (vehicle_id, recorded_at, odometer_km, drive_id, source)
             VALUES ($1, $2, $3, $4, 'poll')",
        )
        .bind(vehicle)
        .bind(at(10, minute))
        .bind(odo)
        .bind(drive)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn edits_merge_split_trash_and_purge_keep_the_import_link() {
        let Some(db) = crate::test_util::TestDb::migrated("edits").await else {
            return;
        };
        let vehicle = vehicle(&db.pool).await;
        let actor: i64 = sqlx::query_scalar(
            "INSERT INTO users (username, password_hash, role, totp_enabled)
             VALUES ('editor', 'x', 'admin', true) RETURNING id",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let open = sqlx::query_scalar(
            "INSERT INTO drives (vehicle_id, start_at, source) VALUES ($1, $2, 'poll') RETURNING id",
        )
        .bind(vehicle)
        .bind(at(8, 0))
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let err = super::edit_drive(
            &db.pool,
            actor,
            vehicle,
            open,
            &DriveEdit {
                note: Some("no".into()),
                tag: None,
                start_address_id: None,
                end_address_id: None,
                start_geofence_id: None,
                end_geofence_id: None,
            },
        )
        .await;
        assert!(err.is_err());

        let first = drive(&db.pool, vehicle, 10, 11).await;
        let second = drive(&db.pool, vehicle, 11, 12).await;
        point(&db.pool, vehicle, first, 0, 10.0).await;
        point(&db.pool, vehicle, first, 20, 10.4).await;
        point(&db.pool, vehicle, second, 30, 10.4).await;
        point(&db.pool, vehicle, second, 50, 11.0).await;
        super::edit_drive(
            &db.pool,
            actor,
            vehicle,
            first,
            &DriveEdit {
                note: Some("home".into()),
                tag: Some("commute".into()),
                start_address_id: None,
                end_address_id: None,
                start_geofence_id: None,
                end_geofence_id: None,
            },
        )
        .await
        .unwrap();
        let note: (Option<String>,) = sqlx::query_as("SELECT note FROM drives WHERE id = $1")
            .bind(first)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(note.0.as_deref(), Some("home"));

        super::merge_drives(&db.pool, actor, vehicle, first, second)
            .await
            .unwrap();
        let distance: (Option<f64>,) =
            sqlx::query_as("SELECT distance_km FROM drives WHERE id = $1")
                .bind(first)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!((distance.0.unwrap() - 1.0).abs() < 0.01);
        let gone: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM drives WHERE id = $1")
            .bind(second)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(gone.0, 0);
        let note_kept: (Option<String>,) = sqlx::query_as("SELECT note FROM drives WHERE id = $1")
            .bind(first)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(note_kept.0.as_deref(), Some("home"));

        let created = super::split_drive(&db.pool, actor, vehicle, first, at(10, 30))
            .await
            .unwrap();
        let moved: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM positions WHERE drive_id = $1")
            .bind(created)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert!(moved.0 >= 1);

        sqlx::query(
            "INSERT INTO import_links (entity, external_id, local_id) VALUES ('drives', '77', $1)",
        )
        .bind(first)
        .execute(&db.pool)
        .await
        .unwrap();
        super::trash_drive(&db.pool, actor, vehicle, first)
            .await
            .unwrap();
        let visible: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM drives WHERE id = $1 AND deleted_at IS NULL")
                .bind(first)
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(visible.0, 0);
        super::restore_drive(&db.pool, actor, vehicle, first)
            .await
            .unwrap();
        super::purge_drive(&db.pool, actor, vehicle, first, PurgePositions::Detach)
            .await
            .unwrap();
        let link: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM import_links WHERE entity = 'drives' AND external_id = '77'",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        assert_eq!(link.0, 1);
        let audit: (serde_json::Value,) = sqlx::query_as(
            "SELECT detail FROM audit_log WHERE action = 'drive.purge' ORDER BY id DESC LIMIT 1",
        )
        .fetch_one(&db.pool)
        .await
        .unwrap();
        let text = audit.0.to_string();
        assert!(!text.contains("latitude"));
        assert!(text.contains("before"));

        let charge = super::create_manual_charge(
            &db.pool,
            actor,
            vehicle,
            at(13, 0),
            at(14, 0),
            &ChargeEdit {
                cost: Some(4.5),
                is_dc: Some(false),
                charger_type: Some("wall".into()),
                geofence_id: None,
                note: Some("garage".into()),
            },
        )
        .await
        .unwrap();
        let source: (String,) = sqlx::query_as("SELECT source FROM charges WHERE id = $1")
            .bind(charge)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(source.0, "manual");
    }
}
