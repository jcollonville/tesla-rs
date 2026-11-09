// SPDX-License-Identifier: MIT OR Apache-2.0

//! OAuth 2.0 for the Tesla Fleet API.
//!
//! This crate implements the third-party authorization-code flow with PKCE (`S256`), refresh-token
//! rotation, and partner tokens via the client-credentials grant. It is not affiliated with Tesla.
//!
//! Token material is held in [`secrecy::SecretString`]. [`Debug`] implementations redact access
//! tokens, refresh tokens, ID tokens, client secrets, authorization codes, and PKCE verifiers.
//! The HTTP client does not log headers or bodies. Redirects are disabled so a token response
//! cannot be replayed to another host.
//!
//! # Region and audience
//!
//! A token is minted for one Fleet API audience. Pick the user's region before
//! [`AuthClient::exchange_code`]. `GET /api/1/users/region` (in `tesla-fleet-api`) can confirm the
//! base URL, but an authorization code cannot be exchanged twice.
//!
//! # Example
//!
//! ```
//! use secrecy::SecretString;
//! use tesla_auth::{AuthClient, AuthorizeOptions, ClientConfig, Region, Scope};
//!
//! let client = AuthClient::new(ClientConfig {
//!     client_id: "client-id".to_owned(),
//!     client_secret: SecretString::from("client-secret".to_owned()),
//!     redirect_uri: "http://127.0.0.1:8734/callback".to_owned(),
//!     region: Region::NorthAmerica,
//!     scopes: Scope::third_party_vehicle(),
//! })
//! .expect("config");
//! let pending = client
//!     .begin_login(&AuthorizeOptions::default())
//!     .expect("login");
//! let url = pending.url().as_str();
//! assert!(url.starts_with("https://auth.tesla.com/oauth2/v3/authorize?"));
//! assert!(url.contains("code_challenge_method=S256"));
//! assert!(!url.contains("client-secret"));
//! ```

#![forbid(unsafe_code)]

mod client;
mod error;
mod pkce;
mod region;
mod scope;
mod secure;
mod store;
mod token;

pub use client::{
    AuthClient, AuthClientBuilder, AuthorizeOptions, CallbackParams, ClientConfig, Endpoints,
    PendingAuthorization, Session, bind_callback_listener, listen_for_callback,
    loopback_redirect_uri,
};
pub use error::AuthError;
pub use pkce::Pkce;
pub use region::Region;
pub use scope::Scope;
pub use secrecy::{ExposeSecret, SecretString};
pub use store::{FileTokenStore, MemoryTokenStore, TokenStore};
pub use token::TokenSet;
