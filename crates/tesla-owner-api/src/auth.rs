// SPDX-License-Identifier: MIT OR Apache-2.0

use std::time::{Duration, SystemTime};

use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use url::Url;
use zeroize::Zeroize;

use crate::error::OwnerError;
use crate::pkce::{self, Pkce};
use crate::vehicles::OwnerVehicle;

const CLIENT_ID: &str = "ownerapi";
const SCOPES: &str = "openid email offline_access";
const DEFAULT_EXPIRES_SECS: u64 = 8 * 60 * 60;
/// Registered redirect for the public `ownerapi` client, on both auth hosts.
///
/// Tesla no longer accepts `https://auth.tesla.com/void/callback`. A browser cannot
/// navigate to this scheme; the operator pastes the `tesla://auth/callback?...` URL.
const REDIRECT_URI: &str = "tesla://auth/callback";

/// Which Tesla account host to use.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerRegion {
    /// North America, Europe, and Asia-Pacific, excluding China.
    Global,
    /// A Chinese Tesla account.
    China,
}

#[derive(Clone)]
struct Endpoints {
    authorize: Url,
    token: Url,
    api_base: Url,
    stream: String,
    redirect_uri: String,
}

impl OwnerRegion {
    fn endpoints(self) -> Result<Endpoints, OwnerError> {
        let (authorize, token, api, stream) = match self {
            Self::Global => (
                "https://auth.tesla.com/oauth2/v3/authorize",
                "https://auth.tesla.com/oauth2/v3/token",
                "https://owner-api.teslamotors.com",
                "wss://streaming.vn.teslamotors.com/streaming/",
            ),
            Self::China => (
                "https://auth.tesla.cn/oauth2/v3/authorize",
                "https://auth.tesla.cn/oauth2/v3/token",
                "https://owner-api.vn.cloud.tesla.cn",
                "wss://streaming.vn.cloud.tesla.cn/streaming/",
            ),
        };
        Ok(Endpoints {
            authorize: Url::parse(authorize)?,
            token: Url::parse(token)?,
            api_base: Url::parse(api)?,
            stream: stream.to_owned(),
            redirect_uri: REDIRECT_URI.to_owned(),
        })
    }
}

impl From<url::ParseError> for OwnerError {
    fn from(_: url::ParseError) -> Self {
        Self::InsecureUrl
    }
}

/// Owner API access token and rotating refresh token.
///
/// `Debug` redacts both tokens.
#[derive(Clone)]
pub struct OwnerTokens {
    access_token: SecretString,
    refresh_token: Option<SecretString>,
    expires_at: SystemTime,
}

impl std::fmt::Debug for OwnerTokens {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnerTokens")
            .field("access_token", &"<redacted>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<redacted>"),
            )
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl OwnerTokens {
    /// Builds a token set the caller already holds.
    #[must_use]
    pub fn from_parts(
        access_token: SecretString,
        refresh_token: Option<SecretString>,
        expires_at: SystemTime,
    ) -> Self {
        Self {
            access_token,
            refresh_token,
            expires_at,
        }
    }

    /// Bearer access token.
    #[must_use]
    pub fn access_token(&self) -> &SecretString {
        &self.access_token
    }

    /// Refresh token, when one was issued or preserved.
    #[must_use]
    pub fn refresh_token(&self) -> Option<&SecretString> {
        self.refresh_token.as_ref()
    }

    /// Absolute expiry of the access token.
    #[must_use]
    pub fn expires_at(&self) -> SystemTime {
        self.expires_at
    }

    /// Whether the access token should be refreshed before use.
    #[must_use]
    pub fn needs_refresh(&self, skew: Duration, now: SystemTime) -> bool {
        match self.expires_at.checked_sub(skew) {
            Some(deadline) => now >= deadline,
            None => true,
        }
    }

    /// Keeps a previous refresh token when Tesla omits the rotated value.
    pub fn keep_refresh_token(&mut self, refresh_token: SecretString) {
        if self.refresh_token.is_none() {
            self.refresh_token = Some(refresh_token);
        }
    }
}

/// In-progress Owner API sign-in.
///
/// The authorize URL is safe to show to the signed-in operator. The PKCE verifier is redacted.
#[derive(Clone)]
pub struct OwnerLogin {
    url: Url,
    state: String,
    pkce: Pkce,
}

impl std::fmt::Debug for OwnerLogin {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnerLogin")
            .field("url", &self.url)
            .field("state", &"<redacted>")
            .field("pkce", &self.pkce)
            .finish()
    }
}

impl OwnerLogin {
    /// URL the operator opens in a browser.
    #[must_use]
    pub fn authorize_url(&self) -> &Url {
        &self.url
    }
}

/// HTTP client for the public `ownerapi` OAuth client and the vehicle list.
#[derive(Clone)]
pub struct OwnerClient {
    http: reqwest::Client,
    endpoints: Endpoints,
}

impl std::fmt::Debug for OwnerClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("OwnerClient")
            .field("token", &self.endpoints.token)
            .field("api_base", &self.endpoints.api_base)
            .field("stream", &self.endpoints.stream)
            .finish_non_exhaustive()
    }
}

impl OwnerClient {
    /// Client for the region's Owner API and streaming hosts.
    ///
    /// Token calls use the workspace `reqwest` client (rustls, HTTP/2). TLS 1.3 is
    /// negotiated when the auth host offers it. Refreshing the Owner API token over
    /// HTTP/1.1 can return an access token that `owner-api` later rejects.
    ///
    /// # Errors
    ///
    /// Returns [`OwnerError::Transport`] when the HTTP client cannot be built.
    pub fn new(region: OwnerRegion) -> Result<Self, OwnerError> {
        Self::build(region.endpoints()?, None)
    }

    /// Streaming websocket URL for this client.
    #[must_use]
    pub fn stream_url(&self) -> &str {
        &self.endpoints.stream
    }

    /// Points token and vehicle calls at one origin. Used by tests.
    #[cfg(test)]
    pub(crate) fn for_test_origin(origin: &str) -> Result<Self, OwnerError> {
        let origin = origin.trim_end_matches('/');
        let endpoints = Endpoints {
            authorize: Url::parse(&format!("{origin}/oauth2/v3/authorize"))?,
            token: Url::parse(&format!("{origin}/oauth2/v3/token"))?,
            api_base: Url::parse(&format!("{origin}/"))?,
            stream: "wss://streaming.vn.teslamotors.com/streaming/".to_owned(),
            redirect_uri: REDIRECT_URI.to_owned(),
        };
        Self::build(endpoints, None)
    }

    /// Starts a PKCE login. The operator pastes the final redirect URL back.
    ///
    /// # Errors
    ///
    /// Returns [`OwnerError::Entropy`] when the system RNG fails.
    pub fn begin_login(&self) -> Result<OwnerLogin, OwnerError> {
        let pkce = Pkce::generate()?;
        let state = pkce::random_state()?;
        let mut url = self.endpoints.authorize.clone();
        url.query_pairs_mut()
            .append_pair("client_id", CLIENT_ID)
            .append_pair("redirect_uri", &self.endpoints.redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", SCOPES)
            .append_pair("code_challenge", pkce.challenge())
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        Ok(OwnerLogin { url, state, pkce })
    }

    /// Exchanges a pasted `tesla://auth/callback?...` URL, or a bare query string, for tokens.
    ///
    /// # Errors
    ///
    /// Returns [`OwnerError::StateMismatch`] or [`OwnerError::MissingCode`] when the paste is not
    /// the redirect from [`Self::begin_login`]. Tesla errors become [`OwnerError::Rejected`] or
    /// [`OwnerError::LoginRequired`].
    pub async fn exchange(
        &self,
        login: &OwnerLogin,
        pasted: &str,
    ) -> Result<OwnerTokens, OwnerError> {
        let callback = parse_callback(pasted)?;
        if !pkce::constant_time_eq(&callback.state, &login.state) {
            return Err(OwnerError::StateMismatch);
        }
        let mut form = vec![
            ("grant_type".to_owned(), "authorization_code".to_owned()),
            ("client_id".to_owned(), CLIENT_ID.to_owned()),
            ("code".to_owned(), callback.code.expose_secret().to_owned()),
            ("code_verifier".to_owned(), login.pkce.verifier().to_owned()),
            (
                "redirect_uri".to_owned(),
                self.endpoints.redirect_uri.clone(),
            ),
        ];
        let result = self.token_request(&form).await;
        zeroize_form(&mut form);
        result
    }

    /// Exchanges a refresh token. When Tesla omits a new refresh token, the previous one is kept.
    ///
    /// # Errors
    ///
    /// Returns [`OwnerError::LoginRequired`] when Tesla rejects the refresh token.
    pub async fn refresh(&self, refresh_token: &SecretString) -> Result<OwnerTokens, OwnerError> {
        let mut form = vec![
            ("grant_type".to_owned(), "refresh_token".to_owned()),
            ("client_id".to_owned(), CLIENT_ID.to_owned()),
            (
                "refresh_token".to_owned(),
                refresh_token.expose_secret().to_owned(),
            ),
            ("scope".to_owned(), SCOPES.to_owned()),
        ];
        let mut tokens = self.token_request(&form).await;
        if let Ok(tokens) = tokens.as_mut()
            && let Some((_, refresh)) = form.iter().find(|(key, _)| key == "refresh_token")
        {
            tokens.keep_refresh_token(SecretString::from(refresh.clone()));
        }
        zeroize_form(&mut form);
        tokens
    }

    /// Lists vehicles visible to the Owner API token.
    ///
    /// The response can include streaming tokens. Those fields are ignored and are not logged.
    ///
    /// # Errors
    ///
    /// Returns [`OwnerError::LoginRequired`] on HTTP 401 and [`OwnerError::Malformed`] when the
    /// body is not a vehicle list.
    pub async fn list_vehicles(
        &self,
        access_token: &SecretString,
    ) -> Result<Vec<OwnerVehicle>, OwnerError> {
        let url = self
            .endpoints
            .api_base
            .join("/api/1/vehicles")
            .map_err(|_| OwnerError::InsecureUrl)?;
        ensure_http(&url)?;
        let response = self
            .http
            .get(url)
            .bearer_auth(access_token.expose_secret())
            .send()
            .await
            .map_err(OwnerError::Transport)?;
        let status = response.status().as_u16();
        let bytes = response.bytes().await.map_err(OwnerError::Transport)?;
        if status == 401 {
            return Err(OwnerError::LoginRequired);
        }
        if !(200..300).contains(&status) {
            return Err(OwnerError::Rejected { status });
        }
        parse_vehicle_list(&bytes)
    }

    fn build(endpoints: Endpoints, http: Option<reqwest::Client>) -> Result<Self, OwnerError> {
        ensure_http(&endpoints.token)?;
        ensure_http(&endpoints.api_base)?;
        let http = match http {
            Some(http) => http,
            None => reqwest::Client::builder()
                .user_agent(concat!(
                    "tesla-owner-api/",
                    env!("CARGO_PKG_VERSION"),
                    " (unofficial)"
                ))
                // ALPN offers h2. Do not disable the workspace `http2` feature.
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(30))
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(OwnerError::Transport)?,
        };
        Ok(Self { http, endpoints })
    }

    async fn token_request(&self, form: &[(String, String)]) -> Result<OwnerTokens, OwnerError> {
        ensure_http(&self.endpoints.token)?;
        let response = self
            .http
            .post(self.endpoints.token.clone())
            .form(form)
            .send()
            .await
            .map_err(OwnerError::Transport)?;
        let status = response.status().as_u16();
        let bytes = response.bytes().await.map_err(OwnerError::Transport)?;
        if !(200..300).contains(&status) {
            return Err(token_error(status, &bytes));
        }
        parse_token_body(&bytes)
    }
}

struct CallbackParts {
    code: SecretString,
    state: String,
}

fn zeroize_form(form: &mut [(String, String)]) {
    for (key, value) in form {
        key.zeroize();
        value.zeroize();
    }
}

/// Accepts `tesla://auth/callback?code=…&state=…` and a bare `code=…&state=…` query.
///
/// The `url` crate parses the custom scheme. An https host is not required.
fn parse_callback(pasted: &str) -> Result<CallbackParts, OwnerError> {
    let pasted = pasted.trim();
    if pasted.is_empty() || pasted.len() > 8_192 {
        return Err(OwnerError::MissingCode);
    }
    let query = if let Ok(url) = Url::parse(pasted) {
        if url.query().is_none() {
            return Err(OwnerError::MissingCode);
        }
        url.query().unwrap_or("").to_owned()
    } else if pasted.contains('=') {
        pasted.trim_start_matches('?').to_owned()
    } else {
        return Err(OwnerError::MissingCode);
    };
    let mut code = None;
    let mut state = None;
    let mut error = None;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        match key.as_ref() {
            "code" if !value.is_empty() => code = Some(value.into_owned()),
            "state" if !value.is_empty() => state = Some(value.into_owned()),
            "error" if !value.is_empty() => error = Some(value.into_owned()),
            _ => {}
        }
    }
    if error.is_some() {
        return Err(OwnerError::Rejected { status: 0 });
    }
    let Some(mut code) = code else {
        return Err(OwnerError::MissingCode);
    };
    let Some(state) = state else {
        code.zeroize();
        return Err(OwnerError::StateMismatch);
    };
    let secret = SecretString::from(code.clone());
    code.zeroize();
    Ok(CallbackParts {
        code: secret,
        state,
    })
}

fn parse_token_body(bytes: &[u8]) -> Result<OwnerTokens, OwnerError> {
    let mut value: Value = serde_json::from_slice(bytes).map_err(|_| OwnerError::Malformed)?;
    let mut access = match value.get_mut("access_token") {
        Some(Value::String(token)) if !token.is_empty() => std::mem::take(token),
        _ => return Err(OwnerError::Malformed),
    };
    let refresh = match value.get_mut("refresh_token") {
        Some(Value::String(token)) if !token.is_empty() => {
            let mut token = std::mem::take(token);
            let secret = SecretString::from(token.clone());
            token.zeroize();
            Some(secret)
        }
        _ => None,
    };
    let expires_in = value
        .get("expires_in")
        .and_then(json_u64)
        .unwrap_or(DEFAULT_EXPIRES_SECS);
    let secret = SecretString::from(access.clone());
    access.zeroize();
    Ok(OwnerTokens {
        access_token: secret,
        refresh_token: refresh,
        expires_at: SystemTime::now() + Duration::from_secs(expires_in),
    })
}

fn parse_vehicle_list(bytes: &[u8]) -> Result<Vec<OwnerVehicle>, OwnerError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| OwnerError::Malformed)?;
    let Some(rows) = value.get("response").and_then(Value::as_array) else {
        return Err(OwnerError::Malformed);
    };
    let mut vehicles = Vec::new();
    for row in rows {
        let Some(vin) = row.get("vin").and_then(Value::as_str) else {
            continue;
        };
        let Some(id) = row.get("id").and_then(json_i64) else {
            continue;
        };
        let Some(vehicle_id) = row.get("vehicle_id").and_then(json_i64) else {
            continue;
        };
        if vin.is_empty() || vehicle_id == 0 {
            continue;
        }
        let state = row.get("state").and_then(Value::as_str).map(str::to_owned);
        vehicles.push(OwnerVehicle {
            vin: vin.to_owned(),
            id,
            vehicle_id,
            state,
        });
    }
    Ok(vehicles)
}

fn token_error(status: u16, bytes: &[u8]) -> OwnerError {
    let code = serde_json::from_slice::<Value>(bytes)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_default();
    if status == 401 || code == "invalid_grant" || code == "login_required" {
        OwnerError::LoginRequired
    } else {
        OwnerError::Rejected { status }
    }
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|number| u64::try_from(number).ok()))
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn ensure_http(url: &Url) -> Result<(), OwnerError> {
    match url.scheme() {
        "https" => Ok(()),
        "http" if host_is_loopback(url) => Ok(()),
        _ => Err(OwnerError::InsecureUrl),
    }
}

pub(crate) fn host_is_loopback(url: &Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"))
}

#[cfg(test)]
mod tests {
    use super::{OwnerClient, OwnerError, OwnerRegion, OwnerTokens, parse_callback};
    use secrecy::SecretString;
    use std::time::{Duration, SystemTime};

    #[test]
    fn debug_redacts_tokens_and_login_starts_ownerapi() {
        let client = OwnerClient::new(OwnerRegion::Global).unwrap();
        let login = client.begin_login().unwrap();
        let url = login.authorize_url().to_string();
        assert!(url.starts_with("https://auth.tesla.com/oauth2/v3/authorize?"));
        assert!(url.contains("client_id=ownerapi"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("redirect_uri=tesla%3A%2F%2Fauth%2Fcallback"));
        assert!(!url.contains("void/callback"));
        assert!(!url.contains("client_secret"));
        let rendered = format!("{login:?}");
        assert!(!rendered.contains("code_verifier"));
        let tokens = OwnerTokens::from_parts(
            SecretString::from("access-secret".to_owned()),
            Some(SecretString::from("refresh-secret".to_owned())),
            SystemTime::now() + Duration::from_secs(60),
        );
        let rendered = format!("{tokens:?}");
        assert!(!rendered.contains("access-secret"));
        assert!(!rendered.contains("refresh-secret"));
        assert!(tokens.needs_refresh(Duration::from_secs(120), SystemTime::now()));
        assert_eq!(
            client.stream_url(),
            "wss://streaming.vn.teslamotors.com/streaming/"
        );
    }

    #[tokio::test]
    async fn refresh_sends_ownerapi_and_hides_the_token_from_errors() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth2/v3/token"))
            .respond_with(wiremock::ResponseTemplate::new(401).set_body_string(
                r#"{"error":"invalid_grant","error_description":"refresh-secret should not echo"}"#,
            ))
            .mount(&server)
            .await;
        let client = OwnerClient::for_test_origin(&server.uri()).unwrap();
        let err = client
            .refresh(&SecretString::from("refresh-secret".to_owned()))
            .await
            .expect_err("rejected");
        assert!(err.is_login_required());
        let rendered = err.to_string();
        assert!(!rendered.contains("refresh-secret"));
        assert!(!rendered.contains("should not echo"));
        let requests = server.received_requests().await.unwrap();
        let content_type = requests[0]
            .headers
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(content_type.starts_with("application/x-www-form-urlencoded"));
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(body.contains("grant_type=refresh_token"));
        assert!(body.contains("client_id=ownerapi"));
        assert!(body.contains("refresh_token=refresh-secret"));
        assert!(body.contains("scope=openid+email+offline_access"));
        assert!(!body.contains("client_secret"));
        assert!(!body.starts_with('{'));
    }

    #[tokio::test]
    async fn exchange_posts_the_code_and_verifier() {
        use secrecy::ExposeSecret;

        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/oauth2/v3/token"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"access_token":"access-1","refresh_token":"refresh-1","expires_in":120,"token_type":"Bearer"}"#,
            ))
            .mount(&server)
            .await;
        let client = OwnerClient::for_test_origin(&server.uri()).unwrap();
        let login = client.begin_login().unwrap();
        let state = login
            .authorize_url()
            .query_pairs()
            .find(|(key, _)| key == "state")
            .unwrap()
            .1;
        let pasted = format!(
            "tesla://auth/callback?code=auth-code&state={state}&issuer=https://auth.tesla.com/oauth2/v3"
        );
        let tokens = client.exchange(&login, &pasted).await.unwrap();
        assert_eq!(tokens.access_token().expose_secret(), "access-1");
        let bare = client
            .exchange(&login, &format!("code=auth-code&state={state}"))
            .await
            .unwrap();
        assert_eq!(bare.access_token().expose_secret(), "access-1");
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        for request in &requests {
            let content_type = request
                .headers
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap();
            assert!(content_type.starts_with("application/x-www-form-urlencoded"));
            let body = String::from_utf8_lossy(&request.body);
            assert!(body.contains("grant_type=authorization_code"));
            assert!(body.contains("client_id=ownerapi"));
            assert!(body.contains("code=auth-code"));
            assert!(body.contains("code_verifier="));
            assert!(body.contains("redirect_uri=tesla%3A%2F%2Fauth%2Fcallback"));
            assert!(!body.contains("client_secret"));
            assert!(!body.contains("void/callback"));
            assert!(!body.starts_with('{'));
        }
        let mismatch = client
            .exchange(&login, "tesla://auth/callback?code=x&state=other")
            .await
            .unwrap_err();
        assert!(matches!(mismatch, OwnerError::StateMismatch));
    }

    #[tokio::test]
    async fn vehicle_list_uses_vehicle_id_not_the_rest_id() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/api/1/vehicles"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(
                r#"{"response":[{"id":1,"vehicle_id":99,"vin":"5YJ3E1EA7KF000001","state":"online","tokens":["do-not-keep"]}]}"#,
            ))
            .mount(&server)
            .await;
        let client = OwnerClient::for_test_origin(&server.uri()).unwrap();
        let vehicles = client
            .list_vehicles(&SecretString::from("access-token".to_owned()))
            .await
            .unwrap();
        assert_eq!(vehicles.len(), 1);
        assert_eq!(vehicles[0].id, 1);
        assert_eq!(vehicles[0].vehicle_id, 99);
        assert_eq!(vehicles[0].vin, "5YJ3E1EA7KF000001");
        let header = server.received_requests().await.unwrap()[0]
            .headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .to_owned();
        assert_eq!(header, "Bearer access-token");
    }

    #[test]
    fn parse_callback_accepts_the_custom_scheme_and_a_bare_query() {
        use secrecy::ExposeSecret;

        let custom = parse_callback(
            "  tesla://auth/callback?code=auth-code&state=state-1&issuer=https://auth.tesla.com/oauth2/v3  ",
        )
        .unwrap();
        assert_eq!(custom.code.expose_secret(), "auth-code");
        assert_eq!(custom.state, "state-1");
        let bare = parse_callback("?code=auth-code&state=state-1").unwrap();
        assert_eq!(bare.code.expose_secret(), "auth-code");
        assert_eq!(bare.state, "state-1");
        assert!(matches!(
            parse_callback("tesla://auth/callback"),
            Err(OwnerError::MissingCode)
        ));
        assert!(matches!(
            parse_callback("code=only"),
            Err(OwnerError::StateMismatch)
        ));
    }

    #[test]
    fn china_uses_the_china_hosts() {
        let client = OwnerClient::new(OwnerRegion::China).unwrap();
        let login = client.begin_login().unwrap();
        let url = login.authorize_url().as_str();
        assert!(url.starts_with("https://auth.tesla.cn/oauth2/v3/authorize?"));
        assert!(url.contains("redirect_uri=tesla%3A%2F%2Fauth%2Fcallback"));
        assert!(!url.contains("void/callback"));
        assert_eq!(
            client.stream_url(),
            "wss://streaming.vn.cloud.tesla.cn/streaming/"
        );
    }
}
