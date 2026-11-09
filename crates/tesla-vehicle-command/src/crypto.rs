// SPDX-License-Identifier: MIT OR Apache-2.0

use aes_gcm::aead::generic_array::GenericArray;
use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes128Gcm, KeyInit};
use hmac::{Hmac, Mac};
use p256::PublicKey;
use p256::elliptic_curve::sec1::ToSec1Point;
use secrecy::{ExposeSecret, SecretBox};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroize;

use crate::CommandError;

type HmacSha256 = Hmac<Sha256>;

pub(crate) const PUBLIC_KEY_LEN: usize = 65;
const SHARED_KEY_LEN: usize = 16;
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;

/// 16-byte session key `K = SHA1(x)[:16]`.
pub(crate) struct SessionKey(SecretBox<[u8; SHARED_KEY_LEN]>);

impl SessionKey {
    pub(crate) fn expose(&self) -> &[u8; SHARED_KEY_LEN] {
        self.0.expose_secret()
    }
}

impl std::fmt::Debug for SessionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionKey(<redacted>)")
    }
}

/// ECDH shared secret, then the protocol's SHA-1 truncation.
///
/// SHA-1 is required by the vehicle. It maps the x coordinate to 128 bits. It is not used as a
/// signature hash.
pub(crate) fn shared_secret(
    local: &p256::SecretKey,
    remote_uncompressed: &[u8],
) -> Result<SessionKey, CommandError> {
    if remote_uncompressed.len() != PUBLIC_KEY_LEN || remote_uncompressed[0] != 0x04 {
        return Err(CommandError::InvalidRemoteKey);
    }
    let remote = PublicKey::from_sec1_bytes(remote_uncompressed)
        .map_err(|_| CommandError::InvalidRemoteKey)?;
    let shared = p256::ecdh::diffie_hellman(local.to_nonzero_scalar(), remote.as_affine());
    let mut x = [0_u8; 32];
    x.copy_from_slice(shared.raw_secret_bytes());
    let digest = <Sha1 as sha1::Digest>::digest(x);
    x.zeroize();
    let mut key = [0_u8; SHARED_KEY_LEN];
    key.copy_from_slice(&digest[..SHARED_KEY_LEN]);
    let mut full = [0_u8; 20];
    full.copy_from_slice(&digest);
    full.zeroize();
    Ok(SessionKey(SecretBox::new(Box::new(key))))
}

pub(crate) fn session_info_tag(
    key: &[u8; SHARED_KEY_LEN],
    vin: &[u8],
    challenge: &[u8],
    encoded_info: &[u8],
) -> Result<[u8; 32], CommandError> {
    use crate::metadata::Metadata;
    let mut meta = Metadata::new();
    meta.add(0, &[6])?;
    meta.add(2, vin)?;
    meta.add(6, challenge)?;
    let message = meta.finish(encoded_info);
    let mac_key = labeled_key(key, b"session info")?;
    hmac_sha256(mac_key.as_slice(), &message)
}

pub(crate) fn command_tag(
    key: &[u8; SHARED_KEY_LEN],
    metadata: &[u8],
) -> Result<[u8; 32], CommandError> {
    let mac_key = labeled_key(key, b"authenticated command")?;
    hmac_sha256(mac_key.as_slice(), metadata)
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut out = [0_u8; 32];
    out.copy_from_slice(&digest);
    out
}

pub(crate) fn seal(
    key: &[u8; SHARED_KEY_LEN],
    nonce: &[u8; NONCE_LEN],
    plaintext: &[u8],
    aad: &[u8],
) -> Result<(Vec<u8>, [u8; TAG_LEN]), CommandError> {
    let cipher = Aes128Gcm::new_from_slice(key).map_err(|_| CommandError::Metadata)?;
    let nonce = GenericArray::from_slice(nonce);
    let encrypted = cipher
        .encrypt(
            nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| CommandError::Metadata)?;
    if encrypted.len() < TAG_LEN {
        return Err(CommandError::Metadata);
    }
    let split = encrypted.len() - TAG_LEN;
    let mut tag = [0_u8; TAG_LEN];
    tag.copy_from_slice(&encrypted[split..]);
    Ok((encrypted[..split].to_vec(), tag))
}

pub(crate) fn open(
    key: &[u8; SHARED_KEY_LEN],
    nonce: &[u8],
    ciphertext: &[u8],
    tag: &[u8],
    aad: &[u8],
) -> Result<Vec<u8>, CommandError> {
    if nonce.len() != NONCE_LEN || tag.len() != TAG_LEN {
        return Err(CommandError::Decrypt);
    }
    let cipher = Aes128Gcm::new_from_slice(key).map_err(|_| CommandError::Decrypt)?;
    let nonce = GenericArray::from_slice(nonce);
    let mut combined = Vec::with_capacity(ciphertext.len() + tag.len());
    combined.extend_from_slice(ciphertext);
    combined.extend_from_slice(tag);
    cipher
        .decrypt(
            nonce,
            Payload {
                msg: &combined,
                aad,
            },
        )
        .map_err(|_| CommandError::Decrypt)
}

pub(crate) fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    bool::from(left.ct_eq(right))
}

pub(crate) fn random_bytes<const N: usize>() -> Result<[u8; N], CommandError> {
    let mut bytes = [0_u8; N];
    getrandom::getrandom(&mut bytes).map_err(|_| CommandError::Entropy)?;
    Ok(bytes)
}

pub(crate) fn uncompressed_public(key: &p256::SecretKey) -> [u8; PUBLIC_KEY_LEN] {
    let encoded = key.public_key().to_sec1_point(false);
    let mut out = [0_u8; PUBLIC_KEY_LEN];
    out.copy_from_slice(encoded.as_bytes());
    out
}

fn labeled_key(key: &[u8; SHARED_KEY_LEN], label: &[u8]) -> Result<MacKey, CommandError> {
    Ok(MacKey(hmac_sha256(key, label)?))
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<[u8; 32], CommandError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).map_err(|_| CommandError::Metadata)?;
    mac.update(data);
    let bytes = mac.finalize().into_bytes();
    let mut out = [0_u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

struct MacKey([u8; 32]);

impl MacKey {
    fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

impl Drop for MacKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}
