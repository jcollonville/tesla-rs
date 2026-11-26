// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Ordered setup checklist for the Tesla connection screen.
//!
//! Each step is computed here. The page prints the label in words; colour is extra.
//! Tesla documents no server endpoint that revokes a Fleet token. The consent page
//! is a link the user opens, not a call this server makes.

use std::time::Duration;

use chrono::{DateTime, Utc};
use tesla_auth::Scope;

use crate::qr;

const EXPIRING: chrono::Duration = chrono::Duration::minutes(60);
const REQUIRED_STEPS: u8 = 5;

/// Cached partner and virtual-key answers. Tesla is not called on every render.
pub(crate) struct LinkCache {
    pub partner: PartnerState,
    pub partner_at: Option<std::time::Instant>,
    pub pairing: std::collections::BTreeMap<String, KeyState>,
    pub pairing_at: Option<std::time::Instant>,
}

impl LinkCache {
    pub(crate) fn unknown() -> Self {
        Self {
            partner: PartnerState::Unknown {
                message: "Pas encore vérifié.".to_owned(),
            },
            partner_at: None,
            pairing: std::collections::BTreeMap::new(),
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
    pub scopes: Vec<String>,
    pub refresh_failed: bool,
    pub partner: PartnerState,
    pub public_key_published: bool,
    pub domain: String,
    pub redirect_uri: String,
    pub client_configured: bool,
    pub client_id: String,
    pub region: tesla_auth::Region,
    pub fingerprint: String,
    pub telemetry_enabled: bool,
    pub telemetry_port: u16,
    pub fresh_for: Duration,
    pub is_admin: bool,
    pub vehicles: Vec<VehicleInput>,
}

pub(crate) struct VehicleInput {
    pub id: i64,
    pub name: String,
    pub key: KeyState,
    pub skip_reason: Option<String>,
    pub config_pushed: bool,
    pub last_message_age: Option<Duration>,
    /// Short telemetry health chip. Empty hides it.
    pub health: String,
    pub health_tone: &'static str,
}

/// One row of the setup list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GuideStep {
    pub index: u8,
    pub title: &'static str,
    pub optional: bool,
    pub label: &'static str,
    pub tone: &'static str,
    pub detail: String,
    pub help: &'static str,
    pub action: Option<GuideAction>,
    pub manual: Option<Manual>,
    pub show_refresh: bool,
    pub cars: Vec<CarLine>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GuideAction {
    RegisterPartner,
    Connect,
    Pair { url: String, qr: String },
    ResendTelemetry,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Manual {
    pub text: &'static str,
    pub copies: Vec<CopyValue>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CopyValue {
    pub label: &'static str,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CarLine {
    pub name: String,
    pub key: String,
    pub config: String,
    pub stream: String,
    pub health: String,
    pub health_tone: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct VehicleDeletion {
    pub id: i64,
    pub name: String,
}

/// Text the connection page prints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Report {
    pub steps: Vec<GuideStep>,
    pub progress: String,
    pub show_disconnect: bool,
    pub revoke_url: Option<String>,
    pub deletions: Vec<VehicleDeletion>,
}

impl Report {
    #[cfg(test)]
    pub(crate) fn blank() -> Self {
        build(&sample_input())
    }
}

pub(crate) fn build(input: &Input) -> Report {
    let domain = display_domain(&input.domain);
    let redirect = redirect_uri(input, &domain);
    let public_key_url = public_key_url(&domain);
    let step0 = domain_step(input, &domain, &public_key_url);
    let step1 = app_step(input, &domain, &redirect, step0.validated());
    let step2 = partner_step(input, &domain, &step0, &step1);
    let step3 = account_step(input, &step2);
    let step4 = key_step(input, &domain, &step3);
    let step5 = telemetry_step(input, &step4);
    let validated = [&step0, &step1, &step2, &step3, &step4]
        .iter()
        .filter(|step| step.validated())
        .count();
    let validated = u8::try_from(validated).unwrap_or(REQUIRED_STEPS);
    Report {
        show_disconnect: step3.validated(),
        revoke_url: revoke_url(input),
        deletions: input
            .vehicles
            .iter()
            .map(|vehicle| VehicleDeletion {
                id: vehicle.id,
                name: vehicle.name.clone(),
            })
            .collect(),
        progress: progress_label(validated, REQUIRED_STEPS),
        steps: vec![step0, step1, step2, step3, step4, step5],
    }
}

impl GuideStep {
    fn validated(&self) -> bool {
        self.label == "Validé"
    }
}

pub(crate) fn pairing_url(domain: &str) -> Option<String> {
    let domain = domain.trim().to_ascii_lowercase();
    if domain.is_empty()
        || domain.contains('/')
        || domain.contains(':')
        || domain.contains(' ')
        || domain.contains('@')
        || !domain.contains('.')
    {
        return None;
    }
    Some(format!("https://tesla.com/_ak/{domain}"))
}

/// A disconnected Fleet account must not call Tesla. Polling resumes once the
/// status is no longer `disconnected`.
pub(crate) fn disconnected_poll_wait(status: &str) -> Option<Duration> {
    (status == "disconnected").then(|| Duration::from_secs(60))
}

fn progress_label(done: u8, total: u8) -> String {
    if done > 1 {
        format!("{done} étapes sur {total} validées")
    } else {
        format!("{done} étape sur {total} validée")
    }
}

fn display_domain(domain: &str) -> String {
    domain.trim().to_ascii_lowercase()
}

fn domain_ready(domain: &str) -> bool {
    pairing_url(domain).is_some()
}

fn redirect_uri(input: &Input, domain: &str) -> String {
    let configured = input.redirect_uri.trim();
    if !configured.is_empty() {
        return configured.to_owned();
    }
    if domain_ready(domain) {
        format!("https://{domain}/auth/callback")
    } else {
        String::new()
    }
}

fn public_key_url(domain: &str) -> String {
    if domain_ready(domain) {
        format!("https://{domain}/.well-known/appspecific/com.tesla.3p.public-key.pem")
    } else {
        String::new()
    }
}

fn revoke_url(input: &Input) -> Option<String> {
    let client_id = input.client_id.trim();
    if client_id.is_empty() || !domain_ready(&input.domain) {
        return None;
    }
    let domain = display_domain(&input.domain);
    let back = format!("https://{domain}/connection");
    input
        .region
        .revoke_consent_url(client_id, &back)
        .ok()
        .map(|url| url.to_string())
}

fn done(index: u8, title: &'static str, help: &'static str, detail: String) -> GuideStep {
    GuideStep {
        index,
        title,
        optional: false,
        label: "Validé",
        tone: "ok",
        detail,
        help,
        action: None,
        manual: None,
        show_refresh: false,
        cars: Vec::new(),
    }
}

struct Open {
    label: &'static str,
    tone: &'static str,
    detail: String,
    action: Option<GuideAction>,
    manual: Option<Manual>,
}

fn open_step(index: u8, title: &'static str, help: &'static str, open: Open) -> GuideStep {
    GuideStep {
        index,
        title,
        optional: false,
        label: open.label,
        tone: open.tone,
        detail: open.detail,
        help,
        action: open.action,
        manual: open.manual,
        show_refresh: false,
        cars: Vec::new(),
    }
}

fn gap(
    label: &'static str,
    tone: &'static str,
    detail: String,
    action: Option<GuideAction>,
    manual: Option<Manual>,
) -> Open {
    Open {
        label,
        tone,
        detail,
        action,
        manual,
    }
}

fn blocked(index: u8, title: &'static str, help: &'static str, needs: u8) -> GuideStep {
    open_step(
        index,
        title,
        help,
        Open {
            label: "bloquée",
            tone: "warn",
            detail: format!("Nécessite l'étape {needs}."),
            action: None,
            manual: None,
        },
    )
}

fn domain_step(input: &Input, domain: &str, public_key_url: &str) -> GuideStep {
    const HELP: &str = "Ce logger doit être joignable en HTTPS sur son domaine, avec une paire de clés. Tesla vérifie la clé publique servie à /.well-known/appspecific/com.tesla.3p.public-key.pem.";
    let ready = domain_ready(domain);
    if ready && input.public_key_published {
        let mut detail = "Le domaine HTTPS est configuré et la clé publique est servie.".to_owned();
        if !input.fingerprint.is_empty() {
            detail.push(' ');
            detail.push_str("Empreinte ");
            detail.push_str(&input.fingerprint);
            detail.push('.');
        }
        return done(0, "Domaine et clés", HELP, detail);
    }
    let (label, tone, detail) = match (ready, input.public_key_published) {
        (false, false) => (
            "à faire",
            "",
            "Le domaine HTTPS et la paire de clés manquent.".to_owned(),
        ),
        (true, false) => (
            "en erreur",
            "bad",
            "La clé privée n'est pas lisible, donc la clé publique n'est pas servie.".to_owned(),
        ),
        (false, true) => (
            "en erreur",
            "bad",
            "Le domaine HTTPS n'est pas utilisable.".to_owned(),
        ),
        (true, true) => unreachable!("both parts are validated above"),
    };
    let mut copies = Vec::new();
    if ready {
        copies.push(CopyValue {
            label: "Domaine",
            value: domain.to_owned(),
        });
    }
    if !public_key_url.is_empty() {
        copies.push(CopyValue {
            label: "Clé publique",
            value: public_key_url.to_owned(),
        });
    }
    open_step(
        0,
        "Domaine et clés",
        HELP,
        gap(
            label,
            tone,
            detail,
            None,
            Some(Manual {
                text: "Configurez TESLA_DOMAIN (HTTPS) et la paire de clés. La clé publique doit répondre à ce chemin. Aucun secret n'est affiché.",
                copies,
            }),
        ),
    )
}

fn app_step(input: &Input, domain: &str, redirect: &str, domain_ok: bool) -> GuideStep {
    const HELP: &str = "L'application déclarée sur developer.tesla.com identifie ce logger. L'URI de redirection doit être exactement celle de ce serveur, sinon Tesla refuse le retour de connexion.";
    if !domain_ok {
        return blocked(1, "Application Tesla", HELP, 0);
    }
    if input.client_configured {
        return done(
            1,
            "Application Tesla",
            HELP,
            "L'identifiant et le secret d'application sont configurés.".to_owned(),
        );
    }
    let mut copies = Vec::new();
    if domain_ready(domain) {
        copies.push(CopyValue {
            label: "Domaine",
            value: domain.to_owned(),
        });
    }
    if !redirect.is_empty() {
        copies.push(CopyValue {
            label: "URI de redirection",
            value: redirect.to_owned(),
        });
    }
    open_step(
        1,
        "Application Tesla",
        HELP,
        gap(
            "à faire",
            "",
            "L'application Tesla n'est pas configurée.".to_owned(),
            None,
            Some(Manual {
                text: "Sur developer.tesla.com, créez une application avec ces valeurs, puis renseignez TESLA_CLIENT_ID et TESLA_CLIENT_SECRET. Le secret n'est pas affiché.",
                copies,
            }),
        ),
    )
}

fn partner_step(
    input: &Input,
    domain: &str,
    domain_step: &GuideStep,
    app: &GuideStep,
) -> GuideStep {
    const HELP: &str = "Enregistrer le domaine associe cette clé publique au domaine chez Tesla. Sans cela, la voiture n'accepte pas la clé virtuelle.";
    if !domain_step.validated() {
        return blocked(2, "Domaine partenaire", HELP, 0);
    }
    if !app.validated() {
        return blocked(2, "Domaine partenaire", HELP, 1);
    }
    let manual = || Manual {
        text: "Enregistre ce domaine chez Tesla (POST /api/1/partner_accounts).",
        copies: vec![CopyValue {
            label: "Domaine",
            value: domain.to_owned(),
        }],
    };
    let action = input.is_admin.then_some(GuideAction::RegisterPartner);
    match &input.partner {
        PartnerState::Registered => done(
            2,
            "Domaine partenaire",
            HELP,
            "Tesla connaît ce domaine.".to_owned(),
        ),
        PartnerState::DifferentKey => open_step(
            2,
            "Domaine partenaire",
            HELP,
            gap(
                "en erreur",
                "bad",
                "Tesla a une clé différente pour ce domaine.".to_owned(),
                action,
                Some(manual()),
            ),
        ),
        PartnerState::Missing => open_step(
            2,
            "Domaine partenaire",
            HELP,
            gap(
                "à faire",
                "",
                "Le domaine n'est pas enregistré chez Tesla.".to_owned(),
                action,
                Some(manual()),
            ),
        ),
        PartnerState::Unknown { message } if partner_failed(message) => open_step(
            2,
            "Domaine partenaire",
            HELP,
            gap("en erreur", "bad", message.clone(), action, Some(manual())),
        ),
        PartnerState::Unknown { message } => open_step(
            2,
            "Domaine partenaire",
            HELP,
            gap("à faire", "", message.clone(), action, Some(manual())),
        ),
    }
}

fn partner_failed(message: &str) -> bool {
    message.contains("n'a pas abouti") || message.contains("expiré") || message.contains("refus")
}

fn account_step(input: &Input, partner: &GuideStep) -> GuideStep {
    const HELP: &str = "Le compte Tesla autorise ce logger à lire les véhicules avec la Fleet API. Le jeton doit être valide et couvrir les scopes demandés.";
    if !partner.validated() {
        return blocked(3, "Compte Tesla", HELP, 2);
    }
    if !input.fleet_readable {
        return open_step(
            3,
            "Compte Tesla",
            HELP,
            gap(
                "en erreur",
                "bad",
                "Le jeton Fleet stocké ne peut pas être lu.".to_owned(),
                Some(GuideAction::Connect),
                Some(account_manual()),
            ),
        );
    }
    if !input.fleet_present {
        return open_step(
            3,
            "Compte Tesla",
            HELP,
            gap(
                "à faire",
                "",
                "Aucun jeton Fleet. Connectez le compte Tesla.".to_owned(),
                Some(GuideAction::Connect),
                Some(account_manual()),
            ),
        );
    }
    let missing = missing_scopes(&input.scopes);
    let expired = input
        .fleet_expires
        .is_some_and(|expires| expires <= input.now);
    let expiring = input
        .fleet_expires
        .is_some_and(|expires| expires <= input.now + EXPIRING);
    if expired || !missing.is_empty() {
        let detail = if expired {
            "Le jeton Fleet est expiré.".to_owned()
        } else {
            format!("Scopes manquants : {}.", missing.join(", "))
        };
        let mut step = open_step(
            3,
            "Compte Tesla",
            HELP,
            gap(
                "en erreur",
                "bad",
                detail,
                Some(GuideAction::Connect),
                Some(account_manual()),
            ),
        );
        step.show_refresh = expired || input.refresh_failed;
        return step;
    }
    let mut detail = "Le jeton Fleet est utilisable.".to_owned();
    if expiring {
        detail.push_str(" Il expire dans moins d'une heure.");
    }
    if input.refresh_failed {
        detail.push_str(" Le dernier rafraîchissement a échoué.");
    }
    let mut step = done(3, "Compte Tesla", HELP, detail);
    step.show_refresh = expiring || input.refresh_failed;
    step
}

fn account_manual() -> Manual {
    Manual {
        text: "Ouvrez la connexion Tesla et acceptez les scopes demandés.",
        copies: Vec::new(),
    }
}

fn missing_scopes(have: &[String]) -> Vec<String> {
    Scope::third_party_vehicle()
        .iter()
        .map(Scope::as_str)
        .filter(|scope| !have.iter().any(|item| item == scope))
        .map(str::to_owned)
        .collect()
}

fn key_step(input: &Input, domain: &str, account: &GuideStep) -> GuideStep {
    const HELP: &str = "La clé virtuelle fait que la voiture fait confiance aux messages signés par l'application. Elle est nécessaire pour la télémétrie et les commandes.";
    if !account.validated() {
        return blocked(4, "Clé virtuelle", HELP, 3);
    }
    let cars = input
        .vehicles
        .iter()
        .map(|vehicle| CarLine {
            name: vehicle.name.clone(),
            key: key_label(vehicle.key),
            config: String::new(),
            stream: String::new(),
            health: String::new(),
            health_tone: "",
        })
        .collect::<Vec<_>>();
    let url = pairing_url(domain).unwrap_or_default();
    let all_paired = !input.vehicles.is_empty()
        && input
            .vehicles
            .iter()
            .all(|vehicle| vehicle.key == KeyState::Paired);
    if all_paired {
        let mut step = done(
            4,
            "Clé virtuelle",
            HELP,
            "La clé est appariée sur chaque véhicule.".to_owned(),
        );
        step.cars = cars;
        return step;
    }
    let unknown = input
        .vehicles
        .iter()
        .any(|vehicle| vehicle.key == KeyState::Unknown);
    let unpaired = input
        .vehicles
        .iter()
        .any(|vehicle| vehicle.key == KeyState::Unpaired);
    let (label, tone, detail) = if input.vehicles.is_empty() {
        (
            "à faire",
            "",
            "Aucun véhicule sur ce compte pour l'instant.".to_owned(),
        )
    } else if unpaired || !unknown {
        (
            "à faire",
            "",
            "Ajoutez la clé virtuelle sur chaque véhicule.".to_owned(),
        )
    } else {
        (
            "en erreur",
            "bad",
            "L'état de la clé virtuelle n'a pas pu être lu.".to_owned(),
        )
    };
    let action = if url.is_empty() {
        None
    } else {
        let qr = qr::svg_labeled(&url, "Code QR pour ajouter la clé virtuelle").unwrap_or_default();
        Some(GuideAction::Pair {
            url: url.clone(),
            qr,
        })
    };
    let mut step = open_step(
        4,
        "Clé virtuelle",
        HELP,
        gap(
            label,
            tone,
            detail,
            action,
            Some(Manual {
                text: "Ouvrez le lien sur le téléphone, dans l'application Tesla, et ajoutez la clé au véhicule.",
                copies: if url.is_empty() {
                    Vec::new()
                } else {
                    vec![CopyValue {
                        label: "Lien",
                        value: url,
                    }]
                },
            }),
        ),
    );
    step.cars = cars;
    step
}

fn telemetry_step(input: &Input, key: &GuideStep) -> GuideStep {
    const HELP: &str = "La télémétrie envoie l'état sans interroger la voiture. Elle est facultative : le polling Fleet fonctionne sans elle.";
    if !key.validated() {
        let mut step = blocked(5, "Fleet Telemetry", HELP, 4);
        step.optional = true;
        return step;
    }
    let cars = input
        .vehicles
        .iter()
        .map(|vehicle| CarLine {
            name: vehicle.name.clone(),
            key: String::new(),
            config: config_label(vehicle, input.telemetry_enabled),
            stream: stream_label(vehicle.last_message_age, input.fresh_for),
            health: vehicle.health.clone(),
            health_tone: vehicle.health_tone,
        })
        .collect::<Vec<_>>();
    let received = |vehicle: &VehicleInput| vehicle.last_message_age.is_some();
    let ready = input.telemetry_enabled
        && !input.vehicles.is_empty()
        && input.vehicles.iter().all(|vehicle| {
            vehicle.skip_reason.is_none() && vehicle.config_pushed && received(vehicle)
        });
    if ready {
        let mut step = done(
            5,
            "Fleet Telemetry",
            HELP,
            "La configuration est envoyée et des messages arrivent. Le polling reste disponible."
                .to_owned(),
        );
        step.optional = true;
        step.cars = cars;
        return step;
    }
    let skipped = input
        .vehicles
        .iter()
        .any(|vehicle| vehicle.skip_reason.is_some());
    let (label, tone, detail) = if !input.telemetry_enabled {
        (
            "à faire",
            "",
            "La télémétrie n'est pas activée. Le polling Fleet reste la source.".to_owned(),
        )
    } else if input.vehicles.is_empty() {
        (
            "à faire",
            "",
            "Aucun véhicule à configurer. Le polling Fleet reste la source.".to_owned(),
        )
    } else if skipped {
        (
            "en erreur",
            "bad",
            "Tesla a ignoré au moins un véhicule.".to_owned(),
        )
    } else if input.vehicles.iter().all(|vehicle| vehicle.config_pushed) {
        (
            "à faire",
            "",
            "La configuration est envoyée, aucun message reçu.".to_owned(),
        )
    } else {
        (
            "à faire",
            "",
            "La configuration n'a pas encore été envoyée.".to_owned(),
        )
    };
    let action = input
        .telemetry_enabled
        .then_some(GuideAction::ResendTelemetry);
    let manual = if input.telemetry_enabled {
        Manual {
            text: "Renvoyez la configuration signée. Un message doit arriver pour valider l'étape. Le polling fonctionne sans elle.",
            copies: Vec::new(),
        }
    } else {
        Manual {
            text: "Installez un certificat que la voiture peut vérifier, ouvrez le port, puis activez la télémétrie et redémarrez. Le polling fonctionne sans cette étape.",
            copies: vec![
                CopyValue {
                    label: "Port",
                    value: input.telemetry_port.to_string(),
                },
                CopyValue {
                    label: "Variable",
                    value: "TELEMETRY_ENABLED=true".to_owned(),
                },
            ],
        }
    };
    let mut step = open_step(
        5,
        "Fleet Telemetry",
        HELP,
        gap(label, tone, detail, action, Some(manual)),
    );
    step.optional = true;
    step.cars = cars;
    step
}

fn key_label(state: KeyState) -> String {
    match state {
        KeyState::Paired => "Appariée".to_owned(),
        KeyState::Unpaired => "Pas appariée".to_owned(),
        KeyState::Unknown => "Inconnu".to_owned(),
    }
}

fn config_label(vehicle: &VehicleInput, telemetry_enabled: bool) -> String {
    if let Some(reason) = vehicle.skip_reason.as_deref() {
        format!("Ignorée : {}", skip_label(reason))
    } else if vehicle.config_pushed {
        "Envoyée".to_owned()
    } else if telemetry_enabled {
        "Pas envoyée".to_owned()
    } else {
        "Non demandée".to_owned()
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

fn stream_label(age: Option<Duration>, fresh_for: Duration) -> String {
    let Some(age) = age else {
        return "Aucun message reçu".to_owned();
    };
    let text = if age.as_secs() < 60 {
        format!("Reçu il y a {} s", age.as_secs())
    } else if age.as_secs() < 3600 {
        format!("Reçu il y a {} min", age.as_secs() / 60)
    } else {
        format!("Reçu il y a {} h", age.as_secs() / 3600)
    };
    if age > fresh_for {
        format!("{text}, trop ancien pour remplacer le polling")
    } else {
        text
    }
}

#[cfg(test)]
fn sample_input() -> Input {
    Input {
        now: Utc::now(),
        fleet_present: false,
        fleet_readable: true,
        fleet_expires: None,
        scopes: Vec::new(),
        refresh_failed: false,
        partner: PartnerState::Missing,
        public_key_published: true,
        domain: "logger.example".to_owned(),
        redirect_uri: "https://logger.example/auth/callback".to_owned(),
        client_configured: true,
        client_id: "logger-app".to_owned(),
        region: tesla_auth::Region::Europe,
        fingerprint: "sha256:abcd".to_owned(),
        telemetry_enabled: false,
        telemetry_port: 4443,
        fresh_for: Duration::from_secs(900),
        is_admin: true,
        vehicles: Vec::new(),
    }
}

#[cfg(test)]
pub(crate) fn test_scopes() -> Vec<String> {
    full_scopes()
}

#[cfg(test)]
fn full_scopes() -> Vec<String> {
    Scope::third_party_vehicle()
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect()
}

#[cfg(test)]
fn guide(report: &Report, index: u8) -> &GuideStep {
    report
        .steps
        .iter()
        .find(|step| step.index == index)
        .unwrap_or_else(|| panic!("step {index}"))
}

#[cfg(test)]
mod tests {
    use super::{
        GuideAction, Input, KeyState, PartnerState, VehicleInput, build, disconnected_poll_wait,
        guide, pairing_url,
    };
    use chrono::{Duration as ChronoDuration, Utc};
    use std::time::Duration;

    fn ready() -> Input {
        let mut input = super::sample_input();
        input.partner = PartnerState::Registered;
        input.fleet_present = true;
        input.fleet_expires = Some(Utc::now() + ChronoDuration::hours(8));
        input.scopes = super::full_scopes();
        input.vehicles = vec![VehicleInput {
            id: 4,
            name: "Faucon".to_owned(),
            key: KeyState::Paired,
            skip_reason: None,
            config_pushed: false,
            last_message_age: None,
            health: String::new(),
            health_tone: "",
        }];
        input
    }

    #[test]
    fn step_five_carries_the_telemetry_health_chip() {
        let mut facts = ready();
        facts.telemetry_enabled = true;
        facts.vehicles[0].config_pushed = true;
        facts.vehicles[0].last_message_age = Some(Duration::from_secs(30));
        facts.vehicles[0].health = "Silencieuse".to_owned();
        facts.vehicles[0].health_tone = "warn";
        let report = build(&facts);
        assert_eq!(guide(&report, 5).cars[0].health, "Silencieuse");
        assert_eq!(guide(&report, 5).cars[0].health_tone, "warn");
        assert_eq!(guide(&report, 4).cars[0].health, "");
    }

    #[test]
    fn validated_steps_hide_the_button_and_the_manual_text() {
        let report = build(&ready());
        for index in 0..=4 {
            let step = guide(&report, index);
            assert_eq!(step.label, "Validé", "step {index}");
            assert!(step.action.is_none(), "step {index} still has an action");
            assert!(step.manual.is_none(), "step {index} still has manual text");
        }
        assert_eq!(report.progress, "5 étapes sur 5 validées");
        assert!(report.show_disconnect);
        assert!(!guide(&report, 3).show_refresh);
        assert!(guide(&report, 5).optional);
        assert_eq!(guide(&report, 5).label, "à faire");
        assert!(guide(&report, 5).action.is_none());
        let manual = guide(&report, 5).manual.as_ref().unwrap();
        assert!(manual.copies.iter().any(|copy| copy.value == "4443"));
        assert!(
            manual
                .copies
                .iter()
                .any(|copy| copy.value == "TELEMETRY_ENABLED=true")
        );
    }

    #[test]
    fn a_missing_prerequisite_blocks_the_step_and_hides_its_button() {
        let report = build(&super::sample_input());
        assert_eq!(guide(&report, 0).label, "Validé");
        assert!(guide(&report, 0).manual.is_none());
        assert_eq!(guide(&report, 1).label, "Validé");
        assert!(guide(&report, 1).manual.is_none());
        assert!(
            !guide(&report, 1)
                .detail
                .contains("https://logger.example/auth/callback")
        );
        assert_eq!(guide(&report, 2).label, "à faire");
        assert_eq!(guide(&report, 2).action, Some(GuideAction::RegisterPartner));
        assert_eq!(guide(&report, 3).label, "bloquée");
        assert_eq!(guide(&report, 3).detail, "Nécessite l'étape 2.");
        assert!(guide(&report, 3).action.is_none());
        assert!(guide(&report, 3).manual.is_none());
        assert!(!guide(&report, 3).show_refresh);
        assert_eq!(guide(&report, 4).detail, "Nécessite l'étape 3.");
        assert!(guide(&report, 4).action.is_none());
        assert_eq!(guide(&report, 5).detail, "Nécessite l'étape 4.");
        assert!(guide(&report, 5).optional);
        assert_eq!(report.progress, "2 étapes sur 5 validées");
        assert!(!report.show_disconnect);
        let help = guide(&report, 4).help;
        assert!(help.contains("fait confiance"));
        assert!(help.contains("télémétrie"));
    }

    #[test]
    fn an_unconfigured_app_shows_the_redirect_uri_and_blocks_the_partner_step() {
        let mut facts = super::sample_input();
        facts.client_configured = false;
        facts.client_id.clear();
        let report = build(&facts);
        assert_eq!(guide(&report, 1).label, "à faire");
        let manual = guide(&report, 1).manual.as_ref().unwrap();
        assert!(
            manual
                .copies
                .iter()
                .any(|copy| copy.value == "https://logger.example/auth/callback")
        );
        assert!(
            manual
                .copies
                .iter()
                .any(|copy| copy.value == "logger.example")
        );
        assert!(!manual.text.contains("secret-value"));
        assert_eq!(guide(&report, 2).label, "bloquée");
        assert_eq!(guide(&report, 2).detail, "Nécessite l'étape 1.");
        assert!(guide(&report, 2).action.is_none());
        assert!(report.revoke_url.is_none());
    }

    #[test]
    fn a_usable_account_can_disconnect_and_an_expiring_token_keeps_refresh() {
        let mut facts = ready();
        facts.fleet_expires = Some(facts.now + ChronoDuration::minutes(10));
        facts.refresh_failed = true;
        let report = build(&facts);
        assert_eq!(guide(&report, 3).label, "Validé");
        assert!(guide(&report, 3).show_refresh);
        assert!(guide(&report, 3).action.is_none());
        assert!(guide(&report, 3).detail.contains("moins d'une heure"));
        assert!(guide(&report, 3).detail.contains("rafraîchissement"));
        assert!(report.show_disconnect);
        let revoke = report.revoke_url.unwrap();
        assert!(revoke.starts_with("https://auth.tesla.com/user/revoke/consent?"));
        assert!(revoke.contains("revoke_client_id=logger-app"));
        assert!(revoke.contains("back_url="));
        assert!(!revoke.contains("secret"));
    }

    #[test]
    fn an_expired_or_incomplete_token_is_an_error_and_offers_connect() {
        let mut facts = ready();
        facts.fleet_expires = Some(facts.now - ChronoDuration::minutes(1));
        let expired = build(&facts);
        assert_eq!(guide(&expired, 3).label, "en erreur");
        assert_eq!(guide(&expired, 3).action, Some(GuideAction::Connect));
        assert!(guide(&expired, 3).show_refresh);
        assert!(guide(&expired, 3).manual.is_some());
        assert!(!expired.show_disconnect);
        assert_eq!(guide(&expired, 4).label, "bloquée");

        facts.fleet_expires = Some(facts.now + ChronoDuration::hours(2));
        facts.scopes.retain(|scope| scope != "vehicle_cmds");
        facts.refresh_failed = false;
        let scopes = build(&facts);
        assert_eq!(guide(&scopes, 3).label, "en erreur");
        assert!(guide(&scopes, 3).detail.contains("vehicle_cmds"));
        assert_eq!(guide(&scopes, 3).action, Some(GuideAction::Connect));
        assert!(!guide(&scopes, 3).show_refresh);
        assert!(!scopes.show_disconnect);
    }

    #[test]
    fn the_virtual_key_link_is_hidden_once_every_car_is_paired() {
        let mut facts = ready();
        facts.vehicles.push(VehicleInput {
            id: 9,
            name: "Millenium".to_owned(),
            key: KeyState::Unpaired,
            skip_reason: None,
            config_pushed: false,
            last_message_age: None,
            health: String::new(),
            health_tone: "",
        });
        let open = build(&facts);
        assert_eq!(guide(&open, 4).label, "à faire");
        match &guide(&open, 4).action {
            Some(GuideAction::Pair { url, qr }) => {
                assert_eq!(url, "https://tesla.com/_ak/logger.example");
                assert!(qr.contains("<svg"));
                assert!(!qr.contains("https://"));
            }
            other => panic!("expected the pairing link, got {other:?}"),
        }
        assert!(guide(&open, 4).manual.is_some());
        assert_eq!(guide(&open, 4).cars[1].key, "Pas appariée");

        facts.vehicles[1].key = KeyState::Paired;
        let paired = build(&facts);
        assert_eq!(guide(&paired, 4).label, "Validé");
        assert!(guide(&paired, 4).action.is_none());
        assert!(guide(&paired, 4).manual.is_none());
    }

    #[test]
    fn telemetry_is_optional_and_validated_only_after_a_message() {
        let mut facts = ready();
        facts.telemetry_enabled = true;
        facts.vehicles[0].config_pushed = true;
        facts.vehicles[0].last_message_age = Some(Duration::from_secs(30));
        let report = build(&facts);
        assert_eq!(guide(&report, 5).label, "Validé");
        assert!(guide(&report, 5).optional);
        assert!(guide(&report, 5).action.is_none());
        assert!(guide(&report, 5).manual.is_none());
        assert!(guide(&report, 5).cars[0].stream.contains("Reçu"));
        assert_eq!(report.progress, "5 étapes sur 5 validées");

        facts.vehicles[0].last_message_age = None;
        facts.vehicles[0].skip_reason = Some("missing_key".to_owned());
        let skipped = build(&facts);
        assert_eq!(guide(&skipped, 5).label, "en erreur");
        assert_eq!(
            guide(&skipped, 5).action,
            Some(GuideAction::ResendTelemetry)
        );
        assert!(
            guide(&skipped, 5).cars[0]
                .config
                .contains("clé virtuelle absente")
        );
    }

    #[test]
    fn a_non_admin_does_not_get_the_partner_button_and_bad_domains_have_no_link() {
        let mut facts = super::sample_input();
        facts.is_admin = false;
        facts.public_key_published = false;
        let report = build(&facts);
        assert_eq!(guide(&report, 0).label, "en erreur");
        assert!(guide(&report, 0).manual.is_some());
        assert_eq!(guide(&report, 1).label, "bloquée");
        assert_eq!(guide(&report, 2).action, None);
        assert_eq!(report.progress, "0 étape sur 5 validée");
        assert!(pairing_url("https://evil.example").is_none());
        assert!(pairing_url("localhost").is_none());
        assert!(disconnected_poll_wait("disconnected").is_some());
        assert!(disconnected_poll_wait("connected").is_none());
    }
}
