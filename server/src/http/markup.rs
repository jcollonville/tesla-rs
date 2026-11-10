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
    pub stream_lag: String,
    pub stream_queue: String,
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
    document(chrome.debug_script, body)
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
                    div { dt { "Retard stream" } dd { (overview.stream_lag) } }
                    div { dt { "File stream" } dd { (overview.stream_queue) } }
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
            (party_card(&view.fleet))
            section class="card" {
                h2 { "Fleet API" }
                div class="toolbar-actions" {
                    a class="button" href="/auth/login" { "Connecter" }
                    form method="post" action="/auth/refresh" {
                        input type="hidden" name="csrf" value=(view.csrf);
                        (vehicle_field(view.vehicle_id))
                        button class="tonal" type="submit" { "Rafraîchir" }
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
    document(chrome.debug_script, body)
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
    document(chrome.debug_script, body)
}

pub(super) fn system_page(chrome: &Chrome) -> String {
    let body = html! {
        (toolbar(chrome, "Système"))
        section class="card" {
            h2 { "Système" }
            p { "La taille de la base, les migrations, les tâches de fond et la sauvegarde arrivent dans une phase suivante." }
        }
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

pub(crate) fn toolbar(chrome: &Chrome, title: &str) -> Markup {
    let current = chrome.vehicles.iter().find(|vehicle| vehicle.current);
    html! {
        header class="toolbar app-bar" {
            (back_link(chrome))
            div class="app-bar-title" {
                p class="kicker" { (brand_mark()) a href="/" { "Tesla Logger" } }
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
                meta name="viewport" content="width=device-width, initial-scale=1";
                meta name="robots" content="noindex";
                meta name="theme-color" content="#111318";
                link rel="icon" href="/favicon.ico" sizes="any";
                link rel="icon" href="/favicon-32.png" type="image/png" sizes="32x32";
                link rel="icon" href="/icon-192.png" type="image/png" sizes="192x192";
                link rel="icon" href="/icon-512.png" type="image/png" sizes="512x512";
                link rel="apple-touch-icon" href="/apple-touch-icon.png" sizes="180x180";
                link rel="manifest" href="/site.webmanifest";
                title { "Tesla Logger" }
                link rel="stylesheet" href="/static/ui.css";
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
        Chrome, ConnectionView, FleetCard, GlobalTab, NavVehicle, Overview, Party, VehicleTab,
        connection_page, fleet_page, login_page, overview_page,
    };

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
                settings_page(&page(GlobalTab::Settings), &[] as &[SettingRow]),
            ),
            ("debug", debug_page(&page(GlobalTab::Debug), 0, 0, "")),
            ("system", system_page(&page(GlobalTab::System))),
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
                stream_lag: "3 s".to_owned(),
                stream_queue: "0".to_owned(),
                last_drive: "Aucun trajet".to_owned(),
                last_charge: "Aucune charge".to_owned(),
            },
        );
        assert!(html.contains("role=\"tablist\""));
        assert!(html.contains("aria-selected=\"true\""));
        assert!(html.contains(">Aperçu<"));
        assert!(html.contains("hybrid"));
        assert!(html.contains("Retard stream"));
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
                position_mode: "hybrid".to_owned(),
                last_poll: "jamais".to_owned(),
                last_data: "jamais".to_owned(),
                last_error: "—".to_owned(),
                stream_lag: "3 s".to_owned(),
                stream_queue: "0".to_owned(),
                last_drive: "Aucun trajet".to_owned(),
                last_charge: "Aucune charge".to_owned(),
            },
        );
        let drives = super::placeholder_page(&chrome, "Trajets", "bientôt", None);
        let connection = connection_page(
            &Chrome {
                section: Some(VehicleTab::Connection),
                ..chrome
            },
            &ConnectionView {
                fleet: Party {
                    title: "Compte Fleet".to_owned(),
                    status: "connecté".to_owned(),
                    tone: "ok",
                    expires: "—".to_owned(),
                    scopes: "—".to_owned(),
                    last_refresh: "—".to_owned(),
                    last_error: None,
                    connected: false,
                },
                owner_status: "déconnecté".to_owned(),
                owner_tone: "neutral",
                owner_expires: "—".to_owned(),
                owner_scopes: "—".to_owned(),
                owner_last_refresh: "—".to_owned(),
                owner_last_error: None,
                owner_connected: false,
                fingerprint: "sha256:abcd".to_owned(),
                shared_names: Vec::new(),
                shared_ok: true,
                csrf: "csrf-token",
                vehicle_id: Some(4),
                error: None,
                notice: None,
                stream_status: "disabled",
                streaming_enabled: false,
                authorize_url: None,
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
                        position_mode: "hybrid".to_owned(),
                        last_poll: "—".to_owned(),
                        last_data: "—".to_owned(),
                        last_error: "—".to_owned(),
                        stream_lag: "—".to_owned(),
                        stream_queue: "0".to_owned(),
                        last_drive: "—".to_owned(),
                        last_charge: "—".to_owned(),
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
                        fleet: Party {
                            title: "Fleet".to_owned(),
                            status: "connecté".to_owned(),
                            tone: "ok",
                            expires: "—".to_owned(),
                            scopes: "—".to_owned(),
                            last_refresh: "—".to_owned(),
                            last_error: None,
                            connected: false,
                        },
                        owner_status: "déconnecté".to_owned(),
                        owner_tone: "neutral",
                        owner_expires: "—".to_owned(),
                        owner_scopes: "—".to_owned(),
                        owner_last_refresh: "—".to_owned(),
                        owner_last_error: None,
                        owner_connected: false,
                        fingerprint: "sha256:abcd".to_owned(),
                        shared_names: Vec::new(),
                        shared_ok: true,
                        csrf: "csrf-token",
                        vehicle_id: Some(4),
                        error: None,
                        notice: None,
                        stream_status: "disabled",
                        streaming_enabled: false,
                        authorize_url: None,
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
                system_page(&Chrome {
                    global: GlobalTab::System,
                    ..chrome()
                }),
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
