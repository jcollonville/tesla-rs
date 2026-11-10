// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::time::{Duration, Instant};

use axum::extract::{Form, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;
use tesla_auth::{Scope, TokenSet};
use tesla_owner_api::{OwnerClient, OwnerRegion, OwnerTokens};
use zeroize::Zeroize;

use super::ui::{self, CsrfIssued};
use super::ui_session;
use crate::app::AppState;
use crate::crypto::TokenFile;

const PENDING_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Deserialize)]
pub(super) struct RedirectForm {
    csrf: String,
    redirect_url: String,
}

#[derive(Deserialize)]
pub(super) struct RefreshForm {
    csrf: String,
    refresh_token: String,
}

#[derive(Deserialize)]
pub(super) struct CsrfForm {
    csrf: String,
}

pub(super) async fn page(State(app): State<AppState>, headers: HeaderMap) -> Response {
    if ui_session(&app, &headers).await.is_none() {
        return Redirect::to("/").into_response();
    }
    let csrf = match ui::ensure_csrf(&app, &headers) {
        Ok(value) => value,
        Err(status) => return (status, "could not start a session").into_response(),
    };
    let body = render(&app, &csrf.0, None, None).await;
    html_response(body, csrf)
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
    Redirect::to("/auth/owner").into_response()
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
            Ok(()) => Redirect::to("/").into_response(),
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
            Ok(()) => Redirect::to("/").into_response(),
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
    let store = match TokenFile::open(
        &app.inner.config.owner_token_store_path,
        app.inner.config.token_encryption_key.as_deref(),
    ) {
        Ok(store) => store,
        Err(err) => {
            tracing::warn!(error = %err, "owner tokens could not be cleared");
            return message(
                &app,
                &headers,
                "Owner API tokens could not be cleared",
                None,
            )
            .await;
        }
    };
    if let Err(err) = store.clear().await {
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
    Redirect::to("/").into_response()
}

async fn render(app: &AppState, csrf: &str, error: Option<&str>, notice: Option<&str>) -> String {
    let configured = owner_configured(app).await;
    let snapshot = app.inner.stream_board.snapshot().await;
    let pending = app.inner.owner_pending.lock().await.clone();
    let link = pending.and_then(|(login, created)| {
        (created.elapsed() <= PENDING_TTL).then(|| login.authorize_url().to_string())
    });
    owner_markup(&OwnerView {
        csrf,
        error,
        notice,
        stream_status: snapshot.summary(),
        token_stored: configured,
        streaming_enabled: app.inner.config.owner_stream_enabled,
        authorize_url: link.as_deref(),
    })
}

struct OwnerView<'a> {
    csrf: &'a str,
    error: Option<&'a str>,
    notice: Option<&'a str>,
    stream_status: &'a str,
    token_stored: bool,
    streaming_enabled: bool,
    authorize_url: Option<&'a str>,
}

fn owner_markup(view: &OwnerView<'_>) -> String {
    let error = view
        .error
        .map(|text| ui::banner("bad", text))
        .unwrap_or_default();
    let notice = view
        .notice
        .map(|text| ui::banner("note", text))
        .unwrap_or_default();
    let stream_tone = match view.stream_status {
        "connected" => "ok",
        "error" => "bad",
        _ => "neutral",
    };
    let stream = ui::chip(&format!("Stream {}", view.stream_status), stream_tone);
    let stored = if view.token_stored {
        ui::chip("Owner token stored", "ok")
    } else {
        ui::chip("No owner token", "neutral")
    };
    let config_note = if view.streaming_enabled {
        String::new()
    } else {
        ui::banner(
            "note",
            "Streaming is off because OWNER_STREAM_ENABLED is false. Tokens can still be saved.",
        )
    };
    let authorize = view.authorize_url.map_or_else(
        || "<p class=\"muted\">Prepare a sign-in to get the Tesla link.</p>".to_owned(),
        |url| {
            format!(
                "<p class=\"stack\"><a class=\"button\" href=\"{}\" rel=\"noreferrer noopener\" target=\"_blank\">{}Open the Tesla sign-in</a></p>",
                ui::escape(url),
                ui::icon(ui::ICON_KEY)
            )
        },
    );
    let csrf = view.csrf;
    ui::page(&format!(
        "<header class=\"toolbar\"><div><p class=\"kicker\"><a href=\"/\">tesla-logger</a></p>\
         <h1 class=\"title\">{}Owner API streaming</h1></div>\
         <div class=\"toolbar-actions\"><a class=\"button ghost\" href=\"/logs\">{}Logs</a>\
         <a class=\"button ghost\" href=\"/\">{}Back</a></div></header>\
         {error}{notice}\
         <p>This uses Tesla's legacy Owner API and streaming socket. They are unofficial and Tesla may shut them down. Fleet API polling stays the source for everything else. The stream adds about one position per second while driving.</p>\
         <div class=\"meta\">{stream}{stored}</div>{config_note}\
         <section class=\"card\"><h2>Sign in with Tesla</h2>\
         <ol class=\"steps\">\
         <li>Choose <strong>Prepare Tesla sign-in</strong>.</li>\
         <li>Open the Tesla link and sign in with the account that owns the car. The link opens in a new tab so this page stays here.</li>\
         <li>The browser cannot open <code>tesla://</code>. It may say the address is not understood, the site cannot be reached, or offer to open the Tesla app. Do not open the Tesla app.</li>\
         <li>Copy the full URL from the address bar. It starts with <code>tesla://auth/callback?</code>. If Chrome drops it, the Network tab in developer tools still lists the failed navigation.</li>\
         <li>Paste that URL into <strong>Redirect URL</strong> and choose <strong>Save sign-in</strong>. The field is masked so the authorization code is not displayed.</li>\
         </ol>\
         <form class=\"stack\" method=\"post\" action=\"/auth/owner/start\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
         <button type=\"submit\">{}Prepare Tesla sign-in</button></form>\
         {authorize}\
         <form class=\"stack\" method=\"post\" action=\"/auth/owner/callback\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
         <label class=\"field\">Redirect URL <input type=\"password\" name=\"redirect_url\" autocomplete=\"off\" spellcheck=\"false\" maxlength=\"8000\" required></label>\
         <button type=\"submit\">Save sign-in</button></form></section>\
         <section class=\"card\"><h2>Refresh token</h2>\
         <p>Paste a refresh token created for this logger. Do not reuse TeslaMate's refresh token. Tesla rotates refresh tokens, and using TeslaMate's token here can sign TeslaMate out.</p>\
         <form class=\"stack\" method=\"post\" action=\"/auth/owner/refresh\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
         <label class=\"field\">Owner API refresh token <input type=\"password\" name=\"refresh_token\" autocomplete=\"off\" spellcheck=\"false\" maxlength=\"8000\" required></label>\
         <button type=\"submit\">Save refresh token</button></form></section>\
         <section class=\"card\"><h2>Disconnect</h2>\
         <p class=\"muted\">Removes the stored Owner API token. Fleet API polling is unchanged.</p>\
         <form method=\"post\" action=\"/auth/owner/disconnect\"><input type=\"hidden\" name=\"csrf\" value=\"{csrf}\">\
         <button class=\"danger\" type=\"submit\">{}Disconnect Owner API</button></form></section>",
        ui::icon(ui::ICON_CAR),
        ui::icon(ui::ICON_LOGS),
        ui::icon(ui::ICON_ARROW),
        ui::icon(ui::ICON_KEY),
        ui::icon(ui::ICON_UNLINK)
    ))
}

async fn message(
    app: &AppState,
    headers: &HeaderMap,
    error: &str,
    notice: Option<&str>,
) -> Response {
    let csrf = match ui::ensure_csrf(app, headers) {
        Ok(value) => value,
        Err(status) => return (status, "could not start a session").into_response(),
    };
    let body = render(app, &csrf.0, Some(error), notice).await;
    html_response(body, csrf)
}

fn html_response(body: String, csrf: CsrfIssued) -> Response {
    let mut response = Html(body).into_response();
    if let Some(cookie) = csrf.1 {
        response.headers_mut().append(header::SET_COOKIE, cookie);
    }
    response
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

async fn owner_configured(app: &AppState) -> bool {
    let Ok(store) = TokenFile::open(
        &app.inner.config.owner_token_store_path,
        app.inner.config.token_encryption_key.as_deref(),
    ) else {
        return false;
    };
    matches!(store.load().await, Ok(Some(_)))
}

async fn store_tokens(app: &AppState, tokens: &OwnerTokens) -> Result<(), crate::error::AppError> {
    let store = TokenFile::open(
        &app.inner.config.owner_token_store_path,
        app.inner.config.token_encryption_key.as_deref(),
    )?;
    let saved = TokenSet::bearer(
        tokens.access_token().clone(),
        tokens.refresh_token().cloned(),
        tokens.expires_at(),
        vec![Scope::OpenId, Scope::Email, Scope::OfflineAccess],
    );
    store
        .save(&saved)
        .await
        .map_err(crate::error::AppError::from)?;
    app.inner
        .owner_tokens
        .send_modify(|value| *value = value.wrapping_add(1));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{OwnerView, owner_markup};

    #[test]
    fn sign_in_page_explains_the_custom_scheme_callback() {
        let html = owner_markup(&OwnerView {
            csrf: "csrf-token",
            error: Some("Owner API sign-in failed"),
            notice: None,
            stream_status: "disabled",
            token_stored: false,
            streaming_enabled: true,
            authorize_url: Some(
                "https://auth.tesla.com/oauth2/v3/authorize?client_id=ownerapi&redirect_uri=tesla%3A%2F%2Fauth%2Fcallback",
            ),
        });
        assert!(html.contains("tesla://auth/callback?"));
        assert!(html.contains("Do not open the Tesla app"));
        assert!(html.contains("Network tab"));
        assert!(html.contains("Do not reuse TeslaMate's refresh token"));
        assert!(!html.contains("void/callback"));
        assert!(html.contains("href=\"/logs\""));
        assert!(html.contains("action=\"/auth/owner/start\""));
        assert!(html.contains("action=\"/auth/owner/callback\""));
        assert!(html.contains("action=\"/auth/owner/refresh\""));
        assert!(html.contains("action=\"/auth/owner/disconnect\""));
        assert!(html.contains("name=\"redirect_url\""));
        assert!(html.contains("name=\"refresh_token\""));
        assert!(html.contains("name=\"csrf\" value=\"csrf-token\""));
        assert!(html.contains("redirect_uri=tesla%3A%2F%2Fauth%2Fcallback"));
        assert!(html.contains("&amp;redirect_uri="));
        assert!(html.contains("role=\"alert\""));
        assert!(!html.contains("OWNER_STREAM_ENABLED"));
        super::super::ui::preview("owner.html", &html);

        let off = owner_markup(&OwnerView {
            csrf: "csrf-token",
            error: None,
            notice: None,
            stream_status: "disabled",
            token_stored: false,
            streaming_enabled: false,
            authorize_url: None,
        });
        assert!(off.contains("OWNER_STREAM_ENABLED"));
        assert!(off.contains("Prepare a sign-in"));
        super::super::ui::preview("owner-off.html", &off);
    }
}
