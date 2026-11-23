// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Fleet API `vehicle_data` budget.
//!
//! Tesla bills that endpoint at $1 per 500 calls. HTTP 408 and other 4xx
//! responses are billed too. The daily and monthly caps stop further
//! `vehicle_data` calls. The vehicle list and the Owner stream are not capped.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use sqlx::PgPool;

use crate::error::AppError;

pub(crate) const VEHICLE_DATA: &str = "vehicle_data";

/// Calls Tesla prices at $1. The monthly $10 credit is not subtracted here.
pub(crate) const CALLS_PER_DOLLAR: f64 = 500.0;

pub(crate) const DRIVE_FALLBACK_DEFAULT_SECS: u64 = 120;
pub(crate) const DRIVE_FALLBACK_MIN_SECS: u64 = 60;
pub(crate) const DRIVE_FALLBACK_MAX_SECS: u64 = 3_600;

pub(crate) const CHARGE_POLL_DEFAULT_SECS: u64 = 300;
pub(crate) const CHARGE_POLL_MIN_SECS: u64 = 60;
pub(crate) const CHARGE_POLL_MAX_SECS: u64 = 86_400;

pub(crate) const DAILY_CAP_DEFAULT: u64 = 150;
pub(crate) const DAILY_CAP_MIN: u64 = 0;
pub(crate) const DAILY_CAP_MAX: u64 = 100_000;

pub(crate) const MONTHLY_CAP_DEFAULT: u64 = 4_500;
pub(crate) const MONTHLY_CAP_MIN: u64 = 0;
pub(crate) const MONTHLY_CAP_MAX: u64 = 1_000_000;

pub(crate) const BLOCKED_PROBE_DEFAULT_SECS: u64 = 3_600;
pub(crate) const BLOCKED_PROBE_MIN_SECS: u64 = 300;
pub(crate) const BLOCKED_PROBE_MAX_SECS: u64 = 86_400;

const BACKOFF_FLOOR_SECS: u64 = 60;
const BACKOFF_CEILING_SECS: u64 = 3_600;

/// Why `vehicle_data` is not being called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PauseKind {
    DailyCap,
    MonthlyCap,
    Billing,
    Counters,
}

impl PauseKind {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::DailyCap => "daily_cap",
            Self::MonthlyCap => "monthly_cap",
            Self::Billing => "billing",
            Self::Counters => "counters",
        }
    }

    pub(crate) const fn debug_banner(self) -> &'static str {
        match self {
            Self::DailyCap => {
                "vehicle_data is paused: the daily Fleet API cap is reached. The vehicle list and the Owner stream keep running."
            }
            Self::MonthlyCap => {
                "vehicle_data is paused: the monthly Fleet API cap is reached. The vehicle list and the Owner stream keep running."
            }
            Self::Billing => {
                "vehicle_data is paused: Tesla billing limit. A periodic probe checks whether the block has lifted."
            }
            Self::Counters => "vehicle_data is paused: the usage counters could not be read.",
        }
    }

    pub(crate) const fn system_banner(self) -> &'static str {
        match self {
            Self::DailyCap => {
                "vehicle_data est suspendu : plafond journalier atteint. La liste des véhicules et le stream Owner continuent."
            }
            Self::MonthlyCap => {
                "vehicle_data est suspendu : plafond mensuel atteint. La liste des véhicules et le stream Owner continuent."
            }
            Self::Billing => {
                "vehicle_data est suspendu : limite de facturation Tesla. Un test périodique vérifie si le blocage est levé."
            }
            Self::Counters => "vehicle_data est suspendu : les compteurs d'usage sont illisibles.",
        }
    }
}

/// Counters and the pause decision for one instant.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Gate {
    pub today_calls: i64,
    pub month_calls: i64,
    pub daily_cap: i64,
    pub monthly_cap: i64,
    /// Set while Tesla is rejecting calls for the billing limit.
    pub block: Option<BillingBlock>,
    pub pause: Option<PauseKind>,
}

/// One billing block and the next single probe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BillingBlock {
    pub since: DateTime<Utc>,
    pub next_probe: DateTime<Utc>,
    /// The free vehicle list returned the block, so the probe uses that list.
    pub probe_list: bool,
}

impl Gate {
    pub(crate) fn blocks_vehicle_data(&self) -> bool {
        self.pause.is_some()
    }

    pub(crate) fn today_cost_usd(&self) -> f64 {
        estimated_cost_usd(self.today_calls)
    }

    pub(crate) fn month_cost_usd(&self) -> f64 {
        estimated_cost_usd(self.month_calls)
    }

    pub(crate) fn debug_line(&self) -> Option<&'static str> {
        self.pause.map(PauseKind::debug_banner)
    }

    /// Daily or monthly cap, or unreadable counters. A billing block alone does not.
    pub(crate) fn cap_reached(&self) -> bool {
        matches!(self.pause, Some(PauseKind::Counters))
            || self.today_calls >= self.daily_cap
            || self.month_calls >= self.monthly_cap
    }

    /// Wait before the next list call when the list itself is the probe.
    pub(crate) fn list_wait(&self, now: DateTime<Utc>) -> Option<Duration> {
        let block = self.block.filter(|block| block.probe_list)?;
        block.remaining(now)
    }

    fn counters_down(daily_cap: i64, monthly_cap: i64) -> Self {
        Self {
            today_calls: 0,
            month_calls: 0,
            daily_cap,
            monthly_cap,
            block: None,
            pause: Some(PauseKind::Counters),
        }
    }
}

impl BillingBlock {
    pub(crate) fn due(self, now: DateTime<Utc>) -> bool {
        now >= self.next_probe
    }

    pub(crate) fn remaining(self, now: DateTime<Utc>) -> Option<Duration> {
        if self.due(now) {
            return None;
        }
        (self.next_probe - now)
            .to_std()
            .ok()
            .filter(|wait| !wait.is_zero())
    }
}

/// Consecutive HTTP 429s. A successful `vehicle_data` resets the count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct StrikeBook {
    pub consecutive: u32,
}

impl StrikeBook {
    pub(crate) fn on_success(&mut self) {
        self.consecutive = 0;
    }

    /// Honour `Retry-After` when it is longer than the exponential delay.
    pub(crate) fn on_limited(
        &mut self,
        retry_after: Option<Duration>,
        jitter_secs: u64,
    ) -> Duration {
        self.consecutive = self.consecutive.saturating_add(1);
        backoff_delay(self.consecutive, retry_after, jitter_secs)
    }

    pub(crate) fn on_limited_now(&mut self, retry_after: Option<Duration>) -> Duration {
        let next = self.consecutive.saturating_add(1);
        self.on_limited(retry_after, live_jitter(backoff_base(next)))
    }
}

pub(crate) fn as_display_f64(value: i64) -> f64 {
    f64::from(u32::try_from(value.max(0)).unwrap_or(u32::MAX))
}

pub(crate) fn estimated_cost_usd(calls: i64) -> f64 {
    as_display_f64(calls) / CALLS_PER_DOLLAR
}

pub(crate) fn backoff_base(consecutive: u32) -> u64 {
    let shift = consecutive.saturating_sub(1).min(6);
    BACKOFF_FLOOR_SECS
        .saturating_mul(1_u64 << shift)
        .min(BACKOFF_CEILING_SECS)
}

/// Exponential delay from 60 s to 3600 s, plus jitter, and at least `Retry-After`.
pub(crate) fn backoff_delay(
    consecutive: u32,
    retry_after: Option<Duration>,
    jitter_secs: u64,
) -> Duration {
    let base = backoff_base(consecutive.max(1));
    let jitter = jitter_secs.min(base / 5);
    let exponential = Duration::from_secs(base.saturating_add(jitter).min(BACKOFF_CEILING_SECS));
    match retry_after {
        Some(advertised) => exponential.max(advertised),
        None => exponential,
    }
}

/// Up to 20% of `base`, from the clock. Tests pass an explicit jitter instead.
pub(crate) fn live_jitter(base_secs: u64) -> u64 {
    let span = (base_secs / 5).max(1);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| u64::from(duration.subsec_nanos()));
    nanos % span
}

/// Wait after a failed ingest cycle. Never shorter than the 429 floor.
///
/// A reached cap does not schedule `vehicle_data`; the next cycle still lists
/// vehicles. An active backoff is kept when it is longer than the floor.
pub(crate) fn error_retry(active_backoff: Option<Duration>, budget_blocked: bool) -> Duration {
    let floor = Duration::from_secs(BACKOFF_FLOOR_SECS);
    let wait = active_backoff.unwrap_or(Duration::ZERO).max(floor);
    // The cap blocks `vehicle_data` inside the cycle. It must not shorten this wait.
    if budget_blocked && wait < floor {
        floor
    } else {
        wait
    }
}

pub(crate) fn pause_kind(
    today: i64,
    month: i64,
    daily_cap: i64,
    monthly_cap: i64,
    billing_blocked: bool,
    counters_ok: bool,
) -> Option<PauseKind> {
    if !counters_ok {
        return Some(PauseKind::Counters);
    }
    if billing_blocked {
        return Some(PauseKind::Billing);
    }
    if month >= monthly_cap {
        return Some(PauseKind::MonthlyCap);
    }
    if today >= daily_cap {
        return Some(PauseKind::DailyCap);
    }
    None
}

pub(crate) async fn record(
    pool: &PgPool,
    endpoint: &str,
    status: i32,
    at: DateTime<Utc>,
) -> Result<(), AppError> {
    let day = at.date_naive();
    sqlx::query(
        "INSERT INTO fleet_usage (day, endpoint, status, count)
         VALUES ($1, $2, $3, 1)
         ON CONFLICT (day, endpoint, status)
         DO UPDATE SET count = fleet_usage.count + 1",
    )
    .bind(day)
    .bind(endpoint)
    .bind(status)
    .execute(pool)
    .await?;
    Ok(())
}

pub(crate) async fn gate(
    pool: &PgPool,
    daily_cap: u64,
    monthly_cap: u64,
    now: DateTime<Utc>,
) -> Gate {
    let daily_cap = i64::try_from(daily_cap).unwrap_or(i64::MAX);
    let monthly_cap = i64::try_from(monthly_cap).unwrap_or(i64::MAX);
    let today = now.date_naive();
    let (start, end) = utc_month(today);
    let today_calls = match sum_between(pool, today, day_after(today)).await {
        Ok(count) => count,
        Err(err) => {
            tracing::warn!(error = %err, "fleet usage counters are unavailable");
            return Gate::counters_down(daily_cap, monthly_cap);
        }
    };
    let month_calls = match sum_between(pool, start, end).await {
        Ok(count) => count,
        Err(err) => {
            tracing::warn!(error = %err, "fleet usage counters are unavailable");
            return Gate::counters_down(daily_cap, monthly_cap);
        }
    };
    let control = match control_deadlines(pool).await {
        Ok(control) => control,
        Err(err) => {
            tracing::warn!(error = %err, "fleet billing pause could not be read");
            return Gate::counters_down(daily_cap, monthly_cap);
        }
    };
    let block = match (control.since, control.next_probe) {
        (Some(since), Some(next_probe)) => Some(BillingBlock {
            since,
            next_probe,
            probe_list: control.probe_list,
        }),
        _ => None,
    };
    let pause = pause_kind(
        today_calls,
        month_calls,
        daily_cap,
        monthly_cap,
        block.is_some(),
        true,
    );
    Gate {
        today_calls,
        month_calls,
        daily_cap,
        monthly_cap,
        block,
        pause,
    }
}

async fn sum_between(pool: &PgPool, start: NaiveDate, end: NaiveDate) -> Result<i64, AppError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COALESCE(SUM(count), 0)::bigint FROM fleet_usage
         WHERE endpoint = $1 AND day >= $2 AND day < $3",
    )
    .bind(VEHICLE_DATA)
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

fn day_after(day: NaiveDate) -> NaiveDate {
    day.checked_add_signed(chrono::Duration::days(1))
        .unwrap_or(day)
}

fn utc_month(day: NaiveDate) -> (NaiveDate, NaiveDate) {
    let start = NaiveDate::from_ymd_opt(day.year(), day.month(), 1).unwrap_or(day);
    let end = if day.month() == 12 {
        NaiveDate::from_ymd_opt(day.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(day.year(), day.month() + 1, 1)
    }
    .unwrap_or(start);
    (start, end)
}

pub(crate) fn probe_deadline(now: DateTime<Utc>, interval: Duration) -> DateTime<Utc> {
    let secs = i64::try_from(interval.as_secs()).unwrap_or(3_600);
    now + chrono::Duration::seconds(secs.max(1))
}

/// Enter or refresh the billing block. The original `blocked_since` is kept.
/// A list failure upgrades the probe to the free vehicle list.
pub(crate) async fn enter_block(
    pool: &PgPool,
    now: DateTime<Utc>,
    interval: Duration,
    probe_list: bool,
) -> Result<DateTime<Utc>, AppError> {
    let next = probe_deadline(now, interval);
    sqlx::query(
        "INSERT INTO fleet_control (id, blocked_since, next_probe_at, probe_list)
         VALUES (1, $1, $2, $3)
         ON CONFLICT (id) DO UPDATE SET
            blocked_since = COALESCE(fleet_control.blocked_since, EXCLUDED.blocked_since),
            next_probe_at = EXCLUDED.next_probe_at,
            probe_list = fleet_control.probe_list OR EXCLUDED.probe_list",
    )
    .bind(now)
    .bind(next)
    .bind(probe_list)
    .execute(pool)
    .await?;
    Ok(next)
}

pub(crate) async fn reschedule_probe(
    pool: &PgPool,
    now: DateTime<Utc>,
    interval: Duration,
) -> Result<DateTime<Utc>, AppError> {
    let next = probe_deadline(now, interval);
    sqlx::query(
        "UPDATE fleet_control
         SET next_probe_at = $1
         WHERE id = 1 AND blocked_since IS NOT NULL",
    )
    .bind(next)
    .execute(pool)
    .await?;
    Ok(next)
}

pub(crate) async fn resume_billing(pool: &PgPool) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE fleet_control
         SET billing_paused_until = NULL,
             list_paused_until = NULL,
             blocked_since = NULL,
             next_probe_at = NULL,
             probe_list = FALSE
         WHERE id = 1",
    )
    .execute(pool)
    .await?;
    Ok(())
}

struct ControlDeadlines {
    since: Option<DateTime<Utc>>,
    next_probe: Option<DateTime<Utc>>,
    probe_list: bool,
}

#[derive(sqlx::FromRow)]
struct ControlRow {
    blocked_since: Option<DateTime<Utc>>,
    next_probe_at: Option<DateTime<Utc>>,
    probe_list: bool,
}

async fn control_deadlines(pool: &PgPool) -> Result<ControlDeadlines, AppError> {
    let row = sqlx::query_as::<_, ControlRow>(
        "SELECT blocked_since, next_probe_at, probe_list FROM fleet_control WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(match row {
        Some(row) => ControlDeadlines {
            since: row.blocked_since,
            next_probe: row.next_probe_at,
            probe_list: row.probe_list,
        },
        None => ControlDeadlines {
            since: None,
            next_probe: None,
            probe_list: false,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{
        Duration, PauseKind, StrikeBook, Utc, backoff_delay, error_retry, estimated_cost_usd,
        pause_kind, probe_deadline,
    };
    use chrono::TimeZone;

    fn at(year: i32, month: u32, day: u32, hour: u32) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0)
            .single()
            .expect("date")
    }

    #[test]
    fn cost_is_one_dollar_per_five_hundred_calls() {
        assert!((estimated_cost_usd(0) - 0.0).abs() < f64::EPSILON);
        assert!((estimated_cost_usd(500) - 1.0).abs() < 1e-9);
        assert!((estimated_cost_usd(150) - 0.3).abs() < 1e-9);
    }

    #[test]
    fn cap_blocks_and_a_new_month_does_not_keep_the_old_count() {
        assert_eq!(
            pause_kind(150, 150, 150, 4500, false, true),
            Some(PauseKind::DailyCap)
        );
        assert_eq!(
            pause_kind(10, 4500, 150, 4500, false, true),
            Some(PauseKind::MonthlyCap)
        );
        assert_eq!(pause_kind(149, 149, 150, 4500, false, true), None);
        assert_eq!(
            pause_kind(0, 0, 150, 4500, false, false),
            Some(PauseKind::Counters)
        );
    }

    #[test]
    fn probe_deadline_uses_the_interval_and_a_block_stays_until_cleared() {
        let now = at(2026, 11, 23, 18);
        let next = probe_deadline(now, Duration::from_secs(3_600));
        assert_eq!(next, at(2026, 11, 23, 19));
        assert_eq!(
            pause_kind(0, 0, 150, 4500, true, true),
            Some(PauseKind::Billing)
        );
        assert_eq!(pause_kind(0, 0, 150, 4500, false, true), None);
    }

    #[test]
    fn rate_limit_backoff_honours_retry_after_then_doubles_to_an_hour() {
        assert_eq!(backoff_delay(1, None, 0), Duration::from_secs(60));
        assert_eq!(
            backoff_delay(1, Some(Duration::from_secs(10)), 0),
            Duration::from_secs(60)
        );
        assert_eq!(
            backoff_delay(1, Some(Duration::from_secs(90)), 0),
            Duration::from_secs(90)
        );
        assert_eq!(
            backoff_delay(1, Some(Duration::from_secs(7_200)), 0),
            Duration::from_secs(7_200)
        );
        assert_eq!(backoff_delay(2, None, 0), Duration::from_secs(120));
        assert_eq!(backoff_delay(3, None, 0), Duration::from_secs(240));
        assert_eq!(backoff_delay(7, None, 0), Duration::from_secs(3_600));
        assert_eq!(backoff_delay(20, None, 10_000), Duration::from_secs(3_600));

        let mut book = StrikeBook::default();
        assert_eq!(book.on_limited(None, 0), Duration::from_secs(60));
        assert_eq!(
            book.on_limited(Some(Duration::from_secs(30)), 0),
            Duration::from_secs(120)
        );
        book.on_success();
        assert_eq!(book.consecutive, 0);
        assert_eq!(book.on_limited(None, 0), Duration::from_secs(60));
    }

    #[test]
    fn error_retry_keeps_the_backoff_and_the_floor() {
        assert_eq!(error_retry(None, false), Duration::from_secs(60));
        assert_eq!(
            error_retry(Some(Duration::from_secs(30)), true),
            Duration::from_secs(60)
        );
        assert_eq!(
            error_retry(Some(Duration::from_secs(900)), true),
            Duration::from_secs(900)
        );
        assert_eq!(error_retry(None, true), Duration::from_secs(60));
    }

    #[tokio::test]
    async fn stored_usage_counts_408_blocks_at_the_cap_and_resets_on_the_month() {
        let Some(db) = crate::test_util::TestDb::migrated("fleetusage").await else {
            return;
        };
        let october = at(2026, 10, 31, 22);
        let november = at(2026, 11, 1, 0);
        super::record(&db.pool, super::VEHICLE_DATA, 408, october)
            .await
            .unwrap();
        super::record(&db.pool, super::VEHICLE_DATA, 408, october)
            .await
            .unwrap();
        super::record(&db.pool, super::VEHICLE_DATA, 408, november)
            .await
            .unwrap();
        super::record(&db.pool, super::VEHICLE_DATA, 200, november)
            .await
            .unwrap();

        let open = super::gate(&db.pool, 150, 4500, november).await;
        assert_eq!(open.today_calls, 2, "408 is counted with 200");
        assert_eq!(open.month_calls, 2, "October does not carry into November");
        assert_eq!(open.pause, None);

        let previous = super::gate(&db.pool, 150, 4500, october).await;
        assert_eq!(previous.today_calls, 2);
        assert_eq!(previous.month_calls, 2);

        sqlx::query("UPDATE fleet_usage SET count = 150 WHERE day = $1 AND status = 408")
            .bind(november.date_naive())
            .execute(&db.pool)
            .await
            .unwrap();
        let blocked = super::gate(&db.pool, 150, 4500, november).await;
        assert_eq!(blocked.today_calls, 151);
        assert_eq!(blocked.pause, Some(PauseKind::DailyCap));
        assert!(blocked.blocks_vehicle_data());

        let december = at(2026, 12, 1, 0);
        let rolled = super::gate(&db.pool, 150, 4500, december).await;
        assert_eq!(rolled.today_calls, 0);
        assert_eq!(rolled.month_calls, 0);
        assert_eq!(rolled.pause, None);
    }

    #[tokio::test]
    async fn billing_pause_blocks_until_resume() {
        let Some(db) = crate::test_util::TestDb::migrated("fleetbill").await else {
            return;
        };
        let now = at(2026, 11, 23, 8);
        let next = super::enter_block(&db.pool, now, Duration::from_secs(3_600), true)
            .await
            .unwrap();
        assert_eq!(next, at(2026, 11, 23, 9));
        let paused = super::gate(&db.pool, 150, 4500, now).await;
        assert_eq!(paused.pause, Some(PauseKind::Billing));
        assert!(paused.blocks_vehicle_data());
        let block = paused.block.expect("blocked");
        assert!(block.probe_list);
        assert_eq!(block.since, now);
        assert!(paused.list_wait(now).is_some());
        super::resume_billing(&db.pool).await.unwrap();
        let resumed = super::gate(&db.pool, 150, 4500, now).await;
        assert_eq!(resumed.pause, None);
        assert!(!resumed.blocks_vehicle_data());
    }
}
