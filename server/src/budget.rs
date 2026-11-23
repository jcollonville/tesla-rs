// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Fleet API `vehicle_data` budget.
//!
//! Tesla bills that endpoint at $1 per 500 calls. HTTP 408 and other 4xx
//! responses are billed too. The daily and monthly caps stop further
//! `vehicle_data` calls. The vehicle list and the Owner stream are not capped.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Datelike, NaiveDate, TimeZone, Utc};
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
                "vehicle_data is paused until the 1st of the month (UTC): Tesla billing limit. The vehicle list and the Owner stream keep running."
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
                "vehicle_data est suspendu jusqu'au 1er du mois (UTC) : limite de facturation Tesla. La liste et le stream continuent."
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
    pub billing_until: Option<DateTime<Utc>>,
    pub pause: Option<PauseKind>,
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

    fn counters_down(daily_cap: i64, monthly_cap: i64) -> Self {
        Self {
            today_calls: 0,
            month_calls: 0,
            daily_cap,
            monthly_cap,
            billing_until: None,
            pause: Some(PauseKind::Counters),
        }
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

/// First instant of the next UTC month. A 402 pause lasts until then.
pub(crate) fn next_utc_month(now: DateTime<Utc>) -> DateTime<Utc> {
    let (year, month) = if now.month() == 12 {
        (now.year() + 1, 1)
    } else {
        (now.year(), now.month() + 1)
    };
    Utc.with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .unwrap_or(now)
}

pub(crate) fn billing_active(until: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    until.is_some_and(|at| at > now)
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
    billing_until: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
    counters_ok: bool,
) -> Option<PauseKind> {
    if !counters_ok {
        return Some(PauseKind::Counters);
    }
    if billing_active(billing_until, now) {
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
    let billing_until = match billing_until(pool).await {
        Ok(until) => until,
        Err(err) => {
            tracing::warn!(error = %err, "fleet billing pause could not be read");
            return Gate::counters_down(daily_cap, monthly_cap);
        }
    };
    let pause = pause_kind(
        today_calls,
        month_calls,
        daily_cap,
        monthly_cap,
        billing_until,
        now,
        true,
    );
    Gate {
        today_calls,
        month_calls,
        daily_cap,
        monthly_cap,
        billing_until,
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

pub(crate) async fn pause_billing(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<DateTime<Utc>, AppError> {
    let until = next_utc_month(now);
    sqlx::query(
        "INSERT INTO fleet_control (id, billing_paused_until)
         VALUES (1, $1)
         ON CONFLICT (id) DO UPDATE SET billing_paused_until = EXCLUDED.billing_paused_until",
    )
    .bind(until)
    .execute(pool)
    .await?;
    Ok(until)
}

pub(crate) async fn resume_billing(pool: &PgPool) -> Result<(), AppError> {
    sqlx::query("UPDATE fleet_control SET billing_paused_until = NULL WHERE id = 1")
        .execute(pool)
        .await?;
    Ok(())
}

async fn billing_until(pool: &PgPool) -> Result<Option<DateTime<Utc>>, AppError> {
    let until: Option<DateTime<Utc>> =
        sqlx::query_scalar("SELECT billing_paused_until FROM fleet_control WHERE id = 1")
            .fetch_optional(pool)
            .await?
            .flatten();
    Ok(until)
}

#[cfg(test)]
mod tests {
    use super::{
        Duration, PauseKind, StrikeBook, Utc, backoff_delay, billing_active, error_retry,
        estimated_cost_usd, next_utc_month, pause_kind,
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
        let november = at(2026, 11, 23, 12);
        assert_eq!(
            pause_kind(150, 150, 150, 4500, None, november, true),
            Some(PauseKind::DailyCap)
        );
        assert_eq!(
            pause_kind(10, 4500, 150, 4500, None, november, true),
            Some(PauseKind::MonthlyCap)
        );
        assert_eq!(pause_kind(149, 149, 150, 4500, None, november, true), None);
        assert_eq!(
            pause_kind(0, 0, 150, 4500, None, november, false),
            Some(PauseKind::Counters)
        );
    }

    #[test]
    fn billing_pause_lasts_until_the_first_of_the_next_utc_month() {
        let now = at(2026, 11, 23, 18);
        let until = next_utc_month(now);
        assert_eq!(until, at(2026, 12, 1, 0));
        assert!(billing_active(Some(until), at(2026, 11, 30, 23)));
        assert!(!billing_active(Some(until), until));
        assert_eq!(next_utc_month(at(2026, 12, 31, 22)), at(2027, 1, 1, 0));
        assert_eq!(
            pause_kind(0, 0, 150, 4500, Some(until), now, true),
            Some(PauseKind::Billing)
        );
        assert_eq!(pause_kind(0, 0, 150, 4500, Some(until), until, true), None);
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
        let until = super::pause_billing(&db.pool, now).await.unwrap();
        assert_eq!(until, at(2026, 12, 1, 0));
        let paused = super::gate(&db.pool, 150, 4500, now).await;
        assert_eq!(paused.pause, Some(PauseKind::Billing));
        assert!(paused.blocks_vehicle_data());
        super::resume_billing(&db.pool).await.unwrap();
        let resumed = super::gate(&db.pool, 150, 4500, now).await;
        assert_eq!(resumed.pause, None);
        assert!(!resumed.blocks_vehicle_data());
    }
}
