// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Server-side status for the Tesla connection screen.
//!
//! Colours are not the status. Each component has a short label and a sentence.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};

use crate::qr;

const EXPIRING: chrono::Duration = chrono::Duration::minutes(60);

/// Cached partner and virtual-key answers. Tesla is not called on every render.
pub(crate) struct LinkCache {
    pub partner: PartnerState,
    pub partner_at: Option<Instant>,
    pub pairing: BTreeMap<String, KeyState>,
    pub pairing_at: Option<Instant>,
}

impl LinkCache {
    pub(crate) fn unknown() -> Self {
        Self {
            partner: PartnerState::Unknown {
                message: "Pas encore vérifié.".to_owned(),
            },
            partner_at: None,
            pairing: BTreeMap::new(),
            pairing_at: None,
        }
    }

    pub(crate) fn partner_fresh(&self, ttl: Duration) -> bool {
        self.partner_at.is_some_and(|at| at.elapsed() < ttl)
    }

    pub(crate) fn pairing_fresh(&self, ttl: Duration) -> bool {
        self.pairing_at.is_some_and(|at| at.elapsed() < ttl)
    }
}

/// Result of `GET /api/1/partner_accounts/public_key`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PartnerState {
    Registered,
    DifferentKey,
    Missing,
    Unknown { message: String },
}

/// Whether this application's virtual key is on the car.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyState {
    Paired,
    Unpaired,
    Unknown,
}

/// Facts the page is allowed to show. No tokens and no private key.
pub(crate) struct Input {
    pub now: DateTime<Utc>,
    pub fleet_present: bool,
    pub fleet_readable: bool,
    pub fleet_expires: Option<DateTime<Utc>>,
    pub partner: PartnerState,
    pub public_key_published: bool,
    pub domain: String,
    pub telemetry_enabled: bool,
    pub fresh_for: Duration,
    pub is_admin: bool,
    pub vehicles: Vec<VehicleInput>,
}

pub(crate) struct VehicleInput {
    pub name: String,
    pub key: KeyState,
    pub skip_reason: Option<String>,
    pub config_pushed: bool,
    pub last_message_age: Option<Duration>,
}

/// Text the connection page prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Report {
    pub fleet_state: String,
    pub fleet_tone: &'static str,
    pub fleet_detail: String,
    pub partner_state: String,
    pub partner_tone: &'static str,
    pub partner_detail: String,
    pub public_key_state: String,
    pub public_key_tone: &'static str,
    pub public_key_detail: String,
    pub pairing_url: String,
    pub pairing_qr: String,
    pub vehicles: Vec<VehicleLink>,
    pub data_source: String,
    pub telemetry_note: String,
    pub show_partner_action: bool,
    pub show_telemetry_action: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VehicleLink {
    pub name: String,
    pub key_state: String,
    pub config_state: String,
    pub stream_state: String,
    pub source: String,
}

impl Report {
    #[cfg(test)]
    pub(crate) fn blank() -> Self {
        build(&Input {
            now: Utc::now(),
            fleet_present: false,
            fleet_readable: true,
            fleet_expires: None,
            partner: PartnerState::Unknown {
                message: "Pas encore vérifié.".to_owned(),
            },
            public_key_published: true,
            domain: "logger.example".to_owned(),
            telemetry_enabled: false,
            fresh_for: Duration::from_secs(900),
            is_admin: false,
            vehicles: Vec::new(),
        })
    }
}

pub(crate) fn build(input: &Input) -> Report {
    let (fleet_state, fleet_tone, fleet_detail) = fleet_text(input);
    let (partner_state, partner_tone, partner_detail) = partner_text(&input.partner);
    let (public_key_state, public_key_tone, public_key_detail) = if input.public_key_published {
        (
            "Publiée".to_owned(),
            "ok",
            "Ce logger sert la clé à /.well-known/appspecific/com.tesla.3p.public-key.pem."
                .to_owned(),
        )
    } else {
        (
            "Absente".to_owned(),
            "bad",
            "La clé privée n'est pas lisible, donc la clé publique n'est pas servie.".to_owned(),
        )
    };
    let pairing_url = pairing_url(&input.domain).unwrap_or_default();
    let pairing_qr = if pairing_url.is_empty() {
        String::new()
    } else {
        qr::svg_labeled(&pairing_url, "Code QR pour ajouter la clé virtuelle").unwrap_or_default()
    };
    let vehicles = input
        .vehicles
        .iter()
        .map(|vehicle| vehicle_link(vehicle, input.telemetry_enabled, input.fresh_for))
        .collect::<Vec<_>>();
    let data_source = if vehicles
        .iter()
        .any(|vehicle| vehicle.source == "Télémétrie")
    {
        "Télémétrie".to_owned()
    } else if input.telemetry_enabled {
        "Secours (interrogation)".to_owned()
    } else {
        "Interrogation".to_owned()
    };
    let telemetry_note = if input.telemetry_enabled {
        "La configuration est signée puis envoyée. Un message récent fait de la télémétrie la source. Sinon le polling Fleet reprend.".to_owned()
    } else {
        "La télémétrie n'est pas activée. Le polling Fleet reste la source.".to_owned()
    };
    Report {
        fleet_state,
        fleet_tone,
        fleet_detail,
        partner_state,
        partner_tone,
        partner_detail,
        public_key_state,
        public_key_tone,
        public_key_detail,
        pairing_url,
        pairing_qr,
        vehicles,
        data_source,
        telemetry_note,
        show_partner_action: input.is_admin,
        show_telemetry_action: input.telemetry_enabled,
    }
}

pub(crate) fn pairing_url(domain: &str) -> Option<String> {
    let domain = domain.trim().to_ascii_lowercase();
    if domain.is_empty()
        || domain.contains('/')
        || domain.contains(':')
        || domain.contains(' ')
        || domain.contains('@')
    {
        return None;
    }
    Some(format!("https://tesla.com/_ak/{domain}"))
}

fn fleet_text(input: &Input) -> (String, &'static str, String) {
    if !input.fleet_readable {
        return (
            "Illisible".to_owned(),
            "bad",
            "Le jeton Fleet stocké ne peut pas être lu.".to_owned(),
        );
    }
    if !input.fleet_present {
        return (
            "Absent".to_owned(),
            "bad",
            "Aucun jeton Fleet. Connectez le compte Tesla.".to_owned(),
        );
    }
    if input
        .fleet_expires
        .is_some_and(|expires| expires <= input.now)
    {
        return (
            "Expiré".to_owned(),
            "bad",
            "Le jeton Fleet est expiré. Rafraîchissez-le.".to_owned(),
        );
    }
    if input
        .fleet_expires
        .is_some_and(|expires| expires <= input.now + EXPIRING)
    {
        return (
            "Expire bientôt".to_owned(),
            "warn",
            "Le jeton Fleet expire dans moins d'une heure.".to_owned(),
        );
    }
    (
        "Valide".to_owned(),
        "ok",
        "Le jeton Fleet est utilisable.".to_owned(),
    )
}

fn partner_text(state: &PartnerState) -> (String, &'static str, String) {
    match state {
        PartnerState::Registered => (
            "Enregistré".to_owned(),
            "ok",
            "Tesla connaît ce domaine.".to_owned(),
        ),
        PartnerState::DifferentKey => (
            "Autre clé".to_owned(),
            "bad",
            "Tesla a une clé différente pour ce domaine.".to_owned(),
        ),
        PartnerState::Missing => (
            "Pas enregistré".to_owned(),
            "bad",
            "Le domaine n'est pas enregistré chez Tesla.".to_owned(),
        ),
        PartnerState::Unknown { message } => ("Inconnu".to_owned(), "warn", message.clone()),
    }
}

fn vehicle_link(
    vehicle: &VehicleInput,
    telemetry_enabled: bool,
    fresh_for: Duration,
) -> VehicleLink {
    let key_state = match vehicle.key {
        KeyState::Paired => "Appariée".to_owned(),
        KeyState::Unpaired => "Pas appariée".to_owned(),
        KeyState::Unknown => "Inconnu".to_owned(),
    };
    let config_state = if let Some(reason) = vehicle.skip_reason.as_deref() {
        format!("Ignorée : {}", skip_label(reason))
    } else if vehicle.config_pushed {
        "Envoyée".to_owned()
    } else if telemetry_enabled {
        "Pas envoyée".to_owned()
    } else {
        "Non demandée".to_owned()
    };
    let source = if telemetry_enabled {
        match crate::telemetry_config::live_data_source(vehicle.last_message_age, fresh_for) {
            "telemetry" => "Télémétrie",
            _ => "Secours (interrogation)",
        }
    } else {
        "Interrogation"
    };
    VehicleLink {
        name: vehicle.name.clone(),
        key_state,
        config_state,
        stream_state: stream_label(vehicle.last_message_age),
        source: source.to_owned(),
    }
}

fn skip_label(reason: &str) -> &str {
    match reason {
        "missing_key" => "clé virtuelle absente",
        "unsupported_hardware" => "matériel non pris en charge",
        "unsupported_firmware" => "logiciel trop ancien",
        "max_configs" => "trop de configurations",
        other => other,
    }
}

fn stream_label(age: Option<Duration>) -> String {
    match age {
        None => "Aucun message reçu".to_owned(),
        Some(age) if age.as_secs() < 60 => format!("Reçu il y a {} s", age.as_secs()),
        Some(age) if age.as_secs() < 3600 => format!("Reçu il y a {} min", age.as_secs() / 60),
        Some(age) => format!("Reçu il y a {} h", age.as_secs() / 3600),
    }
}

#[cfg(test)]
mod tests {
    use super::{Input, KeyState, PartnerState, VehicleInput, build, pairing_url};
    use chrono::{Duration as ChronoDuration, Utc};
    use std::time::Duration;

    fn input() -> Input {
        Input {
            now: Utc::now(),
            fleet_present: true,
            fleet_readable: true,
            fleet_expires: Some(Utc::now() + ChronoDuration::hours(2)),
            partner: PartnerState::Missing,
            public_key_published: true,
            domain: "Logger.Example".to_owned(),
            telemetry_enabled: true,
            fresh_for: Duration::from_secs(900),
            is_admin: true,
            vehicles: vec![VehicleInput {
                name: "Faucon".to_owned(),
                key: KeyState::Unpaired,
                skip_reason: Some("missing_key".to_owned()),
                config_pushed: false,
                last_message_age: None,
            }],
        }
    }

    #[test]
    fn status_names_each_gap_and_the_fallback_source() {
        let report = build(&input());
        assert_eq!(report.fleet_state, "Valide");
        assert_eq!(report.partner_state, "Pas enregistré");
        assert_eq!(report.public_key_state, "Publiée");
        assert_eq!(report.pairing_url, "https://tesla.com/_ak/logger.example");
        assert!(report.pairing_qr.contains("<svg"));
        assert!(!report.pairing_qr.contains("https://"));
        assert_eq!(report.vehicles[0].key_state, "Pas appariée");
        assert_eq!(
            report.vehicles[0].config_state,
            "Ignorée : clé virtuelle absente"
        );
        assert_eq!(report.vehicles[0].stream_state, "Aucun message reçu");
        assert_eq!(report.vehicles[0].source, "Secours (interrogation)");
        assert_eq!(report.data_source, "Secours (interrogation)");
        assert!(report.show_partner_action);
        assert!(report.show_telemetry_action);
    }

    #[test]
    fn a_recent_message_is_telemetry_and_an_expiring_token_is_named() {
        let mut facts = input();
        facts.fleet_expires = Some(facts.now + ChronoDuration::minutes(10));
        facts.vehicles[0].skip_reason = None;
        facts.vehicles[0].config_pushed = true;
        facts.vehicles[0].key = KeyState::Paired;
        facts.vehicles[0].last_message_age = Some(Duration::from_secs(30));
        let report = build(&facts);
        assert_eq!(report.fleet_state, "Expire bientôt");
        assert_eq!(report.fleet_tone, "warn");
        assert_eq!(report.vehicles[0].config_state, "Envoyée");
        assert_eq!(report.vehicles[0].source, "Télémétrie");
        assert_eq!(report.data_source, "Télémétrie");
        assert!(report.vehicles[0].stream_state.contains("Reçu"));
    }

    #[test]
    fn empty_and_unreadable_states_stay_explicit() {
        let mut facts = input();
        facts.fleet_present = false;
        facts.fleet_readable = false;
        facts.public_key_published = false;
        facts.telemetry_enabled = false;
        facts.is_admin = false;
        facts.vehicles.clear();
        facts.partner = PartnerState::Unknown {
            message: "Pas encore vérifié.".to_owned(),
        };
        let report = build(&facts);
        assert_eq!(report.fleet_state, "Illisible");
        assert_eq!(report.public_key_state, "Absente");
        assert_eq!(report.partner_state, "Inconnu");
        assert_eq!(report.vehicles.len(), 0);
        assert_eq!(report.data_source, "Interrogation");
        assert!(!report.show_partner_action);
        assert!(!report.show_telemetry_action);
        assert!(report.telemetry_note.contains("n'est pas activée"));
        assert!(pairing_url("https://evil.example").is_none());
    }
}
