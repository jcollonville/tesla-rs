// SPDX-License-Identifier: MIT OR Apache-2.0

//! Runs the Tesla third-party login on `127.0.0.1` and prints the account's vehicles.
//!
//! Required environment variables:
//!
//! * `TESLA_CLIENT_ID`
//! * `TESLA_CLIENT_SECRET`
//!
//! Optional:
//!
//! * `TESLA_REGION` — `na` (default), `eu`, or `cn`
//! * `TESLA_CALLBACK_PORT` — default `8734`
//! * `TESLA_REDIRECT_URI` — must match the application registration. The default is
//!   `http://127.0.0.1:{port}/callback`.
//! * `TESLA_SCOPES` — space-delimited override
//! * `TESLA_TOKEN_FILE` — when set, the token set is written there with mode `0600` on Unix
//!
//! The authorize URL does not contain the client secret or the PKCE verifier. Tokens are not printed.

use std::env;
use std::process::ExitCode;
use std::time::Duration;

use tesla_auth::{
    AuthClient, AuthorizeOptions, ClientConfig, Region, Scope, SecretString, TokenStore,
    bind_callback_listener, listen_for_callback, loopback_redirect_uri,
};
use tesla_fleet_api::{BearerToken, FleetClient, FleetError, VehicleTag};

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let client_id = env::var("TESLA_CLIENT_ID")?;
    let client_secret = env::var("TESLA_CLIENT_SECRET")?;
    let region = env::var("TESLA_REGION")
        .unwrap_or_else(|_| "na".to_owned())
        .parse::<Region>()?;
    let port = env::var("TESLA_CALLBACK_PORT")
        .ok()
        .map(|value| value.parse::<u16>())
        .transpose()?
        .unwrap_or(8734);
    let listener = bind_callback_listener(port).await?;
    let bound = listener.local_addr()?;
    let redirect_uri = match env::var("TESLA_REDIRECT_URI") {
        Ok(uri) => uri,
        Err(_) => loopback_redirect_uri(bound.port(), "/callback")?,
    };
    if !redirect_uri.contains(&bound.port().to_string()) {
        eprintln!(
            "warning: redirect URI does not contain callback port {}",
            bound.port()
        );
    }
    let scopes = match env::var("TESLA_SCOPES") {
        Ok(scopes) => Scope::parse_list(&scopes)?,
        Err(_) => Scope::third_party_vehicle(),
    };
    let client = AuthClient::new(ClientConfig {
        client_id,
        client_secret: SecretString::from(client_secret),
        redirect_uri,
        region,
        scopes,
    })?;
    let pending = client.begin_login(&AuthorizeOptions {
        prompt: Some("login".to_owned()),
        locale: Some("en-US".to_owned()),
        ..AuthorizeOptions::default()
    })?;
    println!("Open this URL and approve access:\n{}", pending.url());
    let callback = listen_for_callback(listener, "/callback", Duration::from_secs(300)).await?;
    let tokens = client.exchange_code(&pending, callback).await?;
    if let Ok(path) = env::var("TESLA_TOKEN_FILE") {
        let store = tesla_auth::FileTokenStore::new(path);
        store.save(&tokens).await?;
        eprintln!(
            "saved tokens (unix mode 0600) to {}",
            store.path().display()
        );
    }

    let fleet = FleetClient::for_region(
        region,
        BearerToken::from_secret(tokens.access_token().clone()),
    )?;
    let user_region = match fleet.user_region().await {
        Ok(region) => Some(region),
        Err(FleetError::IncorrectRegion { .. }) => None,
        Err(err) => return Err(err.into()),
    };
    let fleet = if let Some(user_region) = user_region.as_ref() {
        println!(
            "account region {} ({})",
            user_region.region, user_region.fleet_api_base_url
        );
        if user_region.known_region() == Some(region) {
            fleet
        } else {
            eprintln!(
                "token audience is {region}; listing from {}",
                user_region.fleet_api_base_url
            );
            FleetClient::builder(
                &user_region.fleet_api_base_url,
                BearerToken::from_secret(tokens.access_token().clone()),
            )?
            .build()?
        }
    } else {
        eprintln!("token was minted for {region}, and that host rejected it");
        eprintln!("sign in again with TESLA_REGION set to the account region");
        fleet
    };

    let page = fleet
        .list_vehicles(&tesla_fleet_api::ListVehiclesQuery::default())
        .await?;
    if page.vehicles.is_empty() {
        println!("no vehicles");
        return Ok(());
    }
    for vehicle in &page.vehicles {
        let name = vehicle.display_name.as_deref().unwrap_or("(unnamed)");
        let state = vehicle.state.as_deref().unwrap_or("unknown");
        println!("{name}  {}  {state}", vehicle.vin);
        if VehicleTag::new(&vehicle.vin).is_err() {
            eprintln!("skipping unexpected vin {}", vehicle.vin);
        }
    }
    Ok(())
}
