// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Process entry points for `serve`, `import-teslamate`, and `healthcheck`.

use std::time::Duration;

use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

use crate::app;
use crate::config::Config;
use crate::error::AppError;
use crate::import;

/// Runs the command selected on the command line.
///
/// # Errors
///
/// Returns an error when configuration, the database, or a request fails.
/// The message does not include tokens, the client secret, or the private key.
pub async fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let cli = Cli::parse();
    init_tracing();
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => app::serve().await?,
        Command::ImportTeslaMate { source, batch_size } => {
            let dest = Config::database_url()?;
            let stats = import::import_url(&dest, &source, batch_size).await?;
            println!(
                "imported cars={} addresses={} geofences={} drives={} charges={} charge_samples={} positions={}",
                stats.cars,
                stats.addresses,
                stats.geofences,
                stats.drives,
                stats.charges,
                stats.charge_samples,
                stats.positions
            );
        }
        Command::Healthcheck { url } => healthcheck(&url).await?,
    }
    Ok(())
}

#[derive(Parser)]
#[command(
    name = "tesla-logger",
    version,
    about = "Self-hosted archive of Tesla drives, charges, and vehicle state"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP server, Fleet API poller, and optional telemetry and MQTT tasks.
    Serve,
    /// Import cars, drives, positions, charges, addresses, and geofences from TeslaMate.
    #[command(name = "import-teslamate", visible_alias = "import-tesla-mate")]
    ImportTeslaMate {
        /// Read-only TeslaMate Postgres URL.
        #[arg(long, env = "TESLAMATE_DATABASE_URL")]
        source: String,
        /// Rows per batch for every table except positions.
        /// Positions are read 5000 rows at a time.
        #[arg(long, default_value_t = 1000)]
        batch_size: i64,
    },
    /// GET a URL and exit 0 when the status is successful.
    Healthcheck {
        /// URL to request. Defaults to the local health endpoint.
        #[arg(long, default_value = "http://127.0.0.1:4000/healthz")]
        url: String,
    },
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        EnvFilter::new("info,sqlx=warn,hyper=warn,h2=warn,rumqttc=warn,reqwest=warn")
    });
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();
}

async fn healthcheck(url: &str) -> Result<(), AppError> {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(4))
        .build()
        .map_err(|_| AppError::message("healthcheck client failed"))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| AppError::message("healthcheck request failed"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(AppError::message(format!(
            "healthcheck status {}",
            response.status()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, Command};
    use clap::Parser;

    #[test]
    fn import_accepts_the_readme_name_and_the_clap_alias() {
        for name in ["import-teslamate", "import-tesla-mate"] {
            let cli = Cli::try_parse_from([
                "tesla-logger",
                name,
                "--source",
                "postgres://localhost/teslamate",
            ])
            .unwrap_or_else(|err| panic!("{name}: {err}"));
            match cli.command.unwrap() {
                Command::ImportTeslaMate { source, batch_size } => {
                    assert_eq!(source, "postgres://localhost/teslamate");
                    assert_eq!(batch_size, 1000);
                }
                Command::Serve | Command::Healthcheck { .. } => {
                    panic!("{name} parsed as a different command");
                }
            }
        }
    }
}
