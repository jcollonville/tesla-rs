// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::time::{Duration, Instant};

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use tesla_auth::{Scope, TokenSet};
use tesla_owner_api::{OwnerClient, OwnerRegion, OwnerTokens};
use zeroize::Zeroize;

use super::ui;
use super::ui_session;
use crate::app::AppState;
use tesla_auth::TokenStore;

pub(super) const PENDING_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Deserialize)]
pub(super) struct RedirectForm {
    csrf: String,
    redirect_url: String,
    #[serde(default)]
    vehicle_id: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct RefreshForm {
    csrf: String,
    refresh_token: String,
    #[serde(default)]
    vehicle_id: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct CsrfForm {
    csrf: String,
    #[serde(default)]
    vehicle_id: Option<i64>,
}

pub(super) async fn start(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    if !session_ok(&app, &headers, &form.csrf).await {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    let client = match owner_client(&app) {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(error = %err, "owner sign-in could not start");
            return message(&app, &headers, "Owner API sign-in could not start", None).await;
        }
    };
    let login = match client.begin_login() {
        Ok(login) => login,
        Err(err) => {
            tracing::warn!(error = %err, "owner sign-in could not start");
            return message(&app, &headers, "Owner API sign-in could not start", None).await;
        }
    };
    *app.inner.owner_pending.lock().await = Some((login, Instant::now()));
    Redirect::to(&ui::connection_location(form.vehicle_id)).into_response()
}

pub(super) async fn callback(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(mut form): Form<RedirectForm>,
) -> Response {
    if !session_ok(&app, &headers, &form.csrf).await {
        form.redirect_url.zeroize();
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    let pending = app.inner.owner_pending.lock().await.take();
    let Some((login, created)) = pending else {
        form.redirect_url.zeroize();
        return message(
            &app,
            &headers,
            "Prepare the Tesla sign-in again, then paste the redirect URL.",
            None,
        )
        .await;
    };
    if created.elapsed() > PENDING_TTL {
        form.redirect_url.zeroize();
        return message(
            &app,
            &headers,
            "That sign-in expired. Prepare it again.",
            None,
        )
        .await;
    }
    let client = match owner_client(&app) {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(error = %err, "owner sign-in failed");
            form.redirect_url.zeroize();
            return message(&app, &headers, "Owner API sign-in failed", None).await;
        }
    };
    let exchanged = client.exchange(&login, &form.redirect_url).await;
    form.redirect_url.zeroize();
    match exchanged {
        Ok(tokens) => match store_tokens(&app, &tokens).await {
            Ok(()) => Redirect::to(&ui::connection_location(form.vehicle_id)).into_response(),
            Err(err) => {
                tracing::warn!(error = %err, "owner tokens could not be stored");
                message(&app, &headers, "Owner API tokens could not be stored", None).await
            }
        },
        Err(err) => {
            tracing::warn!(error = %err, "owner sign-in failed");
            message(
                &app,
                &headers,
                "Owner API sign-in failed. Prepare it again and paste the full redirect URL.",
                None,
            )
            .await
        }
    }
}

pub(super) async fn refresh(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(mut form): Form<RefreshForm>,
) -> Response {
    if !session_ok(&app, &headers, &form.csrf).await {
        form.refresh_token.zeroize();
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    let mut pasted = std::mem::take(&mut form.refresh_token);
    let trimmed = pasted.trim();
    if trimmed.contains("://") || trimmed.len() < 20 || trimmed.len() > 8_000 {
        pasted.zeroize();
        return message(
            &app,
            &headers,
            "Paste an Owner API refresh token. A tesla:// callback belongs in Redirect URL.",
            None,
        )
        .await;
    }
    let secret = tesla_owner_api::SecretString::from(trimmed.to_owned());
    pasted.zeroize();
    let client = match owner_client(&app) {
        Ok(client) => client,
        Err(err) => {
            tracing::warn!(error = %err, "owner refresh failed");
            return message(&app, &headers, "Owner API sign-in failed", None).await;
        }
    };
    match client.refresh(&secret).await {
        Ok(tokens) => match store_tokens(&app, &tokens).await {
            Ok(()) => Redirect::to(&ui::connection_location(form.vehicle_id)).into_response(),
            Err(err) => {
                tracing::warn!(error = %err, "owner tokens could not be stored");
                message(&app, &headers, "Owner API tokens could not be stored", None).await
            }
        },
        Err(err) => {
            tracing::warn!(error = %err, "owner refresh failed");
            message(
                &app,
                &headers,
                "That refresh token was rejected. Use the Tesla sign-in instead.",
                None,
            )
            .await
        }
    }
}

pub(super) async fn disconnect(
    State(app): State<AppState>,
    headers: HeaderMap,
    Form(form): Form<CsrfForm>,
) -> Response {
    if !session_ok(&app, &headers, &form.csrf).await {
        return (StatusCode::BAD_REQUEST, "csrf mismatch").into_response();
    }
    if let Err(err) = app.inner.owner_store.clear().await {
        tracing::warn!(error = %err, "owner tokens could not be cleared");
        return message(
            &app,
            &headers,
            "Owner API tokens could not be cleared",
            None,
        )
        .await;
    }
    app.inner
        .owner_tokens
        .send_modify(|value| *value = value.wrapping_add(1));
    Redirect::to(&ui::connection_location(form.vehicle_id)).into_response()
}

async fn message(
    app: &AppState,
    headers: &HeaderMap,
    error: &str,
    notice: Option<&str>,
) -> Response {
    let _ = notice;
    ui::render_connection(app, headers, None, Some(error), notice).await
}

async fn session_ok(app: &AppState, headers: &HeaderMap, csrf: &str) -> bool {
    ui_session(app, headers).await.is_some() && ui::csrf_ok(headers, csrf)
}

fn owner_client(app: &AppState) -> Result<OwnerClient, tesla_owner_api::OwnerError> {
    let region = match app.inner.config.region {
        tesla_auth::Region::China => OwnerRegion::China,
        _ => OwnerRegion::Global,
    };
    OwnerClient::new(region)
}

async fn store_tokens(app: &AppState, tokens: &OwnerTokens) -> Result<(), crate::error::AppError> {
    let saved = TokenSet::bearer(
        tokens.access_token().clone(),
        tokens.refresh_token().cloned(),
        tokens.expires_at(),
        vec![Scope::OpenId, Scope::Email, Scope::OfflineAccess],
    );
    app.inner
        .owner_store
        .save(&saved)
        .await
        .map_err(crate::error::AppError::from)?;
    app.inner
        .owner_tokens
        .send_modify(|value| *value = value.wrapping_add(1));
    Ok(())
}
