// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Hybrid configuration: environment defaults overridden by the `settings`
//! and `vehicle_settings` tables, published on a watch channel for hot reload.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use sqlx::PgPool;
use tokio::sync::watch;

use crate::config::Config;
use crate::error::AppError;

/// How a setting is applied after it changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ReloadMode {
    /// Consumers read the watch snapshot; no process restart.
    Hot,
    /// Shown in the effective table but only changed via the environment.
    EnvOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SettingGroup {
    Polling,
    Fleet,
    Nominatim,
    Mqtt,
    Misc,
}

impl SettingGroup {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Polling => "Polling",
            Self::Fleet => "Quotas Fleet",
            Self::Nominatim => "Nominatim",
            Self::Mqtt => "MQTT",
            Self::Misc => "Divers",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ValueKind {
    Bool,
    U64 { min: u64, max: u64 },
    F64,
    Text,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct SettingSpec {
    pub key: &'static str,
    pub group: SettingGroup,
    pub label: &'static str,
    pub kind: ValueKind,
    #[allow(dead_code)]
    pub mode: ReloadMode,
}

/// Editable keys that can be stored in the settings tables.
pub(crate) const REGISTRY: &[SettingSpec] = &[
    SettingSpec {
        key: "POLL_DRIVE_FALLBACK_SECS",
        group: SettingGroup::Polling,
        label: "Poll conduite (repli)",
        kind: ValueKind::U64 {
            min: crate::budget::DRIVE_FALLBACK_MIN_SECS,
            max: crate::budget::DRIVE_FALLBACK_MAX_SECS,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "POLL_CHARGE_SECS",
        group: SettingGroup::Polling,
        label: "Poll charge",
        kind: ValueKind::U64 {
            min: crate::budget::CHARGE_POLL_MIN_SECS,
            max: crate::budget::CHARGE_POLL_MAX_SECS,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "POLL_PARKED_SECS",
        group: SettingGroup::Polling,
        label: "Poll à l'arrêt",
        kind: ValueKind::U64 {
            min: 60,
            max: 86_400,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "POLL_ASLEEP_SECS",
        group: SettingGroup::Polling,
        label: "Poll endormi",
        kind: ValueKind::U64 {
            min: crate::budget::IDLE_MIN_SECS,
            max: 86_400,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "POLL_BACKOFF_SECS",
        group: SettingGroup::Polling,
        label: "Repli au repos",
        kind: ValueKind::U64 {
            min: crate::budget::IDLE_MIN_SECS,
            max: 86_400,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "POLL_GPS_FALLBACK_SECS",
        group: SettingGroup::Polling,
        label: "Repli GPS",
        kind: ValueKind::U64 {
            min: 60,
            max: 86_400,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "POLL_PARKED_LIVE_LIMIT",
        group: SettingGroup::Polling,
        label: "Lectures à l'arrêt",
        kind: ValueKind::U64 { min: 0, max: 100 },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "DRIVE_END_GRACE_SECS",
        group: SettingGroup::Polling,
        label: "Délai fin de trajet",
        kind: ValueKind::U64 {
            min: crate::state::DRIVE_END_GRACE_MIN_SECS,
            max: crate::state::DRIVE_END_GRACE_MAX_SECS,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "DRIVE_PARK_GRACE_SECS",
        group: SettingGroup::Polling,
        label: "Délai à 0 km/h",
        kind: ValueKind::U64 {
            min: crate::state::DRIVE_PARK_GRACE_MIN_SECS,
            max: crate::state::DRIVE_PARK_GRACE_MAX_SECS,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "DRIVE_P_CONFIRM_SECS",
        group: SettingGroup::Polling,
        label: "Confirmation parking",
        kind: ValueKind::U64 {
            min: crate::state::DRIVE_P_CONFIRM_MIN_SECS,
            max: crate::state::DRIVE_P_CONFIRM_MAX_SECS,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "DRIVE_STOP_IN_GEAR_SECS",
        group: SettingGroup::Polling,
        label: "Arrêt en prise",
        kind: ValueKind::U64 {
            min: crate::state::DRIVE_STOP_IN_GEAR_MIN_SECS,
            max: crate::state::DRIVE_STOP_IN_GEAR_MAX_SECS,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "FLEET_DATA_DAILY_CAP",
        group: SettingGroup::Fleet,
        label: "Plafond vehicle_data / jour",
        kind: ValueKind::U64 {
            min: crate::budget::DAILY_CAP_MIN,
            max: crate::budget::DAILY_CAP_MAX,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "FLEET_DATA_MONTHLY_CAP",
        group: SettingGroup::Fleet,
        label: "Plafond vehicle_data / mois",
        kind: ValueKind::U64 {
            min: crate::budget::MONTHLY_CAP_MIN,
            max: crate::budget::MONTHLY_CAP_MAX,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "POLL_TELEMETRY_FALLBACK_SECS",
        group: SettingGroup::Polling,
        label: "Repli télémétrie",
        kind: ValueKind::U64 {
            min: crate::budget::TELEMETRY_FALLBACK_MIN_SECS,
            max: crate::budget::TELEMETRY_FALLBACK_MAX_SECS,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "FLEET_DATA_DYNAMIC_BUDGET",
        group: SettingGroup::Fleet,
        label: "Budget journalier dynamique",
        kind: ValueKind::Bool,
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "FLEET_DATA_MONTH_SEED",
        group: SettingGroup::Fleet,
        label: "Amorce du mois",
        kind: ValueKind::U64 {
            min: 0,
            max: crate::budget::MONTH_SEED_MAX,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "FLEET_DATA_MONTH_SEED_MONTH",
        group: SettingGroup::Fleet,
        label: "Mois de l'amorce",
        kind: ValueKind::Text,
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "FLEET_BLOCKED_PROBE_SECS",
        group: SettingGroup::Fleet,
        label: "Sonde blocage Tesla",
        kind: ValueKind::U64 {
            min: crate::budget::BLOCKED_PROBE_MIN_SECS,
            max: crate::budget::BLOCKED_PROBE_MAX_SECS,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "NOMINATIM_ENABLED",
        group: SettingGroup::Nominatim,
        label: "Nominatim",
        kind: ValueKind::Bool,
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "NOMINATIM_URL",
        group: SettingGroup::Nominatim,
        label: "Nominatim URL",
        kind: ValueKind::Text,
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "NOMINATIM_MIN_INTERVAL_MS",
        group: SettingGroup::Nominatim,
        label: "Nominatim intervalle (ms)",
        kind: ValueKind::U64 {
            min: 100,
            max: 60_000,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "MQTT_ENABLED",
        group: SettingGroup::Mqtt,
        label: "MQTT",
        kind: ValueKind::Bool,
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "MQTT_HOST",
        group: SettingGroup::Mqtt,
        label: "MQTT hôte",
        kind: ValueKind::Text,
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "MQTT_PORT",
        group: SettingGroup::Mqtt,
        label: "MQTT port",
        kind: ValueKind::U64 {
            min: 1,
            max: 65_535,
        },
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "MQTT_TOPIC_PREFIX",
        group: SettingGroup::Mqtt,
        label: "MQTT préfixe",
        kind: ValueKind::Text,
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "TESLA_TIMEZONE",
        group: SettingGroup::Misc,
        label: "Fuseau",
        kind: ValueKind::Text,
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "ENERGY_PRICE_PER_KWH",
        group: SettingGroup::Misc,
        label: "Prix du kWh",
        kind: ValueKind::F64,
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "ADMIN_SEES_ALL_TRIPS",
        group: SettingGroup::Misc,
        label: "Admin voit tous les trajets",
        kind: ValueKind::Bool,
        mode: ReloadMode::Hot,
    },
    SettingSpec {
        key: "OWNER_STREAM_ENABLED",
        group: SettingGroup::Misc,
        label: "Stream Owner",
        kind: ValueKind::Bool,
        mode: ReloadMode::Hot,
    },
];

/// Keys accepted by [`set`]. Kept for call sites that iterate the whitelist.
pub(crate) const KEYS: &[&str] = &[
    "ENERGY_PRICE_PER_KWH",
    "TESLA_TIMEZONE",
    "POLL_DRIVE_FALLBACK_SECS",
    "POLL_CHARGE_SECS",
    "POLL_PARKED_SECS",
    "POLL_ASLEEP_SECS",
    "POLL_BACKOFF_SECS",
    "POLL_GPS_FALLBACK_SECS",
    "POLL_PARKED_LIVE_LIMIT",
    "DRIVE_END_GRACE_SECS",
    "DRIVE_PARK_GRACE_SECS",
    "DRIVE_P_CONFIRM_SECS",
    "DRIVE_STOP_IN_GEAR_SECS",
    "FLEET_DATA_DAILY_CAP",
    "FLEET_DATA_MONTHLY_CAP",
    "FLEET_DATA_DYNAMIC_BUDGET",
    "FLEET_DATA_MONTH_SEED",
    "FLEET_DATA_MONTH_SEED_MONTH",
    "FLEET_BLOCKED_PROBE_SECS",
    "POLL_TELEMETRY_FALLBACK_SECS",
    "NOMINATIM_ENABLED",
    "NOMINATIM_URL",
    "NOMINATIM_MIN_INTERVAL_MS",
    "MQTT_ENABLED",
    "MQTT_HOST",
    "MQTT_PORT",
    "MQTT_TOPIC_PREFIX",
    "ADMIN_SEES_ALL_TRIPS",
    "OWNER_STREAM_ENABLED",
];

pub(crate) fn spec(key: &str) -> Option<&'static SettingSpec> {
    REGISTRY.iter().find(|entry| entry.key == key)
}

pub(crate) fn allowed(key: &str) -> bool {
    KEYS.contains(&key)
}

/// Resolved hot values. Cheap to clone; published on a watch channel.
#[derive(Clone, Debug)]
pub(crate) struct RuntimeSnapshot {
    pub drive_fallback: Duration,
    pub charge: Duration,
    pub parked: Duration,
    pub asleep: Duration,
    pub backoff: Duration,
    pub gps_fallback: Duration,
    pub parked_live_limit: u32,
    pub drive_end_grace: Duration,
    pub drive_park_grace: Duration,
    pub drive_p_confirm: Duration,
    pub drive_stop_in_gear: Duration,
    pub daily_cap: u64,
    pub monthly_cap: u64,
    pub blocked_probe: Duration,
    pub telemetry_fallback: Duration,
    pub dynamic_budget: bool,
    pub month_seed: u64,
    pub month_seed_month: Option<chrono::NaiveDate>,
    pub nominatim_enabled: bool,
    pub nominatim_url: String,
    pub nominatim_min_interval: Duration,
    pub mqtt_enabled: bool,
    pub mqtt_host: String,
    pub mqtt_port: u16,
    pub mqtt_prefix: String,
    pub timezone: String,
    pub energy_price_per_kwh: Option<f64>,
    pub admin_sees_all_trips: bool,
    pub owner_stream_enabled: bool,
    /// Global DB overrides only (`settings` table). Vehicle rows stay on demand.
    pub db_overrides: HashMap<String, String>,
    pub generation: u64,
}

impl RuntimeSnapshot {
    fn from_config(config: &Config) -> Self {
        Self {
            drive_fallback: config.drive_fallback_poll,
            charge: config.charge_poll,
            parked: config.parked_poll,
            asleep: config.asleep_poll,
            backoff: config.backoff_poll,
            gps_fallback: config.gps_fallback_poll,
            parked_live_limit: config.parked_live_limit,
            drive_end_grace: config.drive_end_grace,
            drive_park_grace: config.drive_park_grace,
            drive_p_confirm: config.drive_p_confirm,
            drive_stop_in_gear: config.drive_stop_in_gear,
            daily_cap: config.fleet_daily_cap,
            monthly_cap: config.fleet_monthly_cap,
            blocked_probe: config.fleet_blocked_probe,
            telemetry_fallback: config.telemetry_fallback,
            dynamic_budget: config.fleet_dynamic_budget,
            month_seed: config.fleet_month_seed,
            month_seed_month: config.fleet_month_seed_month,
            nominatim_enabled: config.nominatim_enabled,
            nominatim_url: config.nominatim_url.clone(),
            nominatim_min_interval: config.nominatim_min_interval,
            mqtt_enabled: config.mqtt_enabled,
            mqtt_host: config.mqtt_host.clone(),
            mqtt_port: config.mqtt_port,
            mqtt_prefix: config.mqtt_prefix.clone(),
            timezone: config.timezone.clone(),
            energy_price_per_kwh: config.energy_price_per_kwh,
            admin_sees_all_trips: true,
            owner_stream_enabled: config.owner_stream_enabled,
            db_overrides: HashMap::new(),
            generation: 0,
        }
    }

    fn apply_override(&mut self, key: &str, value: &str) {
        match key {
            "POLL_DRIVE_FALLBACK_SECS" => {
                if let Some(secs) = parse_bounded_u64(
                    value,
                    crate::budget::DRIVE_FALLBACK_MIN_SECS,
                    crate::budget::DRIVE_FALLBACK_MAX_SECS,
                ) {
                    self.drive_fallback = Duration::from_secs(secs);
                }
            }
            "POLL_CHARGE_SECS" => {
                if let Some(secs) = parse_bounded_u64(
                    value,
                    crate::budget::CHARGE_POLL_MIN_SECS,
                    crate::budget::CHARGE_POLL_MAX_SECS,
                ) {
                    self.charge = Duration::from_secs(secs);
                }
            }
            "POLL_PARKED_SECS" => {
                if let Some(secs) = parse_bounded_u64(value, 60, 86_400) {
                    self.parked = Duration::from_secs(secs);
                }
            }
            "POLL_ASLEEP_SECS" => {
                if let Some(secs) = parse_bounded_u64(value, crate::budget::IDLE_MIN_SECS, 86_400) {
                    self.asleep = Duration::from_secs(secs);
                }
            }
            "POLL_BACKOFF_SECS" => {
                if let Some(secs) = parse_bounded_u64(value, crate::budget::IDLE_MIN_SECS, 86_400) {
                    self.backoff = Duration::from_secs(secs);
                }
            }
            "POLL_GPS_FALLBACK_SECS" => {
                if let Some(secs) = parse_bounded_u64(value, 60, 86_400) {
                    self.gps_fallback = Duration::from_secs(secs);
                }
            }
            "POLL_PARKED_LIVE_LIMIT" => {
                if let Some(limit) = parse_bounded_u64(value, 0, 100) {
                    self.parked_live_limit = u32::try_from(limit).unwrap_or(u32::MAX);
                }
            }
            "DRIVE_END_GRACE_SECS" => {
                if let Some(secs) = parse_bounded_u64(
                    value,
                    crate::state::DRIVE_END_GRACE_MIN_SECS,
                    crate::state::DRIVE_END_GRACE_MAX_SECS,
                ) {
                    self.drive_end_grace = Duration::from_secs(secs);
                }
            }
            "DRIVE_PARK_GRACE_SECS" => {
                if let Some(secs) = parse_bounded_u64(
                    value,
                    crate::state::DRIVE_PARK_GRACE_MIN_SECS,
                    crate::state::DRIVE_PARK_GRACE_MAX_SECS,
                ) {
                    self.drive_park_grace = Duration::from_secs(secs);
                }
            }
            "DRIVE_P_CONFIRM_SECS" => {
                if let Some(secs) = parse_bounded_u64(
                    value,
                    crate::state::DRIVE_P_CONFIRM_MIN_SECS,
                    crate::state::DRIVE_P_CONFIRM_MAX_SECS,
                ) {
                    self.drive_p_confirm = Duration::from_secs(secs);
                }
            }
            "DRIVE_STOP_IN_GEAR_SECS" => {
                if let Some(secs) = parse_bounded_u64(
                    value,
                    crate::state::DRIVE_STOP_IN_GEAR_MIN_SECS,
                    crate::state::DRIVE_STOP_IN_GEAR_MAX_SECS,
                ) {
                    self.drive_stop_in_gear = Duration::from_secs(secs);
                }
            }
            "FLEET_DATA_DAILY_CAP" => {
                if let Some(cap) = parse_bounded_u64(
                    value,
                    crate::budget::DAILY_CAP_MIN,
                    crate::budget::DAILY_CAP_MAX,
                ) {
                    self.daily_cap = cap;
                }
            }
            "FLEET_DATA_MONTHLY_CAP" => {
                if let Some(cap) = parse_bounded_u64(
                    value,
                    crate::budget::MONTHLY_CAP_MIN,
                    crate::budget::MONTHLY_CAP_MAX,
                ) {
                    self.monthly_cap = cap;
                }
            }
            "FLEET_BLOCKED_PROBE_SECS" => {
                if let Some(secs) = parse_bounded_u64(
                    value,
                    crate::budget::BLOCKED_PROBE_MIN_SECS,
                    crate::budget::BLOCKED_PROBE_MAX_SECS,
                ) {
                    self.blocked_probe = Duration::from_secs(secs);
                }
            }
            "POLL_TELEMETRY_FALLBACK_SECS" => {
                if let Some(secs) = parse_bounded_u64(
                    value,
                    crate::budget::TELEMETRY_FALLBACK_MIN_SECS,
                    crate::budget::TELEMETRY_FALLBACK_MAX_SECS,
                ) {
                    self.telemetry_fallback = Duration::from_secs(secs);
                }
            }
            "FLEET_DATA_DYNAMIC_BUDGET" => {
                if let Some(flag) = parse_bool(value) {
                    self.dynamic_budget = flag;
                }
            }
            "FLEET_DATA_MONTH_SEED" => {
                if let Some(seed) = parse_bounded_u64(value, 0, crate::budget::MONTH_SEED_MAX) {
                    self.month_seed = seed;
                }
            }
            "FLEET_DATA_MONTH_SEED_MONTH" => {
                self.month_seed_month = crate::budget::parse_seed_month(value);
            }
            "NOMINATIM_ENABLED" => {
                if let Some(flag) = parse_bool(value) {
                    self.nominatim_enabled = flag;
                }
            }
            "NOMINATIM_URL" => {
                if !value.is_empty() {
                    self.nominatim_url.clear();
                    self.nominatim_url.push_str(value);
                }
            }
            "NOMINATIM_MIN_INTERVAL_MS" => {
                if let Some(ms) = parse_bounded_u64(value, 100, 60_000) {
                    self.nominatim_min_interval = Duration::from_millis(ms);
                }
            }
            "MQTT_ENABLED" => {
                if let Some(flag) = parse_bool(value) {
                    self.mqtt_enabled = flag;
                }
            }
            "MQTT_HOST" => {
                if !value.is_empty() {
                    self.mqtt_host.clear();
                    self.mqtt_host.push_str(value);
                }
            }
            "MQTT_PORT" => {
                if let Some(port) = parse_bounded_u64(value, 1, 65_535) {
                    self.mqtt_port = u16::try_from(port).unwrap_or(1883);
                }
            }
            "MQTT_TOPIC_PREFIX" => {
                if !value.is_empty() {
                    self.mqtt_prefix.clear();
                    self.mqtt_prefix.push_str(value);
                }
            }
            "TESLA_TIMEZONE" => {
                if !value.is_empty() {
                    self.timezone.clear();
                    self.timezone.push_str(value);
                }
            }
            "ENERGY_PRICE_PER_KWH" => {
                self.energy_price_per_kwh = value.parse().ok();
            }
            "ADMIN_SEES_ALL_TRIPS" => {
                if let Some(flag) = parse_bool(value) {
                    self.admin_sees_all_trips = flag;
                }
            }
            "OWNER_STREAM_ENABLED" => {
                if let Some(flag) = parse_bool(value) {
                    self.owner_stream_enabled = flag;
                }
            }
            _ => {}
        }
        if self.monthly_cap < self.daily_cap {
            self.monthly_cap = self.daily_cap;
        }
        if self.drive_park_grace > self.drive_end_grace {
            self.drive_park_grace = self.drive_end_grace;
        }
    }
}

/// Process-wide hot configuration. Clones share the same watch sender.
#[derive(Clone)]
pub(crate) struct RuntimeConfig {
    pool: PgPool,
    boot: Arc<Config>,
    tx: watch::Sender<RuntimeSnapshot>,
}

impl RuntimeConfig {
    pub(crate) async fn load(pool: PgPool, boot: Arc<Config>) -> Result<Self, AppError> {
        let snapshot = resolve(&pool, &boot, 0).await?;
        let (tx, _) = watch::channel(snapshot);
        Ok(Self { pool, boot, tx })
    }

    /// Boot defaults only. Used by tests that do not open a real settings table.
    #[cfg(test)]
    pub(crate) fn from_boot(pool: PgPool, boot: Arc<Config>) -> Self {
        let (tx, _) = watch::channel(RuntimeSnapshot::from_config(&boot));
        Self { pool, boot, tx }
    }

    pub(crate) fn subscribe(&self) -> watch::Receiver<RuntimeSnapshot> {
        self.tx.subscribe()
    }

    pub(crate) fn snapshot(&self) -> RuntimeSnapshot {
        self.tx.borrow().clone()
    }

    pub(crate) async fn reload(&self) -> Result<(), AppError> {
        let generation = self.tx.borrow().generation.wrapping_add(1);
        let snapshot = resolve(&self.pool, &self.boot, generation).await?;
        let _ = self.tx.send(snapshot);
        Ok(())
    }

    pub(crate) fn boot(&self) -> &Config {
        &self.boot
    }
}

async fn resolve(
    pool: &PgPool,
    boot: &Config,
    generation: u64,
) -> Result<RuntimeSnapshot, AppError> {
    let mut snapshot = RuntimeSnapshot::from_config(boot);
    snapshot.generation = generation;
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await?;
    for (key, value) in rows {
        snapshot.db_overrides.insert(key.clone(), value.clone());
        snapshot.apply_override(&key, &value);
    }
    Ok(snapshot)
}

pub(crate) async fn get(
    pool: &PgPool,
    vehicle_id: Option<i64>,
    key: &str,
) -> Result<Option<(String, &'static str)>, AppError> {
    if let Some(vehicle_id) = vehicle_id {
        let value: Option<String> = sqlx::query_scalar(
            "SELECT value FROM vehicle_settings WHERE vehicle_id = $1 AND key = $2",
        )
        .bind(vehicle_id)
        .bind(key)
        .fetch_optional(pool)
        .await?;
        if let Some(value) = value {
            return Ok(Some((value, "véhicule")));
        }
    }
    let value: Option<String> = sqlx::query_scalar("SELECT value FROM settings WHERE key = $1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(value.map(|value| (value, "réglage")))
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) async fn set(
    pool: &PgPool,
    vehicle_id: Option<i64>,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    set_and_publish(pool, None, vehicle_id, key, value).await
}

pub(crate) async fn set_and_publish(
    pool: &PgPool,
    runtime: Option<&RuntimeConfig>,
    vehicle_id: Option<i64>,
    key: &str,
    value: &str,
) -> Result<(), AppError> {
    if !allowed(key) {
        return Err(AppError::message("this setting cannot be changed here"));
    }
    let value = value.trim();
    if value.is_empty() {
        clear(pool, vehicle_id, key).await?;
    } else {
        check_value(key, value)?;
        if let Some(vehicle_id) = vehicle_id {
            sqlx::query(
                "INSERT INTO vehicle_settings (vehicle_id, key, value) VALUES ($1, $2, $3)
                 ON CONFLICT (vehicle_id, key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
            )
            .bind(vehicle_id)
            .bind(key)
            .bind(value)
            .execute(pool)
            .await?;
        } else {
            sqlx::query(
                "INSERT INTO settings (key, value) VALUES ($1, $2)
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now()",
            )
            .bind(key)
            .bind(value)
            .execute(pool)
            .await?;
        }
    }
    if let Some(runtime) = runtime {
        runtime.reload().await?;
    }
    Ok(())
}

async fn clear(pool: &PgPool, vehicle_id: Option<i64>, key: &str) -> Result<(), AppError> {
    if let Some(vehicle_id) = vehicle_id {
        sqlx::query("DELETE FROM vehicle_settings WHERE vehicle_id = $1 AND key = $2")
            .bind(vehicle_id)
            .bind(key)
            .execute(pool)
            .await?;
    } else {
        sqlx::query("DELETE FROM settings WHERE key = $1")
            .bind(key)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub(crate) async fn f64_or(
    pool: &PgPool,
    vehicle_id: Option<i64>,
    key: &str,
    fallback: Option<f64>,
) -> Option<f64> {
    match get(pool, vehicle_id, key).await {
        Ok(Some((value, _))) => value.parse().ok().or(fallback),
        _ => fallback,
    }
}

pub(crate) async fn text_or(
    pool: &PgPool,
    vehicle_id: Option<i64>,
    key: &str,
    fallback: &str,
) -> String {
    match get(pool, vehicle_id, key).await {
        Ok(Some((value, _))) if !value.is_empty() => value,
        _ => fallback.to_owned(),
    }
}

pub(crate) async fn secs_or(pool: &PgPool, key: &str, fallback: Duration) -> Duration {
    bounded_secs(pool, key, fallback, 0, u64::MAX).await
}

pub(crate) async fn bounded_secs(
    pool: &PgPool,
    key: &str,
    fallback: Duration,
    min: u64,
    max: u64,
) -> Duration {
    match bounded_u64(pool, key, fallback.as_secs(), min, max).await {
        secs if secs == fallback.as_secs() => fallback,
        secs => Duration::from_secs(secs),
    }
}

pub(crate) async fn bounded_u64(
    pool: &PgPool,
    key: &str,
    fallback: u64,
    min: u64,
    max: u64,
) -> u64 {
    match get(pool, None, key).await {
        Ok(Some((value, _))) => value
            .parse::<u64>()
            .ok()
            .filter(|value| (min..=max).contains(value))
            .unwrap_or(fallback),
        _ => fallback,
    }
}

/// Effective value for the settings table: DB override, else env/default.
pub(crate) fn effective_row(
    boot: &Config,
    snapshot: &RuntimeSnapshot,
    key: &str,
    label: &'static str,
    display: String,
) -> EffectiveSetting {
    let (value, source) = if let Some(override_value) = snapshot.db_overrides.get(key) {
        let source = if boot.env_explicit(key) {
            "réglage · env masqué"
        } else {
            "réglage"
        };
        (override_value.clone(), source)
    } else {
        (display, boot.source_label(key))
    };
    EffectiveSetting {
        key: key.to_owned(),
        label,
        value,
        source,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct EffectiveSetting {
    #[allow(dead_code)]
    pub key: String,
    pub label: &'static str,
    pub value: String,
    pub source: &'static str,
}

fn check_value(key: &str, value: &str) -> Result<(), AppError> {
    let Some(entry) = spec(key) else {
        return Ok(());
    };
    match entry.kind {
        ValueKind::Bool => {
            if parse_bool(value).is_none() {
                return Err(AppError::message(format!("{key} must be true or false")));
            }
        }
        ValueKind::U64 { min, max } => {
            let parsed = value
                .parse::<u64>()
                .map_err(|_| AppError::message(format!("{key} must be an integer")))?;
            if parsed < min || parsed > max {
                return Err(AppError::message(format!(
                    "{key} must be between {min} and {max}"
                )));
            }
        }
        ValueKind::F64 => {
            if value
                .parse::<f64>()
                .ok()
                .as_ref()
                .is_none_or(|n| !n.is_finite())
            {
                return Err(AppError::message(format!("{key} must be a number")));
            }
        }
        ValueKind::Text => {
            if key == "NOMINATIM_URL" {
                let parsed = url::Url::parse(value)
                    .map_err(|_| AppError::message("NOMINATIM_URL is invalid"))?;
                if parsed.scheme() != "https" {
                    return Err(AppError::message("NOMINATIM_URL must be https"));
                }
            }
        }
    }
    Ok(())
}

fn parse_bounded_u64(value: &str, min: u64, max: u64) -> Option<u64> {
    value
        .parse::<u64>()
        .ok()
        .filter(|parsed| (min..=max).contains(parsed))
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{RuntimeSnapshot, parse_bool};
    use crate::config::Config;

    #[test]
    fn bool_parser_accepts_common_forms() {
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("OFF"), Some(false));
        assert_eq!(parse_bool("maybe"), None);
    }

    #[test]
    fn snapshot_applies_db_overrides_over_boot() {
        let boot = Config::from_map(&std::collections::BTreeMap::from([
            ("TESLA_CLIENT_ID".into(), "id".into()),
            ("TESLA_CLIENT_SECRET".into(), "secret".into()),
            (
                "TESLA_FLEET_AUDIENCE".into(),
                "https://fleet-api.prd.eu.vn.cloud.tesla.com".into(),
            ),
            ("TESLA_DOMAIN".into(), "logger.example.com".into()),
            (
                "DATABASE_URL".into(),
                "postgres://tesla:tesla@127.0.0.1/tesla_logger".into(),
            ),
            ("TESLA_PRIVATE_KEY_PATH".into(), "key.pem".into()),
            ("TESLA_TOKEN_STORE_PATH".into(), "tokens.json".into()),
            ("TESLA_LOGGER_UI_PASSWORD".into(), "password".into()),
            ("MQTT_ENABLED".into(), "false".into()),
            ("POLL_PARKED_LIVE_LIMIT".into(), "2".into()),
        ]))
        .unwrap();
        let mut snap = RuntimeSnapshot::from_config(&boot);
        assert!(!snap.mqtt_enabled);
        assert_eq!(snap.parked_live_limit, 2);
        snap.apply_override("MQTT_ENABLED", "true");
        snap.apply_override("POLL_PARKED_LIVE_LIMIT", "5");
        assert!(snap.mqtt_enabled);
        assert_eq!(snap.parked_live_limit, 5);
    }
}
