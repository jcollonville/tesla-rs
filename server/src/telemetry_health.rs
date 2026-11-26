// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Per-vehicle Fleet Telemetry health.
//!
//! Ok and Asleep are normal. Every other state is shown with words and an icon,
//! and the page links to `/connection`.

use std::time::Duration;

/// Online with no telemetry message for this long is silent, not merely asleep.
pub(crate) const SILENT_ONLINE: Duration = Duration::from_secs(180);

/// Why telemetry is or is not the live source for one vehicle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TelemetryHealth {
    Ok,
    Asleep,
    SilentWhileOnline,
    ConfigSkipped(String),
    ConfigMissing,
    RetryScheduled,
    ListenerError(String),
    TlsRejected { count: u64 },
}

/// Banner copy. `alert` selects `role="alert"`; otherwise `role="status"`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HealthNotice {
    pub alert: bool,
    pub icon: &'static str,
    pub text: String,
}

/// Inputs for [`classify`]. No VIN and no secret.
#[derive(Clone, Debug)]
pub(crate) struct HealthFacts<'a> {
    pub enabled: bool,
    pub asleep: bool,
    pub online: bool,
    pub message_age: Option<Duration>,
    pub configured: bool,
    pub skip_reason: Option<&'a str>,
    pub retry_scheduled: bool,
    pub listener_error: Option<&'a str>,
    pub tls_rejected: u64,
}

impl TelemetryHealth {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Asleep => "asleep",
            Self::SilentWhileOnline => "silent_while_online",
            Self::ConfigSkipped(_) => "config_skipped",
            Self::ConfigMissing => "config_missing",
            Self::RetryScheduled => "retry_scheduled",
            Self::ListenerError(_) => "listener_error",
            Self::TlsRejected { .. } => "tls_rejected",
        }
    }

    /// Short chip label and tone. The words carry the state; colour is extra.
    pub(crate) fn chip(&self) -> (&'static str, &'static str) {
        match self {
            Self::Ok => ("OK", "ok"),
            Self::Asleep => ("Endormie", "ok"),
            Self::SilentWhileOnline => ("Silencieuse", "warn"),
            Self::ConfigSkipped(_) => ("Ignorée", "bad"),
            Self::ConfigMissing => ("Config absente", "warn"),
            Self::RetryScheduled => ("Nouvel essai", "warn"),
            Self::ListenerError(_) => ("Récepteur", "bad"),
            Self::TlsRejected { .. } => ("TLS refusé", "bad"),
        }
    }

    /// Home and vehicle-card banner. Ok and Asleep stay quiet.
    pub(crate) fn notice(&self) -> Option<HealthNotice> {
        let (alert, icon, text) = match self {
            Self::Ok | Self::Asleep => return None,
            Self::SilentWhileOnline => (
                false,
                "i",
                "La télémétrie est silencieuse alors que la voiture est en ligne. Vérifiez le port et la configuration.".to_owned(),
            ),
            Self::ConfigSkipped(reason) => (
                true,
                "!",
                format!(
                    "Tesla a ignoré la configuration ({reason}). Corrigez-la puis renvoyez-la."
                ),
            ),
            Self::ConfigMissing => (
                false,
                "i",
                "La configuration télémétrie n'a pas été envoyée.".to_owned(),
            ),
            Self::RetryScheduled => (
                false,
                "i",
                "L'envoi de la configuration télémétrie est reporté.".to_owned(),
            ),
            Self::ListenerError(message) => (
                true,
                "!",
                format!("Le récepteur télémétrie ne démarre pas ({message})."),
            ),
            Self::TlsRejected { count } => (
                true,
                "!",
                format!(
                    "Le récepteur a refusé {count} poignée(s) de main TLS. Vérifiez le certificat."
                ),
            ),
        };
        Some(HealthNotice { alert, icon, text })
    }
}

pub(crate) fn classify(facts: &HealthFacts<'_>) -> TelemetryHealth {
    if !facts.enabled {
        return TelemetryHealth::Ok;
    }
    if let Some(message) = facts.listener_error.filter(|text| !text.is_empty()) {
        return TelemetryHealth::ListenerError(clip(message));
    }
    if facts.asleep {
        return TelemetryHealth::Asleep;
    }
    let recent = facts.message_age.is_some_and(|age| age < SILENT_ONLINE);
    if recent {
        return TelemetryHealth::Ok;
    }
    if facts.online && facts.tls_rejected > 0 {
        return TelemetryHealth::TlsRejected {
            count: facts.tls_rejected,
        };
    }
    if let Some(reason) = facts.skip_reason.filter(|text| !text.is_empty()) {
        return TelemetryHealth::ConfigSkipped(skip_label(reason).to_owned());
    }
    if facts.retry_scheduled {
        return TelemetryHealth::RetryScheduled;
    }
    if !facts.configured {
        return TelemetryHealth::ConfigMissing;
    }
    let silent = facts.online && facts.message_age.is_none_or(|age| age >= SILENT_ONLINE);
    if silent {
        return TelemetryHealth::SilentWhileOnline;
    }
    TelemetryHealth::Ok
}

/// `telemetry` stays Télémétrie. A configured receiver that is not fresh is the fallback poll.
pub(crate) fn data_source_label(enabled: bool, reported: &str) -> &'static str {
    if !enabled {
        "Interrogation"
    } else if reported == "telemetry" {
        "Télémétrie"
    } else {
        "Secours (interrogation)"
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

fn clip(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars().take(80) {
        if ch.is_control() {
            continue;
        }
        out.push(ch);
    }
    out
}

/// Whether this disconnect belongs to the socket that is connected now.
pub(crate) fn disconnect_is_current(active: Option<&str>, event_id: &str) -> bool {
    match active {
        Some(active) => !active.is_empty() && active == event_id,
        None => event_id.is_empty(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        HealthFacts, SILENT_ONLINE, TelemetryHealth, classify, data_source_label,
        disconnect_is_current,
    };
    use std::time::Duration;

    fn facts() -> HealthFacts<'static> {
        HealthFacts {
            enabled: true,
            asleep: false,
            online: true,
            message_age: Some(Duration::from_secs(10)),
            configured: true,
            skip_reason: None,
            retry_scheduled: false,
            listener_error: None,
            tls_rejected: 0,
        }
    }

    #[test]
    fn asleep_is_normal_and_online_silence_is_not() {
        let mut asleep = facts();
        asleep.asleep = true;
        asleep.online = false;
        asleep.message_age = None;
        assert_eq!(classify(&asleep), TelemetryHealth::Asleep);
        assert!(classify(&asleep).notice().is_none());

        let mut silent = facts();
        silent.message_age = Some(SILENT_ONLINE + Duration::from_secs(1));
        assert_eq!(classify(&silent), TelemetryHealth::SilentWhileOnline);
        let notice = classify(&silent).notice().unwrap();
        assert!(!notice.alert);
        assert!(notice.text.contains("silencieuse"));
    }

    #[test]
    fn errors_use_an_alert_and_name_the_action() {
        let mut skipped = facts();
        skipped.message_age = None;
        skipped.skip_reason = Some("missing_key");
        skipped.configured = false;
        let health = classify(&skipped);
        assert_eq!(
            health,
            TelemetryHealth::ConfigSkipped("clé virtuelle absente".to_owned())
        );
        assert!(health.notice().unwrap().alert);

        let mut tls = facts();
        tls.message_age = None;
        tls.tls_rejected = 2;
        assert_eq!(classify(&tls), TelemetryHealth::TlsRejected { count: 2 });
    }

    #[test]
    fn fallback_label_is_secours_when_telemetry_is_enabled() {
        assert_eq!(data_source_label(false, "polling"), "Interrogation");
        assert_eq!(data_source_label(true, "telemetry"), "Télémétrie");
        assert_eq!(
            data_source_label(true, "polling"),
            "Secours (interrogation)"
        );
    }

    #[test]
    fn an_old_socket_disconnect_does_not_clear_the_new_one() {
        assert!(!disconnect_is_current(Some("socket-2"), "socket-1"));
        assert!(disconnect_is_current(Some("socket-2"), "socket-2"));
        assert!(!disconnect_is_current(None, "socket-1"));
    }
}
