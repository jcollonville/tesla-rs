// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use axum::body::Body;
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::app::test_state;
use crate::crypto::sha256_hex;
use crate::http::{OPENAPI, router};
use crate::import::import_pools;
use crate::ingest::{Archive, apply_for_vin};
use crate::persist::VehicleIdentity;
use crate::state::{Observation, Presence};
use crate::test_util::TestDb;

const FIXTURE: &str = include_str!("../tests/fixtures/teslamate.sql");

#[test]
fn openapi_lists_the_read_api() {
    for path in [
        "/api/v1/vehicles",
        "/api/v1/drives",
        "/api/v1/charges",
        "/api/v1/stats",
        "/api/v1/geofences",
        "/api/v1/ws",
    ] {
        assert!(OPENAPI.contains(path), "{path}");
    }
}

#[tokio::test]
async fn api_requires_a_token_and_returns_drives() {
    let Some(db) = TestDb::migrated("api").await else {
        return;
    };
    let state = test_state(db.pool.clone()).await;
    let response = router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/vehicles")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::UNAUTHORIZED);

    let token = "tl_test-token";
    sqlx::query("INSERT INTO api_tokens (name, token_hash) VALUES ('ha', $1)")
        .bind(sha256_hex(token))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO vehicles (vin, name) VALUES ('5YJ3E1EA7KF000001', 'Red')")
        .execute(&db.pool)
        .await
        .unwrap();
    let vehicle_id: (i64,) = sqlx::query_as("SELECT id FROM vehicles")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO drives (vehicle_id, start_at, end_at, distance_km, duration_s, source)
         VALUES ($1, '2024-06-01T10:00:00Z', '2024-06-01T10:30:00Z', 12.5, 1800, 'poll')",
    )
    .bind(vehicle_id.0)
    .execute(&db.pool)
    .await
    .unwrap();
    let drive_id: (i64,) = sqlx::query_as("SELECT id FROM drives")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    for index in 0..30 {
        sqlx::query(
            "INSERT INTO positions (vehicle_id, recorded_at, latitude, longitude, drive_id, source)
             VALUES ($1, $2, $3, 2.0, $4, 'poll')",
        )
        .bind(vehicle_id.0)
        .bind(Utc.with_ymd_and_hms(2024, 6, 1, 10, index, 0).unwrap())
        .bind(48.0 + f64::from(index) * 0.001)
        .bind(drive_id.0)
        .execute(&db.pool)
        .await
        .unwrap();
    }

    let body = authed(&state, "GET", "/api/v1/vehicles", token).await;
    assert!(body.contains("5YJ3E1EA7KF000001"));
    let drives = authed(
        &state,
        "GET",
        "/api/v1/drives?from=2024-06-01T00:00:00Z&to=2024-06-02T00:00:00Z",
        token,
    )
    .await;
    assert!(drives.contains("12.5"));
    let detail = authed(
        &state,
        "GET",
        &format!("/api/v1/drives/{}?max_points=5", drive_id.0),
        token,
    )
    .await;
    let parsed: serde_json::Value = serde_json::from_str(&detail).unwrap();
    assert!(parsed["route"].as_array().unwrap().len() <= 5);
    assert!(parsed["series"].as_array().unwrap().len() <= 5);
    let missing = router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/stats")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), axum::http::StatusCode::BAD_REQUEST);
    let key = authed_status(
        &state,
        "/.well-known/appspecific/com.tesla.3p.public-key.pem",
        None,
    )
    .await;
    assert!(key.contains("BEGIN PUBLIC KEY"));
    assert!(!key.contains("PRIVATE KEY"));
}

#[tokio::test]
async fn polling_sequence_opens_and_closes_one_drive() {
    let Some(db) = TestDb::migrated("drive").await else {
        return;
    };
    let (events, _) = tokio::sync::broadcast::channel(8);
    let archive = Archive {
        db: db.pool.clone(),
        geocoder: crate::geocode::Geocoder::disabled(),
        events,
        runtimes: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
    };
    let vin = "5YJ3E1EA7KF000001";
    let identity = || VehicleIdentity {
        vin,
        tesla_id: Some(10),
        legacy_vehicle_id: Some(20),
        name: Some("Red"),
    };
    let mut minute = 0i64;
    let mut step = |shift: Option<&str>, odometer: f64, speed: f64| {
        let mut obs = Observation::at(
            Utc.with_ymd_and_hms(2024, 6, 1, 10, 0, 0).unwrap() + chrono::Duration::minutes(minute),
            Presence::Online,
        );
        minute += 5;
        obs.shift_state = shift.map(str::to_owned);
        obs.odometer_km = Some(odometer);
        obs.speed_kmh = Some(speed);
        obs.rated_range_km = Some(400.0 - (odometer - 100.0));
        obs.battery_level = Some(80.0);
        obs.latitude = Some(48.85);
        obs.longitude = Some(2.35);
        obs.efficiency_kwh_per_km = Some(0.16);
        obs
    };
    for obs in [
        step(Some("P"), 100.0, 0.0),
        step(Some("D"), 100.0, 20.0),
        step(Some("D"), 110.0, 80.0),
        step(Some("P"), 112.0, 0.0),
    ] {
        apply_for_vin(&archive, vin, identity(), obs, true, false)
            .await
            .unwrap();
    }
    let asleep = Observation::at(
        Utc.with_ymd_and_hms(2024, 6, 1, 11, 0, 0).unwrap(),
        Presence::Asleep,
    );
    apply_for_vin(&archive, vin, identity(), asleep, false, false)
        .await
        .unwrap();
    let (count, distance): (i64, Option<f64>) =
        sqlx::query_as("SELECT COUNT(*), MAX(distance_km) FROM drives WHERE end_at IS NOT NULL")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    assert!((distance.unwrap() - 12.0).abs() < 1e-6);
}

#[tokio::test]
async fn teslamate_import_is_idempotent_and_resumable() {
    let Some(dest) = TestDb::migrated("dest").await else {
        return;
    };
    let Some(source) = TestDb::create("tm").await else {
        return;
    };
    sqlx::raw_sql(FIXTURE).execute(&source.pool).await.unwrap();
    let first = import_pools(&dest.pool, &source.pool, 1).await.unwrap();
    assert_eq!(first.cars, 2);
    assert_eq!(first.drives, 2);
    assert_eq!(first.positions, 4);
    assert_eq!(first.charges, 1);
    assert_eq!(first.charge_samples, 2);
    assert_eq!(first.addresses, 1);
    assert_eq!(first.geofences, 1);
    let second = import_pools(&dest.pool, &source.pool, 1).await.unwrap();
    assert_eq!(second.positions, 0);
    let positions: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM positions")
        .fetch_one(&dest.pool)
        .await
        .unwrap();
    assert_eq!(positions.0, 4);
    let gain: (f64,) =
        sqlx::query_as("SELECT elevation_gain_m FROM drives WHERE external_id = '1'")
            .fetch_one(&dest.pool)
            .await
            .unwrap();
    assert!((gain.0 - 15.0).abs() < 1e-9);
    let energy: (f64,) =
        sqlx::query_as("SELECT energy_used_kwh FROM drives WHERE external_id = '1'")
            .fetch_one(&dest.pool)
            .await
            .unwrap();
    assert!((energy.0 - 1.5).abs() < 1e-9);
    let priority: (i32,) =
        sqlx::query_as("SELECT display_priority FROM vehicles WHERE vin = '5YJ3E1EA7KF000001'")
            .fetch_one(&dest.pool)
            .await
            .unwrap();
    assert_eq!(priority.0, 3);
    let drive: (chrono::DateTime<Utc>, i32, f64, f64, f64) = sqlx::query_as(
        "SELECT start_at, duration_s, max_speed_kmh, max_power_kw, inside_temp_avg_c
         FROM drives WHERE external_id = '1'",
    )
    .fetch_one(&dest.pool)
    .await
    .unwrap();
    assert_eq!(drive.0, Utc.with_ymd_and_hms(2024, 6, 1, 10, 0, 0).unwrap());
    assert_eq!(drive.1, 1800);
    assert!((drive.2 - 110.0).abs() < 1e-9);
    assert!((drive.3 - 80.0).abs() < 1e-9);
    assert!((drive.4 - 21.5).abs() < 1e-9);
    let position: (chrono::DateTime<Utc>, f64, f64, f64) = sqlx::query_as(
        "SELECT recorded_at, speed_kmh, elevation_m, battery_level
         FROM positions WHERE external_id = '2'",
    )
    .fetch_one(&dest.pool)
    .await
    .unwrap();
    assert_eq!(
        position.0,
        Utc.with_ymd_and_hms(2024, 6, 1, 10, 30, 0).unwrap()
    );
    assert!((position.1 - 80.0).abs() < 1e-9);
    assert!((position.2 - 55.0).abs() < 1e-9);
    assert!((position.3 - 74.0).abs() < 1e-9);
    let missing_power: (Option<f64>,) =
        sqlx::query_as("SELECT power_kw FROM positions WHERE external_id = '4'")
            .fetch_one(&dest.pool)
            .await
            .unwrap();
    assert_eq!(missing_power.0, None);
    let sample: (f64, f64, f64) = sqlx::query_as(
        "SELECT battery_level, charger_power_kw, charger_voltage
         FROM charge_samples WHERE external_id = '1'",
    )
    .fetch_one(&dest.pool)
    .await
    .unwrap();
    assert!((sample.0 - 45.0).abs() < 1e-9);
    assert!((sample.1 - 50.0).abs() < 1e-9);
    assert!((sample.2 - 400.0).abs() < 1e-9);
    let radius: (f64,) = sqlx::query_as("SELECT radius_m FROM geofences WHERE external_id = '1'")
        .fetch_one(&dest.pool)
        .await
        .unwrap();
    assert!((radius.0 - 100.0).abs() < 1e-9);
    let address: (Option<String>, f64) =
        sqlx::query_as("SELECT city, latitude FROM addresses WHERE external_id = '1'")
            .fetch_one(&dest.pool)
            .await
            .unwrap();
    assert_eq!(address.0.as_deref(), Some("Paris"));
    assert!((address.1 - 48.8566).abs() < 1e-6);
    let level: (f64, f64) =
        sqlx::query_as("SELECT start_battery_level, cost FROM charges WHERE external_id = '1'")
            .fetch_one(&dest.pool)
            .await
            .unwrap();
    assert!((level.0 - 40.0).abs() < 1e-9);
    assert!((level.1 - 4.5).abs() < 1e-9);
    let dc: (Option<bool>,) = sqlx::query_as("SELECT is_dc FROM charges WHERE external_id = '1'")
        .fetch_one(&dest.pool)
        .await
        .unwrap();
    assert_eq!(dc.0, Some(true));
    sqlx::query("DELETE FROM positions")
        .execute(&dest.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE import_cursors SET cursor_id = 2 WHERE source = 'teslamate.positions'")
        .execute(&dest.pool)
        .await
        .unwrap();
    let resumed = import_pools(&dest.pool, &source.pool, 1).await.unwrap();
    assert_eq!(resumed.positions, 2);
}

async fn authed(state: &crate::app::AppState, method: &str, uri: &str, token: &str) -> String {
    let response = router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

async fn authed_status(state: &crate::app::AppState, uri: &str, token: Option<&str>) -> String {
    let mut builder = axum::http::Request::builder().uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let response = router(state.clone())
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(response.status().is_success(), "{}", response.status());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}
