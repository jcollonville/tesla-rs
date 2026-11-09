// SPDX-License-Identifier: MIT OR Apache-2.0

use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use url::Url;
use zeroize::Zeroize;

use crate::pkce::Pkce;
use crate::secure::{constant_time_eq, ensure_secret_transport, random_token};
use crate::{AuthError, Region, Scope, TokenSet, TokenStore};

const DEFAULT_SKEW: Duration = Duration::from_secs(60);

/// Application credentials and the default region used for new grants.
#[derive(Clone)]
pub struct ClientConfig {
    /// Partner application client id from the Tesla developer portal.
    pub client_id: String,
    /// Partner application client secret.
    pub client_secret: SecretString,
    /// Registered redirect URI. May be empty when the client is only used for partner tokens.
    ///
    /// HTTPS is required, except for loopback `http://127.0.0.1`, `http://localhost`, and
    /// `http://[::1]`, which Tesla allows for native applications.
    pub redirect_uri: String,
    /// Region whose audience is embedded in newly issued tokens.
    pub region: Region,
    /// Scopes used by [`AuthClient::begin_login`] when the authorize options do not override them.
    pub scopes: Vec<Scope>,
}

impl std::fmt::Debug for ClientConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClientConfig")
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("redirect_uri", &self.redirect_uri)
            .field("region", &self.region)
            .field("scopes", &self.scopes)
            .finish()
    }
}

/// Authorization-server endpoints. Production values come from [`Region`].
#[derive(Clone, Debug)]
pub struct Endpoints {
    /// Browser `/authorize` URL.
    pub authorize: Url,
    /// Server-side `/token` URL.
    pub token: Url,
}

impl Endpoints {
    /// Endpoints documented for `region`.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::InvalidUrl`] if a built-in URL cannot be parsed.
    pub fn for_region(region: Region) -> Result<Self, AuthError> {
        Ok(Self {
            authorize: Url::parse(region.authorize_endpoint())?,
            token: Url::parse(region.token_endpoint())?,
        })
    }
}

/// Optional knobs for the authorization-code redirect.
#[derive(Clone, Debug, Default)]
pub struct AuthorizeOptions {
    /// Overrides [`ClientConfig::scopes`] when set.
    pub scopes: Option<Vec<Scope>>,
    /// Optional `locale` query parameter, for example `en-US`. Tesla's sample includes it.
    pub locale: Option<String>,
    /// Optional `prompt` query parameter. Tesla's sample uses `login`.
    pub prompt: Option<String>,
    /// When true, Tesla prompts only for scopes the user has not granted yet.
    pub prompt_missing_scopes: bool,
    /// When true, the user must grant every requested scope.
    pub require_requested_scopes: bool,
    /// When true, Tesla tells the user that virtual-key pairing is a second step.
    pub show_keypair_step: bool,
    /// When true, a random `nonce` is added for the caller to check inside `id_token` later.
    pub nonce: bool,
}

/// In-progress authorization-code login.
///
/// The PKCE verifier and nonce are kept here and are redacted in [`Debug`].
#[derive(Clone)]
pub struct PendingAuthorization {
    url: Url,
    state: String,
    pkce: Pkce,
    nonce: Option<SecretString>,
    scopes: Vec<Scope>,
}

impl PendingAuthorization {
    /// URL the user's browser should open.
    #[must_use]
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// CSRF `state` embedded in [`Self::url`].
    #[must_use]
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Nonce sent to Tesla, when [`AuthorizeOptions::nonce`] was set.
    #[must_use]
    pub fn nonce(&self) -> Option<&SecretString> {
        self.nonce.as_ref()
    }
}

impl std::fmt::Debug for PendingAuthorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingAuthorization")
            .field("url", &self.url)
            .field("state", &"<redacted>")
            .field("pkce", &self.pkce)
            .field("nonce", &self.nonce.as_ref().map(|_| "<redacted>"))
            .field("scopes", &self.scopes)
            .finish()
    }
}

/// Query parameters from the redirect URI.
#[derive(Clone, Debug, Default)]
pub struct CallbackParams {
    code: Option<SecretString>,
    /// `state` echoed by the authorization server.
    pub state: Option<String>,
    /// OAuth `error` code, when the user denied the request or Tesla aborted it.
    pub error: Option<String>,
    /// OAuth `error_description`.
    pub error_description: Option<String>,
}

impl CallbackParams {
    /// Parses a raw query string (`code=...&state=...`), with or without a leading `?`.
    #[must_use]
    pub fn from_query(query: &str) -> Self {
        let query = query.trim().trim_start_matches('?');
        let mut params = Self::default();
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "code" if !value.is_empty() => {
                    params.code = Some(SecretString::from(value.into_owned()));
                }
                "state" => params.state = Some(value.into_owned()),
                "error" => params.error = Some(value.into_owned()),
                "error_description" => params.error_description = Some(value.into_owned()),
                _ => {}
            }
        }
        params
    }

    /// Authorization code. Absent when the user denied the request.
    ///
    /// The value is a secret. [`Debug`] redacts it.
    #[must_use]
    pub fn code(&self) -> Option<&SecretString> {
        self.code.as_ref()
    }
}

/// Tesla OAuth client for third-party and partner tokens.
#[derive(Clone)]
pub struct AuthClient {
    config: ClientConfig,
    endpoints: Endpoints,
    http: reqwest::Client,
}

impl std::fmt::Debug for AuthClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthClient")
            .field("config", &self.config)
            .field("endpoints", &self.endpoints)
            .finish_non_exhaustive()
    }
}

impl AuthClient {
    /// Builds a client for the region's documented endpoints.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when the client id, secret, or redirect URI is unusable.
    pub fn new(config: ClientConfig) -> Result<Self, AuthError> {
        let endpoints = Endpoints::for_region(config.region)?;
        Self::builder(config, endpoints)?.build()
    }

    /// Starts a builder when the token URL must be overridden (tests, or a future host change).
    ///
    /// # Errors
    ///
    /// Returns a configuration error before any network call.
    pub fn builder(
        config: ClientConfig,
        endpoints: Endpoints,
    ) -> Result<AuthClientBuilder, AuthError> {
        validate_config(&config)?;
        ensure_secret_transport(&endpoints.authorize)?;
        ensure_secret_transport(&endpoints.token)?;
        Ok(AuthClientBuilder {
            config,
            endpoints,
            http: None,
        })
    }

    /// Region embedded in new token audiences.
    #[must_use]
    pub fn region(&self) -> Region {
        self.config.region
    }

    /// Builds the authorization redirect and a PKCE verifier.
    ///
    /// This does not contact Tesla. The URL includes `code_challenge` and `code_challenge_method=S256`.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when the redirect URI or scopes are missing, or when the
    /// system RNG fails.
    pub fn begin_login(
        &self,
        options: &AuthorizeOptions,
    ) -> Result<PendingAuthorization, AuthError> {
        if self.config.redirect_uri.is_empty() {
            return Err(AuthError::InvalidConfig(
                "redirect_uri is required for the authorization code flow",
            ));
        }
        let scopes = options
            .scopes
            .clone()
            .unwrap_or_else(|| self.config.scopes.clone());
        if scopes.is_empty() {
            return Err(AuthError::InvalidConfig(
                "at least one scope is required for the authorization code flow",
            ));
        }
        let state = random_token()?;
        let pkce = Pkce::generate()?;
        let nonce = if options.nonce {
            Some(SecretString::from(random_token()?))
        } else {
            None
        };
        let mut url = self.endpoints.authorize.clone();
        {
            let mut pairs = url.query_pairs_mut();
            pairs.append_pair("response_type", "code");
            pairs.append_pair("client_id", &self.config.client_id);
            pairs.append_pair("redirect_uri", &self.config.redirect_uri);
            pairs.append_pair("scope", &Scope::join(&scopes));
            pairs.append_pair("state", &state);
            pairs.append_pair("code_challenge", pkce.challenge());
            pairs.append_pair("code_challenge_method", "S256");
            if let Some(locale) = options.locale.as_deref() {
                pairs.append_pair("locale", locale);
            }
            if let Some(prompt) = options.prompt.as_deref() {
                pairs.append_pair("prompt", prompt);
            }
            if options.prompt_missing_scopes {
                pairs.append_pair("prompt_missing_scopes", "true");
            }
            if options.require_requested_scopes {
                pairs.append_pair("require_requested_scopes", "true");
            }
            if options.show_keypair_step {
                pairs.append_pair("show_keypair_step", "true");
            }
            if let Some(nonce) = nonce.as_ref() {
                pairs.append_pair("nonce", nonce.expose_secret());
            }
        }
        Ok(PendingAuthorization {
            url,
            state,
            pkce,
            nonce,
            scopes,
        })
    }

    /// Exchanges an authorization code for a token set.
    ///
    /// The request includes `code_verifier`. Tesla's HTML parameter table does not list it; PKCE
    /// requires it once a challenge was sent, and the discovery document supports `S256`.
    /// `client_secret` is sent in the form body and is not placed in the URL.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::StateMismatch`], [`AuthError::MissingAuthorizationCode`], or the OAuth
    /// error Tesla returned. A response of `invalid_auth_code` usually means the code expired.
    pub async fn exchange_code(
        &self,
        pending: &PendingAuthorization,
        callback: CallbackParams,
    ) -> Result<TokenSet, AuthError> {
        if let Some(error) = callback.error.as_deref() {
            return Err(oauth_callback_error(error, callback.error_description));
        }
        let state = callback.state.as_deref().unwrap_or("");
        if !constant_time_eq(&pending.state, state) {
            return Err(AuthError::StateMismatch);
        }
        let Some(code) = callback.code else {
            return Err(AuthError::MissingAuthorizationCode);
        };
        let mut form = vec![
            ("grant_type".to_owned(), "authorization_code".to_owned()),
            ("client_id".to_owned(), self.config.client_id.clone()),
            (
                "client_secret".to_owned(),
                self.config.client_secret.expose_secret().to_owned(),
            ),
            ("code".to_owned(), code.expose_secret().to_owned()),
            (
                "audience".to_owned(),
                self.config.region.audience().to_owned(),
            ),
            ("redirect_uri".to_owned(), self.config.redirect_uri.clone()),
            (
                "code_verifier".to_owned(),
                pending.pkce.expose_verifier().to_owned(),
            ),
        ];
        let tokens = self.token_request(&form, pending.scopes.clone()).await;
        zeroize_form(&mut form);
        tokens
    }

    /// Rotates a refresh token.
    ///
    /// Tesla's refresh request is documented as `grant_type`, `client_id`, and `refresh_token`.
    /// The client secret is not sent. The previous refresh token should be discarded in favor of
    /// the one in the returned [`TokenSet`].
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::LoginRequired`] on Tesla's `401 login_required` response.
    pub async fn refresh(&self, refresh_token: &SecretString) -> Result<TokenSet, AuthError> {
        let mut form = vec![
            ("grant_type".to_owned(), "refresh_token".to_owned()),
            ("client_id".to_owned(), self.config.client_id.clone()),
            (
                "refresh_token".to_owned(),
                refresh_token.expose_secret().to_owned(),
            ),
        ];
        let tokens = self.token_request(&form, self.config.scopes.clone()).await;
        zeroize_form(&mut form);
        tokens
    }

    /// Requests a partner token with the client-credentials grant.
    ///
    /// Partner tokens authorize partner endpoints such as `POST /api/1/partner_accounts`. They are
    /// not written to a [`TokenStore`]. Authentication calls are not billed.
    ///
    /// # Errors
    ///
    /// Returns a configuration error when `scopes` is empty, or the OAuth error Tesla returned.
    pub async fn partner_token(&self, scopes: &[Scope]) -> Result<TokenSet, AuthError> {
        if scopes.is_empty() {
            return Err(AuthError::InvalidConfig(
                "at least one scope is required for a partner token",
            ));
        }
        let mut form = vec![
            ("grant_type".to_owned(), "client_credentials".to_owned()),
            ("client_id".to_owned(), self.config.client_id.clone()),
            (
                "client_secret".to_owned(),
                self.config.client_secret.expose_secret().to_owned(),
            ),
            (
                "audience".to_owned(),
                self.config.region.audience().to_owned(),
            ),
            ("scope".to_owned(), Scope::join(scopes)),
        ];
        let tokens = self.token_request(&form, scopes.to_vec()).await;
        zeroize_form(&mut form);
        tokens
    }

    async fn token_request(
        &self,
        form: &[(String, String)],
        scopes: Vec<Scope>,
    ) -> Result<TokenSet, AuthError> {
        let response = self
            .http
            .post(self.endpoints.token.clone())
            .form(form)
            .send()
            .await
            .map_err(AuthError::http)?;
        let status = response.status();
        let bytes = response.bytes().await.map_err(AuthError::http)?;
        if !status.is_success() {
            return Err(parse_oauth_error(status.as_u16(), &bytes));
        }
        let parsed: crate::token::TokenResponse =
            serde_json::from_slice(&bytes).map_err(|_| AuthError::MalformedResponse)?;
        if parsed.access_token.is_empty() {
            return Err(AuthError::MalformedResponse);
        }
        TokenSet::from_response(parsed, scopes, std::time::SystemTime::now())
    }
}

/// Builder for [`AuthClient`] with an overridden HTTP client.
#[derive(Debug)]
pub struct AuthClientBuilder {
    config: ClientConfig,
    endpoints: Endpoints,
    http: Option<reqwest::Client>,
}

impl AuthClientBuilder {
    /// Replaces the default `reqwest` client.
    #[must_use]
    pub fn http(mut self, http: reqwest::Client) -> Self {
        self.http = Some(http);
        self
    }

    /// Finishes the client.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::Http`] when the default HTTP client cannot be built.
    pub fn build(self) -> Result<AuthClient, AuthError> {
        let http = match self.http {
            Some(http) => http,
            None => default_http()?,
        };
        Ok(AuthClient {
            config: self.config,
            endpoints: self.endpoints,
            http,
        })
    }
}

fn default_http() -> Result<reqwest::Client, AuthError> {
    reqwest::Client::builder()
        .user_agent(concat!(
            "tesla-auth/",
            env!("CARGO_PKG_VERSION"),
            " (unofficial)"
        ))
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(AuthError::http)
}

fn validate_config(config: &ClientConfig) -> Result<(), AuthError> {
    if config.client_id.trim().is_empty() {
        return Err(AuthError::InvalidConfig("client_id is required"));
    }
    if config.client_secret.expose_secret().is_empty() {
        return Err(AuthError::InvalidConfig("client_secret is required"));
    }
    if !config.redirect_uri.is_empty() {
        validate_redirect(&config.redirect_uri)?;
    }
    Ok(())
}

fn validate_redirect(value: &str) -> Result<(), AuthError> {
    let url = Url::parse(value)?;
    if !url.username().is_empty() || url.password().is_some() {
        return Err(AuthError::InsecureRedirectUri);
    }
    match url.scheme() {
        "https" => Ok(()),
        "http" if crate::secure::host_is_loopback(&url) => Ok(()),
        _ => Err(AuthError::InsecureRedirectUri),
    }
}

fn oauth_callback_error(error: &str, description: Option<String>) -> AuthError {
    if error == "login_required" {
        AuthError::LoginRequired { description }
    } else {
        AuthError::Oauth {
            status: 0,
            error: error.to_owned(),
            description,
        }
    }
}

fn parse_oauth_error(status: u16, bytes: &[u8]) -> AuthError {
    #[derive(Deserialize)]
    struct Body {
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        error_description: Option<String>,
    }
    let body: Body = serde_json::from_slice(bytes).unwrap_or(Body {
        error: None,
        error_description: None,
    });
    let error = body.error.unwrap_or_default();
    if status == 401 && (error == "login_required" || error.is_empty()) {
        return AuthError::LoginRequired {
            description: body.error_description,
        };
    }
    AuthError::Oauth {
        status,
        error: if error.is_empty() {
            "request_failed".to_owned()
        } else {
            error
        },
        description: body.error_description,
    }
}

fn zeroize_form(form: &mut [(String, String)]) {
    for (key, value) in form {
        let _ = key;
        value.zeroize();
    }
}

/// User-token session that refreshes and stores tokens.
///
/// Refresh is serialized with an in-process mutex. Refresh tokens are single-use; do not share one
/// store across processes without an external lock.
pub struct Session<S> {
    client: AuthClient,
    store: S,
    skew: Duration,
    refresh_lock: Mutex<()>,
}

impl<S> std::fmt::Debug for Session<S> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("client", &self.client)
            .field("skew", &self.skew)
            .finish_non_exhaustive()
    }
}

impl<S: TokenStore> Session<S> {
    /// Session that refreshes an access token `skew` before it expires.
    ///
    /// `skew` defaults to 60 seconds when constructed with [`Self::new`].
    #[must_use]
    pub fn with_skew(client: AuthClient, store: S, skew: Duration) -> Self {
        Self {
            client,
            store,
            skew,
            refresh_lock: Mutex::new(()),
        }
    }

    /// Session with a 60 second expiry skew.
    #[must_use]
    pub fn new(client: AuthClient, store: S) -> Self {
        Self::with_skew(client, store, DEFAULT_SKEW)
    }

    /// Exchanges the code and stores the resulting token set.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`AuthClient::exchange_code`], plus store failures.
    pub async fn complete_login(
        &self,
        pending: &PendingAuthorization,
        callback: CallbackParams,
    ) -> Result<TokenSet, AuthError> {
        let tokens = self.client.exchange_code(pending, callback).await?;
        self.store.save(&tokens).await?;
        Ok(tokens)
    }

    /// Returns a usable access token, refreshing it when it is close to expiry.
    ///
    /// # Errors
    ///
    /// Returns [`AuthError::NotAuthenticated`] when the store is empty,
    /// [`AuthError::LoginRequired`] when a refresh is required and no refresh token is stored,
    /// or the token-endpoint error.
    pub async fn access_token(&self) -> Result<SecretString, AuthError> {
        let _guard = self.refresh_lock.lock().await;
        let Some(tokens) = self.store.load().await? else {
            return Err(AuthError::NotAuthenticated);
        };
        if !tokens.needs_refresh(self.skew, std::time::SystemTime::now()) {
            return Ok(tokens.access_token().clone());
        }
        let Some(refresh) = tokens.refresh_token() else {
            return Err(AuthError::LoginRequired { description: None });
        };
        let refreshed = self.client.refresh(refresh).await?;
        let access = refreshed.access_token().clone();
        self.store.save(&refreshed).await?;
        Ok(access)
    }

    /// Borrows the underlying client, for partner-token calls that must not overwrite user tokens.
    #[must_use]
    pub fn client(&self) -> &AuthClient {
        &self.client
    }
}

/// Binds `127.0.0.1` and `port` for a one-shot login callback. Port `0` asks the OS for a port.
///
/// # Errors
///
/// Returns [`AuthError::Callback`] when the socket cannot be bound.
pub async fn bind_callback_listener(port: u16) -> Result<TcpListener, AuthError> {
    TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(AuthError::callback)
}

/// `http://127.0.0.1:{port}{path}` for a loopback redirect URI.
///
/// # Errors
///
/// Returns [`AuthError::InvalidConfig`] when `path` does not start with `/`.
pub fn loopback_redirect_uri(port: u16, path: &str) -> Result<String, AuthError> {
    if !path.starts_with('/') {
        return Err(AuthError::InvalidConfig("callback path must start with /"));
    }
    Ok(format!("http://127.0.0.1:{port}{path}"))
}

/// Waits for one `GET` whose path equals `expected_path`, ignoring other paths such as `/favicon.ico`.
///
/// The HTML response does not echo the authorization code. Only loopback listeners should be passed
/// in; this function does not re-bind the socket.
///
/// # Errors
///
/// Returns [`AuthError::Timeout`] when no matching request arrives, or [`AuthError::Callback`]
/// when the socket fails.
pub async fn listen_for_callback(
    listener: TcpListener,
    expected_path: &str,
    timeout: Duration,
) -> Result<CallbackParams, AuthError> {
    use tokio::io::AsyncReadExt;

    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err(AuthError::Timeout);
        }
        let (mut socket, _) = tokio::time::timeout(remaining, listener.accept())
            .await
            .map_err(|_| AuthError::Timeout)?
            .map_err(AuthError::callback)?;
        let mut buf = vec![0_u8; 8192];
        let read = tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buf))
            .await
            .map_err(|_| AuthError::Timeout)?
            .map_err(AuthError::callback)?;
        buf.truncate(read);
        let text = String::from_utf8_lossy(&buf);
        let Some(line) = text.lines().next() else {
            write_html(&mut socket, 400, "Bad Request", FAILURE_HTML).await?;
            continue;
        };
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("");
        let target = parts.next().unwrap_or("");
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        if method != "GET" || path != expected_path {
            write_html(&mut socket, 404, "Not Found", FAILURE_HTML).await?;
            continue;
        }
        let params = CallbackParams::from_query(query);
        let page = if params.error.is_some() {
            FAILURE_HTML
        } else {
            SUCCESS_HTML
        };
        write_html(&mut socket, 200, "OK", page).await?;
        return Ok(params);
    }
}

const SUCCESS_HTML: &str = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Tesla login</title></head><body><p>Login complete. You can close this window.</p></body></html>";
const FAILURE_HTML: &str = "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>Tesla login</title></head><body><p>Authorization failed. Return to the application.</p></body></html>";

async fn write_html(
    socket: &mut tokio::net::TcpStream,
    status: u16,
    reason: &str,
    body: &str,
) -> Result<(), AuthError> {
    use tokio::io::AsyncWriteExt;
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n",
        body.len()
    );
    socket
        .write_all(header.as_bytes())
        .await
        .map_err(AuthError::callback)?;
    socket
        .write_all(body.as_bytes())
        .await
        .map_err(AuthError::callback)?;
    socket.shutdown().await.map_err(AuthError::callback)?;
    Ok(())
}
