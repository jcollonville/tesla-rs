// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use axum::body::Body;
use chrono::{TimeZone, Utc};
use http_body_util::BodyExt;
use sqlx::Row;
use tower::ServiceExt;

use crate::app::test_state;
use crate::crypto::sha256_hex;
use crate::http::{OPENAPI, router};
use crate::import::{POSITIONS_PAGE, POSITIONS_PAGE_SQL, import_pools};
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
    for field in [
        "usable_battery_level",
        "climate_on",
        "locked",
        "sentry_mode",
        "charge_limit_soc",
        "is_dc",
        "charger_voltage",
        "charger_current",
        "plugged_in",
        "tpms_pressure_fl",
        "fd_window",
        "df:",
        "ft:",
        "is_preconditioning",
        "scheduled_charging_start_time",
        "stream_status",
    ] {
        assert!(OPENAPI.contains(field), "{field}");
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

    sqlx::query(
        "INSERT INTO vehicle_state (
            vehicle_id, activity, battery_level, usable_battery_level, climate_on, locked,
            sentry_mode, charge_limit_soc, is_dc, charger_voltage, charger_current, plugged_in,
            tpms_pressure_fl, df, fd_window, ft, is_preconditioning, scheduled_charging_start_time,
            inside_temp_c, elevation_m
         ) VALUES (
            $1, 'charging', 80, 79, true, false, true, 90, false, 240, 32, true,
            2.9, 1, 0, 0, true, 1320, 21.5, 35
         )",
    )
    .bind(vehicle_id.0)
    .execute(&db.pool)
    .await
    .unwrap();
    let state_body = authed(
        &state,
        "GET",
        &format!("/api/v1/vehicles/{}/state", vehicle_id.0),
        token,
    )
    .await;
    assert!(state_body.contains("\"usable_battery_level\":79"));
    assert!(state_body.contains("\"climate_on\":true"));
    assert!(state_body.contains("\"locked\":false"));
    assert!(state_body.contains("\"plugged_in\":true"));
    assert!(state_body.contains("\"tpms_pressure_fl\":2.9"));
    assert!(state_body.contains("\"df\":1"));
    assert!(state_body.contains("\"fd_window\":0"));
    assert!(state_body.contains("\"ft\":0"));
    assert!(state_body.contains("\"scheduled_charging_start_time\":1320"));
    assert!(state_body.contains("\"is_preconditioning\":true"));
    assert!(state_body.contains("\"elevation_m\":35"));
    assert!(state_body.contains("\"stream_status\":\"disabled\""));
    let snaps = crate::http::api::snapshot_rows(&db.pool).await.unwrap();
    assert_eq!(snaps.len(), 1);
    assert_eq!(snaps[0].0, "5YJ3E1EA7KF000001");
    assert_eq!(snaps[0].1, "charging");
    assert_eq!(snaps[0].2, Some(80.0));
    assert_eq!(snaps[0].3, Some(1));
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
    let (stream_kick, _) = tokio::sync::broadcast::channel(8);
    let archive = Archive {
        db: db.pool.clone(),
        geocoder: crate::geocode::Geocoder::disabled(),
        events,
        runtimes: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        stream_kick,
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
    let battery: (f64, f64) = sqlx::query_as(
        "SELECT start_battery_level, end_battery_level FROM drives WHERE end_at IS NOT NULL",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!((battery.0 - 80.0).abs() < 1e-9);
    assert!((battery.1 - 80.0).abs() < 1e-9);

    let mut parked = Observation::at(
        Utc.with_ymd_and_hms(2024, 6, 1, 12, 0, 0).unwrap(),
        Presence::Online,
    );
    parked.shift_state = Some("P".to_owned());
    parked.odometer_km = Some(200.0);
    parked.speed_kmh = Some(0.0);
    parked.battery_level = Some(82.0);
    parked.latitude = Some(48.85);
    parked.longitude = Some(2.35);
    apply_for_vin(&archive, vin, identity(), parked, true, false)
        .await
        .unwrap();
    let mut depart = Observation::at(
        Utc.with_ymd_and_hms(2024, 6, 1, 12, 5, 0).unwrap(),
        Presence::Online,
    );
    depart.shift_state = Some("D".to_owned());
    depart.odometer_km = Some(200.0);
    depart.speed_kmh = Some(30.0);
    depart.latitude = Some(48.86);
    depart.longitude = Some(2.36);
    apply_for_vin(&archive, vin, identity(), depart, true, false)
        .await
        .unwrap();
    let mut cruise = Observation::at(
        Utc.with_ymd_and_hms(2024, 6, 1, 12, 15, 0).unwrap(),
        Presence::Online,
    );
    cruise.shift_state = Some("D".to_owned());
    cruise.odometer_km = Some(210.0);
    cruise.speed_kmh = Some(80.0);
    cruise.battery_level = Some(76.0);
    cruise.latitude = Some(48.87);
    cruise.longitude = Some(2.37);
    apply_for_vin(&archive, vin, identity(), cruise, true, false)
        .await
        .unwrap();
    let mut arrive = Observation::at(
        Utc.with_ymd_and_hms(2024, 6, 1, 12, 25, 0).unwrap(),
        Presence::Online,
    );
    arrive.shift_state = Some("P".to_owned());
    arrive.odometer_km = Some(212.0);
    arrive.speed_kmh = Some(0.0);
    arrive.latitude = Some(48.88);
    arrive.longitude = Some(2.38);
    apply_for_vin(&archive, vin, identity(), arrive, true, false)
        .await
        .unwrap();
    let carried: (f64, f64) = sqlx::query_as(
        "SELECT start_battery_level, end_battery_level
         FROM drives WHERE end_at IS NOT NULL ORDER BY start_at DESC LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!((carried.0 - 82.0).abs() < 1e-9, "start {}", carried.0);
    assert!((carried.1 - 76.0).abs() < 1e-9, "end {}", carried.1);
}

#[tokio::test]
async fn stream_points_follow_one_drive_and_yield_to_a_poll() {
    let Some(db) = TestDb::migrated("stream").await else {
        return;
    };
    let (events, _) = tokio::sync::broadcast::channel(8);
    let (stream_kick, _) = tokio::sync::broadcast::channel(8);
    let archive = Archive {
        db: db.pool.clone(),
        geocoder: crate::geocode::Geocoder::disabled(),
        events,
        runtimes: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        stream_kick,
    };
    let vin = "5YJ3E1EA7KF000001";
    let identity = || VehicleIdentity {
        vin,
        tesla_id: Some(10),
        legacy_vehicle_id: Some(99),
        name: Some("Red"),
    };
    let base = Utc.with_ymd_and_hms(2024, 6, 1, 10, 0, 0).unwrap();
    let mut charging = Observation::at(base - chrono::Duration::minutes(30), Presence::Online);
    charging.shift_state = Some("P".to_owned());
    charging.charging_state = Some("Charging".to_owned());
    charging.latitude = Some(48.8);
    charging.longitude = Some(2.3);
    charging.battery_level = Some(40.0);
    apply_for_vin(&archive, vin, identity(), charging, true, false)
        .await
        .unwrap();

    let samples = [
        (-3_i64, 99.97, 48.8497),
        (-2, 99.98, 48.8498),
        (0, 100.0, 48.85),
        (2, 100.02, 48.8502),
        (3, 100.03, 48.8503),
    ];
    for (offset, odometer_km, latitude) in samples {
        let at = base + chrono::Duration::seconds(offset);
        crate::ingest::apply_stream_for_vin(
            &archive,
            vin,
            identity(),
            &crate::persist::StreamFix {
                at,
                shift_state: Some("D".to_owned()),
                speed_kmh: Some(30.0),
                power_kw: Some(12.0),
                odometer_km: Some(odometer_km),
                battery_level: Some(80.0),
                latitude: Some(latitude),
                longitude: Some(2.35),
                elevation_m: Some(30.0),
                heading_deg: Some(90.0),
                rated_range_km: Some(400.0),
                est_range_km: Some(380.0),
            },
        )
        .await
        .unwrap();
    }
    let mut poll = Observation::at(base, Presence::Online);
    poll.shift_state = Some("D".to_owned());
    poll.speed_kmh = Some(31.0);
    poll.odometer_km = Some(100.0);
    poll.latitude = Some(48.85);
    poll.longitude = Some(2.35);
    poll.battery_level = Some(80.0);
    apply_for_vin(&archive, vin, identity(), poll, true, false)
        .await
        .unwrap();

    let open: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM drives WHERE end_at IS NULL AND source = 'stream'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(open.0, 1);
    let charges: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM charges WHERE end_at IS NOT NULL")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(charges.0, 1, "starting a drive closes the charge");
    let stream_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM positions WHERE source = 'stream'")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let poll_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM positions WHERE source = 'poll'")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        stream_count.0, 2,
        "points within 2s of the poll are removed"
    );
    assert_eq!(poll_count.0, 2);

    crate::ingest::apply_stream_for_vin(
        &archive,
        vin,
        identity(),
        &crate::persist::StreamFix {
            at: base + chrono::Duration::seconds(12),
            shift_state: None,
            speed_kmh: None,
            power_kw: Some(8.0),
            odometer_km: Some(100.2),
            battery_level: None,
            latitude: Some(48.86),
            longitude: Some(2.36),
            elevation_m: None,
            heading_deg: None,
            rated_range_km: None,
            est_range_km: None,
        },
    )
    .await
    .unwrap();
    let still_open: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM drives WHERE end_at IS NULL")
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(still_open.0, 1);

    crate::ingest::apply_stream_for_vin(
        &archive,
        vin,
        identity(),
        &crate::persist::StreamFix {
            at: base + chrono::Duration::seconds(20),
            shift_state: Some("P".to_owned()),
            speed_kmh: Some(0.0),
            power_kw: Some(0.0),
            odometer_km: Some(100.4),
            battery_level: Some(79.0),
            latitude: Some(48.87),
            longitude: Some(2.37),
            elevation_m: Some(32.0),
            heading_deg: Some(10.0),
            rated_range_km: Some(398.0),
            est_range_km: None,
        },
    )
    .await
    .unwrap();
    let closed: (i64, Option<f64>) = sqlx::query_as(
        "SELECT COUNT(*), MAX(distance_km) FROM drives WHERE end_at IS NOT NULL AND source = 'stream'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(closed.0, 1);
    assert!(closed.1.unwrap() > 0.0);
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
    let drive_soc: (f64, f64) = sqlx::query_as(
        "SELECT start_battery_level, end_battery_level FROM drives WHERE external_id = '1'",
    )
    .fetch_one(&dest.pool)
    .await
    .unwrap();
    assert!((drive_soc.0 - 80.0).abs() < 1e-9);
    assert!((drive_soc.1 - 74.0).abs() < 1e-9);
    let second_soc: (f64, f64) = sqlx::query_as(
        "SELECT start_battery_level, end_battery_level FROM drives WHERE external_id = '2'",
    )
    .fetch_one(&dest.pool)
    .await
    .unwrap();
    assert!((second_soc.0 - 70.0).abs() < 1e-9);
    assert!((second_soc.1 - 69.0).abs() < 1e-9);
    sqlx::query("UPDATE drives SET start_battery_level = NULL, end_battery_level = NULL")
        .execute(&dest.pool)
        .await
        .unwrap();
    crate::import::backfill_drive_battery(&dest.pool)
        .await
        .unwrap();
    let restored: (f64, f64) = sqlx::query_as(
        "SELECT start_battery_level, end_battery_level FROM drives WHERE external_id = '1'",
    )
    .fetch_one(&dest.pool)
    .await
    .unwrap();
    assert!((restored.0 - 80.0).abs() < 1e-9);
    assert!((restored.1 - 74.0).abs() < 1e-9);
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

/// The positions keyset must be an index scan on `positions_pkey` with no Sort
/// above that scan. `enable_seqscan` stays at its default.
#[tokio::test]
async fn positions_page_uses_primary_key_index() {
    let Some(source) = TestDb::create("plan").await else {
        return;
    };
    sqlx::raw_sql(FIXTURE).execute(&source.pool).await.unwrap();
    // Enough rows that the default planner prefers the primary key for a
    // keyset page. A production cursor of 17000 sits inside this range.
    sqlx::query("TRUNCATE positions")
        .execute(&source.pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO positions (id, date, latitude, longitude, car_id)
         SELECT g, timestamp '2024-01-01 00:00:00', 48.856600, 2.352200, 1
         FROM generate_series(1, 40000) AS g",
    )
    .execute(&source.pool)
    .await
    .unwrap();
    sqlx::query("ANALYZE positions")
        .execute(&source.pool)
        .await
        .unwrap();

    assert_eq!(POSITIONS_PAGE, 5_000);
    let explain = format!("EXPLAIN (FORMAT JSON) {POSITIONS_PAGE_SQL}");
    let row = sqlx::query(&explain)
        .bind(17_000_i64)
        .bind(POSITIONS_PAGE)
        .fetch_one(&source.pool)
        .await
        .unwrap();
    let plan: serde_json::Value = row.try_get("QUERY PLAN").unwrap();
    assert_positions_pkey_scan(&plan);

    let ids: Vec<i64> = sqlx::query_as(&format!("SELECT id FROM ({POSITIONS_PAGE_SQL}) AS page"))
        .bind(17_000_i64)
        .bind(POSITIONS_PAGE)
        .fetch_all(&source.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|(id,): (i64,)| id)
        .collect();
    assert_eq!(ids.len(), usize::try_from(POSITIONS_PAGE).unwrap());
    assert_eq!(ids.first().copied(), Some(17_001));
    assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
}

fn assert_positions_pkey_scan(plan: &serde_json::Value) {
    let mut found = false;
    let mut sort_above = false;
    let nodes: Vec<&serde_json::Value> = if let Some(list) = plan.as_array() {
        list.iter()
            .map(|entry| entry.get("Plan").unwrap_or(entry))
            .collect()
    } else if let Some(node) = plan.get("Plan") {
        vec![node]
    } else {
        vec![plan]
    };
    for node in nodes {
        walk_plan(node, false, &mut found, &mut sort_above);
    }
    assert!(
        found,
        "expected an Index Scan on positions_pkey, plan was {plan}"
    );
    assert!(
        !sort_above,
        "Sort node above the positions_pkey scan, plan was {plan}"
    );
}

fn walk_plan(node: &serde_json::Value, under_sort: bool, found: &mut bool, sort_above: &mut bool) {
    let node_type = node
        .get("Node Type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let under_sort = under_sort || node_type == "Sort";
    let index = node.get("Index Name").and_then(|value| value.as_str());
    if matches!(node_type, "Index Scan" | "Index Only Scan") && index == Some("positions_pkey") {
        *found = true;
        if under_sort {
            *sort_above = true;
        }
    }
    if let Some(children) = node.get("Plans").and_then(|value| value.as_array()) {
        for child in children {
            walk_plan(child, under_sort, found, sort_above);
        }
    }
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
