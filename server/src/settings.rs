// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Non-secret settings that override the environment without a restart.

use sqlx::PgPool;

use crate::error::AppError;

pub(crate) const KEYS: &[&str] = &[
    "ENERGY_PRICE_PER_KWH",
    "TESLA_TIMEZONE",
    "POLL_DRIVE_SECS",
    "POLL_CHARGE_SECS",
    "POLL_PARKED_SECS",
    "POLL_ASLEEP_SECS",
    "POLL_BACKOFF_SECS",
    "POLL_GPS_FALLBACK_SECS",
    "POLL_PARKED_LIVE_LIMIT",
    "NOMINATIM_ENABLED",
    "NOMINATIM_URL",
    "NOMINATIM_MIN_INTERVAL_MS",
    "MQTT_ENABLED",
    "MQTT_HOST",
    "MQTT_PORT",
    "MQTT_TOPIC_PREFIX",
];

pub(crate) fn allowed(key: &str) -> bool {
    KEYS.contains(&key)
}

pub(crate) async fn get(
    pool: &PgPool,
    vehicle_id: Option<i64>,
    key: &str,
) -> Result<Option<(String, &'static str)>, AppError> {
    if let Some(vehicle_id) = vehicle_id {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT value FROM vehicle_settings WHERE vehicle_id = $1 AND key = $2",
        )
        .bind(vehicle_id)
        .bind(key)
        .fetch_optional(pool)
        .await?;
        if let Some(value) = value {
            return Ok(Some((value, "véhicule")));
        }
    }
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(value.map(|value| (value, "réglage")))
}

pub(crate) async fn set(
    pool: &PgPool,
    vehicle_id: Option<i64>,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    if !allowed(key) {
        return Err(AppError::message("this setting cannot be changed here"));
    }
    let value = value.trim();
    if value.is_empty() {
        clear(pool, vehicle_id, key).await?;
        return Ok(());
    }
    if let Some(vehicle_id) = vehicle_id {
        sqlx::query(
            "INSERT INTO vehicle_settings (vehicle_id, key, value) VALUES ($1, $2, $3)
             ON CONFLICT (vehicle_id, key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(vehicle_id)
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO settings (key, value) VALUES ($1, $2)
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
        )
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    }
    Ok(())
}

async fn clear(pool: &PgPool, vehicle_id: Option<i64>, key: &str) -> Result<(), AppError> {
    if let Some(vehicle_id) = vehicle_id {
        sqlx::query("DELETE FROM vehicle_settings WHERE vehicle_id = $1 AND key = $2")
            .bind(vehicle_id)
            .bind(key)
            .execute(pool)
            .await?;
    } else {
        sqlx::query("DELETE FROM settings WHERE key = $1")
            .bind(key)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub(crate) async fn f64_or(
    pool: &PgPool,
    vehicle_id: Option<i64>,
    key: &str,
    fallback: Option<f64>,
) -> Option<f64> {
    match get(pool, vehicle_id, key).await {
        Ok(Some((value, _))) => value.parse().ok().or(fallback),
        _ => fallback,
    }
}

pub(crate) async fn text_or(
    pool: &PgPool,
    vehicle_id: Option<i64>,
    key: &str,
    fallback: &str,
) -> String {
    match get(pool, vehicle_id, key).await {
        Ok(Some((value, _))) if !value.is_empty() => value,
        _ => fallback.to_owned(),
    }
}

pub(crate) async fn secs_or(
    pool: &PgPool,
    key: &str,
    fallback: std::time::Duration,
) -> std::time::Duration {
    match get(pool, None, key).await {
        Ok(Some((value, _))) => value
            .parse::<u64>()
            .map_or(fallback, std::time::Duration::from_secs),
        _ => fallback,
    }
}
