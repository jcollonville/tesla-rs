// SPDX-License-Identifier: MIT OR Apache-2.0

//! Wakes a vehicle and locks or unlocks it with a virtual key.
//!
//! The access token is read from `tesla-auth`'s file store. When the file is empty and
//! `TESLA_CLIENT_ID` / `TESLA_CLIENT_SECRET` are set, the program runs the same loopback login as
//! `list-vehicles` and saves the token set.
//!
//! Required for `lock` and `unlock`:
//!
//! * `TESLA_VIN`
//! * `TESLA_KEY_FILE` — P-256 SEC1 or PKCS#8 PEM. Mode `0600` is appropriate.
//! * `TESLA_TOKEN_FILE`
//! * `TESLA_CLIENT_ID` and `TESLA_CLIENT_SECRET` (needed to refresh)
//!
//! Optional:
//!
//! * `TESLA_REGION` — `na` (default), `eu`, or `cn`
//! * `TESLA_COMMAND` — `lock` or `unlock`, used when no argument is passed
//!
//! `keygen` writes a new key to `TESLA_KEY_FILE` and prints the public key. It refuses to replace
//! an existing file unless `TESLA_KEY_FORCE=1`.
//!
//! `wake_up` is billable. The private key and tokens are not printed.

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use tesla_auth::{
    AuthClient, AuthError, AuthorizeOptions, ClientConfig, FileTokenStore, Region, Scope, Session,
    bind_callback_listener, listen_for_callback, loopback_redirect_uri,
};
use tesla_fleet_api::{FleetClient, FleetError};
use tesla_vehicle_command::{FleetTransport, PrivateKey, PrivateKeyFormat, Vehicle};

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
    let mut args = env::args().skip(1);
    let command = match args.next() {
        Some(value) => value,
        None => env::var("TESLA_COMMAND").unwrap_or_else(|_| "lock".to_owned()),
    };
    if args.next().is_some() {
        return Err("usage: lock-unlock [keygen|lock|unlock]".into());
    }
    let key_path = PathBuf::from(env::var("TESLA_KEY_FILE")?);
    if command == "keygen" {
        return keygen(&key_path);
    }
    let vin = env::var("TESLA_VIN")?;
    let region = env::var("TESLA_REGION")
        .unwrap_or_else(|_| "na".to_owned())
        .parse::<Region>()?;
    let key = PrivateKey::read_file(&key_path)?;
    let fleet = fleet_client(region).await?;
    let transport = FleetTransport::new(fleet, vin)?;
    let mut vehicle = Vehicle::new(transport, key)?;
    eprintln!("public key {}", hex(vehicle.public_key()));
    let wake = vehicle.wake().await?;
    println!("wake state {}", wake.state.as_deref().unwrap_or("unknown"));
    if !wake.online {
        return Err("vehicle is not online after wake_up; retry when it is online".into());
    }
    match command.as_str() {
        "lock" => vehicle.lock().await?,
        "unlock" => vehicle.unlock().await?,
        other => return Err(format!("unknown command {other}").into()),
    }
    println!("{command} sent");
    Ok(())
}

fn keygen(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    if path.exists() && env::var("TESLA_KEY_FORCE").ok().as_deref() != Some("1") {
        return Err("key file already exists; set TESLA_KEY_FORCE=1 to replace it".into());
    }
    let key = PrivateKey::generate()?;
    key.write_pem_file(path, PrivateKeyFormat::Sec1)?;
    println!("{}", key.public_key_pem()?);
    eprintln!("wrote private key to {}", path.display());
    Ok(())
}

async fn fleet_client(region: Region) -> Result<FleetClient<Session<FileTokenStore>>, FleetError> {
    let client_id = env::var("TESLA_CLIENT_ID")
        .map_err(|_| FleetError::InvalidRequest("TESLA_CLIENT_ID is required to refresh tokens"))?;
    let client_secret = env::var("TESLA_CLIENT_SECRET").map_err(|_| {
        FleetError::InvalidRequest("TESLA_CLIENT_SECRET is required to refresh tokens")
    })?;
    let token_path = env::var("TESLA_TOKEN_FILE")
        .map_err(|_| FleetError::InvalidRequest("TESLA_TOKEN_FILE is required"))?;
    let port = env::var("TESLA_CALLBACK_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(8734);
    let redirect_uri = env::var("TESLA_REDIRECT_URI").unwrap_or_else(|_| {
        loopback_redirect_uri(port, "/callback")
            .unwrap_or_else(|_| format!("http://127.0.0.1:{port}/callback"))
    });
    let scopes = match env::var("TESLA_SCOPES") {
        Ok(scopes) => Scope::parse_list(&scopes).map_err(FleetError::from)?,
        Err(_) => Scope::third_party_vehicle(),
    };
    let auth = AuthClient::new(ClientConfig {
        client_id,
        client_secret: tesla_auth::SecretString::from(client_secret),
        redirect_uri,
        region,
        scopes,
    })
    .map_err(FleetError::from)?;
    let store = FileTokenStore::new(token_path);
    let session = Session::new(auth, store);
    if let Err(AuthError::NotAuthenticated) = session.access_token().await {
        login(&session).await.map_err(FleetError::from)?;
    }
    FleetClient::for_region(region, session)
}

async fn login(session: &Session<FileTokenStore>) -> Result<(), AuthError> {
    let listener = bind_callback_listener(
        env::var("TESLA_CALLBACK_PORT")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(8734),
    )
    .await?;
    let pending = session.client().begin_login(&AuthorizeOptions {
        prompt: Some("login".to_owned()),
        locale: Some("en-US".to_owned()),
        ..AuthorizeOptions::default()
    })?;
    println!("Open this URL and approve access:\n{}", pending.url());
    let callback = listen_for_callback(listener, "/callback", Duration::from_secs(300)).await?;
    session.complete_login(&pending, callback).await?;
    eprintln!("saved tokens");
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}
