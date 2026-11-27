// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Process entry points for `serve`, `import-teslamate`, `repair-drive-ends`,
//! `rebuild-drive`, `rebuild-charge`, `register-partner`, and `healthcheck`.

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
    let logs = crate::logs::LogBuffer::new(400);
    init_tracing(logs.clone());
    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => app::serve(logs).await?,
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
        Command::RepairDriveEnds { apply } => {
            let url = Config::database_url()?;
            let report = crate::persist::repair_drive_ends_at(&url, apply).await?;
            print!("{}", crate::persist::format_drive_end_repair(&report));
        }
        Command::RebuildDrive {
            vehicle_id,
            from,
            to,
            apply,
        } => rebuild_drive(vehicle_id, from, to, apply).await?,
        Command::RebuildCharge {
            vehicle_id,
            from,
            to,
            apply,
        } => rebuild_charge(vehicle_id, from, to, apply).await?,
        Command::RegisterPartner => {
            let config = Config::load()?;
            crate::partner::register(&config).await?;
            println!(
                "registered domain={}",
                crate::partner::partner_domain(&config.domain)?
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
    /// Recompute closed-drive ends from the last exact 0 km/h sample.
    ///
    /// Prints a dry-run unless `--apply` is set. Never run by `serve`.
    #[command(name = "repair-drive-ends")]
    RepairDriveEnds {
        /// Write the new ends. Without this flag the command only prints the plan.
        #[arg(long)]
        apply: bool,
    },
    /// Rebuild one drive from positions left without a drive by a discard.
    ///
    /// Prints a dry-run unless `--apply` is set. Refuses a window that overlaps
    /// an existing drive. Never run by `serve`.
    #[command(name = "rebuild-drive")]
    RebuildDrive {
        /// `vehicles.id` of the car.
        #[arg(long)]
        vehicle_id: i64,
        /// Window start, RFC 3339 (for example `2026-01-01T10:00:00Z`).
        #[arg(long)]
        from: chrono::DateTime<chrono::Utc>,
        /// Window end, RFC 3339. At most 24 hours after `--from`.
        #[arg(long)]
        to: chrono::DateTime<chrono::Utc>,
        /// Write the drive. Without this flag the command only prints the plan.
        #[arg(long)]
        apply: bool,
    },
    /// Merge the live charges that start in a window into one charge.
    ///
    /// Prints a dry-run unless `--apply` is set. Refuses an open charge, a pause
    /// longer than the charge resume grace, or a drive in between. The merged
    /// rows are soft-deleted. Never run by `serve`.
    #[command(name = "rebuild-charge")]
    RebuildCharge {
        /// `vehicles.id` of the car.
        #[arg(long)]
        vehicle_id: i64,
        /// Window start, RFC 3339 (for example `2026-01-01T10:00:00Z`).
        #[arg(long)]
        from: chrono::DateTime<chrono::Utc>,
        /// Window end, RFC 3339. At most 24 hours after `--from`.
        #[arg(long)]
        to: chrono::DateTime<chrono::Utc>,
        /// Write the merge. Without this flag the command only prints the plan.
        #[arg(long)]
        apply: bool,
    },
    /// Register `TESLA_DOMAIN` with Tesla (`POST /api/1/partner_accounts`).
    ///
    /// The public key must already be reachable at
    /// `https://<domain>/.well-known/appspecific/com.tesla.3p.public-key.pem`.
    #[command(name = "register-partner")]
    RegisterPartner,
    /// GET a URL and exit 0 when the status is successful.
    Healthcheck {
        /// URL to request. Defaults to the local health endpoint.
        #[arg(long, default_value = "http://127.0.0.1:4000/healthz")]
        url: String,
    },
}

async fn rebuild_drive(
    vehicle_id: i64,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    apply: bool,
) -> Result<(), AppError> {
    let url = Config::database_url()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await?;
    let (report, jobs) = crate::persist::rebuild_drive(&pool, vehicle_id, from, to, apply).await?;
    print!("{}", crate::persist::format_drive_rebuild(&report));
    if !jobs.is_empty() {
        // Addresses are best effort: the drive is already committed.
        match geocoder(&pool).await {
            Ok(geocoder) => {
                crate::persist::settle_addresses(&pool, &geocoder, jobs).await;
                println!("addresses=requested");
            }
            Err(_) => println!("addresses=skipped"),
        }
    }
    pool.close().await;
    Ok(())
}

async fn rebuild_charge(
    vehicle_id: i64,
    from: chrono::DateTime<chrono::Utc>,
    to: chrono::DateTime<chrono::Utc>,
    apply: bool,
) -> Result<(), AppError> {
    let url = Config::database_url()?;
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await?;
    let (report, jobs) = crate::persist::rebuild_charge(&pool, vehicle_id, from, to, apply).await?;
    print!("{}", crate::persist::format_charge_rebuild(&report));
    if !jobs.is_empty() {
        // Addresses are best effort: the merge is already committed.
        match geocoder(&pool).await {
            Ok(geocoder) => {
                crate::persist::settle_addresses(&pool, &geocoder, jobs).await;
                println!("addresses=requested");
            }
            Err(_) => println!("addresses=skipped"),
        }
    }
    pool.close().await;
    Ok(())
}

async fn geocoder(pool: &sqlx::PgPool) -> Result<crate::geocode::Geocoder, AppError> {
    let boot = std::sync::Arc::new(Config::load()?);
    let runtime =
        crate::settings::RuntimeConfig::load(pool.clone(), std::sync::Arc::clone(&boot)).await?;
    crate::geocode::Geocoder::from_runtime(runtime.subscribe(), &boot)
}

fn init_tracing(logs: crate::logs::LogBuffer) {
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let fmt_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| default_filter());
    let fmt_layer = tracing_subscriber::fmt::layer()
        .with_target(false)
        .with_filter(fmt_filter);
    let capture = crate::logs::CaptureLayer::new(logs).with_filter(default_filter());
    let _ = tracing_subscriber::registry()
        .with(fmt_layer)
        .with(capture)
        .try_init();
}

fn default_filter() -> EnvFilter {
    EnvFilter::new("info,sqlx=warn,hyper=warn,h2=warn,rumqttc=warn,reqwest=warn")
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
                Command::Serve
                | Command::Healthcheck { .. }
                | Command::RepairDriveEnds { .. }
                | Command::RebuildDrive { .. }
                | Command::RebuildCharge { .. }
                | Command::RegisterPartner => {
                    panic!("{name} parsed as a different command");
                }
            }
        }
    }

    #[test]
    fn repair_drive_ends_defaults_to_a_dry_run() {
        let dry = Cli::try_parse_from(["tesla-logger", "repair-drive-ends"]).unwrap();
        match dry.command.unwrap() {
            Command::RepairDriveEnds { apply } => assert!(!apply),
            Command::Serve
            | Command::ImportTeslaMate { .. }
            | Command::Healthcheck { .. }
            | Command::RebuildDrive { .. }
            | Command::RebuildCharge { .. }
            | Command::RegisterPartner => {
                panic!("dry-run parsed as a different command");
            }
        }
        let write = Cli::try_parse_from(["tesla-logger", "repair-drive-ends", "--apply"]).unwrap();
        match write.command.unwrap() {
            Command::RepairDriveEnds { apply } => assert!(apply),
            Command::Serve
            | Command::ImportTeslaMate { .. }
            | Command::Healthcheck { .. }
            | Command::RebuildDrive { .. }
            | Command::RebuildCharge { .. }
            | Command::RegisterPartner => {
                panic!("apply parsed as a different command");
            }
        }
    }

    #[test]
    fn rebuild_drive_defaults_to_a_dry_run() {
        let cli = Cli::try_parse_from([
            "tesla-logger",
            "rebuild-drive",
            "--vehicle-id",
            "7",
            "--from",
            "2026-01-01T10:00:00Z",
            "--to",
            "2026-01-01T10:15:00Z",
        ])
        .unwrap();
        match cli.command {
            Some(Command::RebuildDrive {
                vehicle_id,
                from,
                to,
                apply,
            }) => {
                assert_eq!(vehicle_id, 7);
                assert_eq!((to - from).num_minutes(), 15);
                assert!(!apply);
            }
            _ => panic!("parsed as a different command"),
        }
        assert!(
            Cli::try_parse_from(["tesla-logger", "rebuild-drive", "--vehicle-id", "7"]).is_err(),
            "the window is required"
        );
    }

    #[test]
    fn rebuild_charge_defaults_to_a_dry_run() {
        let cli = Cli::try_parse_from([
            "tesla-logger",
            "rebuild-charge",
            "--vehicle-id",
            "7",
            "--from",
            "2026-11-27T01:25:00Z",
            "--to",
            "2026-11-27T03:50:00Z",
        ])
        .unwrap();
        match cli.command {
            Some(Command::RebuildCharge {
                vehicle_id, apply, ..
            }) => {
                assert_eq!(vehicle_id, 7);
                assert!(!apply);
            }
            _ => panic!("parsed as a different command"),
        }
    }

    #[test]
    fn register_partner_is_a_command() {
        let cli = Cli::try_parse_from(["tesla-logger", "register-partner"]).unwrap();
        assert!(matches!(cli.command, Some(Command::RegisterPartner)));
    }
}
