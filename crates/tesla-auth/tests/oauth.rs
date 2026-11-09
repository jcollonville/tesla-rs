//! OAuth client tests against a local mock.

use std::time::{Duration, SystemTime};

use secrecy::{ExposeSecret, SecretString};
use tesla_auth::{
    AuthClient, AuthorizeOptions, CallbackParams, ClientConfig, Endpoints, FileTokenStore,
    MemoryTokenStore, Region, Scope, Session, TokenSet, TokenStore,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(redirect: &str) -> ClientConfig {
    ClientConfig {
        client_id: "client-id".to_owned(),
        client_secret: SecretString::from("super-secret".to_owned()),
        redirect_uri: redirect.to_owned(),
        region: Region::NorthAmerica,
        scopes: Scope::third_party_vehicle(),
    }
}

fn client(server: &MockServer) -> AuthClient {
    let mut endpoints = Endpoints::for_region(Region::NorthAmerica).unwrap();
    endpoints.token = format!("{}/oauth2/v3/token", server.uri()).parse().unwrap();
    AuthClient::builder(config("http://127.0.0.1:8734/callback"), endpoints)
        .unwrap()
        .build()
        .unwrap()
}

fn token_json(access: &str, refresh: Option<&str>, expires_in: u64) -> serde_json::Value {
    let mut value = serde_json::json!({
        "access_token": access,
        "token_type": "Bearer",
        "expires_in": expires_in,
        "scope": "openid offline_access vehicle_device_data"
    });
    if let Some(refresh) = refresh {
        value["refresh_token"] = serde_json::json!(refresh);
    }
    value
}

#[tokio::test]
async fn authorization_code_flow_sends_pkce_and_hides_secrets() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/v3/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_json(
            "access-secret",
            Some("refresh-secret"),
            3600,
        )))
        .mount(&server)
        .await;
    let client = client(&server);
    let pending = client.begin_login(&AuthorizeOptions::default()).unwrap();
    let url = pending.url().as_str();
    assert!(url.starts_with("https://auth.tesla.com/oauth2/v3/authorize?"));
    assert!(url.contains("code_challenge_method=S256"));
    assert!(!url.contains("audience"));
    assert!(!url.contains("super-secret"));
    assert!(!format!("{pending:?}").contains("super-secret"));
    assert!(!format!("{client:?}").contains("super-secret"));

    let callback = CallbackParams::from_query(&format!("code=auth-code&state={}", pending.state()));
    let tokens = client.exchange_code(&pending, callback).await.unwrap();
    assert_eq!(tokens.access_token().expose_secret(), "access-secret");
    assert!(!format!("{tokens:?}").contains("access-secret"));
    assert!(!format!("{tokens:?}").contains("refresh-secret"));

    let requests = server.received_requests().await.unwrap();
    let body = String::from_utf8(requests[0].body.clone()).unwrap();
    assert!(body.contains("grant_type=authorization_code"));
    assert!(body.contains("client_secret=super-secret"));
    assert!(body.contains("code=auth-code"));
    assert!(body.contains("code_verifier="));
    assert!(body.contains("audience=https%3A%2F%2Ffleet-api.prd.na.vn.cloud.tesla.com"));
    assert!(!body.contains("client_secret=super-secret&client_secret"));
}

#[tokio::test]
async fn state_mismatch_does_not_call_the_token_endpoint() {
    let server = MockServer::start().await;
    let client = client(&server);
    let pending = client.begin_login(&AuthorizeOptions::default()).unwrap();
    let error = client
        .exchange_code(
            &pending,
            CallbackParams::from_query("code=auth-code&state=nope"),
        )
        .await
        .unwrap_err();
    assert!(matches!(error, tesla_auth::AuthError::StateMismatch));
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn oauth_error_body_does_not_echo_unrelated_secrets() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/v3/token"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "error": "login_required",
            "error_description": "User must sign in",
            "client_secret": "super-secret",
            "refresh_token": "refresh-secret"
        })))
        .mount(&server)
        .await;
    let client = client(&server);
    let error = client
        .refresh(&SecretString::from("refresh-secret".to_owned()))
        .await
        .unwrap_err();
    let rendered = error.to_string();
    assert!(error.is_login_required());
    assert!(rendered.contains("login required"));
    assert!(!rendered.contains("super-secret"));
    assert!(!rendered.contains("refresh-secret"));
}

#[tokio::test]
async fn refresh_omits_client_secret_and_partner_token_uses_client_credentials() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/v3/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_json(
            "partner-access",
            None,
            600,
        )))
        .mount(&server)
        .await;
    let client = client(&server);
    let tokens = client
        .partner_token(&Scope::partner_account())
        .await
        .unwrap();
    assert!(tokens.refresh_token().is_none());
    let body =
        String::from_utf8(server.received_requests().await.unwrap()[0].body.clone()).unwrap();
    assert!(body.contains("grant_type=client_credentials"));
    assert!(body.contains("client_secret=super-secret"));
    assert!(body.contains("scope=openid"));
    assert!(!body.contains("refresh_token="));

    let store = MemoryTokenStore::new();
    store
        .save(&TokenSet::bearer(
            SecretString::from("old-access".to_owned()),
            Some(SecretString::from("old-refresh".to_owned())),
            SystemTime::UNIX_EPOCH,
            Scope::third_party_vehicle(),
        ))
        .await
        .unwrap();
    // Remount a distinct response by using the same mock; the next call still hits it.
    let session = Session::new(client, store);
    let access = session.access_token().await.unwrap();
    assert_eq!(access.expose_secret(), "partner-access");
    let requests = server.received_requests().await.unwrap();
    let refresh = String::from_utf8(requests[1].body.clone()).unwrap();
    assert!(refresh.contains("grant_type=refresh_token"));
    assert!(refresh.contains("refresh_token=old-refresh"));
    assert!(!refresh.contains("client_secret"));
}

#[tokio::test]
async fn session_refreshes_once_when_called_concurrently() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth2/v3/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_json(
            "new-access",
            Some("new-refresh"),
            3600,
        )))
        .mount(&server)
        .await;
    let client = client(&server);
    let store = MemoryTokenStore::new();
    store
        .save(&TokenSet::bearer(
            SecretString::from("old-access".to_owned()),
            Some(SecretString::from("old-refresh".to_owned())),
            SystemTime::UNIX_EPOCH,
            vec![Scope::OpenId],
        ))
        .await
        .unwrap();
    let session = Session::new(client, store);
    let (left, right) = tokio::join!(session.access_token(), session.access_token());
    assert_eq!(left.unwrap().expose_secret(), "new-access");
    assert_eq!(right.unwrap().expose_secret(), "new-access");
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn file_store_is_mode_0600_and_round_trips() {
    let dir = std::env::temp_dir().join(format!(
        "tesla-auth-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("tokens.json");
    let store = FileTokenStore::new(&path);
    let tokens = TokenSet::bearer(
        SecretString::from("access-secret".to_owned()),
        Some(SecretString::from("refresh-secret".to_owned())),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        vec![Scope::OfflineAccess],
    );
    store.save(&tokens).await.unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    let loaded = store.load().await.unwrap().unwrap();
    assert_eq!(loaded.access_token().expose_secret(), "access-secret");
    assert!(!format!("{loaded:?}").contains("access-secret"));
    store.clear().await.unwrap();
    assert!(store.load().await.unwrap().is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn rejects_public_http_redirect_and_unknown_region() {
    let mut bad = config("http://example.com/callback");
    assert!(AuthClient::new(bad.clone()).is_err());
    bad.redirect_uri = "https://example.com/callback".to_owned();
    assert!(AuthClient::new(bad).is_ok());
    assert!("us".parse::<Region>().is_err());
}
