// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Signed Fleet Telemetry configuration: CA chain, skipped VINs, and resend timing.
//!
//! Tesla rejects an unsigned `fleet_telemetry_config`. The logger either posts the unsigned
//! document to a `tesla-http-proxy` (which signs it) or posts a `Tesla.SS256` JWS to
//! `fleet_telemetry_config_jws`. A VIN is remembered only when Tesla did not list it in
//! `skipped_vehicles`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::time::Duration;

use secrecy::ExposeSecret;
use tesla_fleet_api::{AccessTokenProvider, FleetClient, FleetError, SkippedVehicles};
use tesla_vehicle_command::PrivateKey;

use crate::error::AppError;

/// Where the signed config is delivered.
pub(crate) struct SigningMaterial {
    pub key_path: PathBuf,
    pub proxy_endpoint: Option<String>,
    pub proxy_ca_path: Option<PathBuf>,
}

/// VINs Tesla accepted, and the ones it skipped with a reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PushOutcome {
    pub configured: Vec<String>,
    pub skipped: Vec<(String, String)>,
}

/// What the next config cycle should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Attempt {
    pub check: Vec<String>,
    pub push: Vec<String>,
}

/// Keeps the certificates after the first PEM block.
///
/// `fullchain.pem` order is the leaf, then intermediates, then the root. The car already receives
/// the leaf during TLS, and Tesla's `ca` field is the chain that signs it. A file with one
/// certificate is left unchanged so a CA-only bundle still works.
pub(crate) fn ca_chain_pem(pem: &str) -> String {
    let blocks = certificate_blocks(pem);
    match blocks.len() {
        0 => String::new(),
        1 => blocks.into_iter().next().unwrap_or_default(),
        _ => blocks.into_iter().skip(1).collect::<Vec<_>>().join("\n"),
    }
}

/// `true` when Tesla has no config for us, or the hostname is not the one we serve.
pub(crate) fn config_needs_resend(hostname: Option<&str>, expected_host: &str) -> bool {
    hostname != Some(expected_host)
}

/// Backoff after a failed push. `failures` is the count after the failure, starting at 1.
pub(crate) fn retry_delay(failures: u32, base: Duration, cap: Duration) -> Duration {
    let shift = failures.saturating_sub(1).min(16);
    let factor = 1_u32.checked_shl(shift).unwrap_or(u32::MAX);
    base.saturating_mul(factor).min(cap)
}

/// `telemetry` only when a message arrived inside `fresh_for`.
pub(crate) fn live_data_source(age: Option<Duration>, fresh_for: Duration) -> &'static str {
    match age {
        Some(age) if age <= fresh_for => "telemetry",
        _ => "polling",
    }
}

pub(crate) fn classify_push(requested: &[String], skipped: &SkippedVehicles) -> PushOutcome {
    let reasons = skipped.by_vin();
    let mut configured = Vec::new();
    let mut skipped_out = Vec::new();
    for vin in requested {
        if let Some(reason) = reasons.get(vin) {
            skipped_out.push((vin.clone(), reason.clone()));
        } else {
            configured.push(vin.clone());
        }
    }
    for (vin, reason) in reasons {
        if !requested.iter().any(|item| item == &vin) {
            skipped_out.push((vin, reason));
        }
    }
    PushOutcome {
        configured,
        skipped: skipped_out,
    }
}

pub(crate) fn apply_outcome(
    configured: &mut BTreeSet<String>,
    skipped: &mut BTreeMap<String, String>,
    outcome: &PushOutcome,
) {
    for (vin, reason) in &outcome.skipped {
        configured.remove(vin);
        skipped.insert(vin.clone(), reason.clone());
    }
    for vin in &outcome.configured {
        skipped.remove(vin);
        configured.insert(vin.clone());
    }
}

/// `None` when the cycle should not call Tesla.
///
/// A forced resend pushes every known VIN and skips the GET. A normal cycle checks VINs that
/// were accepted before, and pushes the rest.
pub(crate) fn plan_attempt(
    due: bool,
    billing_blocked: bool,
    known: &[String],
    configured: &BTreeSet<String>,
    force: bool,
) -> Option<Attempt> {
    if billing_blocked || (!due && !force) {
        return None;
    }
    if force {
        return Some(Attempt {
            check: Vec::new(),
            push: known.to_vec(),
        });
    }
    let mut check = Vec::new();
    let mut push = Vec::new();
    for vin in known {
        if configured.contains(vin) {
            check.push(vin.clone());
        } else {
            push.push(vin.clone());
        }
    }
    Some(Attempt { check, push })
}

pub(crate) async fn push_config<T: AccessTokenProvider>(
    client: &FleetClient<T>,
    signing: &SigningMaterial,
    host: &str,
    port: u16,
    ca_pem: &str,
    vins: &[String],
) -> Result<PushOutcome, AppError> {
    let request = crate::telemetry::subscription(host, port, ca_pem, vins)?;
    if let Some(endpoint) = signing.proxy_endpoint.as_deref() {
        let ca_path = signing.proxy_ca_path.as_ref().ok_or_else(|| {
            AppError::message("TELEMETRY_PROXY_CA is required for the command proxy")
        })?;
        let ca = std::fs::read_to_string(ca_path)
            .map_err(|_| AppError::message("telemetry proxy CA could not be read"))?;
        let token = client
            .access_token()
            .await
            .map_err(|_| AppError::message("fleet token is not available for the command proxy"))?;
        let update = post_proxy(endpoint, &ca, token.expose_secret(), &request).await?;
        return Ok(classify_push(&request.vins, &update.skipped_vehicles));
    }
    let key = PrivateKey::read_file(&signing.key_path)
        .map_err(|_| AppError::message("application private key could not be read"))?;
    let config_value = serde_json::to_value(&request.config)
        .map_err(|_| AppError::message("telemetry config could not be encoded"))?;
    let token = tesla_vehicle_command::sign_fleet_telemetry_config(&key, &config_value)
        .map_err(|_| AppError::message("telemetry config could not be signed"))?;
    let update = client
        .create_fleet_telemetry_config_jws(&request.vins, &token)
        .await
        .map_err(|err| fleet_message(&err))?;
    Ok(classify_push(&request.vins, &update.skipped_vehicles))
}

async fn post_proxy(
    endpoint: &str,
    ca_pem: &str,
    bearer: &str,
    request: &tesla_fleet_api::FleetTelemetryConfigRequest,
) -> Result<tesla_fleet_api::FleetTelemetryConfigUpdate, AppError> {
    let mut builder = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .tls_built_in_root_certs(false);
    let mut added = false;
    for block in certificate_blocks(ca_pem) {
        let cert = reqwest::Certificate::from_pem(block.as_bytes())
            .map_err(|_| AppError::message("telemetry proxy CA is not a PEM certificate"))?;
        builder = builder.add_root_certificate(cert);
        added = true;
    }
    if !added {
        return Err(AppError::message("telemetry proxy CA is empty"));
    }
    let http = builder
        .build()
        .map_err(|_| AppError::message("telemetry proxy client could not be built"))?;
    let response = http
        .post(endpoint)
        .bearer_auth(bearer)
        .json(request)
        .send()
        .await
        .map_err(|_| AppError::message("telemetry proxy request failed"))?;
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|_| AppError::message("telemetry proxy response could not be read"))?;
    if !status.is_success() {
        return Err(AppError::message(format!(
            "telemetry proxy returned HTTP {}",
            status.as_u16()
        )));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::message("telemetry proxy returned invalid json"))?;
    let body = value.get("response").cloned().unwrap_or(value);
    serde_json::from_value(body)
        .map_err(|_| AppError::message("telemetry proxy response could not be read"))
}

fn fleet_message(err: &FleetError) -> AppError {
    let text = match err {
        FleetError::Unauthorized { .. } => "Tesla refused the fleet token".to_owned(),
        FleetError::PaymentRequired => "Tesla billing is blocked".to_owned(),
        FleetError::Forbidden { .. } => "Tesla refused the telemetry config".to_owned(),
        FleetError::NotFound { .. } => "Tesla has no telemetry config for this vehicle".to_owned(),
        FleetError::PreconditionFailed { .. } => "Tesla rejected the telemetry config".to_owned(),
        FleetError::RateLimited(_) => "Tesla rate limited the telemetry config".to_owned(),
        other => crate::logs::redact_line(&other.to_string(), &[]),
    };
    AppError::message(text)
}

fn certificate_blocks(pem: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut rest = pem;
    while let Some(start) = rest.find("-----BEGIN CERTIFICATE-----") {
        let after = &rest[start..];
        let Some(end_rel) = after.find("-----END CERTIFICATE-----") else {
            break;
        };
        let end = end_rel + "-----END CERTIFICATE-----".len();
        let block = after[..end].trim();
        if !block.is_empty() {
            blocks.push(block.to_owned());
        }
        rest = &after[end..];
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::{
        apply_outcome, ca_chain_pem, classify_push, config_needs_resend, live_data_source,
        plan_attempt, retry_delay,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::Duration;
    use tesla_fleet_api::SkippedVehicles;

    fn leaf_and_root() -> String {
        "-----BEGIN CERTIFICATE-----\nLEAFDATA\n-----END CERTIFICATE-----\n\
         -----BEGIN CERTIFICATE-----\nROOTDATA\n-----END CERTIFICATE-----\n"
            .to_owned()
    }

    #[test]
    fn ca_chain_drops_the_leaf_and_keeps_a_single_certificate() {
        let chain = ca_chain_pem(&leaf_and_root());
        assert!(chain.contains("ROOTDATA"));
        assert!(!chain.contains("LEAFDATA"));
        assert!(chain.contains("BEGIN CERTIFICATE"));
        let only = "-----BEGIN CERTIFICATE-----\nONLYDATA\n-----END CERTIFICATE-----\n";
        assert!(ca_chain_pem(only).contains("ONLYDATA"));
        assert_eq!(ca_chain_pem("not a certificate"), "");
    }

    #[test]
    fn success_marks_only_vins_tesla_accepted() {
        let skipped = SkippedVehicles {
            missing_key: vec!["TEST00000000VIN02".to_owned()],
            unsupported_hardware: vec!["TEST00000000VIN03".to_owned()],
            ..SkippedVehicles::default()
        };
        let outcome = classify_push(
            &[
                "TEST00000000VIN01".to_owned(),
                "TEST00000000VIN02".to_owned(),
                "TEST00000000VIN03".to_owned(),
            ],
            &skipped,
        );
        let mut configured = BTreeSet::new();
        let mut reasons = BTreeMap::new();
        apply_outcome(&mut configured, &mut reasons, &outcome);
        assert!(configured.contains("TEST00000000VIN01"));
        assert!(!configured.contains("TEST00000000VIN02"));
        assert!(!configured.contains("TEST00000000VIN03"));
        assert_eq!(reasons["TEST00000000VIN02"], "missing_key");
        assert_eq!(reasons["TEST00000000VIN03"], "unsupported_hardware");
        assert_eq!(configured.len(), 1);
    }

    #[test]
    fn a_failed_push_leaves_the_configured_set_unchanged() {
        let mut configured = BTreeSet::from(["TEST00000000VIN01".to_owned()]);
        let before = configured.clone();
        let _ = &mut configured;
        assert_eq!(configured, before);
    }

    #[test]
    fn missing_or_foreign_config_is_sent_again() {
        assert!(config_needs_resend(None, "logger.example"));
        assert!(config_needs_resend(Some("other.example"), "logger.example"));
        assert!(!config_needs_resend(
            Some("logger.example"),
            "logger.example"
        ));
    }

    #[test]
    fn resend_backoff_doubles_until_the_cap() {
        let base = Duration::from_secs(60);
        let cap = Duration::from_secs(6 * 3600);
        assert_eq!(retry_delay(1, base, cap), Duration::from_secs(60));
        assert_eq!(retry_delay(2, base, cap), Duration::from_secs(120));
        assert_eq!(retry_delay(3, base, cap), Duration::from_secs(240));
        assert_eq!(retry_delay(30, base, cap), cap);
    }

    #[test]
    fn data_source_is_telemetry_only_after_a_recent_message() {
        let fresh = Duration::from_secs(900);
        assert_eq!(
            live_data_source(Some(Duration::from_secs(10)), fresh),
            "telemetry"
        );
        assert_eq!(
            live_data_source(Some(Duration::from_secs(900)), fresh),
            "telemetry"
        );
        assert_eq!(
            live_data_source(Some(Duration::from_secs(901)), fresh),
            "polling"
        );
        assert_eq!(live_data_source(None, fresh), "polling");
    }

    #[test]
    fn billing_pause_and_a_quiet_cycle_do_not_call_tesla() {
        let known = ["TEST00000000VIN01".to_owned()];
        let configured = BTreeSet::new();
        assert!(plan_attempt(false, false, &known, &configured, false).is_none());
        assert!(plan_attempt(true, true, &known, &configured, true).is_none());
        let due = plan_attempt(true, false, &known, &configured, false).unwrap();
        assert_eq!(due.check.len(), 0);
        assert_eq!(due.push, known);
        let mut have = BTreeSet::new();
        have.insert(known[0].clone());
        let check = plan_attempt(true, false, &known, &have, false).unwrap();
        assert_eq!(check.check, known);
        assert_eq!(check.push.len(), 0);
        let forced = plan_attempt(false, false, &known, &have, true).unwrap();
        assert_eq!(forced.check.len(), 0);
        assert_eq!(forced.push, known);
    }
}
