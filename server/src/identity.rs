// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Local users, sessions, TOTP, and the audit log.
//!
//! Session ids are stored as SHA-256 hashes. TOTP secrets are AES-GCM ciphertext.
//! The audit log is insert-only from this module.

use std::time::{SystemTime, UNIX_EPOCH};

use tesla_auth::ExposeSecret;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
use axum::http::{HeaderMap, Method, header};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use hmac::{Hmac, Mac};
use sha1::Sha1;
use sqlx::PgPool;

use crate::app::AppState;
use crate::config::Config;
use crate::crypto::{self, sha256_hex};
use crate::error::AppError;

pub(crate) const SESSION_COOKIE: &str = "__Host-tl_session";
const IDLE: ChronoDuration = ChronoDuration::minutes(30);
const ABSOLUTE_SECS: i64 = 12 * 60 * 60;
pub(crate) struct Principal {
    pub raw_id: String,
    pub user_id: i64,
    pub username: String,
    pub role: String,
    pub must_change_password: bool,
    pub totp_enabled: bool,
    pub sudo_until: Option<DateTime<Utc>>,
    pub last_vehicle: Option<i64>,
}

impl Principal {
    pub(crate) fn is_admin(&self) -> bool {
        self.role == "admin"
    }

    pub(crate) fn needs_setup(&self) -> bool {
        self.must_change_password || !self.totp_enabled
    }

    pub(crate) fn sudo_active(&self) -> bool {
        self.sudo_until.is_some_and(|until| until > Utc::now())
    }
}

pub(crate) struct IssuedSession {
    pub id: String,
}

pub(crate) enum LoginResult {
    Session(IssuedSession),
    Rejected,
    Locked,
}

struct UserRow {
    id: i64,
    password_hash: String,
    totp_secret: Option<Vec<u8>>,
    totp_enabled: bool,
}

pub(crate) async fn bootstrap(pool: &PgPool, config: &Config) -> Result<(), AppError> {
    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM users")
        .fetch_one(pool)
        .await?;
    if count.0 > 0 {
        return Ok(());
    }
    let hash = hash_password(config.ui_password.expose_secret())?;
    let id: (i64,) = sqlx::query_as(
        "INSERT INTO users (username, password_hash, role, must_change_password)
         VALUES ('admin', $1, 'admin', true)
         RETURNING id",
    )
    .bind(hash)
    .fetch_one(pool)
    .await?;
    sqlx::query(
        "INSERT INTO vehicle_access (user_id, vehicle_id, role)
         SELECT $1, id, 'owner' FROM vehicles
         ON CONFLICT DO NOTHING",
    )
    .bind(id.0)
    .execute(pool)
    .await?;
    sqlx::query("UPDATE api_tokens SET user_id = $1 WHERE user_id IS NULL")
        .bind(id.0)
        .execute(pool)
        .await?;
    audit(pool, Some(id.0), "bootstrap", "ok", "{}").await;
    Ok(())
}

pub(crate) async fn login(
    app: &AppState,
    username: &str,
    password: &str,
    totp: &str,
    ip: &str,
) -> Result<LoginResult, AppError> {
    bootstrap(&app.inner.db, &app.inner.config).await?;
    let account = if username.trim().is_empty() {
        "admin"
    } else {
        username.trim()
    };
    let ip_key = format!("ip:{ip}");
    let user_key = format!("acct:{account}");
    if throttle_blocked(&app.inner.db, &ip_key).await?
        || throttle_blocked(&app.inner.db, &user_key).await?
    {
        audit(
            &app.inner.db,
            None,
            "login",
            "locked",
            &serde_json::json!({ "account": account }).to_string(),
        )
        .await;
        return Ok(LoginResult::Locked);
    }
    let row = sqlx::query_as::<_, (i64, String, Option<Vec<u8>>, bool)>(
        "SELECT id, password_hash, totp_secret, totp_enabled FROM users WHERE username = $1",
    )
    .bind(account)
    .fetch_optional(&app.inner.db)
    .await?;
    let Some((id, password_hash, totp_secret, totp_enabled)) = row else {
        throttle_fail(&app.inner.db, &ip_key).await?;
        throttle_fail(&app.inner.db, &user_key).await?;
        audit(&app.inner.db, None, "login", "failure", "{}").await;
        return Ok(LoginResult::Rejected);
    };
    let user = UserRow {
        id,
        password_hash,
        totp_secret,
        totp_enabled,
    };
    if !verify_password(&user.password_hash, password) {
        throttle_fail(&app.inner.db, &ip_key).await?;
        throttle_fail(&app.inner.db, &user_key).await?;
        audit(&app.inner.db, Some(user.id), "login", "failure", "{}").await;
        return Ok(LoginResult::Rejected);
    }
    if user.totp_enabled && !second_factor(&app.inner.db, &user, totp, &app.inner.config).await? {
        throttle_fail(&app.inner.db, &ip_key).await?;
        throttle_fail(&app.inner.db, &user_key).await?;
        audit(&app.inner.db, Some(user.id), "2fa", "failure", "{}").await;
        return Ok(LoginResult::Rejected);
    }
    throttle_clear(&app.inner.db, &ip_key).await?;
    throttle_clear(&app.inner.db, &user_key).await?;
    let issued = insert_session(&app.inner.db, user.id, ip).await?;
    audit(&app.inner.db, Some(user.id), "login", "ok", "{}").await;
    Ok(LoginResult::Session(issued))
}

pub(crate) async fn load_session(
    pool: &PgPool,
    raw_id: &str,
) -> Result<Option<Principal>, AppError> {
    if raw_id.is_empty() {
        return Ok(None);
    }
    let hash = sha256_hex(raw_id);
    let row = sqlx::query_as::<_, SessionRow>(
        "SELECT s.user_id, u.username, u.role, u.must_change_password, u.totp_enabled,
                s.sudo_until, s.last_vehicle_id, s.last_seen_at, s.expires_at, s.revoked_at
         FROM sessions s
         JOIN users u ON u.id = s.user_id
         WHERE s.id_hash = $1",
    )
    .bind(&hash)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let now = Utc::now();
    if row.revoked_at.is_some() || row.expires_at <= now || row.last_seen_at + IDLE <= now {
        sqlx::query(
            "UPDATE sessions SET revoked_at = now() WHERE id_hash = $1 AND revoked_at IS NULL",
        )
        .bind(&hash)
        .execute(pool)
        .await?;
        return Ok(None);
    }
    sqlx::query("UPDATE sessions SET last_seen_at = now() WHERE id_hash = $1")
        .bind(&hash)
        .execute(pool)
        .await?;
    Ok(Some(Principal {
        raw_id: raw_id.to_owned(),
        user_id: row.user_id,
        username: row.username,
        role: row.role,
        must_change_password: row.must_change_password,
        totp_enabled: row.totp_enabled,
        sudo_until: row.sudo_until,
        last_vehicle: row.last_vehicle_id,
    }))
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    user_id: i64,
    username: String,
    role: String,
    must_change_password: bool,
    totp_enabled: bool,
    sudo_until: Option<DateTime<Utc>>,
    last_vehicle_id: Option<i64>,
    last_seen_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

pub(crate) async fn remember_vehicle(pool: &PgPool, raw_id: &str, vehicle_id: i64) {
    let hash = sha256_hex(raw_id);
    let _ = sqlx::query(
        "UPDATE sessions SET last_vehicle_id = $2
         WHERE id_hash = $1 AND revoked_at IS NULL
           AND EXISTS (SELECT 1 FROM vehicle_access a
                       JOIN sessions s ON s.user_id = a.user_id
                       WHERE s.id_hash = $1 AND a.vehicle_id = $2)",
    )
    .bind(hash)
    .bind(vehicle_id)
    .execute(pool)
    .await;
}

pub(crate) async fn revoke_session(
    pool: &PgPool,
    actor: i64,
    id_hash: &str,
) -> Result<(), AppError> {
    sqlx::query("UPDATE sessions SET revoked_at = now() WHERE id_hash = $1 AND revoked_at IS NULL")
        .bind(id_hash)
        .execute(pool)
        .await?;
    audit(pool, Some(actor), "session.revoke", "ok", "{}").await;
    Ok(())
}

pub(crate) async fn grant_sudo(
    pool: &PgPool,
    principal: &Principal,
    password: &str,
) -> Result<bool, AppError> {
    let hash: (String,) = sqlx::query_as("SELECT password_hash FROM users WHERE id = $1")
        .bind(principal.user_id)
        .fetch_one(pool)
        .await?;
    if !verify_password(&hash.0, password) {
        audit(pool, Some(principal.user_id), "sudo", "failure", "{}").await;
        return Ok(false);
    }
    sqlx::query("UPDATE sessions SET sudo_until = now() + interval '5 minutes' WHERE id_hash = $1")
        .bind(sha256_hex(&principal.raw_id))
        .execute(pool)
        .await?;
    audit(pool, Some(principal.user_id), "sudo", "ok", "{}").await;
    Ok(true)
}

pub(crate) async fn change_password(
    pool: &PgPool,
    user_id: i64,
    password: &str,
) -> Result<(), AppError> {
    if password.len() < 12 {
        return Err(AppError::message("password must be at least 12 characters"));
    }
    let hash = hash_password(password)?;
    sqlx::query("UPDATE users SET password_hash = $2, must_change_password = false WHERE id = $1")
        .bind(user_id)
        .bind(hash)
        .execute(pool)
        .await?;
    audit(pool, Some(user_id), "password.change", "ok", "{}").await;
    Ok(())
}

pub(crate) struct TotpEnrollment {
    pub secret: String,
    pub recovery: Vec<String>,
}

pub(crate) async fn enroll_totp(app: &AppState, user_id: i64) -> Result<TotpEnrollment, AppError> {
    let secret = crypto::random_token().map_err(|_| AppError::message("entropy failed"))?;
    let secret = secret.into_bytes();
    let key = totp_key(&app.inner.config)?;
    let sealed =
        crypto::seal(&key, &secret).map_err(|_| AppError::message("could not store totp"))?;
    let mut recovery = Vec::new();
    sqlx::query("DELETE FROM recovery_codes WHERE user_id = $1 AND used_at IS NULL")
        .bind(user_id)
        .execute(&app.inner.db)
        .await?;
    for _ in 0..8 {
        let code = recovery_code()?;
        sqlx::query("INSERT INTO recovery_codes (user_id, code_hash) VALUES ($1, $2)")
            .bind(user_id)
            .bind(sha256_hex(&code))
            .execute(&app.inner.db)
            .await?;
        recovery.push(code);
    }
    let preview = crypto::seal(&key, recovery.join("\n").as_bytes())
        .map_err(|_| AppError::message("could not store recovery codes"))?;
    sqlx::query(
        "UPDATE users SET totp_secret = $2, totp_enabled = false, recovery_preview = $3 WHERE id = $1",
    )
    .bind(user_id)
    .bind(sealed)
    .bind(preview)
    .execute(&app.inner.db)
    .await?;
    audit(&app.inner.db, Some(user_id), "2fa.enroll", "ok", "{}").await;
    Ok(TotpEnrollment {
        secret: base32(&secret),
        recovery,
    })
}

pub(crate) async fn confirm_totp(
    app: &AppState,
    user_id: i64,
    code: &str,
) -> Result<bool, AppError> {
    let row: Option<(Vec<u8>,)> = sqlx::query_as("SELECT totp_secret FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&app.inner.db)
        .await?;
    let Some((sealed,)) = row else {
        return Ok(false);
    };
    let key = totp_key(&app.inner.config)?;
    let secret = crypto::open_payload(&sealed, Some(&key))
        .map_err(|_| AppError::message("totp unreadable"))?;
    if !totp_matches(&secret, code, unix_now()) {
        audit(&app.inner.db, Some(user_id), "2fa.confirm", "failure", "{}").await;
        return Ok(false);
    }
    sqlx::query("UPDATE users SET totp_enabled = true, recovery_preview = NULL WHERE id = $1")
        .bind(user_id)
        .execute(&app.inner.db)
        .await?;
    audit(&app.inner.db, Some(user_id), "2fa.confirm", "ok", "{}").await;
    Ok(true)
}

pub(crate) async fn create_user(
    pool: &PgPool,
    actor: i64,
    username: &str,
    password: &str,
    role: &str,
) -> Result<(), AppError> {
    if !matches!(role, "admin" | "user") || username.trim().is_empty() {
        return Err(AppError::message("user name and role are required"));
    }
    let hash = hash_password(password)?;
    sqlx::query(
        "INSERT INTO users (username, password_hash, role, must_change_password, totp_enabled)
         VALUES ($1, $2, $3, true, false)",
    )
    .bind(username.trim())
    .bind(hash)
    .bind(role)
    .execute(pool)
    .await?;
    audit(
        pool,
        Some(actor),
        "user.create",
        "ok",
        &serde_json::json!({ "username": username.trim(), "role": role }).to_string(),
    )
    .await;
    Ok(())
}

pub(crate) async fn set_vehicle_access(
    pool: &PgPool,
    actor: i64,
    user_id: i64,
    vehicle_id: i64,
    role: Option<&str>,
) -> Result<(), AppError> {
    match role {
        Some(role) if matches!(role, "owner" | "viewer") => {
            sqlx::query(
                "INSERT INTO vehicle_access (user_id, vehicle_id, role) VALUES ($1, $2, $3)
                 ON CONFLICT (user_id, vehicle_id) DO UPDATE SET role = EXCLUDED.role",
            )
            .bind(user_id)
            .bind(vehicle_id)
            .bind(role)
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query("DELETE FROM vehicle_access WHERE user_id = $1 AND vehicle_id = $2")
                .bind(user_id)
                .bind(vehicle_id)
                .execute(pool)
                .await?;
        }
        _ => return Err(AppError::message("vehicle role must be owner or viewer")),
    }
    audit(pool, Some(actor), "vehicle_access", "ok", "{}").await;
    Ok(())
}

pub(crate) async fn accessible_vehicles(pool: &PgPool, user_id: i64) -> Result<Vec<i64>, AppError> {
    let rows: Vec<(i64,)> = sqlx::query_as(
        "SELECT vehicle_id FROM vehicle_access WHERE user_id = $1 ORDER BY vehicle_id",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|row| row.0).collect())
}

#[derive(sqlx::FromRow)]
pub(crate) struct TokenRecord {
    pub id: i64,
    pub user_id: Option<i64>,
    pub scopes: Vec<String>,
    pub vehicle_ids: Option<Vec<i64>>,
}

pub(crate) async fn authenticate_token(
    pool: &PgPool,
    raw: &str,
) -> Result<Option<TokenRecord>, AppError> {
    let hash = sha256_hex(raw);
    let row = sqlx::query_as::<_, TokenRecord>(
        "SELECT id, user_id, scopes, vehicle_ids
         FROM api_tokens
         WHERE token_hash = $1 AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > now())",
    )
    .bind(hash)
    .fetch_optional(pool)
    .await?;
    if let Some(token) = &row {
        let _ = sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE id = $1")
            .bind(token.id)
            .execute(pool)
            .await;
    }
    Ok(row)
}

pub(crate) async fn audit(
    pool: &PgPool,
    actor: Option<i64>,
    action: &str,
    outcome: &str,
    detail: &str,
) {
    let detail = if serde_json::from_str::<serde_json::Value>(detail).is_ok() {
        detail
    } else {
        "{}"
    };
    let _ = sqlx::query(
        "INSERT INTO audit_log (actor_user_id, action, outcome, detail) VALUES ($1, $2, $3, $4::jsonb)",
    )
    .bind(actor)
    .bind(action)
    .bind(outcome)
    .bind(detail)
    .execute(pool)
    .await;
}

pub(crate) fn session_cookie(id: &str) -> String {
    format!(
        "{SESSION_COOKIE}={id}; HttpOnly; Secure; Path=/; Max-Age={ABSOLUTE_SECS}; SameSite=Strict"
    )
}

pub(crate) fn clear_session_cookie() -> &'static str {
    "__Host-tl_session=; HttpOnly; Secure; Path=/; Max-Age=0; SameSite=Strict"
}

pub(crate) fn origin_allowed(method: &Method, headers: &HeaderMap, public_base: &str) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    let site = headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok());
    // `same-site` would allow a sibling subdomain such as `other.b-612.fr`.
    if let Some(site) = site
        && !matches!(site, "same-origin" | "none")
    {
        return false;
    }
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim_end_matches('/'));
    // `Referrer-Policy: no-referrer` makes browsers send `Origin: null` on a
    // same-origin form POST. Accept that opaque origin only with `same-origin`.
    if origin == Some("null") {
        return site == Some("same-origin");
    }
    let matches = origin.is_some_and(|origin| origin_matches(origin, headers, public_base));
    if site == Some("none") {
        return matches;
    }
    origin.is_none_or(|_| matches)
}

fn origin_matches(origin: &str, headers: &HeaderMap, public_base: &str) -> bool {
    if origin == public_base.trim_end_matches('/') {
        return true;
    }
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|host| origin.ends_with(&format!("//{host}")))
}

pub(crate) fn client_ip(headers: &HeaderMap) -> String {
    headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_owned()
}

fn hash_password(password: &str) -> Result<String, AppError> {
    if password.is_empty() {
        return Err(AppError::message("password is empty"));
    }
    let salt = SaltString::generate(&mut argon2::password_hash::rand_core::OsRng);
    hasher()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| AppError::message("password hash failed"))
}

fn verify_password(hash: &str, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    hasher()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

fn hasher() -> Argon2<'static> {
    let params = Params::new(19 * 1024, 2, 1, None).expect("argon2id parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

fn totp_key(config: &Config) -> Result<[u8; 32], AppError> {
    let material = config
        .token_encryption_key
        .as_deref()
        .unwrap_or_else(|| config.client_secret.expose_secret());
    crypto::derive_key(material)
}

async fn second_factor(
    pool: &PgPool,
    user: &UserRow,
    code: &str,
    config: &Config,
) -> Result<bool, AppError> {
    let code = code.trim();
    if code.is_empty() {
        return Ok(false);
    }
    if let Some(sealed) = &user.totp_secret {
        let key = totp_key(config)?;
        if let Ok(secret) = crypto::open_payload(sealed, Some(&key))
            && totp_matches(&secret, code, unix_now())
        {
            return Ok(true);
        }
    }
    let hash = sha256_hex(code);
    let used = sqlx::query(
        "UPDATE recovery_codes SET used_at = now()
         WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL",
    )
    .bind(user.id)
    .bind(hash)
    .execute(pool)
    .await?;
    Ok(used.rows_affected() == 1)
}

fn totp_matches(secret: &[u8], code: &str, now: u64) -> bool {
    let Ok(presented) = code.trim().parse::<u32>() else {
        return false;
    };
    let step = 30;
    let current = now / step;
    (current.saturating_sub(1)..=current.saturating_add(1))
        .any(|counter| hotp(secret, counter) == presented)
}

fn hotp(secret: &[u8], counter: u64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("hmac key");
    mac.update(&counter.to_be_bytes());
    let bytes = mac.finalize().into_bytes();
    let offset = (bytes[19] & 0x0f) as usize;
    let bin = u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ]) & 0x7fff_ffff;
    bin % 1_000_000
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn recovery_code() -> Result<String, AppError> {
    let raw = crypto::random_token().map_err(|_| AppError::message("entropy failed"))?;
    Ok(raw.chars().take(10).collect())
}

fn base32(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let mut buffer = 0u32;
    let mut bits = 0;
    for byte in bytes {
        buffer = (buffer << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            let index = ((buffer >> bits) & 0x1f) as usize;
            out.push(ALPHABET[index] as char);
        }
    }
    if bits > 0 {
        let index = ((buffer << (5 - bits)) & 0x1f) as usize;
        out.push(ALPHABET[index] as char);
    }
    out
}

async fn insert_session(pool: &PgPool, user_id: i64, ip: &str) -> Result<IssuedSession, AppError> {
    let id = crypto::random_token().map_err(|_| AppError::message("entropy failed"))?;
    let expires = Utc::now() + ChronoDuration::seconds(ABSOLUTE_SECS);
    sqlx::query("INSERT INTO sessions (id_hash, user_id, expires_at, ip) VALUES ($1, $2, $3, $4)")
        .bind(sha256_hex(&id))
        .bind(user_id)
        .bind(expires)
        .bind(ip)
        .execute(pool)
        .await?;
    Ok(IssuedSession { id })
}

#[derive(sqlx::FromRow)]
struct ThrottleRow {
    next_at: Option<DateTime<Utc>>,
    locked_until: Option<DateTime<Utc>>,
}

async fn throttle_blocked(pool: &PgPool, subject: &str) -> Result<bool, AppError> {
    let row = sqlx::query_as::<_, ThrottleRow>(
        "SELECT next_at, locked_until FROM login_throttle WHERE subject = $1",
    )
    .bind(subject)
    .fetch_optional(pool)
    .await?;
    let Some(ThrottleRow {
        next_at,
        locked_until,
    }) = row
    else {
        return Ok(false);
    };
    let now = Utc::now();
    Ok(locked_until.is_some_and(|until| until > now) || next_at.is_some_and(|until| until > now))
}

async fn throttle_fail(pool: &PgPool, subject: &str) -> Result<(), AppError> {
    let failures: i32 = sqlx::query_as(
        "INSERT INTO login_throttle (subject, failures) VALUES ($1, 1)
         ON CONFLICT (subject) DO UPDATE SET failures = login_throttle.failures + 1
         RETURNING failures",
    )
    .bind(subject)
    .fetch_one(pool)
    .await
    .map(|(count,)| count)?;
    if failures >= 8 {
        sqlx::query(
            "UPDATE login_throttle SET locked_until = now() + interval '15 minutes', next_at = NULL WHERE subject = $1",
        )
        .bind(subject)
        .execute(pool)
        .await?;
        tracing::warn!(subject, failures, "login locked for 15 minutes");
    } else if failures >= 5 {
        let delay = 1_i64 << (failures - 5);
        sqlx::query("UPDATE login_throttle SET next_at = now() + make_interval(secs => $2) WHERE subject = $1")
            .bind(subject)
            .bind(delay)
            .execute(pool)
            .await?;
        tracing::warn!(subject, failures, delay_secs = delay, "login delayed");
    }
    Ok(())
}

async fn throttle_clear(pool: &PgPool, subject: &str) -> Result<(), AppError> {
    sqlx::query("DELETE FROM login_throttle WHERE subject = $1")
        .bind(subject)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{base32, hotp, origin_allowed, totp_matches};
    use axum::http::{HeaderMap, Method};

    #[test]
    fn totp_accepts_the_current_counter_only() {
        let secret = b"12345678901234567890";
        let code = hotp(secret, 1);
        let now = 30;
        assert!(totp_matches(secret, &format!("{code:06}"), now));
        assert!(!totp_matches(secret, "000000", now));
        assert_eq!(base32(b"f"), "MY");
    }

    #[test]
    fn cross_site_posts_are_rejected() {
        let mut headers = HeaderMap::new();
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(!origin_allowed(
            &Method::POST,
            &headers,
            "https://logger.example"
        ));
        headers.insert("sec-fetch-site", "same-origin".parse().unwrap());
        headers.insert("origin", "https://logger.example".parse().unwrap());
        assert!(origin_allowed(
            &Method::POST,
            &headers,
            "https://logger.example"
        ));
        assert!(origin_allowed(
            &Method::GET,
            &headers,
            "https://logger.example"
        ));
    }

    #[test]
    fn null_origin_is_allowed_only_for_a_same_origin_fetch() {
        let base = "https://tsl.b-612.fr";
        let mut headers = HeaderMap::new();
        headers.insert("origin", "null".parse().unwrap());
        headers.insert("sec-fetch-site", "same-origin".parse().unwrap());
        assert!(origin_allowed(&Method::POST, &headers, base));

        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(!origin_allowed(&Method::POST, &headers, base));

        headers.remove("sec-fetch-site");
        assert!(!origin_allowed(&Method::POST, &headers, base));

        headers.insert("sec-fetch-site", "none".parse().unwrap());
        assert!(!origin_allowed(&Method::POST, &headers, base));

        headers.insert("origin", "https://evil.example".parse().unwrap());
        headers.insert("sec-fetch-site", "same-origin".parse().unwrap());
        assert!(!origin_allowed(&Method::POST, &headers, base));

        headers.insert("origin", "https://tsl.b-612.fr".parse().unwrap());
        headers.insert("sec-fetch-site", "same-site".parse().unwrap());
        assert!(!origin_allowed(&Method::POST, &headers, base));

        headers.insert("sec-fetch-site", "none".parse().unwrap());
        assert!(origin_allowed(&Method::POST, &headers, base));
    }

    #[tokio::test]
    async fn bootstrap_session_is_hashed_and_legacy_tokens_keep_read_scope() {
        let Some(db) = crate::test_util::TestDb::migrated("identity").await else {
            return;
        };
        let state = crate::app::test_state(db.pool.clone()).await;
        let token = "tl_existing";
        sqlx::query("INSERT INTO api_tokens (name, token_hash) VALUES ('ha', $1)")
            .bind(crate::crypto::sha256_hex(token))
            .execute(&db.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO vehicles (vin, name) VALUES ('5YJ30000000000001', 'Red')")
            .execute(&db.pool)
            .await
            .unwrap();
        let login = super::login(&state, "admin", "test-password", "", "127.0.0.1")
            .await
            .unwrap();
        let super::LoginResult::Session(issued) = login else {
            panic!("expected a session");
        };
        let stored: (String,) = sqlx::query_as("SELECT id_hash FROM sessions")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_ne!(stored.0, issued.id);
        assert_eq!(stored.0, crate::crypto::sha256_hex(&issued.id));
        let scopes: (Vec<String>,) =
            sqlx::query_as("SELECT scopes FROM api_tokens WHERE name = 'ha'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert!(scopes.0.iter().any(|scope| scope == "read"));
        assert!(scopes.0.iter().any(|scope| scope == "read:live"));
        let access: (i64,) = sqlx::query_as("SELECT count(*) FROM vehicle_access")
            .fetch_one(&db.pool)
            .await
            .unwrap();
        assert_eq!(access.0, 1);
        let loaded = super::load_session(&db.pool, &issued.id)
            .await
            .unwrap()
            .unwrap();
        assert!(loaded.needs_setup());
        assert!(loaded.is_admin());
    }
}
