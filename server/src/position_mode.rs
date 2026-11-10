// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Who writes drive GPS points.
//!
//! `hybrid` uses the Owner stream for drive positions and `vehicle_data` for
//! everything the stream does not carry. `fallback` writes `source=poll`
//! positions again because the socket is down, in error, or connected without
//! a `data:update` while the car is driving. Neither mode calls `wake_up`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Notify};

use crate::debug::DebugBoard;
use crate::state::{Activity, Presence};

/// Connected with no `data:update` for this long counts as unhealthy while driving.
///
/// Hello and ping frames keep the socket open, so the read timeout does not
/// notice a silent drive. Ninety seconds is long enough to ignore a gap and
/// shorter than the stream read timeout.
pub(crate) const STREAM_UPDATE_STALE: Duration = Duration::from_secs(90);

/// How drive positions are recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PositionMode {
    /// Stream samples are the drive track. Polls still refresh the other fields.
    Hybrid,
    /// `vehicle_data` writes drive positions on the fallback interval.
    Fallback,
}

impl PositionMode {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Hybrid => "hybrid",
            Self::Fallback => "fallback",
        }
    }
}

/// Owner socket, as far as GPS is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StreamLink {
    /// Subscribe succeeded. `last_update` is the last `data:update`, if any.
    Connected { last_update: Option<Instant> },
    /// Disconnected, error, disabled, or not started.
    Down,
}

/// Result of recomputing the mode. `became_fallback` is a transition into fallback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ModeChange {
    pub mode: PositionMode,
    pub changed: bool,
    pub became_fallback: bool,
}

pub(crate) fn position_mode(
    link: StreamLink,
    activity: Activity,
    now: Instant,
    silence: Duration,
) -> PositionMode {
    match link {
        StreamLink::Down => PositionMode::Fallback,
        StreamLink::Connected { last_update } => {
            if activity != Activity::Driving {
                return PositionMode::Hybrid;
            }
            match last_update {
                Some(at) if now.saturating_duration_since(at) <= silence => PositionMode::Hybrid,
                _ => PositionMode::Fallback,
            }
        }
    }
}

/// Whether this live poll should insert a `source=poll` positions row.
///
/// Parked and charging polls keep their existing rows. A drive in hybrid mode
/// leaves the track to the stream. A drive in fallback mode writes at most one
/// row per fallback interval, and writes immediately when none has been stored.
pub(crate) fn store_poll_position(
    mode: PositionMode,
    driving: bool,
    last_drive_position: Option<Instant>,
    now: Instant,
    interval: Duration,
) -> bool {
    if !driving {
        return true;
    }
    match mode {
        PositionMode::Hybrid => false,
        PositionMode::Fallback => {
            last_drive_position.is_none_or(|at| now.saturating_duration_since(at) >= interval)
        }
    }
}

/// A backup `vehicle_data` read when the normal plan would not poll.
///
/// Asleep and offline cars stay on the normal plan, which does not call
/// `vehicle_data`. Fleet telemetry freshness is unchanged.
pub(crate) fn fallback_observe(
    mode: PositionMode,
    presence: Presence,
    telemetry_fresh: bool,
    normal_observe: bool,
) -> bool {
    mode == PositionMode::Fallback
        && presence == Presence::Online
        && !telemetry_fresh
        && !normal_observe
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

struct Slot {
    up: bool,
    last_update: Option<Instant>,
    activity: Activity,
    mode: PositionMode,
    last_drive_position: Option<Instant>,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            up: false,
            last_update: None,
            activity: Activity::Offline,
            mode: PositionMode::Fallback,
            last_drive_position: None,
        }
    }
}

impl Slot {
    fn link(&self) -> StreamLink {
        if self.up {
            StreamLink::Connected {
                last_update: self.last_update,
            }
        } else {
            StreamLink::Down
        }
    }
}

/// Shared GPS mode. The debug console and the state API read it from here.
#[derive(Clone)]
pub(crate) struct GpsBoard {
    inner: Arc<Mutex<HashMap<i64, Slot>>>,
    debug: DebugBoard,
    wake: Arc<Notify>,
}

impl Default for GpsBoard {
    fn default() -> Self {
        Self::new(
            DebugBoard::new(crate::logs::LogBuffer::new(4)),
            Arc::new(Notify::new()),
        )
    }
}

impl GpsBoard {
    pub(crate) fn new(debug: DebugBoard, wake: Arc<Notify>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            debug,
            wake,
        }
    }

    pub(crate) fn notified(&self) -> tokio::sync::futures::Notified<'_> {
        self.wake.notified()
    }

    pub(crate) fn notify(&self) {
        self.wake.notify_one();
    }

    pub(crate) async fn mode_name(&self, vehicle_id: i64) -> String {
        self.mode(vehicle_id)
            .await
            .map_or("fallback", PositionMode::as_str)
            .to_owned()
    }

    async fn mode(&self, vehicle_id: i64) -> Option<PositionMode> {
        self.inner
            .lock()
            .await
            .get(&vehicle_id)
            .map(|slot| slot.mode)
    }

    pub(crate) async fn mark_up(&self, vehicle_id: i64) -> ModeChange {
        self.recompute(vehicle_id, |slot| slot.up = true).await
    }

    pub(crate) async fn mark_update(&self, vehicle_id: i64) -> ModeChange {
        self.recompute(vehicle_id, |slot| {
            slot.up = true;
            slot.last_update = Some(Instant::now());
        })
        .await
    }

    pub(crate) async fn mark_down(&self, vehicle_id: i64) -> ModeChange {
        self.recompute(vehicle_id, |slot| slot.up = false).await
    }

    /// Remember the latest list presence activity and recompute.
    ///
    /// `presence` is accepted so a sleeping car can be recorded, but silence is
    /// decided from activity: only a drive expects `data:update` frames.
    pub(crate) async fn reconcile(
        &self,
        vehicle_id: i64,
        presence: Presence,
        activity: Activity,
    ) -> ModeChange {
        let activity = if presence == Presence::Asleep {
            Activity::Asleep
        } else if presence == Presence::Offline && activity != Activity::Driving {
            Activity::Offline
        } else {
            activity
        };
        self.recompute(vehicle_id, |slot| slot.activity = activity)
            .await
    }

    pub(crate) async fn allow_poll_position(
        &self,
        vehicle_id: i64,
        driving: bool,
        now: Instant,
        interval: Duration,
    ) -> bool {
        let guard = self.inner.lock().await;
        let Some(slot) = guard.get(&vehicle_id) else {
            return store_poll_position(PositionMode::Fallback, driving, None, now, interval);
        };
        store_poll_position(slot.mode, driving, slot.last_drive_position, now, interval)
    }

    pub(crate) async fn mark_drive_position(&self, vehicle_id: i64, now: Instant) {
        if let Some(slot) = self.inner.lock().await.get_mut(&vehicle_id) {
            slot.last_drive_position = Some(now);
        }
    }

    async fn recompute(&self, vehicle_id: i64, edit: impl FnOnce(&mut Slot)) -> ModeChange {
        let mut guard = self.inner.lock().await;
        let slot = guard.entry(vehicle_id).or_default();
        edit(slot);
        let next = position_mode(
            slot.link(),
            slot.activity,
            Instant::now(),
            STREAM_UPDATE_STALE,
        );
        let previous = slot.mode;
        if previous != next {
            slot.mode = next;
            // vehicle_id is the database id. The VIN is not a log field.
            tracing::info!(vehicle_id, mode = next.as_str(), "position mode");
        }
        let mode = slot.mode;
        drop(guard);
        self.debug.set_position_mode(vehicle_id, mode.as_str());
        ModeChange {
            mode,
            changed: previous != next,
            became_fallback: previous != PositionMode::Fallback && mode == PositionMode::Fallback,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Activity, Duration, Instant, PositionMode, Presence, STREAM_UPDATE_STALE, StreamLink,
        fallback_observe, position_mode, sample_is_driving, store_poll_position,
    };
    use crate::debug::DebugBoard;
    use crate::logs::LogBuffer;
    use crate::position_mode::GpsBoard;
    use std::sync::Arc;
    use tokio::sync::Notify;

    fn now() -> Instant {
        Instant::now()
    }

    #[test]
    fn fresh_drive_updates_stay_hybrid() {
        let now = now();
        let link = StreamLink::Connected {
            last_update: Some(now.checked_sub(Duration::from_secs(2)).unwrap()),
        };
        assert_eq!(
            position_mode(link, Activity::Driving, now, STREAM_UPDATE_STALE),
            PositionMode::Hybrid
        );
    }

    #[test]
    fn silence_while_driving_falls_back() {
        let now = now();
        let stale = now
            .checked_sub(STREAM_UPDATE_STALE + Duration::from_secs(1))
            .unwrap();
        let link = StreamLink::Connected {
            last_update: Some(stale),
        };
        assert_eq!(
            position_mode(link, Activity::Driving, now, STREAM_UPDATE_STALE),
            PositionMode::Fallback
        );
        let silent = StreamLink::Connected { last_update: None };
        assert_eq!(
            position_mode(silent, Activity::Driving, now, STREAM_UPDATE_STALE),
            PositionMode::Fallback
        );
    }

    #[test]
    fn silence_while_parked_stays_hybrid() {
        let now = now();
        let link = StreamLink::Connected { last_update: None };
        assert_eq!(
            position_mode(link, Activity::Online, now, STREAM_UPDATE_STALE),
            PositionMode::Hybrid
        );
        assert_eq!(
            position_mode(link, Activity::Charging, now, STREAM_UPDATE_STALE),
            PositionMode::Hybrid
        );
    }

    #[test]
    fn disconnect_is_fallback_and_a_new_update_returns_to_hybrid() {
        let now = now();
        assert_eq!(
            position_mode(
                StreamLink::Down,
                Activity::Driving,
                now,
                STREAM_UPDATE_STALE
            ),
            PositionMode::Fallback
        );
        assert_eq!(
            position_mode(StreamLink::Down, Activity::Asleep, now, STREAM_UPDATE_STALE),
            PositionMode::Fallback
        );
        let link = StreamLink::Connected {
            last_update: Some(now),
        };
        assert_eq!(
            position_mode(link, Activity::Driving, now, STREAM_UPDATE_STALE),
            PositionMode::Hybrid
        );
    }

    #[test]
    fn fallback_drive_positions_follow_the_interval() {
        let now = now();
        let interval = Duration::from_secs(300);
        assert!(store_poll_position(
            PositionMode::Fallback,
            true,
            None,
            now,
            interval
        ));
        assert!(!store_poll_position(
            PositionMode::Fallback,
            true,
            Some(now.checked_sub(Duration::from_secs(30)).unwrap()),
            now,
            interval
        ));
        assert!(store_poll_position(
            PositionMode::Fallback,
            true,
            Some(now.checked_sub(Duration::from_secs(300)).unwrap()),
            now,
            interval
        ));
        assert!(!store_poll_position(
            PositionMode::Hybrid,
            true,
            None,
            now,
            interval
        ));
        assert!(store_poll_position(
            PositionMode::Hybrid,
            false,
            None,
            now,
            interval
        ));
    }

    #[test]
    fn fallback_poll_does_not_wake_a_sleeping_car() {
        assert!(!fallback_observe(
            PositionMode::Fallback,
            Presence::Asleep,
            false,
            false
        ));
        assert!(!fallback_observe(
            PositionMode::Fallback,
            Presence::Offline,
            false,
            false
        ));
        assert!(fallback_observe(
            PositionMode::Fallback,
            Presence::Online,
            false,
            false
        ));
        assert!(!fallback_observe(
            PositionMode::Hybrid,
            Presence::Online,
            false,
            false
        ));
        assert!(!fallback_observe(
            PositionMode::Fallback,
            Presence::Online,
            true,
            false
        ));
        assert!(!fallback_observe(
            PositionMode::Fallback,
            Presence::Online,
            false,
            true
        ));
    }

    #[test]
    fn gear_or_speed_marks_a_drive_sample() {
        assert!(sample_is_driving(Activity::Online, Some("D"), Some(0.0)));
        assert!(sample_is_driving(Activity::Driving, Some("P"), Some(0.0)));
        assert!(sample_is_driving(Activity::Online, None, Some(12.0)));
        assert!(!sample_is_driving(Activity::Online, Some("P"), Some(0.0)));
    }

    #[tokio::test]
    async fn board_switches_hybrid_and_fallback_without_logging_a_vin() {
        let debug = DebugBoard::new(LogBuffer::new(8));
        let board = GpsBoard::new(debug.clone(), Arc::new(Notify::new()));
        let vin = "5YJ3E1EA7KF000001";
        let up = board.mark_up(7).await;
        assert_eq!(up.mode, PositionMode::Hybrid);
        assert!(!up.became_fallback);
        debug.set_owner_vehicle(7, 99, "connected");
        board.mark_up(7).await;
        let driving = board
            .reconcile(7, Presence::Online, Activity::Driving)
            .await;
        assert_eq!(driving.mode, PositionMode::Fallback);
        assert!(driving.became_fallback);
        let flowing = board.mark_update(7).await;
        assert_eq!(flowing.mode, PositionMode::Hybrid);
        assert!(!flowing.became_fallback);
        let down = board.mark_down(7).await;
        assert_eq!(down.mode, PositionMode::Fallback);
        assert!(down.became_fallback);
        let view = debug.console(&[]);
        assert!(view.body.contains("Position mode"));
        assert!(view.body.contains("fallback"));
        assert!(!view.body.contains(vin), "{vin} leaked into {}", view.body);
        assert_eq!(board.mode_name(7).await, "fallback");
        assert_eq!(board.mode_name(99).await, "fallback");
    }
}
