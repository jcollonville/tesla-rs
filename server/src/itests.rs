// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Arc;

use axum::body::Body;
use chrono::{DateTime, TimeZone, Utc};
use http_body_util::BodyExt;
use sqlx::Row;
use tower::ServiceExt;

use crate::app::test_state;
use crate::crypto::sha256_hex;
use crate::http::{OPENAPI, router};
use crate::import::{POSITIONS_PAGE, POSITIONS_PAGE_SQL, import_pools};
use crate::ingest::{Archive, apply_for_vin, apply_stream_for_vin};
use crate::persist::{StreamFix, VehicleIdentity};
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
        "/api/v1/vehicles/{id}/stats/series",
        "/api/v1/vehicles/{id}/stats/consumption_by_temp",
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
        "position_mode",
        "range_at_100_km",
        "idle_range_loss_km_per_day",
        "temp_c_bucket",
        "drive_duration_s",
        "dc_share_pct",
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
        gps: crate::position_mode::GpsBoard::default(),
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
        gps: crate::position_mode::GpsBoard::default(),
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
async fn short_drives_are_deleted_for_poll_and_stream() {
    let Some(db) = TestDb::migrated("short").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let poll_vin = "5YJ3E1EA7KF000010";
    let stream_vin = "5YJ3E1EA7KF000011";
    let base = Utc.with_ymd_and_hms(2024, 7, 1, 8, 0, 0).unwrap();

    // One stored position, then sleep. The closing sample is not a position.
    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        shifted(base, "D", Some(100.0), 48.85, 2.35),
        true,
        false,
    )
    .await
    .unwrap();
    let deleted_id = open_drive_id(&db.pool, poll_vin).await.expect("open drive");
    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        Observation::at(base + chrono::Duration::minutes(5), Presence::Asleep),
        false,
        false,
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, poll_vin).await.0, 0);
    assert_eq!(detached_positions(&db.pool, poll_vin).await, 1);

    let state = test_state(db.pool.clone()).await;
    let token = "tl_short-drive";
    sqlx::query("INSERT INTO api_tokens (name, token_hash) VALUES ('ha', $1)")
        .bind(sha256_hex(token))
        .execute(&db.pool)
        .await
        .unwrap();
    let poll_vehicle = vehicle_id(&db.pool, poll_vin).await;
    let missing = router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri(format!("/api/v1/drives/{deleted_id}"))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), axum::http::StatusCode::NOT_FOUND);
    let stats = authed(
        &state,
        "GET",
        &format!("/api/v1/stats?vehicle_id={poll_vehicle}"),
        token,
    )
    .await;
    let stats: serde_json::Value = serde_json::from_str(&stats).unwrap();
    assert_eq!(stats["drive_count"], 0);
    assert_eq!(stats["distance_km"], 0.0);
    let drives = authed(
        &state,
        "GET",
        &format!("/api/v1/drives?vehicle_id={poll_vehicle}"),
        token,
    )
    .await;
    let drives: serde_json::Value = serde_json::from_str(&drives).unwrap();
    assert_eq!(drives["total"], 0);
    let last: Option<(i64,)> = sqlx::query_as(
        "SELECT id FROM drives WHERE vehicle_id = $1 AND end_at IS NOT NULL
         ORDER BY end_at DESC LIMIT 1",
    )
    .bind(poll_vehicle)
    .fetch_optional(&db.pool)
    .await
    .unwrap();
    assert!(last.is_none());

    // Two positions, odometer unchanged, GPS far apart.
    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        shifted(
            base + chrono::Duration::minutes(10),
            "D",
            Some(200.0),
            48.0,
            2.0,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        shifted(
            base + chrono::Duration::minutes(20),
            "P",
            Some(200.0),
            49.5,
            3.5,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, poll_vin).await.0, 0);

    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        shifted(
            base + chrono::Duration::minutes(30),
            "D",
            Some(300.0),
            48.1,
            2.1,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        shifted(
            base + chrono::Duration::minutes(40),
            "P",
            Some(300.009),
            48.11,
            2.11,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, poll_vin).await.0, 0);

    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        shifted(base + chrono::Duration::minutes(50), "D", None, 48.2, 2.2),
        true,
        false,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        shifted(base + chrono::Duration::minutes(60), "P", None, 48.4, 2.5),
        true,
        false,
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, poll_vin).await.0, 0);

    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        shifted(
            base + chrono::Duration::minutes(70),
            "D",
            Some(400.0),
            48.3,
            2.3,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        shifted(
            base + chrono::Duration::minutes(80),
            "P",
            Some(400.01),
            48.31,
            2.31,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    let (count, distance) = closed_drives(&db.pool, poll_vin).await;
    assert_eq!(count, 1);
    assert!((distance.unwrap() - 0.01).abs() < 1e-6);
    let attached: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM positions p
         JOIN drives d ON d.id = p.drive_id
         JOIN vehicles v ON v.id = d.vehicle_id
         WHERE v.vin = $1",
    )
    .bind(poll_vin)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(attached.0, 2);
    let kept_stats = authed(
        &state,
        "GET",
        &format!("/api/v1/stats?vehicle_id={poll_vehicle}"),
        token,
    )
    .await;
    let kept_stats: serde_json::Value = serde_json::from_str(&kept_stats).unwrap();
    assert_eq!(kept_stats["drive_count"], 1);

    // Stream: one position (park frame has no coordinates), then 0 km, 0.009 km, 0.01 km.
    let t0 = base + chrono::Duration::hours(3);
    apply_stream_for_vin(
        &archive,
        stream_vin,
        car(stream_vin),
        &stream_fix(t0, Some("D"), Some(10.0), Some(48.0), Some(2.0)),
    )
    .await
    .unwrap();
    apply_stream_for_vin(
        &archive,
        stream_vin,
        car(stream_vin),
        &stream_fix(
            t0 + chrono::Duration::seconds(5),
            Some("P"),
            Some(10.0),
            None,
            None,
        ),
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, stream_vin).await.0, 0);
    assert_eq!(detached_positions(&db.pool, stream_vin).await, 1);

    apply_stream_for_vin(
        &archive,
        stream_vin,
        car(stream_vin),
        &stream_fix(
            t0 + chrono::Duration::seconds(20),
            Some("D"),
            Some(20.0),
            Some(48.1),
            Some(2.1),
        ),
    )
    .await
    .unwrap();
    apply_stream_for_vin(
        &archive,
        stream_vin,
        car(stream_vin),
        &stream_fix(
            t0 + chrono::Duration::seconds(40),
            Some("P"),
            Some(20.0),
            Some(49.2),
            Some(3.2),
        ),
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, stream_vin).await.0, 0);

    apply_stream_for_vin(
        &archive,
        stream_vin,
        car(stream_vin),
        &stream_fix(
            t0 + chrono::Duration::seconds(60),
            Some("D"),
            Some(30.0),
            Some(48.2),
            Some(2.2),
        ),
    )
    .await
    .unwrap();
    apply_stream_for_vin(
        &archive,
        stream_vin,
        car(stream_vin),
        &stream_fix(
            t0 + chrono::Duration::seconds(80),
            Some("P"),
            Some(30.009),
            Some(48.21),
            Some(2.21),
        ),
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, stream_vin).await.0, 0);

    apply_stream_for_vin(
        &archive,
        stream_vin,
        car(stream_vin),
        &stream_fix(
            t0 + chrono::Duration::seconds(100),
            Some("D"),
            Some(40.0),
            Some(48.3),
            Some(2.3),
        ),
    )
    .await
    .unwrap();
    apply_stream_for_vin(
        &archive,
        stream_vin,
        car(stream_vin),
        &stream_fix(
            t0 + chrono::Duration::seconds(120),
            Some("P"),
            Some(40.01),
            Some(48.31),
            Some(2.31),
        ),
    )
    .await
    .unwrap();
    let (stream_count, stream_distance) = closed_drives(&db.pool, stream_vin).await;
    assert_eq!(stream_count, 1);
    assert!((stream_distance.unwrap() - 0.01).abs() < 1e-6);
    let stream_vehicle = vehicle_id(&db.pool, stream_vin).await;
    let stream_stats = authed(
        &state,
        "GET",
        &format!("/api/v1/stats?vehicle_id={stream_vehicle}"),
        token,
    )
    .await;
    let stream_stats: serde_json::Value = serde_json::from_str(&stream_stats).unwrap();
    assert_eq!(stream_stats["drive_count"], 1);
}

fn test_archive(pool: sqlx::PgPool) -> Archive {
    let (events, _) = tokio::sync::broadcast::channel(8);
    let (stream_kick, _) = tokio::sync::broadcast::channel(8);
    Archive {
        db: pool,
        geocoder: crate::geocode::Geocoder::disabled(),
        events,
        runtimes: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        stream_kick,
        gps: crate::position_mode::GpsBoard::default(),
    }
}

fn car(vin: &'static str) -> VehicleIdentity<'static> {
    VehicleIdentity {
        vin,
        tesla_id: Some(11),
        legacy_vehicle_id: Some(21),
        name: Some("Red"),
    }
}

fn shifted(
    at: chrono::DateTime<Utc>,
    shift: &str,
    odometer_km: Option<f64>,
    latitude: f64,
    longitude: f64,
) -> Observation {
    let mut obs = Observation::at(at, Presence::Online);
    obs.shift_state = Some(shift.to_owned());
    obs.odometer_km = odometer_km;
    obs.speed_kmh = Some(if shift == "P" { 0.0 } else { 30.0 });
    obs.latitude = Some(latitude);
    obs.longitude = Some(longitude);
    obs.battery_level = Some(80.0);
    obs
}

fn stream_fix(
    at: chrono::DateTime<Utc>,
    shift: Option<&str>,
    odometer_km: Option<f64>,
    latitude: Option<f64>,
    longitude: Option<f64>,
) -> StreamFix {
    StreamFix {
        at,
        shift_state: shift.map(str::to_owned),
        speed_kmh: Some(if shift == Some("P") { 0.0 } else { 30.0 }),
        power_kw: Some(10.0),
        odometer_km,
        battery_level: Some(80.0),
        latitude,
        longitude,
        elevation_m: Some(30.0),
        heading_deg: Some(90.0),
        rated_range_km: Some(400.0),
        est_range_km: Some(380.0),
    }
}

async fn vehicle_id(pool: &sqlx::PgPool, vin: &str) -> i64 {
    let row: (i64,) = sqlx::query_as("SELECT id FROM vehicles WHERE vin = $1")
        .bind(vin)
        .fetch_one(pool)
        .await
        .unwrap();
    row.0
}

async fn open_drive_id(pool: &sqlx::PgPool, vin: &str) -> Option<i64> {
    sqlx::query_as(
        "SELECT d.id FROM drives d
         JOIN vehicles v ON v.id = d.vehicle_id
         WHERE v.vin = $1 AND d.end_at IS NULL",
    )
    .bind(vin)
    .fetch_optional(pool)
    .await
    .unwrap()
    .map(|(id,): (i64,)| id)
}

async fn closed_drives(pool: &sqlx::PgPool, vin: &str) -> (i64, Option<f64>) {
    sqlx::query_as(
        "SELECT COUNT(*)::bigint, MAX(d.distance_km)
         FROM drives d
         JOIN vehicles v ON v.id = d.vehicle_id
         WHERE v.vin = $1 AND d.end_at IS NOT NULL",
    )
    .bind(vin)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn detached_positions(pool: &sqlx::PgPool, vin: &str) -> i64 {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM positions p
         JOIN vehicles v ON v.id = p.vehicle_id
         WHERE v.vin = $1 AND p.drive_id IS NULL",
    )
    .bind(vin)
    .fetch_one(pool)
    .await
    .unwrap();
    row.0
}

#[tokio::test]
async fn stream_event_keeps_polled_temperatures() {
    let Some(db) = TestDb::migrated("merge").await else {
        return;
    };
    let (events, mut rx) = tokio::sync::broadcast::channel(8);
    let (stream_kick, _) = tokio::sync::broadcast::channel(8);
    let archive = Archive {
        db: db.pool.clone(),
        geocoder: crate::geocode::Geocoder::disabled(),
        events,
        runtimes: Arc::new(tokio::sync::Mutex::new(std::collections::HashMap::new())),
        stream_kick,
        gps: crate::position_mode::GpsBoard::default(),
    };
    let vin = "5YJ3E1EA7KF000001";
    let identity = || VehicleIdentity {
        vin,
        tesla_id: Some(10),
        legacy_vehicle_id: Some(99),
        name: Some("Red"),
    };
    let base = Utc.with_ymd_and_hms(2024, 6, 1, 10, 0, 0).unwrap();
    let mut poll = Observation::at(base, Presence::Online);
    poll.inside_temp_c = Some(21.5);
    poll.outside_temp_c = Some(9.0);
    poll.charge_energy_added_kwh = Some(3.5);
    poll.charge_limit_soc = Some(80.0);
    poll.charging_state = Some("Disconnected".to_owned());
    poll.plugged_in = Some(false);
    poll.latitude = Some(48.1);
    poll.longitude = Some(2.2);
    poll.speed_kmh = Some(0.0);
    apply_for_vin(&archive, vin, identity(), poll, true, false)
        .await
        .unwrap();
    let _ = rx.try_recv();

    crate::ingest::apply_stream_for_vin(
        &archive,
        vin,
        identity(),
        &crate::persist::StreamFix {
            at: base + chrono::Duration::seconds(5),
            shift_state: Some("D".to_owned()),
            speed_kmh: Some(42.0),
            power_kw: Some(11.0),
            odometer_km: Some(100.4),
            battery_level: Some(77.0),
            latitude: Some(48.2),
            longitude: Some(2.3),
            elevation_m: Some(30.0),
            heading_deg: Some(90.0),
            rated_range_km: Some(400.0),
            est_range_km: Some(380.0),
        },
    )
    .await
    .unwrap();
    let event = rx.try_recv().expect("stream event");
    let value = serde_json::to_value(&event).unwrap();
    let text = value.to_string();
    assert_eq!(value["inside_temp_c"], 21.5, "{text}");
    assert_eq!(value["outside_temp_c"], 9.0, "{text}");
    assert_eq!(value["charge_energy_added_kwh"], 3.5, "{text}");
    assert_eq!(value["charge_limit_soc"], 80.0, "{text}");
    assert_eq!(value["charging_state"], "Disconnected", "{text}");
    assert_eq!(value["latitude"], 48.2, "{text}");
    assert_eq!(value["speed_kmh"], 42.0, "{text}");
    assert!(!text.contains("null"), "{text}");

    let mut newer = Observation::at(base + chrono::Duration::seconds(20), Presence::Online);
    newer.inside_temp_c = Some(22.0);
    newer.outside_temp_c = Some(8.0);
    newer.charge_limit_soc = Some(90.0);
    newer.shift_state = Some("D".to_owned());
    newer.latitude = Some(48.25);
    newer.longitude = Some(2.31);
    newer.speed_kmh = Some(40.0);
    apply_for_vin(&archive, vin, identity(), newer, true, false)
        .await
        .unwrap();
    let event = rx.try_recv().expect("poll event");
    assert_eq!(event.inside_temp_c, Some(22.0));
    assert_eq!(event.outside_temp_c, Some(8.0));
    assert_eq!(event.charge_limit_soc, Some(90.0));
    assert_eq!(event.latitude, Some(48.25));

    let stored: (Option<f64>, Option<f64>) = sqlx::query_as(
        "SELECT inside_temp_c, outside_temp_c FROM positions WHERE source = 'stream' ORDER BY recorded_at LIMIT 1",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(stored, (None, None));
    let state: (Option<f64>, Option<f64>) =
        sqlx::query_as("SELECT inside_temp_c, outside_temp_c FROM vehicle_state")
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(state, (Some(22.0), Some(8.0)));
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

#[tokio::test]
async fn stats_series_buckets_indicators_and_temperature() {
    let Some(db) = TestDb::migrated("stats").await else {
        return;
    };
    let state = crate::app::test_state_with(db.pool.clone(), Some(0.5)).await;
    let token = "tl_stats-token";
    sqlx::query("INSERT INTO api_tokens (name, token_hash) VALUES ('ha', $1)")
        .bind(sha256_hex(token))
        .execute(&db.pool)
        .await
        .unwrap();
    let vin = "5YJ3E1EA7KF000099";
    let (vehicle_id,): (i64,) = sqlx::query_as(
        "INSERT INTO vehicles (vin, name, efficiency_kwh_per_km)
         VALUES ($1, 'Stats', 0.25) RETURNING id",
    )
    .bind(vin)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let (other_id,): (i64,) = sqlx::query_as(
        "INSERT INTO vehicles (vin, name, efficiency_kwh_per_km)
         VALUES ('5YJ3E1EA7KF000098', 'Other', 0.25) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let months: Vec<(DateTime<Utc>,)> = sqlx::query_as(
        "SELECT gs AT TIME ZONE 'Europe/Paris'
         FROM generate_series(
            date_trunc('month', timezone('Europe/Paris', now())) - interval '3 months',
            date_trunc('month', timezone('Europe/Paris', now())),
            interval '1 month'
         ) AS gs
         ORDER BY 1",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    let month_2 = months[1].0;
    let prev = months[2].0;
    let current = months[3].0;

    let d1 = current + chrono::Duration::days(2);
    seed_drive(
        &db.pool,
        DriveSeed {
            vehicle_id,
            start: d1,
            end: Some(d1 + chrono::Duration::hours(1)),
            distance: Some(100.0),
            duration_s: Some(3600),
            start_rated: Some(400.0),
            end_rated: Some(385.0),
            temp: Some(7.2),
            ..DriveSeed::closed(vehicle_id, d1)
        },
    )
    .await;
    let d2 = current + chrono::Duration::days(5);
    seed_drive(
        &db.pool,
        DriveSeed {
            vehicle_id,
            start: d2,
            end: Some(d2 + chrono::Duration::seconds(1200)),
            distance: Some(40.0),
            duration_s: Some(1200),
            energy: Some(8.0),
            temp: Some(7.2),
            ..DriveSeed::closed(vehicle_id, d2)
        },
    )
    .await;
    // Open trip and another vehicle must not leak into the series.
    seed_drive(
        &db.pool,
        DriveSeed {
            vehicle_id,
            start: current + chrono::Duration::days(6),
            end: None,
            distance: Some(9_999.0),
            duration_s: Some(9_999),
            energy: Some(9_999.0),
            ..DriveSeed::closed(vehicle_id, current)
        },
    )
    .await;
    seed_drive(
        &db.pool,
        DriveSeed {
            vehicle_id: other_id,
            start: d1,
            end: Some(d1 + chrono::Duration::minutes(10)),
            distance: Some(999.0),
            duration_s: Some(600),
            energy: Some(50.0),
            ..DriveSeed::closed(other_id, d1)
        },
    )
    .await;

    let a_start = prev + chrono::Duration::days(2);
    let a_end = a_start + chrono::Duration::seconds(1800);
    seed_drive(
        &db.pool,
        DriveSeed {
            vehicle_id,
            start: a_start,
            end: Some(a_end),
            distance: Some(50.0),
            duration_s: Some(1800),
            energy: Some(10.0),
            start_rated: Some(310.0),
            end_rated: Some(300.0),
            start_odo: Some(990.0),
            end_odo: Some(1_000.0),
            temp: Some(12.0),
            ..DriveSeed::closed(vehicle_id, a_start)
        },
    )
    .await;
    let b_start = a_end + chrono::Duration::hours(48);
    let b_end = b_start + chrono::Duration::seconds(600);
    seed_drive(
        &db.pool,
        DriveSeed {
            vehicle_id,
            start: b_start,
            end: Some(b_end),
            distance: Some(10.0),
            duration_s: Some(600),
            energy: Some(1.0),
            start_rated: Some(294.0),
            end_rated: Some(290.0),
            start_odo: Some(1_000.2),
            end_odo: Some(1_010.0),
            ..DriveSeed::closed(vehicle_id, b_start)
        },
    )
    .await;
    // Odometer jumped, so this parked gap is not vampire drain.
    let c_start = b_end + chrono::Duration::hours(24);
    seed_drive(
        &db.pool,
        DriveSeed {
            vehicle_id,
            start: c_start,
            end: Some(c_start + chrono::Duration::seconds(300)),
            distance: Some(5.0),
            duration_s: Some(300),
            energy: Some(0.5),
            start_rated: Some(288.0),
            start_odo: Some(1_020.0),
            end_odo: Some(1_025.0),
            ..DriveSeed::closed(vehicle_id, c_start)
        },
    )
    .await;

    let ideal = month_2 + chrono::Duration::days(1);
    seed_drive(
        &db.pool,
        DriveSeed {
            vehicle_id,
            start: ideal,
            end: Some(ideal + chrono::Duration::seconds(900)),
            distance: Some(20.0),
            duration_s: Some(900),
            start_ideal: Some(500.0),
            end_ideal: Some(480.0),
            temp: Some(-3.0),
            source: "teslamate",
            ..DriveSeed::closed(vehicle_id, ideal)
        },
    )
    .await;

    let charge_at = current + chrono::Duration::days(3);
    let charge_end = charge_at + chrono::Duration::hours(2);
    for (energy, cost, is_dc, charger) in [
        (8.0, None, Some(false), None),
        (20.0, Some(6.0), Some(true), None),
        (2.0, None, None, Some("CCS")),
    ] {
        sqlx::query(
            "INSERT INTO charges (
                vehicle_id, start_at, end_at, energy_added_kwh, cost, is_dc, charger_type, source
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, 'poll')",
        )
        .bind(vehicle_id)
        .bind(charge_at)
        .bind(charge_end)
        .bind(energy)
        .bind(cost)
        .bind(is_dc)
        .bind(charger)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let (charge_id,): (i64,) = sqlx::query_as(
        "SELECT id FROM charges WHERE vehicle_id = $1 AND is_dc IS NOT TRUE ORDER BY id LIMIT 1",
    )
    .bind(vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    // Position and charge-sample readings must not affect range_at_100_km.
    // 90% / 900 km would project to 1000 km if the series still scanned them.
    sqlx::query(
        "INSERT INTO positions (vehicle_id, recorded_at, battery_level, rated_range_km, source)
         VALUES ($1, $2, 90, 900, 'poll'), ($1, $3, 8, 100, 'poll')",
    )
    .bind(vehicle_id)
    .bind(charge_at)
    .bind(charge_at + chrono::Duration::hours(1))
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO positions (vehicle_id, recorded_at, battery_level, rated_range_km, source)
         VALUES ($1, $2, 50, 50, 'poll')",
    )
    .bind(other_id)
    .bind(charge_at)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO charge_samples (charge_id, recorded_at, battery_level, rated_range_km, source)
         VALUES ($1, $2, 20, 40, 'poll')",
    )
    .bind(charge_id)
    .bind(charge_at)
    .execute(&db.pool)
    .await
    .unwrap();
    // One health value per drive (end reading) and per charge (end reading).
    sqlx::query(
        "UPDATE drives SET end_battery_level = 50, end_rated_range_km = 180
         WHERE vehicle_id = $1 AND distance_km = 40",
    )
    .bind(vehicle_id)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE charges SET end_battery_level = 75, end_rated_range_km = 300
         WHERE vehicle_id = $1 AND energy_added_kwh = 8",
    )
    .bind(vehicle_id)
    .execute(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE charges SET end_battery_level = 5, end_rated_range_km = 40
         WHERE vehicle_id = $1 AND energy_added_kwh = 2",
    )
    .bind(vehicle_id)
    .execute(&db.pool)
    .await
    .unwrap();

    let denied = router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/api/v1/vehicles/{vehicle_id}/stats/series?period=month"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(denied.status(), axum::http::StatusCode::UNAUTHORIZED);

    let bad = router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri(format!(
                    "/api/v1/vehicles/{vehicle_id}/stats/series?period=day"
                ))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bad.status(), axum::http::StatusCode::BAD_REQUEST);
    let missing = router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/vehicles/999999/stats/series?period=month&count=1")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), axum::http::StatusCode::NOT_FOUND);

    let body = authed(
        &state,
        "GET",
        &format!("/api/v1/vehicles/{vehicle_id}/stats/series?period=month&count=4"),
        token,
    )
    .await;
    assert!(!body.contains(vin), "{body}");
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["period"], "month");
    let buckets = parsed["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 4);
    for (bucket, expected) in buckets.iter().zip(&months) {
        let start = bucket["start"].as_str().unwrap();
        assert!(!start.contains('.'), "{start}");
        let parsed_start = DateTime::parse_from_rfc3339(start).unwrap();
        assert_eq!(parsed_start.format("%H:%M:%S").to_string(), "00:00:00");
        assert_eq!(parsed_start.with_timezone(&Utc), expected.0);
    }
    assert_empty_bucket(&buckets[0]);

    let older = &buckets[1];
    close(num(older, "distance_km"), 20.0);
    close(num(older, "energy_used_kwh"), 5.0);
    close(num(older, "wh_per_km"), 250.0);
    assert_eq!(int(older, "drive_duration_s"), 900);
    assert_eq!(int(older, "drive_count"), 1);
    close(num(older, "energy_charged_kwh"), 0.0);
    assert!(older["range_at_100_km"].is_null());
    assert!(older["idle_range_loss_km_per_day"].is_null());

    let previous = &buckets[2];
    close(num(previous, "distance_km"), 65.0);
    close(num(previous, "energy_used_kwh"), 11.5);
    close(num(previous, "wh_per_km"), 11.5 * 1_000.0 / 65.0);
    assert_eq!(int(previous, "drive_duration_s"), 2_700);
    assert_eq!(int(previous, "drive_count"), 3);
    close(num(previous, "idle_range_loss_km_per_day"), 3.0);
    close(num(previous, "idle_energy_loss_kwh_per_day"), 0.75);
    assert!(previous["charge_cost"].is_null());

    let latest = &buckets[3];
    close(num(latest, "distance_km"), 140.0);
    close(num(latest, "energy_used_kwh"), 11.75);
    close(num(latest, "wh_per_km"), 11.75 * 1_000.0 / 140.0);
    close(num(latest, "energy_charged_kwh"), 30.0);
    assert_eq!(int(latest, "drive_duration_s"), 4_800);
    assert_eq!(int(latest, "drive_count"), 2);
    close(num(latest, "range_at_100_km"), 380.0);
    close(num(latest, "ac_energy_kwh"), 8.0);
    close(num(latest, "dc_energy_kwh"), 22.0);
    close(num(latest, "dc_share_pct"), 22.0 / 30.0 * 100.0);
    close(num(latest, "charge_cost"), 11.0);
    close(num(latest, "cost_per_km"), 11.0 / 140.0);
    assert!(latest["idle_range_loss_km_per_day"].is_null());

    let defaults = authed(
        &state,
        "GET",
        &format!("/api/v1/vehicles/{vehicle_id}/stats/series?period=month"),
        token,
    )
    .await;
    let defaults: serde_json::Value = serde_json::from_str(&defaults).unwrap();
    let default_buckets = defaults["buckets"].as_array().unwrap();
    assert_eq!(default_buckets.len(), 12);
    let distance_sum: f64 = default_buckets
        .iter()
        .map(|bucket| num(bucket, "distance_km"))
        .sum();
    close(distance_sum, 225.0);
    assert!(
        default_buckets
            .iter()
            .any(|bucket| int(bucket, "drive_count") == 0)
    );

    let years = authed(
        &state,
        "GET",
        &format!("/api/v1/vehicles/{vehicle_id}/stats/series?period=year"),
        token,
    )
    .await;
    let years: serde_json::Value = serde_json::from_str(&years).unwrap();
    let year_buckets = years["buckets"].as_array().unwrap();
    assert_eq!(year_buckets.len(), 5);
    let year_distance: f64 = year_buckets
        .iter()
        .map(|bucket| num(bucket, "distance_km"))
        .sum();
    close(year_distance, 225.0);

    let temps = authed(
        &state,
        "GET",
        &format!("/api/v1/vehicles/{vehicle_id}/stats/consumption_by_temp?period=month&count=4"),
        token,
    )
    .await;
    assert!(!temps.contains(vin), "{temps}");
    let temps: serde_json::Value = serde_json::from_str(&temps).unwrap();
    let temps = temps.as_array().unwrap();
    assert_eq!(temps.len(), 3);
    assert_eq!(int(&temps[0], "temp_c_bucket"), -5);
    close(num(&temps[0], "distance_km"), 20.0);
    close(num(&temps[0], "wh_per_km"), 250.0);
    assert_eq!(int(&temps[1], "temp_c_bucket"), 5);
    close(num(&temps[1], "distance_km"), 140.0);
    close(num(&temps[1], "wh_per_km"), 11.75 * 1_000.0 / 140.0);
    assert_eq!(int(&temps[2], "temp_c_bucket"), 10);
    close(num(&temps[2], "distance_km"), 50.0);
    close(num(&temps[2], "wh_per_km"), 200.0);

    // A repeat inside the 5 minute cache must ignore a later edit.
    sqlx::query("UPDATE drives SET distance_km = 0 WHERE vehicle_id = $1 AND distance_km = 100")
        .bind(vehicle_id)
        .execute(&db.pool)
        .await
        .unwrap();
    let cached = authed(
        &state,
        "GET",
        &format!("/api/v1/vehicles/{vehicle_id}/stats/series?period=month&count=4"),
        token,
    )
    .await;
    let cached: serde_json::Value = serde_json::from_str(&cached).unwrap();
    close(num(&cached["buckets"][3], "distance_km"), 140.0);

    // Benchmark note: on a 9.3M-row positions table this used to plan for ~1.7s
    // and execute for 12–85s, because range_at_100_km scanned positions and
    // charge_samples through a subquery bound (no partition pruning) and then
    // nested-looped every bucket. The statement below must stay off those
    // tables. Bounds are timestamptz literals and buckets join on date_trunc.
    let window = crate::http::stats::stats_window("week", 12, "Europe/Paris", Utc::now()).unwrap();
    let sql = crate::http::stats::series_query_sql(&window);
    assert!(!sql.contains("positions"), "{sql}");
    assert!(!sql.contains("charge_samples"), "{sql}");
    assert!(sql.contains("date_trunc('week'"));
    assert!(sql.contains("TIMESTAMPTZ '"));
    assert!(!sql.contains(">= b.start_at"));
    let explained = format!("EXPLAIN (FORMAT JSON) {sql}");
    let (plan,): (serde_json::Value,) = sqlx::query_as(&explained)
        .bind(vehicle_id)
        .bind(0.5_f64)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    let rendered = plan.to_string();
    assert!(
        !rendered.contains("positions") && !rendered.contains("charge_samples"),
        "{rendered}"
    );
}

#[tokio::test]
async fn stats_series_uses_iso_weeks_and_keeps_the_boundary_in_the_later_bucket() {
    let Some(db) = TestDb::migrated("statsweek").await else {
        return;
    };
    let state = crate::app::test_state(db.pool.clone()).await;
    let token = "tl_week-token";
    sqlx::query("INSERT INTO api_tokens (name, token_hash) VALUES ('ha', $1)")
        .bind(sha256_hex(token))
        .execute(&db.pool)
        .await
        .unwrap();
    let (vehicle_id,): (i64,) = sqlx::query_as(
        "INSERT INTO vehicles (vin, name) VALUES ('5YJ3E1EA7KF000097', 'Week') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let edges: (DateTime<Utc>, DateTime<Utc>) = sqlx::query_as(
        "SELECT
            (date_trunc('week', timezone('Europe/Paris', now())) AT TIME ZONE 'Europe/Paris'),
            (date_trunc('week', timezone('Europe/Paris', now())) AT TIME ZONE 'Europe/Paris')
                - interval '1 second'",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    seed_drive(
        &db.pool,
        DriveSeed {
            vehicle_id,
            start: edges.1,
            end: Some(edges.1 + chrono::Duration::minutes(5)),
            distance: Some(7.0),
            duration_s: Some(100),
            ..DriveSeed::closed(vehicle_id, edges.1)
        },
    )
    .await;
    seed_drive(
        &db.pool,
        DriveSeed {
            vehicle_id,
            start: edges.0,
            end: Some(edges.0 + chrono::Duration::minutes(5)),
            distance: Some(3.0),
            duration_s: Some(200),
            ..DriveSeed::closed(vehicle_id, edges.0)
        },
    )
    .await;
    let body = authed(
        &state,
        "GET",
        &format!("/api/v1/vehicles/{vehicle_id}/stats/series?period=week&count=2"),
        token,
    )
    .await;
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["period"], "week");
    let buckets = parsed["buckets"].as_array().unwrap();
    assert_eq!(buckets.len(), 2);
    close(num(&buckets[0], "distance_km"), 7.0);
    assert_eq!(int(&buckets[0], "drive_count"), 1);
    assert_eq!(int(&buckets[0], "drive_duration_s"), 100);
    assert!(buckets[0]["energy_used_kwh"].is_null());
    assert!(buckets[0]["wh_per_km"].is_null());
    close(num(&buckets[1], "distance_km"), 3.0);
    assert_eq!(int(&buckets[1], "drive_duration_s"), 200);
    assert!(buckets[1]["wh_per_km"].is_null());
    let older = DateTime::parse_from_rfc3339(buckets[0]["start"].as_str().unwrap()).unwrap();
    let newer = DateTime::parse_from_rfc3339(buckets[1]["start"].as_str().unwrap()).unwrap();
    assert_eq!(
        (newer.with_timezone(&Utc) - older.with_timezone(&Utc)).num_days(),
        7
    );
    assert_eq!(newer.with_timezone(&Utc), edges.0);
}

fn assert_empty_bucket(bucket: &serde_json::Value) {
    close(num(bucket, "distance_km"), 0.0);
    close(num(bucket, "energy_used_kwh"), 0.0);
    assert!(bucket["wh_per_km"].is_null());
    close(num(bucket, "energy_charged_kwh"), 0.0);
    assert_eq!(int(bucket, "drive_duration_s"), 0);
    assert_eq!(int(bucket, "drive_count"), 0);
    assert!(bucket["range_at_100_km"].is_null());
    close(num(bucket, "ac_energy_kwh"), 0.0);
    close(num(bucket, "dc_energy_kwh"), 0.0);
    assert!(bucket["dc_share_pct"].is_null());
    assert!(bucket["charge_cost"].is_null());
    assert!(bucket["cost_per_km"].is_null());
    assert!(bucket["idle_range_loss_km_per_day"].is_null());
    assert!(bucket["idle_energy_loss_kwh_per_day"].is_null());
}

fn num(value: &serde_json::Value, key: &str) -> f64 {
    value[key]
        .as_f64()
        .unwrap_or_else(|| panic!("{key} missing in {value}"))
}

fn int(value: &serde_json::Value, key: &str) -> i64 {
    value[key]
        .as_i64()
        .unwrap_or_else(|| panic!("{key} missing in {value}"))
}

fn close(actual: f64, expected: f64) {
    assert!((actual - expected).abs() < 1e-6, "{actual} != {expected}");
}

struct DriveSeed {
    vehicle_id: i64,
    start: DateTime<Utc>,
    end: Option<DateTime<Utc>>,
    distance: Option<f64>,
    duration_s: Option<i32>,
    energy: Option<f64>,
    start_rated: Option<f64>,
    end_rated: Option<f64>,
    start_ideal: Option<f64>,
    end_ideal: Option<f64>,
    start_odo: Option<f64>,
    end_odo: Option<f64>,
    temp: Option<f64>,
    source: &'static str,
}

impl DriveSeed {
    fn closed(vehicle_id: i64, start: DateTime<Utc>) -> Self {
        Self {
            vehicle_id,
            start,
            end: None,
            distance: None,
            duration_s: None,
            energy: None,
            start_rated: None,
            end_rated: None,
            start_ideal: None,
            end_ideal: None,
            start_odo: None,
            end_odo: None,
            temp: None,
            source: "poll",
        }
    }
}

async fn seed_drive(pool: &sqlx::PgPool, seed: DriveSeed) {
    sqlx::query(
        "INSERT INTO drives (
            vehicle_id, start_at, end_at, distance_km, duration_s, energy_used_kwh,
            start_rated_range_km, end_rated_range_km, start_ideal_range_km, end_ideal_range_km,
            start_odometer_km, end_odometer_km, outside_temp_avg_c, source
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14
         )",
    )
    .bind(seed.vehicle_id)
    .bind(seed.start)
    .bind(seed.end)
    .bind(seed.distance)
    .bind(seed.duration_s)
    .bind(seed.energy)
    .bind(seed.start_rated)
    .bind(seed.end_rated)
    .bind(seed.start_ideal)
    .bind(seed.end_ideal)
    .bind(seed.start_odo)
    .bind(seed.end_odo)
    .bind(seed.temp)
    .bind(seed.source)
    .execute(pool)
    .await
    .unwrap();
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
