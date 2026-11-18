// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Data behind the overview car card and the fleet cards.

use chrono::{DateTime, Datelike, TimeZone, Utc};
use chrono_tz::Tz;

use super::car::Look;
use super::format::{self, NNBSP, Place};
use super::markup::{CarCard, FleetCard, Recent, Status};
use super::viz::{MiniMap, mini_map, mini_map_empty};
use crate::app::AppState;
use crate::geofence::{Fence, haversine_m, matching_geofence};
use crate::rdp::rdp_limit;

#[derive(sqlx::FromRow, Default)]
struct Live {
    model: Option<String>,
    exterior_color: Option<String>,
    marketing_name: Option<String>,
    trim_badging: Option<String>,
    activity: Option<String>,
    battery_level: Option<f64>,
    rated_range_km: Option<f64>,
    est_range_km: Option<f64>,
    odometer_km: Option<f64>,
    latitude: Option<f64>,
    longitude: Option<f64>,
    heading_deg: Option<f64>,
    speed_kmh: Option<f64>,
    inside_temp_c: Option<f64>,
    outside_temp_c: Option<f64>,
    locked: Option<bool>,
    sentry_mode: Option<bool>,
    charging_state: Option<String>,
    charger_power_kw: Option<f64>,
    charge_energy_added_kwh: Option<f64>,
    charge_limit_soc: Option<f64>,
    time_to_full_charge_h: Option<f64>,
    plugged_in: Option<bool>,
    is_dc: Option<bool>,
    software_version: Option<String>,
    last_poll_at: Option<DateTime<Utc>>,
    last_data_at: Option<DateTime<Utc>>,
}

async fn live(app: &AppState, id: i64) -> Live {
    sqlx::query_as::<_, Live>(
        "SELECT v.model, v.exterior_color, v.marketing_name, v.trim_badging,
                s.activity, s.battery_level, s.rated_range_km, s.est_range_km, s.odometer_km,
                s.latitude, s.longitude, s.heading_deg, s.speed_kmh,
                s.inside_temp_c, s.outside_temp_c, s.locked, s.sentry_mode,
                s.charging_state, s.charger_power_kw, s.charge_energy_added_kwh,
                s.charge_limit_soc, s.time_to_full_charge_h, s.plugged_in, s.is_dc,
                s.software_version, s.last_poll_at, s.last_data_at
         FROM vehicles v
         LEFT JOIN vehicle_state s ON s.vehicle_id = v.id
         WHERE v.id = $1",
    )
    .bind(id)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

impl Live {
    fn look(&self) -> Look {
        Look::from(self.model.as_deref(), self.exterior_color.as_deref())
    }

    fn status(&self) -> Status {
        Status::from(
            self.activity.as_deref(),
            self.charging_state.as_deref(),
            self.plugged_in,
        )
    }

    fn range(&self) -> String {
        format::unit(self.rated_range_km.or(self.est_range_km), 0, "km")
    }
}

/// Last known position: the live state, then the newest archived point.
async fn position(app: &AppState, id: i64, state: &Live) -> Option<(f64, f64)> {
    if let (Some(lat), Some(lon)) = (state.latitude, state.longitude) {
        return Some((lat, lon));
    }
    sqlx::query_as::<_, (f64, f64)>(
        "SELECT latitude, longitude FROM positions
         WHERE vehicle_id = $1 AND recorded_at >= now() - interval '30 days'
           AND latitude IS NOT NULL AND longitude IS NOT NULL
         ORDER BY recorded_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten()
}

/// Geofence name, else the nearest cached address within 250 m.
async fn place_at(app: &AppState, lat: f64, lon: f64) -> Option<Place> {
    let fences: Vec<(i64, String, f64, f64, f64)> =
        sqlx::query_as("SELECT id, name, latitude, longitude, radius_m FROM geofences")
            .fetch_all(&app.inner.db)
            .await
            .unwrap_or_default();
    let shapes: Vec<Fence> = fences
        .iter()
        .map(|(id, _, latitude, longitude, radius_m)| Fence {
            id: *id,
            latitude: *latitude,
            longitude: *longitude,
            radius_m: *radius_m,
        })
        .collect();
    let fence = matching_geofence(lat, lon, &shapes).and_then(|id| {
        fences
            .iter()
            .find(|row| row.0 == id)
            .map(|row| row.1.clone())
    });
    let span_lat = 0.003;
    let span_lon = 0.003 / lat.to_radians().cos().max(0.05);
    let address: Option<(String, f64, f64)> = sqlx::query_as(
        "SELECT display_name, latitude, longitude FROM addresses
         WHERE display_name IS NOT NULL
           AND latitude BETWEEN $1 - $3 AND $1 + $3
           AND longitude BETWEEN $2 - $4 AND $2 + $4
         ORDER BY power(latitude - $1, 2) + power((longitude - $2) * cos(radians($1)), 2)
         LIMIT 1",
    )
    .bind(lat)
    .bind(lon)
    .bind(span_lat)
    .bind(span_lon)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten();
    let address = address
        .filter(|(_, a_lat, a_lon)| haversine_m(lat, lon, *a_lat, *a_lon) <= 250.0)
        .map(|(name, _, _)| name);
    Place::from(address.as_deref(), fence.as_deref())
}

/// Recent drives that pass near the car, as separate polylines.
async fn nearby_tracks(app: &AppState, id: i64, lat: f64, lon: f64) -> Vec<Vec<(f64, f64)>> {
    let span_lat = 0.03;
    let span_lon = 0.045 / lat.to_radians().cos().max(0.05);
    let rows: Vec<(i64, f64, f64)> = sqlx::query_as(
        "SELECT drive_id, latitude, longitude FROM positions
         WHERE drive_id IN (
                SELECT id FROM drives
                WHERE vehicle_id = $1 AND deleted_at IS NULL
                ORDER BY start_at DESC LIMIT 30
           )
           AND latitude BETWEEN $2 - $4 AND $2 + $4
           AND longitude BETWEEN $3 - $5 AND $3 + $5
         ORDER BY drive_id, recorded_at
         LIMIT 60000",
    )
    .bind(id)
    .bind(lat)
    .bind(lon)
    .bind(span_lat)
    .bind(span_lon)
    .fetch_all(&app.inner.db)
    .await
    .unwrap_or_default();
    let mut tracks: Vec<Vec<(f64, f64)>> = Vec::new();
    let mut last: Option<(i64, f64, f64)> = None;
    for (drive, p_lat, p_lon) in rows {
        let jump = last.is_none_or(|(prev, prev_lat, prev_lon)| {
            prev != drive || haversine_m(prev_lat, prev_lon, p_lat, p_lon) > 400.0
        });
        if jump {
            tracks.push(Vec::new());
        }
        if let Some(track) = tracks.last_mut() {
            track.push((p_lat, p_lon));
        }
        last = Some((drive, p_lat, p_lon));
    }
    tracks
        .into_iter()
        .filter(|track| track.len() >= 2)
        .map(|track| {
            let flat: Vec<[f64; 2]> = track.iter().map(|(a, b)| [*a, *b]).collect();
            rdp_limit(&flat, 90)
                .into_iter()
                .filter_map(|index| track.get(index).copied())
                .collect()
        })
        .collect()
}

fn charge_note(state: &Live, status: Status) -> Option<String> {
    let limit = state.charge_limit_soc;
    match status {
        Status::Charging => {
            let mut parts = vec!["En charge".to_owned()];
            if let Some(kw) = state.charger_power_kw.filter(|kw| *kw > 0.0) {
                parts.push(format::unit(Some(kw), 1, "kW"));
            }
            if let Some(added) = state.charge_energy_added_kwh.filter(|kwh| *kwh > 0.0) {
                parts.push(format!("+{}", format::kwh(Some(added))));
            }
            if let (Some(hours), Some(limit)) = (state.time_to_full_charge_h, limit)
                && hours > 0.0
            {
                let seconds = format::hours_to_seconds(hours);
                parts.push(format!(
                    "{} dans {}",
                    format::percent(Some(limit)),
                    format::duration(Some(seconds))
                ));
            }
            if state.is_dc == Some(true) {
                parts.push("Superchargeur".to_owned());
            }
            Some(parts.join(" · "))
        }
        Status::Driving => state
            .speed_kmh
            .map(|speed| format!("Roule à {}", format::unit(Some(speed), 0, "km/h"))),
        _ => limit.map(|limit| format!("Limite de charge {}", format::percent(Some(limit)))),
    }
}

fn temperature(value: Option<f64>) -> Option<String> {
    value.map(|celsius| format!("{}{NNBSP}°C", format::number(celsius, 1)))
}

pub(super) async fn car_card(app: &AppState, id: i64) -> CarCard {
    let state = live(app, id).await;
    let status = state.status();
    let now = Utc::now();
    let position = position(app, id, &state).await;
    let place = match position {
        Some((lat, lon)) => place_at(app, lat, lon).await,
        None => None,
    };
    let map = match position {
        Some((lat, lon)) => {
            let tracks = nearby_tracks(app, id, lat, lon).await;
            let label = Place::name_or_unknown(place.as_ref());
            mini_map(&MiniMap {
                lat,
                lon,
                heading: state.heading_deg,
                moving: status == Status::Driving,
                tracks: &tracks,
                label: &label,
            })
        }
        None => mini_map_empty(),
    };
    let mut facts = Vec::new();
    if let Some(locked) = state.locked {
        facts.push((
            "Portes",
            if locked {
                "Verrouillées"
            } else {
                "Déverrouillées"
            }
            .to_owned(),
        ));
    }
    if let Some(inside) = temperature(state.inside_temp_c) {
        facts.push(("Intérieur", inside));
    }
    if let Some(outside) = temperature(state.outside_temp_c) {
        facts.push(("Extérieur", outside));
    }
    if state.odometer_km.is_some() {
        facts.push(("Compteur", format::unit(state.odometer_km, 0, "km")));
    }
    if state.sentry_mode == Some(true) {
        facts.push(("Sentinelle", "Active".to_owned()));
    }
    if let Some(version) = state
        .software_version
        .as_ref()
        .filter(|text| !text.is_empty())
    {
        facts.push(("Logiciel", version.clone()));
    }
    let seen = state.last_data_at.or(state.last_poll_at);
    let trim = state
        .marketing_name
        .clone()
        .or_else(|| state.trim_badging.clone())
        .filter(|text| !text.is_empty());
    CarCard {
        look: state.look(),
        trim,
        status,
        battery_pct: state.battery_level,
        limit_pct: state.charge_limit_soc,
        range: state.range(),
        note: charge_note(&state, status),
        facts,
        map,
        place,
        updated: format::ago(seen, now),
        updated_iso: seen.map(|at| at.to_rfc3339()),
    }
}

/// Fills the drawing and gauge fields of a fleet card.
pub(super) async fn decorate_fleet_card(app: &AppState, card: &mut FleetCard) {
    let state = live(app, card.id).await;
    card.look = state.look();
    card.status = state.status();
    card.battery_pct = state.battery_level;
    card.limit_pct = state.charge_limit_soc;
    card.range = state.range();
    if let Some((lat, lon)) = position(app, card.id, &state).await {
        card.place = place_at(app, lat, lon).await.map(|place| place.name);
    }
}

#[derive(sqlx::FromRow)]
struct LastDrive {
    id: i64,
    start_at: DateTime<Utc>,
    end_at: Option<DateTime<Utc>>,
    distance_km: Option<f64>,
    duration_s: Option<i32>,
    energy_kwh: Option<f64>,
    start_address: Option<String>,
    end_address: Option<String>,
    start_geofence: Option<String>,
    end_geofence: Option<String>,
}

fn when(start: DateTime<Utc>, tz: Tz) -> String {
    let today = format::local_date(Utc::now(), tz);
    format!(
        "{} · {}",
        format::day_label(format::local_date(start, tz), today),
        format::clock(start, tz)
    )
}

pub(super) async fn recent_drive(app: &AppState, id: i64, tz: Tz) -> Option<Recent> {
    let energy = super::stats::drive_energy_sql("d", "v");
    let row: LastDrive = sqlx::query_as(&format!(
        "SELECT d.id, d.start_at, d.end_at, d.distance_km, d.duration_s, {energy} AS energy_kwh,
                sa.display_name AS start_address, ea.display_name AS end_address,
                sf.name AS start_geofence, ef.name AS end_geofence
         FROM drives d
         JOIN vehicles v ON v.id = d.vehicle_id
         LEFT JOIN addresses sa ON sa.id = d.start_address_id
         LEFT JOIN addresses ea ON ea.id = d.end_address_id
         LEFT JOIN geofences sf ON sf.id = d.start_geofence_id
         LEFT JOIN geofences ef ON ef.id = d.end_geofence_id
         WHERE d.vehicle_id = $1 AND d.deleted_at IS NULL
         ORDER BY d.start_at DESC LIMIT 1"
    ))
    .bind(id)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten()?;
    let from = Place::from(row.start_address.as_deref(), row.start_geofence.as_deref());
    let to = Place::from(row.end_address.as_deref(), row.end_geofence.as_deref());
    let mut secondary = vec![format::duration_i32(row.duration_s)];
    if let Some(wh) = format::efficiency(row.energy_kwh, row.distance_km) {
        secondary.push(format::unit(Some(wh), 0, "Wh/km"));
    }
    Some(Recent {
        href: format!("/v/{id}/drives/{}", row.id),
        when: when(row.start_at, tz),
        from: Some(Place::name_or_unknown(from.as_ref())),
        to: if row.end_at.is_none() {
            "En route".to_owned()
        } else {
            Place::name_or_unknown(to.as_ref())
        },
        primary: format::km(row.distance_km),
        secondary: secondary.join(" · "),
        open: row.end_at.is_none(),
    })
}

#[derive(sqlx::FromRow)]
struct LastCharge {
    id: i64,
    start_at: DateTime<Utc>,
    end_at: Option<DateTime<Utc>>,
    energy_added_kwh: Option<f64>,
    duration_s: Option<i32>,
    start_battery_level: Option<f64>,
    end_battery_level: Option<f64>,
    is_dc: Option<bool>,
    address: Option<String>,
    geofence: Option<String>,
}

pub(super) async fn recent_charge(app: &AppState, id: i64, tz: Tz) -> Option<Recent> {
    let row: LastCharge = sqlx::query_as(
        "SELECT c.id, c.start_at, c.end_at, c.energy_added_kwh, c.duration_s,
                c.start_battery_level, c.end_battery_level, c.is_dc,
                a.display_name AS address, g.name AS geofence
         FROM charges c
         LEFT JOIN addresses a ON a.id = c.address_id
         LEFT JOIN geofences g ON g.id = c.geofence_id
         WHERE c.vehicle_id = $1 AND c.deleted_at IS NULL
         ORDER BY c.start_at DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten()?;
    let place = Place::from(row.address.as_deref(), row.geofence.as_deref());
    let mut secondary = Vec::new();
    if let (Some(start), Some(end)) = (row.start_battery_level, row.end_battery_level) {
        secondary.push(format!(
            "{} → {}",
            format::number(start, 0),
            format::percent(Some(end))
        ));
    }
    let duration = row.duration_s.map(i64::from).or_else(|| {
        row.end_at
            .is_none()
            .then(|| (Utc::now() - row.start_at).num_seconds())
    });
    secondary.push(format::duration(duration));
    if row.is_dc == Some(true) {
        secondary.push("DC".to_owned());
    }
    Some(Recent {
        href: format!("/v/{id}/charges/{}", row.id),
        when: when(row.start_at, tz),
        from: None,
        to: Place::name_or_unknown(place.as_ref()),
        primary: row.energy_added_kwh.map_or_else(
            || {
                if row.end_at.is_none() {
                    "En cours".to_owned()
                } else {
                    "—".to_owned()
                }
            },
            |kwh| format!("+{}", format::kwh(Some(kwh))),
        ),
        secondary: secondary.join(" · "),
        open: row.end_at.is_none(),
    })
}

/// Distance, drive count, efficiency and charged energy since the first of the month.
pub(super) async fn month(app: &AppState, id: i64, tz: Tz) -> Vec<(&'static str, String)> {
    let local = Utc::now().with_timezone(&tz);
    let start = tz
        .with_ymd_and_hms(local.year(), local.month(), 1, 0, 0, 0)
        .earliest()
        .map_or_else(Utc::now, |start| start.with_timezone(&Utc));
    let energy = super::stats::drive_energy_sql("d", "v");
    let drives: Option<(i64, Option<f64>, Option<f64>)> = sqlx::query_as(&format!(
        "SELECT COUNT(*), SUM(d.distance_km), SUM({energy})
         FROM drives d JOIN vehicles v ON v.id = d.vehicle_id
         WHERE d.vehicle_id = $1 AND d.deleted_at IS NULL AND d.start_at >= $2"
    ))
    .bind(id)
    .bind(start)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten();
    let charged: Option<(Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT SUM(energy_added_kwh), SUM(cost) FROM charges
         WHERE vehicle_id = $1 AND deleted_at IS NULL AND start_at >= $2",
    )
    .bind(id)
    .bind(start)
    .fetch_optional(&app.inner.db)
    .await
    .ok()
    .flatten();
    let (count, distance, used) = drives.unwrap_or((0, None, None));
    let (added, cost) = charged.unwrap_or((None, None));
    let mut out = vec![
        ("Distance", format::km(Some(distance.unwrap_or(0.0)))),
        ("Trajets", count.to_string()),
        (
            "Efficacité",
            format::unit(format::efficiency(used, distance), 0, "Wh/km"),
        ),
        ("Énergie chargée", format::kwh(Some(added.unwrap_or(0.0)))),
    ];
    if cost.is_some() {
        out.push(("Coût des charges", format::money(cost)));
    }
    out
}
