// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use axum::body::Body;
use chrono::{DateTime, TimeZone, Utc};
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
        "/api/v1/vehicles/{id}/car.png",
        "/api/v1/drives",
        "/api/v1/charges",
        "/api/v1/stats",
        "/api/v1/vehicles/{id}/stats/series",
        "/api/v1/vehicles/{id}/stats/consumption_by_temp",
        "/api/v1/geofences",
        "/api/v1/fleet/usage",
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
        "scheduled_charging_mode",
        "scheduled_departure_time",
        "off_peak_charging_enabled",
        "off_peak_hours_end_time",
        "charge_status",
        "car_render",
        "today_calls",
        "month_calls",
        "estimated_cost_usd",
        "vehicle_data_paused",
        "blocked_since",
        "next_probe_at",
        "probe_with_list",
        "range_at_100_km",
        "idle_range_loss_km_per_day",
        "temp_c_bucket",
        "drive_duration_s",
        "dc_share_pct",
        "start_address:",
        "end_address:",
        "start_geofence:",
        "end_geofence:",
        "addresses.display_name",
        "geofences.name",
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
    assert!(state_body.contains("\"charge_status\":\"plugged\""));
    assert!(state_body.contains("\"is_preconditioning\":true"));
    assert!(state_body.contains("\"elevation_m\":35"));
    assert!(!state_body.contains("stream_status"));
    assert!(!state_body.contains("position_mode"));
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
async fn drives_and_charges_include_address_and_geofence_names() {
    let Some(db) = TestDb::migrated("addr").await else {
        return;
    };
    let state = test_state(db.pool.clone()).await;
    let token = "tl_addr-token";
    sqlx::query("INSERT INTO api_tokens (name, token_hash) VALUES ('addr', $1)")
        .bind(sha256_hex(token))
        .execute(&db.pool)
        .await
        .unwrap();
    let vehicle_id: i64 = sqlx::query_scalar(
        "INSERT INTO vehicles (vin, name) VALUES ('5YJ300000000ADDR1', 'Addr') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    // Synthetic points only. These are not a street, a home, or a charger.
    let start_address: i64 = sqlx::query_scalar(
        "INSERT INTO addresses (latitude, longitude, display_name)
         VALUES (0.0, 0.0, 'Fixture Origin') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let end_address: i64 = sqlx::query_scalar(
        "INSERT INTO addresses (latitude, longitude, display_name)
         VALUES (0.0, 1.0, 'Fixture Destination') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let blank_address: i64 = sqlx::query_scalar(
        "INSERT INTO addresses (latitude, longitude, display_name)
         VALUES (1.0, 0.0, NULL) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let start_fence: i64 = sqlx::query_scalar(
        "INSERT INTO geofences (name, latitude, longitude, radius_m)
         VALUES ('Fixture Bay', 0.0, 0.0, 25) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let end_fence: i64 = sqlx::query_scalar(
        "INSERT INTO geofences (name, latitude, longitude, radius_m)
         VALUES ('Fixture Pad', 0.0, 1.0, 25) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let charge_fence: i64 = sqlx::query_scalar(
        "INSERT INTO geofences (name, latitude, longitude, radius_m)
         VALUES ('Fixture Stall', 1.0, 0.0, 25) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let named_drive: i64 = sqlx::query_scalar(
        "INSERT INTO drives (
            vehicle_id, start_at, end_at, distance_km, source,
            start_address_id, end_address_id, start_geofence_id, end_geofence_id
         ) VALUES (
            $1, '2024-07-01T10:00:00Z', '2024-07-01T10:20:00Z', 3.0, 'poll', $2, $3, $4, $5
         ) RETURNING id",
    )
    .bind(vehicle_id)
    .bind(start_address)
    .bind(end_address)
    .bind(start_fence)
    .bind(end_fence)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let bare_drive: i64 = sqlx::query_scalar(
        "INSERT INTO drives (vehicle_id, start_at, end_at, distance_km, source)
         VALUES ($1, '2024-07-01T12:00:00Z', '2024-07-01T12:10:00Z', 1.0, 'poll')
         RETURNING id",
    )
    .bind(vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let named_charge: i64 = sqlx::query_scalar(
        "INSERT INTO charges (
            vehicle_id, start_at, end_at, energy_added_kwh, source, address_id, geofence_id
         ) VALUES (
            $1, '2024-07-01T11:00:00Z', '2024-07-01T11:30:00Z', 4.0, 'poll', $2, $3
         ) RETURNING id",
    )
    .bind(vehicle_id)
    .bind(end_address)
    .bind(charge_fence)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let blank_charge: i64 = sqlx::query_scalar(
        "INSERT INTO charges (vehicle_id, start_at, end_at, energy_added_kwh, source, address_id)
         VALUES ($1, '2024-07-01T13:00:00Z', '2024-07-01T13:20:00Z', 2.0, 'poll', $2)
         RETURNING id",
    )
    .bind(vehicle_id)
    .bind(blank_address)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let bare_charge: i64 = sqlx::query_scalar(
        "INSERT INTO charges (vehicle_id, start_at, end_at, energy_added_kwh, source)
         VALUES ($1, '2024-07-01T14:00:00Z', '2024-07-01T14:10:00Z', 1.0, 'poll')
         RETURNING id",
    )
    .bind(vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();

    let list = authed(
        &state,
        "GET",
        &format!("/api/v1/drives?vehicle_id={vehicle_id}"),
        token,
    )
    .await;
    let parsed: serde_json::Value = serde_json::from_str(&list).unwrap();
    let rows = parsed["data"].as_array().unwrap();
    let named = row_by_id(rows, named_drive);
    assert_eq!(named["start_address_id"].as_i64(), Some(start_address));
    assert_eq!(named["end_address_id"].as_i64(), Some(end_address));
    assert_eq!(named["start_address"], "Fixture Origin");
    assert_eq!(named["end_address"], "Fixture Destination");
    assert_eq!(named["start_geofence_id"].as_i64(), Some(start_fence));
    assert_eq!(named["end_geofence_id"].as_i64(), Some(end_fence));
    assert_eq!(named["start_geofence"], "Fixture Bay");
    assert_eq!(named["end_geofence"], "Fixture Pad");
    let bare = row_by_id(rows, bare_drive);
    for key in [
        "start_address_id",
        "end_address_id",
        "start_address",
        "end_address",
        "start_geofence_id",
        "end_geofence_id",
        "start_geofence",
        "end_geofence",
    ] {
        assert!(bare.get(key).is_some(), "missing {key}");
        assert!(bare[key].is_null(), "{key}");
    }

    let detail: serde_json::Value = serde_json::from_str(
        &authed(
            &state,
            "GET",
            &format!("/api/v1/drives/{named_drive}"),
            token,
        )
        .await,
    )
    .unwrap();
    assert_eq!(detail["start_address"], "Fixture Origin");
    assert_eq!(detail["end_address"], "Fixture Destination");
    assert_eq!(detail["start_geofence"], "Fixture Bay");
    assert_eq!(detail["end_geofence"], "Fixture Pad");
    assert!(detail["route"].is_array());

    let charges: serde_json::Value = serde_json::from_str(
        &authed(
            &state,
            "GET",
            &format!("/api/v1/charges?vehicle_id={vehicle_id}"),
            token,
        )
        .await,
    )
    .unwrap();
    let charge_rows = charges["data"].as_array().unwrap();
    let named_row = row_by_id(charge_rows, named_charge);
    assert_eq!(named_row["address_id"].as_i64(), Some(end_address));
    assert_eq!(named_row["address"], "Fixture Destination");
    assert_eq!(named_row["geofence_id"].as_i64(), Some(charge_fence));
    assert_eq!(named_row["geofence"], "Fixture Stall");
    let blank_row = row_by_id(charge_rows, blank_charge);
    assert_eq!(blank_row["address_id"].as_i64(), Some(blank_address));
    assert!(blank_row.get("address").is_some());
    assert!(blank_row["address"].is_null());
    assert!(blank_row.get("geofence").is_some());
    assert!(blank_row["geofence"].is_null());
    let bare_row = row_by_id(charge_rows, bare_charge);
    for key in ["address_id", "address", "geofence_id", "geofence"] {
        assert!(bare_row.get(key).is_some(), "missing {key}");
        assert!(bare_row[key].is_null(), "{key}");
    }

    let charge_detail: serde_json::Value = serde_json::from_str(
        &authed(
            &state,
            "GET",
            &format!("/api/v1/charges/{named_charge}"),
            token,
        )
        .await,
    )
    .unwrap();
    assert_eq!(charge_detail["address"], "Fixture Destination");
    assert_eq!(charge_detail["geofence"], "Fixture Stall");
    assert!(charge_detail["samples"].is_array());
}

fn row_by_id(rows: &[serde_json::Value], id: i64) -> &serde_json::Value {
    rows.iter()
        .find(|row| row["id"].as_i64() == Some(id))
        .unwrap_or_else(|| panic!("missing id {id} in {rows:?}"))
}

async fn api_status(state: &crate::app::AppState, uri: &str, token: &str) -> (u16, String) {
    let response = router(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn a_vehicle_scoped_token_sees_only_its_charges() {
    let Some(db) = TestDb::migrated("charge-scope").await else {
        return;
    };
    let state = test_state(db.pool.clone()).await;
    let allowed_vehicle: i64 = sqlx::query_scalar(
        "INSERT INTO vehicles (vin, name) VALUES ('5YJ300000SCOPE001', 'Mine') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let other_vehicle: i64 = sqlx::query_scalar(
        "INSERT INTO vehicles (vin, name) VALUES ('5YJ300000SCOPE002', 'Theirs') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let allowed_charge: i64 = sqlx::query_scalar(
        "INSERT INTO charges (vehicle_id, start_at, end_at, energy_added_kwh, source)
         VALUES ($1, '2024-07-01T11:00:00Z', '2024-07-01T11:30:00Z', 4.0, 'poll')
         RETURNING id",
    )
    .bind(allowed_vehicle)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let other_charge: i64 = sqlx::query_scalar(
        "INSERT INTO charges (vehicle_id, start_at, end_at, energy_added_kwh, source)
         VALUES ($1, '2024-07-02T11:00:00Z', '2024-07-02T11:30:00Z', 5.0, 'poll')
         RETURNING id",
    )
    .bind(other_vehicle)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let token = "tl_scope-token";
    sqlx::query("INSERT INTO api_tokens (name, token_hash, vehicle_ids) VALUES ('scope', $1, $2)")
        .bind(sha256_hex(token))
        .bind([allowed_vehicle])
        .execute(&db.pool)
        .await
        .unwrap();

    let (status, body) = api_status(&state, "/api/v1/charges", token).await;
    assert_eq!(status, 200, "{body}");
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let ids: Vec<i64> = parsed["data"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["id"].as_i64())
        .collect();
    assert_eq!(ids, [allowed_charge]);

    let (status, _) = api_status(
        &state,
        &format!("/api/v1/charges?vehicle_id={other_vehicle}"),
        token,
    )
    .await;
    assert_eq!(status, 404);
    let (status, _) = api_status(
        &state,
        &format!("/api/v1/charges?vehicle_id={allowed_vehicle}"),
        token,
    )
    .await;
    assert_eq!(status, 200);
    let (status, _) = api_status(&state, &format!("/api/v1/charges/{other_charge}"), token).await;
    assert_eq!(status, 404);
    let (status, body) =
        api_status(&state, &format!("/api/v1/charges/{allowed_charge}"), token).await;
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("\"samples\""));
}

#[tokio::test]
async fn a_vehicle_scoped_token_filters_the_websocket() {
    use futures_util::StreamExt;

    let Some(db) = TestDb::migrated("ws-scope").await else {
        return;
    };
    let state = test_state(db.pool.clone()).await;
    let allowed_vehicle: i64 = sqlx::query_scalar(
        "INSERT INTO vehicles (vin, name) VALUES ('5YJ3000000WSCOPE1', 'Mine') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let other_vehicle: i64 = sqlx::query_scalar(
        "INSERT INTO vehicles (vin, name) VALUES ('5YJ3000000WSCOPE2', 'Theirs') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let token = "tl_ws-scope-token";
    sqlx::query("INSERT INTO api_tokens (name, token_hash, vehicle_ids) VALUES ('scope', $1, $2)")
        .bind(sha256_hex(token))
        .bind([allowed_vehicle])
        .execute(&db.pool)
        .await
        .unwrap();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let events = state.inner.events.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router(state).into_make_service())
            .await
            .unwrap();
    });
    let url = format!("ws://{addr}/api/v1/ws?access_token={token}");
    let (mut socket, response) = tokio_tungstenite::connect_async(url).await.unwrap();
    assert_eq!(response.status().as_u16(), 101);

    // Only the allowed vehicle's snapshot frame arrives.
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let frame: serde_json::Value =
        serde_json::from_str(first.into_text().unwrap().as_str()).unwrap();
    assert_eq!(frame["vehicle_id"].as_i64(), Some(allowed_vehicle));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), socket.next())
            .await
            .is_err(),
        "the other vehicle must not be announced"
    );

    // Live events for other vehicles are dropped, ours go through.
    let mut hidden = crate::persist::LiveEvent::skeleton("5YJ3000000WSCOPE2", "online");
    hidden.vehicle_id = other_vehicle;
    events.send(hidden).unwrap();
    let mut visible = crate::persist::LiveEvent::skeleton("5YJ3000000WSCOPE1", "online");
    visible.vehicle_id = allowed_vehicle;
    events.send(visible).unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let frame: serde_json::Value =
        serde_json::from_str(event.into_text().unwrap().as_str()).unwrap();
    assert_eq!(frame["vehicle_id"].as_i64(), Some(allowed_vehicle));

    drop(socket);
    server.abort();
}

#[tokio::test]
async fn drive_and_charge_name_joins_use_primary_keys() {
    let Some(db) = TestDb::migrated("addrplan").await else {
        return;
    };
    let pkeys: Vec<(String, String)> = sqlx::query_as(
        "SELECT t.relname::text, c.relname::text
         FROM pg_index i
         JOIN pg_class c ON c.oid = i.indexrelid
         JOIN pg_class t ON t.oid = i.indrelid
         JOIN pg_namespace n ON n.oid = t.relnamespace
         WHERE n.nspname = 'public'
           AND t.relname IN ('addresses', 'geofences')
           AND i.indisprimary",
    )
    .fetch_all(&db.pool)
    .await
    .unwrap();
    assert!(
        pkeys
            .iter()
            .any(|(table, index)| table == "addresses" && index == "addresses_pkey"),
        "{pkeys:?}"
    );
    assert!(
        pkeys
            .iter()
            .any(|(table, index)| table == "geofences" && index == "geofences_pkey"),
        "{pkeys:?}"
    );

    let drive_sql = crate::http::api::drive_select(
        "WHERE ($1::bigint IS NULL OR d.vehicle_id = $1)
           AND ($2::timestamptz IS NULL OR d.start_at >= $2)
           AND ($3::timestamptz IS NULL OR d.start_at < $3)
         ORDER BY d.start_at DESC
         LIMIT $4 OFFSET $5",
    );
    assert_eq!(drive_sql.matches("SELECT").count(), 1, "{drive_sql}");
    assert_eq!(drive_sql.matches("LEFT JOIN addresses").count(), 2);
    assert_eq!(drive_sql.matches("LEFT JOIN geofences").count(), 2);
    let charge_sql = crate::http::api::charge_select(
        "WHERE ($1::bigint IS NULL OR c.vehicle_id = $1)
           AND ($2::timestamptz IS NULL OR c.start_at >= $2)
           AND ($3::timestamptz IS NULL OR c.start_at < $3)
         ORDER BY c.start_at DESC LIMIT $4 OFFSET $5",
    );
    assert_eq!(charge_sql.matches("SELECT").count(), 1, "{charge_sql}");
    assert_eq!(charge_sql.matches("LEFT JOIN addresses").count(), 1);
    assert_eq!(charge_sql.matches("LEFT JOIN geofences").count(), 1);

    let mut conn = db.pool.acquire().await.unwrap();
    sqlx::query("SET enable_seqscan = off")
        .execute(&mut *conn)
        .await
        .unwrap();
    let drive_plan = explain_page(&mut conn, &drive_sql).await;
    assert_pk_lookups(&drive_plan, "addresses", "addresses_pkey");
    assert_pk_lookups(&drive_plan, "geofences", "geofences_pkey");
    let charge_plan = explain_page(&mut conn, &charge_sql).await;
    assert_pk_lookups(&charge_plan, "addresses", "addresses_pkey");
    assert_pk_lookups(&charge_plan, "geofences", "geofences_pkey");
}

async fn explain_page(conn: &mut sqlx::PgConnection, sql: &str) -> serde_json::Value {
    let explained = format!("EXPLAIN (FORMAT JSON) {sql}");
    let (plan,): (serde_json::Value,) = sqlx::query_as(&explained)
        .bind(None::<i64>)
        .bind(None::<DateTime<Utc>>)
        .bind(None::<DateTime<Utc>>)
        .bind(50_i64)
        .bind(0_i64)
        .fetch_one(&mut *conn)
        .await
        .unwrap();
    plan
}

fn assert_pk_lookups(plan: &serde_json::Value, relation: &str, index: &str) {
    let mut scans = Vec::new();
    walk_relations(plan, &mut scans);
    let matched: Vec<_> = scans
        .iter()
        .filter(|(name, _, _)| name == relation)
        .collect();
    assert!(
        !matched.is_empty(),
        "expected a lookup of {relation}, plan was {plan}"
    );
    for (name, node_type, index_name) in &matched {
        assert!(
            matches!(
                node_type.as_str(),
                "Index Scan" | "Index Only Scan" | "Bitmap Index Scan"
            ),
            "{name} used {node_type}, plan was {plan}"
        );
        assert_eq!(index_name.as_deref(), Some(index), "{plan}");
    }
}

fn walk_relations(node: &serde_json::Value, out: &mut Vec<(String, String, Option<String>)>) {
    let nodes: Vec<&serde_json::Value> = if let Some(list) = node.as_array() {
        list.iter()
            .map(|entry| entry.get("Plan").unwrap_or(entry))
            .collect()
    } else if let Some(plan) = node.get("Plan") {
        vec![plan]
    } else {
        vec![node]
    };
    for node in nodes {
        collect_relations(node, out);
    }
}

fn collect_relations(node: &serde_json::Value, out: &mut Vec<(String, String, Option<String>)>) {
    let node_type = node
        .get("Node Type")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    if let Some(relation) = node.get("Relation Name").and_then(|value| value.as_str()) {
        out.push((
            relation.to_owned(),
            node_type.to_owned(),
            node.get("Index Name")
                .and_then(|value| value.as_str())
                .map(str::to_owned),
        ));
    }
    if let Some(children) = node.get("Plans").and_then(|value| value.as_array()) {
        for child in children {
            collect_relations(child, out);
        }
    }
}

#[tokio::test]
async fn polling_sequence_opens_and_closes_one_drive() {
    let Some(db) = TestDb::migrated("drive").await else {
        return;
    };
    let (events, _) = tokio::sync::broadcast::channel(8);
    let archive = Archive::new(
        db.pool.clone(),
        crate::geocode::Geocoder::disabled(),
        events,
        crate::drive_gps::GpsBoard::default(),
    );
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
    let mut parked_long_enough = Observation::at(
        Utc.with_ymd_and_hms(2024, 6, 1, 12, 30, 0).unwrap(),
        Presence::Asleep,
    );
    parked_long_enough.shift_state = Some("P".to_owned());
    apply_for_vin(&archive, vin, identity(), parked_long_enough, false, false)
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
async fn short_drives_are_deleted_for_poll_and_telemetry() {
    let Some(db) = TestDb::migrated("short").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let poll_vin = "5YJ3E1EA7KF000010";
    let telemetry_vin = "5YJ3E1EA7KF000011";
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
    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        Observation::at(base + chrono::Duration::minutes(25), Presence::Asleep),
        false,
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
    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        Observation::at(base + chrono::Duration::minutes(45), Presence::Asleep),
        false,
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
    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        Observation::at(base + chrono::Duration::minutes(65), Presence::Asleep),
        false,
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
    apply_for_vin(
        &archive,
        poll_vin,
        car(poll_vin),
        Observation::at(base + chrono::Duration::minutes(85), Presence::Asleep),
        false,
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

    // Telemetry: park grace must elapse before a short drive is closed and deleted.
    let t0 = Utc::now();
    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(t0, Some("D"), Some(10.0), Some(48.0), Some(2.0)),
        true,
        true,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(5),
            Some("P"),
            Some(10.0),
            None,
            None,
        ),
        true,
        true,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(40),
            Some("P"),
            Some(10.0),
            None,
            None,
        ),
        true,
        true,
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, telemetry_vin).await.0, 0);
    assert_eq!(
        open_drive_count(&db.pool, telemetry_vin).await,
        1,
        "park confirmation has not elapsed"
    );
    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(305),
            Some("P"),
            Some(10.0),
            None,
            None,
        ),
        true,
        true,
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, telemetry_vin).await.0, 0);
    assert!(detached_positions(&db.pool, telemetry_vin).await >= 1);

    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(400),
            Some("D"),
            Some(20.0),
            Some(48.1),
            Some(2.1),
        ),
        true,
        true,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(410),
            Some("P"),
            Some(20.0),
            Some(49.2),
            Some(3.2),
        ),
        true,
        true,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(710),
            Some("P"),
            Some(20.0),
            Some(49.2),
            Some(3.2),
        ),
        true,
        true,
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, telemetry_vin).await.0, 0);

    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(800),
            Some("D"),
            Some(30.0),
            Some(48.2),
            Some(2.2),
        ),
        true,
        true,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(810),
            Some("P"),
            Some(30.009),
            Some(48.21),
            Some(2.21),
        ),
        true,
        true,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(1_110),
            Some("P"),
            Some(30.009),
            Some(48.21),
            Some(2.21),
        ),
        true,
        true,
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, telemetry_vin).await.0, 0);

    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(1_200),
            Some("D"),
            Some(40.0),
            Some(48.3),
            Some(2.3),
        ),
        true,
        true,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(1_210),
            Some("P"),
            Some(40.01),
            Some(48.31),
            Some(2.31),
        ),
        true,
        true,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        telemetry_vin,
        car(telemetry_vin),
        telemetry_fix(
            t0 + chrono::Duration::seconds(1_510),
            Some("P"),
            Some(40.01),
            Some(48.31),
            Some(2.31),
        ),
        true,
        true,
    )
    .await
    .unwrap();
    let (telemetry_count, telemetry_distance) = closed_drives(&db.pool, telemetry_vin).await;
    assert_eq!(telemetry_count, 1);
    assert!((telemetry_distance.unwrap() - 0.01).abs() < 1e-6);
    let telemetry_vehicle = vehicle_id(&db.pool, telemetry_vin).await;
    let telemetry_stats = authed(
        &state,
        "GET",
        &format!("/api/v1/stats?vehicle_id={telemetry_vehicle}"),
        token,
    )
    .await;
    let telemetry_stats: serde_json::Value = serde_json::from_str(&telemetry_stats).unwrap();
    assert_eq!(telemetry_stats["drive_count"], 1);
}

fn test_archive(pool: sqlx::PgPool) -> Archive {
    let (events, _) = tokio::sync::broadcast::channel(8);
    Archive::new(
        pool,
        crate::geocode::Geocoder::disabled(),
        events,
        crate::drive_gps::GpsBoard::default(),
    )
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

/// A Fleet Telemetry sample: gear, speed, odometer and an optional location.
fn telemetry_fix(
    at: chrono::DateTime<Utc>,
    shift: Option<&str>,
    odometer_km: Option<f64>,
    latitude: Option<f64>,
    longitude: Option<f64>,
) -> Observation {
    let mut obs = Observation::at(at, Presence::Online);
    obs.shift_state = shift.map(str::to_owned);
    obs.speed_kmh = Some(if shift == Some("P") { 0.0 } else { 30.0 });
    obs.power_kw = Some(10.0);
    obs.odometer_km = odometer_km;
    obs.battery_level = Some(80.0);
    obs.latitude = latitude;
    obs.longitude = longitude;
    obs.elevation_m = Some(30.0);
    obs.heading_deg = Some(90.0);
    obs.rated_range_km = Some(400.0);
    obs.est_range_km = Some(380.0);
    obs
}

async fn vehicle_id(pool: &sqlx::PgPool, vin: &str) -> i64 {
    let row: (i64,) = sqlx::query_as("SELECT id FROM vehicles WHERE vin = $1")
        .bind(vin)
        .fetch_one(pool)
        .await
        .unwrap();
    row.0
}

async fn open_drive_count(pool: &sqlx::PgPool, vin: &str) -> i64 {
    let row: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM drives d
         JOIN vehicles v ON v.id = d.vehicle_id
         WHERE v.vin = $1 AND d.end_at IS NULL",
    )
    .bind(vin)
    .fetch_one(pool)
    .await
    .unwrap();
    row.0
}

async fn positions_on_drive(pool: &sqlx::PgPool, drive_id: i64) -> i64 {
    let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM positions WHERE drive_id = $1")
        .bind(drive_id)
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
async fn short_gap_at_speed_keeps_one_drive() {
    let Some(db) = TestDb::migrated("gap10").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000070";
    let base = Utc.with_ymd_and_hms(2024, 8, 1, 12, 42, 0).unwrap();
    let mut fast = shifted(base, "D", Some(100.0), 10.0, 20.0);
    fast.speed_kmh = Some(61.0);
    apply_for_vin(&archive, vin, car(vin), fast, true, false)
        .await
        .unwrap();
    let open = open_drive_id(&db.pool, vin).await.expect("open");
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        Observation::at(base + chrono::Duration::seconds(5), Presence::Asleep),
        false,
        false,
    )
    .await
    .unwrap();
    assert_eq!(open_drive_id(&db.pool, vin).await, Some(open));
    let mut again = shifted(
        base + chrono::Duration::seconds(10),
        "D",
        Some(100.18),
        10.0016,
        20.0,
    );
    again.speed_kmh = Some(71.0);
    apply_for_vin(&archive, vin, car(vin), again, true, false)
        .await
        .unwrap();
    assert_eq!(open_drive_count(&db.pool, vin).await, 1);
    assert_eq!(closed_drives(&db.pool, vin).await.0, 0);
    assert_eq!(open_drive_id(&db.pool, vin).await, Some(open));
}

#[tokio::test]
async fn parked_stop_stays_two_drives() {
    let Some(db) = TestDb::migrated("parked").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000071";
    let base = Utc.with_ymd_and_hms(2024, 8, 2, 9, 0, 0).unwrap();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(base, "D", Some(10.0), 10.0, 20.0),
        true,
        false,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(
            base + chrono::Duration::seconds(60),
            "D",
            Some(11.0),
            10.01,
            20.0,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(
            base + chrono::Duration::seconds(61),
            "P",
            Some(11.0),
            10.01,
            20.0,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(
            base + chrono::Duration::seconds(61 + 300),
            "P",
            Some(11.0),
            10.01,
            20.0,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    let closed = open_drive_count(&db.pool, vin).await;
    assert_eq!(closed, 0, "park grace elapsed");
    assert_eq!(closed_drives(&db.pool, vin).await.0, 1);
    let park_at = base + chrono::Duration::seconds(61);
    let end: (chrono::DateTime<Utc>,) = sqlx::query_as(
        "SELECT d.end_at FROM drives d
         JOIN vehicles v ON v.id = d.vehicle_id
         WHERE v.vin = $1 AND d.end_at IS NOT NULL",
    )
    .bind(vin)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        end.0, park_at,
        "closed drive ends at the first 0 km/h sample of the final run"
    );
    let kept: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM positions p
         JOIN drives d ON d.id = p.drive_id
         JOIN vehicles v ON v.id = d.vehicle_id
         WHERE v.vin = $1 AND p.recorded_at = $2",
    )
    .bind(vin)
    .bind(park_at)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(kept.0, 1, "the first 0 km/h sample stays on the drive");
    let detached: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM positions p
         JOIN vehicles v ON v.id = p.vehicle_id
         WHERE v.vin = $1 AND p.recorded_at > $2 AND p.drive_id IS NULL",
    )
    .bind(vin)
    .bind(park_at)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        detached.0, 1,
        "later zero-run samples are detached, not deleted"
    );
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(
            base + chrono::Duration::seconds(61 + 310),
            "D",
            Some(11.0),
            10.01,
            20.0,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    assert_eq!(open_drive_count(&db.pool, vin).await, 1);
    assert_eq!(closed_drives(&db.pool, vin).await.0, 1);
}

#[tokio::test]
async fn telemetry_confirms_park_and_keeps_a_stop_in_drive() {
    let Some(db) = TestDb::migrated("telpark").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000091";
    let base = Utc::now();
    let send = |at, shift: &str, speed: f64, odo: f64, lat: f64| {
        let mut obs = shifted(at, shift, Some(odo), lat, 2.2);
        obs.speed_kmh = Some(speed);
        apply_for_vin(&archive, vin, car(vin), obs, true, true)
    };
    send(base, "D", 40.0, 100.0, 48.1).await.unwrap();
    send(base + chrono::Duration::seconds(30), "D", 0.0, 100.2, 48.11)
        .await
        .unwrap();
    send(
        base + chrono::Duration::seconds(600),
        "D",
        0.0,
        100.2,
        48.11,
    )
    .await
    .unwrap();
    assert_eq!(
        open_drive_count(&db.pool, vin).await,
        1,
        "ten minutes at 0 km/h in D stays one trip"
    );
    let park = base + chrono::Duration::seconds(620);
    send(park, "P", 0.0, 100.2, 48.11).await.unwrap();
    send(
        park + chrono::Duration::seconds(30),
        "D",
        20.0,
        100.3,
        48.111,
    )
    .await
    .unwrap();
    assert_eq!(
        closed_drives(&db.pool, vin).await.0,
        0,
        "P for 30 s then D is still one trip"
    );
    let again = park + chrono::Duration::seconds(40);
    send(again, "P", 0.0, 100.4, 48.12).await.unwrap();
    send(
        again + chrono::Duration::seconds(60),
        "P",
        0.0,
        100.4,
        48.12,
    )
    .await
    .unwrap();
    let (count, end, source): (i64, chrono::DateTime<Utc>, String) = sqlx::query_as(
        "SELECT COUNT(*), MAX(d.end_at), MAX(d.source) FROM drives d
         JOIN vehicles v ON v.id = d.vehicle_id
         WHERE v.vin = $1 AND d.end_at IS NOT NULL",
    )
    .bind(vin)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    assert!(
        end.signed_duration_since(again).num_milliseconds().abs() <= 1,
        "end is the P transition ({end} vs {again})"
    );
    assert_eq!(source, "telemetry");
    let points: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM positions p
         JOIN vehicles v ON v.id = p.vehicle_id
         WHERE v.vin = $1 AND p.source = 'telemetry'",
    )
    .bind(vin)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert!(points.0 >= 2);
    let repeat = points.0;
    send(
        again + chrono::Duration::seconds(61),
        "P",
        0.0,
        100.4,
        48.12,
    )
    .await
    .unwrap();
    let after: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM positions p
         JOIN vehicles v ON v.id = p.vehicle_id
         WHERE v.vin = $1 AND p.source = 'telemetry'",
    )
    .bind(vin)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(after.0, repeat, "unchanged location does not add a point");
}

#[tokio::test]
async fn late_telemetry_does_not_open_a_second_drive() {
    let Some(db) = TestDb::migrated("tellate").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000092";
    let base = Utc::now();
    let mut rolling = shifted(base, "D", Some(50.0), 48.2, 2.3);
    rolling.speed_kmh = Some(35.0);
    apply_for_vin(&archive, vin, car(vin), rolling, true, true)
        .await
        .unwrap();
    let mut later = shifted(
        base + chrono::Duration::seconds(5),
        "D",
        Some(50.1),
        48.21,
        2.31,
    );
    later.speed_kmh = Some(36.0);
    apply_for_vin(&archive, vin, car(vin), later, true, true)
        .await
        .unwrap();
    assert_eq!(
        open_drive_count(&db.pool, vin).await,
        1,
        "later telemetry joins the open drive"
    );
    let park = base + chrono::Duration::seconds(20);
    let mut parked = shifted(park, "P", Some(50.2), 48.22, 2.32);
    parked.speed_kmh = Some(0.0);
    apply_for_vin(&archive, vin, car(vin), parked, true, true)
        .await
        .unwrap();
    let mut held = shifted(
        park + chrono::Duration::seconds(60),
        "P",
        Some(50.2),
        48.23,
        2.33,
    );
    held.speed_kmh = Some(0.0);
    apply_for_vin(&archive, vin, car(vin), held, true, true)
        .await
        .unwrap();
    assert_eq!(open_drive_count(&db.pool, vin).await, 0);
    let stale = base + chrono::Duration::seconds(10);
    let mut late = shifted(stale, "D", Some(50.05), 48.205, 2.305);
    late.speed_kmh = Some(34.0);
    apply_for_vin(&archive, vin, car(vin), late, true, true)
        .await
        .unwrap();
    assert_eq!(
        open_drive_count(&db.pool, vin).await,
        0,
        "a late D does not open a new drive"
    );
    assert_eq!(closed_drives(&db.pool, vin).await.0, 1);
    let attached: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM positions p
         JOIN drives d ON d.id = p.drive_id
         JOIN vehicles v ON v.id = d.vehicle_id
         WHERE v.vin = $1 AND p.recorded_at = $2",
    )
    .bind(vin)
    .bind(stale)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(attached.0, 1, "the late sample stays on the closed drive");
}

#[tokio::test]
async fn restart_gap_does_not_split_a_drive_that_resumes() {
    let Some(db) = TestDb::migrated("telrestart").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000093";
    let base = Utc::now() - chrono::Duration::minutes(10);
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(base, "D", Some(80.0), 48.4, 2.4),
        true,
        false,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(
            base + chrono::Duration::seconds(30),
            "D",
            Some(80.4),
            48.41,
            2.41,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    archive.runtimes.lock().await.clear();
    crate::ingest::expire_drives(&archive, Utc::now())
        .await
        .unwrap();
    assert_eq!(
        open_drive_count(&db.pool, vin).await,
        1,
        "a restart inside the continuity window does not close the drive"
    );
    let mut resume = shifted(Utc::now(), "D", Some(82.0), 48.5, 2.5);
    resume.speed_kmh = Some(50.0);
    apply_for_vin(&archive, vin, car(vin), resume, true, false)
        .await
        .unwrap();
    assert_eq!(open_drive_count(&db.pool, vin).await, 1);
    assert_eq!(closed_drives(&db.pool, vin).await.0, 0);
}

#[tokio::test]
async fn asleep_longer_than_five_minutes_closes_at_the_last_position() {
    let Some(db) = TestDb::migrated("asleep5").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000072";
    let base = Utc.with_ymd_and_hms(2024, 8, 3, 15, 0, 0).unwrap();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(base, "D", Some(5.0), 10.0, 20.0),
        true,
        false,
    )
    .await
    .unwrap();
    let last = base + chrono::Duration::seconds(20);
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(last, "D", Some(6.0), 10.02, 20.0),
        true,
        false,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        Observation::at(last + chrono::Duration::seconds(30), Presence::Asleep),
        false,
        false,
    )
    .await
    .unwrap();
    assert_eq!(open_drive_count(&db.pool, vin).await, 1);
    archive.runtimes.lock().await.clear();
    crate::ingest::expire_drives(&archive, last + chrono::Duration::seconds(10))
        .await
        .unwrap();
    assert_eq!(open_drive_count(&db.pool, vin).await, 1);
    crate::ingest::expire_drives(&archive, last + chrono::Duration::seconds(301))
        .await
        .unwrap();
    assert_eq!(open_drive_count(&db.pool, vin).await, 0);
    let end: (chrono::DateTime<Utc>,) = sqlx::query_as(
        "SELECT d.end_at FROM drives d
         JOIN vehicles v ON v.id = d.vehicle_id
         WHERE v.vin = $1 AND d.end_at IS NOT NULL",
    )
    .bind(vin)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(end.0, last);
}

#[tokio::test]
async fn charge_start_closes_the_drive_immediately() {
    let Some(db) = TestDb::migrated("chargeend").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000073";
    let base = Utc.with_ymd_and_hms(2024, 8, 4, 8, 0, 0).unwrap();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(base, "D", Some(5.0), 10.0, 20.0),
        true,
        false,
    )
    .await
    .unwrap();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(
            base + chrono::Duration::seconds(30),
            "D",
            Some(6.0),
            10.01,
            20.0,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    let mut charging = Observation::at(base + chrono::Duration::seconds(35), Presence::Online);
    charging.charging_state = Some("Charging".to_owned());
    charging.charge_energy_added_kwh = Some(0.1);
    charging.odometer_km = Some(6.0);
    charging.latitude = Some(10.01);
    charging.longitude = Some(20.0);
    apply_for_vin(&archive, vin, car(vin), charging, true, false)
        .await
        .unwrap();
    assert_eq!(open_drive_count(&db.pool, vin).await, 0);
    assert_eq!(closed_drives(&db.pool, vin).await.0, 1);
    let charges: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM charges c
         JOIN vehicles v ON v.id = c.vehicle_id
         WHERE v.vin = $1 AND c.end_at IS NULL",
    )
    .bind(vin)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(charges.0, 1);
}

/// The pauses measured between the nine rows of the 27/11/2026 charge, in seconds.
const CHARGE_NIGHT_GAPS: [i64; 8] = [117, 68, 60, 96, 247, 146, 26, 26];

fn charge_reading(at: DateTime<Utc>, state: &str, soc: f64) -> Observation {
    let mut obs = Observation::at(at, Presence::Online);
    obs.charging_state = Some(state.to_owned());
    obs.charger_power_kw = Some(if state == "Charging" { 7.0 } else { 0.0 });
    obs.battery_level = Some(soc);
    obs.shift_state = Some("P".to_owned());
    obs
}

async fn live_charges(pool: &sqlx::PgPool, vin: &str) -> Vec<(i64, Option<DateTime<Utc>>)> {
    sqlx::query_as(
        "SELECT c.id, c.end_at FROM charges c JOIN vehicles v ON v.id = c.vehicle_id
         WHERE v.vin = $1 AND c.deleted_at IS NULL ORDER BY c.start_at",
    )
    .bind(vin)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn short_charge_pauses_resume_one_charge() {
    let Some(db) = TestDb::migrated("chargegap").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000131";
    let send = |obs: Observation| {
        crate::ingest::apply_telemetry_for_vin(&archive, vin, car(vin), obs, false)
    };
    let mut at = Utc.with_ymd_and_hms(2026, 11, 27, 1, 30, 0).unwrap();
    let mut soc = 59.0;
    for gap in CHARGE_NIGHT_GAPS {
        for _ in 0..10 {
            send(charge_reading(at, "Charging", soc)).await.unwrap();
            at += chrono::Duration::minutes(1);
            soc += 0.15;
        }
        // The car reports a pause; charging resumes `gap` seconds later.
        send(charge_reading(at, "Stopped", soc)).await.unwrap();
        at += chrono::Duration::seconds(gap);
    }
    send(charge_reading(at, "Charging", soc)).await.unwrap();
    send(charge_reading(
        at + chrono::Duration::minutes(5),
        "Complete",
        soc + 0.5,
    ))
    .await
    .unwrap();
    let rows = live_charges(&db.pool, vin).await;
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert!(rows[0].1.is_some(), "closed on Complete");

    // A pause longer than the grace starts a second charge.
    let later = at
        + chrono::Duration::minutes(5)
        + chrono::Duration::seconds(crate::persist::CHARGE_RESUME_GRACE_SECS + 1);
    send(charge_reading(later, "Charging", soc + 0.5))
        .await
        .unwrap();
    assert_eq!(live_charges(&db.pool, vin).await.len(), 2);
}

#[tokio::test]
async fn a_drive_between_two_charges_keeps_them_apart() {
    let Some(db) = TestDb::migrated("chargedrive").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000137";
    let base = Utc.with_ymd_and_hms(2026, 11, 27, 1, 30, 0).unwrap();
    let send = |obs: Observation| apply_for_vin(&archive, vin, car(vin), obs, true, false);
    send(charge_reading(base, "Charging", 50.0)).await.unwrap();
    send(charge_reading(
        base + chrono::Duration::minutes(1),
        "Disconnected",
        51.0,
    ))
    .await
    .unwrap();
    send(shifted(
        base + chrono::Duration::minutes(2),
        "D",
        Some(10.0),
        10.0,
        20.0,
    ))
    .await
    .unwrap();
    send(charge_reading(
        base + chrono::Duration::minutes(4),
        "Charging",
        50.0,
    ))
    .await
    .unwrap();
    assert_eq!(live_charges(&db.pool, vin).await.len(), 2);
}

/// Seeds the nine closed rows a split night charge left behind: power and SoC
/// on every sample, no energy counter and no position.
async fn seed_split_charge(pool: &sqlx::PgPool, vehicle_id: i64, start: DateTime<Utc>) -> Vec<i64> {
    let mut ids = Vec::new();
    let mut at = start;
    let mut soc = 59.0;
    for gap in CHARGE_NIGHT_GAPS.iter().copied().chain([0]) {
        let begin = at;
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO charges (vehicle_id, start_at, source) VALUES ($1, $2, 'poll') RETURNING id",
        )
        .bind(vehicle_id)
        .bind(begin)
        .fetch_one(pool)
        .await
        .unwrap();
        for _ in 0..=10 {
            sqlx::query(
                "INSERT INTO charge_samples (charge_id, recorded_at, battery_level, charger_power_kw, source)
                 VALUES ($1, $2, $3, 7.0, 'poll')",
            )
            .bind(id)
            .bind(at)
            .bind(soc)
            .execute(pool)
            .await
            .unwrap();
            at += chrono::Duration::minutes(1);
            soc += 0.15;
        }
        at -= chrono::Duration::minutes(1);
        sqlx::query("UPDATE charges SET end_at = $2 WHERE id = $1")
            .bind(id)
            .bind(at)
            .execute(pool)
            .await
            .unwrap();
        ids.push(id);
        at += chrono::Duration::seconds(gap);
    }
    ids
}

#[tokio::test]
async fn rebuild_charge_merges_a_split_charge_once() {
    let Some(db) = TestDb::migrated("rebuildcharge").await else {
        return;
    };
    let vin = "5YJ3E1EA7KF000139";
    let (vehicle_id,): (i64,) =
        sqlx::query_as("INSERT INTO vehicles (vin, name) VALUES ($1, 'Test') RETURNING id")
            .bind(vin)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    let (home,): (i64,) = sqlx::query_as(
        "INSERT INTO geofences (name, latitude, longitude, radius_m)
         VALUES ('Home', 10.0, 20.0, 50) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let start = Utc.with_ymd_and_hms(2026, 11, 27, 1, 30, 0).unwrap();
    // The last fix before the night, then a later one somewhere else.
    for (at, latitude) in [
        (start - chrono::Duration::hours(2), 10.0),
        (start + chrono::Duration::hours(6), 11.0),
    ] {
        sqlx::query(
            "INSERT INTO positions (vehicle_id, recorded_at, latitude, longitude, source)
             VALUES ($1, $2, $3, 20.0, 'poll')",
        )
        .bind(vehicle_id)
        .bind(at)
        .bind(latitude)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let ids = seed_split_charge(&db.pool, vehicle_id, start).await;
    assert_eq!(ids.len(), 9);
    // A telemetry point left in a pause between two rows.
    sqlx::query(
        "INSERT INTO positions (vehicle_id, recorded_at, battery_level, source)
         VALUES ($1, $2, 60.7, 'telemetry')",
    )
    .bind(vehicle_id)
    .bind(start + chrono::Duration::seconds(10 * 60 + 30))
    .execute(&db.pool)
    .await
    .unwrap();
    let from = start - chrono::Duration::minutes(5);
    let to = start + chrono::Duration::hours(3);

    let (dry, jobs) = crate::persist::rebuild_charge(&db.pool, vehicle_id, from, to, false)
        .await
        .unwrap();
    assert!(!dry.applied);
    assert!(jobs.is_empty());
    assert_eq!(dry.charge_ids, ids);
    assert_eq!(dry.samples, 99);
    assert_eq!(dry.start_at, start);
    assert!(dry.located);
    assert_eq!(dry.geofence_id, Some(home));
    let kwh = dry.energy_added_kwh.unwrap();
    assert!(kwh > 11.0 && kwh < 12.5, "{kwh}");
    let text = crate::persist::format_charge_rebuild(&dry);
    assert!(text.contains("mode=dry-run"));
    assert!(!text.contains("10.0") && !text.contains(vin), "{text}");
    assert_eq!(
        live_charges_by_id(&db.pool, vehicle_id).await.len(),
        9,
        "a dry run writes nothing"
    );

    let (applied, jobs) = crate::persist::rebuild_charge(&db.pool, vehicle_id, from, to, true)
        .await
        .unwrap();
    assert!(applied.applied);
    assert_eq!(jobs.len(), 1);
    let live = live_charges_by_id(&db.pool, vehicle_id).await;
    assert_eq!(live, vec![ids[0]]);
    type Merged = (
        Option<f64>,
        Option<f64>,
        Option<i64>,
        Option<DateTime<Utc>>,
        Option<f64>,
    );
    let row: Merged = sqlx::query_as(
        "SELECT energy_added_kwh, latitude, geofence_id, end_at, end_battery_level
             FROM charges WHERE id = $1",
    )
    .bind(ids[0])
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(row.0, applied.energy_added_kwh);
    assert_eq!(row.1, Some(10.0), "position before the charge, not after");
    assert_eq!(row.2, Some(home));
    assert_eq!(row.3, Some(applied.end_at));
    assert_eq!(row.4, applied.end_battery_level);
    let (attached,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM positions WHERE charge_id = $1")
        .bind(ids[0])
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(attached, 1, "the point in the pause joins the charge");
    let (trashed,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM charges WHERE vehicle_id = $1 AND deleted_at IS NOT NULL",
    )
    .bind(vehicle_id)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(trashed, 8, "merged rows are soft-deleted, not removed");
    let (is_dc,): (Option<bool>,) = sqlx::query_as("SELECT is_dc FROM charges WHERE id = $1")
        .bind(ids[0])
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(is_dc, Some(false), "the merged 7 kW charge is AC");

    sqlx::query("UPDATE charges SET is_dc = NULL WHERE id = $1")
        .bind(ids[0])
        .execute(&db.pool)
        .await
        .unwrap();
    let (again, _) = crate::persist::rebuild_charge(&db.pool, vehicle_id, from, to, true)
        .await
        .unwrap();
    let (is_dc,): (Option<bool>,) = sqlx::query_as("SELECT is_dc FROM charges WHERE id = $1")
        .bind(ids[0])
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(
        is_dc,
        Some(false),
        "a re-run fills the type of a merged charge"
    );
    assert_eq!(again.charge_ids, vec![ids[0]]);
    assert_eq!(again.energy_added_kwh, applied.energy_added_kwh);
    assert_eq!(live_charges_by_id(&db.pool, vehicle_id).await, vec![ids[0]]);

    assert!(
        crate::persist::rebuild_charge(&db.pool, vehicle_id, to, from, false)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn rebuild_charge_refuses_a_drive_or_a_long_pause() {
    let Some(db) = TestDb::migrated("rebuildchargeno").await else {
        return;
    };
    let (vehicle_id,): (i64,) = sqlx::query_as(
        "INSERT INTO vehicles (vin, name) VALUES ('5YJ3E1EA7KF000141', 'T') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let start = Utc.with_ymd_and_hms(2026, 11, 27, 1, 30, 0).unwrap();
    let ids = seed_split_charge(&db.pool, vehicle_id, start).await;
    let from = start - chrono::Duration::minutes(5);
    let to = start + chrono::Duration::hours(3);
    sqlx::query(
        "INSERT INTO drives (vehicle_id, start_at, end_at, source) VALUES ($1, $2, $3, 'poll')",
    )
    .bind(vehicle_id)
    .bind(start + chrono::Duration::minutes(30))
    .bind(start + chrono::Duration::minutes(31))
    .execute(&db.pool)
    .await
    .unwrap();
    let Err(err) = crate::persist::rebuild_charge(&db.pool, vehicle_id, from, to, true).await
    else {
        panic!("a drive inside the charges is refused");
    };
    assert!(err.to_string().contains("drive"), "{err}");
    sqlx::query("DELETE FROM drives WHERE vehicle_id = $1")
        .bind(vehicle_id)
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE charges SET start_at = start_at + interval '20 minutes' WHERE id = $1")
        .bind(ids[8])
        .execute(&db.pool)
        .await
        .unwrap();
    let Err(err) = crate::persist::rebuild_charge(&db.pool, vehicle_id, from, to, true).await
    else {
        panic!("a long pause is refused");
    };
    assert!(err.to_string().contains("contiguous"), "{err}");
    assert_eq!(live_charges_by_id(&db.pool, vehicle_id).await.len(), 9);
}

async fn live_charges_by_id(pool: &sqlx::PgPool, vehicle_id: i64) -> Vec<i64> {
    sqlx::query_scalar(
        "SELECT id FROM charges WHERE vehicle_id = $1 AND deleted_at IS NULL ORDER BY start_at",
    )
    .bind(vehicle_id)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn reopen_joins_a_closed_drive_when_the_gap_was_still_moving() {
    let Some(db) = TestDb::migrated("reopen").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000074";
    let base = Utc.with_ymd_and_hms(2024, 8, 5, 12, 42, 0).unwrap();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(
            base - chrono::Duration::minutes(5),
            "D",
            Some(99.0),
            10.0,
            20.0,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    let mut end = shifted(base, "D", Some(100.0), 10.0, 20.0);
    end.speed_kmh = Some(61.0);
    apply_for_vin(&archive, vin, car(vin), end, true, false)
        .await
        .unwrap();
    let drive = open_drive_id(&db.pool, vin).await.expect("open");
    sqlx::query("UPDATE drives SET end_at = $2, end_odometer_km = 100, end_latitude = 10, end_longitude = 20 WHERE id = $1")
        .bind(drive)
        .bind(base)
        .execute(&db.pool)
        .await
        .unwrap();
    archive.runtimes.lock().await.clear();
    let mut again = shifted(
        base + chrono::Duration::seconds(10),
        "D",
        Some(100.18),
        10.0016,
        20.0,
    );
    again.speed_kmh = Some(71.0);
    apply_for_vin(&archive, vin, car(vin), again, true, false)
        .await
        .unwrap();
    assert_eq!(open_drive_id(&db.pool, vin).await, Some(drive));
    assert_eq!(closed_drives(&db.pool, vin).await.0, 0);

    sqlx::query("UPDATE drives SET end_at = $2, end_odometer_km = 100.18, end_latitude = 10.0016, end_longitude = 20 WHERE id = $1")
        .bind(drive)
        .bind(base + chrono::Duration::seconds(10))
        .execute(&db.pool)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE positions SET speed_kmh = 0, shift_state = 'P'
         WHERE drive_id = $1 AND recorded_at = $2",
    )
    .bind(drive)
    .bind(base + chrono::Duration::seconds(10))
    .execute(&db.pool)
    .await
    .unwrap();
    let vehicle = vehicle_id(&db.pool, vin).await;
    sqlx::query(
        "INSERT INTO charges (vehicle_id, start_at, end_at, source)
         VALUES ($1, $2, $3, 'poll')",
    )
    .bind(vehicle)
    .bind(base + chrono::Duration::seconds(12))
    .bind(base + chrono::Duration::seconds(40))
    .execute(&db.pool)
    .await
    .unwrap();
    archive.runtimes.lock().await.clear();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(
            base + chrono::Duration::seconds(50),
            "D",
            Some(100.2),
            10.002,
            20.0,
        ),
        true,
        false,
    )
    .await
    .unwrap();
    assert_eq!(closed_drives(&db.pool, vin).await.0, 1);
    assert_eq!(open_drive_count(&db.pool, vin).await, 1);
    assert_ne!(open_drive_id(&db.pool, vin).await, Some(drive));
}

#[tokio::test]
async fn telemetry_start_reuses_the_open_drive() {
    let Some(db) = TestDb::migrated("reuse").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000043";
    let now = Utc::now();
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        shifted(now, "P", Some(10.0), 48.0, 2.0),
        true,
        false,
    )
    .await
    .unwrap();
    let vehicle = vehicle_id(&db.pool, vin).await;
    sqlx::query(
        "INSERT INTO drives (
            vehicle_id, start_at, start_latitude, start_longitude, start_odometer_km, source
         ) VALUES ($1, $2, 48.0, 2.0, 10.0, 'poll')",
    )
    .bind(vehicle)
    .bind(now)
    .execute(&db.pool)
    .await
    .unwrap();
    let existing = open_drive_id(&db.pool, vin)
        .await
        .expect("seeded open drive");
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        telemetry_fix(
            now + chrono::Duration::seconds(10),
            Some("D"),
            Some(10.2),
            Some(48.01),
            Some(2.01),
        ),
        true,
        true,
    )
    .await
    .unwrap();
    assert_eq!(open_drive_count(&db.pool, vin).await, 1);
    assert_eq!(open_drive_id(&db.pool, vin).await, Some(existing));
    assert!(positions_on_drive(&db.pool, existing).await >= 1);
    let duplicate = sqlx::query(
        "INSERT INTO drives (vehicle_id, start_at, source) VALUES ($1, $2, 'telemetry')",
    )
    .bind(vehicle)
    .bind(now)
    .execute(&db.pool)
    .await;
    assert!(duplicate.is_err());
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

#[tokio::test]
async fn repair_drive_ends_is_a_dry_run_until_apply_and_stays_idempotent() {
    let Some(db) = TestDb::migrated("repairends").await else {
        return;
    };
    let vehicle: (i64,) = sqlx::query_as(
        "INSERT INTO vehicles (vin, name) VALUES ('repair-fixture', 'Fixture') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let vehicle_id = vehicle.0;
    let base = Utc.with_ymd_and_hms(2024, 3, 4, 12, 0, 0).unwrap();

    let trim = insert_closed_drive(
        &db.pool,
        vehicle_id,
        base,
        base + chrono::Duration::minutes(2),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(trim),
        base,
        Some(40.0),
        1.0,
        Some("D"),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(trim),
        base + chrono::Duration::minutes(1),
        Some(0.0),
        1.2,
        Some("P"),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(trim),
        base + chrono::Duration::minutes(2),
        None,
        1.2,
        None,
    )
    .await;

    let no_zero_at = base + chrono::Duration::hours(1);
    let no_zero = insert_closed_drive(
        &db.pool,
        vehicle_id,
        no_zero_at,
        no_zero_at + chrono::Duration::minutes(1),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(no_zero),
        no_zero_at,
        Some(20.0),
        2.0,
        Some("D"),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(no_zero),
        no_zero_at + chrono::Duration::minutes(1),
        Some(25.0),
        2.3,
        Some("D"),
    )
    .await;

    let middle_at = base + chrono::Duration::hours(2);
    let middle = insert_closed_drive(
        &db.pool,
        vehicle_id,
        middle_at,
        middle_at + chrono::Duration::minutes(2),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(middle),
        middle_at,
        Some(40.0),
        3.0,
        Some("D"),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(middle),
        middle_at + chrono::Duration::minutes(1),
        Some(0.0),
        3.1,
        Some("P"),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(middle),
        middle_at + chrono::Duration::minutes(2),
        Some(30.0),
        3.4,
        Some("D"),
    )
    .await;

    let zeros_at = base + chrono::Duration::hours(3);
    let zeros = insert_closed_drive(
        &db.pool,
        vehicle_id,
        zeros_at,
        zeros_at + chrono::Duration::minutes(1),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(zeros),
        zeros_at,
        Some(40.0),
        4.0,
        Some("D"),
    )
    .await;
    for minute in 1..=3 {
        insert_fix(
            &db.pool,
            vehicle_id,
            Some(zeros),
            zeros_at + chrono::Duration::minutes(minute),
            Some(0.0),
            4.2,
            Some("P"),
        )
        .await;
    }

    let detached_at = base + chrono::Duration::hours(4);
    let detached = insert_closed_drive(
        &db.pool,
        vehicle_id,
        detached_at,
        detached_at + chrono::Duration::minutes(1),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(detached),
        detached_at,
        Some(40.0),
        5.0,
        Some("D"),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(detached),
        detached_at + chrono::Duration::minutes(1),
        Some(0.0),
        5.2,
        Some("P"),
    )
    .await;
    insert_fix(
        &db.pool,
        vehicle_id,
        None,
        detached_at + chrono::Duration::minutes(2),
        Some(0.0),
        5.2,
        Some("P"),
    )
    .await;

    let deleted_at = base + chrono::Duration::hours(5);
    let deleted = insert_closed_drive(
        &db.pool,
        vehicle_id,
        deleted_at,
        deleted_at + chrono::Duration::minutes(2),
    )
    .await;
    sqlx::query("UPDATE drives SET deleted_at = $2 WHERE id = $1")
        .bind(deleted)
        .bind(base)
        .execute(&db.pool)
        .await
        .unwrap();

    let open_at = base + chrono::Duration::hours(6);
    let open: (i64,) = sqlx::query_as(
        "INSERT INTO drives (vehicle_id, start_at, source) VALUES ($1, $2, 'poll') RETURNING id",
    )
    .bind(vehicle_id)
    .bind(open_at)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    insert_fix(
        &db.pool,
        vehicle_id,
        Some(open.0),
        open_at,
        Some(15.0),
        6.0,
        Some("D"),
    )
    .await;

    let dry = crate::persist::repair_drive_ends(&db.pool, false)
        .await
        .unwrap();
    assert!(!dry.applied);
    assert_eq!(dry.closed, 5);
    assert_eq!(dry.unchanged, 3);
    assert_eq!(dry.changed, vec![trim, zeros]);
    let printed = crate::persist::format_drive_end_repair(&dry);
    assert!(!printed.contains("repair-fixture"));
    assert!(!printed.contains("vin"));
    assert!(!printed.contains("latitude"));
    assert_eq!(
        drive_end(&db.pool, trim).await,
        base + chrono::Duration::minutes(2)
    );
    assert_eq!(
        drive_end(&db.pool, zeros).await,
        zeros_at + chrono::Duration::minutes(1)
    );

    let applied = crate::persist::repair_drive_ends(&db.pool, true)
        .await
        .unwrap();
    assert!(applied.applied);
    assert_eq!(applied.changed, vec![trim, zeros]);
    assert_eq!(
        drive_end(&db.pool, trim).await,
        base + chrono::Duration::minutes(1),
        "end moves to the first exact 0 km/h sample"
    );
    assert_eq!(
        attached_at(&db.pool, trim, base + chrono::Duration::minutes(2)).await,
        0
    );
    assert_eq!(
        attached_at(&db.pool, trim, base + chrono::Duration::minutes(1)).await,
        1
    );
    assert_eq!(
        drive_end(&db.pool, zeros).await,
        zeros_at + chrono::Duration::minutes(1)
    );
    assert_eq!(
        attached_at(&db.pool, zeros, zeros_at + chrono::Duration::minutes(1)).await,
        1
    );
    assert_eq!(
        attached_at(&db.pool, zeros, zeros_at + chrono::Duration::minutes(3)).await,
        0
    );
    assert_eq!(
        drive_end(&db.pool, no_zero).await,
        no_zero_at + chrono::Duration::minutes(1)
    );
    assert_eq!(
        drive_end(&db.pool, middle).await,
        middle_at + chrono::Duration::minutes(2)
    );
    assert_eq!(
        attached_at(&db.pool, middle, middle_at + chrono::Duration::minutes(1)).await,
        1
    );
    assert_eq!(
        drive_end(&db.pool, detached).await,
        detached_at + chrono::Duration::minutes(1)
    );
    let still_null: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM positions WHERE vehicle_id = $1 AND recorded_at = $2 AND drive_id IS NULL",
    )
    .bind(vehicle_id)
    .bind(detached_at + chrono::Duration::minutes(2))
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(
        still_null.0, 1,
        "a position with no drive id stays detached"
    );
    assert_eq!(
        drive_end(&db.pool, deleted).await,
        deleted_at + chrono::Duration::minutes(2)
    );
    let open_end: (Option<chrono::DateTime<Utc>>,) =
        sqlx::query_as("SELECT end_at FROM drives WHERE id = $1")
            .bind(open.0)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(open_end.0, None);

    let again = crate::persist::repair_drive_ends(&db.pool, true)
        .await
        .unwrap();
    assert!(again.changed.is_empty(), "a second apply changes nothing");
    assert_eq!(again.unchanged, 5);
}

async fn insert_closed_drive(
    pool: &sqlx::PgPool,
    vehicle_id: i64,
    start: chrono::DateTime<Utc>,
    end: chrono::DateTime<Utc>,
) -> i64 {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO drives (vehicle_id, start_at, end_at, source)
         VALUES ($1, $2, $3, 'poll') RETURNING id",
    )
    .bind(vehicle_id)
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await
    .unwrap();
    row.0
}

async fn insert_fix(
    pool: &sqlx::PgPool,
    vehicle_id: i64,
    drive_id: Option<i64>,
    at: chrono::DateTime<Utc>,
    speed_kmh: Option<f64>,
    odometer_km: f64,
    shift_state: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO positions (
            vehicle_id, recorded_at, speed_kmh, odometer_km, shift_state, drive_id, source
         ) VALUES ($1, $2, $3, $4, $5, $6, 'poll')",
    )
    .bind(vehicle_id)
    .bind(at)
    .bind(speed_kmh)
    .bind(odometer_km)
    .bind(shift_state)
    .bind(drive_id)
    .execute(pool)
    .await
    .unwrap();
}

async fn drive_end(pool: &sqlx::PgPool, drive_id: i64) -> chrono::DateTime<Utc> {
    let row: (chrono::DateTime<Utc>,) = sqlx::query_as("SELECT end_at FROM drives WHERE id = $1")
        .bind(drive_id)
        .fetch_one(pool)
        .await
        .unwrap();
    row.0
}

async fn attached_at(pool: &sqlx::PgPool, drive_id: i64, at: chrono::DateTime<Utc>) -> i64 {
    let row: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM positions WHERE drive_id = $1 AND recorded_at = $2")
            .bind(drive_id)
            .bind(at)
            .fetch_one(pool)
            .await
            .unwrap();
    row.0
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

async fn insert_detached_position(
    pool: &sqlx::PgPool,
    vehicle_id: i64,
    at: DateTime<Utc>,
    speed_kmh: f64,
    odometer_km: Option<f64>,
    latitude: f64,
    source: &str,
) {
    sqlx::query(
        "INSERT INTO positions (vehicle_id, recorded_at, latitude, longitude, speed_kmh,
                                odometer_km, battery_level, source)
         VALUES ($1, $2, $3, 20.0, $4, $5, 80.0, $6)",
    )
    .bind(vehicle_id)
    .bind(at)
    .bind(latitude)
    .bind(speed_kmh)
    .bind(odometer_km)
    .bind(source)
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn rebuild_drive_reattaches_detached_positions_once() {
    let Some(db) = TestDb::migrated("rebuild").await else {
        return;
    };
    let archive = test_archive(db.pool.clone());
    let vin = "5YJ3E1EA7KF000093";
    let base = Utc::now() - chrono::Duration::hours(2);
    apply_for_vin(
        &archive,
        vin,
        car(vin),
        Observation::at(base - chrono::Duration::minutes(5), Presence::Asleep),
        false,
        false,
    )
    .await
    .unwrap();
    let (vehicle_id,): (i64,) = sqlx::query_as("SELECT id FROM vehicles WHERE vin = $1")
        .bind(vin)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    // A telemetry opener without odometer, then points recorded by the old
    // Owner stream (still readable), then a stop.
    insert_detached_position(&db.pool, vehicle_id, base, 8.0, None, 10.0, "telemetry").await;
    for step in 1..=30_i32 {
        let speed = if step > 27 { 0.0 } else { 40.0 };
        insert_detached_position(
            &db.pool,
            vehicle_id,
            base + chrono::Duration::seconds(i64::from(step) * 10),
            speed,
            Some(1000.0 + 0.1 * f64::from(step.min(27))),
            10.0 + 0.001 * f64::from(step.min(27)),
            "stream",
        )
        .await;
    }
    let from = base - chrono::Duration::seconds(1);
    let to = base + chrono::Duration::seconds(301);

    let dry = crate::persist::rebuild_drive(&db.pool, vehicle_id, from, to, false)
        .await
        .unwrap()
        .0;
    assert!(!dry.applied);
    assert_eq!(dry.positions, 31);
    assert!(dry.drive_id.is_none());
    let untouched: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM drives), (SELECT COUNT(drive_id) FROM positions)",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    assert_eq!(untouched, (0, 0), "a dry run writes nothing");

    let applied = crate::persist::rebuild_drive(&db.pool, vehicle_id, from, to, true)
        .await
        .unwrap()
        .0;
    assert!(applied.applied);
    let drive_id = applied.drive_id.unwrap();
    let (end_at, distance, source): (Option<DateTime<Utc>>, Option<f64>, String) =
        sqlx::query_as("SELECT end_at, distance_km, source FROM drives WHERE id = $1")
            .bind(drive_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
    assert_eq!(source, "telemetry", "the first position's source");
    assert!(end_at.is_some());
    assert!(
        distance.is_some_and(|km| (km - 2.6).abs() < 1e-6),
        "{distance:?}"
    );

    let again = crate::persist::rebuild_drive(&db.pool, vehicle_id, from, to, true).await;
    assert!(again.is_err(), "a window that overlaps a drive is refused");
    let backwards = crate::persist::rebuild_drive(&db.pool, vehicle_id, to, from, false).await;
    assert!(backwards.is_err());
}

/// A drive fed by the stream and by telemetry at once draws one track.
#[tokio::test]
async fn drive_detail_draws_the_stream_where_it_covers_telemetry() {
    let Some(db) = TestDb::migrated("onetrack").await else {
        return;
    };
    let state = test_state(db.pool.clone()).await;
    let token = "tl_onetrack-token";
    sqlx::query("INSERT INTO api_tokens (name, token_hash) VALUES ('one', $1)")
        .bind(sha256_hex(token))
        .execute(&db.pool)
        .await
        .unwrap();
    let vehicle: i64 = sqlx::query_scalar(
        "INSERT INTO vehicles (vin, name) VALUES ('TEST0000000TRACK1', 'Track') RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let drive: i64 = sqlx::query_scalar(
        "INSERT INTO drives (vehicle_id, start_at, end_at, source)
         VALUES ($1, '2024-06-01T10:00:00Z', '2024-06-01T10:03:00Z', 'stream') RETURNING id",
    )
    .bind(vehicle)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO positions (vehicle_id, recorded_at, latitude, longitude, speed_kmh, drive_id, source)
         SELECT $1, '2024-06-01T10:00:00Z'::timestamptz + make_interval(secs => s),
                0.001 * s, 0.0, 40, $2, 'stream'
         FROM generate_series(0, 120) s
         UNION ALL
         SELECT $1, '2024-06-01T10:00:00.5Z'::timestamptz + make_interval(secs => s),
                0.001 * s, 0.0005, 38, $2, 'telemetry'
         FROM generate_series(0, 180, 10) s",
    )
    .bind(vehicle)
    .bind(drive)
    .execute(&db.pool)
    .await
    .unwrap();
    let body = authed(
        &state,
        "GET",
        &format!("/api/v1/drives/{drive}?max_points=1000"),
        token,
    )
    .await;
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    let route = parsed["route"].as_array().unwrap();
    let telemetry: Vec<f64> = route
        .iter()
        .filter(|point| point["longitude"].as_f64().is_some_and(|lon| lon > 0.0))
        .filter_map(|point| point["latitude"].as_f64())
        .collect();
    assert!(!telemetry.is_empty(), "the telemetry-only tail still draws");
    assert!(
        telemetry.iter().all(|lat| *lat > 0.135),
        "no telemetry point inside the stream span: {telemetry:?}"
    );
    let latitudes: Vec<f64> = route
        .iter()
        .filter_map(|point| point["latitude"].as_f64())
        .collect();
    assert!(
        latitudes.windows(2).all(|pair| pair[0] <= pair[1]),
        "{latitudes:?}"
    );
}

#[tokio::test]
async fn recompute_drive_fixes_the_inverted_pack_power_once() {
    let Some(db) = TestDb::migrated("recomputedrive").await else {
        return;
    };
    let (vehicle_id,): (i64,) = sqlx::query_as(
        "INSERT INTO vehicles (vin, name, efficiency_kwh_per_km)
         VALUES ('5YJ3E1EA7KF000140', 'Test', 0.15) RETURNING id",
    )
    .fetch_one(&db.pool)
    .await
    .unwrap();
    let start = Utc.with_ymd_and_hms(2026, 11, 27, 14, 16, 23).unwrap();
    let end = start + chrono::Duration::minutes(9);
    let (drive_id,): (i64,) = sqlx::query_as(
        "INSERT INTO drives (vehicle_id, start_at, end_at, source)
         VALUES ($1, $2, $3, 'telemetry') RETURNING id",
    )
    .bind(vehicle_id)
    .bind(start)
    .bind(end)
    .fetch_one(&db.pool)
    .await
    .unwrap();
    // Stored by 1224370: discharge recorded as negative kW, no rated range.
    for minute in 0..=9_i32 {
        let power = if minute == 8 { 6.0 } else { -12.0 };
        sqlx::query(
            "INSERT INTO positions (vehicle_id, recorded_at, latitude, longitude, speed_kmh,
                                    power_kw, odometer_km, battery_level, drive_id, source,
                                    shift_state)
             VALUES ($1, $2, 10.0, $3, 30.0, $4, $5, 72.0, $6, 'telemetry', 'D')",
        )
        .bind(vehicle_id)
        .bind(start + chrono::Duration::minutes(i64::from(minute)))
        .bind(20.0 + f64::from(minute) * 0.001)
        .bind(power)
        .bind(1_000.0 + f64::from(minute) * 0.5)
        .bind(drive_id)
        .execute(&db.pool)
        .await
        .unwrap();
    }
    let expected = (12.0 * 8.0 - 6.0) / 60.0;
    let stored = || async {
        let row: (Option<f64>, Option<f64>, Option<f64>) = sqlx::query_as(
            "SELECT energy_used_kwh, max_power_kw,
                    (SELECT sum(power_kw) FROM positions WHERE drive_id = $1)
             FROM drives WHERE id = $1",
        )
        .bind(drive_id)
        .fetch_one(&db.pool)
        .await
        .unwrap();
        row
    };

    let dry = crate::persist::recompute_drive(&db.pool, drive_id, true, false)
        .await
        .unwrap();
    assert!(!dry.applied);
    assert!(dry.power_flipped);
    assert!(
        (dry.energy_used_kwh.unwrap() - expected).abs() < 1e-6,
        "{dry:?}"
    );
    assert_eq!(
        stored().await,
        (None, None, Some(-102.0)),
        "dry-run writes nothing"
    );

    let applied = crate::persist::recompute_drive(&db.pool, drive_id, true, true)
        .await
        .unwrap();
    assert!(applied.applied && applied.power_flipped);
    let (energy, max_power, sum) = stored().await;
    assert!((energy.unwrap() - expected).abs() < 1e-6);
    assert_eq!((max_power, sum), (Some(12.0), Some(102.0)));

    // A second run does not flip back.
    let again = crate::persist::recompute_drive(&db.pool, drive_id, true, true)
        .await
        .unwrap();
    assert!(!again.power_flipped);
    let (energy, _, sum) = stored().await;
    assert!((energy.unwrap() - expected).abs() < 1e-6);
    assert_eq!(sum, Some(102.0));

    // An open drive is refused.
    sqlx::query("UPDATE drives SET end_at = NULL WHERE id = $1")
        .bind(drive_id)
        .execute(&db.pool)
        .await
        .unwrap();
    assert!(
        crate::persist::recompute_drive(&db.pool, drive_id, false, false)
            .await
            .is_err()
    );
}
