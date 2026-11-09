// SPDX-License-Identifier: MIT OR Apache-2.0

//! Listens for Tesla Fleet Telemetry and prints decoded fields.
//!
//! Required:
//!
//! * `TESLA_TELEMETRY_CERT` — PEM server certificate chain, leaf first
//! * `TESLA_TELEMETRY_KEY` — PEM private key
//!
//! Optional:
//!
//! * `TESLA_TELEMETRY_BIND` — default `0.0.0.0:4443`
//! * `TESLA_TELEMETRY_CA` — `prod` (default) or `eng`
//! * `TESLA_TELEMETRY_CLIENT_CA` — extra PEM appended to the Tesla bundle
//!
//! The Tesla CA bundles are the ones shipped with the reference server. This example does not
//! prove that a live vehicle certificate chains to them.

use std::env;
use std::fs;
use std::net::SocketAddr;
use std::process::ExitCode;

use tesla_fleet_telemetry::{ClientCa, ServerConfig, TelemetryServer};

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
    let cert_path = env::var("TESLA_TELEMETRY_CERT")?;
    let key_path = env::var("TESLA_TELEMETRY_KEY")?;
    let bind: SocketAddr = env::var("TESLA_TELEMETRY_BIND")
        .unwrap_or_else(|_| "0.0.0.0:4443".to_owned())
        .parse()?;
    let mut config = ServerConfig::new(
        bind,
        fs::read_to_string(cert_path)?,
        fs::read_to_string(key_path)?,
    );
    let base = match env::var("TESLA_TELEMETRY_CA").as_deref() {
        Ok("eng") => "eng",
        _ => "prod",
    };
    config.client_ca = match (base, env::var("TESLA_TELEMETRY_CLIENT_CA")) {
        ("eng", Ok(path)) => ClientCa::TeslaEngAndPem(fs::read_to_string(path)?),
        ("eng", Err(_)) => ClientCa::TeslaEng,
        (_, Ok(path)) => ClientCa::TeslaProdAndPem(fs::read_to_string(path)?),
        (_, Err(_)) => ClientCa::TeslaProd,
    };
    let mut server = TelemetryServer::bind(config).await?;
    println!("listening on {}", server.local_addr());
    let incoming = server.incoming();
    while let Some(record) = incoming.recv().await {
        println!("{}", record.summary());
    }
    Ok(())
}
