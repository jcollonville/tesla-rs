// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Drive GPS points written by the Fleet poller.
//!
//! Fleet Telemetry is the drive track. A live `vehicle_data` poll during a
//! drive (telemetry silent or not configured) writes at most one `source=poll`
//! row per fallback interval. Parked and charging polls always keep their row.
//! Nothing here calls `wake_up`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Notify};

use crate::state::{Activity, Presence};

/// Whether this live poll should insert a `source=poll` positions row.
///
/// A drive writes at most one row per `interval`, and writes immediately when
/// none has been stored.
pub(crate) fn store_poll_position(
    driving: bool,
    last_drive_position: Option<Instant>,
    now: Instant,
    interval: Duration,
) -> bool {
    !driving || last_drive_position.is_none_or(|at| now.saturating_duration_since(at) >= interval)
}

/// A backup `vehicle_data` read when the normal plan would not poll.
///
/// Asleep and offline cars stay on the normal plan, which does not call
/// `vehicle_data`. Grace and Hold (`movement_recent` false) stay there too, and
/// fresh telemetry needs no backup read.
pub(crate) fn fallback_observe(
    presence: Presence,
    telemetry_fresh: bool,
    normal_observe: bool,
    movement_recent: bool,
) -> bool {
    presence == Presence::Online && !telemetry_fresh && !normal_observe && movement_recent
}

/// This poll sample is a drive point. Gear wins; a moving car with no gear counts too.
pub(crate) fn sample_is_driving(
    activity: Activity,
    shift: Option<&str>,
    speed_kmh: Option<f64>,
) -> bool {
    if activity == Activity::Driving {
        return true;
    }
    match shift.map(str::trim) {
        Some("D" | "d" | "R" | "r" | "N" | "n") => true,
        _ => speed_kmh.is_some_and(|speed| speed.is_finite() && speed > 1.0),
    }
}

/// Last polled drive point per vehicle, and the poller's wake-up.
#[derive(Clone)]
pub(crate) struct GpsBoard {
    last_drive_position: Arc<Mutex<HashMap<i64, Instant>>>,
    wake: Arc<Notify>,
}

impl Default for GpsBoard {
    fn default() -> Self {
        Self::new(Arc::new(Notify::new()))
    }
}

impl GpsBoard {
    pub(crate) fn new(wake: Arc<Notify>) -> Self {
        Self {
            last_drive_position: Arc::new(Mutex::new(HashMap::new())),
            wake,
        }
    }

    pub(crate) fn notified(&self) -> tokio::sync::futures::Notified<'_> {
        self.wake.notified()
    }

    pub(crate) fn notify(&self) {
        self.wake.notify_one();
    }

    pub(crate) async fn allow_poll_position(
        &self,
        vehicle_id: i64,
        driving: bool,
        now: Instant,
        interval: Duration,
    ) -> bool {
        let last = self
            .last_drive_position
            .lock()
            .await
            .get(&vehicle_id)
            .copied();
        store_poll_position(driving, last, now, interval)
    }

    pub(crate) async fn mark_drive_position(&self, vehicle_id: i64, now: Instant) {
        self.last_drive_position
            .lock()
            .await
            .insert(vehicle_id, now);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Activity, Duration, GpsBoard, Instant, Presence, fallback_observe, sample_is_driving,
        store_poll_position,
    };

    #[test]
    fn drive_positions_follow_the_interval() {
        let now = Instant::now();
        let interval = Duration::from_secs(300);
        let ago = |secs| now.checked_sub(Duration::from_secs(secs));
        assert!(store_poll_position(true, None, now, interval));
        assert!(!store_poll_position(true, ago(30), now, interval));
        assert!(store_poll_position(true, ago(300), now, interval));
        assert!(store_poll_position(false, ago(1), now, interval));
    }

    #[test]
    fn fallback_poll_does_not_wake_a_sleeping_car() {
        assert!(!fallback_observe(Presence::Asleep, false, false, true));
        assert!(!fallback_observe(Presence::Offline, false, false, true));
        assert!(fallback_observe(Presence::Online, false, false, true));
        assert!(!fallback_observe(Presence::Online, false, false, false));
        assert!(!fallback_observe(Presence::Online, true, false, true));
        assert!(!fallback_observe(Presence::Online, false, true, true));
    }

    #[test]
    fn gear_or_speed_marks_a_drive_sample() {
        assert!(sample_is_driving(Activity::Online, Some("D"), Some(0.0)));
        assert!(sample_is_driving(Activity::Driving, Some("P"), Some(0.0)));
        assert!(sample_is_driving(Activity::Online, None, Some(12.0)));
        assert!(!sample_is_driving(Activity::Online, Some("P"), Some(0.0)));
    }

    #[tokio::test]
    async fn board_spaces_polled_drive_points_per_vehicle() {
        let board = GpsBoard::default();
        let now = Instant::now();
        let interval = Duration::from_secs(300);
        assert!(board.allow_poll_position(7, true, now, interval).await);
        board.mark_drive_position(7, now).await;
        assert!(!board.allow_poll_position(7, true, now, interval).await);
        assert!(board.allow_poll_position(8, true, now, interval).await);
        assert!(board.allow_poll_position(7, false, now, interval).await);
    }
}
