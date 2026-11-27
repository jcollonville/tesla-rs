// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Compile-time HTML. Maud escapes text and attributes.

use maud::{DOCTYPE, Markup, PreEscaped, html};
use serde::Serialize;

use super::car::{IMAGE_UNAVAILABLE, Look, image_url};
use super::format::Place;
use super::viz::MiniMapModel;
use super::widget::{element, to_json};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GlobalTab {
    None,
    Settings,
    Debug,
    System,
    Security,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum VehicleTab {
    Overview,
    Drives,
    Charges,
    Stats,
    Connection,
}

#[derive(Clone)]
pub(super) struct NavVehicle {
    pub id: i64,
    pub name: String,
    pub current: bool,
}

#[derive(Clone)]
pub(super) struct Chrome {
    pub csrf: String,
    pub vehicles: Vec<NavVehicle>,
    pub section: Option<VehicleTab>,
    pub global: GlobalTab,
    pub debug_script: bool,
    /// Last vehicle this session opened. Ignored when that id is not accessible.
    pub remembered: Option<i64>,
}

#[derive(Default)]
pub(super) struct FleetCard {
    pub id: i64,
    pub name: String,
    pub activity: String,
    pub battery: String,
    pub last_seen: String,
    pub last_error: Option<String>,
    pub month: String,
    pub look: Look,
    pub status: Status,
    pub schedule: Option<String>,
    pub battery_pct: Option<f64>,
    pub limit_pct: Option<f64>,
    pub range: String,
    pub place: Option<String>,
    pub telemetry_notice: Option<StatusNotice>,
}

/// Words and an icon for a telemetry problem. Colour is not the only signal.
pub(super) struct StatusNotice {
    pub role: &'static str,
    pub class: &'static str,
    pub icon: &'static str,
    pub text: String,
}

/// What the car is doing, from `vehicle_state`.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Status {
    Driving,
    Charging,
    Plugged,
    Parked,
    Asleep,
    Offline,
    Updating,
    #[default]
    Unknown,
}

impl Status {
    /// `Charging` is "En charge". `Starting`, `Stopped`, `Complete`, and `NoPower`
    /// are plugged in ("Branchée"), including while a charge session is open.
    /// `Disconnected` stays parked. Sleep, offline, and a software update keep
    /// those labels.
    pub(crate) fn from(
        activity: Option<&str>,
        charging_state: Option<&str>,
        plugged_in: Option<bool>,
    ) -> Self {
        let charging = charging_state.unwrap_or("").trim().to_ascii_lowercase();
        let plugged_idle = matches!(
            charging.as_str(),
            "starting" | "complete" | "stopped" | "nopower"
        );
        match activity.unwrap_or("") {
            "driving" => Self::Driving,
            _ if charging == "charging" => Self::Charging,
            "updating" => Self::Updating,
            "asleep" => Self::Asleep,
            "offline" => Self::Offline,
            _ if plugged_idle => Self::Plugged,
            // An open session with no newer idle state is still charging.
            // `Disconnected` is unplugged and must not stay "En charge".
            "charging" if charging != "disconnected" => Self::Charging,
            // `Disconnected` is unplugged. A stored `plugged_in` flag must not override it.
            _ if charging != "disconnected" && plugged_in == Some(true) => Self::Plugged,
            "online" | "charging" => Self::Parked,
            _ => Self::Unknown,
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Driving => "En route",
            Self::Charging => "En charge",
            Self::Plugged => "Branchée",
            Self::Parked => "Garée",
            Self::Asleep => "Endormie",
            Self::Offline => "Hors ligne",
            Self::Updating => "Mise à jour",
            Self::Unknown => "État inconnu",
        }
    }

    fn tone(self) -> &'static str {
        match self {
            Self::Driving => "status drive",
            Self::Charging | Self::Plugged => "status charge",
            Self::Parked => "status park",
            Self::Updating => "status note",
            Self::Asleep | Self::Offline | Self::Unknown => "status idle",
        }
    }

    pub(super) fn tone_key(self) -> &'static str {
        match self {
            Self::Driving => "drive",
            Self::Charging | Self::Plugged => "charge",
            Self::Parked => "park",
            Self::Updating => "note",
            Self::Asleep | Self::Offline | Self::Unknown => "idle",
        }
    }
}

/// Live state for the car card on the overview.
#[derive(Default)]
pub(super) struct CarCard {
    pub look: Look,
    pub trim: Option<String>,
    pub status: Status,
    pub battery_pct: Option<f64>,
    pub limit_pct: Option<f64>,
    pub range: String,
    /// One line under the battery, such as the charging power and time to full.
    pub note: Option<String>,
    /// Charge schedule in the car's local clock. Hidden when absent.
    pub schedule: Option<String>,
    pub facts: Vec<(&'static str, String)>,
    pub map: MiniMapModel,
    pub place: Option<Place>,
    pub updated: String,
    pub updated_iso: Option<String>,
}

/// A recent drive or charge on the overview.
pub(super) struct Recent {
    pub href: String,
    pub when: String,
    pub from: Option<String>,
    pub to: String,
    pub primary: String,
    pub secondary: String,
    pub open: bool,
}

#[derive(Default)]
pub(super) struct Overview {
    pub name: String,
    pub activity: String,
    pub battery: String,
    pub last_poll: String,
    pub last_data: String,
    pub last_error: String,
    pub last_drive: Option<Recent>,
    pub last_charge: Option<Recent>,
    pub car: CarCard,
    pub month: Vec<(&'static str, String)>,
    pub telemetry_notice: Option<StatusNotice>,
}

pub(super) struct ConnectionView<'a> {
    pub admin: bool,
    pub sudo: bool,
    pub shared_names: Vec<String>,
    pub shared_ok: bool,
    pub csrf: &'a str,
    pub vehicle_id: Option<i64>,
    pub error: Option<&'a str>,
    pub notice: Option<&'a str>,
    pub report: crate::connection::Report,
}

pub(super) struct SettingRow {
    pub label: &'static str,
    pub value: String,
    pub source: &'static str,
}

pub(super) struct TokenLine {
    pub id: i64,
    pub name: String,
    pub active: bool,
    pub scopes: String,
}

pub(super) struct AccessLine {
    pub user: String,
    pub vehicle: String,
    pub role: String,
}

pub(super) struct SessionLine {
    pub id_hash: String,
    pub user: String,
    pub seen: String,
}

pub(super) struct AuditLine {
    pub at: String,
    pub action: String,
    pub outcome: String,
}

pub(super) fn login_page(csrf: &str, error: Option<&str>) -> String {
    document(
        false,
        html! {
            section class="card auth-card" {
                p class="kicker" { (brand_mark()) "Tesla Logger" }
                h1 { "Connexion" }
                p class="muted" { "Mot de passe de ce logger. Ce n'est pas le compte Tesla." }
                @if let Some(error) = error {
                    div class="banner bad" role="alert" { p { (error) } }
                }
                form class="stack" method="post" action="/" {
                    input type="hidden" name="csrf" value=(csrf);
                    label class="field" for="username" {
                        "Nom"
                        input id="username" name="username" value="admin" autocomplete="username" required;
                    }
                    label class="field" for="password" {
                        "Mot de passe"
                        input id="password" type="password" name="password" autocomplete="current-password" required;
                    }
                    label class="field" for="totp" {
                        "Code TOTP"
                        input id="totp" name="totp" inputmode="numeric" autocomplete="one-time-code";
                    }
                    button type="submit" { "Se connecter" }
                }
            }
        },
    )
}

pub(super) enum SetupStep<'a> {
    Password {
        require_current: bool,
    },
    Authenticator {
        secret: &'a str,
        groups: &'a str,
        qr: &'a str,
    },
    Verify,
    Recovery {
        codes: &'a [String],
    },
    Finished,
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn setup_page(csrf: &str, error: Option<&str>, step: SetupStep<'_>) -> String {
    let current = match step {
        SetupStep::Password { .. } => 1,
        SetupStep::Authenticator { .. } => 2,
        SetupStep::Verify => 3,
        SetupStep::Recovery { .. } | SetupStep::Finished => 4,
    };
    let body = html! {
        section class="card" {
            h1 { "Première connexion" }
            (setup_steps(current))
            @if let Some(error) = error {
                div class="banner bad" role="alert" { p { (error) } }
            }
            @match step {
                SetupStep::Password { require_current } => {
                    form class="stack" method="post" action="/security/password" {
                        input type="hidden" name="csrf" value=(csrf);
                        @if require_current {
                            label class="field" for="current-password" {
                                "Mot de passe actuel"
                                input id="current-password" type="password" name="current" autocomplete="current-password" required;
                            }
                        }
                        label class="field" for="new-password" {
                            "Nouveau mot de passe"
                            input id="new-password" type="password" name="password" autocomplete="new-password" minlength="12" required;
                        }
                        p class="hint" { "12 caractères minimum." }
                        p id="password-strength" class="hint" aria-live="polite" { "Saisissez un mot de passe pour voir sa solidité." }
                        button class="tonal" type="button" data-toggle="new-password" aria-pressed="false" { "Afficher" }
                        label class="field" for="confirm-password" {
                            "Confirmer le mot de passe"
                            input id="confirm-password" type="password" name="confirm" autocomplete="new-password" minlength="12" required;
                        }
                        button class="tonal" type="button" data-toggle="confirm-password" aria-pressed="false" { "Afficher" }
                        button type="submit" { "Enregistrer le mot de passe" }
                    }
                }
                SetupStep::Authenticator { secret, groups, qr } => {
                    h2 { "Application d'authentification" }
                    p { "Ouvrez votre application (Aegis, 1Password, ou une autre) et ajoutez un compte. Scannez le code, ou saisissez la clé. L'émetteur est Tesla Logger. Ce secret expire dans 15 minutes." }
                    div class="qr-frame" { (maud::PreEscaped(qr)) }
                    p class="hint" { "Clé secrète" }
                    p id="totp-secret" class="secret-groups" data-value=(secret) { (groups) }
                    button class="tonal" type="button" data-copy-target="totp-secret" { "Copier la clé" }
                    div class="actions" {
                        a class="button" href="/security/setup?step=verify" { "Continuer" }
                        form method="post" action="/security/totp" {
                            input type="hidden" name="csrf" value=(csrf);
                            button class="tonal" type="submit" { "Nouveau code" }
                        }
                    }
                }
                SetupStep::Verify => {
                    h2 { "Vérification" }
                    p { "Saisissez le code à 6 chiffres affiché par l'application." }
                    form class="stack" method="post" action="/security/totp/confirm" {
                        input type="hidden" name="csrf" value=(csrf);
                        label class="field" for="setup-totp" {
                            "Code à 6 chiffres"
                            input id="setup-totp" name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" minlength="6" maxlength="6" required;
                        }
                        button type="submit" { "Activer" }
                    }
                    p { a href="/security/setup" { "Retour au code QR" } }
                }
                SetupStep::Recovery { codes } => {
                    h2 { "Codes de secours" }
                    div class="banner note" role="alert" {
                        p { "Ces codes ne sont affichés qu'une seule fois. Conservez-les hors ligne. Sans eux, un téléphone perdu vous bloque." }
                    }
                    ul id="recovery-codes" class="recovery-grid" data-value=(codes.join("\n")) {
                        @for code in codes {
                            li { code class="recovery-code" { (code) } }
                        }
                    }
                    div class="actions" {
                        button class="tonal" type="button" data-copy-target="recovery-codes" { "Tout copier" }
                        button class="tonal" type="button" data-download="recovery-codes" { "Télécharger .txt" }
                    }
                    form class="stack" method="post" action="/security/setup/finish" {
                        input type="hidden" name="csrf" value=(csrf);
                        label class="check" for="saved-codes" {
                            input id="saved-codes" type="checkbox" name="saved" value="yes" required;
                            " J'ai sauvegardé mes codes"
                        }
                        button type="submit" { "Terminer" }
                    }
                }
                SetupStep::Finished => {
                    h2 { "Codes de secours" }
                    p { "Les codes de secours ne sont plus affichés. L'authentification à deux facteurs est active." }
                    a class="button" href="/" { "Continuer" }
                }
            }
        }
        script src="/static/setup.js" defer {}
    };
    document(false, body)
}

fn setup_steps(current: u8) -> Markup {
    let labels = [
        "1 Mot de passe",
        "2 Application d'authentification",
        "3 Vérification",
        "4 Codes de secours",
    ];
    html! {
        ol class="steps" {
            @for (index, label) in labels.iter().enumerate() {
                @if u8::try_from(index).unwrap_or(0) + 1 == current {
                    li aria-current="step" { (label) }
                } @else {
                    li { (label) }
                }
            }
        }
    }
}

pub(super) fn fleet_page(chrome: &Chrome, cards: &[FleetCard], month_total: &str) -> String {
    let body = html! {
        (toolbar(chrome, "Voitures"))
        @if cards.is_empty() {
            section class="card empty-state" {
                h2 { "Aucune voiture pour l'instant" }
                p class="muted" { "Connectez un compte Tesla pour commencer l'archive." }
                a class="button" href="/connection" { "Connexion Tesla" }
            }
        } @else {
            p class="lede" { "Ce mois-ci : " (month_total) }
            div class="fleet" {
                @for card in cards {
                    (telemetry_banner(card.telemetry_notice.as_ref()))
                    a class="card fleet-card" href=(format!("/v/{}", card.id)) {
                        div class="fleet-head" {
                            div {
                                h2 class="fleet-name" { (card.name) }
                                p class="eyebrow" { (card.look.model.unwrap_or("Voiture")) }
                            }
                            div class="status-wrap" title=(card.activity) {
                                (status_chip(card.status))
                                @if let Some(schedule) = &card.schedule {
                                    p class="muted fleet-schedule" { (schedule) }
                                }
                            }
                        }
                        div class="fleet-car" { (car_media(image_url(&card.look, card.id).as_deref(), &card.look.label())) }
                        div class="fleet-battery" {
                            span class="figure" { (card.battery) }
                            span class="muted" { (card.range) }
                        }
                        (battery_el(card.battery_pct, card.limit_pct, card.status == Status::Charging))
                        dl class="fleet-facts" {
                            @if let Some(place) = &card.place {
                                div { dt { "Position" } dd { (place) } }
                            }
                            div { dt { "Ce mois" } dd { (card.month) } }
                            div { dt { "Dernière activité" } dd { (card.last_seen) } }
                        }
                        @if let Some(error) = &card.last_error {
                            p class="banner bad" { "Dernière erreur : " (error) }
                        }
                    }
                }
            }
        }
    };
    document(chrome.debug_script, body)
}

fn telemetry_banner(notice: Option<&StatusNotice>) -> Markup {
    let Some(notice) = notice else {
        return html! {};
    };
    html! {
        div class=(notice.class) role=(notice.role) {
            p {
                span aria-hidden="true" { (notice.icon) " " }
                (notice.text)
                " "
                a href="/connection" { "Connexion" }
            }
        }
    }
}

pub(super) fn status_chip(status: Status) -> Markup {
    html! {
        span class=(format!("chip {}", status.tone())) {
            span class="status-dot" aria-hidden="true" {}
            (status.label())
        }
    }
}

fn battery_el(level: Option<f64>, limit: Option<f64>, charging: bool) -> Markup {
    let level_text = level.map(|value| value.to_string());
    let limit_text = limit.map(|value| value.to_string());
    let mut attrs = Vec::new();
    if let Some(level) = level_text.as_deref() {
        attrs.push(("level", level));
    }
    if let Some(limit) = limit_text.as_deref() {
        attrs.push(("limit", limit));
    }
    if charging {
        attrs.push(("charging", ""));
    }
    element("tl-battery", &attrs, None, html! {})
}

fn car_media(url: Option<&str>, alt: &str) -> Markup {
    let mut attrs = vec![
        ("alt", alt),
        ("unavailable", IMAGE_UNAVAILABLE),
        ("locale", "fr"),
    ];
    if let Some(url) = url {
        attrs.push(("image-url", url));
    }
    element("tl-car-image", &attrs, None, html! {})
}

#[derive(Serialize)]
struct FactJson<'a> {
    label: &'a str,
    value: &'a str,
}

#[derive(Serialize)]
struct VehicleJson<'a> {
    name: &'a str,
    model: Option<&'a str>,
    trim: Option<&'a str>,
    status_label: &'a str,
    status_tone: &'a str,
    charging: bool,
    battery_level: Option<f64>,
    charge_limit_soc: Option<f64>,
    battery_text: &'a str,
    range_text: &'a str,
    note: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schedule: Option<&'a str>,
    facts: Vec<FactJson<'a>>,
}

#[derive(Serialize)]
struct RecentJson<'a> {
    href: &'a str,
    when: &'a str,
    from: Option<&'a str>,
    to: &'a str,
    primary: &'a str,
    secondary: &'a str,
    open: bool,
}

#[derive(Serialize)]
struct TabJson<'a> {
    id: &'a str,
    label: &'a str,
    href: String,
    selected: bool,
}

fn recent_card(
    id: &str,
    title: &str,
    item: Option<&Recent>,
    empty: &str,
    all_href: Option<&str>,
    all_label: &str,
) -> Markup {
    let mut attrs = vec![
        ("id", id),
        ("title", title),
        ("empty", empty),
        ("locale", "fr"),
    ];
    if let Some(href) = all_href {
        attrs.push(("link-href", href));
        attrs.push(("link-label", all_label));
    }
    let json = item.map(|item| {
        to_json(&RecentJson {
            href: &item.href,
            when: &item.when,
            from: item.from.as_deref(),
            to: &item.to,
            primary: &item.primary,
            secondary: &item.secondary,
            open: item.open,
        })
    });
    element("tl-recent-card", &attrs, json.as_deref(), html! {})
}

pub(super) fn overview_page(chrome: &Chrome, overview: &Overview) -> String {
    let car = &overview.car;
    let id = current_id(chrome);
    let facts: Vec<FactJson<'_>> = car
        .facts
        .iter()
        .map(|(label, value)| FactJson { label, value })
        .collect();
    let vehicle = to_json(&VehicleJson {
        name: &overview.name,
        model: car.look.model,
        trim: car.trim.as_deref(),
        status_label: car.status.label(),
        status_tone: car.status.tone_key(),
        charging: car.status == Status::Charging,
        battery_level: car.battery_pct,
        charge_limit_soc: car.limit_pct,
        battery_text: &overview.battery,
        range_text: &car.range,
        note: car.note.as_deref(),
        schedule: car.schedule.as_deref(),
        facts,
    });
    let image = id.and_then(|vehicle_id| image_url(&car.look, vehicle_id));
    let mut card_attrs = vec![
        ("locale", "fr"),
        ("unavailable", IMAGE_UNAVAILABLE),
        ("alt", overview.name.as_str()),
    ];
    if let Some(url) = image.as_deref() {
        card_attrs.push(("image-url", url));
    }
    let map = to_json(&car.map);
    let drives_href = id.map(|vehicle_id| format!("/v/{vehicle_id}/drives"));
    let charges_href = id.map(|vehicle_id| format!("/v/{vehicle_id}/charges"));
    let stats_href = id.map(|vehicle_id| format!("/v/{vehicle_id}/stats"));
    let month: Vec<FactJson<'_>> = overview
        .month
        .iter()
        .map(|(label, value)| FactJson { label, value })
        .collect();
    let month_json = to_json(&month);
    let month_card = if overview.month.is_empty() {
        html! {}
    } else {
        let mut month_attrs = vec![
            ("variant", "card"),
            ("title", "Ce mois-ci"),
            ("label", "Ce mois-ci"),
            ("locale", "fr"),
        ];
        if let Some(href) = stats_href.as_deref() {
            month_attrs.push(("link-href", href));
            month_attrs.push(("link-label", "Statistiques"));
        }
        element(
            "tl-summary-strip",
            &month_attrs,
            Some(&month_json),
            html! {},
        )
    };
    let body = html! {
        (toolbar(chrome, &overview.name))
        (vehicle_tabs(chrome))
        div id="panel-overview" class="overview" role="tabpanel" aria-labelledby="tab-overview" {
            @if overview.last_error != "—" && !overview.last_error.is_empty() {
                div class="banner bad overview-alert" role="status" { p { "Dernière erreur : " (overview.last_error) } }
            }
            (telemetry_banner(overview.telemetry_notice.as_ref()))
            (element("tl-vehicle-card", &card_attrs, Some(&vehicle), html! {}))
            section class="card place-card" aria-labelledby="place-title" {
                h2 id="place-title" class="visually-hidden" { "Position" }
                div class="map-frame" {
                    (element("tl-mini-map", &[("locale", "fr")], Some(&map), html! {}))
                }
                div class="place-text" {
                    p class="place-name" { (Place::name_or_unknown(car.place.as_ref())) }
                    @if let Some(detail) = car.place.as_ref().and_then(|place| place.detail.as_ref()) {
                        p class="muted place-detail" { (detail) }
                    }
                    p class="updated" {
                        "Mis à jour "
                        @if let Some(iso) = &car.updated_iso {
                            time datetime=(iso) { (car.updated) }
                        } @else {
                            (car.updated)
                        }
                    }
                }
            }
            (recent_card("dernier-trajet", "Dernier trajet", overview.last_drive.as_ref(), "Aucun trajet", drives_href.as_deref(), "Tous les trajets"))
            (recent_card("derniere-charge", "Dernière charge", overview.last_charge.as_ref(), "Aucune charge", charges_href.as_deref(), "Toutes les charges"))
            (month_card)
            details class="card diagnostics" {
                summary { "Diagnostic" span class="muted" { " · " (overview.activity) } }
                dl class="facts" {
                    div { dt { "Activité" } dd { (overview.activity) } }
                    div { dt { "Batterie" } dd { (overview.battery) } }
                    div { dt { "Dernier poll" } dd { (overview.last_poll) } }
                    div { dt { "Dernière donnée" } dd { (overview.last_data) } }
                    div { dt { "Dernière erreur" } dd { (overview.last_error) } }
                }
            }
        }
    };
    document(chrome.debug_script, body)
}

#[allow(dead_code)]
pub(super) fn placeholder_page(
    chrome: &Chrome,
    title: &str,
    text: &str,
    existing: Option<(&str, &str)>,
) -> String {
    let body = html! {
        (toolbar(chrome, title))
        (vehicle_tabs(chrome))
        div role="tabpanel" {
            section class="card" {
                h2 { (title) }
                p { (text) }
                @if let Some((label, href)) = existing {
                    p { a href=(href) { (label) } }
                }
            }
        }
    };
    document(chrome.debug_script, body)
}

pub(super) fn connection_page(chrome: &Chrome, view: &ConnectionView<'_>) -> String {
    let body = html! {
        (toolbar(chrome, "Connexion Tesla"))
        (vehicle_tabs(chrome))
        div id="panel-connection" role="tabpanel" aria-labelledby="tab-connection" {
            @if let Some(error) = view.error {
                div class="banner bad" role="alert" { p { (error) } }
            }
            @if let Some(notice) = view.notice {
                div class="banner note" role="status" { p { (notice) } }
            }
            p {
                @if view.shared_ok {
                    @if view.shared_names.is_empty() {
                        "Ce compte ne gère pas d'autre véhicule."
                    } @else {
                        "Ce compte gère aussi : "
                        (view.shared_names.join(", "))
                        "."
                    }
                } @else {
                    "La liste des véhicules de ce compte est indisponible."
                }
            }
            section class="card" {
                h2 id="setup-heading" { "Mise en route" }
                p id="setup-progress" { (view.report.progress) }
                ol class="setup" start="0" aria-labelledby="setup-heading" {
                    @for step in &view.report.steps {
                        li {
                            div class="step-line" {
                                span class=(format!("chip {}", step.tone)) {
                                    @if step.label == "Validé" {
                                        (check_icon())
                                    }
                                    (step.label)
                                }
                                h3 {
                                    (step.title)
                                    @if step.optional {
                                        span class="muted" { " (facultative)" }
                                    }
                                    span class="step-help" {
                                        button type="button" class="help" aria-describedby=(format!("step-help-{}", step.index)) aria-label=(format!("Aide : {}", step.title)) { "?" }
                                        p id=(format!("step-help-{}", step.index)) class="help-tip" role="tooltip" { (step.help) }
                                    }
                                }
                            }
                            p { (step.detail) }
                            @if !step.cars.is_empty() {
                                ul {
                                    @for car in &step.cars {
                                        li {
                                            strong { (car.name) }
                                            @if !car.key.is_empty() { " — clé " (car.key) }
                                            @if !car.config.is_empty() { " — configuration " (car.config) }
                                            @if !car.stream.is_empty() { " — " (car.stream) }
                                            @if !car.health.is_empty() {
                                                " "
                                                span class=(format!("chip {}", car.health_tone)) { (car.health) }
                                            }
                                        }
                                    }
                                }
                            }
                            @if let Some(action) = &step.action {
                                (guide_action(view, action))
                            }
                            @if step.show_refresh {
                                form method="post" action="/auth/refresh" {
                                    input type="hidden" name="csrf" value=(view.csrf);
                                    (vehicle_field(view.vehicle_id))
                                    button class="ghost small" type="submit" { "Rafraîchir" }
                                }
                            }
                            @if let Some(manual) = &step.manual {
                                p class="muted" {
                                    (manual.text)
                                    @for copy in &manual.copies {
                                        br;
                                        (copy.label) " " code { (copy.value) }
                                    }
                                }
                            }
                        }
                    }
                }
                p class="muted" { "Aucun secret n'est affiché : ni jeton, ni clé privée." }
                @if view.report.show_disconnect {
                    details class="danger-zone" {
                        summary { "Déconnecter mon compte Tesla" }
                        p { "Tesla ne fournit pas d'endpoint serveur pour révoquer le jeton Fleet. La documentation ne propose qu'une page de consentement, à confirmer sur le compte Tesla. Cette action retire la configuration de télémétrie des véhicules de ce compte si Tesla l'accepte, efface les jetons Fleet stockés ici et arrête l'interrogation. Les véhicules, trajets, charges, positions et réglages sont conservés." }
                        @if let Some(url) = &view.report.revoke_url {
                            p { a href=(url) rel="noreferrer noopener" target="_blank" { "Ouvrir la page Tesla pour retirer l'application" } }
                        }
                        p { "Retirez aussi l'application dans les applications tierces du compte Tesla si elle y figure encore." }
                        form method="post" action="/auth/disconnect" {
                            input type="hidden" name="csrf" value=(view.csrf);
                            input type="hidden" name="confirm" value="disconnect";
                            (vehicle_field(view.vehicle_id))
                            button class="danger" type="submit" { "Déconnecter mon compte Tesla" }
                        }
                    }
                }
            }
            @if view.admin {
                section class="card danger-zone" {
                    h2 { "Supprimer les données" }
                    p { "Irréversible. Efface un véhicule et seulement ses trajets, charges, positions et réglages. Les autres véhicules restent. La déconnexion ne fait pas ça." }
                    @if view.sudo {
                        @for car in &view.report.deletions {
                            form class="stack" method="post" action=(format!("/v/{}/delete", car.id)) {
                                input type="hidden" name="csrf" value=(view.csrf);
                                label class="field" for=(format!("delete-{}", car.id)) {
                                    "Tapez " strong { (car.name) } " pour confirmer"
                                    input id=(format!("delete-{}", car.id)) type="text" name="confirm" autocomplete="off" spellcheck="false" maxlength="120" required;
                                }
                                button class="danger" type="submit" { "Supprimer les données de ce véhicule" }
                            }
                        }
                    } @else {
                        p class="muted" { "Un administrateur avec sudo actif peut supprimer les données. Sudo dure 5 minutes." }
                    }
                }
            }
        }
    };
    document(chrome.debug_script, body)
}

fn check_icon() -> Markup {
    html! {
        svg class="check" viewBox="0 0 16 16" width="14" height="14" aria-hidden="true" {
            path fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" d="M3 8.5 6.5 12 13 4" {}
        }
    }
}

fn guide_action(view: &ConnectionView<'_>, action: &crate::connection::GuideAction) -> Markup {
    html! {
        @match action {
            crate::connection::GuideAction::RegisterPartner => {
                form method="post" action="/connection/partner" {
                    input type="hidden" name="csrf" value=(view.csrf);
                    (vehicle_field(view.vehicle_id))
                    button type="submit" { "Enregistrer le domaine" }
                }
            }
            crate::connection::GuideAction::Connect => {
                p { a class="button" href="/auth/login" { "Connecter" } }
            }
            crate::connection::GuideAction::ResendTelemetry => {
                form method="post" action="/connection/telemetry" {
                    input type="hidden" name="csrf" value=(view.csrf);
                    (vehicle_field(view.vehicle_id))
                    button type="submit" { "Renvoyer la configuration" }
                }
            }
            crate::connection::GuideAction::Pair { url, qr } => {
                p { a href=(url) rel="noreferrer noopener" target="_blank" { (url) } }
                @if !qr.is_empty() {
                    (maud::PreEscaped(qr.clone()))
                }
            }
        }
    }
}

pub(super) fn settings_page(
    chrome: &Chrome,
    rows: &[SettingRow],
    override_form: Option<&str>,
) -> String {
    let body = html! {
        (toolbar(chrome, "Réglages"))
        section class="card" {
            h2 { "Configuration effective" }
            p class="muted" {
                "Valeurs résolues : réglage en base, sinon environnement, sinon défaut. Les secrets restent dans l'environnement et ne sont pas affichés."
            }
            table class="settings" {
                caption class="muted" { "Réglages publics" }
                thead {
                    tr { th { "Réglage" } th { "Valeur" } th { "Source" } }
                }
                tbody {
                    @for row in rows {
                        tr {
                            th scope="row" { (row.label) }
                            td { (row.value) }
                            td { (row.source) }
                        }
                    }
                }
            }
        }
        @if let Some(form) = override_form {
            (maud::PreEscaped(form))
        }
    };
    document(chrome.debug_script, body)
}

pub(super) fn system_page(chrome: &Chrome, status: &str) -> String {
    let body = html! {
        (toolbar(chrome, "Système"))
        (maud::PreEscaped(status))
    };
    document(chrome.debug_script, body)
}

pub(super) fn security_page(
    chrome: &Chrome,
    tokens: &[TokenLine],
    revealed: Option<&str>,
    account: &str,
    access: &[AccessLine],
    sessions: &[SessionLine],
    audit: &[AuditLine],
) -> String {
    let body = html! {
        (toolbar(chrome, "Sécurité"))
        section class="card" {
            h2 { "Compte" }
            p { "Connecté en tant que " (account) "." }
            form class="stack" method="post" action="/sudo" {
                input type="hidden" name="csrf" value=(&chrome.csrf);
                label class="field" for="sudo-password" {
                    "Mot de passe pour les actions sensibles"
                    input id="sudo-password" type="password" name="password" autocomplete="current-password" required;
                }
                button class="tonal" type="submit" { "Confirmer pour 5 minutes" }
            }
        }
        section class="card" {
            h2 { "Mot de passe" }
            form class="stack" method="post" action="/security/password" {
                input type="hidden" name="csrf" value=(&chrome.csrf);
                label class="field" for="current-password" {
                    "Mot de passe actuel"
                    input id="current-password" type="password" name="current" autocomplete="current-password" required;
                }
                label class="field" for="new-password" {
                    "Nouveau mot de passe"
                    input id="new-password" type="password" name="password" autocomplete="new-password" required minlength="12";
                }
                label class="field" for="confirm-password" {
                    "Confirmer le mot de passe"
                    input id="confirm-password" type="password" name="confirm" autocomplete="new-password" required minlength="12";
                }
                button type="submit" { "Changer le mot de passe" }
            }
        }
        section class="card" {
            h2 { "Authentification à deux facteurs" }
            p class="muted" { "Ré-enrôler remplace le secret TOTP actuel et affiche de nouveaux codes de secours." }
            form class="stack" method="post" action="/security/totp" {
                input type="hidden" name="csrf" value=(&chrome.csrf);
                button class="tonal" type="submit" { "Configurer / régénérer le TOTP" }
            }
        }
        section class="card" {
            h2 { "Passkeys" }
            p class="muted" { "Enregistrer une clé d'accès pour ce compte. Nécessite un navigateur compatible." }
            button id="passkey-register" class="tonal" type="button" data-csrf=(&chrome.csrf) { "Enregistrer une passkey" }
            p id="passkey-status" class="muted" aria-live="polite" {}
            script src="/static/setup.js" defer {}
        }
        section class="card" {
            h2 { "Utilisateurs et véhicules" }
            ul class="list" {
                @for row in access {
                    li { span { (row.user) " · " (row.vehicle) } span class="chip" { (row.role) } }
                }
            }
            form class="stack" method="post" action="/users" {
                input type="hidden" name="csrf" value=(&chrome.csrf);
                label class="field" for="new-user" {
                    "Nouvel utilisateur"
                    input id="new-user" name="username" required maxlength="80";
                }
                label class="field" for="new-user-password" {
                    "Mot de passe"
                    input id="new-user-password" type="password" name="password" required minlength="12";
                }
                label class="field" for="new-user-role" {
                    "Rôle"
                    select id="new-user-role" name="role" {
                        option value="user" { "user" }
                        option value="admin" { "admin" }
                    }
                }
                button type="submit" { "Créer l'utilisateur" }
            }
            form class="stack" method="post" action="/vehicle-access" {
                input type="hidden" name="csrf" value=(&chrome.csrf);
                label class="field" for="access-user" {
                    "Utilisateur (id)"
                    input id="access-user" name="user_id" required;
                }
                label class="field" for="access-vehicle" {
                    "Véhicule (id)"
                    input id="access-vehicle" name="vehicle_id" required;
                }
                label class="field" for="access-role" {
                    "Accès"
                    select id="access-role" name="role" {
                        option value="viewer" { "viewer" }
                        option value="owner" { "owner" }
                        option value="" { "retirer" }
                    }
                }
                button class="tonal" type="submit" { "Mettre à jour l'accès" }
            }
        }
        section class="card" {
            h2 { "Sessions" }
            ul class="list" {
                @for session in sessions {
                    li {
                        span { (session.user) " · " (session.seen) }
                        form method="post" action=(format!("/sessions/{}/revoke", session.id_hash)) {
                            input type="hidden" name="csrf" value=(&chrome.csrf);
                            button class="ghost" type="submit" { "Révoquer" }
                        }
                    }
                }
            }
        }
        section class="card" {
            h2 { "Journal d'audit" }
            p class="muted" { "Ajouts seulement. Rien ne se supprime depuis l'interface." }
            ul class="list" {
                @for row in audit {
                    li { span { (row.at) " · " (row.action) } span class="chip" { (row.outcome) } }
                }
            }
        }
        section class="card" {
            h2 { "Jetons API" }
            p class="muted" { "Un jeton ne voit pas plus que son créateur. Il n'est affiché qu'une fois." }
            @if let Some(token) = revealed {
                div class="banner note" role="status" {
                    p { "Nouveau jeton API, affiché une seule fois." }
                    code class="secret" { (token) }
                }
            }
            form class="stack" method="post" action="/tokens" {
                input type="hidden" name="csrf" value=(&chrome.csrf);
                label class="field" for="token-name" {
                    "Nom"
                    input id="token-name" name="name" required maxlength="80" autocomplete="off";
                }
                label class="field" for="token-scopes" {
                    "Scopes (read, read:live, write, admin)"
                    input id="token-scopes" name="scopes" value="read read:live";
                }
                label class="field" for="token-days" {
                    "Expiration en jours, vide pour aucune"
                    input id="token-days" name="expires_days";
                }
                label class="field" for="token-vehicles" {
                    "Véhicules, vide pour ceux du créateur"
                    input id="token-vehicles" name="vehicle_ids";
                }
                button type="submit" { "Créer un jeton" }
            }
            ul class="list" {
                @for token in tokens {
                    li {
                        span { (token.name) " · " (token.scopes) }
                        span class="row" {
                            @if token.active {
                                span class="chip ok" { "actif" }
                            } @else {
                                span class="chip" { "révoqué" }
                            }
                            form method="post" action=(format!("/tokens/{}/revoke", token.id)) {
                                input type="hidden" name="csrf" value=(&chrome.csrf);
                                button class="ghost" type="submit" { "Révoquer" }
                            }
                        }
                    }
                }
            }
        }
    };
    document(chrome.debug_script, body)
}

pub(super) fn debug_page(chrome: &Chrome, frames: u64, logs: u64, console: &str) -> String {
    let body = html! {
        (toolbar(chrome, "Debug"))
        div id="debug-root" data-frames=(frames.to_string()) data-logs=(logs.to_string()) {
            header class="toolbar" {
                div {
                    h1 class="title" { "Debug" }
                }
                div class="toolbar-actions" {
                    span class="chip" id="live" { "connecting" }
                    button class="ghost" type="button" id="pause" aria-pressed="false" { "Pause" }
                }
            }
            p class="muted" { "Processes, websocket frames, and recent log lines from this process. They stay in memory and reset on restart. Tokens, passwords, and VINs are removed." }
            (PreEscaped(console))
        }
    };
    document(true, body)
}

pub(crate) fn missing_page(chrome: &Chrome) -> String {
    let body = html! {
        (toolbar(chrome, "Véhicule"))
        section class="card" {
            h2 { "Introuvable" }
            p { "Ce véhicule n'est pas dans l'archive." }
            a href="/" { "Retour à la flotte" }
        }
    };
    document(chrome.debug_script, body)
}

fn vehicle_field(vehicle_id: Option<i64>) -> Markup {
    html! {
        @if let Some(id) = vehicle_id {
            input type="hidden" name="vehicle_id" value=(id);
        }
    }
}

pub(crate) fn toolbar(chrome: &Chrome, title: &str) -> Markup {
    let current = chrome.vehicles.iter().find(|vehicle| vehicle.current);
    html! {
        a class="skip-link" href="#contenu" { "Aller au contenu" }
        header class="topbar" {
            div class="topbar-inner" {
                a class="wordmark" href="/" { (brand_mark()) span { "Tesla Logger" } }
                (global_nav(chrome.global))
                form class="logout" method="post" action="/logout" {
                    input type="hidden" name="csrf" value=(&chrome.csrf);
                    button class="ghost small" type="submit" { "Déconnexion" }
                }
            }
        }
        div class="page-head" id="contenu" tabindex="-1" {
            (back_link(chrome))
            div class="page-title" {
                h1 { (title) }
                @if chrome.vehicles.len() >= 2 {
                    nav class="vehicle-select" aria-label="Véhicule" {
                        @for vehicle in &chrome.vehicles {
                            @if vehicle.current {
                                a href=(format!("/v/{}", vehicle.id)) aria-current="page" { (vehicle.name) }
                            } @else {
                                a href=(format!("/v/{}", vehicle.id)) { (vehicle.name) }
                            }
                        }
                    }
                } @else if let Some(vehicle) = current {
                    @if vehicle.name != title {
                        p class="kicker" { (vehicle.name) }
                    }
                }
            }
        }
    }
}

fn back_link(chrome: &Chrome) -> Markup {
    if chrome.global == GlobalTab::None {
        return html! {};
    }
    let (href, label) = back_target(chrome);
    html! {
        a class="back-link" href=(href) { (label) }
    }
}

/// Last viewed car when it is still accessible, otherwise the only car.
/// Several cars and no memory go back to the fleet list.
fn back_target(chrome: &Chrome) -> (String, &'static str) {
    let remembered = chrome
        .remembered
        .filter(|id| chrome.vehicles.iter().any(|vehicle| vehicle.id == *id));
    if let Some(id) = remembered {
        return (format!("/v/{id}"), "← Retour à la voiture");
    }
    if let Some(only) = chrome
        .vehicles
        .first()
        .filter(|_| chrome.vehicles.len() == 1)
    {
        return (format!("/v/{}", only.id), "← Retour à la voiture");
    }
    ("/".to_owned(), "Retour aux voitures")
}

fn global_nav(active: GlobalTab) -> Markup {
    let tabs = [
        (GlobalTab::Settings, "/settings", "Réglages"),
        (GlobalTab::Debug, "/debug", "Debug"),
        (GlobalTab::System, "/system", "Système"),
        (GlobalTab::Security, "/security", "Sécurité"),
    ];
    html! {
        nav class="global-nav" aria-label="Application" {
            @for (tab, href, label) in tabs {
                @if tab == active {
                    a href=(href) aria-current="page" { (label) }
                } @else {
                    a href=(href) { (label) }
                }
            }
        }
    }
}

pub(crate) fn vehicle_tabs(chrome: &Chrome) -> Markup {
    let Some(id) = current_id(chrome) else {
        return html! {};
    };
    let Some(section) = chrome.section else {
        return html! {};
    };
    let tabs = [
        (
            VehicleTab::Overview,
            format!("/v/{id}"),
            "tab-overview",
            "Aperçu",
        ),
        (
            VehicleTab::Drives,
            format!("/v/{id}/drives"),
            "tab-drives",
            "Trajets",
        ),
        (
            VehicleTab::Charges,
            format!("/v/{id}/charges"),
            "tab-charges",
            "Charges",
        ),
        (
            VehicleTab::Stats,
            format!("/v/{id}/stats"),
            "tab-stats",
            "Stats",
        ),
        (
            VehicleTab::Connection,
            format!("/v/{id}/connection"),
            "tab-connection",
            "Connexion Tesla",
        ),
    ];
    let payload: Vec<TabJson<'_>> = tabs
        .iter()
        .map(|(tab, href, tab_id, label)| TabJson {
            id: tab_id,
            label,
            href: href.clone(),
            selected: *tab == section,
        })
        .collect();
    element(
        "tl-tabs",
        &[("label", "Sections du véhicule")],
        Some(&to_json(&payload)),
        html! {},
    )
}

fn current_id(chrome: &Chrome) -> Option<i64> {
    chrome
        .vehicles
        .iter()
        .find(|vehicle| vehicle.current)
        .map(|vehicle| vehicle.id)
}

/// App icon shown before the "Tesla Logger" label in page headers.
fn brand_mark() -> Markup {
    html! {
        img class="brand-mark" src="/favicon-32.png" alt="" width="18" height="18";
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn document(debug_script: bool, body: Markup) -> String {
    html! {
        (DOCTYPE)
        html lang="fr" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover";
                meta name="robots" content="noindex";
                meta name="color-scheme" content="light dark";
                meta name="theme-color" content="#111318" media="(prefers-color-scheme: dark)";
                meta name="theme-color" content="#ffffff" media="(prefers-color-scheme: light)";
                link rel="icon" href="/favicon.ico" sizes="any";
                link rel="icon" href="/favicon-32.png" type="image/png" sizes="32x32";
                link rel="icon" href="/icon-192.png" type="image/png" sizes="192x192";
                link rel="icon" href="/icon-512.png" type="image/png" sizes="512x512";
                link rel="apple-touch-icon" href="/apple-touch-icon.png" sizes="180x180";
                link rel="manifest" href="/site.webmanifest";
                title { "Tesla Logger" }
                link rel="stylesheet" href="/static/ui.css";
                script type="module" src="/static/tesla-logger-ui.js" {}
                script src="/static/tabs.js" defer {}
                @if debug_script {
                    script src="/static/debug.js" defer {}
                }
            }
            body {
                main class="shell" { (body) }
                footer {
                    p {
                        "tesla-logger est un logiciel libre sous licence GNU AGPL-3.0-only. "
                        a href="https://github.com/jcollonville/tesla-rs" { "Code source" }
                        "."
                    }
                }
            }
        }
    }
    .into_string()
}

#[cfg(test)]
mod tests {
    use super::{
        CarCard, Chrome, ConnectionView, FleetCard, GlobalTab, NavVehicle, Overview, Status,
        StatusNotice, VehicleTab, connection_page, fleet_page, login_page, overview_page,
    };

    #[test]
    fn disconnected_never_renders_branchee() {
        for state in ["Disconnected", "disconnected", " DISCONNECTED "] {
            let status = Status::from(Some("online"), Some(state), Some(true));
            assert_ne!(status, Status::Plugged, "{state}");
            assert_ne!(status.label(), "Branchée", "{state}");
            assert_eq!(status, Status::Parked, "{state}");
            assert_eq!(status.label(), "Garée", "{state}");
        }
        let idle = Status::from(None, Some("Disconnected"), Some(true));
        assert_ne!(idle, Status::Plugged);
        assert_ne!(idle.label(), "Branchée");
        assert_eq!(
            Status::from(Some("driving"), Some("Disconnected"), Some(true)),
            Status::Driving
        );
    }

    #[test]
    fn plugged_states_still_render_branchee() {
        for state in ["Complete", "Stopped", "NoPower"] {
            let status = Status::from(Some("online"), Some(state), Some(false));
            assert_eq!(status, Status::Plugged, "{state}");
            assert_eq!(status.label(), "Branchée", "{state}");
        }
        let flagged = Status::from(Some("online"), None, Some(true));
        assert_eq!(flagged, Status::Plugged);
        assert_eq!(flagged.label(), "Branchée");
        assert_eq!(
            Status::from(Some("online"), Some("Charging"), Some(true)),
            Status::Charging
        );
        assert_eq!(
            Status::from(Some("online"), Some("Charging"), Some(true)).label(),
            "En charge"
        );
    }

    #[test]
    fn starting_and_idle_plugged_states_render_branchee_while_charging_stays_en_charge() {
        for state in ["Starting", "starting", "Stopped", "Complete", "NoPower"] {
            let parked = Status::from(Some("online"), Some(state), Some(true));
            assert_eq!(parked, Status::Plugged, "{state}");
            assert_eq!(parked.label(), "Branchée", "{state}");
            let session = Status::from(Some("charging"), Some(state), Some(true));
            assert_eq!(session, Status::Plugged, "activity charging + {state}");
            assert_eq!(session.label(), "Branchée", "{state}");
        }
        assert_eq!(
            Status::from(Some("charging"), Some("Charging"), Some(true)),
            Status::Charging
        );
        assert_eq!(
            Status::from(Some("charging"), Some("Charging"), Some(true)).label(),
            "En charge"
        );
        assert_eq!(
            Status::from(Some("charging"), None, Some(true)),
            Status::Charging,
            "an open charge with no newer state stays En charge"
        );
        assert_eq!(
            Status::from(Some("asleep"), Some("Stopped"), Some(true)),
            Status::Asleep
        );
        assert_eq!(
            Status::from(Some("online"), Some("Disconnected"), Some(true)).label(),
            "Garée"
        );
    }

    #[test]
    fn overview_puts_the_schedule_next_to_the_status() {
        let chrome = Chrome {
            vehicles: vec![vehicle(4, "Faucon")],
            section: Some(VehicleTab::Overview),
            ..chrome()
        };
        let overview = Overview {
            name: "Faucon".to_owned(),
            car: CarCard {
                status: Status::Plugged,
                schedule: Some("Charge programmée à 22:00".to_owned()),
                ..CarCard::default()
            },
            ..Overview::default()
        };
        let html = overview_page(&chrome, &overview);
        assert!(html.contains("Branchée"));
        assert!(html.contains("Charge programmée à 22:00"));
        let bare = overview_page(&chrome, &Overview::default());
        assert!(!bare.contains("Charge programmée"));
        assert!(!bare.contains("hero-schedule"));
    }

    #[test]
    fn pages_link_same_origin_icons_and_show_the_mark_by_the_title() {
        let html = login_page("csrf-token", None);
        let head = html.split("</head>").next().expect("head");
        for href in [
            "/favicon.ico",
            "/favicon-32.png",
            "/apple-touch-icon.png",
            "/icon-192.png",
            "/icon-512.png",
            "/site.webmanifest",
        ] {
            assert!(head.contains(href), "{href} missing from {head}");
        }
        assert!(head.contains("<title>Tesla Logger</title>"));
        assert!(head.contains("name=\"theme-color\" content=\"#111318\""));
        assert!(!head.contains("://"));
        assert!(
            html.contains("<p class=\"kicker\"><img class=\"brand-mark\" src=\"/favicon-32.png\"")
        );
    }

    fn preview(name: &str, html: &str) {
        if let Ok(dir) = std::env::var("UI_PREVIEW_DIR") {
            let _ = std::fs::create_dir_all(&dir);
            let _ = std::fs::write(format!("{dir}/{name}"), html);
        }
    }

    fn chrome() -> Chrome {
        Chrome {
            csrf: "csrf-token".to_owned(),
            vehicles: Vec::new(),
            section: None,
            global: GlobalTab::None,
            debug_script: false,
            remembered: None,
        }
    }

    fn vehicle(id: i64, name: &str) -> NavVehicle {
        NavVehicle {
            id,
            name: name.to_owned(),
            current: false,
        }
    }

    fn global_pages(
        vehicles: &[NavVehicle],
        remembered: Option<i64>,
    ) -> Vec<(&'static str, String)> {
        use super::{SettingRow, TokenLine, debug_page, security_page, settings_page, system_page};
        let page = |global| Chrome {
            vehicles: vehicles.to_vec(),
            global,
            remembered,
            ..chrome()
        };
        vec![
            (
                "settings",
                settings_page(&page(GlobalTab::Settings), &[] as &[SettingRow], None),
            ),
            ("debug", debug_page(&page(GlobalTab::Debug), 0, 0, "")),
            ("system", system_page(&page(GlobalTab::System), "")),
            (
                "security",
                security_page(
                    &page(GlobalTab::Security),
                    &[] as &[TokenLine],
                    None,
                    "admin",
                    &[],
                    &[],
                    &[],
                ),
            ),
        ]
    }

    fn assert_back(name: &str, html: &str, href: &str, label: &str) {
        let needle = format!("<a class=\"back-link\" href=\"{href}\">{label}</a>");
        assert!(html.contains(&needle), "{name} missing {needle}");
    }

    #[test]
    fn login_form_posts_the_password() {
        let html = login_page("csrf-token", Some("Mot de passe incorrect"));
        assert!(html.contains("action=\"/\""));
        assert!(html.contains("name=\"password\""));
        assert!(html.contains("name=\"csrf\" value=\"csrf-token\""));
        assert!(html.contains("Mot de passe incorrect"));
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains("viewport"));
        assert!(html.contains("/static/ui.css"));
        preview("login.html", &html);
    }

    #[test]
    fn fleet_cards_escape_names_and_link_vehicles() {
        let chrome = Chrome {
            vehicles: vec![NavVehicle {
                id: 7,
                name: "Red".to_owned(),
                current: false,
            }],
            ..chrome()
        };
        let cards = [FleetCard {
            id: 7,
            name: "Red <script>".to_owned(),
            activity: "driving".to_owned(),
            battery: "72%".to_owned(),
            last_seen: "—".to_owned(),
            last_error: Some("poll failed".to_owned()),
            month: "1.0 km".to_owned(),
            ..FleetCard::default()
        }];
        let html = fleet_page(&chrome, &cards, "1.0 km · 0.0 kWh");
        assert!(html.contains("href=\"/v/7\""));
        assert!(html.contains("Red &lt;script&gt;"));
        assert!(!html.contains("Red <script>"));
        assert!(html.contains("poll failed"));
        let warned = [FleetCard {
            id: 7,
            name: "Red".to_owned(),
            telemetry_notice: Some(StatusNotice {
                role: "alert",
                class: "banner bad",
                icon: "!",
                text: "La télémétrie est en erreur.".to_owned(),
            }),
            ..FleetCard::default()
        }];
        let banner = fleet_page(&chrome, &warned, "1.0 km");
        assert!(banner.contains("role=\"alert\""));
        assert!(banner.contains("La télémétrie est en erreur."));
        assert!(banner.contains("href=\"/connection\">Connexion</a>"));
        assert!(banner.contains("aria-hidden=\"true\">!"));
        assert!(html.contains("72%"));
        assert!(!html.contains("Mode GPS"));
        assert!(html.contains("href=\"/settings\""));
        assert!(html.contains("href=\"/debug\""));
        assert!(html.contains("action=\"/logout\""));
        preview("fleet.html", &html);
    }

    #[test]
    fn overview_marks_the_selected_tab() {
        let chrome = Chrome {
            vehicles: vec![NavVehicle {
                id: 4,
                name: "Faucon".to_owned(),
                current: true,
            }],
            section: Some(VehicleTab::Overview),
            ..chrome()
        };
        let html = overview_page(
            &chrome,
            &Overview {
                name: "Faucon".to_owned(),
                activity: "online".to_owned(),
                battery: "80%".to_owned(),
                last_poll: "jamais".to_owned(),
                last_data: "jamais".to_owned(),
                last_error: "—".to_owned(),
                last_drive: None,
                last_charge: None,
                ..Overview::default()
            },
        );
        assert!(html.contains("<tl-tabs"));
        assert!(html.contains("\"selected\":true"));
        assert!(html.contains("Aperçu"));
        assert!(!html.contains("Mode GPS"));
        assert!(!html.contains("stream"));
        assert!(!html.contains("<svg class=\"car\""));
        assert!(html.contains("Image du véhicule indisponible"));
        assert!(html.contains("Aucun trajet"));
        assert!(html.contains("link-href=\"/v/4/drives\""));
        assert!(html.contains("/v/4/connection"));
        preview("overview.html", &html);
    }

    #[test]
    fn connection_page_has_no_owner_step_and_no_secrets() {
        let chrome = Chrome {
            vehicles: vec![NavVehicle {
                id: 4,
                name: "Faucon".to_owned(),
                current: true,
            }],
            section: Some(VehicleTab::Connection),
            ..chrome()
        };
        let html = connection_page(
            &chrome,
            &ConnectionView {
                admin: true,
                sudo: false,
                shared_names: vec!["Millenium".to_owned()],
                shared_ok: true,
                csrf: "csrf-token",
                vehicle_id: Some(4),
                error: Some("Fleet sign-in failed"),
                notice: None,
                report: crate::connection::Report::blank(),
            },
        );
        assert!(!html.contains("void/callback"));
        assert!(html.contains("href=\"/debug\""));
        assert!(html.contains("<ol class=\"setup\" start=\"0\""));
        assert!(html.contains("aria-describedby=\"step-help-0\""));
        assert!(html.contains("role=\"tooltip\""));
        assert!(html.contains("Validé"));
        assert!(html.contains("2 étapes sur 5 validées"));
        assert!(html.contains("Enregistrer le domaine"));
        assert!(html.contains("bloquée"));
        assert!(html.contains("Nécessite l'étape 2."));
        assert!(html.contains("facultative"));
        assert!(!html.contains("href=\"/auth/login\""));
        assert!(!html.contains("action=\"/auth/refresh\""));
        assert!(!html.contains("action=\"/auth/disconnect\""));
        assert!(html.contains("sudo"));
        assert!(!html.contains("/auth/owner"));
        assert!(!html.contains("Owner"));
        assert!(!html.contains("name=\"redirect_url\""));
        assert!(html.contains("name=\"csrf\" value=\"csrf-token\""));
        assert!(html.contains("name=\"vehicle_id\" value=\"4\""));
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains("Ce compte gère aussi"));
        assert!(html.contains("Millenium"));
        assert!(!html.contains("OWNER_STREAM_ENABLED"));
        assert!(html.contains("<tl-tabs"));
        assert!(html.contains("\"selected\":true"));
        assert!(html.contains("Connexion Tesla"));
        preview("connection.html", &html);
    }

    fn connection_html(facts: &crate::connection::Input) -> String {
        connection_page(
            &Chrome {
                section: Some(VehicleTab::Connection),
                ..chrome()
            },
            &ConnectionView {
                admin: true,
                sudo: true,
                shared_names: Vec::new(),
                shared_ok: true,
                csrf: "csrf-token",
                vehicle_id: None,
                error: None,
                notice: None,
                report: crate::connection::build(facts),
            },
        )
    }

    #[test]
    fn connection_page_shows_each_component_with_one_action() {
        let mut facts = crate::connection::Input {
            now: chrono::Utc::now(),
            fleet_present: true,
            fleet_readable: true,
            fleet_expires: Some(chrono::Utc::now() + chrono::Duration::hours(2)),
            scopes: crate::connection::test_scopes(),
            refresh_failed: false,
            partner: crate::connection::PartnerState::Registered,
            public_key_published: true,
            domain: "logger.example".to_owned(),
            redirect_uri: "https://logger.example/auth/callback".to_owned(),
            client_configured: true,
            client_id: "logger-app".to_owned(),
            region: tesla_auth::Region::Europe,
            fingerprint: "sha256:abcd".to_owned(),
            telemetry_enabled: true,
            telemetry_port: 4443,
            fresh_for: std::time::Duration::from_secs(900),
            is_admin: true,
            vehicles: vec![crate::connection::VehicleInput {
                id: 4,
                name: "Faucon".to_owned(),
                key: crate::connection::KeyState::Unpaired,
                skip_reason: Some("missing_key".to_owned()),
                config_pushed: false,
                last_message_age: None,
                health: "Ignorée".to_owned(),
                health_tone: "bad",
            }],
        };
        let open = connection_html(&facts);
        preview("connection-open.html", &open);
        assert!(open.contains("https://tesla.com/_ak/logger.example"));
        assert!(open.contains("Pas appariée"));
        assert!(open.contains("action=\"/auth/disconnect\""));
        assert!(open.contains("Déconnecter mon compte Tesla"));
        assert!(open.contains("ne fournit pas d'endpoint serveur"));
        assert!(open.contains("applications tierces"));
        assert!(!open.contains("href=\"/auth/login\""));
        assert!(!open.contains("action=\"/auth/refresh\""));
        assert!(!open.contains("action=\"/connection/partner\""));
        assert!(!open.contains("action=\"/connection/telemetry\""));
        assert!(open.contains("Supprimer les données de ce véhicule"));
        assert!(open.contains("action=\"/v/4/delete\""));
        assert!(open.contains("<svg"));
        assert!(!open.contains("BEGIN EC PRIVATE KEY"));
        assert!(!open.contains("logger-app-secret"));

        facts.fleet_expires = Some(facts.now + chrono::Duration::minutes(5));
        let expiring = connection_html(&facts);
        preview("connection-expiring.html", &expiring);
        assert!(expiring.contains("action=\"/auth/refresh\""));
        assert!(expiring.contains("Rafraîchir"));

        facts.partner = crate::connection::PartnerState::Missing;
        facts.fleet_present = false;
        facts.fleet_expires = None;
        let early = connection_html(&facts);
        preview("connection-early.html", &early);
        assert!(early.contains("Pas enregistré") || early.contains("n'est pas enregistré"));
        assert!(early.contains("action=\"/connection/partner\""));
        assert!(early.contains("Enregistrer le domaine"));
        assert!(!early.contains("href=\"/auth/login\""));
        assert!(!early.contains("https://tesla.com/_ak/logger.example"));
        assert!(!early.contains("Owner"));
        assert!(early.contains("name=\"csrf\" value=\"csrf-token\""));
    }

    #[test]
    fn global_pages_link_back_to_the_only_vehicle() {
        for (name, html) in global_pages(&[vehicle(4, "Faucon")], None) {
            assert_back(name, &html, "/v/4", "← Retour à la voiture");
        }
    }

    #[test]
    fn global_pages_link_back_to_the_remembered_vehicle() {
        let vehicles = vec![vehicle(4, "Faucon"), vehicle(9, "Millenium")];
        for (name, html) in global_pages(&vehicles, Some(9)) {
            assert_back(name, &html, "/v/9", "← Retour à la voiture");
        }
    }

    #[test]
    fn global_pages_fall_back_to_the_fleet_without_a_remembered_vehicle() {
        let several = vec![vehicle(4, "Faucon"), vehicle(9, "Millenium")];
        for (name, html) in global_pages(&several, None) {
            assert_back(name, &html, "/", "Retour aux voitures");
            assert!(!html.contains("Retour à la voiture"), "{name} named a car");
        }
        for (name, html) in global_pages(&[], None) {
            assert_back(name, &html, "/", "Retour aux voitures");
        }
        let stale = vec![vehicle(4, "Faucon"), vehicle(9, "Millenium")];
        for (name, html) in global_pages(&stale, Some(99)) {
            assert_back(name, &html, "/", "Retour aux voitures");
        }
    }

    #[test]
    fn vehicle_pages_omit_the_back_link() {
        let chrome = Chrome {
            vehicles: vec![vehicle(4, "Faucon")],
            section: Some(VehicleTab::Overview),
            remembered: Some(4),
            ..chrome()
        };
        let overview = overview_page(
            &chrome,
            &Overview {
                name: "Faucon".to_owned(),
                activity: "online".to_owned(),
                battery: "80%".to_owned(),
                last_poll: "jamais".to_owned(),
                last_data: "jamais".to_owned(),
                last_error: "—".to_owned(),
                last_drive: None,
                last_charge: None,
                ..Overview::default()
            },
        );
        let drives = super::placeholder_page(&chrome, "Trajets", "bientôt", None);
        let connection = connection_page(
            &Chrome {
                section: Some(VehicleTab::Connection),
                ..chrome
            },
            &ConnectionView {
                admin: false,
                sudo: false,
                shared_names: Vec::new(),
                shared_ok: true,
                csrf: "csrf-token",
                vehicle_id: Some(4),
                error: None,
                notice: None,
                report: crate::connection::Report::blank(),
            },
        );
        for (name, html) in [
            ("overview", overview),
            ("drives", drives),
            ("connection", connection),
        ] {
            assert!(!html.contains("back-link"), "{name}");
            assert!(!html.contains("Retour à la voiture"), "{name}");
            assert!(!html.contains("Retour aux voitures"), "{name}");
        }
    }

    #[test]
    fn every_page_uses_the_same_content_column() {
        use super::{
            debug_page, missing_page, placeholder_page, security_page, settings_page, system_page,
        };
        let vehicle_chrome = Chrome {
            vehicles: vec![vehicle(4, "Faucon")],
            section: Some(VehicleTab::Overview),
            ..chrome()
        };
        let connection_chrome = Chrome {
            section: Some(VehicleTab::Connection),
            ..vehicle_chrome.clone()
        };
        let pages = [
            ("login", login_page("csrf-token", None)),
            ("fleet", fleet_page(&chrome(), &[], "0")),
            (
                "overview",
                overview_page(
                    &vehicle_chrome,
                    &Overview {
                        name: "Faucon".to_owned(),
                        activity: "online".to_owned(),
                        battery: "80%".to_owned(),
                        last_poll: "—".to_owned(),
                        last_data: "—".to_owned(),
                        last_error: "—".to_owned(),
                        last_drive: None,
                        last_charge: None,
                        ..Overview::default()
                    },
                ),
            ),
            (
                "drives",
                placeholder_page(&vehicle_chrome, "Trajets", "bientôt", None),
            ),
            (
                "connection",
                connection_page(
                    &connection_chrome,
                    &ConnectionView {
                        admin: false,
                        sudo: false,
                        shared_names: Vec::new(),
                        shared_ok: true,
                        csrf: "csrf-token",
                        vehicle_id: Some(4),
                        error: None,
                        notice: None,
                        report: crate::connection::Report::blank(),
                    },
                ),
            ),
            (
                "settings",
                settings_page(
                    &Chrome {
                        global: GlobalTab::Settings,
                        ..chrome()
                    },
                    &[],
                    None,
                ),
            ),
            (
                "debug",
                debug_page(
                    &Chrome {
                        global: GlobalTab::Debug,
                        debug_script: true,
                        ..chrome()
                    },
                    1,
                    1,
                    "",
                ),
            ),
            (
                "system",
                system_page(
                    &Chrome {
                        global: GlobalTab::System,
                        ..chrome()
                    },
                    "",
                ),
            ),
            (
                "security",
                security_page(
                    &Chrome {
                        global: GlobalTab::Security,
                        ..chrome()
                    },
                    &[],
                    None,
                    "admin",
                    &[],
                    &[],
                    &[],
                ),
            ),
            ("missing", missing_page(&chrome())),
        ];
        for (name, html) in pages {
            assert!(
                html.contains("<main class=\"shell\">"),
                "{name} is outside the shared column: {html}"
            );
            assert!(
                !html.contains("shell wide"),
                "{name} overrides the column width"
            );
            assert!(!html.contains("max-width"), "{name} sets its own width");
        }
        let css = include_str!("ui.css");
        assert!(!css.contains(".shell.wide"), "CSS still widens one page");
        assert!(!css.contains("72rem"), "debug column width remains");
        if let Some(rule) = css.split(".auth-card").nth(1) {
            let body = rule.split('}').next().unwrap_or(rule);
            assert!(
                !body.contains("width"),
                "sign-in card sets its own width: {body}"
            );
        }
    }
}
