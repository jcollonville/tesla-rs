// SPDX-License-Identifier: MIT OR Apache-2.0

use std::fmt;
use std::fs;
use std::path::Path;

use p256::SecretKey;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey, LineEnding};
use secrecy::{ExposeSecret, SecretBox, SecretString};
use zeroize::Zeroize;

use crate::CommandError;
use crate::crypto::{self, PUBLIC_KEY_LEN};

/// Which PEM wrapper to write for a P-256 private key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrivateKeyFormat {
    /// `BEGIN EC PRIVATE KEY` (SEC1).
    Sec1,
    /// `BEGIN PRIVATE KEY` (PKCS#8).
    Pkcs8,
}

/// P-256 private key used as a Tesla virtual key.
///
/// The scalar is stored in a [`SecretBox`] and zeroized on drop. [`Debug`] does not print it.
/// The public key Tesla expects is the 65-byte uncompressed point (`0x04 || x || y`).
pub struct PrivateKey {
    scalar: SecretBox<[u8; 32]>,
    public: [u8; PUBLIC_KEY_LEN],
}

impl fmt::Debug for PrivateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PrivateKey(<redacted>)")
    }
}

impl PrivateKey {
    /// Generates a new P-256 key.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::Entropy`] when the operating system RNG fails.
    pub fn generate() -> Result<Self, CommandError> {
        let mut bytes = [0_u8; 32];
        loop {
            getrandom::getrandom(&mut bytes).map_err(|_| CommandError::Entropy)?;
            if let Ok(key) = SecretKey::from_slice(&bytes) {
                bytes.zeroize();
                return Ok(Self::from_secret_key(&key));
            }
            bytes.zeroize();
        }
    }

    /// Loads a PEM private key.
    ///
    /// SEC1 (`BEGIN EC PRIVATE KEY`) and unencrypted PKCS#8 (`BEGIN PRIVATE KEY`) are accepted.
    /// Encrypted PKCS#8 is rejected.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidPrivateKey`] or [`CommandError::EncryptedPrivateKey`].
    /// The error does not include the PEM text.
    pub fn from_pem(pem: &str) -> Result<Self, CommandError> {
        let trimmed = pem.trim();
        if trimmed.contains("BEGIN ENCRYPTED PRIVATE KEY")
            || trimmed.contains("Proc-Type: 4,ENCRYPTED")
        {
            return Err(CommandError::EncryptedPrivateKey);
        }
        let key = if trimmed.contains("BEGIN EC PRIVATE KEY") {
            SecretKey::from_sec1_pem(trimmed).map_err(|_| CommandError::InvalidPrivateKey)?
        } else if trimmed.contains("BEGIN PRIVATE KEY") {
            SecretKey::from_pkcs8_pem(trimmed).map_err(|_| CommandError::InvalidPrivateKey)?
        } else {
            return Err(CommandError::InvalidPrivateKey);
        };
        Ok(Self::from_secret_key(&key))
    }

    /// Loads PEM or DER from `path`, or a raw 32-byte scalar.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::KeyFile`] when the file cannot be read, and the same key errors as
    /// [`Self::from_pem`].
    pub fn read_file(path: &Path) -> Result<Self, CommandError> {
        let mut bytes = fs::read(path).map_err(|_| CommandError::KeyFile)?;
        let parsed = if let Ok(text) = std::str::from_utf8(&bytes) {
            if text.contains("BEGIN") {
                Self::from_pem(text)
            } else {
                Self::from_der_or_scalar(&bytes)
            }
        } else {
            Self::from_der_or_scalar(&bytes)
        };
        bytes.zeroize();
        parsed
    }

    /// SEC1 PEM. The returned string is a secret.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidPrivateKey`] when the scalar cannot be encoded.
    pub fn to_sec1_pem(&self) -> Result<SecretString, CommandError> {
        let key = self.secret_key()?;
        let pem = key
            .to_sec1_pem(LineEnding::LF)
            .map_err(|_| CommandError::InvalidPrivateKey)?;
        Ok(SecretString::from(pem.to_string()))
    }

    /// PKCS#8 PEM. The returned string is a secret.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidPrivateKey`] when the scalar cannot be encoded.
    pub fn to_pkcs8_pem(&self) -> Result<SecretString, CommandError> {
        let key = self.secret_key()?;
        let pem = key
            .to_pkcs8_pem(LineEnding::LF)
            .map_err(|_| CommandError::InvalidPrivateKey)?;
        Ok(SecretString::from(pem.to_string()))
    }

    /// SubjectPublicKeyInfo PEM. This is not secret.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::InvalidPrivateKey`] when the public key cannot be encoded.
    pub fn public_key_pem(&self) -> Result<String, CommandError> {
        let key = self.secret_key()?;
        key.public_key()
            .to_public_key_pem(LineEnding::LF)
            .map_err(|_| CommandError::InvalidPrivateKey)
    }

    /// Uncompressed SEC1 public key: `0x04 || X || Y`.
    #[must_use]
    pub fn public_key_uncompressed(&self) -> &[u8; PUBLIC_KEY_LEN] {
        &self.public
    }

    /// Writes a private-key PEM with mode `0600` on Unix.
    ///
    /// The file is replaced via a temporary sibling. Existing contents are overwritten.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError::KeyFile`] on I/O failure and [`CommandError::InvalidPrivateKey`]
    /// when the PEM cannot be encoded.
    pub fn write_pem_file(
        &self,
        path: &Path,
        format: PrivateKeyFormat,
    ) -> Result<(), CommandError> {
        let pem = match format {
            PrivateKeyFormat::Sec1 => self.to_sec1_pem()?,
            PrivateKeyFormat::Pkcs8 => self.to_pkcs8_pem()?,
        };
        write_secret_file(path, pem.expose_secret().as_bytes())
    }

    pub(crate) fn secret_key(&self) -> Result<SecretKey, CommandError> {
        SecretKey::from_slice(self.scalar.expose_secret())
            .map_err(|_| CommandError::InvalidPrivateKey)
    }

    fn from_secret_key(key: &SecretKey) -> Self {
        let mut raw = [0_u8; 32];
        raw.copy_from_slice(key.to_bytes().as_slice());
        let public = crypto::uncompressed_public(key);
        let scalar = SecretBox::new(Box::new(raw));
        raw.zeroize();
        Self { scalar, public }
    }

    fn from_der_or_scalar(bytes: &[u8]) -> Result<Self, CommandError> {
        if bytes.len() == 32 {
            let mut raw = [0_u8; 32];
            raw.copy_from_slice(bytes);
            let key = SecretKey::from_slice(&raw).map_err(|_| CommandError::InvalidPrivateKey)?;
            raw.zeroize();
            return Ok(Self::from_secret_key(&key));
        }
        if let Ok(key) = SecretKey::from_pkcs8_der(bytes) {
            return Ok(Self::from_secret_key(&key));
        }
        if let Ok(key) = SecretKey::from_sec1_der(bytes) {
            return Ok(Self::from_secret_key(&key));
        }
        Err(CommandError::InvalidPrivateKey)
    }
}

fn write_secret_file(path: &Path, bytes: &[u8]) -> Result<(), CommandError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|_| CommandError::KeyFile)?;
    }
    let tmp = temporary_path(path);
    fs::write(&tmp, bytes).map_err(|_| CommandError::KeyFile)?;
    restrict_permissions(&tmp)?;
    fs::rename(&tmp, path).map_err(|_| CommandError::KeyFile)?;
    restrict_permissions(path)?;
    Ok(())
}

fn temporary_path(path: &Path) -> std::path::PathBuf {
    let mut name = path
        .file_name()
        .map_or_else(|| "key".into(), |name| name.to_os_string());
    name.push(".tmp");
    path.with_file_name(name)
}

fn restrict_permissions(path: &Path) -> Result<(), CommandError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|_| CommandError::KeyFile)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
