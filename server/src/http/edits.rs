// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use axum::Router;
use axum::extract::{Form, Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::post;
use chrono::{DateTime, Utc};
use serde::Deserialize;

use super::ui::{csrf_ok, signed_page};
use crate::app::AppState;
use crate::edits::{self, ChargeEdit, DriveEdit, PurgePositions};

pub(super) fn routes() -> Router<AppState> {
    Router::new()
        .route("/v/{id}/drives/{drive_id}/edit", post(edit_drive))
        .route("/v/{id}/drives/{drive_id}/recompute", post(recompute_drive))
        .route("/v/{id}/drives/{drive_id}/split", post(split_drive))
        .route("/v/{id}/drives/{drive_id}/merge", post(merge_drives))
        .route("/v/{id}/drives/{drive_id}/trash", post(trash_drive))
        .route("/v/{id}/drives/{drive_id}/restore", post(restore_drive))
        .route("/v/{id}/drives/{drive_id}/purge", post(purge_drive))
        .route("/v/{id}/charges/{charge_id}/edit", post(edit_charge))
        .route("/v/{id}/charges/{charge_id}/trash", post(trash_charge))
        .route("/v/{id}/charges/manual", post(manual_charge))
        .route("/v/{id}/recompute", post(recompute_all))
}

#[derive(Deserialize)]
struct DriveForm {
    csrf: String,
    #[serde(default)]
    note: String,
    #[serde(default)]
    tag: String,
    #[serde(default)]
    start_address_id: String,
    #[serde(default)]
    end_address_id: String,
    #[serde(default)]
    start_geofence_id: String,
    #[serde(default)]
    end_geofence_id: String,
}

#[derive(Deserialize)]
struct ChargeForm {
    csrf: String,
    #[serde(default)]
    cost: String,
    #[serde(default)]
    is_dc: String,
    #[serde(default)]
    charger_type: String,
    #[serde(default)]
    geofence_id: String,
    #[serde(default)]
    note: String,
}

#[derive(Deserialize)]
struct CsrfOnly {
    csrf: String,
}

#[derive(Deserialize)]
struct SplitForm {
    csrf: String,
    at: String,
}

#[derive(Deserialize)]
struct MergeForm {
    csrf: String,
    other: i64,
}

#[derive(Deserialize)]
struct PurgeForm {
    csrf: String,
    confirm_id: String,
    positions: String,
}

#[derive(Deserialize)]
struct ManualForm {
    csrf: String,
    start: String,
    end: String,
    #[serde(default)]
    cost: String,
    #[serde(default)]
    note: String,
}

async fn gate(app: &AppState, headers: &HeaderMap, csrf: &str, vehicle: i64) -> Option<i64> {
    let principal = super::ui_session(app, headers).await?;
    let page = signed_page(app, headers).await?;
    if !csrf_ok(headers, csrf) || !page.vehicles.iter().any(|row| row.id == vehicle) {
        return None;
    }
    Some(principal.user_id)
}

async fn edit_drive(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((vehicle, drive)): Path<(i64, i64)>,
    Form(form): Form<DriveForm>,
) -> Response {
    let Some(actor) = gate(&app, &headers, &form.csrf, vehicle).await else {
        return Redirect::to("/").into_response();
    };
    let _ = edits::edit_drive(
        &app.inner.db,
        actor,
        vehicle,
        drive,
        &DriveEdit {
            note: blank(&form.note),
            tag: blank(&form.tag),
            start_address_id: id_field(&form.start_address_id),
            end_address_id: id_field(&form.end_address_id),
            start_geofence_id: id_field(&form.start_geofence_id),
            end_geofence_id: id_field(&form.end_geofence_id),
        },
    )
    .await;
    back(vehicle, drive, true)
}

async fn recompute_drive(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((vehicle, drive)): Path<(i64, i64)>,
    Form(form): Form<CsrfOnly>,
) -> Response {
    let Some(actor) = gate(&app, &headers, &form.csrf, vehicle).await else {
        return Redirect::to("/").into_response();
    };
    let _ = edits::recompute_drive(&app.inner.db, actor, vehicle, drive).await;
    back(vehicle, drive, true)
}

async fn split_drive(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((vehicle, drive)): Path<(i64, i64)>,
    Form(form): Form<SplitForm>,
) -> Response {
    let Some(actor) = gate(&app, &headers, &form.csrf, vehicle).await else {
        return Redirect::to("/").into_response();
    };
    let Ok(at) = DateTime::parse_from_rfc3339(&form.at) else {
        return back(vehicle, drive, true);
    };
    let _ = edits::split_drive(&app.inner.db, actor, vehicle, drive, at.with_timezone(&Utc)).await;
    back(vehicle, drive, true)
}

async fn merge_drives(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((vehicle, drive)): Path<(i64, i64)>,
    Form(form): Form<MergeForm>,
) -> Response {
    let Some(actor) = gate(&app, &headers, &form.csrf, vehicle).await else {
        return Redirect::to("/").into_response();
    };
    let _ = edits::merge_drives(&app.inner.db, actor, vehicle, drive, form.other).await;
    back(vehicle, drive, true)
}

async fn trash_drive(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((vehicle, drive)): Path<(i64, i64)>,
    Form(form): Form<CsrfOnly>,
) -> Response {
    let Some(actor) = gate(&app, &headers, &form.csrf, vehicle).await else {
        return Redirect::to("/").into_response();
    };
    let _ = edits::trash_drive(&app.inner.db, actor, vehicle, drive).await;
    Redirect::to(&format!("/v/{vehicle}/drives")).into_response()
}

async fn restore_drive(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((vehicle, drive)): Path<(i64, i64)>,
    Form(form): Form<CsrfOnly>,
) -> Response {
    let Some(actor) = gate(&app, &headers, &form.csrf, vehicle).await else {
        return Redirect::to("/").into_response();
    };
    let _ = edits::restore_drive(&app.inner.db, actor, vehicle, drive).await;
    back(vehicle, drive, true)
}

async fn purge_drive(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((vehicle, drive)): Path<(i64, i64)>,
    Form(form): Form<PurgeForm>,
) -> Response {
    let Some(page) = signed_page(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    if !csrf_ok(&headers, &form.csrf)
        || !page.admin
        || !sudo_fresh(&app, &headers).await
        || form.confirm_id != drive.to_string()
    {
        return (axum::http::StatusCode::FORBIDDEN, "sudo required").into_response();
    }
    let Some(principal) = super::ui_session(&app, &headers).await else {
        return Redirect::to("/").into_response();
    };
    let mode = if form.positions == "delete" {
        PurgePositions::DeleteRange
    } else {
        PurgePositions::Detach
    };
    let _ = edits::purge_drive(&app.inner.db, principal.user_id, vehicle, drive, mode).await;
    Redirect::to(&format!("/v/{vehicle}/drives")).into_response()
}

async fn trash_charge(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((vehicle, charge)): Path<(i64, i64)>,
    Form(form): Form<CsrfOnly>,
) -> Response {
    let Some(actor) = gate(&app, &headers, &form.csrf, vehicle).await else {
        return Redirect::to("/").into_response();
    };
    let _ = edits::trash_charge(&app.inner.db, actor, vehicle, charge).await;
    Redirect::to(&format!("/v/{vehicle}/charges")).into_response()
}

async fn edit_charge(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path((vehicle, charge)): Path<(i64, i64)>,
    Form(form): Form<ChargeForm>,
) -> Response {
    let Some(actor) = gate(&app, &headers, &form.csrf, vehicle).await else {
        return Redirect::to("/").into_response();
    };
    let is_dc = match form.is_dc.as_str() {
        "1" | "true" | "dc" => Some(true),
        "0" | "false" | "ac" => Some(false),
        _ => None,
    };
    let _ = edits::edit_charge(
        &app.inner.db,
        actor,
        vehicle,
        charge,
        &ChargeEdit {
            cost: form.cost.trim().parse().ok(),
            is_dc,
            charger_type: blank(&form.charger_type),
            geofence_id: id_field(&form.geofence_id),
            note: blank(&form.note),
        },
    )
    .await;
    back(vehicle, charge, false)
}

async fn manual_charge(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(vehicle): Path<i64>,
    Form(form): Form<ManualForm>,
) -> Response {
    let Some(actor) = gate(&app, &headers, &form.csrf, vehicle).await else {
        return Redirect::to("/").into_response();
    };
    let (Ok(start), Ok(end)) = (
        DateTime::parse_from_rfc3339(&form.start),
        DateTime::parse_from_rfc3339(&form.end),
    ) else {
        return Redirect::to(&format!("/v/{vehicle}/charges")).into_response();
    };
    let _ = edits::create_manual_charge(
        &app.inner.db,
        actor,
        vehicle,
        start.with_timezone(&Utc),
        end.with_timezone(&Utc),
        &ChargeEdit {
            cost: form.cost.trim().parse().ok(),
            is_dc: None,
            charger_type: None,
            geofence_id: None,
            note: blank(&form.note),
        },
    )
    .await;
    Redirect::to(&format!("/v/{vehicle}/charges")).into_response()
}

async fn recompute_all(
    State(app): State<AppState>,
    headers: HeaderMap,
    Path(vehicle): Path<i64>,
    Form(form): Form<CsrfOnly>,
) -> Response {
    let Some(actor) = gate(&app, &headers, &form.csrf, vehicle).await else {
        return Redirect::to("/").into_response();
    };
    let _ = edits::recompute_vehicle(&app.inner.db, actor, vehicle).await;
    Redirect::to(&format!("/v/{vehicle}/drives")).into_response()
}

async fn sudo_fresh(app: &AppState, headers: &HeaderMap) -> bool {
    super::ui_session(app, headers)
        .await
        .is_some_and(|principal| principal.sudo_active())
}

fn back(vehicle: i64, id: i64, drive: bool) -> Response {
    let kind = if drive { "drives" } else { "charges" };
    Redirect::to(&format!("/v/{vehicle}/{kind}/{id}")).into_response()
}

fn blank(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn id_field(value: &str) -> Option<i64> {
    let value = value.trim();
    if value.is_empty() || value == "0" {
        None
    } else {
        value.parse().ok()
    }
}
