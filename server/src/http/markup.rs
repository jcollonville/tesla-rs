// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Compile-time HTML. Maud escapes text and attributes.

use maud::{DOCTYPE, Markup, PreEscaped, html};

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

pub(super) struct NavVehicle {
    pub id: i64,
    pub name: String,
    pub current: bool,
}

pub(super) struct Chrome {
    pub csrf: String,
    pub vehicles: Vec<NavVehicle>,
    pub section: Option<VehicleTab>,
    pub global: GlobalTab,
    pub wide: bool,
    pub debug_script: bool,
}

pub(super) struct FleetCard {
    pub id: i64,
    pub name: String,
    pub activity: String,
    pub battery: String,
    pub position_mode: String,
    pub last_seen: String,
    pub last_error: Option<String>,
    pub month: String,
}

pub(super) struct Overview {
    pub name: String,
    pub activity: String,
    pub battery: String,
    pub position_mode: String,
    pub last_poll: String,
    pub last_data: String,
    pub last_error: String,
    pub last_drive: String,
    pub last_charge: String,
}

pub(super) struct Party {
    pub title: String,
    pub status: String,
    pub tone: &'static str,
    pub expires: String,
    pub scopes: String,
    pub last_refresh: String,
    pub last_error: Option<String>,
    pub connected: bool,
}

pub(super) struct ConnectionView<'a> {
    pub fleet: Party,
    pub owner_status: String,
    pub owner_tone: &'static str,
    pub owner_expires: String,
    pub owner_scopes: String,
    pub owner_last_refresh: String,
    pub owner_last_error: Option<String>,
    pub owner_connected: bool,
    pub fingerprint: String,
    pub shared_names: Vec<String>,
    pub shared_ok: bool,
    pub csrf: &'a str,
    pub vehicle_id: Option<i64>,
    pub error: Option<&'a str>,
    pub notice: Option<&'a str>,
    pub stream_status: &'a str,
    pub streaming_enabled: bool,
    pub authorize_url: Option<&'a str>,
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
}

pub(super) fn login_page(csrf: &str, error: Option<&str>) -> String {
    document(
        false,
        false,
        html! {
            section class="card auth-card" {
                p class="kicker" { "tesla-logger" }
                h1 { "Connexion" }
                p class="muted" { "Mot de passe de ce logger. Ce n'est pas le compte Tesla." }
                @if let Some(error) = error {
                    div class="banner bad" role="alert" { p { (error) } }
                }
                form class="stack" method="post" action="/" {
                    input type="hidden" name="csrf" value=(csrf);
                    label class="field" for="password" {
                        "Mot de passe"
                        input id="password" type="password" name="password" autocomplete="current-password" required;
                    }
                    button type="submit" { "Se connecter" }
                }
            }
        },
    )
}

pub(super) fn fleet_page(chrome: &Chrome, cards: &[FleetCard], month_total: &str) -> String {
    let body = html! {
        (toolbar(chrome, "Flotte"))
        p class="muted" { "Totaux du mois : " (month_total) }
        @if cards.is_empty() {
            section class="card" {
                h2 { "Véhicules" }
                p class="muted" { "Aucun véhicule pour l'instant." }
                a class="button" href="/connection" { "Connexion Tesla" }
            }
        } @else {
            div class="vehicles" {
                @for card in cards {
                    article class="vehicle" {
                        div class="vehicle-head" {
                            h3 { a href=(format!("/v/{}", card.id)) { (card.name) } }
                            span class="chip" { (card.activity) }
                        }
                        dl class="facts" {
                            div { dt { "Batterie" } dd { (card.battery) } }
                            div { dt { "Mode GPS" } dd { (card.position_mode) } }
                            div { dt { "Dernière activité" } dd { (card.last_seen) } }
                            div { dt { "Ce mois" } dd { (card.month) } }
                            @if let Some(error) = &card.last_error {
                                div { dt { "Dernière erreur" } dd { (error) } }
                            }
                        }
                    }
                }
            }
        }
    };
    document(chrome.wide, chrome.debug_script, body)
}

pub(super) fn overview_page(chrome: &Chrome, overview: &Overview) -> String {
    let body = html! {
        (toolbar(chrome, &overview.name))
        (vehicle_tabs(chrome))
        div id="panel-overview" role="tabpanel" aria-labelledby="tab-overview" {
            section class="card" {
                h2 { "Aperçu" }
                dl class="facts" {
                    div { dt { "Activité" } dd { (overview.activity) } }
                    div { dt { "Batterie" } dd { (overview.battery) } }
                    div { dt { "Mode GPS" } dd { (overview.position_mode) } }
                    div { dt { "Dernier poll" } dd { (overview.last_poll) } }
                    div { dt { "Dernière donnée" } dd { (overview.last_data) } }
                    div { dt { "Dernière erreur" } dd { (overview.last_error) } }
                }
            }
            section class="card" id="dernier-trajet" {
                h2 { "Dernier trajet" }
                p { (overview.last_drive) }
                @if let Some(id) = current_id(chrome) {
                    p { a href=(format!("/v/{id}/drives")) { "Ouvrir Trajets" } }
                }
            }
            section class="card" id="derniere-charge" {
                h2 { "Dernière charge" }
                p { (overview.last_charge) }
                @if let Some(id) = current_id(chrome) {
                    p { a href=(format!("/v/{id}/charges")) { "Ouvrir Charges" } }
                }
            }
        }
    };
    document(chrome.wide, chrome.debug_script, body)
}

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
    document(chrome.wide, chrome.debug_script, body)
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
            (party_card(&view.fleet))
            section class="card" {
                h2 { "Fleet API" }
                div class="toolbar-actions" {
                    a class="button" href="/auth/login" { "Connecter" }
                    form method="post" action="/auth/refresh" {
                        input type="hidden" name="csrf" value=(view.csrf);
                        (vehicle_field(view.vehicle_id))
                        button type="submit" { "Rafraîchir" }
                    }
                    form method="post" action="/auth/disconnect" {
                        input type="hidden" name="csrf" value=(view.csrf);
                        (vehicle_field(view.vehicle_id))
                        button class="danger" type="submit" { "Déconnecter" }
                    }
                }
            }
            section class="card" lang="en" {
                h2 { "Owner API streaming" }
                p { "This uses Tesla's legacy Owner API and streaming socket. They are unofficial and Tesla may shut them down. Fleet API polling stays the source for everything else. The stream adds about one position per second while driving." }
                div class="meta" {
                    span class=(format!("chip {}", stream_tone(view.stream_status))) {
                        "Stream " (view.stream_status)
                    }
                    @if view.owner_connected {
                        span class="chip ok" { "Owner token stored" }
                    } @else {
                        span class="chip" { "No owner token" }
                    }
                }
                @if !view.streaming_enabled {
                    div class="banner note" role="status" {
                        p { "Streaming is off because OWNER_STREAM_ENABLED is false. Tokens can still be saved." }
                    }
                }
                ol class="steps" {
                    li { "Choose " strong { "Prepare Tesla sign-in" } "." }
                    li { "Open the Tesla link and sign in with the account that owns the car. The link opens in a new tab so this page stays here." }
                    li { "The browser cannot open " code { "tesla://" } ". It may say the address is not understood, the site cannot be reached, or offer to open the Tesla app. Do not open the Tesla app." }
                    li { "Copy the full URL from the address bar. It starts with " code { "tesla://auth/callback?" } ". If Chrome drops it, the Network tab in developer tools still lists the failed navigation." }
                    li { "Paste that URL into " strong { "Redirect URL" } " and choose " strong { "Save sign-in" } ". The field is masked so the authorization code is not displayed." }
                }
                form class="stack" method="post" action="/auth/owner/start" {
                    input type="hidden" name="csrf" value=(view.csrf);
                    (vehicle_field(view.vehicle_id))
                    button type="submit" { "Prepare Tesla sign-in" }
                }
                @if let Some(url) = view.authorize_url {
                    p class="stack" {
                        a class="button" href=(url) rel="noreferrer noopener" target="_blank" { "Open the Tesla sign-in" }
                    }
                } @else {
                    p class="muted" { "Prepare a sign-in to get the Tesla link." }
                }
                form class="stack" method="post" action="/auth/owner/callback" {
                    input type="hidden" name="csrf" value=(view.csrf);
                    (vehicle_field(view.vehicle_id))
                    label class="field" for="redirect_url" {
                        "Redirect URL"
                        input id="redirect_url" type="password" name="redirect_url" autocomplete="off" spellcheck="false" maxlength="8000" required;
                    }
                    button type="submit" { "Save sign-in" }
                }
                h2 { "Refresh token" }
                p { "Paste a refresh token created for this logger. Do not reuse TeslaMate's refresh token. Tesla rotates refresh tokens, and using TeslaMate's token here can sign TeslaMate out." }
                form class="stack" method="post" action="/auth/owner/refresh" {
                    input type="hidden" name="csrf" value=(view.csrf);
                    (vehicle_field(view.vehicle_id))
                    label class="field" for="refresh_token" {
                        "Owner API refresh token"
                        input id="refresh_token" type="password" name="refresh_token" autocomplete="off" spellcheck="false" maxlength="8000" required;
                    }
                    button type="submit" { "Save refresh token" }
                }
                h2 { "Disconnect" }
                p class="muted" { "Removes the stored Owner API token. Fleet API polling is unchanged." }
                form method="post" action="/auth/owner/disconnect" {
                    input type="hidden" name="csrf" value=(view.csrf);
                    (vehicle_field(view.vehicle_id))
                    button class="danger" type="submit" { "Disconnect Owner API" }
                }
            }
            section class="card" {
                h2 { "Owner API" }
                dl class="facts" {
                    div { dt { "État" } dd { span class=(format!("chip {}", view.owner_tone)) { (view.owner_status) } } }
                    div { dt { "Expiration du jeton" } dd { (view.owner_expires) } }
                    div { dt { "Scopes" } dd { (view.owner_scopes) } }
                    div { dt { "Dernier rafraîchissement" } dd { (view.owner_last_refresh) } }
                    @if let Some(error) = &view.owner_last_error {
                        div { dt { "Dernière erreur" } dd { (error) } }
                    }
                }
            }
            section class="card" {
                h2 { "Clé publique" }
                dl class="facts" {
                    div { dt { "Empreinte" } dd { code { (view.fingerprint) } } }
                }
                p class="muted" { "Aucun secret n'est affiché : ni jeton, ni clé privée." }
            }
        }
    };
    document(chrome.wide, chrome.debug_script, body)
}

pub(super) fn settings_page(chrome: &Chrome, rows: &[SettingRow]) -> String {
    let body = html! {
        (toolbar(chrome, "Réglages"))
        section class="card" {
            h2 { "Configuration effective" }
            p class="muted" { "Lecture seule. La source est l'environnement, ou la valeur par défaut du programme. Les secrets restent dans l'environnement et ne sont pas affichés." }
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
    };
    document(chrome.wide, chrome.debug_script, body)
}

pub(super) fn system_page(chrome: &Chrome) -> String {
    let body = html! {
        (toolbar(chrome, "Système"))
        section class="card" {
            h2 { "Système" }
            p { "La taille de la base, les migrations, les tâches de fond et la sauvegarde arrivent dans une phase suivante." }
        }
    };
    document(chrome.wide, chrome.debug_script, body)
}

pub(super) fn security_page(
    chrome: &Chrome,
    tokens: &[TokenLine],
    revealed: Option<&str>,
) -> String {
    let body = html! {
        (toolbar(chrome, "Sécurité"))
        section class="card" {
            h2 { "Jetons API" }
            p class="muted" { "Les comptes, les sessions et le journal d'audit arrivent ensuite. Les jetons API existants restent gérés ici." }
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
                button type="submit" { "Créer un jeton" }
            }
            ul class="list" {
                @for token in tokens {
                    li {
                        span { (token.name) }
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
    document(chrome.wide, chrome.debug_script, body)
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
    document(true, true, body)
}

pub(super) fn missing_page(chrome: &Chrome) -> String {
    let body = html! {
        (toolbar(chrome, "Véhicule"))
        section class="card" {
            h2 { "Introuvable" }
            p { "Ce véhicule n'est pas dans l'archive." }
            a href="/" { "Retour à la flotte" }
        }
    };
    document(chrome.wide, chrome.debug_script, body)
}

fn party_card(party: &Party) -> Markup {
    html! {
        section class="card" {
            div class="vehicle-head" {
                h2 { (party.title) }
                span class=(format!("chip {}", party.tone)) { (party.status) }
            }
            dl class="facts" {
                div { dt { "Expiration du jeton" } dd { (party.expires) } }
                div { dt { "Scopes" } dd { (party.scopes) } }
                div { dt { "Jeton" } dd { (if party.connected { "enregistré" } else { "absent" }) } }
                div { dt { "Dernier rafraîchissement" } dd { (party.last_refresh) } }
                @if let Some(error) = &party.last_error {
                    div { dt { "Dernière erreur" } dd { (error) } }
                }
            }
        }
    }
}

fn vehicle_field(vehicle_id: Option<i64>) -> Markup {
    html! {
        @if let Some(id) = vehicle_id {
            input type="hidden" name="vehicle_id" value=(id);
        }
    }
}

fn toolbar(chrome: &Chrome, title: &str) -> Markup {
    let current = chrome.vehicles.iter().find(|vehicle| vehicle.current);
    html! {
        header class="toolbar" {
            div {
                p class="kicker" { a href="/" { "tesla-logger" } }
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
                    p class="kicker" { "Véhicule " (vehicle.name) }
                }
            }
            div class="toolbar-actions" {
                (global_nav(chrome.global))
                form method="post" action="/logout" {
                    input type="hidden" name="csrf" value=(&chrome.csrf);
                    button class="ghost" type="submit" { "Déconnexion" }
                }
            }
        }
    }
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

fn vehicle_tabs(chrome: &Chrome) -> Markup {
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
    html! {
        nav class="tabs" role="tablist" aria-label="Sections du véhicule" {
            @for (tab, href, tab_id, label) in tabs {
                @if tab == section {
                    a id=(tab_id) role="tab" href=(href) aria-selected="true" aria-current="page" { (label) }
                } @else {
                    a id=(tab_id) role="tab" href=(href) aria-selected="false" { (label) }
                }
            }
        }
    }
}

fn current_id(chrome: &Chrome) -> Option<i64> {
    chrome
        .vehicles
        .iter()
        .find(|vehicle| vehicle.current)
        .map(|vehicle| vehicle.id)
}

fn stream_tone(status: &str) -> &'static str {
    match status {
        "connected" => "ok",
        "error" => "bad",
        _ => "neutral",
    }
}

#[allow(clippy::needless_pass_by_value)]
fn document(wide: bool, debug_script: bool, body: Markup) -> String {
    let class = if wide { "shell wide" } else { "shell" };
    html! {
        (DOCTYPE)
        html lang="fr" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="robots" content="noindex";
                title { "tesla-logger" }
                link rel="stylesheet" href="/static/ui.css";
                script src="/static/tabs.js" defer {}
                @if debug_script {
                    script src="/static/debug.js" defer {}
                }
            }
            body {
                main class=(class) { (body) }
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
        Chrome, ConnectionView, FleetCard, GlobalTab, NavVehicle, Overview, Party, VehicleTab,
        connection_page, fleet_page, login_page, overview_page,
    };

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
            wide: false,
            debug_script: false,
        }
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
            position_mode: "fallback".to_owned(),
            last_seen: "—".to_owned(),
            last_error: Some("poll failed".to_owned()),
            month: "1.0 km".to_owned(),
        }];
        let html = fleet_page(&chrome, &cards, "1.0 km · 0.0 kWh");
        assert!(html.contains("href=\"/v/7\""));
        assert!(html.contains("Red &lt;script&gt;"));
        assert!(!html.contains("Red <script>"));
        assert!(html.contains("poll failed"));
        assert!(html.contains("72%"));
        assert!(html.contains("fallback"));
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
                position_mode: "hybrid".to_owned(),
                last_poll: "jamais".to_owned(),
                last_data: "jamais".to_owned(),
                last_error: "—".to_owned(),
                last_drive: "Aucun trajet".to_owned(),
                last_charge: "Aucune charge".to_owned(),
            },
        );
        assert!(html.contains("role=\"tablist\""));
        assert!(html.contains("aria-selected=\"true\""));
        assert!(html.contains(">Aperçu<"));
        assert!(html.contains("hybrid"));
        assert!(html.contains("href=\"/v/4/drives\""));
        assert!(html.contains("href=\"/v/4/connection\""));
        preview("overview.html", &html);
    }

    #[test]
    fn connection_page_groups_fleet_and_owner_without_secrets() {
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
                fleet: Party {
                    title: "Compte Fleet".to_owned(),
                    status: "connecté".to_owned(),
                    tone: "ok",
                    expires: "2026-01-01".to_owned(),
                    scopes: "openid vehicle_device_data".to_owned(),
                    last_refresh: "—".to_owned(),
                    last_error: None,
                    connected: true,
                },
                owner_status: "déconnecté".to_owned(),
                owner_tone: "neutral",
                owner_expires: "—".to_owned(),
                owner_scopes: "—".to_owned(),
                owner_last_refresh: "—".to_owned(),
                owner_last_error: None,
                owner_connected: false,
                fingerprint: "sha256:abcd".to_owned(),
                shared_names: vec!["Millenium".to_owned()],
                shared_ok: true,
                csrf: "csrf-token",
                vehicle_id: Some(4),
                error: Some("Owner API sign-in failed"),
                notice: None,
                stream_status: "disabled",
                streaming_enabled: true,
                authorize_url: Some(
                    "https://auth.tesla.com/oauth2/v3/authorize?client_id=ownerapi&redirect_uri=tesla%3A%2F%2Fauth%2Fcallback",
                ),
            },
        );
        assert!(html.contains("tesla://auth/callback?"));
        assert!(html.contains("Do not open the Tesla app"));
        assert!(html.contains("Network tab"));
        assert!(html.contains("Do not reuse TeslaMate's refresh token"));
        assert!(!html.contains("void/callback"));
        assert!(html.contains("href=\"/debug\""));
        assert!(html.contains("href=\"/auth/login\""));
        assert!(html.contains("action=\"/auth/refresh\""));
        assert!(html.contains("action=\"/auth/disconnect\""));
        assert!(html.contains("action=\"/auth/owner/start\""));
        assert!(html.contains("action=\"/auth/owner/callback\""));
        assert!(html.contains("action=\"/auth/owner/refresh\""));
        assert!(html.contains("action=\"/auth/owner/disconnect\""));
        assert!(html.contains("name=\"redirect_url\""));
        assert!(html.contains("name=\"refresh_token\""));
        assert!(html.contains("name=\"csrf\" value=\"csrf-token\""));
        assert!(html.contains("name=\"vehicle_id\" value=\"4\""));
        assert!(html.contains("redirect_uri=tesla%3A%2F%2Fauth%2Fcallback"));
        assert!(html.contains("&amp;redirect_uri="));
        assert!(html.contains("role=\"alert\""));
        assert!(html.contains("sha256:abcd"));
        assert!(html.contains("Ce compte gère aussi"));
        assert!(html.contains("Millenium"));
        assert!(!html.contains("OWNER_STREAM_ENABLED"));
        assert!(html.contains("aria-selected=\"true\""));
        preview("connection.html", &html);
    }
}
