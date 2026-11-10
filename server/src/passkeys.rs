// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Passkeys via webauthn-rs. The registration state is stored as JSON and expires.

use chrono::{Duration, Utc};
use sqlx::PgPool;
use url::Url;
use uuid::Uuid;
use webauthn_rs::prelude::{PasskeyRegistration, Webauthn, WebauthnBuilder};

use crate::config::Config;
use crate::error::AppError;

pub(crate) fn webauthn(config: &Config) -> Result<Webauthn, AppError> {
    let origin = Url::parse(&config.public_base_url)
        .map_err(|_| AppError::message("public URL is not valid for passkeys"))?;
    WebauthnBuilder::new(&config.domain, &origin)
        .map_err(|_| AppError::message("passkeys could not be configured"))?
        .build()
        .map_err(|_| AppError::message("passkeys could not be configured"))
}

pub(crate) async fn start_registration(
    pool: &PgPool,
    config: &Config,
    user_id: i64,
    username: &str,
) -> Result<serde_json::Value, AppError> {
    let webauthn = webauthn(config)?;
    let unique = Uuid::from_u128(u128::try_from(user_id).unwrap_or(0));
    let (challenge, state) = webauthn
        .start_passkey_registration(unique, username, username, None)
        .map_err(|_| AppError::message("passkey registration could not start"))?;
    let state_json =
        serde_json::to_value(&state).map_err(|_| AppError::message("passkey state"))?;
    sqlx::query("DELETE FROM passkey_challenges WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    sqlx::query("INSERT INTO passkey_challenges (user_id, state, expires_at) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(state_json)
        .bind(Utc::now() + Duration::minutes(5))
        .execute(pool)
        .await?;
    serde_json::to_value(&challenge).map_err(|_| AppError::message("passkey challenge"))
}

pub(crate) async fn finish_registration(
    pool: &PgPool,
    config: &Config,
    user_id: i64,
    body: &serde_json::Value,
) -> Result<(), AppError> {
    let webauthn = webauthn(config)?;
    let row: Option<serde_json::Value> = sqlx::query_scalar(
        "SELECT state FROM passkey_challenges
         WHERE user_id = $1 AND expires_at > now()
         ORDER BY id DESC LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    let Some(state_json) = row else {
        return Err(AppError::message("passkey challenge expired"));
    };
    let state: PasskeyRegistration = serde_json::from_value(state_json)
        .map_err(|_| AppError::message("passkey challenge is unreadable"))?;
    let register = serde_json::from_value(body.clone())
        .map_err(|_| AppError::message("passkey response is unreadable"))?;
    let passkey = webauthn
        .finish_passkey_registration(&register, &state)
        .map_err(|_| AppError::message("passkey was rejected"))?;
    let credential_id = passkey.cred_id().as_ref().to_vec();
    let stored = serde_json::to_value(&passkey).map_err(|_| AppError::message("passkey store"))?;
    sqlx::query("INSERT INTO passkeys (user_id, credential_id, passkey) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(credential_id)
        .bind(stored)
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM passkey_challenges WHERE user_id = $1")
        .bind(user_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::webauthn;
    use crate::config::Config;

    #[test]
    fn builder_accepts_https_on_the_configured_domain() {
        let mut map = std::collections::BTreeMap::new();
        map.insert("TESLA_CLIENT_ID".into(), "id".into());
        map.insert("TESLA_CLIENT_SECRET".into(), "secret".into());
        map.insert(
            "TESLA_FLEET_AUDIENCE".into(),
            "https://fleet-api.prd.eu.vn.cloud.tesla.com".into(),
        );
        map.insert("TESLA_DOMAIN".into(), "logger.example".into());
        map.insert(
            "TESLA_PUBLIC_BASE_URL".into(),
            "https://logger.example".into(),
        );
        map.insert("DATABASE_URL".into(), "postgres://localhost/tesla".into());
        map.insert("TESLA_PRIVATE_KEY_PATH".into(), "/keys/key.pem".into());
        map.insert("TESLA_TOKEN_STORE_PATH".into(), "/data/tokens.json".into());
        map.insert("TESLA_LOGGER_UI_PASSWORD".into(), "password-value".into());
        let config = Config::from_map(&map).unwrap();
        assert!(webauthn(&config).is_ok());
    }
}
