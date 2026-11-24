// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use p256::SecretKey;
use p256::pkcs8::{DecodePrivateKey, EncodePublicKey, LineEnding};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tesla_auth::{AuthError, ExposeSecret, Scope, SecretString, TokenSet, TokenStore};
use tokio::sync::Mutex;
use zeroize::Zeroize;

use crate::error::AppError;

const MAGIC: &[u8; 4] = b"TLG1";

/// AES-256-GCM token file, or plaintext JSON when no encryption key is configured.
///
/// The file mode is `0600` on Unix. The encrypted form does not contain the token
/// bytes. The id token is not retained: Fleet API calls only need the access token
/// and the refresh token. `FileTokenStore` in `tesla-auth` remains available for
/// callers that must keep the id token; this server keeps the two tokens required
/// to poll.
pub(crate) type TokenSaved = std::sync::Arc<dyn Fn(std::time::SystemTime) + Send + Sync>;

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct TokenFile {
    path: PathBuf,
    key: Option<EncryptionKey>,
    lock: Mutex<()>,
    saved: std::sync::Mutex<Option<TokenSaved>>,
}

#[allow(clippy::missing_fields_in_debug)]
impl std::fmt::Debug for TokenFile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TokenFile")
            .field("path", &self.path)
            .field("encrypted", &self.key.is_some())
            .finish()
    }
}

impl Drop for TokenFile {
    fn drop(&mut self) {
        if let Some(key) = self.key.as_mut() {
            key.zeroize();
        }
    }
}

/// AES-256 key material. Passphrases derive with Argon2id; a legacy SHA-256
/// digest is kept so files sealed before that change still open.
pub(crate) struct EncryptionKey {
    primary: [u8; 32],
    legacy_sha256: Option<[u8; 32]>,
}

impl EncryptionKey {
    pub(crate) fn primary(&self) -> &[u8; 32] {
        &self.primary
    }

    fn candidates(&self) -> impl Iterator<Item = &[u8; 32]> {
        std::iter::once(&self.primary).chain(self.legacy_sha256.as_ref())
    }

    fn zeroize(&mut self) {
        self.primary.zeroize();
        if let Some(legacy) = self.legacy_sha256.as_mut() {
            legacy.zeroize();
        }
    }
}

impl Drop for EncryptionKey {
    fn drop(&mut self) {
        self.zeroize();
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl TokenFile {
    pub(crate) fn open(
        path: impl Into<PathBuf>,
        encryption_key: Option<&str>,
    ) -> Result<Self, AppError> {
        let key = match encryption_key
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            Some(value) => Some(derive_encryption_key(value)?),
            None => None,
        };
        Ok(Self {
            path: path.into(),
            key,
            lock: Mutex::new(()),
            saved: std::sync::Mutex::new(None),
        })
    }

    /// Called after a token set is stored. The hook receives the access-token expiry only.
    #[allow(dead_code)]
    pub(crate) fn on_saved(&self, hook: TokenSaved) {
        *self
            .saved
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(hook);
    }

    pub(crate) async fn load(&self) -> Result<Option<TokenSet>, AuthError> {
        let _guard = self.lock.lock().await;
        let bytes = match tokio::fs::read(&self.path).await {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(store_err(err)),
        };
        let mut plaintext = match open_payload_with(&bytes, self.key.as_ref()) {
            Ok(plaintext) => plaintext,
            Err(err) => return Err(store_err(err)),
        };
        let parsed = tokens_from_json(&plaintext);
        plaintext.zeroize();
        parsed.map(Some)
    }

    pub(crate) async fn save(&self, tokens: &TokenSet) -> Result<(), AuthError> {
        let _guard = self.lock.lock().await;
        let mut plaintext = tokens_to_json(tokens).map_err(store_err)?;
        let sealed = match self.key.as_ref() {
            Some(key) => seal(key.primary(), &plaintext).map_err(store_err)?,
            None => plaintext.clone(),
        };
        plaintext.zeroize();
        write_private(&self.path, &sealed)
            .await
            .map_err(store_err)?;
        let hook = self
            .saved
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if let Some(hook) = hook {
            hook(tokens.expires_at());
        }
        Ok(())
    }

    pub(crate) async fn clear(&self) -> Result<(), AuthError> {
        let _guard = self.lock.lock().await;
        match tokio::fs::remove_file(&self.path).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(store_err(err)),
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl TokenStore for TokenFile {
    async fn load(&self) -> Result<Option<TokenSet>, AuthError> {
        TokenFile::load(self).await
    }

    async fn save(&self, tokens: &TokenSet) -> Result<(), AuthError> {
        TokenFile::save(self, tokens).await
    }

    async fn clear(&self) -> Result<(), AuthError> {
        TokenFile::clear(self).await
    }
}

pub(crate) fn public_key_pem(private_pem: &str) -> Result<String, AppError> {
    let secret = if private_pem.contains("BEGIN EC PRIVATE KEY") {
        SecretKey::from_sec1_pem(private_pem)
            .map_err(|_| AppError::message("private key is not a P-256 SEC1 PEM"))?
    } else {
        SecretKey::from_pkcs8_pem(private_pem)
            .map_err(|_| AppError::message("private key is not a P-256 PKCS#8 PEM"))?
    };
    secret
        .public_key()
        .to_public_key_pem(LineEnding::LF)
        .map_err(|_| AppError::message("could not encode the P-256 public key"))
}

pub(crate) fn read_public_key(path: &Path) -> Result<String, AppError> {
    let pem = std::fs::read_to_string(path).map_err(|_| {
        AppError::message(format!(
            "could not read the private key at {}",
            path.display()
        ))
    })?;
    public_key_pem(&pem)
}

pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N], AppError> {
    let mut bytes = [0u8; N];
    getrandom::getrandom(&mut bytes).map_err(|_| AppError::message("entropy source failed"))?;
    Ok(bytes)
}

pub(crate) fn random_token() -> Result<String, AppError> {
    Ok(URL_SAFE_NO_PAD.encode(random_bytes::<32>()?))
}

pub(crate) fn sha256_hex(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

/// SHA-256 of the public PEM. The private key is never hashed or returned.
pub(crate) fn public_key_fingerprint(pem: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(pem.as_bytes())))
}

/// Constant-time equality for equal-length byte slices. Different lengths return false.
pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut diff = 0u8;
    for (a, b) in left.iter().zip(right.iter()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Derive the token-file encryption key.
///
/// 64 hex characters are the raw 32-byte key. Any other value is hashed with
/// Argon2id (same memory/time as the login hasher). A legacy SHA-256 digest is
/// retained so ciphertext sealed before the Argon2id change still decrypts.
pub(crate) fn derive_encryption_key(material: &str) -> Result<EncryptionKey, AppError> {
    if material.len() == 64 && material.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        let mut key = [0u8; 32];
        hex::decode_to_slice(material, &mut key)
            .map_err(|_| AppError::message("TESLA_TOKEN_ENCRYPTION_KEY is not valid hex"))?;
        return Ok(EncryptionKey {
            primary: key,
            legacy_sha256: None,
        });
    }
    Ok(EncryptionKey {
        primary: argon2id_key(material.as_bytes())?,
        legacy_sha256: Some(sha256_key(material.as_bytes())),
    })
}

fn sha256_key(material: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(material);
    let mut key = [0u8; 32];
    key.copy_from_slice(&digest);
    key
}

fn argon2id_key(material: &[u8]) -> Result<[u8; 32], AppError> {
    use argon2::{Algorithm, Argon2, Params, Version};
    // Fixed public salt: this is a KDF for a configured secret, not a password store.
    const SALT: &[u8] = b"tesla-logger-tok1";
    let params = Params::new(19 * 1024, 2, 1, Some(32))
        .map_err(|_| AppError::message("token encryption key derivation failed"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(material, SALT, &mut key)
        .map_err(|_| AppError::message("token encryption key derivation failed"))?;
    Ok(key)
}

pub(crate) fn seal(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|_| "encryption key was rejected".to_owned())?;
    let mut nonce_bytes = [0u8; 12];
    getrandom::getrandom(&mut nonce_bytes).map_err(|_| "entropy source failed".to_owned())?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| "token encryption failed".to_owned())?;
    let mut out = Vec::with_capacity(4 + nonce_bytes.len() + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

pub(crate) fn open_payload_with(
    bytes: &[u8],
    key: Option<&EncryptionKey>,
) -> Result<Vec<u8>, String> {
    if bytes.starts_with(MAGIC) {
        let Some(key) = key else {
            return Err("token file is encrypted; set TESLA_TOKEN_ENCRYPTION_KEY".to_owned());
        };
        if bytes.len() < 4 + 12 + 16 {
            return Err("token file is truncated".to_owned());
        }
        let nonce = Nonce::from_slice(&bytes[4..16]);
        let mut last_err = "token file could not be decrypted";
        for candidate in key.candidates() {
            let Ok(cipher) = Aes256Gcm::new_from_slice(candidate) else {
                last_err = "encryption key was rejected";
                continue;
            };
            match cipher.decrypt(nonce, &bytes[16..]) {
                Ok(plaintext) => return Ok(plaintext),
                Err(_) => last_err = "token file could not be decrypted",
            }
        }
        return Err(last_err.to_owned());
    }
    if key.is_some() && !bytes.starts_with(b"{") {
        return Err("token file is not a tesla-logger token store".to_owned());
    }
    Ok(bytes.to_vec())
}

#[derive(Serialize, Deserialize)]
struct StoredTokens {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    token_type: String,
    expires_at: u64,
    #[serde(default)]
    scopes: Vec<String>,
}

impl Drop for StoredTokens {
    fn drop(&mut self) {
        self.access_token.zeroize();
        if let Some(token) = self.refresh_token.as_mut() {
            token.zeroize();
        }
    }
}

pub(crate) fn tokens_to_json(tokens: &TokenSet) -> Result<Vec<u8>, String> {
    let stored = StoredTokens {
        access_token: tokens.access_token().expose_secret().to_owned(),
        refresh_token: tokens
            .refresh_token()
            .map(|token| token.expose_secret().to_owned()),
        token_type: tokens.token_type().to_owned(),
        expires_at: tokens
            .expires_at()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_secs()),
        scopes: tokens
            .scopes()
            .iter()
            .map(|scope| scope.as_str().to_owned())
            .collect(),
    };
    serde_json::to_vec(&stored).map_err(|_| "token set could not be encoded".to_owned())
}

pub(crate) fn tokens_from_json(bytes: &[u8]) -> Result<TokenSet, AuthError> {
    let mut stored: StoredTokens =
        serde_json::from_slice(bytes).map_err(|_| AuthError::TokenStore {
            message: "token file is not a valid token set".to_owned(),
        })?;
    let scope_text = std::mem::take(&mut stored.scopes).join(" ");
    let scopes = if scope_text.is_empty() {
        Vec::new()
    } else {
        Scope::parse_list(&scope_text).map_err(|_| AuthError::TokenStore {
            message: "token file contains an invalid scope".to_owned(),
        })?
    };
    let access = SecretString::from(std::mem::take(&mut stored.access_token));
    let refresh = stored.refresh_token.take().map(SecretString::from);
    let expires_at = UNIX_EPOCH + Duration::from_secs(stored.expires_at);
    Ok(TokenSet::bearer(access, refresh, expires_at, scopes))
}

#[cfg_attr(not(test), allow(dead_code))]
async fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|err| err.to_string())?;
    }
    let tmp = temporary_path(path);
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|err| err.to_string())?;
    restrict(&tmp)?;
    tokio::fs::rename(&tmp, path)
        .await
        .map_err(|err| err.to_string())?;
    restrict(path)?;
    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map_or_else(|| "tokens".into(), |name| name.to_os_string());
    name.push(".tmp");
    path.with_file_name(name)
}

#[cfg_attr(not(test), allow(dead_code))]
fn restrict(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|err| err.to_string())?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
fn store_err(err: impl std::fmt::Display) -> AuthError {
    AuthError::TokenStore {
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use tesla_auth::{ExposeSecret, Scope, SecretString, TokenSet};

    use super::{TokenFile, public_key_pem};

    fn sample_tokens() -> TokenSet {
        TokenSet::bearer(
            SecretString::from("super-secret-access".to_owned()),
            Some(SecretString::from("super-secret-refresh".to_owned())),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_900_000_000),
            vec![Scope::OpenId, Scope::OfflineAccess],
        )
    }

    #[test]
    fn derives_a_public_pem_without_the_private_header() {
        use p256::elliptic_curve::Generate;
        use p256::pkcs8::{EncodePrivateKey, LineEnding};
        let secret = p256::SecretKey::generate();
        let private_pem = secret.to_pkcs8_pem(LineEnding::LF).unwrap();
        let public_pem = public_key_pem(private_pem.as_str()).unwrap();
        assert!(public_pem.contains("BEGIN PUBLIC KEY"));
        assert!(!public_pem.contains("PRIVATE KEY"));
        assert!(!public_pem.contains(private_pem.as_str()));
    }

    #[test]
    fn passphrase_keys_keep_sha256_legacy_for_old_ciphertext() {
        let material = "legacy-passphrase-value";
        let keys = super::derive_encryption_key(material).unwrap();
        let legacy = super::sha256_key(material.as_bytes());
        let plaintext = b"{\"access_token\":\"x\",\"token_type\":\"Bearer\",\"expires_at\":1}";
        let sealed = super::seal(&legacy, plaintext).unwrap();
        let opened = super::open_payload_with(&sealed, Some(&keys)).unwrap();
        assert_eq!(opened, plaintext);
        assert_ne!(keys.primary(), &legacy);
    }

    #[test]
    fn constant_time_eq_rejects_length_mismatches() {
        assert!(super::constant_time_eq(b"abcd", b"abcd"));
        assert!(!super::constant_time_eq(b"abcd", b"abce"));
        assert!(!super::constant_time_eq(b"abc", b"abcd"));
    }

    #[tokio::test]
    async fn encrypted_store_hides_tokens_and_is_private() {
        let dir = std::env::temp_dir().join(format!("tl-token-{}", super::random_token().unwrap()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tokens.json");
        let store = TokenFile::open(&path, Some("test-passphrase-value")).unwrap();
        let tokens = sample_tokens();
        store.save(&tokens).await.unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let rendered = String::from_utf8_lossy(&bytes);
        assert!(!rendered.contains("super-secret-access"));
        assert!(!rendered.contains("super-secret-refresh"));
        assert!(bytes.starts_with(b"TLG1"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let loaded = store.load().await.unwrap().unwrap();
        assert_eq!(loaded.access_token().expose_secret(), "super-secret-access");
        assert_eq!(
            loaded.refresh_token().unwrap().expose_secret(),
            "super-secret-refresh"
        );
        store.clear().await.unwrap();
        assert!(store.load().await.unwrap().is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
