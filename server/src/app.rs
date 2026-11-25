// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tesla_auth::{AuthClient, ClientConfig, PendingAuthorization, Scope, Session};
use tesla_owner_api::OwnerLogin;
use tokio::net::TcpListener;
use tokio::sync::{Mutex, Notify, broadcast, watch};

use crate::accounts::{self, DbTokenStore};
use crate::config::Config;
use crate::crypto::read_public_key;
use crate::debug::DebugBoard;
use crate::error::AppError;
use crate::geocode::Geocoder;
use crate::http;
use crate::http::{Basemap, CarRenders};
use crate::ingest::{self, Archive, FleetPoll, PollPolicy};
use crate::logs::LogBuffer;
use crate::mqtt;
use crate::owner_stream::{self, StreamBoard};
use crate::persist::LiveEvent;
use crate::telemetry;

/// One Fleet authorization-code login waiting for Tesla's redirect.
///
/// `session_hash` is the SHA-256 of the logger session id, the same value stored
/// in `sessions.id_hash`. The raw session id and the PKCE verifier stay out of logs.
pub(crate) struct PendingLogin {
    pub(crate) authorization: PendingAuthorization,
    pub(crate) session_hash: String,
    pub(crate) created: Instant,
}

/// Process-wide handles. Cheap to clone.
#[derive(Clone)]
pub(crate) struct AppState {
    pub inner: Arc<Inner>,
}

pub(crate) struct Inner {
    pub config: Config,
    pub runtime: crate::settings::RuntimeConfig,
    pub db: PgPool,
    pub session: Arc<Session<DbTokenStore>>,
    pub fleet_store: Arc<DbTokenStore>,
    pub owner_store: Arc<DbTokenStore>,
    pub fleet_account_id: i64,
    pub owner_account_id: i64,
    /// In-progress Fleet OAuth logins, keyed by the unguessable `state`.
    ///
    /// Each entry is bound to the logger session that called `/auth/login`.
    pub pending: Mutex<HashMap<String, PendingLogin>>,
    pub public_key_pem: String,
    pub events: broadcast::Sender<LiveEvent>,
    pub revealed: Mutex<HashMap<String, String>>,
    pub archive: Arc<Archive>,
    pub owner_pending: Mutex<Option<(OwnerLogin, Instant)>>,
    pub stream_board: StreamBoard,
    pub owner_tokens: watch::Sender<u64>,
    pub logs: LogBuffer,
    pub debug: DebugBoard,
    pub car_renders: CarRenders,
    pub basemap: Basemap,
}

pub(crate) async fn serve(logs: LogBuffer) -> Result<(), AppError> {
    let config = Config::load()?;
    logs.install_config(&config);
    let state = build(config, logs).await?;
    let listen = state.inner.config.listen_addr;
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let poll_state = state.clone();
    let poll_rx = shutdown_rx.clone();
    let poll = tokio::spawn(async move {
        let policy = policy_from(&poll_state.inner.config);
        let fleet = match FleetPoll::connect(
            poll_state.inner.config.region,
            Arc::clone(&poll_state.inner.session),
            poll_state.inner.debug.clone(),
        ) {
            Ok(fleet) => fleet,
            Err(err) => {
                let _ = accounts::set_last_error(
                    &poll_state.inner.db,
                    poll_state.inner.fleet_account_id,
                    &err.to_string(),
                )
                .await;
                return;
            }
        };
        ingest::run_poll(
            &fleet,
            Arc::clone(&poll_state.inner.archive),
            policy,
            Some(poll_state.inner.fleet_account_id),
            poll_rx,
        )
        .await;
    });
    let telemetry_state = state.clone();
    let telemetry_rx = shutdown_rx.clone();
    let telemetry_task = tokio::spawn(async move {
        telemetry::run(
            &telemetry_state.inner.config,
            Arc::clone(&telemetry_state.inner.archive),
            telemetry_state.inner.debug.clone(),
            telemetry_rx,
        )
        .await;
    });
    let mqtt_state = state.clone();
    let mqtt_rx = shutdown_rx.clone();
    let mqtt_task = tokio::spawn(async move {
        mqtt::run(
            mqtt_state.inner.runtime.clone(),
            mqtt_state.inner.db.clone(),
            mqtt_state.inner.events.subscribe(),
            mqtt_state.inner.debug.clone(),
            mqtt_rx,
        )
        .await;
    });
    let owner_state = state.clone();
    let owner_rx = shutdown_rx.clone();
    let owner_task = tokio::spawn(async move {
        owner_stream::run(
            &owner_state.inner.config,
            Arc::clone(&owner_state.inner.archive),
            owner_state.inner.stream_board.clone(),
            owner_state.inner.debug.clone(),
            owner_state.inner.owner_tokens.subscribe(),
            owner_rx,
        )
        .await;
    });
    let grace_state = state.clone();
    let mut grace_rx = shutdown_rx.clone();
    let grace_task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                changed = grace_rx.changed() => {
                    if changed.is_err() || *grace_rx.borrow() {
                        break;
                    }
                }
                _ = tick.tick() => {
                    if let Err(err) = ingest::expire_drives(&grace_state.inner.archive, chrono::Utc::now()).await {
                        tracing::warn!(error = %err, "drive grace tick failed");
                    }
                }
            }
        }
    });
    let listener = TcpListener::bind(listen).await?;
    let data_source = state.inner.archive.data_source();
    tracing::info!(data_source, %listen, "fleet data source");
    axum::serve(listener, http::router(state))
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
            let _ = shutdown_tx.send(true);
        })
        .await
        .map_err(|err| AppError::message(err.to_string()))?;
    let _ = poll.await;
    let _ = telemetry_task.await;
    let _ = mqtt_task.await;
    let _ = owner_task.await;
    let _ = grace_task.await;
    Ok(())
}

pub(crate) async fn build(config: Config, logs: LogBuffer) -> Result<AppState, AppError> {
    let public_key_pem = read_public_key(&config.private_key_path)?;
    if config.token_encryption_key.is_none() {
        tracing::warn!(
            "TESLA_TOKEN_ENCRYPTION_KEY is unset; Fleet and Owner tokens are stored in plaintext"
        );
    }
    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await?;
    sqlx::migrate!("./migrations").run(&db).await?;
    crate::identity::bootstrap(&db, &config).await?;
    let imported = accounts::import_token_files(&db, &config).await?;
    let fleet_store = Arc::new(DbTokenStore::open(
        db.clone(),
        imported.fleet_id,
        config.token_encryption_key.as_deref(),
    )?);
    let owner_store = Arc::new(DbTokenStore::open(
        db.clone(),
        imported.owner_id,
        config.token_encryption_key.as_deref(),
    )?);
    let auth = AuthClient::new(ClientConfig {
        client_id: config.client_id.clone(),
        client_secret: config.client_secret.clone(),
        redirect_uri: config.redirect_uri.clone(),
        region: config.region,
        scopes: Scope::third_party_vehicle(),
    })?;
    let session = Arc::new(Session::new(auth, (*fleet_store).clone()));
    let (events, _) = broadcast::channel(256);
    let (stream_kick, _) = broadcast::channel(32);
    let (owner_tokens, _) = watch::channel(0_u64);
    let debug = DebugBoard::new(logs.clone());
    debug.apply_config(&config);
    let gps = crate::position_mode::GpsBoard::new(debug.clone(), Arc::new(Notify::new()));
    let debug_saved = debug.clone();
    fleet_store.on_saved(std::sync::Arc::new(move |expires| {
        debug_saved.token_saved("fleet", expires);
    }));
    let owner_saved = debug.clone();
    owner_store.on_saved(std::sync::Arc::new(move |expires| {
        owner_saved.token_saved("owner", expires);
    }));
    let boot = Arc::new(config);
    let runtime = crate::settings::RuntimeConfig::load(db.clone(), Arc::clone(&boot)).await?;
    let geocoder = Geocoder::from_runtime(runtime.subscribe(), &boot)?;
    let mut archive = Archive::new(db.clone(), geocoder, events.clone(), stream_kick, gps);
    archive.fleet_account_id = Some(imported.fleet_id);
    archive.owner_account_id = Some(imported.owner_id);
    let snap = runtime.snapshot();
    archive.drive_grace = crate::state::DriveGrace::from_secs(
        snap.drive_end_grace.as_secs(),
        snap.drive_park_grace.as_secs(),
    );
    archive.runtime = Some(runtime.clone());
    if boot.telemetry_configured() {
        match boot
            .telemetry_cert_path
            .as_ref()
            .map(std::fs::read_to_string)
        {
            Some(Ok(ca)) if !ca.trim().is_empty() => {
                archive.telemetry = Some(crate::ingest::TelemetryTarget::new(
                    boot.telemetry_hostname.clone(),
                    boot.telemetry_port,
                    ca,
                ));
                tracing::info!(
                    host = %boot.telemetry_hostname,
                    port = boot.telemetry_port,
                    data_source = "telemetry",
                    "fleet telemetry is the data source"
                );
            }
            Some(Ok(_)) => {
                tracing::error!("telemetry certificate is empty; staying on polling");
            }
            Some(Err(err)) => {
                tracing::error!(error = %err, "telemetry certificate could not be read; staying on polling");
            }
            None => {}
        }
    } else {
        tracing::info!(data_source = "polling", "fleet polling is the data source");
    }
    let archive = Arc::new(archive);
    let car_renders = CarRenders::from_config(&boot);
    let basemap = Basemap::from_config(&boot);
    Ok(AppState {
        inner: Arc::new(Inner {
            config: (*boot).clone(),
            runtime,
            db,
            session,
            fleet_store,
            owner_store,
            fleet_account_id: imported.fleet_id,
            owner_account_id: imported.owner_id,
            pending: Mutex::new(HashMap::new()),
            public_key_pem,
            events,
            revealed: Mutex::new(HashMap::new()),
            archive,
            owner_pending: Mutex::new(None),
            stream_board: StreamBoard::default(),
            owner_tokens,
            logs,
            debug,
            car_renders,
            basemap,
        }),
    })
}

fn policy_from(config: &Config) -> PollPolicy {
    PollPolicy {
        drive_fallback: config.drive_fallback_poll,
        charge: config.charge_poll,
        parked: config.parked_poll,
        asleep: config.asleep_poll,
        backoff: config.backoff_poll,
        parked_live_limit: config.parked_live_limit,
        gps_fallback: config.gps_fallback_poll,
        daily_cap: config.fleet_daily_cap,
        monthly_cap: config.fleet_monthly_cap,
        blocked_probe: config.fleet_blocked_probe,
        telemetry_fallback: config.telemetry_fallback,
        telemetry_mode: config.telemetry_configured(),
        dynamic_budget: config.fleet_dynamic_budget,
        month_seed: config.fleet_month_seed,
        month_seed_month: config.fleet_month_seed_month,
        now: None,
    }
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut terminate =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(signal) => signal,
                Err(err) => {
                    tracing::warn!(error = %err, "could not listen for SIGTERM");
                    let _ = ctrl_c.await;
                    return;
                }
            };
        tokio::select! {
            _ = ctrl_c => {}
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}

#[cfg(test)]
pub(crate) async fn test_state(pool: PgPool) -> AppState {
    test_state_with(pool, None).await
}

#[cfg(test)]
pub(crate) async fn test_state_with(pool: PgPool, energy_price_per_kwh: Option<f64>) -> AppState {
    test_state_full(pool, energy_price_per_kwh, None, None).await
}

/// Test state whose car renders come from `renders`, normally a mock server.
#[cfg(test)]
pub(crate) async fn test_state_renders(pool: PgPool, renders: CarRenders) -> AppState {
    test_state_full(pool, None, Some(renders), None).await
}

/// Points the Fleet token endpoint at a local mock and stores tokens on `fleet_account_id`.
#[cfg(test)]
pub(crate) struct OauthDouble {
    pub token_endpoint: url::Url,
    pub fleet_account_id: i64,
}

#[cfg(test)]
pub(crate) async fn test_state_oauth(pool: PgPool, oauth: OauthDouble) -> AppState {
    test_state_full(pool, None, None, Some(oauth)).await
}

#[cfg(test)]
#[allow(clippy::unused_async)]
async fn test_state_full(
    pool: PgPool,
    energy_price_per_kwh: Option<f64>,
    renders: Option<CarRenders>,
    oauth: Option<OauthDouble>,
) -> AppState {
    use std::collections::BTreeMap;

    use p256::SecretKey;
    use p256::elliptic_curve::Generate;
    use p256::pkcs8::{EncodePrivateKey, LineEnding};

    let dir =
        std::env::temp_dir().join(format!("tl-app-{}", crate::crypto::random_token().unwrap()));
    std::fs::create_dir_all(&dir).unwrap();
    let key_path = dir.join("key.pem");
    let secret = SecretKey::generate();
    let pem = secret.to_pkcs8_pem(LineEnding::LF).unwrap();
    std::fs::write(&key_path, pem.as_str()).unwrap();
    let mut map = BTreeMap::new();
    map.insert("TESLA_CLIENT_ID".to_owned(), "client".to_owned());
    map.insert("TESLA_CLIENT_SECRET".to_owned(), "secret".to_owned());
    map.insert(
        "TESLA_FLEET_AUDIENCE".to_owned(),
        "https://fleet-api.prd.eu.vn.cloud.tesla.com".to_owned(),
    );
    map.insert("TESLA_DOMAIN".to_owned(), "logger.test".to_owned());
    map.insert(
        "TESLA_PUBLIC_BASE_URL".to_owned(),
        "http://127.0.0.1:4000".to_owned(),
    );
    map.insert("DATABASE_URL".to_owned(), "postgres://unused".to_owned());
    map.insert(
        "TESLA_PRIVATE_KEY_PATH".to_owned(),
        key_path.display().to_string(),
    );
    map.insert(
        "TESLA_TOKEN_STORE_PATH".to_owned(),
        dir.join("tokens.json").display().to_string(),
    );
    map.insert(
        "TESLA_LOGGER_UI_PASSWORD".to_owned(),
        "test-password".to_owned(),
    );
    map.insert(
        "TESLA_TOKEN_ENCRYPTION_KEY".to_owned(),
        "test-key".to_owned(),
    );
    map.insert("NOMINATIM_ENABLED".to_owned(), "false".to_owned());
    map.insert("CAR_RENDERS_ENABLED".to_owned(), "false".to_owned());
    if let Some(price) = energy_price_per_kwh {
        map.insert("ENERGY_PRICE_PER_KWH".to_owned(), price.to_string());
    }
    let config = Config::from_map(&map).unwrap();
    let public_key_pem = read_public_key(&config.private_key_path).unwrap();
    let fleet_account_id = oauth.as_ref().map_or(0, |oauth| oauth.fleet_account_id);
    let fleet_store = Arc::new(
        DbTokenStore::open(
            pool.clone(),
            fleet_account_id,
            config.token_encryption_key.as_deref(),
        )
        .unwrap(),
    );
    let owner_store = Arc::new(
        DbTokenStore::open(pool.clone(), 0, config.token_encryption_key.as_deref()).unwrap(),
    );
    let client_config = ClientConfig {
        client_id: config.client_id.clone(),
        client_secret: config.client_secret.clone(),
        redirect_uri: config.redirect_uri.clone(),
        region: config.region,
        scopes: Scope::third_party_vehicle(),
    };
    let auth = match oauth {
        Some(oauth) => {
            let mut endpoints = tesla_auth::Endpoints::for_region(config.region).unwrap();
            endpoints.token = oauth.token_endpoint;
            AuthClient::builder(client_config, endpoints)
                .unwrap()
                .build()
                .unwrap()
        }
        None => AuthClient::new(client_config).unwrap(),
    };
    let session = Arc::new(Session::new(auth, (*fleet_store).clone()));
    let (events, _) = broadcast::channel(16);
    let (stream_kick, _) = broadcast::channel(8);
    let (owner_tokens, _) = watch::channel(0_u64);
    let logs = LogBuffer::new(50);
    logs.install_config(&config);
    let debug = DebugBoard::new(logs.clone());
    debug.apply_config(&config);
    let gps = crate::position_mode::GpsBoard::new(debug.clone(), Arc::new(Notify::new()));
    let geocoder = Geocoder::disabled();
    let archive = Arc::new(Archive::new(
        pool.clone(),
        geocoder,
        events.clone(),
        stream_kick,
        gps,
    ));
    let boot = Arc::new(config);
    let runtime = crate::settings::RuntimeConfig::from_boot(pool.clone(), Arc::clone(&boot));
    AppState {
        inner: Arc::new(Inner {
            config: (*boot).clone(),
            runtime,
            db: pool,
            session,
            fleet_store,
            owner_store,
            fleet_account_id,
            owner_account_id: 0,
            pending: Mutex::new(HashMap::new()),
            public_key_pem,
            events,
            revealed: Mutex::new(HashMap::new()),
            archive,
            owner_pending: Mutex::new(None),
            stream_board: StreamBoard::default(),
            owner_tokens,
            logs,
            debug,
            car_renders: renders.unwrap_or_else(|| CarRenders::new(false, String::new(), None)),
            basemap: Basemap::disabled(),
        }),
    }
}
