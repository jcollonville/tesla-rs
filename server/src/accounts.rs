// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Tesla accounts and the database token store.
//!
//! Phase 1 keeps a single Fleet row and a single Owner row. Callers still pass
//! an account id. Token files are imported once and then left untouched.

use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use tesla_auth::{AuthError, TokenSet, TokenStore};
use tokio::sync::Mutex;
use zeroize::Zeroize;

use crate::config::Config;
use crate::crypto::{self, TokenSaved};
use crate::error::{AppError, redact_secrets};
use crate::logs::redact_line;

const IMPORT_LOCK: i64 = 842_175;

pub(crate) struct ImportedAccounts {
    pub fleet_id: i64,
    pub owner_id: i64,
}

#[derive(Clone, sqlx::FromRow)]
pub(crate) struct TeslaAccount {
    pub id: i64,
    pub label: String,
    pub kind: String,
    pub status: String,
    pub last_refresh_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Clone, sqlx::FromRow)]
pub(crate) struct SharedVehicle {
    pub id: i64,
    pub name: String,
}

/// What the UI may show about a token set. Access and refresh tokens are dropped
/// before this value is returned.
#[derive(Clone, Debug)]
pub(crate) struct TokenPublic {
    pub present: bool,
    pub readable: bool,
    pub expires_at: Option<DateTime<Utc>>,
    pub scopes: Vec<String>,
}

impl TokenPublic {
    pub(crate) fn absent() -> Self {
        Self {
            present: false,
            readable: true,
            expires_at: None,
            scopes: Vec::new(),
        }
    }
}

struct StoreInner {
    pool: PgPool,
    account_id: i64,
    key: Option<[u8; 32]>,
    lock: Mutex<()>,
    saved: std::sync::Mutex<Option<TokenSaved>>,
}

impl Drop for StoreInner {
    fn drop(&mut self) {
        if let Some(key) = self.key.as_mut() {
            key.zeroize();
        }
    }
}

/// Database-backed token store for one [`tesla_accounts`](TeslaAccount) row.
#[derive(Clone)]
pub(crate) struct DbTokenStore {
    inner: Arc<StoreInner>,
}

impl DbTokenStore {
    pub(crate) fn open(
        pool: PgPool,
        account_id: i64,
        encryption_key: Option<&str>,
    ) -> Result<Self, AppError> {
        let key = match encryption_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value) => Some(crypto::derive_key(value)?),
            None => None,
        };
        Ok(Self {
            inner: Arc::new(StoreInner {
                pool,
                account_id,
                key,
                lock: Mutex::new(()),
                saved: std::sync::Mutex::new(None),
            }),
        })
    }

    pub(crate) fn on_saved(&self, hook: TokenSaved) {
        *self
            .inner
            .saved
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hook);
    }

    pub(crate) async fn public_view(&self) -> TokenPublic {
        if self.inner.account_id <= 0 {
            return TokenPublic::absent();
        }
        match self.load().await {
            Ok(Some(tokens)) => {
                let expires_at = system_time_to_utc(tokens.expires_at());
                let scopes = tokens
                    .scopes()
                    .iter()
                    .map(|scope| scope.as_str().to_owned())
                    .collect();
                drop(tokens);
                TokenPublic {
                    present: true,
                    readable: true,
                    expires_at,
                    scopes,
                }
            }
            Ok(None) => TokenPublic::absent(),
            Err(err) => {
                tracing::warn!(
                    account_id = self.inner.account_id,
                    error = %redact_line(&err.to_string(), &[]),
                    "token metadata could not be read"
                );
                TokenPublic {
                    present: false,
                    readable: false,
                    expires_at: None,
                    scopes: Vec::new(),
                }
            }
        }
    }
}

impl TokenStore for DbTokenStore {
    async fn load(&self) -> Result<Option<TokenSet>, AuthError> {
        if self.inner.account_id <= 0 {
            return Ok(None);
        }
        let _guard = self.inner.lock.lock().await;
        let row: Option<(Option<Vec<u8>>,)> =
            sqlx::query_as("SELECT tokens FROM tesla_accounts WHERE id = $1")
                .bind(self.inner.account_id)
                .fetch_optional(&self.inner.pool)
                .await
                .map_err(|err| store_err(&err))?;
        let Some((Some(bytes),)) = row else {
            return Ok(None);
        };
        let mut plaintext =
            crypto::open_payload(&bytes, self.inner.key.as_ref()).map_err(|err| {
                AuthError::TokenStore {
                    message: redact_line(&err, &[]),
                }
            })?;
        let parsed = crypto::tokens_from_json(&plaintext);
        plaintext.zeroize();
        parsed.map(Some)
    }

    async fn save(&self, tokens: &TokenSet) -> Result<(), AuthError> {
        if self.inner.account_id <= 0 {
            return Err(AuthError::TokenStore {
                message: "no tesla account is configured".to_owned(),
            });
        }
        let _guard = self.inner.lock.lock().await;
        let mut plaintext = crypto::tokens_to_json(tokens).map_err(store_msg)?;
        let sealed = match self.inner.key.as_ref() {
            Some(key) => crypto::seal(key, &plaintext).map_err(store_msg)?,
            None => plaintext.clone(),
        };
        plaintext.zeroize();
        sqlx::query(
            "UPDATE tesla_accounts
             SET tokens = $2, status = 'connected', last_refresh_at = now(), last_error = NULL
             WHERE id = $1",
        )
        .bind(self.inner.account_id)
        .bind(&sealed)
        .execute(&self.inner.pool)
        .await
        .map_err(|err| store_err(&err))?;
        let hook = self
            .inner
            .saved
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook(tokens.expires_at());
        }
        Ok(())
    }

    async fn clear(&self) -> Result<(), AuthError> {
        if self.inner.account_id <= 0 {
            return Ok(());
        }
        let _guard = self.inner.lock.lock().await;
        sqlx::query(
            "UPDATE tesla_accounts
             SET tokens = NULL, status = 'disconnected', last_error = NULL
             WHERE id = $1",
        )
        .bind(self.inner.account_id)
        .execute(&self.inner.pool)
        .await
        .map_err(|err| store_err(&err))?;
        Ok(())
    }
}

pub(crate) async fn import_token_files(
    pool: &PgPool,
    config: &Config,
) -> Result<ImportedAccounts, AppError> {
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(IMPORT_LOCK)
        .execute(&mut *tx)
        .await?;
    let key = encryption_key(config.token_encryption_key.as_deref())?;
    let fleet_id = import_one(
        &mut tx,
        "fleet",
        "Fleet",
        &config.token_store_path,
        key.as_ref(),
    )
    .await?;
    let owner_id = import_one(
        &mut tx,
        "owner",
        "Owner",
        &config.owner_token_store_path,
        key.as_ref(),
    )
    .await?;
    sqlx::query("UPDATE vehicles SET account_id = $1 WHERE account_id IS NULL")
        .bind(fleet_id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(ImportedAccounts { fleet_id, owner_id })
}

pub(crate) async fn load_account(
    pool: &PgPool,
    account_id: i64,
) -> Result<Option<TeslaAccount>, AppError> {
    if account_id <= 0 {
        return Ok(None);
    }
    let row = sqlx::query_as::<_, TeslaAccount>(
        "SELECT id, label, kind, status, last_refresh_at, last_error
         FROM tesla_accounts WHERE id = $1",
    )
    .bind(account_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub(crate) async fn vehicles_for_account(
    pool: &PgPool,
    account_id: i64,
) -> Result<Vec<SharedVehicle>, AppError> {
    if account_id <= 0 {
        return Ok(Vec::new());
    }
    let rows = sqlx::query_as::<_, SharedVehicle>(
        "SELECT id, COALESCE(NULLIF(name, ''), 'Véhicule ' || id::text) AS name
         FROM vehicles WHERE account_id = $1 ORDER BY id",
    )
    .bind(account_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub(crate) async fn set_last_error(
    pool: &PgPool,
    account_id: i64,
    message: &str,
) -> Result<(), AppError> {
    if account_id <= 0 {
        return Ok(());
    }
    let message = clip(message);
    sqlx::query("UPDATE tesla_accounts SET last_error = $2, status = 'error' WHERE id = $1")
        .bind(account_id)
        .bind(message)
        .execute(pool)
        .await?;
    Ok(())
}

pub(crate) async fn clear_last_error(pool: &PgPool, account_id: i64) -> Result<(), AppError> {
    if account_id <= 0 {
        return Ok(());
    }
    sqlx::query(
        "UPDATE tesla_accounts
         SET last_error = NULL,
             status = CASE WHEN tokens IS NOT NULL AND status = 'error' THEN 'connected' ELSE status END
         WHERE id = $1 AND last_error IS NOT NULL",
    )
    .bind(account_id)
    .execute(pool)
    .await?;
    Ok(())
}

struct EncryptionKey(Option<[u8; 32]>);

impl Drop for EncryptionKey {
    fn drop(&mut self) {
        if let Some(key) = self.0.as_mut() {
            key.zeroize();
        }
    }
}

impl EncryptionKey {
    fn as_ref(&self) -> Option<&[u8; 32]> {
        self.0.as_ref()
    }
}

fn encryption_key(material: Option<&str>) -> Result<EncryptionKey, AppError> {
    let key = match material.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => Some(crypto::derive_key(value)?),
        None => None,
    };
    Ok(EncryptionKey(key))
}

#[derive(sqlx::FromRow)]
struct ExistingAccount {
    id: i64,
    has_tokens: bool,
    imported: bool,
}

async fn import_one(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    kind: &str,
    label: &str,
    path: &Path,
    key: Option<&[u8; 32]>,
) -> Result<i64, AppError> {
    let existing = sqlx::query_as::<_, ExistingAccount>(
        "SELECT id,
                tokens IS NOT NULL AS has_tokens,
                file_imported_at IS NOT NULL AS imported
         FROM tesla_accounts
         WHERE kind = $1
         ORDER BY id
         LIMIT 1",
    )
    .bind(kind)
    .fetch_optional(&mut **tx)
    .await?;
    let id = if let Some(row) = existing {
        if row.has_tokens || row.imported {
            if !row.imported {
                sqlx::query(
                    "UPDATE tesla_accounts SET file_imported_at = now()
                     WHERE id = $1 AND file_imported_at IS NULL",
                )
                .bind(row.id)
                .execute(&mut **tx)
                .await?;
            }
            return Ok(row.id);
        }
        row.id
    } else {
        let (id,): (i64,) = sqlx::query_as(
            "INSERT INTO tesla_accounts (label, kind, status) VALUES ($1, $2, 'unknown') RETURNING id",
        )
        .bind(label)
        .bind(kind)
        .fetch_one(&mut **tx)
        .await?;
        id
    };

    let mut bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(id),
        Err(err) => {
            let message = format!("token file could not be read: {err}");
            record_import_error(tx, id, &message).await?;
            return Ok(id);
        }
    };
    let stored = match normalize_token_bytes(&bytes, key) {
        Ok(stored) => stored,
        Err(err) => {
            bytes.zeroize();
            record_import_error(tx, id, &err.to_string()).await?;
            return Ok(id);
        }
    };
    bytes.zeroize();
    sqlx::query(
        "UPDATE tesla_accounts
         SET tokens = $2, status = 'connected', last_error = NULL, file_imported_at = now()
         WHERE id = $1",
    )
    .bind(id)
    .bind(&stored)
    .execute(&mut **tx)
    .await?;
    Ok(id)
}

async fn record_import_error(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: i64,
    message: &str,
) -> Result<(), AppError> {
    let message = clip(message);
    tracing::warn!(account_id = id, error = %message, "token file was not imported");
    sqlx::query("UPDATE tesla_accounts SET last_error = $2 WHERE id = $1")
        .bind(id)
        .bind(message)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

fn normalize_token_bytes(bytes: &[u8], key: Option<&[u8; 32]>) -> Result<Vec<u8>, AppError> {
    if bytes.starts_with(b"TLG1") {
        let mut plaintext = crypto::open_payload(bytes, key).map_err(AppError::message)?;
        let parsed = crypto::tokens_from_json(&plaintext)?;
        drop(parsed);
        plaintext.zeroize();
        return Ok(bytes.to_vec());
    }
    let mut plaintext = crypto::open_payload(bytes, key).map_err(AppError::message)?;
    let parsed = crypto::tokens_from_json(&plaintext)?;
    drop(parsed);
    let stored = match key {
        Some(key) => {
            let sealed = crypto::seal(key, &plaintext).map_err(AppError::message)?;
            plaintext.zeroize();
            sealed
        }
        None => plaintext,
    };
    Ok(stored)
}

fn clip(message: &str) -> String {
    redact_line(message, &[]).chars().take(500).collect()
}

fn store_err(err: &sqlx::Error) -> AuthError {
    tracing::warn!(
        error = %redact_secrets(&err.to_string()),
        "token store query failed"
    );
    AuthError::TokenStore {
        message: "token store query failed".to_owned(),
    }
}

fn store_msg(err: impl std::fmt::Display) -> AuthError {
    AuthError::TokenStore {
        message: redact_line(&err.to_string(), &[]),
    }
}

fn system_time_to_utc(time: SystemTime) -> Option<DateTime<Utc>> {
    let Ok(duration) = time.duration_since(UNIX_EPOCH) else {
        return None;
    };
    let secs = i64::try_from(duration.as_secs()).ok()?;
    DateTime::<Utc>::from_timestamp(secs, duration.subsec_nanos())
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use tesla_auth::{ExposeSecret, Scope, SecretString, TokenSet, TokenStore};

    use super::{DbTokenStore, import_token_files};
    use crate::config::Config;
    use crate::crypto::TokenFile;

    fn sample_tokens(access: &str) -> TokenSet {
        TokenSet::bearer(
            SecretString::from(access.to_owned()),
            Some(SecretString::from("super-secret-refresh".to_owned())),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_900_000_000),
            vec![
                Scope::OpenId,
                Scope::OfflineAccess,
                Scope::VehicleDeviceData,
            ],
        )
    }

    fn config_for(dir: &std::path::Path) -> Config {
        use std::collections::BTreeMap;
        let mut map = BTreeMap::new();
        map.insert("TESLA_CLIENT_ID".to_owned(), "client".to_owned());
        map.insert("TESLA_CLIENT_SECRET".to_owned(), "client-secret".to_owned());
        map.insert(
            "TESLA_FLEET_AUDIENCE".to_owned(),
            "https://fleet-api.prd.eu.vn.cloud.tesla.com".to_owned(),
        );
        map.insert("TESLA_DOMAIN".to_owned(), "logger.test".to_owned());
        map.insert(
            "DATABASE_URL".to_owned(),
            "postgres://tesla:db-secret@localhost/tesla".to_owned(),
        );
        map.insert(
            "TESLA_PRIVATE_KEY_PATH".to_owned(),
            "/tmp/not-read.pem".to_owned(),
        );
        map.insert(
            "TESLA_TOKEN_STORE_PATH".to_owned(),
            dir.join("tokens.json").display().to_string(),
        );
        map.insert(
            "TESLA_OWNER_TOKEN_STORE_PATH".to_owned(),
            dir.join("owner-tokens.json").display().to_string(),
        );
        map.insert(
            "TESLA_LOGGER_UI_PASSWORD".to_owned(),
            "ui-secret".to_owned(),
        );
        map.insert(
            "TESLA_TOKEN_ENCRYPTION_KEY".to_owned(),
            "test-key".to_owned(),
        );
        Config::from_map(&map).unwrap()
    }

    #[tokio::test]
    async fn token_file_import_is_idempotent_and_leaves_the_file() {
        let Some(db) = crate::test_util::TestDb::migrated("accounts").await else {
            return;
        };
        let dir = std::env::temp_dir().join(format!(
            "tl-import-{}",
            crate::crypto::random_token().unwrap()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tokens.json");
        let file = TokenFile::open(&path, None).unwrap();
        file.save(&sample_tokens("super-secret-access"))
            .await
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        assert!(
            String::from_utf8_lossy(&before).contains("super-secret-access"),
            "the backup file stays plaintext so the test can see it was not rewritten"
        );
        sqlx::query("INSERT INTO vehicles (vin, name) VALUES ('5YJ3E1EA7KF000001', 'Red')")
            .execute(&db.pool)
            .await
            .unwrap();

        let config = config_for(&dir);
        let imported = import_token_files(&db.pool, &config).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert!(!dir.join("owner-tokens.json").exists());

        let fleet_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM tesla_accounts WHERE kind = 'fleet'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        let owner_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM tesla_accounts WHERE kind = 'owner'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(fleet_count.0, 1);
        assert_eq!(owner_count.0, 1);

        let stored: (Vec<u8>,) = sqlx::query_as("SELECT tokens FROM tesla_accounts WHERE id = $1")
            .bind(imported.fleet_id)
            .fetch_one(&db.pool)
            .await
            .unwrap();
        let rendered = String::from_utf8_lossy(&stored.0);
        assert!(!rendered.contains("super-secret-access"));
        assert!(!rendered.contains("super-secret-refresh"));
        assert!(stored.0.starts_with(b"TLG1"));

        let linked: (Option<i64>,) =
            sqlx::query_as("SELECT account_id FROM vehicles WHERE name = 'Red'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(linked.0, Some(imported.fleet_id));

        std::fs::write(&path, b"{\"access_token\":\"other-secret-access\"}").unwrap();
        let again = import_token_files(&db.pool, &config).await.unwrap();
        assert_eq!(again.fleet_id, imported.fleet_id);
        assert_eq!(again.owner_id, imported.owner_id);
        let fleet_count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM tesla_accounts WHERE kind = 'fleet'")
                .fetch_one(&db.pool)
                .await
                .unwrap();
        assert_eq!(fleet_count.0, 1);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"access_token\":\"other-secret-access\"}"
        );

        let store =
            DbTokenStore::open(db.pool.clone(), imported.fleet_id, Some("test-key")).unwrap();
        let loaded = store.load().await.unwrap().unwrap();
        assert_eq!(loaded.access_token().expose_secret(), "super-secret-access");
        assert_eq!(
            loaded.scopes(),
            &[
                Scope::OpenId,
                Scope::OfflineAccess,
                Scope::VehicleDeviceData
            ]
        );
        drop(loaded);

        store.clear().await.unwrap();
        assert!(store.load().await.unwrap().is_none());
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"{\"access_token\":\"other-secret-access\"}"
        );
        import_token_files(&db.pool, &config).await.unwrap();
        assert!(
            store.load().await.unwrap().is_none(),
            "a disconnect must not import the backup file again"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
