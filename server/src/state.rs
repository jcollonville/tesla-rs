// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

use chrono::{DateTime, Utc};

use crate::geofence::haversine_m;

/// Coarse Fleet API presence. This is not the same as [`Activity`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Presence {
    Online,
    Asleep,
    Offline,
}

impl Presence {
    pub(crate) fn from_fleet(state: Option<&str>) -> Self {
        match state.map(str::to_ascii_lowercase).as_deref() {
            Some("online") => Self::Online,
            Some("asleep") => Self::Asleep,
            _ => Self::Offline,
        }
    }
}

/// What the archive is currently recording for a vehicle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Activity {
    Online,
    Asleep,
    Offline,
    Driving,
    Charging,
    Updating,
}

impl Activity {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Asleep => "asleep",
            Self::Offline => "offline",
            Self::Driving => "driving",
            Self::Charging => "charging",
            Self::Updating => "updating",
        }
    }

    pub(crate) fn parse(value: &str) -> Self {
        match value {
            "online" => Self::Online,
            "asleep" => Self::Asleep,
            "driving" => Self::Driving,
            "charging" => Self::Charging,
            "updating" => Self::Updating,
            _ => Self::Offline,
        }
    }
}

/// One observation from polling or, later, from Fleet Telemetry.
#[derive(Clone, Debug)]
pub(crate) struct Observation {
    pub at: DateTime<Utc>,
    pub presence: Presence,
    pub shift_state: Option<String>,
    pub charging_state: Option<String>,
    pub speed_kmh: Option<f64>,
    pub power_kw: Option<f64>,
    pub odometer_km: Option<f64>,
    pub battery_level: Option<f64>,
    pub usable_battery_level: Option<f64>,
    pub rated_range_km: Option<f64>,
    pub ideal_range_km: Option<f64>,
    pub est_range_km: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub elevation_m: Option<f64>,
    pub heading_deg: Option<f64>,
    pub inside_temp_c: Option<f64>,
    pub outside_temp_c: Option<f64>,
    pub charge_energy_added_kwh: Option<f64>,
    pub charger_power_kw: Option<f64>,
    pub charger_voltage: Option<f64>,
    pub charger_current: Option<f64>,
    pub fast_charger: Option<bool>,
    pub fast_charger_type: Option<String>,
    pub charge_limit_soc: Option<f64>,
    pub update_status: Option<String>,
    pub software_version: Option<String>,
    pub locked: Option<bool>,
    pub sentry_mode: Option<bool>,
    pub climate_on: Option<bool>,
    pub charge_current_request: Option<f64>,
    pub charge_port_door_open: Option<bool>,
    pub charge_port_latch: Option<String>,
    pub conn_charge_cable: Option<String>,
    pub plugged_in: Option<bool>,
    pub time_to_full_charge_h: Option<f64>,
    /// Minutes after midnight on the car's local clock. This is the Fleet API unit.
    pub scheduled_charging_start_time: Option<i32>,
    /// `Off`, `StartAt`, or `DepartBy`.
    pub scheduled_charging_mode: Option<String>,
    /// Minutes after midnight on the car's local clock.
    pub scheduled_departure_time: Option<i32>,
    pub off_peak_charging_enabled: Option<bool>,
    /// Minutes after midnight on the car's local clock.
    pub off_peak_hours_end_time: Option<i32>,
    pub tpms_pressure_fl: Option<f64>,
    pub tpms_pressure_fr: Option<f64>,
    pub tpms_pressure_rl: Option<f64>,
    pub tpms_pressure_rr: Option<f64>,
    /// Driver front door. `0` closed, nonzero open.
    pub df: Option<i32>,
    pub dr: Option<i32>,
    pub pf: Option<i32>,
    pub pr: Option<i32>,
    pub fd_window: Option<i32>,
    pub fp_window: Option<i32>,
    pub rd_window: Option<i32>,
    pub rp_window: Option<i32>,
    pub ft: Option<i32>,
    pub rt: Option<i32>,
    pub is_user_present: Option<bool>,
    pub battery_heater_on: Option<bool>,
    pub is_preconditioning: Option<bool>,
    pub model: Option<String>,
    pub trim_badging: Option<String>,
    pub exterior_color: Option<String>,
    pub wheel_type: Option<String>,
    pub distance_unit: Option<String>,
    pub efficiency_kwh_per_km: Option<f64>,
}

impl Observation {
    pub(crate) fn at(at: DateTime<Utc>, presence: Presence) -> Self {
        Self {
            at,
            presence,
            shift_state: None,
            charging_state: None,
            speed_kmh: None,
            power_kw: None,
            odometer_km: None,
            battery_level: None,
            usable_battery_level: None,
            rated_range_km: None,
            ideal_range_km: None,
            est_range_km: None,
            latitude: None,
            longitude: None,
            elevation_m: None,
            heading_deg: None,
            inside_temp_c: None,
            outside_temp_c: None,
            charge_energy_added_kwh: None,
            charger_power_kw: None,
            charger_voltage: None,
            charger_current: None,
            fast_charger: None,
            fast_charger_type: None,
            charge_limit_soc: None,
            update_status: None,
            software_version: None,
            locked: None,
            sentry_mode: None,
            climate_on: None,
            charge_current_request: None,
            charge_port_door_open: None,
            charge_port_latch: None,
            conn_charge_cable: None,
            plugged_in: None,
            time_to_full_charge_h: None,
            scheduled_charging_start_time: None,
            scheduled_charging_mode: None,
            scheduled_departure_time: None,
            off_peak_charging_enabled: None,
            off_peak_hours_end_time: None,
            tpms_pressure_fl: None,
            tpms_pressure_fr: None,
            tpms_pressure_rl: None,
            tpms_pressure_rr: None,
            df: None,
            dr: None,
            pf: None,
            pr: None,
            fd_window: None,
            fp_window: None,
            rd_window: None,
            rp_window: None,
            ft: None,
            rt: None,
            is_user_present: None,
            battery_heater_on: None,
            is_preconditioning: None,
            model: None,
            trim_badging: None,
            exterior_color: None,
            wheel_type: None,
            distance_unit: None,
            efficiency_kwh_per_km: None,
        }
    }

    pub(crate) fn has_sample(&self) -> bool {
        self.latitude.is_some()
            || self.odometer_km.is_some()
            || self.battery_level.is_some()
            || self.speed_kmh.is_some()
            || self.charger_power_kw.is_some()
    }

    pub(crate) fn sample(&self) -> Sample {
        Sample {
            at: self.at,
            latitude: self.latitude,
            longitude: self.longitude,
            speed_kmh: self.speed_kmh,
            power_kw: self.power_kw,
            odometer_km: self.odometer_km,
            elevation_m: self.elevation_m,
            battery_level: self.battery_level,
            rated_range_km: self.rated_range_km,
            ideal_range_km: self.ideal_range_km,
            inside_temp_c: self.inside_temp_c,
            outside_temp_c: self.outside_temp_c,
            charge_energy_added_kwh: self.charge_energy_added_kwh,
            charger_power_kw: self.charger_power_kw,
            fast_charger: self.fast_charger,
            fast_charger_type: self.fast_charger_type.clone(),
        }
    }
}

/// State of charge to store for a drive boundary.
///
/// `battery_level` is the displayed percent. `usable_battery_level` fills in when
/// the car omits it.
pub(crate) fn soc_level(battery: Option<f64>, usable: Option<f64>) -> Option<f64> {
    finite_soc(battery).or_else(|| finite_soc(usable))
}

fn finite_soc(value: Option<f64>) -> Option<f64> {
    value.filter(|level| level.is_finite())
}

/// Whether a charge cable is connected.
///
/// A real `conn_charge_cable` means plugged in. `charging_state` of
/// `Disconnected` (also `none` or `null`) means unplugged, whatever the latch
/// says. Any other charging state means plugged in, except `Unknown`, which is
/// not a connection decision. With no decision from the charging state, latch
/// `Engaged` or `Blocking` means plugged in unless the port door is closed and
/// charger power is 0: that latch is holding the door, not a cable. An explicit
/// `Disengaged` latch with no other signal is unplugged. Missing inputs stay
/// unknown.
pub(crate) fn derive_plugged_in(
    charge_port_latch: Option<&str>,
    charging_state: Option<&str>,
    conn_charge_cable: Option<&str>,
    charge_port_door_open: Option<bool>,
    charger_power_kw: Option<f64>,
) -> Option<bool> {
    if cable_is_connected(conn_charge_cable) {
        return Some(true);
    }
    if let Some(state) = nonempty(charging_state)
        && let Some(plugged) = charge_connection(state)
    {
        return Some(plugged);
    }
    if latch_is(charge_port_latch, &["engaged", "blocking"]) {
        return Some(!door_is_latched_shut(
            charge_port_door_open,
            charger_power_kw,
        ));
    }
    if latch_is(charge_port_latch, &["disengaged"]) {
        return Some(false);
    }
    None
}

pub(crate) fn refresh_plugged_in(obs: &mut Observation) {
    obs.plugged_in = derive_plugged_in(
        obs.charge_port_latch.as_deref(),
        obs.charging_state.as_deref(),
        obs.conn_charge_cable.as_deref(),
        obs.charge_port_door_open,
        obs.charger_power_kw,
    );
}

fn latch_is(latch: Option<&str>, names: &[&str]) -> bool {
    let Some(latch) = nonempty(latch) else {
        return false;
    };
    names.iter().any(|name| latch.eq_ignore_ascii_case(name))
}

fn cable_is_connected(cable: Option<&str>) -> bool {
    let Some(cable) = nonempty(cable) else {
        return false;
    };
    !matches!(
        cable.to_ascii_lowercase().as_str(),
        "<invalid>" | "none" | "null" | "disconnected"
    )
}

/// Charger type worth storing.
///
/// Tesla reports `fast_charger_type` as `<invalid>` when the car is not on a
/// fast charger. Empty text, `none`, `null`, and any `<...>` placeholder are
/// dropped. Comparison is trimmed and ASCII case-insensitive.
pub(crate) fn sanitize_charger_type(value: Option<&str>) -> Option<String> {
    let value = nonempty(value)?;
    let normalized = value.to_ascii_lowercase();
    if matches!(normalized.as_str(), "<invalid>" | "none" | "null")
        || (value.starts_with('<') && value.ends_with('>'))
    {
        return None;
    }
    Some(value.to_owned())
}

/// `None` when `state` does not say whether a cable is connected.
fn charge_connection(state: &str) -> Option<bool> {
    match state.to_ascii_lowercase().as_str() {
        "disconnected" | "none" | "null" => Some(false),
        "unknown" => None,
        _ => Some(true),
    }
}

/// Closed port and no charger power. The latch is holding the door shut.
fn door_is_latched_shut(door_open: Option<bool>, power_kw: Option<f64>) -> bool {
    door_open == Some(false) && power_kw.is_some_and(|power| power.is_finite() && power <= 0.0)
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
}

/// Cable state for the read API. Independent of sleep and driving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CableStatus {
    Charging,
    Plugged,
    Unplugged,
}

impl CableStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Charging => "charging",
            Self::Plugged => "plugged",
            Self::Unplugged => "unplugged",
        }
    }
}

/// `Charging` is charging. `Starting`, `Stopped`, `Complete`, and `NoPower` are
/// plugged in. `Disconnected` is unplugged even when a stale `plugged_in` is set.
pub(crate) fn cable_status(
    charging_state: Option<&str>,
    plugged_in: Option<bool>,
) -> Option<CableStatus> {
    let charging = nonempty(charging_state).unwrap_or("").to_ascii_lowercase();
    if charging == "charging" {
        return Some(CableStatus::Charging);
    }
    if matches!(
        charging.as_str(),
        "starting" | "complete" | "stopped" | "nopower"
    ) {
        return Some(CableStatus::Plugged);
    }
    if charging != "disconnected" && plugged_in == Some(true) {
        return Some(CableStatus::Plugged);
    }
    if charging == "disconnected" || plugged_in == Some(false) {
        return Some(CableStatus::Unplugged);
    }
    None
}

/// Canonical schedule mode. Unknown text is dropped.
pub(crate) fn normalize_charging_mode(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "scheduledchargingmodeoff" => Some("Off"),
        "startat" | "start_at" | "scheduledchargingmodestartat" => Some("StartAt"),
        "departby" | "depart_by" | "scheduledchargingmodedepartby" => Some("DepartBy"),
        _ => None,
    }
}

/// Open-session memory. Aggregates are recomputed from stored samples when a session closes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Tracker {
    pub activity: Activity,
    pub drive_open: bool,
    pub charge_open: bool,
}

impl Default for Tracker {
    fn default() -> Self {
        Self {
            activity: Activity::Offline,
            drive_open: false,
            charge_open: false,
        }
    }
}

/// A session boundary detected by [`reduce`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Transition {
    DriveStarted,
    DriveEnded,
    ChargeStarted,
    ChargeEnded,
}

/// Advances the activity tracker by one observation.
///
/// A drive starts when the car is online and the shift state is `D`, `R`, or `N`,
/// or when speed is above 1 km/h and no park shift is reported. Charging closes an
/// open drive immediately. Park, sleep, offline, and an incomplete poll do not:
/// [`plan_drive_close`] keeps the drive open until the matching grace has elapsed.
/// A charge starts on `Charging` or `Starting` and ends on any other charging state,
/// including sleep. The reducer never asks the car to wake.
pub(crate) fn reduce(tracker: &Tracker, obs: &Observation) -> (Tracker, Vec<Transition>) {
    let driving = is_driving(obs);
    let charging = is_charging(obs);
    let updating = obs.presence == Presence::Online && !driving && !charging && is_updating(obs);
    let mut events = Vec::new();
    let mut next = tracker.clone();

    if driving {
        if tracker.charge_open {
            events.push(Transition::ChargeEnded);
            next.charge_open = false;
        }
        if !tracker.drive_open {
            events.push(Transition::DriveStarted);
            next.drive_open = true;
        }
        next.activity = Activity::Driving;
        return (next, events);
    }

    // Signal loss and park stay open here. Only plugging in ends the drive at once.
    if charging && tracker.drive_open {
        events.push(Transition::DriveEnded);
        next.drive_open = false;
    }

    if charging {
        if !tracker.charge_open {
            events.push(Transition::ChargeStarted);
            next.charge_open = true;
        }
        next.activity = Activity::Charging;
        return (next, events);
    }

    if tracker.charge_open {
        events.push(Transition::ChargeEnded);
        next.charge_open = false;
    }

    next.activity = if updating {
        Activity::Updating
    } else {
        match obs.presence {
            Presence::Online => Activity::Online,
            Presence::Asleep => Activity::Asleep,
            Presence::Offline => Activity::Offline,
        }
    };
    (next, events)
}

fn is_driving(obs: &Observation) -> bool {
    if obs.presence != Presence::Online {
        return false;
    }
    match normalized_shift(obs.shift_state.as_deref()) {
        Some("D" | "R" | "N") => true,
        Some("P") => false,
        _ => obs
            .speed_kmh
            .is_some_and(|speed| speed.is_finite() && speed > 1.0),
    }
}

fn is_charging(obs: &Observation) -> bool {
    if obs.presence != Presence::Online || is_driving(obs) {
        return false;
    }
    matches!(
        obs.charging_state
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("charging" | "starting")
    )
}

fn is_updating(obs: &Observation) -> bool {
    let status = obs
        .update_status
        .as_deref()
        .unwrap_or("")
        .to_ascii_lowercase();
    status.contains("install") || status.contains("download")
}

/// Default wait after the last driving evidence before a signal gap closes a drive.
pub(crate) const DRIVE_END_GRACE_DEFAULT_SECS: u64 = 300;
/// Shortest accepted signal-loss grace.
pub(crate) const DRIVE_END_GRACE_MIN_SECS: u64 = 30;
/// Longest accepted signal-loss grace.
pub(crate) const DRIVE_END_GRACE_MAX_SECS: u64 = 1_800;
/// Default time at exact 0 km/h before a drive closes. Five minutes.
pub(crate) const DRIVE_PARK_GRACE_DEFAULT_SECS: u64 = 300;
/// Shortest accepted park grace.
pub(crate) const DRIVE_PARK_GRACE_MIN_SECS: u64 = 15;
/// Longest accepted park grace.
pub(crate) const DRIVE_PARK_GRACE_MAX_SECS: u64 = 900;

/// Speed at or below this, with shift `P`, is a confirmed park.
const STANDSTILL_KMH: f64 = 1.0;
/// End or start speed above this counts as motion across a gap.
const GAP_MOTION_KMH: f64 = 5.0;
/// Odometer change above this is motion, not a parked stop.
const ODO_MOTION_KM: f64 = 0.05;
/// Odometer rewind larger than this is a different drive.
const ODO_REWIND_KM: f64 = 0.05;
/// Straight-line gap above this is not the same place.
const SAME_PLACE_M: f64 = 50.0;
/// Upper bound used to reject an odometer or map jump.
const MAX_JOIN_SPEED_KMH: f64 = 250.0;

/// How long an open drive may stay open without new driving evidence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DriveGrace {
    /// Signal loss, asleep, offline, or an incomplete poll.
    pub end: chrono::Duration,
    /// Sustained exact 0 km/h. Closure only; the trim uses the first 0 sample.
    pub park: chrono::Duration,
}

impl DriveGrace {
    /// Built-in 300 s signal-loss window and 300 s at 0 km/h.
    pub(crate) const fn standard() -> Self {
        Self {
            end: chrono::Duration::seconds(300),
            park: chrono::Duration::seconds(300),
        }
    }

    /// Windows already checked against the config bounds.
    pub(crate) fn from_secs(end_secs: u64, park_secs: u64) -> Self {
        Self {
            end: chrono::Duration::seconds(i64::try_from(end_secs).unwrap_or(300)),
            park: chrono::Duration::seconds(i64::try_from(park_secs).unwrap_or(300)),
        }
    }
}

/// What one sample says about an open drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DriveMotion {
    /// Shift `D`/`R`/`N`, or speed above 1 km/h without park.
    Driving,
    /// Shift `P` and speed about zero.
    Park,
    /// Charging or starting to charge. Closes the drive at once.
    Charging,
    /// Asleep, offline, incomplete poll, or park while still moving.
    Gap,
    /// Stream frame with no shift and no moving speed. Keeps the open drive.
    Hold,
}

/// Last driving evidence and the start of a confirmed-park run.
///
/// Both instants come from position timestamps, so a restart can rebuild this
/// from stored rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DriveClock {
    /// Time of the latest sample that was still driving.
    pub last_driving_at: DateTime<Utc>,
    /// First confirmed-park sample of the current park run.
    pub park_since: Option<DateTime<Utc>>,
}

/// Whether to keep, wait, or close an open drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ClosePlan {
    /// Driving resumed, or the frame does not prove a stop.
    Continue {
        /// Clock after this sample.
        clock: DriveClock,
    },
    /// Close once `deadline` is reached, even if nothing else arrives.
    Wait {
        /// Instant at which the grace has elapsed.
        deadline: DateTime<Utc>,
        /// Clock after this sample.
        clock: DriveClock,
    },
    /// Grace elapsed, or charging started.
    Close {
        /// Clock at the close. [`drive_end_cutoff`] picks `end_at` as the first
        /// exact 0 km/h sample of the final zero run. Samples after that instant
        /// are detached. A tail with no 0 km/h sample keeps its newest time.
        clock: DriveClock,
    },
}

/// One stored fix used to rebuild [`DriveClock`] after a restart.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PositionCue {
    /// Sample time.
    pub at: DateTime<Utc>,
    /// Speed in km/h, when the row has one.
    pub speed_kmh: Option<f64>,
    /// Shift state, when the row has one.
    pub shift_state: Option<String>,
}

/// Facts about the hole between two drives. No coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GapFacts {
    /// `later.start - earlier.end`, in seconds.
    pub gap_secs: i64,
    /// Speed of the last position on the earlier drive.
    pub end_speed_kmh: Option<f64>,
    /// Speed of the first position on the later drive.
    pub start_speed_kmh: Option<f64>,
    /// Odometer at the end of the earlier drive.
    pub end_odometer_km: Option<f64>,
    /// Odometer at the start of the later drive.
    pub start_odometer_km: Option<f64>,
    /// Distance between the end point and the start point, in metres.
    pub gap_m: Option<f64>,
}

/// Speed is present and about zero, or missing next to an explicit park shift.
pub(crate) fn speed_is_standstill(speed: Option<f64>) -> bool {
    match speed {
        Some(speed) if speed.is_finite() => speed <= STANDSTILL_KMH,
        Some(_) => false,
        None => true,
    }
}

/// Classifies a poll or telemetry observation.
pub(crate) fn observation_motion(obs: &Observation) -> DriveMotion {
    if is_charging(obs) {
        return DriveMotion::Charging;
    }
    if is_driving(obs) {
        return DriveMotion::Driving;
    }
    if obs.presence == Presence::Online
        && normalized_shift(obs.shift_state.as_deref()) == Some("P")
        && speed_is_standstill(obs.speed_kmh)
    {
        return DriveMotion::Park;
    }
    DriveMotion::Gap
}

/// Classifies one streaming sample.
///
/// An unknown shift on an open drive is [`DriveMotion::Hold`]: frames often omit
/// `shift_state` when it did not change. Confirmed park is shift `P` with speed
/// about zero. `P` while still moving is a gap, not a park.
pub(crate) fn stream_motion(
    shift: Option<&str>,
    speed_kmh: Option<f64>,
    drive_open: bool,
) -> DriveMotion {
    match normalized_shift(shift) {
        Some("D" | "R" | "N") => DriveMotion::Driving,
        Some("P") if speed_is_standstill(speed_kmh) => DriveMotion::Park,
        Some("P") => DriveMotion::Gap,
        _ => match speed_kmh {
            Some(speed) if speed.is_finite() && speed > STANDSTILL_KMH => DriveMotion::Driving,
            _ if drive_open => DriveMotion::Hold,
            _ => DriveMotion::Gap,
        },
    }
}

/// Decides whether the open drive should close at `at`.
///
/// Charging closes immediately. Exact 0 km/h starts, or continues, a stop.
/// [`DriveGrace::park`] is how long that stop must last; it does not choose
/// `end_at`. A missing speed does not start the stop. Movement, including a
/// driving shift with no speed, clears it. A signal gap with no 0 km/h sample
/// uses [`DriveGrace::end`], measured from the last driving evidence.
/// Row `end_at` is chosen later by [`drive_end_cutoff`].
pub(crate) fn plan_drive_close(
    motion: DriveMotion,
    at: DateTime<Utc>,
    clock: DriveClock,
    grace: DriveGrace,
    speed_kmh: Option<f64>,
) -> ClosePlan {
    if motion == DriveMotion::Charging {
        return ClosePlan::Close { clock };
    }
    if motion == DriveMotion::Hold && !speed_is_exact_zero(speed_kmh) {
        return ClosePlan::Continue { clock };
    }
    let clock = if speed_is_exact_zero(speed_kmh) {
        DriveClock {
            last_driving_at: clock.last_driving_at,
            park_since: Some(clock.park_since.unwrap_or(at)),
        }
    } else if motion == DriveMotion::Driving || speed_is_movement(speed_kmh) {
        let late_park = clock
            .park_since
            .is_some_and(|since| at + chrono::Duration::seconds(1) < since);
        if late_park {
            return ClosePlan::Continue { clock };
        }
        DriveClock {
            last_driving_at: clock.last_driving_at.max(at),
            park_since: None,
        }
    } else {
        clock
    };
    if let Some(since) = clock.park_since {
        return grace_or_wait(at, since, grace.park, clock);
    }
    if motion == DriveMotion::Gap {
        return grace_or_wait(at, clock.last_driving_at, grace.end, clock);
    }
    ClosePlan::Continue { clock }
}

fn grace_or_wait(
    at: DateTime<Utc>,
    since: DateTime<Utc>,
    grace: chrono::Duration,
    clock: DriveClock,
) -> ClosePlan {
    if at.signed_duration_since(since) >= grace {
        ClosePlan::Close { clock }
    } else {
        ClosePlan::Wait {
            deadline: since + grace,
            clock,
        }
    }
}

/// Rebuilds the close clock from newest-first position cues.
///
/// `park_since` is the first exact 0 km/h sample of the final zero run, the same
/// instant [`drive_end_cutoff`] keeps. The returned motion is [`DriveMotion::Park`]
/// when that run exists, otherwise [`DriveMotion::Gap`].
pub(crate) fn clock_from_tail(
    cues: &[PositionCue],
    drive_start: DateTime<Utc>,
) -> (DriveClock, DriveMotion) {
    let park_since = final_zero_run_start(cues);
    let last_driving = cues
        .iter()
        .find(|cue| position_is_later_movement(cue))
        .map_or(drive_start, |cue| cue.at);
    let motion = if park_since.is_some() {
        DriveMotion::Park
    } else {
        DriveMotion::Gap
    };
    (
        DriveClock {
            last_driving_at: last_driving,
            park_since,
        },
        motion,
    )
}

/// Exact 0 km/h. The 1 km/h standstill band used for park detection does not apply.
fn speed_is_exact_zero(speed: Option<f64>) -> bool {
    let Some(speed) = speed else {
        return false;
    };
    speed.is_finite() && speed.abs() <= f64::EPSILON
}

fn speed_is_movement(speed: Option<f64>) -> bool {
    match speed {
        Some(speed) if speed.is_finite() => speed.abs() > f64::EPSILON,
        _ => false,
    }
}

/// Newer than a candidate stop, and not itself an exact 0 km/h sample.
///
/// Any other finite speed counts, including a creep below 1 km/h. A driving
/// shift with no speed counts too, so a missing reading cannot hide a later roll.
/// A missing speed without a driving shift does not: it stays inside the run
/// and cannot start one.
fn position_is_later_movement(cue: &PositionCue) -> bool {
    if speed_is_exact_zero(cue.speed_kmh) {
        return false;
    }
    match cue.speed_kmh {
        Some(speed) if speed.is_finite() => true,
        _ => matches!(
            normalized_shift(cue.shift_state.as_deref()),
            Some("D" | "R" | "N")
        ),
    }
}

/// First exact 0 km/h sample of the final non-moving run, newest cue first.
///
/// A missing speed stays in the run and does not start it. Movement ends the
/// walk, so a 0 km/h sample followed by a later moving sample is ignored.
pub(crate) fn final_zero_run_start(cues: &[PositionCue]) -> Option<DateTime<Utc>> {
    let mut start = None;
    for cue in cues {
        if position_is_later_movement(cue) {
            return start;
        }
        if speed_is_exact_zero(cue.speed_kmh) {
            start = Some(cue.at);
        }
    }
    start
}

/// Trip end used when a drive closes and when [`plan_drive_end_repair`] replays it.
///
/// `cues` are newest first. The cutoff is the first exact 0 km/h sample of the
/// final zero run. Positions after that instant are detached, not deleted.
/// When that run has no exact 0 km/h sample, the cutoff is the newest sample.
/// [`DriveGrace`] is not consulted: it only decides when the drive closes.
pub(crate) fn drive_end_cutoff(cues: &[PositionCue], drive_start: DateTime<Utc>) -> DateTime<Utc> {
    let Some(newest) = cues.first() else {
        return drive_start;
    };
    final_zero_run_start(cues).unwrap_or(newest.at)
}

/// New `end_at` for one closed drive, or `None` when the row must stay as stored.
///
/// `cues` are newest first and include only positions still attached to the drive.
/// The end moves to the first exact 0 km/h sample of the final zero run, the same
/// instant as [`drive_end_cutoff`]. No such sample, or a 0 km/h sample followed by
/// movement, leaves the drive unchanged. An end that already matches that instant,
/// with nothing attached after it, is unchanged.
pub(crate) fn plan_drive_end_repair(
    cues: &[PositionCue],
    drive_start: DateTime<Utc>,
    end_at: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    if !tail_has_exact_zero(cues) {
        return None;
    }
    let cutoff = drive_end_cutoff(cues, drive_start);
    let attached_after = cues.iter().any(|cue| cue.at > cutoff);
    if end_at == cutoff && !attached_after {
        None
    } else {
        Some(cutoff)
    }
}

/// Exact 0 km/h before any later movement. Missing speed does not count.
fn tail_has_exact_zero(cues: &[PositionCue]) -> bool {
    for cue in cues {
        if speed_is_exact_zero(cue.speed_kmh) {
            return true;
        }
        if position_is_later_movement(cue) {
            return false;
        }
    }
    false
}

/// Prefers in-memory evidence when it is newer than the rows already stored.
///
/// A hybrid poll can prove the car is still driving without writing a position.
pub(crate) fn merge_clock(stored: DriveClock, memory: Option<DriveClock>) -> DriveClock {
    let Some(memory) = memory else {
        return stored;
    };
    if memory.last_driving_at > stored.last_driving_at {
        return DriveClock {
            last_driving_at: memory.last_driving_at,
            park_since: None,
        };
    }
    if stored.park_since.is_none()
        && let Some(since) = memory.park_since
        && since >= stored.last_driving_at
    {
        return DriveClock {
            last_driving_at: stored.last_driving_at,
            park_since: Some(since),
        };
    }
    stored
}

/// A read-only split candidate: a short gap with motion, and no huge odometer jump.
pub(crate) fn split_candidate(gap: &GapFacts, grace_secs: i64) -> bool {
    gap.gap_secs >= 0
        && gap.gap_secs < grace_secs
        && odometer_plausible(gap)
        && moving_across_gap(gap)
}

/// Safety net: reopen the previous drive instead of inserting a new one.
///
/// Requires the same motion test as [`split_candidate`] plus a continuous
/// odometer. A parked stop at speed zero in the same place stays split.
pub(crate) fn should_reopen_drive(gap: &GapFacts, grace_secs: i64) -> bool {
    split_candidate(gap, grace_secs) && odometer_delta(gap).is_some()
}

fn odometer_delta(gap: &GapFacts) -> Option<f64> {
    let end = gap.end_odometer_km.filter(|km| km.is_finite())?;
    let start = gap.start_odometer_km.filter(|km| km.is_finite())?;
    Some(start - end)
}

fn odometer_plausible(gap: &GapFacts) -> bool {
    let Some(delta) = odometer_delta(gap) else {
        return true;
    };
    if delta < -ODO_REWIND_KM {
        return false;
    }
    let elapsed = f64::from(i32::try_from(gap.gap_secs.max(0)).unwrap_or(i32::MAX));
    let max_km = (MAX_JOIN_SPEED_KMH / 3_600.0) * elapsed + 0.3;
    delta <= max_km.max(0.3)
}

fn moving_across_gap(gap: &GapFacts) -> bool {
    let end_fast = speed_above(gap.end_speed_kmh, GAP_MOTION_KMH);
    let start_fast = speed_above(gap.start_speed_kmh, GAP_MOTION_KMH);
    let odo_moving = odometer_delta(gap).is_some_and(|delta| delta > ODO_MOTION_KM);
    let gps_moving = gap
        .gap_m
        .is_some_and(|metres| metres.is_finite() && metres > SAME_PLACE_M);
    if !end_fast && !odo_moving && !gps_moving {
        return false;
    }
    let distance_ok = gap.gap_m.is_none_or(|metres| {
        if !metres.is_finite() || metres < 0.0 {
            return false;
        }
        let elapsed = f64::from(i32::try_from(gap.gap_secs.max(0)).unwrap_or(i32::MAX));
        let max_m = (MAX_JOIN_SPEED_KMH / 3.6) * elapsed + 300.0;
        if metres > max_m {
            return false;
        }
        match odometer_delta(gap) {
            Some(delta) => metres <= delta * 1_000.0 + 500.0_f64.max(delta.abs() * 500.0),
            None => true,
        }
    });
    if !distance_ok {
        return false;
    }
    (end_fast && (odo_moving || gps_moving))
        || (odo_moving && (gps_moving || start_fast || end_fast))
}

fn speed_above(speed: Option<f64>, threshold: f64) -> bool {
    speed.is_some_and(|speed| speed.is_finite() && speed > threshold)
}

/// Drive boundaries implied by one Owner API streaming sample.
///
/// An unknown shift keeps the current drive open. Streaming frames often omit
/// `shift_state` when the value did not change, and that must not close the drive.
/// [`StreamDrive::end_drive`] marks a confirmed park sample so it is stored. The
/// caller closes only after [`plan_drive_close`] says the park grace has elapsed.
/// A charge stays open until a drive actually starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StreamDrive {
    /// This sample should open a drive when none is open.
    pub start_drive: bool,
    /// Confirmed park on an open drive. Storage only; closing waits for the grace.
    pub end_drive: bool,
    /// Starting a drive ends an open charge.
    pub end_charge: bool,
    /// Activity implied by this sample, ignoring an unelapsed grace.
    pub activity: Activity,
    /// The car is driving, or an unknown frame is keeping an open drive.
    pub driving: bool,
    /// How this sample affects the close clock.
    pub motion: DriveMotion,
}

pub(crate) fn stream_drive_decision(
    tracker: &Tracker,
    shift: Option<&str>,
    speed_kmh: Option<f64>,
) -> StreamDrive {
    let motion = stream_motion(shift, speed_kmh, tracker.drive_open);
    let driving = matches!(motion, DriveMotion::Driving | DriveMotion::Hold)
        && (tracker.drive_open || motion == DriveMotion::Driving);
    let start_drive = motion == DriveMotion::Driving && !tracker.drive_open;
    let end_drive = motion == DriveMotion::Park && tracker.drive_open;
    let end_charge = start_drive && tracker.charge_open;
    let charge_open = tracker.charge_open && !end_charge;
    let activity = if driving {
        Activity::Driving
    } else if charge_open {
        Activity::Charging
    } else if tracker.activity == Activity::Updating {
        Activity::Updating
    } else {
        Activity::Online
    };
    StreamDrive {
        start_drive,
        end_drive,
        end_charge,
        activity,
        driving,
        motion,
    }
}

/// A drive that has already been closed. Late stream samples may join it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ClosedDrive {
    pub id: i64,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
}

/// Samples older than this at processing time must not open a drive.
pub(crate) const STALE_STREAM_OPEN: chrono::Duration = chrono::Duration::seconds(30);

/// How far before a closed drive a late sample may still belong to it.
pub(crate) const CLOSED_DRIVE_SLACK: chrono::Duration = chrono::Duration::seconds(30);

/// Whether one stream sample may open a drive, and which closed drive it joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StreamSampleGate {
    pub allow_open: bool,
    pub attach_closed: Option<i64>,
}

/// Gates a stream sample against the last closed drive and the processing clock.
///
/// A sample at or before that drive's `end_at`, or older than [`STALE_STREAM_OPEN`],
/// must not open a drive. A sample inside the drive, or up to [`CLOSED_DRIVE_SLACK`]
/// before it started, is attached to that drive instead.
pub(crate) fn gate_stream_sample(
    at: DateTime<Utc>,
    now: DateTime<Utc>,
    last_closed: Option<ClosedDrive>,
) -> StreamSampleGate {
    let stale = now.signed_duration_since(at) > STALE_STREAM_OPEN;
    let covers_closed = last_closed.is_some_and(|closed| at <= closed.end_at);
    let attach_closed = last_closed.and_then(|closed| {
        let earliest = closed.start_at - CLOSED_DRIVE_SLACK;
        (at >= earliest && at <= closed.end_at).then_some(closed.id)
    });
    StreamSampleGate {
        allow_open: !stale && !covers_closed,
        attach_closed,
    }
}

/// A streamed fix within two seconds of a polled row is the same moment.
///
/// Poll rows are every fifteen seconds or slower, so this only drops the stream
/// sample that landed on the poll. Neighbouring one-hertz samples stay.
#[allow(dead_code)] // Unit tests lock this window. The SQL uses the same two seconds.
pub(crate) fn stream_collides_with_poll(at: DateTime<Utc>, poll_at: DateTime<Utc>) -> bool {
    at.signed_duration_since(poll_at)
        .num_milliseconds()
        .unsigned_abs()
        <= 2_000
}

/// Whether this streaming fix should become a `positions` row.
pub(crate) fn should_store_stream_position(
    driving: bool,
    ending_drive: bool,
    latitude: Option<f64>,
    longitude: Option<f64>,
) -> bool {
    let Some(latitude) = latitude else {
        return false;
    };
    let Some(longitude) = longitude else {
        return false;
    };
    if !latitude.is_finite() || !longitude.is_finite() {
        return false;
    }
    if latitude.abs() < 1e-7 && longitude.abs() < 1e-7 {
        return false;
    }
    driving || ending_drive
}

fn normalized_shift(shift: Option<&str>) -> Option<&str> {
    match shift.map(str::trim) {
        Some("D" | "d") => Some("D"),
        Some("R" | "r") => Some("R"),
        Some("N" | "n") => Some("N"),
        Some("P" | "p") => Some("P"),
        _ => None,
    }
}

/// One stored sample used to close a drive or a charge.
#[derive(Clone, Debug)]
pub(crate) struct Sample {
    pub at: DateTime<Utc>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub speed_kmh: Option<f64>,
    pub power_kw: Option<f64>,
    pub odometer_km: Option<f64>,
    pub elevation_m: Option<f64>,
    pub battery_level: Option<f64>,
    pub rated_range_km: Option<f64>,
    pub ideal_range_km: Option<f64>,
    pub inside_temp_c: Option<f64>,
    pub outside_temp_c: Option<f64>,
    pub charge_energy_added_kwh: Option<f64>,
    pub charger_power_kw: Option<f64>,
    pub fast_charger: Option<bool>,
    pub fast_charger_type: Option<String>,
}

/// Closed drive totals. Distances are kilometres.
#[derive(Clone, Debug)]
pub(crate) struct DriveSummary {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub start_latitude: Option<f64>,
    pub start_longitude: Option<f64>,
    pub end_latitude: Option<f64>,
    pub end_longitude: Option<f64>,
    pub start_odometer_km: Option<f64>,
    pub end_odometer_km: Option<f64>,
    pub distance_km: Option<f64>,
    pub duration_s: i32,
    pub start_battery_level: Option<f64>,
    pub end_battery_level: Option<f64>,
    pub start_rated_range_km: Option<f64>,
    pub end_rated_range_km: Option<f64>,
    pub start_ideal_range_km: Option<f64>,
    pub end_ideal_range_km: Option<f64>,
    pub energy_used_kwh: Option<f64>,
    pub max_speed_kmh: Option<f64>,
    pub avg_speed_kmh: Option<f64>,
    pub max_power_kw: Option<f64>,
    pub min_power_kw: Option<f64>,
    pub elevation_gain_m: Option<f64>,
    pub elevation_loss_m: Option<f64>,
    pub inside_temp_avg_c: Option<f64>,
    pub outside_temp_avg_c: Option<f64>,
}

/// Minimum odometer delta, in kilometres, for a closed drive to be kept.
///
/// TeslaMate `Log.close_drive/2` uses this same 0.01 km threshold.
pub(crate) const MIN_CLOSED_DRIVE_KM: f64 = 0.01;

/// Whether closing this drive should update the row.
///
/// TeslaMate keeps a drive only when it has at least two positions and the
/// odometer of the last position minus the odometer of the first (ordered by
/// date) is at least [`MIN_CLOSED_DRIVE_KM`]. A missing odometer makes that
/// distance NULL. GPS distance is not consulted.
pub(crate) fn keep_closed_drive(samples: &[Sample]) -> bool {
    let Some((first, last)) = samples.first().zip(samples.last()) else {
        return false;
    };
    let Some(start) = first.odometer_km.filter(|km| km.is_finite()) else {
        return false;
    };
    let Some(end) = last.odometer_km.filter(|km| km.is_finite()) else {
        return false;
    };
    if samples.len() < 2 {
        return false;
    }
    // Odometers are decimal kilometres in TeslaMate and float8 here. Round the
    // delta to the nearest millimetre so a stored 0.01 km still meets the check.
    let delta_km = ((end - start) * 1_000_000.0).round() / 1_000_000.0;
    delta_km >= MIN_CLOSED_DRIVE_KM
}

pub(crate) fn summarize_drive(
    samples: &[Sample],
    efficiency_kwh_per_km: Option<f64>,
) -> Option<DriveSummary> {
    let first = samples.first()?;
    let last = samples.last()?;
    let start_odometer = first_finite(samples.iter().filter_map(|sample| sample.odometer_km));
    let end_odometer = last_finite(samples.iter().filter_map(|sample| sample.odometer_km));
    let distance_km = odometer_distance(start_odometer, end_odometer)
        .or_else(|| Some(path_distance_km(samples)))
        .filter(|distance| distance.is_finite());
    let duration_s = duration_s(first.at, last.at);
    let start_rated = first_finite(samples.iter().filter_map(|sample| sample.rated_range_km));
    let end_rated = last_finite(samples.iter().filter_map(|sample| sample.rated_range_km));
    let (elevation_gain_m, elevation_loss_m) = elevation_change(samples);
    let (start_latitude, start_longitude) = first_coord(samples);
    let (end_latitude, end_longitude) = last_coord(samples);
    Some(DriveSummary {
        start: first.at,
        end: last.at,
        start_latitude,
        start_longitude,
        end_latitude,
        end_longitude,
        start_odometer_km: start_odometer,
        end_odometer_km: end_odometer,
        distance_km,
        duration_s,
        start_battery_level: first_finite(samples.iter().filter_map(|sample| sample.battery_level)),
        end_battery_level: last_finite(samples.iter().filter_map(|sample| sample.battery_level)),
        start_rated_range_km: start_rated,
        end_rated_range_km: end_rated,
        start_ideal_range_km: first_finite(
            samples.iter().filter_map(|sample| sample.ideal_range_km),
        ),
        end_ideal_range_km: last_finite(samples.iter().filter_map(|sample| sample.ideal_range_km)),
        energy_used_kwh: range_energy(start_rated, end_rated, efficiency_kwh_per_km),
        max_speed_kmh: max_finite(samples.iter().filter_map(|sample| sample.speed_kmh)),
        avg_speed_kmh: average_speed(distance_km, duration_s),
        max_power_kw: max_finite(samples.iter().filter_map(|sample| sample.power_kw)),
        min_power_kw: min_finite(samples.iter().filter_map(|sample| sample.power_kw)),
        elevation_gain_m,
        elevation_loss_m,
        inside_temp_avg_c: mean(samples.iter().filter_map(|sample| sample.inside_temp_c)),
        outside_temp_avg_c: mean(samples.iter().filter_map(|sample| sample.outside_temp_c)),
    })
}

/// Closed charge totals.
#[derive(Clone, Debug)]
pub(crate) struct ChargeSummary {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub energy_added_kwh: Option<f64>,
    pub start_battery_level: Option<f64>,
    pub end_battery_level: Option<f64>,
    pub start_rated_range_km: Option<f64>,
    pub end_rated_range_km: Option<f64>,
    pub start_ideal_range_km: Option<f64>,
    pub end_ideal_range_km: Option<f64>,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub is_dc: Option<bool>,
    pub charger_type: Option<String>,
    pub max_power_kw: Option<f64>,
    pub duration_s: i32,
    pub outside_temp_avg_c: Option<f64>,
}

pub(crate) fn summarize_charge(samples: &[Sample]) -> Option<ChargeSummary> {
    let first = samples.first()?;
    let last = samples.last()?;
    let (latitude, longitude) = first_coord(samples);
    let is_dc = if samples
        .iter()
        .any(|sample| sample.fast_charger == Some(true))
    {
        Some(true)
    } else if samples
        .iter()
        .any(|sample| sample.fast_charger == Some(false))
    {
        Some(false)
    } else {
        None
    };
    let charger_type = samples
        .iter()
        .find_map(|sample| sanitize_charger_type(sample.fast_charger_type.as_deref()));
    Some(ChargeSummary {
        start: first.at,
        end: last.at,
        energy_added_kwh: max_finite(
            samples
                .iter()
                .filter_map(|sample| sample.charge_energy_added_kwh),
        ),
        start_battery_level: first_finite(samples.iter().filter_map(|sample| sample.battery_level)),
        end_battery_level: last_finite(samples.iter().filter_map(|sample| sample.battery_level)),
        start_rated_range_km: first_finite(
            samples.iter().filter_map(|sample| sample.rated_range_km),
        ),
        end_rated_range_km: last_finite(samples.iter().filter_map(|sample| sample.rated_range_km)),
        start_ideal_range_km: first_finite(
            samples.iter().filter_map(|sample| sample.ideal_range_km),
        ),
        end_ideal_range_km: last_finite(samples.iter().filter_map(|sample| sample.ideal_range_km)),
        latitude,
        longitude,
        is_dc,
        charger_type,
        max_power_kw: max_finite(samples.iter().filter_map(|sample| sample.charger_power_kw)),
        duration_s: duration_s(first.at, last.at),
        outside_temp_avg_c: mean(samples.iter().filter_map(|sample| sample.outside_temp_c)),
    })
}

pub(crate) fn range_energy(
    start_km: Option<f64>,
    end_km: Option<f64>,
    efficiency_kwh_per_km: Option<f64>,
) -> Option<f64> {
    match (start_km, end_km, efficiency_kwh_per_km) {
        (Some(start), Some(end), Some(efficiency))
            if start >= end && efficiency.is_finite() && efficiency > 0.0 =>
        {
            Some((start - end) * efficiency)
        }
        _ => None,
    }
}

fn odometer_distance(start: Option<f64>, end: Option<f64>) -> Option<f64> {
    match (start, end) {
        (Some(start), Some(end)) if end >= start && (end - start) <= 2_000.0 => Some(end - start),
        _ => None,
    }
}

fn path_distance_km(samples: &[Sample]) -> f64 {
    let mut total = 0.0;
    let mut previous: Option<(f64, f64)> = None;
    for sample in samples {
        let Some(latitude) = sample.latitude else {
            continue;
        };
        let Some(longitude) = sample.longitude else {
            continue;
        };
        if let Some((prev_lat, prev_lon)) = previous {
            total += haversine_m(prev_lat, prev_lon, latitude, longitude) / 1_000.0;
        }
        previous = Some((latitude, longitude));
    }
    total
}

fn elevation_change(samples: &[Sample]) -> (Option<f64>, Option<f64>) {
    let mut gain = 0.0;
    let mut loss = 0.0;
    let mut previous = None;
    let mut any = false;
    for sample in samples {
        let Some(elevation) = sample.elevation_m.filter(|value| value.is_finite()) else {
            continue;
        };
        if let Some(prior) = previous {
            let delta: f64 = elevation - prior;
            // Ignore sub-metre noise and impossible GPS spikes.
            if (0.5..100.0).contains(&delta) {
                gain += delta;
            } else if (-100.0..-0.5).contains(&delta) {
                loss += -delta;
            }
        }
        previous = Some(elevation);
        any = true;
    }
    if any {
        (Some(gain), Some(loss))
    } else {
        (None, None)
    }
}

fn first_coord(samples: &[Sample]) -> (Option<f64>, Option<f64>) {
    samples
        .iter()
        .find_map(|sample| Some((sample.latitude?, sample.longitude?)))
        .unzip_opt()
}

fn last_coord(samples: &[Sample]) -> (Option<f64>, Option<f64>) {
    samples
        .iter()
        .rev()
        .find_map(|sample| Some((sample.latitude?, sample.longitude?)))
        .unzip_opt()
}

trait UnzipOpt {
    fn unzip_opt(self) -> (Option<f64>, Option<f64>);
}

impl UnzipOpt for Option<(f64, f64)> {
    fn unzip_opt(self) -> (Option<f64>, Option<f64>) {
        match self {
            Some((latitude, longitude)) => (Some(latitude), Some(longitude)),
            None => (None, None),
        }
    }
}

fn duration_s(start: DateTime<Utc>, end: DateTime<Utc>) -> i32 {
    let seconds = (end - start).num_seconds().max(0);
    i32::try_from(seconds).unwrap_or(i32::MAX)
}

fn average_speed(distance_km: Option<f64>, duration_s: i32) -> Option<f64> {
    let distance = distance_km.filter(|value| value.is_finite() && *value >= 0.0)?;
    if duration_s <= 0 {
        return None;
    }
    Some(distance / (f64::from(duration_s) / 3_600.0))
}

fn first_finite(values: impl Iterator<Item = f64>) -> Option<f64> {
    values.into_iter().find(|value| value.is_finite())
}

fn last_finite(values: impl Iterator<Item = f64>) -> Option<f64> {
    values.into_iter().filter(|value| value.is_finite()).last()
}

fn max_finite(values: impl Iterator<Item = f64>) -> Option<f64> {
    values
        .into_iter()
        .filter(|value| value.is_finite())
        .max_by(f64::total_cmp)
}

fn min_finite(values: impl Iterator<Item = f64>) -> Option<f64> {
    values
        .into_iter()
        .filter(|value| value.is_finite())
        .min_by(f64::total_cmp)
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let mut count = 0u32;
    let mut sum = 0.0;
    for value in values.filter(|value| value.is_finite()) {
        sum += value;
        count += 1;
    }
    (count > 0).then_some(sum / f64::from(count))
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};

    use super::{
        Activity, ClosedDrive, Observation, Presence, Sample, Tracker, Transition,
        gate_stream_sample, keep_closed_drive, plan_drive_close, reduce, sanitize_charger_type,
        should_store_stream_position, stream_collides_with_poll, stream_drive_decision,
        summarize_charge, summarize_drive,
    };

    fn odometer_sample(minute: i64, odometer_km: Option<f64>, latitude: f64) -> Sample {
        let mut obs = online(minute);
        obs.odometer_km = odometer_km;
        obs.latitude = Some(latitude);
        obs.longitude = Some(2.35);
        obs.speed_kmh = Some(20.0);
        obs.sample()
    }

    fn at(minute: i64) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 6, 1, 10, 0, 0).unwrap() + Duration::minutes(minute)
    }

    fn online(minute: i64) -> Observation {
        Observation::at(at(minute), Presence::Online)
    }

    fn plugged(
        latch: Option<&str>,
        state: Option<&str>,
        cable: Option<&str>,
        door_open: Option<bool>,
        power_kw: Option<f64>,
    ) -> Option<bool> {
        super::derive_plugged_in(latch, state, cable, door_open, power_kw)
    }

    #[test]
    fn disconnected_is_unplugged_whatever_the_latch_says() {
        for latch in ["Engaged", "engaged", "Blocking", "Disengaged", ""] {
            let latch = (!latch.is_empty()).then_some(latch);
            assert_eq!(
                plugged(
                    latch,
                    Some("Disconnected"),
                    Some("<invalid>"),
                    Some(false),
                    Some(0.0)
                ),
                Some(false),
                "{latch:?}"
            );
            assert_eq!(
                plugged(latch, Some("disconnected"), None, Some(true), Some(11.0)),
                Some(false),
                "{latch:?} with the port open and power flowing"
            );
        }
        assert_eq!(
            plugged(Some("Engaged"), Some(" DISCONNECTED "), None, None, None),
            Some(false)
        );
        assert_eq!(
            plugged(Some("Engaged"), Some("none"), None, Some(true), Some(7.0)),
            Some(false)
        );
        assert_eq!(
            plugged(Some("Engaged"), Some("null"), None, None, None),
            Some(false)
        );
    }

    #[test]
    fn plugged_charging_states_win_over_the_latch_and_a_closed_door() {
        for state in ["Charging", "Starting", "Complete", "Stopped", "NoPower"] {
            for latch in [None, Some("Engaged"), Some("Disengaged"), Some("Blocking")] {
                assert_eq!(
                    plugged(latch, Some(state), None, Some(false), Some(0.0)),
                    Some(true),
                    "{state} latch={latch:?}"
                );
            }
            assert_eq!(
                plugged(None, Some(state), Some("<invalid>"), None, None),
                Some(true),
                "{state}"
            );
        }
        assert_eq!(
            plugged(
                Some("Disengaged"),
                Some("charging"),
                Some("<invalid>"),
                Some(false),
                Some(0.0)
            ),
            Some(true)
        );
    }

    #[test]
    fn missing_or_unknown_charging_state_uses_the_latch_unless_the_door_is_shut() {
        assert_eq!(plugged(Some("Engaged"), None, None, None, None), Some(true));
        assert_eq!(
            plugged(Some("Blocking"), Some(""), None, Some(true), Some(0.0)),
            Some(true)
        );
        assert_eq!(
            plugged(Some("Engaged"), Some("Unknown"), None, None, None),
            Some(true),
            "unknown charging state falls through to the latch"
        );
        assert_eq!(
            plugged(None, Some("Unknown"), None, Some(false), Some(0.0)),
            None
        );
        assert_eq!(
            plugged(Some("Engaged"), Some("Foo"), None, Some(false), Some(0.0)),
            Some(true),
            "any other charging state still means plugged in"
        );
        assert_eq!(
            plugged(Some("Engaged"), None, None, Some(false), Some(0.0)),
            Some(false),
            "a closed door and no power is the door latch, not a cable"
        );
        assert_eq!(
            plugged(Some("Blocking"), Some("  "), None, Some(false), Some(0.0)),
            Some(false)
        );
        assert_eq!(
            plugged(
                Some("ENGAGED"),
                Some("unknown"),
                None,
                Some(false),
                Some(-0.0)
            ),
            Some(false)
        );
        assert_eq!(
            plugged(Some("Engaged"), None, None, Some(false), None),
            Some(true),
            "power must be known to be zero"
        );
        assert_eq!(
            plugged(Some("Engaged"), None, None, None, Some(0.0)),
            Some(true),
            "the door must be known to be closed"
        );
        assert_eq!(
            plugged(Some("Engaged"), None, None, Some(false), Some(0.4)),
            Some(true)
        );
        assert_eq!(
            plugged(Some("Engaged"), None, Some("IEC"), Some(false), Some(0.0)),
            Some(true),
            "a named cable is not the latch alone"
        );
        assert_eq!(
            plugged(Some("Disengaged"), None, None, Some(false), Some(0.0)),
            Some(false)
        );
        assert_eq!(plugged(None, None, None, Some(false), Some(0.0)), None);
        assert_eq!(
            plugged(
                Some("Disengaged"),
                Some("Disconnected"),
                Some("SAE"),
                Some(false),
                Some(0.0)
            ),
            Some(true),
            "a named cable still means plugged in"
        );
    }

    #[test]
    fn refresh_plugged_in_matches_a_disconnected_poll() {
        let mut obs = online(0);
        obs.charging_state = Some("Disconnected".to_owned());
        obs.charge_port_latch = Some("Engaged".to_owned());
        obs.charge_port_door_open = Some(false);
        obs.charger_power_kw = Some(0.0);
        obs.conn_charge_cable = Some("<invalid>".to_owned());
        super::refresh_plugged_in(&mut obs);
        assert_eq!(obs.plugged_in, Some(false));

        let mut door = online(1);
        door.charge_port_latch = Some("Engaged".to_owned());
        door.charge_port_door_open = Some(false);
        door.charger_power_kw = Some(0.0);
        super::refresh_plugged_in(&mut door);
        assert_eq!(door.plugged_in, Some(false));
    }

    #[test]
    fn synthetic_drive_then_charge_then_sleep() {
        let mut tracker = Tracker::default();
        let mut steps = Vec::new();

        let mut asleep = Observation::at(at(0), Presence::Asleep);
        asleep.battery_level = Some(80.0);
        let (next, events) = reduce(&tracker, &asleep);
        assert_eq!(events, []);
        assert_eq!(next.activity, Activity::Asleep);
        tracker = next;

        let mut parked = online(1);
        parked.shift_state = Some("P".to_owned());
        parked.odometer_km = Some(100.0);
        let (next, events) = reduce(&tracker, &parked);
        assert_eq!(events, []);
        assert_eq!(next.activity, Activity::Online);
        tracker = next;

        let mut depart = online(2);
        depart.shift_state = Some("D".to_owned());
        depart.odometer_km = Some(100.0);
        depart.speed_kmh = Some(20.0);
        let (next, events) = reduce(&tracker, &depart);
        assert_eq!(events, vec![Transition::DriveStarted]);
        tracker = next;
        steps.push(depart.sample());

        let mut cruise = online(12);
        cruise.shift_state = Some("D".to_owned());
        cruise.odometer_km = Some(108.0);
        cruise.speed_kmh = Some(90.0);
        cruise.power_kw = Some(40.0);
        cruise.elevation_m = Some(120.0);
        cruise.rated_range_km = Some(400.0);
        cruise.battery_level = Some(78.0);
        let (next, events) = reduce(&tracker, &cruise);
        assert_eq!(events, []);
        assert_eq!(next.activity, Activity::Driving);
        tracker = next;
        steps.push(cruise.sample());

        let mut arrive = online(22);
        arrive.shift_state = Some("P".to_owned());
        arrive.odometer_km = Some(112.0);
        arrive.speed_kmh = Some(0.0);
        arrive.elevation_m = Some(150.0);
        arrive.rated_range_km = Some(390.0);
        arrive.battery_level = Some(74.0);
        let (next, events) = reduce(&tracker, &arrive);
        assert!(events.is_empty(), "park waits for the park grace");
        assert!(next.drive_open);
        tracker = next;
        steps.push(arrive.sample());

        let summary = summarize_drive(&steps, Some(0.16)).unwrap();
        assert!((summary.distance_km.unwrap() - 12.0).abs() < 1e-6);
        assert_eq!(summary.duration_s, 20 * 60);
        assert!((summary.max_speed_kmh.unwrap() - 90.0).abs() < 1e-6);
        assert!((summary.energy_used_kwh.unwrap() - 1.6).abs() < 1e-9);
        assert!((summary.elevation_gain_m.unwrap() - 30.0).abs() < 1e-6);
        assert_eq!(summary.start_battery_level, Some(78.0));
        assert_eq!(summary.end_battery_level, Some(74.0));

        assert_eq!(
            super::derive_plugged_in(
                Some("Engaged"),
                Some("Disconnected"),
                Some("<invalid>"),
                Some(false),
                Some(0.0),
            ),
            Some(false)
        );
        assert_eq!(
            super::derive_plugged_in(
                Some("Disengaged"),
                Some("Disconnected"),
                Some("SAE"),
                None,
                None,
            ),
            Some(true)
        );
        assert_eq!(
            super::derive_plugged_in(None, Some("Charging"), None, None, None),
            Some(true)
        );
        assert_eq!(
            super::derive_plugged_in(
                Some("Disengaged"),
                Some("Disconnected"),
                Some("<invalid>"),
                None,
                None,
            ),
            Some(false)
        );
        assert_eq!(
            super::derive_plugged_in(Some("Disengaged"), None, None, None, None),
            Some(false)
        );
        assert_eq!(super::derive_plugged_in(None, None, None, None, None), None);
        assert_eq!(super::soc_level(None, Some(70.0)), Some(70.0));
        assert_eq!(super::soc_level(Some(80.0), Some(79.0)), Some(80.0));

        let mut charging = online(23);
        charging.charging_state = Some("Charging".to_owned());
        charging.charge_energy_added_kwh = Some(1.0);
        charging.charger_power_kw = Some(11.0);
        charging.battery_level = Some(75.0);
        charging.fast_charger = Some(false);
        let (next, events) = reduce(&tracker, &charging);
        assert_eq!(
            events,
            vec![Transition::DriveEnded, Transition::ChargeStarted]
        );
        assert!(!next.drive_open);
        tracker = next;

        let mut done = online(83);
        done.charging_state = Some("Disconnected".to_owned());
        done.charge_energy_added_kwh = Some(18.5);
        done.charger_power_kw = Some(0.0);
        done.battery_level = Some(90.0);
        let (next, events) = reduce(&tracker, &done);
        assert_eq!(events, vec![Transition::ChargeEnded]);
        tracker = next;
        let charge = summarize_charge(&[charging.sample(), done.sample()]).unwrap();
        assert!((charge.energy_added_kwh.unwrap() - 18.5).abs() < 1e-9);
        assert_eq!(charge.is_dc, Some(false));
        assert_eq!(charge.end_battery_level, Some(90.0));

        let sleep = Observation::at(at(90), Presence::Asleep);
        let (next, events) = reduce(&tracker, &sleep);
        assert_eq!(events, []);
        assert_eq!(next.activity, Activity::Asleep);
    }

    #[test]
    fn sleep_does_not_close_an_open_drive_and_speed_without_shift_counts() {
        let mut rolling = online(0);
        rolling.speed_kmh = Some(15.0);
        let (tracker, events) = reduce(&Tracker::default(), &rolling);
        assert_eq!(events, vec![Transition::DriveStarted]);

        let sleep = Observation::at(at(5), Presence::Asleep);
        let (tracker, events) = reduce(&tracker, &sleep);
        assert_eq!(events, []);
        assert_eq!(tracker.activity, Activity::Asleep);
        assert!(tracker.drive_open);
    }

    fn clock_at(minute: i64) -> super::DriveClock {
        super::DriveClock {
            last_driving_at: at(minute),
            park_since: None,
        }
    }

    #[test]
    fn ten_second_gap_at_speed_stays_one_drive() {
        let grace = super::DriveGrace::standard();
        let started = at(0);
        let gap = plan_drive_close(
            super::DriveMotion::Gap,
            started + Duration::seconds(10),
            clock_at(0),
            grace,
            None,
        );
        assert!(matches!(gap, super::ClosePlan::Wait { .. }));
        let resumed = plan_drive_close(
            super::DriveMotion::Driving,
            started + Duration::seconds(10),
            clock_at(0),
            grace,
            Some(30.0),
        );
        assert!(matches!(resumed, super::ClosePlan::Continue { .. }));
    }

    #[test]
    fn five_minutes_at_zero_closes_and_a_shorter_stop_does_not() {
        let grace = super::DriveGrace::standard();
        let park_at = at(0);
        let clock = super::DriveClock {
            last_driving_at: park_at - Duration::seconds(30),
            park_since: Some(park_at),
        };
        let early = plan_drive_close(
            super::DriveMotion::Park,
            park_at + Duration::seconds(240),
            clock,
            grace,
            Some(0.0),
        );
        assert!(matches!(early, super::ClosePlan::Wait { .. }));
        let stopped = plan_drive_close(
            super::DriveMotion::Park,
            park_at + Duration::seconds(300),
            clock,
            grace,
            Some(0.0),
        );
        assert!(matches!(stopped, super::ClosePlan::Close { .. }));
        let custom = super::DriveGrace {
            end: Duration::seconds(300),
            park: Duration::seconds(120),
        };
        let custom_wait = plan_drive_close(
            super::DriveMotion::Park,
            park_at + Duration::seconds(119),
            clock,
            custom,
            Some(0.0),
        );
        assert!(matches!(custom_wait, super::ClosePlan::Wait { .. }));
        let at_threshold = plan_drive_close(
            super::DriveMotion::Park,
            park_at + Duration::seconds(120),
            clock,
            custom,
            Some(0.0),
        );
        assert!(matches!(at_threshold, super::ClosePlan::Close { .. }));
        let missing = plan_drive_close(super::DriveMotion::Park, park_at, clock_at(0), grace, None);
        assert!(matches!(
            missing,
            super::ClosePlan::Continue { clock: held } if held.park_since.is_none()
        ));
        let super::ClosePlan::Continue {
            clock: resumed_clock,
        } = plan_drive_close(
            super::DriveMotion::Driving,
            park_at + Duration::seconds(60),
            clock,
            grace,
            Some(12.0),
        )
        else {
            panic!("movement must clear the zero run");
        };
        assert!(resumed_clock.park_since.is_none());
        let after_move = plan_drive_close(
            super::DriveMotion::Gap,
            park_at + Duration::seconds(120),
            resumed_clock,
            grace,
            None,
        );
        assert!(
            matches!(after_move, super::ClosePlan::Wait { .. }),
            "a stop shorter than the threshold must not close the drive"
        );

        let parked = super::GapFacts {
            gap_secs: 204,
            end_speed_kmh: Some(0.0),
            start_speed_kmh: Some(30.0),
            end_odometer_km: Some(50.0),
            start_odometer_km: Some(50.0),
            gap_m: Some(0.0),
        };
        assert!(!super::split_candidate(&parked, 300));
        assert!(!super::should_reopen_drive(&parked, 300));
    }

    #[test]
    fn asleep_beyond_five_minutes_closes_and_charge_closes_at_once() {
        let grace = super::DriveGrace::standard();
        let driving_at = at(0);
        let brief = plan_drive_close(
            super::DriveMotion::Gap,
            driving_at + Duration::seconds(299),
            clock_at(0),
            grace,
            None,
        );
        assert!(matches!(brief, super::ClosePlan::Wait { .. }));
        let asleep = plan_drive_close(
            super::DriveMotion::Gap,
            driving_at + Duration::seconds(301),
            clock_at(0),
            grace,
            None,
        );
        assert!(matches!(asleep, super::ClosePlan::Close { .. }));
        let charging = plan_drive_close(
            super::DriveMotion::Charging,
            driving_at + Duration::seconds(5),
            clock_at(0),
            grace,
            None,
        );
        assert!(matches!(charging, super::ClosePlan::Close { .. }));
    }

    #[test]
    fn moving_gap_can_reopen_and_a_huge_odometer_jump_cannot() {
        let moving = super::GapFacts {
            gap_secs: 10,
            end_speed_kmh: Some(61.0),
            start_speed_kmh: Some(71.0),
            end_odometer_km: Some(100.0),
            start_odometer_km: Some(100.18),
            gap_m: Some(176.0),
        };
        assert!(super::split_candidate(&moving, 300));
        assert!(super::should_reopen_drive(&moving, 300));

        let jump = super::GapFacts {
            gap_secs: 10,
            end_speed_kmh: Some(61.0),
            start_speed_kmh: Some(71.0),
            end_odometer_km: Some(100.0),
            start_odometer_km: Some(110.0),
            gap_m: Some(176.0),
        };
        assert!(!super::should_reopen_drive(&jump, 300));
        assert!(!super::split_candidate(&jump, 300));
    }

    fn cue(
        at: chrono::DateTime<Utc>,
        speed_kmh: Option<f64>,
        shift_state: Option<&str>,
    ) -> super::PositionCue {
        super::PositionCue {
            at,
            speed_kmh,
            shift_state: shift_state.map(str::to_owned),
        }
    }

    #[test]
    fn drive_end_cutoff_trims_to_the_first_zero_of_the_final_run() {
        let start = at(0);
        let moving_at = at(1);
        let first_zero = moving_at + Duration::milliseconds(752);
        let before_zero = moving_at + Duration::milliseconds(200);
        let park = moving_at + Duration::milliseconds(1_910);
        let missing = park + Duration::seconds(30);
        let later_zero = moving_at + Duration::seconds(244);
        let cues = vec![
            cue(later_zero, Some(0.0), Some("P")),
            cue(missing, None, None),
            cue(park, Some(0.0), Some("P")),
            cue(first_zero, Some(0.0), None),
            cue(before_zero, None, None),
            cue(moving_at, Some(1.6), Some("D")),
        ];
        assert_eq!(super::drive_end_cutoff(&cues, start), first_zero);
        assert_eq!(
            super::plan_drive_end_repair(&cues, start, later_zero),
            Some(first_zero)
        );
    }

    #[test]
    fn drive_end_cutoff_keeps_the_end_when_the_tail_has_no_zero_speed() {
        let start = at(0);
        let moving = cue(at(1), Some(40.0), Some("D"));
        let parked = cue(at(2), None, Some("P"));
        let newest = cue(at(3), None, Some("P"));
        let cues = vec![newest.clone(), parked, moving];
        assert_eq!(
            super::drive_end_cutoff(&cues, start),
            newest.at,
            "missing speed is not a 0 km/h sample"
        );
    }

    #[test]
    fn drive_end_cutoff_does_not_cut_at_a_zero_followed_by_movement() {
        let start = at(0);
        let moving = cue(at(1), Some(40.0), Some("D"));
        let zero = cue(at(2), Some(0.0), Some("P"));
        let again = cue(at(3), Some(35.0), Some("D"));
        let cues = vec![again.clone(), zero, moving];
        assert_eq!(super::drive_end_cutoff(&cues, start), again.at);
    }

    #[test]
    fn drive_end_cutoff_uses_the_first_of_several_trailing_zeros() {
        let start = at(0);
        let moving = cue(at(1), Some(40.0), Some("D"));
        let first = cue(at(2), Some(0.0), Some("P"));
        let middle = cue(at(3), None, None);
        let last = cue(at(4), Some(0.0), Some("P"));
        let cues = vec![last, middle, first.clone(), moving];
        assert_eq!(super::drive_end_cutoff(&cues, start), first.at);
    }

    #[test]
    fn plan_drive_end_repair_moves_the_end_to_the_first_trailing_zero() {
        let start = at(0);
        let moving = cue(at(1), Some(40.0), Some("D"));
        let zero = cue(at(2), Some(0.0), Some("P"));
        let after = cue(at(3), None, None);
        let cues = vec![after.clone(), zero.clone(), moving];
        assert_eq!(
            super::plan_drive_end_repair(&cues, start, after.at),
            Some(zero.at)
        );
    }

    #[test]
    fn plan_drive_end_repair_leaves_a_tail_without_zero_speed() {
        let start = at(0);
        let moving = cue(at(1), Some(40.0), Some("D"));
        let newest = cue(at(2), Some(30.0), Some("D"));
        let cues = vec![newest.clone(), moving];
        assert_eq!(super::plan_drive_end_repair(&cues, start, newest.at), None);
    }

    #[test]
    fn plan_drive_end_repair_does_not_cut_a_zero_followed_by_movement() {
        let start = at(0);
        let moving = cue(at(1), Some(40.0), Some("D"));
        let zero = cue(at(2), Some(0.0), Some("P"));
        let again = cue(at(3), Some(35.0), Some("D"));
        let cues = vec![again.clone(), zero, moving];
        assert_eq!(super::plan_drive_end_repair(&cues, start, again.at), None);
    }

    #[test]
    fn plan_drive_end_repair_uses_the_first_of_several_trailing_zeros() {
        let start = at(0);
        let moving = cue(at(1), Some(40.0), Some("D"));
        let first = cue(at(2), Some(0.0), Some("P"));
        let middle = cue(at(3), Some(0.0), None);
        let last = cue(at(4), Some(0.0), Some("P"));
        let cues = vec![last.clone(), middle, first.clone(), moving.clone()];
        assert_eq!(
            super::plan_drive_end_repair(&cues, start, last.at),
            Some(first.at)
        );
        assert_eq!(
            super::plan_drive_end_repair(&cues, start, first.at),
            Some(first.at),
            "later zeros are still attached, so the end is rewritten to the first"
        );
        let already = vec![first.clone(), moving];
        assert_eq!(
            super::plan_drive_end_repair(&already, start, first.at),
            None
        );
    }

    #[test]
    fn drive_end_cutoff_keeps_a_moving_or_unknown_tail() {
        let drive_start = at(0);
        let moving_only = vec![cue(at(4), Some(30.0), Some("D"))];
        assert_eq!(
            super::drive_end_cutoff(&moving_only, drive_start),
            moving_only[0].at
        );

        let unknown_speed = vec![cue(at(5), None, None), cue(at(4), None, None)];
        assert_eq!(
            super::drive_end_cutoff(&unknown_speed, drive_start),
            unknown_speed[0].at,
            "missing speed must not invent a zero-speed tail"
        );
    }

    #[test]
    fn restart_clock_uses_the_last_park_or_driving_position() {
        let start = at(0);
        let driving = super::PositionCue {
            at: start,
            speed_kmh: Some(61.0),
            shift_state: Some("D".to_owned()),
        };
        let park = super::PositionCue {
            at: start + Duration::seconds(40),
            speed_kmh: Some(0.0),
            shift_state: Some("P".to_owned()),
        };
        let (clock, motion) = super::clock_from_tail(&[park.clone(), driving.clone()], start);
        assert_eq!(motion, super::DriveMotion::Park);
        assert_eq!(clock.park_since, Some(park.at));
        assert_eq!(clock.last_driving_at, driving.at);

        let (silent, motion) = super::clock_from_tail(std::slice::from_ref(&driving), start);
        assert_eq!(motion, super::DriveMotion::Gap);
        assert_eq!(silent.last_driving_at, driving.at);
        assert_eq!(silent.park_since, None);

        let newer = super::DriveClock {
            last_driving_at: start + Duration::seconds(15),
            park_since: None,
        };
        let merged = super::merge_clock(silent, Some(newer));
        assert_eq!(merged.last_driving_at, newer.last_driving_at);
        assert_eq!(merged.park_since, None);
    }

    #[test]
    fn starting_opens_a_charge_and_stopped_does_not_keep_it_charging() {
        let mut starting = online(0);
        starting.charging_state = Some("Starting".to_owned());
        let (tracker, events) = reduce(&Tracker::default(), &starting);
        assert_eq!(events, vec![Transition::ChargeStarted]);
        assert_eq!(tracker.activity, Activity::Charging);
        assert!(tracker.charge_open);

        let mut charging = online(1);
        charging.charging_state = Some("Charging".to_owned());
        let (tracker, events) = reduce(&tracker, &charging);
        assert_eq!(events, []);
        assert!(tracker.charge_open);
        assert_eq!(tracker.activity, Activity::Charging);

        let mut stopped = online(2);
        stopped.charging_state = Some("Stopped".to_owned());
        let (tracker, events) = reduce(&tracker, &stopped);
        assert_eq!(events, vec![Transition::ChargeEnded]);
        assert!(!tracker.charge_open);
        assert_eq!(tracker.activity, Activity::Online);
    }

    #[test]
    fn software_install_is_updating_until_the_car_moves() {
        let mut installing = online(0);
        installing.update_status = Some("installing".to_owned());
        let (tracker, events) = reduce(&Tracker::default(), &installing);
        assert_eq!(events, []);
        assert_eq!(tracker.activity, Activity::Updating);

        let mut depart = online(1);
        depart.shift_state = Some("D".to_owned());
        depart.update_status = Some("installing".to_owned());
        let (tracker, events) = reduce(&tracker, &depart);
        assert_eq!(events, vec![Transition::DriveStarted]);
        assert_eq!(tracker.activity, Activity::Driving);
    }

    #[test]
    fn dc_charge_records_the_peak_power() {
        let mut start = online(0);
        start.charging_state = Some("Starting".to_owned());
        start.fast_charger = Some(true);
        start.fast_charger_type = Some("Combo".to_owned());
        start.charger_power_kw = Some(40.0);
        start.charge_energy_added_kwh = Some(0.2);
        let mut peak = online(10);
        peak.charging_state = Some("Charging".to_owned());
        peak.fast_charger = Some(true);
        peak.charger_power_kw = Some(150.0);
        peak.charge_energy_added_kwh = Some(22.0);
        let summary = summarize_charge(&[start.sample(), peak.sample()]).unwrap();
        assert_eq!(summary.is_dc, Some(true));
        assert_eq!(summary.charger_type.as_deref(), Some("Combo"));
        assert!((summary.max_power_kw.unwrap() - 150.0).abs() < 1e-9);
    }

    #[test]
    fn open_charge_with_invalid_charger_type_ends_with_none() {
        let mut charging = online(0);
        charging.charging_state = Some("Charging".to_owned());
        charging.fast_charger = Some(false);
        charging.fast_charger_type = Some("<invalid>".to_owned());
        charging.charger_power_kw = Some(7.4);
        charging.charge_energy_added_kwh = Some(1.2);

        let mut still = online(20);
        still.charging_state = Some("Charging".to_owned());
        still.fast_charger = Some(false);
        still.fast_charger_type = Some("<INVALID>".to_owned());
        still.charger_power_kw = Some(7.4);
        still.charge_energy_added_kwh = Some(4.0);

        let summary = summarize_charge(&[charging.sample(), still.sample()]).unwrap();
        assert_eq!(summary.charger_type, None);
    }

    #[test]
    fn sanitize_charger_type_rejects_placeholders() {
        assert_eq!(sanitize_charger_type(None), None);
        assert_eq!(sanitize_charger_type(Some("")), None);
        assert_eq!(sanitize_charger_type(Some("   ")), None);
        assert_eq!(sanitize_charger_type(Some("<invalid>")), None);
        assert_eq!(sanitize_charger_type(Some(" <INVALID> ")), None);
        assert_eq!(sanitize_charger_type(Some("<none>")), None);
        assert_eq!(sanitize_charger_type(Some("none")), None);
        assert_eq!(sanitize_charger_type(Some("NULL")), None);
        assert_eq!(sanitize_charger_type(Some(" Null ")), None);
        assert_eq!(
            sanitize_charger_type(Some("Combo")).as_deref(),
            Some("Combo")
        );
        assert_eq!(
            sanitize_charger_type(Some("  Tesla  ")).as_deref(),
            Some("Tesla")
        );
    }

    #[test]
    fn stream_shift_keeps_an_open_drive_and_a_charge() {
        let parked = Tracker {
            activity: Activity::Online,
            drive_open: false,
            charge_open: false,
        };
        let started = stream_drive_decision(&parked, Some("D"), Some(0.0));
        assert!(started.start_drive);
        assert!(started.driving);
        assert_eq!(started.activity, Activity::Driving);

        let driving = Tracker {
            activity: Activity::Driving,
            drive_open: true,
            charge_open: false,
        };
        let held = stream_drive_decision(&driving, None, None);
        assert!(!held.end_drive);
        assert!(held.driving);
        let ended = stream_drive_decision(&driving, Some("P"), Some(0.0));
        assert!(ended.end_drive);
        assert_eq!(ended.motion, super::DriveMotion::Park);
        assert!(!ended.driving);
        let grace = super::DriveGrace::standard();
        let waited = plan_drive_close(ended.motion, at(0), clock_at(0), grace, Some(0.0));
        assert!(matches!(waited, super::ClosePlan::Wait { .. }));
        assert_eq!(ended.activity, Activity::Online);

        let charging = Tracker {
            activity: Activity::Charging,
            drive_open: false,
            charge_open: true,
        };
        let still = stream_drive_decision(&charging, Some("P"), Some(0.0));
        assert!(!still.end_charge);
        assert_eq!(still.activity, Activity::Charging);
        let depart = stream_drive_decision(&charging, None, Some(5.0));
        assert!(depart.start_drive);
        assert!(depart.end_charge);

        let updating = Tracker {
            activity: Activity::Updating,
            drive_open: false,
            charge_open: false,
        };
        assert_eq!(
            stream_drive_decision(&updating, None, None).activity,
            Activity::Updating
        );

        let poll = at(0);
        assert!(stream_collides_with_poll(poll, poll));
        assert!(stream_collides_with_poll(poll + Duration::seconds(2), poll));
        assert!(!stream_collides_with_poll(
            poll + Duration::seconds(3),
            poll
        ));
        assert!(should_store_stream_position(
            true,
            false,
            Some(48.1),
            Some(11.5)
        ));
        assert!(!should_store_stream_position(
            false,
            false,
            Some(48.1),
            Some(11.5)
        ));
        assert!(should_store_stream_position(
            false,
            true,
            Some(48.1),
            Some(11.5)
        ));
        assert!(!should_store_stream_position(
            true,
            false,
            Some(0.0),
            Some(0.0)
        ));
        assert!(!should_store_stream_position(true, false, None, Some(11.5)));
    }

    #[test]
    fn late_samples_attach_to_the_closed_drive_and_do_not_open() {
        let start = Utc.with_ymd_and_hms(2024, 11, 10, 12, 9, 0).unwrap();
        let end = start + Duration::minutes(10);
        let closed = ClosedDrive {
            id: 7,
            start_at: start,
            end_at: end,
        };
        let now = Utc::now();
        let inside = gate_stream_sample(start + Duration::minutes(2), now, Some(closed));
        assert!(!inside.allow_open);
        assert_eq!(inside.attach_closed, Some(7));
        let before = gate_stream_sample(start - Duration::seconds(10), now, Some(closed));
        assert!(!before.allow_open);
        assert_eq!(before.attach_closed, Some(7));
        let ancient = gate_stream_sample(start - Duration::hours(2), now, Some(closed));
        assert!(!ancient.allow_open);
        assert_eq!(ancient.attach_closed, None);
        let stale = gate_stream_sample(now - Duration::seconds(45), now, Some(closed));
        assert!(!stale.allow_open);
        assert_eq!(stale.attach_closed, None);
        let fresh = gate_stream_sample(now, now, Some(closed));
        assert!(fresh.allow_open);
        assert_eq!(fresh.attach_closed, None);
    }

    #[test]
    fn one_position_is_not_a_drive() {
        let samples = vec![odometer_sample(0, Some(100.0), 48.85)];
        assert!(!keep_closed_drive(&samples));
    }

    #[test]
    fn two_positions_with_zero_odometer_delta_are_not_a_drive() {
        // Far apart on the map. The filter uses the odometer, not GPS.
        let samples = vec![
            odometer_sample(0, Some(100.0), 48.0),
            odometer_sample(1, Some(100.0), 49.5),
        ];
        assert!(!keep_closed_drive(&samples));
    }

    #[test]
    fn odometer_delta_under_ten_metres_is_not_a_drive() {
        let samples = vec![
            odometer_sample(0, Some(100.0), 48.85),
            odometer_sample(1, Some(100.009), 48.86),
        ];
        assert!(!keep_closed_drive(&samples));
    }

    #[test]
    fn odometer_delta_of_ten_metres_with_two_positions_is_a_drive() {
        let samples = vec![
            odometer_sample(0, Some(100.0), 48.85),
            odometer_sample(1, Some(100.01), 48.86),
        ];
        assert!(keep_closed_drive(&samples));
        // 400.01 is not an exact float. The threshold is still the decimal 0.01 km.
        let large = vec![
            odometer_sample(0, Some(400.0), 48.85),
            odometer_sample(1, Some(400.01), 48.86),
        ];
        assert!(keep_closed_drive(&large));
    }

    #[test]
    fn missing_odometer_is_not_a_drive() {
        let samples = vec![
            odometer_sample(0, None, 48.0),
            odometer_sample(1, Some(110.0), 49.0),
        ];
        assert!(!keep_closed_drive(&samples));
    }
}
