// SPDX-License-Identifier: MIT OR Apache-2.0

use std::future::Future;
use std::path::{Path, PathBuf};

use tokio::sync::Mutex;
use zeroize::Zeroize;

use crate::{AuthError, TokenSet};

/// Persistence for a user [`TokenSet`](crate::TokenSet).
///
/// Implementations must not log token values. [`Session`](crate::Session) serializes refresh so
/// two tasks do not rotate the single-use refresh token at the same time. A store shared across
/// processes still needs its own lock.
pub trait TokenStore: Send + Sync {
    /// Loads the stored token set, or `Ok(None)` when nothing has been stored.
    fn load(&self) -> impl Future<Output = Result<Option<TokenSet>, AuthError>> + Send;

    /// Replaces the stored token set.
    fn save(&self, tokens: &TokenSet) -> impl Future<Output = Result<(), AuthError>> + Send;

    /// Removes any stored token set.
    fn clear(&self) -> impl Future<Output = Result<(), AuthError>> + Send;
}

/// In-memory token store for tests and short-lived processes.
#[derive(Debug, Default)]
pub struct MemoryTokenStore {
    inner: Mutex<Option<TokenSet>>,
}

impl MemoryTokenStore {
    /// Empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl TokenStore for MemoryTokenStore {
    async fn load(&self) -> Result<Option<TokenSet>, AuthError> {
        Ok(self.inner.lock().await.clone())
    }

    async fn save(&self, tokens: &TokenSet) -> Result<(), AuthError> {
        *self.inner.lock().await = Some(tokens.clone());
        Ok(())
    }

    async fn clear(&self) -> Result<(), AuthError> {
        *self.inner.lock().await = None;
        Ok(())
    }
}

/// JSON token file.
///
/// On Unix the file is created with mode `0600` and replaced via rename so a crash during refresh
/// does not leave an empty file. The JSON contains secrets; protect the path and do not commit it.
#[derive(Debug)]
pub struct FileTokenStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileTokenStore {
    /// Stores tokens at `path`. The file is created on the first successful [`TokenStore::save`].
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock: Mutex::new(()),
        }
    }

    /// Path passed to [`Self::new`].
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl TokenStore for FileTokenStore {
    async fn load(&self) -> Result<Option<TokenSet>, AuthError> {
        let _guard = self.lock.lock().await;
        match tokio::fs::read(&self.path).await {
            Ok(mut bytes) => {
                let parsed = TokenSet::from_file_bytes(&bytes);
                bytes.zeroize();
                parsed.map(Some)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(AuthError::token_store(err)),
        }
    }

    async fn save(&self, tokens: &TokenSet) -> Result<(), AuthError> {
        let _guard = self.lock.lock().await;
        let mut bytes = tokens.to_file_bytes()?;
        let tmp = temporary_path(&self.path);
        let write_result = write_token_file(&tmp, &self.path, &bytes).await;
        bytes.zeroize();
        write_result
    }

    async fn clear(&self) -> Result<(), AuthError> {
        let _guard = self.lock.lock().await;
        match tokio::fs::remove_file(&self.path).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(AuthError::token_store(err)),
        }
    }
}

async fn write_token_file(tmp: &Path, dest: &Path, bytes: &[u8]) -> Result<(), AuthError> {
    if let Some(parent) = dest.parent()
        && !parent.as_os_str().is_empty()
    {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(AuthError::token_store)?;
    }
    tokio::fs::write(tmp, bytes)
        .await
        .map_err(AuthError::token_store)?;
    restrict_permissions(tmp)?;
    tokio::fs::rename(tmp, dest)
        .await
        .map_err(AuthError::token_store)?;
    restrict_permissions(dest)?;
    Ok(())
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map_or_else(|| "tokens".into(), |name| name.to_os_string());
    name.push(".tmp");
    path.with_file_name(name)
}

fn restrict_permissions(path: &Path) -> Result<(), AuthError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(AuthError::token_store)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
