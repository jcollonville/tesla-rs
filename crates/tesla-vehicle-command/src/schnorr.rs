// SPDX-License-Identifier: MIT OR Apache-2.0

//! Schnorr signatures over NIST P-256 with SHA-256, matching Tesla's `Tesla.SS256`.
//!
//! The scheme follows Tesla's vehicle-command `internal/schnorr` package: RFC 8235 with a
//! deterministic RFC 6979 nonce, and a length-prefixed challenge that binds the generator, the
//! nonce point, the signer public key, and the message. The ECDH session key still uses SHA-1, so
//! the two uses of the same scalar stay domain-separated.

use hmac::{Hmac, Mac};
use p256::elliptic_curve::bigint::U256;
use p256::elliptic_curve::ff::PrimeField;
use p256::elliptic_curve::ops::Reduce;
use p256::elliptic_curve::sec1::ToSec1Point;
use p256::{AffinePoint, Scalar, SecretKey};
#[cfg(test)]
use p256::{ProjectivePoint, PublicKey};
use sha2::{Digest, Sha256};

use crate::CommandError;
use crate::crypto::uncompressed_public;

type HmacSha256 = Hmac<Sha256>;

const SCALAR_LEN: usize = 32;

/// Signs `message` with a 32-byte big-endian P-256 scalar.
///
/// The result is `x || y || r` (96 bytes). `x || y` is the nonce point without the `0x04` prefix.
pub(crate) fn sign(scalar: &[u8; SCALAR_LEN], message: &[u8]) -> Result<[u8; 96], CommandError> {
    let digest = sha256(message);
    let nonce = deterministic_nonce(scalar, digest)?;
    let nonce_key = SecretKey::from_slice(&nonce).map_err(|_| CommandError::Signing)?;
    let public_nonce = uncompressed_public(&nonce_key);
    let sender = public_uncompressed(scalar)?;
    let challenge = challenge(&public_nonce, &sender, message);
    let v = scalar_from_bytes(&nonce)?;
    let a = scalar_from_bytes(scalar)?;
    let c = Scalar::reduce(&U256::from_be_slice(&challenge));
    let r = v - (a * c);
    let mut signature = [0_u8; 96];
    signature[..64].copy_from_slice(&public_nonce[1..]);
    signature[64..].copy_from_slice(r.to_bytes().as_slice());
    Ok(signature)
}

/// Verifies a signature produced by [`sign`].
#[cfg(test)]
pub(crate) fn verify(
    public_key: &[u8],
    message: &[u8],
    signature: &[u8],
) -> Result<(), CommandError> {
    if signature.len() != 96 {
        return Err(CommandError::InvalidMac);
    }
    let mut nonce_point = [0_u8; 65];
    nonce_point[0] = 0x04;
    nonce_point[1..].copy_from_slice(&signature[..64]);
    let expected =
        PublicKey::from_sec1_bytes(&nonce_point).map_err(|_| CommandError::InvalidMac)?;
    let signer =
        PublicKey::from_sec1_bytes(public_key).map_err(|_| CommandError::InvalidRemoteKey)?;
    let challenge = challenge(&nonce_point, public_key, message);
    let c = Scalar::reduce(&U256::from_be_slice(&challenge));
    let r = Scalar::reduce(&U256::from_be_slice(&signature[64..]));
    let computed = signer.to_projective() * c + ProjectivePoint::GENERATOR * r;
    let encoded = computed.to_sec1_point(false);
    if encoded.as_bytes() == nonce_point.as_slice()
        && encoded.as_bytes() == expected.to_sec1_point(false).as_bytes()
    {
        Ok(())
    } else {
        Err(CommandError::InvalidMac)
    }
}

fn public_uncompressed(scalar: &[u8; SCALAR_LEN]) -> Result<[u8; 65], CommandError> {
    let key = SecretKey::from_slice(scalar).map_err(|_| CommandError::Signing)?;
    Ok(uncompressed_public(&key))
}

fn scalar_from_bytes(bytes: &[u8; SCALAR_LEN]) -> Result<Scalar, CommandError> {
    let mut array = [0_u8; SCALAR_LEN];
    array.copy_from_slice(bytes);
    Scalar::from_repr(array.into())
        .into_option()
        .ok_or(CommandError::Signing)
}

/// RFC 6979 nonce for P-256/SHA-256. The reduced hash is 32 bytes, matching Tesla.
fn deterministic_nonce(
    scalar: &[u8; SCALAR_LEN],
    message_hash: [u8; 32],
) -> Result<[u8; SCALAR_LEN], CommandError> {
    let h1 = bits2octets(&message_hash);
    let mut k = [0_u8; 32];
    let mut v = [0x01_u8; 32];
    k = hmac(&k, &concat_parts(&[&v, &[0x00], scalar, &h1]))?;
    v = hmac(&k, &v)?;
    k = hmac(&k, &concat_parts(&[&v, &[0x01], scalar, &h1]))?;
    v = hmac(&k, &v)?;
    for _ in 0..64 {
        v = hmac(&k, &v)?;
        if SecretKey::from_slice(&v).is_ok() {
            return Ok(v);
        }
        k = hmac(&k, &concat_parts(&[&v, &[0x00]]))?;
        v = hmac(&k, &v)?;
    }
    Err(CommandError::Signing)
}

fn bits2octets(hash: &[u8; 32]) -> [u8; 32] {
    // Tesla reduces the hash modulo the subgroup order and encodes it at the
    // order's announced width, which is 32 bytes for P-256. Leading zeros stay.
    let reduced = Scalar::reduce(&U256::from_be_slice(hash));
    let mut out = [0_u8; 32];
    out.copy_from_slice(reduced.to_bytes().as_slice());
    out
}

fn hmac(key: &[u8], data: &[u8]) -> Result<[u8; 32], CommandError> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).map_err(|_| CommandError::Signing)?;
    mac.update(data);
    let bytes = mac.finalize().into_bytes();
    let mut out = [0_u8; 32];
    out.copy_from_slice(&bytes);
    Ok(out)
}

fn concat_parts(parts: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for part in parts {
        out.extend_from_slice(part);
    }
    out
}

fn challenge(public_nonce: &[u8], sender: &[u8], message: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    write_lv(&mut hasher, generator());
    write_lv(&mut hasher, public_nonce);
    write_lv(&mut hasher, sender);
    write_lv(&mut hasher, message);
    sha256_finalize(hasher.finalize())
}

fn generator() -> &'static [u8] {
    use std::sync::OnceLock;
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| {
        AffinePoint::GENERATOR
            .to_sec1_point(false)
            .as_bytes()
            .to_vec()
    })
}

fn write_lv(hasher: &mut Sha256, bytes: &[u8]) {
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    hasher.update(len.to_be_bytes());
    hasher.update(bytes);
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    sha256_finalize(Sha256::digest(bytes))
}

fn sha256_finalize(digest: impl AsRef<[u8]>) -> [u8; 32] {
    let mut out = [0_u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}

#[cfg(test)]
mod tests {
    use super::{deterministic_nonce, sign, verify};
    use crate::crypto::uncompressed_public;
    use p256::SecretKey;
    use sha2::{Digest, Sha256};

    fn test_scalar() -> [u8; 32] {
        let mut scalar = [0_u8; 32];
        scalar[0] = 3;
        scalar
    }

    #[test]
    fn rfc6979_nonce_matches_the_p256_sha256_sample() {
        let scalar = [
            0xc9, 0xaf, 0xa9, 0xd8, 0x45, 0xba, 0x75, 0x16, 0x6b, 0x5c, 0x21, 0x57, 0x67, 0xb1,
            0xd6, 0x93, 0x4e, 0x50, 0xc3, 0xdb, 0x36, 0xe8, 0x9b, 0x12, 0x7b, 0x8a, 0x62, 0x2b,
            0x12, 0x0f, 0x67, 0x21,
        ];
        let expected = [
            0xa6, 0xe3, 0xc5, 0x7d, 0xd0, 0x1a, 0xbe, 0x90, 0x08, 0x65, 0x38, 0x39, 0x83, 0x55,
            0xdd, 0x4c, 0x3b, 0x17, 0xaa, 0x87, 0x33, 0x82, 0xb0, 0xf2, 0x4d, 0x61, 0x29, 0x49,
            0x3d, 0x8a, 0xad, 0x60,
        ];
        let hash = Sha256::digest(b"sample");
        let mut digest = [0_u8; 32];
        digest.copy_from_slice(&hash);
        let nonce = deterministic_nonce(&scalar, digest).expect("nonce");
        assert_eq!(nonce, expected);
    }

    #[test]
    fn rejection_sampling_matches_tesla_vector() {
        let digest = [
            0x00, 0x80, 0xc3, 0x68, 0x64, 0xc5, 0xf2, 0xf4, 0x60, 0xe3, 0x76, 0x79, 0x83, 0xc6,
            0x56, 0x77, 0xb6, 0x5c, 0xef, 0x90, 0x1b, 0xce, 0xdc, 0xb2, 0x23, 0xf9, 0xb3, 0x65,
            0xc6, 0x8f, 0x52, 0xf6,
        ];
        let expected = [
            0x26, 0x4f, 0xc6, 0x59, 0x2f, 0xbe, 0xa2, 0x4f, 0xd0, 0x95, 0x4e, 0x0b, 0x86, 0xb8,
            0x86, 0xe8, 0x74, 0x31, 0x61, 0x75, 0x8d, 0xda, 0xd2, 0xf7, 0xe9, 0xfe, 0xd7, 0x5a,
            0x00, 0x19, 0xe0, 0x05,
        ];
        let nonce = deterministic_nonce(&test_scalar(), digest).expect("nonce");
        assert_eq!(nonce, expected);
        assert!(SecretKey::from_slice(&nonce).is_ok());
    }

    #[test]
    fn signature_matches_tesla_hello_world_vector() {
        let signature = sign(&test_scalar(), b"hello world").expect("sign");
        let expected = [
            0x7c, 0xfd, 0xbe, 0xb5, 0xba, 0xa7, 0x30, 0x54, 0x04, 0x01, 0x55, 0x0b, 0xde, 0xfa,
            0x20, 0x97, 0x64, 0x53, 0xe8, 0x53, 0x9a, 0xe4, 0xb2, 0xf2, 0x6c, 0xe3, 0x31, 0x25,
            0x80, 0x1a, 0x08, 0xf9, 0x0e, 0xd2, 0x0c, 0x3d, 0x84, 0x64, 0x97, 0xff, 0x82, 0xcc,
            0x97, 0x72, 0xe3, 0xdb, 0x47, 0x03, 0x98, 0x2f, 0x47, 0xbd, 0x0b, 0x0b, 0x89, 0xdf,
            0xb9, 0xa4, 0x9c, 0xd2, 0xe5, 0x24, 0x05, 0x46, 0x02, 0xb1, 0xe0, 0x5f, 0xbf, 0x95,
            0xf5, 0x68, 0x6f, 0xae, 0xa7, 0xa5, 0x80, 0x9e, 0xb9, 0x2f, 0x5e, 0xcc, 0x22, 0xea,
            0xe7, 0x4c, 0xec, 0xcc, 0x5e, 0x2a, 0x65, 0xdd, 0x67, 0xff, 0x20, 0xfc,
        ];
        assert_eq!(signature.as_slice(), expected.as_slice());
        let key = SecretKey::from_slice(&test_scalar()).expect("key");
        let public = uncompressed_public(&key);
        assert!(verify(&public, b"hello world", &signature).is_ok());
        assert!(verify(&public, b"hello worle", &signature).is_err());
    }
}
