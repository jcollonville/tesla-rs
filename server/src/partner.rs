// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! `POST /api/1/partner_accounts` for the application domain.
//!
//! Tesla fetches the public key from `https://{domain}/.well-known/appspecific/com.tesla.3p.public-key.pem`.
//! This module sends the domain only. The private key is never logged or sent.

use tesla_auth::{AuthClient, ClientConfig, ExposeSecret, Scope};
use tesla_fleet_api::{BearerToken, FleetClient, FleetError, RegisterPartnerAccount};
use tesla_vehicle_command::PrivateKey;

use crate::config::Config;
use crate::connection::PartnerState;
use crate::error::AppError;

/// Registers `TESLA_DOMAIN` with a partner token.
///
/// # Errors
///
/// Returns a short message when the token or the registration call fails. The message does not
/// include the client secret, the partner token, or the private key.
pub(crate) async fn register(config: &Config) -> Result<(), AppError> {
    let domain = partner_domain(&config.domain)?;
    let client = partner_client(config).await?;
    client
        .register_partner_account(&RegisterPartnerAccount { domain, csr: None })
        .await
        .map_err(|err| fleet_message(&err))?;
    Ok(())
}

/// Compares Tesla's stored key with the local public key.
///
/// # Errors
///
/// Returns a short message when the partner token or the private key cannot be used.
pub(crate) async fn lookup(config: &Config) -> Result<PartnerState, AppError> {
    let domain = partner_domain(&config.domain)?;
    let client = partner_client(config).await?;
    let remote = match client.partner_public_key(&domain).await {
        Ok(key) => key.public_key,
        Err(FleetError::NotFound { .. } | FleetError::PreconditionFailed { .. }) => {
            return Ok(PartnerState::Missing);
        }
        Err(err) => return Err(fleet_message(&err)),
    };
    let local = local_public_hex(&config.private_key_path)?;
    if keys_match(&remote, &local) {
        Ok(PartnerState::Registered)
    } else {
        Ok(PartnerState::DifferentKey)
    }
}

pub(crate) fn partner_domain(domain: &str) -> Result<String, AppError> {
    let domain = domain.trim().to_ascii_lowercase();
    if domain.is_empty()
        || domain.contains('/')
        || domain.contains(':')
        || domain.contains(' ')
        || !domain.contains('.')
    {
        return Err(AppError::message(
            "TESLA_DOMAIN must be a lowercase hostname",
        ));
    }
    Ok(domain)
}

async fn partner_client(config: &Config) -> Result<FleetClient<BearerToken>, AppError> {
    let auth = AuthClient::new(ClientConfig {
        client_id: config.client_id.clone(),
        client_secret: config.client_secret.clone(),
        redirect_uri: config.redirect_uri.clone(),
        region: config.region,
        scopes: Scope::third_party_vehicle(),
    })
    .map_err(|_| AppError::message("partner token client could not be built"))?;
    let tokens = auth
        .partner_token(&Scope::third_party_vehicle())
        .await
        .map_err(|_| AppError::message("Tesla refused the partner token"))?;
    FleetClient::for_region(
        config.region,
        BearerToken::new(tokens.access_token().expose_secret()),
    )
    .map_err(|_| AppError::message("fleet client could not be built"))
}

fn local_public_hex(path: &std::path::Path) -> Result<String, AppError> {
    let key = PrivateKey::read_file(path)
        .map_err(|_| AppError::message("application private key could not be read"))?;
    Ok(hex::encode(key.public_key_uncompressed()))
}

fn keys_match(remote: &str, local_uncompressed_hex: &str) -> bool {
    let remote: String = remote
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(|ch| ch.to_lowercase())
        .collect();
    let local = local_uncompressed_hex.to_ascii_lowercase();
    let raw = local.strip_prefix("04").unwrap_or(&local);
    remote == local || remote == raw
}

fn fleet_message(err: &FleetError) -> AppError {
    let text = match err {
        FleetError::Unauthorized { .. } => "Tesla refused the partner token".to_owned(),
        FleetError::Forbidden { .. } => "Tesla refused partner registration".to_owned(),
        FleetError::NotFound { .. } => "partner account was not found".to_owned(),
        FleetError::PreconditionFailed { .. } => "partner registration was rejected".to_owned(),
        other => crate::logs::redact_line(&other.to_string(), &[]),
    };
    AppError::message(text)
}

#[cfg(test)]
mod tests {
    use super::{keys_match, partner_domain};

    #[test]
    fn domain_is_lowercased_and_rejected_when_it_is_a_url() {
        assert_eq!(partner_domain("Logger.Example").unwrap(), "logger.example");
        assert!(partner_domain("https://logger.example").is_err());
        assert!(partner_domain("localhost").is_err());
    }

    #[test]
    fn public_key_compare_accepts_the_uncompressed_point_with_or_without_the_prefix() {
        let local = "04abcd";
        assert!(keys_match("04ABCD", local));
        assert!(keys_match("abcd", local));
        assert!(keys_match("04 ab cd", local));
        assert!(!keys_match("ffff", local));
    }
}
