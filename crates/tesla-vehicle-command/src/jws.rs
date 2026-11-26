// SPDX-License-Identifier: MIT OR Apache-2.0

//! JSON Web Signature for `POST /api/1/vehicles/fleet_telemetry_config_jws`.
//!
//! Tesla's `tesla-jws -fleet sign TelemetryClient` puts the config object in the claims, then
//! overwrites `iss` with the standard-base64 uncompressed public key and `aud` with
//! `com.tesla.fleet.TelemetryClient`. The signature algorithm is `Tesla.SS256` (Schnorr/P-256),
//! not ES256. The compact token is not a bearer credential.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use serde_json::{Map, Value};

use crate::CommandError;
use crate::key::PrivateKey;

/// Audience Tesla's fleet proxy writes for a telemetry client configuration.
pub const FLEET_TELEMETRY_AUDIENCE: &str = "com.tesla.fleet.TelemetryClient";

const HEADER: &[u8] = br#"{"alg":"Tesla.SS256","typ":"JWT"}"#;

/// Signs a fleet telemetry config object.
///
/// `config` is the `config` object (hostname, CA chain, port, fields), not the `vins` wrapper.
/// Any `iss` or `aud` already present is replaced.
///
/// # Errors
///
/// Returns [`CommandError::Signing`] when `config` is not a JSON object or the signature cannot
/// be produced. The error text does not include the private key or the token.
pub fn sign_fleet_telemetry_config(
    key: &PrivateKey,
    config: &Value,
) -> Result<String, CommandError> {
    let mut claims = config.as_object().cloned().ok_or(CommandError::Signing)?;
    claims.insert(
        "iss".to_owned(),
        Value::String(STANDARD.encode(key.public_key_uncompressed())),
    );
    claims.insert(
        "aud".to_owned(),
        Value::String(FLEET_TELEMETRY_AUDIENCE.to_owned()),
    );
    let payload = canonical_claims(&claims).map_err(|_| CommandError::Signing)?;
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(HEADER),
        URL_SAFE_NO_PAD.encode(payload)
    );
    let signature = key.sign_schnorr(signing_input.as_bytes())?;
    Ok(format!(
        "{signing_input}.{}",
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn canonical_claims(claims: &Map<String, Value>) -> Result<Vec<u8>, serde_json::Error> {
    let mut ordered = Map::new();
    let mut keys = claims.keys().cloned().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        if let Some(value) = claims.get(&key) {
            ordered.insert(key, value.clone());
        }
    }
    serde_json::to_vec(&ordered)
}

#[cfg(test)]
mod tests {
    use super::{FLEET_TELEMETRY_AUDIENCE, sign_fleet_telemetry_config};
    use crate::PrivateKey;
    use crate::schnorr::verify;
    use base64::Engine;
    use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
    use serde_json::json;

    #[test]
    fn jws_uses_tesla_ss256_and_overwrites_iss_and_aud() {
        let key = PrivateKey::generate().expect("key");
        let token = sign_fleet_telemetry_config(
            &key,
            &json!({
                "hostname": "telemetry.example.com",
                "port": 4443,
                "ca": "-----BEGIN CERTIFICATE-----\nMIIB\n-----END CERTIFICATE-----",
                "iss": "replaced",
                "aud": "replaced",
                "fields": { "Soc": { "interval_seconds": 60 } }
            }),
        )
        .expect("jws");
        let mut parts = token.split('.');
        let header = parts.next().expect("header");
        let payload = parts.next().expect("payload");
        let signature = parts.next().expect("signature");
        assert!(parts.next().is_none());
        let header_json = URL_SAFE_NO_PAD.decode(header).expect("header b64");
        assert_eq!(
            header_json,
            br#"{"alg":"Tesla.SS256","typ":"JWT"}"#.as_slice()
        );
        let claims: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).expect("payload b64"))
                .expect("claims");
        assert_eq!(claims["aud"], FLEET_TELEMETRY_AUDIENCE);
        assert_eq!(
            claims["iss"],
            STANDARD.encode(key.public_key_uncompressed())
        );
        assert_eq!(claims["hostname"], "telemetry.example.com");
        assert_ne!(claims["iss"], "replaced");
        let signing_input = format!("{header}.{payload}");
        let raw = URL_SAFE_NO_PAD.decode(signature).expect("sig");
        assert!(
            verify(
                key.public_key_uncompressed(),
                signing_input.as_bytes(),
                &raw
            )
            .is_ok()
        );
        assert!(!token.contains("BEGIN EC PRIVATE KEY"));
    }

    #[test]
    fn a_non_object_config_is_rejected() {
        let key = PrivateKey::generate().expect("key");
        assert!(sign_fleet_telemetry_config(&key, &json!("hostname")).is_err());
    }
}
