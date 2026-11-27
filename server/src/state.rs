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
    /// The last cable reading is paused or missing. Not a claim that the cable is out.
    Unknown,
}

impl CableStatus {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Charging => "charging",
            Self::Plugged => "plugged",
            Self::Unplugged => "unplugged",
            Self::Unknown => "unknown",
        }
    }
}

/// Cable state published on the API.
///
/// A paused `vehicle_data` budget must not turn a missing reading into
/// `unplugged`. A known charging or plugged reading is kept and marked stale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ChargePublication {
    pub status: Option<CableStatus>,
    pub stale: bool,
}

pub(crate) fn publish_charge(
    charging_state: Option<&str>,
    plugged_in: Option<bool>,
    reading_fresh: bool,
) -> ChargePublication {
    let known = cable_status(charging_state, plugged_in);
    if reading_fresh {
        return ChargePublication {
            status: known,
            stale: false,
        };
    }
    let status = match known {
        Some(CableStatus::Charging | CableStatus::Plugged) => known,
        Some(CableStatus::Unplugged | CableStatus::Unknown) | None => Some(CableStatus::Unknown),
    };
    ChargePublication {
        status,
        stale: true,
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
/// A charge starts on `Charging` or `Starting` and ends on any other known charging
/// state, on sleep, or when a drive starts. A reading without a charging state (a
/// vehicle-list cycle, a telemetry disconnect) says nothing about the cable, so an
/// open charge stays open. The reducer never asks the car to wake.
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
        if charge_continues(obs) {
            next.activity = match obs.presence {
                Presence::Online => Activity::Charging,
                Presence::Asleep => Activity::Asleep,
                Presence::Offline => Activity::Offline,
            };
            return (next, events);
        }
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

/// An open charge survives a reading that does not report a charging state.
///
/// A car does not sleep while it charges, so `Asleep` ends the charge.
fn charge_continues(obs: &Observation) -> bool {
    obs.presence != Presence::Asleep && nonempty(obs.charging_state.as_deref()).is_none()
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
/// Default time at exact 0 km/h with an unknown gear before a drive closes.
pub(crate) const DRIVE_PARK_GRACE_DEFAULT_SECS: u64 = 300;
/// Shortest accepted unknown-gear park grace.
pub(crate) const DRIVE_PARK_GRACE_MIN_SECS: u64 = 15;
/// Longest accepted unknown-gear park grace.
pub(crate) const DRIVE_PARK_GRACE_MAX_SECS: u64 = 900;
/// Default time in shift `P` at standstill before a drive closes.
pub(crate) const DRIVE_P_CONFIRM_DEFAULT_SECS: u64 = 60;
/// Shortest accepted park confirmation.
pub(crate) const DRIVE_P_CONFIRM_MIN_SECS: u64 = 15;
/// Longest accepted park confirmation.
pub(crate) const DRIVE_P_CONFIRM_MAX_SECS: u64 = 300;
/// Default time stopped in `D`, `R`, or `N` before a drive closes.
pub(crate) const DRIVE_STOP_IN_GEAR_DEFAULT_SECS: u64 = 1_800;
/// Shortest accepted in-gear stop. Kept above the old five-minute split.
pub(crate) const DRIVE_STOP_IN_GEAR_MIN_SECS: u64 = 300;
/// Longest accepted in-gear stop.
pub(crate) const DRIVE_STOP_IN_GEAR_MAX_SECS: u64 = 7_200;
/// `D` or `R` this soon after a park close, at the same place, is the same trip.
pub(crate) const PARK_REOPEN_SECS: i64 = 120;
/// Coherent odometer and position may rejoin a drive across a restart or a gap.
///
/// Fifteen minutes covers a ten-minute process gap plus ingest lag. A flat
/// odometer at the same place does not qualify: that stays the two-minute
/// park resume. Past this window a later departure is a new drive.
pub(crate) const CONTINUITY_JOIN_SECS: i64 = 900;
/// A 0 km/h sample this long before the `P` transition may be the stored end.
pub(crate) const ZERO_BEFORE_PARK_SECS: i64 = 120;

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
    /// Exact 0 km/h when the gear is unknown.
    pub park: chrono::Duration,
    /// Shift `P` with speed at or below 1 km/h, or unknown.
    pub p_confirm: chrono::Duration,
    /// Stopped in `D`, `R`, or `N`. Does not apply to a confirmed park.
    pub stop_in_gear: chrono::Duration,
}

impl DriveGrace {
    /// Built-in windows: 300 s signal loss, 300 s unknown-gear zero, 60 s in `P`, 1800 s in gear.
    pub(crate) const fn standard() -> Self {
        Self {
            end: chrono::Duration::seconds(300),
            park: chrono::Duration::seconds(300),
            p_confirm: chrono::Duration::seconds(60),
            stop_in_gear: chrono::Duration::seconds(1_800),
        }
    }

    /// All four windows, already checked against their bounds.
    pub(crate) fn from_config(
        end_secs: u64,
        park_secs: u64,
        p_confirm_secs: u64,
        stop_in_gear_secs: u64,
    ) -> Self {
        Self {
            end: chrono::Duration::seconds(i64::try_from(end_secs).unwrap_or(300)),
            park: chrono::Duration::seconds(i64::try_from(park_secs).unwrap_or(300)),
            p_confirm: chrono::Duration::seconds(i64::try_from(p_confirm_secs).unwrap_or(60)),
            stop_in_gear: chrono::Duration::seconds(
                i64::try_from(stop_in_gear_secs)
                    .unwrap_or(i64::try_from(DRIVE_STOP_IN_GEAR_DEFAULT_SECS).unwrap_or(1_800)),
            ),
        }
    }
}

/// What one sample says about an open drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DriveMotion {
    /// Shift `D`/`R`/`N` while moving or with unknown speed, or speed above 1 km/h.
    Driving,
    /// Shift `P` and speed at or below 1 km/h, or unknown.
    Park,
    /// Shift `D`/`R`/`N` with a known speed at or below 1 km/h.
    StoppedInGear,
    /// Exact 0 km/h and no gear. Uses [`DriveGrace::park`].
    UnknownStandstill,
    /// Charging or starting to charge. Closes the drive at once.
    Charging,
    /// Asleep, offline, incomplete poll, or park while still moving.
    Gap,
    /// Sample with no shift and no moving speed. Keeps the open drive.
    Hold,
}

/// Which stop timer [`DriveClock::park_since`] belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopKind {
    /// Confirmed park. [`DriveGrace::p_confirm`].
    Park,
    /// Unknown gear at exact 0 km/h. [`DriveGrace::park`].
    Unknown,
    /// Held in `D`, `R`, or `N`. [`DriveGrace::stop_in_gear`].
    InGear,
}

/// Last driving evidence and the start of the current stop run.
///
/// Instants come from position timestamps, so a restart can rebuild this from
/// stored rows. [`Self::park_since`] is never older than [`Self::last_driving_at`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DriveClock {
    /// Time of the latest sample that was still driving.
    pub last_driving_at: DateTime<Utc>,
    /// Car timestamp of the latest accepted message, driving or not.
    pub last_message_at: DateTime<Utc>,
    /// First sample of the current stop run. Never older than [`Self::last_driving_at`].
    pub park_since: Option<DateTime<Utc>>,
    /// Which grace [`Self::park_since`] uses.
    pub stop_kind: Option<StopKind>,
}

impl DriveClock {
    /// A drive that is moving at `at`, with no stop timer.
    pub(crate) fn moving_at(at: DateTime<Utc>) -> Self {
        Self {
            last_driving_at: at,
            last_message_at: at,
            park_since: None,
            stop_kind: None,
        }
    }
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

/// A telemetry fix repeats the previous location and gear, so it is not a new point.
pub(crate) fn telemetry_point_repeats(previous: Option<&Observation>, next: &Observation) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if previous.shift_state != next.shift_state {
        return false;
    }
    let Some((left_lat, left_lon)) = previous.latitude.zip(previous.longitude) else {
        return false;
    };
    let Some((right_lat, right_lon)) = next.latitude.zip(next.longitude) else {
        return false;
    };
    left_lat.is_finite()
        && left_lon.is_finite()
        && right_lat.is_finite()
        && right_lon.is_finite()
        && (left_lat - right_lat).abs() < 1e-5
        && (left_lon - right_lon).abs() < 1e-5
}

/// Open drive, car still marked driving, and no telemetry message for 90 seconds.
pub(crate) fn telemetry_drive_needs_fallback(
    drive_open: bool,
    activity: Activity,
    age: Option<std::time::Duration>,
) -> bool {
    drive_open
        && activity == Activity::Driving
        && age.is_some_and(|age| age >= std::time::Duration::from_secs(90))
}

/// Speed is present and about zero, or missing next to an explicit park shift.
pub(crate) fn speed_is_standstill(speed: Option<f64>) -> bool {
    match speed {
        Some(speed) if speed.is_finite() => speed <= STANDSTILL_KMH,
        Some(_) => false,
        None => true,
    }
}

/// Known speed at or below 1 km/h. A missing reading is not a stop.
fn known_standstill(speed: Option<f64>) -> bool {
    match speed {
        Some(speed) if speed.is_finite() => speed <= STANDSTILL_KMH,
        _ => false,
    }
}

/// Classifies a poll or telemetry observation.
pub(crate) fn observation_motion(obs: &Observation) -> DriveMotion {
    if is_charging(obs) {
        return DriveMotion::Charging;
    }
    if obs.presence == Presence::Online {
        match normalized_shift(obs.shift_state.as_deref()) {
            Some("D" | "R" | "N") if known_standstill(obs.speed_kmh) => {
                return DriveMotion::StoppedInGear;
            }
            Some("D" | "R" | "N") => return DriveMotion::Driving,
            Some("P") if speed_is_standstill(obs.speed_kmh) => return DriveMotion::Park,
            Some("P") => return DriveMotion::Gap,
            _ if obs
                .speed_kmh
                .is_some_and(|speed| speed.is_finite() && speed > STANDSTILL_KMH) =>
            {
                return DriveMotion::Driving;
            }
            _ if speed_is_exact_zero(obs.speed_kmh) => return DriveMotion::UnknownStandstill,
            _ => {}
        }
    }
    if is_driving(obs) {
        return DriveMotion::Driving;
    }
    DriveMotion::Gap
}

/// Decides whether the open drive should close at `at`.
///
/// `at` is a car timestamp. Charging closes immediately. Shift `P` at standstill
/// (speed missing counts) starts [`DriveGrace::p_confirm`]. `D`/`R`/`N` at a
/// known standstill uses [`DriveGrace::stop_in_gear`] and does not start the
/// park timer. Unknown gear at exact 0 km/h keeps [`DriveGrace::park`]. A sample
/// older than the last accepted message does not move `park_since` backward.
/// Signal loss uses [`DriveGrace::end`] from the previous car timestamp.
/// Row `end_at` is chosen later by [`drive_end_cutoff`].
pub(crate) fn plan_drive_close(
    motion: DriveMotion,
    at: DateTime<Utc>,
    clock: DriveClock,
    grace: DriveGrace,
    speed_kmh: Option<f64>,
) -> ClosePlan {
    plan_drive_close_at(motion, at, clock, grace, speed_kmh, true)
}

/// Wall-clock check used by [`crate::persist::close_drive_if_due`].
///
/// `at` is `now` and is not stored as a car message, so a quiet expire tick
/// cannot refresh the silence anchor.
pub(crate) fn plan_drive_expire(
    motion: DriveMotion,
    at: DateTime<Utc>,
    clock: DriveClock,
    grace: DriveGrace,
) -> ClosePlan {
    plan_drive_close_at(motion, at, clock, grace, None, false)
}

fn plan_drive_close_at(
    motion: DriveMotion,
    at: DateTime<Utc>,
    mut clock: DriveClock,
    grace: DriveGrace,
    speed_kmh: Option<f64>,
    record_message: bool,
) -> ClosePlan {
    if motion == DriveMotion::Charging {
        return ClosePlan::Close { clock };
    }
    let anchor = clock.last_message_at.max(clock.last_driving_at);
    if record_message && at + chrono::Duration::seconds(1) < anchor {
        return ClosePlan::Continue { clock };
    }
    if motion == DriveMotion::Hold && !speed_is_exact_zero(speed_kmh) {
        if record_message && at > clock.last_message_at {
            clock.last_message_at = at;
        }
        return ClosePlan::Continue { clock };
    }
    let stop = match motion {
        DriveMotion::Park => Some((StopKind::Park, grace.p_confirm)),
        DriveMotion::StoppedInGear => Some((StopKind::InGear, grace.stop_in_gear)),
        DriveMotion::UnknownStandstill => Some((StopKind::Unknown, grace.park)),
        _ => None,
    };
    if let Some((kind, window)) = stop {
        let since = if clock.stop_kind == Some(kind) {
            clock.park_since.unwrap_or(at)
        } else {
            at
        };
        let since = since.max(clock.last_driving_at);
        clock.park_since = Some(since);
        clock.stop_kind = Some(kind);
        if record_message && at > clock.last_message_at {
            clock.last_message_at = at;
        }
        return grace_or_wait(at, since, window, clock);
    }
    if motion == DriveMotion::Driving || speed_is_movement(speed_kmh) {
        let late_park = clock
            .park_since
            .is_some_and(|since| at + chrono::Duration::seconds(1) < since);
        if late_park {
            return ClosePlan::Continue { clock };
        }
        clock.last_driving_at = clock.last_driving_at.max(at);
        clock.park_since = None;
        clock.stop_kind = None;
        if record_message {
            clock.last_message_at = clock.last_message_at.max(at);
        }
        return ClosePlan::Continue { clock };
    }
    if motion == DriveMotion::Gap {
        let proof = clock.last_message_at.max(clock.last_driving_at);
        if record_message && at > clock.last_message_at {
            clock.last_message_at = at;
        }
        if let Some(since) = clock.park_since
            && let Some(kind) = clock.stop_kind
        {
            let window = match kind {
                StopKind::Park => grace.p_confirm,
                StopKind::Unknown => grace.park,
                StopKind::InGear => grace.stop_in_gear,
            };
            if matches!(
                grace_or_wait(at, since, window, clock),
                ClosePlan::Close { .. }
            ) {
                return ClosePlan::Close { clock };
            }
        }
        return grace_or_wait(at, proof, grace.end, clock);
    }
    if record_message && at > clock.last_message_at {
        clock.last_message_at = at;
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
    let last_message = cues.first().map_or(drive_start, |cue| cue.at);
    let last_driving = cues
        .iter()
        .find(|cue| position_is_later_movement(cue))
        .map_or(drive_start, |cue| cue.at);
    let (park_since, stop_kind, motion) = tail_stop(cues);
    let park_since = park_since.filter(|since| *since >= last_driving);
    (
        DriveClock {
            last_driving_at: last_driving,
            last_message_at: last_message.max(last_driving),
            park_since,
            stop_kind: park_since.and(stop_kind),
        },
        motion,
    )
}

/// Newest cue first. The stop is the final non-moving run, classified by gear.
fn tail_stop(cues: &[PositionCue]) -> (Option<DateTime<Utc>>, Option<StopKind>, DriveMotion) {
    let mut kind = None;
    let mut since = None;
    for cue in cues {
        if position_is_later_movement(cue) {
            break;
        }
        let sample_kind = match normalized_shift(cue.shift_state.as_deref()) {
            Some("P") if speed_is_standstill(cue.speed_kmh) => Some(StopKind::Park),
            Some("D" | "R" | "N") if known_standstill(cue.speed_kmh) => Some(StopKind::InGear),
            _ if speed_is_exact_zero(cue.speed_kmh) => Some(StopKind::Unknown),
            _ => None,
        };
        let Some(sample_kind) = sample_kind else {
            continue;
        };
        if kind.is_some_and(|current| current != sample_kind) {
            break;
        }
        kind = Some(sample_kind);
        since = Some(cue.at);
    }
    let motion = match kind {
        Some(StopKind::Park) => DriveMotion::Park,
        Some(StopKind::InGear) => DriveMotion::StoppedInGear,
        Some(StopKind::Unknown) => DriveMotion::UnknownStandstill,
        None => DriveMotion::Gap,
    };
    (since, kind, motion)
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
    if let Some(park_at) = confirmed_park_transition(cues) {
        let window = park_at - chrono::Duration::seconds(ZERO_BEFORE_PARK_SECS);
        let mut zero = None;
        for cue in cues {
            if cue.at > park_at {
                continue;
            }
            if cue.at < window || (cue.at < park_at && position_is_later_movement(cue)) {
                break;
            }
            if speed_is_exact_zero(cue.speed_kmh) {
                zero = Some(cue.at);
            }
        }
        return zero.unwrap_or(park_at);
    }
    final_zero_run_start(cues).unwrap_or(newest.at)
}

/// Earliest confirmed `P` in the final non-moving tail, newest cue first.
fn confirmed_park_transition(cues: &[PositionCue]) -> Option<DateTime<Utc>> {
    let mut saw_park = false;
    let mut start = None;
    for cue in cues {
        if position_is_later_movement(cue) {
            break;
        }
        match normalized_shift(cue.shift_state.as_deref()) {
            Some("P") if speed_is_standstill(cue.speed_kmh) => {
                saw_park = true;
                start = Some(cue.at);
            }
            Some("D" | "R" | "N") if saw_park => break,
            _ => {}
        }
    }
    saw_park.then_some(start).flatten()
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
        // Telemetry speed-only messages are not stored as positions, so memory is
        // often newer. A stop that began after that evidence still counts.
        let stop = memory
            .park_since
            .filter(|since| *since >= memory.last_driving_at)
            .zip(memory.stop_kind);
        return DriveClock {
            last_driving_at: memory.last_driving_at,
            last_message_at: memory.last_message_at.max(stored.last_message_at),
            park_since: stop.map(|(since, _)| since),
            stop_kind: stop.map(|(_, kind)| kind),
        };
    }
    if stored.park_since.is_none()
        && let Some(since) = memory.park_since
        && since >= stored.last_driving_at
    {
        return DriveClock {
            last_driving_at: stored.last_driving_at,
            last_message_at: stored.last_message_at.max(memory.last_message_at),
            park_since: Some(since),
            stop_kind: memory.stop_kind,
        };
    }
    DriveClock {
        last_message_at: stored.last_message_at.max(memory.last_message_at),
        ..stored
    }
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
    if split_candidate(gap, grace_secs) && odometer_delta(gap).is_some() {
        return true;
    }
    if same_place_after_park(gap) {
        return true;
    }
    continuous_resume(gap)
}

/// `D`/`R` within two minutes of a park close, still at that place.
fn same_place_after_park(gap: &GapFacts) -> bool {
    if gap.gap_secs < 0 || gap.gap_secs > PARK_REOPEN_SECS {
        return false;
    }
    let end_slow = gap
        .end_speed_kmh
        .is_none_or(|speed| !speed.is_finite() || speed <= STANDSTILL_KMH);
    let close = gap
        .gap_m
        .is_some_and(|metres| metres.is_finite() && metres < SAME_PLACE_M);
    end_slow && close && odometer_plausible(gap)
}

/// The earlier drive was still moving and the odometer stayed plausible.
fn continuous_resume(gap: &GapFacts) -> bool {
    if gap.gap_secs < 0 || gap.gap_secs > CONTINUITY_JOIN_SECS {
        return false;
    }
    if !speed_above(gap.end_speed_kmh, GAP_MOTION_KMH) {
        return false;
    }
    odometer_plausible(gap) && moving_across_gap(gap) && odometer_delta(gap).is_some()
}

/// Signal-gap closes wait out the continuity window after this process started.
///
/// A restart otherwise sees `now` far ahead of the last car timestamp and splits
/// a drive that is about to resume. Park, in-gear, and charge closes still run:
/// those timers come from stored samples, not from the process being down.
pub(crate) fn gap_close_suppressed(now: DateTime<Utc>, process_started: DateTime<Utc>) -> bool {
    let since = now.signed_duration_since(process_started);
    since >= chrono::Duration::zero() && since < chrono::Duration::seconds(CONTINUITY_JOIN_SECS)
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

/// A drive that has already been closed. Late telemetry samples may join it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ClosedDrive {
    pub id: i64,
    pub start_at: DateTime<Utc>,
    pub end_at: DateTime<Utc>,
    /// Retained end point. Address and geofence use this, not a later sample.
    pub end_latitude: Option<f64>,
    pub end_longitude: Option<f64>,
}

/// Samples older than this at processing time must not open a drive.
pub(crate) const STALE_LIVE_OPEN: chrono::Duration = chrono::Duration::seconds(30);

/// How far before a closed drive a late sample may still belong to it.
pub(crate) const CLOSED_DRIVE_SLACK: chrono::Duration = chrono::Duration::seconds(30);

/// Whether one live sample may open a drive, and which closed drive it joins.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LiveSampleGate {
    pub allow_open: bool,
    pub attach_closed: Option<i64>,
}

/// Gates a live (telemetry) sample against the last closed drive and the processing clock.
///
/// A sample at or before that drive's `end_at`, or older than [`STALE_LIVE_OPEN`],
/// must not open a drive. A sample inside the drive, or up to [`CLOSED_DRIVE_SLACK`]
/// before it started, is attached to that drive instead.
pub(crate) fn gate_live_sample(
    at: DateTime<Utc>,
    now: DateTime<Utc>,
    last_closed: Option<ClosedDrive>,
) -> LiveSampleGate {
    let stale = now.signed_duration_since(at) > STALE_LIVE_OPEN;
    let covers_closed = last_closed.is_some_and(|closed| at <= closed.end_at);
    let attach_closed = last_closed.and_then(|closed| {
        let earliest = closed.start_at - CLOSED_DRIVE_SLACK;
        (at >= earliest && at <= closed.end_at).then_some(closed.id)
    });
    LiveSampleGate {
        allow_open: !stale && !covers_closed,
        attach_closed,
    }
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
/// last known odometer minus the first known odometer (ordered by date) is at
/// least [`MIN_CLOSED_DRIVE_KM`]. Positions without an odometer are skipped: a
/// telemetry message that opens a drive may not carry one. No odometer at all
/// makes that distance NULL. GPS distance is not consulted.
pub(crate) fn keep_closed_drive(samples: &[Sample]) -> bool {
    let Some(start) = first_finite(samples.iter().filter_map(|sample| sample.odometer_km)) else {
        return false;
    };
    let Some(end) = last_finite(samples.iter().filter_map(|sample| sample.odometer_km)) else {
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

/// Odometer travel in a drive window that proves the car really moved.
pub(crate) const MOVEMENT_GUARD_KM: f64 = 0.1;

/// Speed that proves movement when no position in the window has an odometer.
pub(crate) const MOVEMENT_GUARD_KMH: f64 = 5.0;

/// Positions above [`MOVEMENT_GUARD_KMH`] needed when there is no odometer.
/// One fast sample alone can be noise or a single sparse poll.
pub(crate) const MOVEMENT_GUARD_SAMPLES: i64 = 2;

/// Whether a drive window shows real movement, so the drive must not be discarded.
///
/// The odometer wins when the window has one: a short odometer delta stays a
/// non-drive even if one sample reported a speed. Without any odometer, at
/// least [`MOVEMENT_GUARD_SAMPLES`] positions above [`MOVEMENT_GUARD_KMH`] count.
pub(crate) fn window_shows_movement(odometer_delta_km: Option<f64>, moving_samples: i64) -> bool {
    match odometer_delta_km.filter(|km| km.is_finite()) {
        Some(km) => km >= MOVEMENT_GUARD_KM,
        None => moving_samples >= MOVEMENT_GUARD_SAMPLES,
    }
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
    /// Battery-side counter (`charge_energy_added`, telemetry
    /// `DCChargingEnergyIn`). Without one, the integrated charger power.
    pub energy_added_kwh: Option<f64>,
    /// Integrated charger input power. AC only: DC power is not metered here.
    pub energy_used_kwh: Option<f64>,
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

/// Highest charger power, in kW, an on-board (AC) charger can draw.
///
/// Tesla on-board chargers top out at 22 kW (three-phase, some Model S/X).
const AC_MAX_POWER_KW: f64 = 22.0;

/// AC when the charge drew power and never more than an on-board charger can.
///
/// Used only when no sample says whether a fast charger was present, as with
/// telemetry charges recorded before the AC/DC flag was folded.
fn on_board_power(samples: &[Sample]) -> Option<bool> {
    let peak = max_finite(samples.iter().filter_map(|sample| sample.charger_power_kw))?;
    (peak > 0.0 && peak <= AC_MAX_POWER_KW).then_some(false)
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
        on_board_power(samples)
    };
    let charger_type = samples
        .iter()
        .find_map(|sample| sanitize_charger_type(sample.fast_charger_type.as_deref()));
    let counter = max_finite(
        samples
            .iter()
            .filter_map(|sample| sample.charge_energy_added_kwh),
    );
    let integrated = integrated_power_kwh(samples);
    Some(ChargeSummary {
        start: first.at,
        end: last.at,
        energy_added_kwh: counter.or(integrated),
        energy_used_kwh: if is_dc == Some(true) {
            None
        } else {
            integrated
        },
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

/// Longest pause between two charge samples that is still integrated.
///
/// Telemetry re-sends the state of charge every five minutes, so a longer
/// silence is a gap in the data, not a steady charge.
pub(crate) const CHARGE_POWER_GAP_MAX_SECS: i64 = 1_800;

/// Charger power integrated over time, holding each reading until the next.
///
/// `ACChargingPower` is published on change only, so the last value holds.
/// `None` when no interval could be integrated.
pub(crate) fn integrated_power_kwh(samples: &[Sample]) -> Option<f64> {
    let mut total = None;
    let mut held: Option<(DateTime<Utc>, f64)> = None;
    for sample in samples {
        if let Some((since, power)) = held {
            let millis = sample.at.signed_duration_since(since).num_milliseconds();
            if millis > 0 && millis <= CHARGE_POWER_GAP_MAX_SECS * 1_000 {
                #[allow(clippy::cast_precision_loss)]
                let hours = millis as f64 / 3_600_000.0;
                total = Some(total.unwrap_or(0.0) + power.max(0.0) * hours);
            }
        }
        let power = sample
            .charger_power_kw
            .filter(|power| power.is_finite())
            .or(held.map(|(_, power)| power));
        held = power.map(|power| (sample.at, power));
    }
    total
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
        Activity, ClosedDrive, DriveClock, Observation, Presence, Sample, StopKind, Tracker,
        Transition, drive_end_cutoff, gate_live_sample, keep_closed_drive, merge_clock,
        plan_drive_close, reduce, sanitize_charger_type, should_reopen_drive, summarize_charge,
        summarize_drive, window_shows_movement,
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
    fn paused_cable_reading_is_unknown_instead_of_unplugged() {
        let fresh = super::publish_charge(Some("Disconnected"), Some(false), true);
        assert_eq!(fresh.status, Some(super::CableStatus::Unplugged));
        assert!(!fresh.stale);
        let paused = super::publish_charge(Some("Disconnected"), Some(false), false);
        assert_eq!(paused.status, Some(super::CableStatus::Unknown));
        assert!(paused.stale);
        let still_charging = super::publish_charge(Some("Charging"), Some(true), false);
        assert_eq!(still_charging.status, Some(super::CableStatus::Charging));
        assert!(still_charging.stale);
        let missing = super::publish_charge(None, None, false);
        assert_eq!(missing.status, Some(super::CableStatus::Unknown));
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
        super::DriveClock::moving_at(at(minute))
    }

    fn stopped_clock(
        last_driving: chrono::DateTime<Utc>,
        park_at: chrono::DateTime<Utc>,
        kind: super::StopKind,
    ) -> super::DriveClock {
        super::DriveClock {
            last_driving_at: last_driving,
            last_message_at: park_at.max(last_driving),
            park_since: Some(park_at),
            stop_kind: Some(kind),
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
    fn confirmed_park_closes_after_sixty_seconds_not_five_minutes() {
        let grace = super::DriveGrace::standard();
        let park_at = at(0);
        let clock = stopped_clock(
            park_at - Duration::seconds(30),
            park_at,
            super::StopKind::Park,
        );
        let early = plan_drive_close(
            super::DriveMotion::Park,
            park_at + Duration::seconds(30),
            clock,
            grace,
            Some(0.0),
        );
        assert!(
            matches!(early, super::ClosePlan::Wait { .. }),
            "thirty seconds in P keeps the trip open"
        );
        let closed = plan_drive_close(
            super::DriveMotion::Park,
            park_at + Duration::seconds(60),
            clock,
            grace,
            Some(0.0),
        );
        assert!(
            matches!(closed, super::ClosePlan::Close { .. }),
            "P held for the confirm window closes, end time stays the P transition"
        );
        let unknown = plan_drive_close(
            super::DriveMotion::Park,
            park_at + Duration::seconds(60),
            super::DriveClock::moving_at(park_at),
            grace,
            None,
        );
        assert!(
            matches!(
                unknown,
                super::ClosePlan::Close { .. } | super::ClosePlan::Wait { .. }
            ),
            "P with unknown speed must start the confirm timer"
        );
        assert!(
            !matches!(unknown, super::ClosePlan::Continue { clock: held } if held.park_since.is_none()),
            "P with unknown speed must not ignore the park timer"
        );
    }

    #[test]
    fn drive_at_zero_for_ten_minutes_does_not_close() {
        let grace = super::DriveGrace::standard();
        let start = at(0);
        let mut stopped = super::Observation::at(start, super::Presence::Online);
        stopped.shift_state = Some("D".to_owned());
        stopped.speed_kmh = Some(0.0);
        let motion = super::observation_motion(&stopped);
        let first = plan_drive_close(motion, start, clock_at(0), grace, Some(0.0));
        let (super::ClosePlan::Continue { clock } | super::ClosePlan::Wait { clock, .. }) = first
        else {
            panic!("the first 0 km/h sample in D must not close");
        };
        let later = plan_drive_close(
            motion,
            start + Duration::seconds(600),
            clock,
            grace,
            Some(0.0),
        );
        assert!(
            !matches!(later, super::ClosePlan::Close { .. }),
            "ten minutes stopped in D is one trip"
        );
    }

    #[test]
    fn park_then_drive_within_thirty_seconds_continues() {
        let grace = super::DriveGrace::standard();
        let park_at = at(0);
        let clock = stopped_clock(
            park_at - Duration::seconds(20),
            park_at,
            super::StopKind::Park,
        );
        let resumed = plan_drive_close(
            super::DriveMotion::Driving,
            park_at + Duration::seconds(30),
            clock,
            grace,
            Some(18.0),
        );
        assert!(matches!(
            resumed,
            super::ClosePlan::Continue { clock: next } if next.park_since.is_none()
        ));
    }

    #[test]
    fn late_zero_does_not_set_park_since_before_the_last_drive() {
        let grace = super::DriveGrace::standard();
        let clock = super::DriveClock::moving_at(at(10));
        let late = plan_drive_close(super::DriveMotion::Park, at(0), clock, grace, Some(0.0));
        let held = match late {
            super::ClosePlan::Continue { clock }
            | super::ClosePlan::Wait { clock, .. }
            | super::ClosePlan::Close { clock } => clock,
        };
        assert!(
            held.park_since
                .is_none_or(|since| since >= held.last_driving_at),
            "park_since {held:?} is older than the last driving proof"
        );
    }

    #[test]
    fn same_place_departure_within_two_minutes_reopens() {
        let parked = super::GapFacts {
            gap_secs: 90,
            end_speed_kmh: Some(0.0),
            start_speed_kmh: Some(22.0),
            end_odometer_km: Some(50.0),
            start_odometer_km: Some(50.01),
            gap_m: Some(12.0),
        };
        assert!(should_reopen_drive(&parked, 300));
    }

    #[test]
    fn ten_minute_gap_rejoins_when_the_drive_was_still_moving() {
        let gap = super::GapFacts {
            gap_secs: 600,
            end_speed_kmh: Some(48.0),
            start_speed_kmh: Some(52.0),
            end_odometer_km: Some(100.0),
            start_odometer_km: Some(108.0),
            gap_m: Some(7_500.0),
        };
        assert!(should_reopen_drive(&gap, 300));
        let parked = super::GapFacts {
            gap_secs: 600,
            end_speed_kmh: Some(0.0),
            start_speed_kmh: Some(30.0),
            end_odometer_km: Some(100.0),
            start_odometer_km: Some(100.0),
            gap_m: Some(8.0),
        };
        assert!(!should_reopen_drive(&parked, 300));
    }

    #[test]
    fn end_cutoff_uses_the_park_transition_unless_zero_is_within_two_minutes() {
        let start = at(0);
        let p_at = at(10);
        let old_zero = p_at - Duration::minutes(3);
        let moving = old_zero - Duration::seconds(20);
        let long_gap = vec![
            cue(p_at, Some(0.0), Some("P")),
            cue(old_zero, Some(0.0), Some("D")),
            cue(moving, Some(40.0), Some("D")),
        ];
        assert_eq!(drive_end_cutoff(&long_gap, start), p_at);
        let near_zero = p_at - Duration::seconds(90);
        let close_stop = vec![
            cue(p_at, Some(0.0), Some("P")),
            cue(near_zero, Some(0.0), Some("D")),
            cue(near_zero - Duration::seconds(15), Some(36.0), Some("D")),
        ];
        assert_eq!(drive_end_cutoff(&close_stop, start), near_zero);
    }

    #[test]
    fn five_minutes_at_zero_closes_and_a_shorter_stop_does_not() {
        let grace = super::DriveGrace::standard();
        let park_at = at(0);
        let clock = stopped_clock(
            park_at - Duration::seconds(30),
            park_at,
            super::StopKind::Unknown,
        );
        let early = plan_drive_close(
            super::DriveMotion::UnknownStandstill,
            park_at + Duration::seconds(240),
            clock,
            grace,
            Some(0.0),
        );
        assert!(matches!(early, super::ClosePlan::Wait { .. }));
        let stopped = plan_drive_close(
            super::DriveMotion::UnknownStandstill,
            park_at + Duration::seconds(300),
            clock,
            grace,
            Some(0.0),
        );
        assert!(matches!(stopped, super::ClosePlan::Close { .. }));
        let mut custom = super::DriveGrace::standard();
        custom.park = Duration::seconds(120);
        let custom_wait = plan_drive_close(
            super::DriveMotion::UnknownStandstill,
            park_at + Duration::seconds(119),
            clock,
            custom,
            Some(0.0),
        );
        assert!(matches!(custom_wait, super::ClosePlan::Wait { .. }));
        let at_threshold = plan_drive_close(
            super::DriveMotion::UnknownStandstill,
            park_at + Duration::seconds(120),
            clock,
            custom,
            Some(0.0),
        );
        assert!(matches!(at_threshold, super::ClosePlan::Close { .. }));
        let missing = plan_drive_close(super::DriveMotion::Park, park_at, clock_at(0), grace, None);
        assert!(matches!(
            missing,
            super::ClosePlan::Wait { clock: held, .. }
                if held.park_since == Some(park_at) && held.stop_kind == Some(super::StopKind::Park)
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

    fn charging(minute: i64, soc: f64, power_kw: f64) -> Observation {
        let mut obs = online(minute);
        obs.charging_state = Some("Charging".to_owned());
        obs.charger_power_kw = Some(power_kw);
        obs.battery_level = Some(soc);
        obs
    }

    #[test]
    fn a_charge_stays_open_on_a_reading_without_a_charging_state() {
        let tracker = Tracker::default();
        let (open, events) = reduce(&tracker, &charging(0, 59.0, 7.0));
        assert_eq!(events, vec![Transition::ChargeStarted]);
        // The vehicle list says online and telemetry is fresh, so the poller
        // records a bare resting reading. It carries no charging state.
        let (still, events) = reduce(&open, &online(12));
        assert_eq!(events, [], "a list-only cycle must not end the charge");
        assert!(still.charge_open);
        assert_eq!(still.activity, Activity::Charging);
        let offline = Observation::at(at(13), Presence::Offline);
        let (still, events) = reduce(&still, &offline);
        assert_eq!(events, [], "a dropped connection must not end the charge");
        assert!(still.charge_open);
        let (again, events) = reduce(&still, &charging(14, 61.0, 7.0));
        assert_eq!(events, [], "no second charge is opened");
        assert!(again.charge_open);
    }

    #[test]
    fn a_charge_ends_on_a_known_state_or_sleep() {
        for state in ["Complete", "Stopped", "Disconnected", "NoPower"] {
            let (open, _) = reduce(&Tracker::default(), &charging(0, 59.0, 7.0));
            let mut done = online(5);
            done.charging_state = Some(state.to_owned());
            let (closed, events) = reduce(&open, &done);
            assert_eq!(events, vec![Transition::ChargeEnded], "{state}");
            assert!(!closed.charge_open);
        }
        let (open, _) = reduce(&Tracker::default(), &charging(0, 59.0, 7.0));
        let (closed, events) = reduce(&open, &Observation::at(at(5), Presence::Asleep));
        assert_eq!(events, vec![Transition::ChargeEnded]);
        assert!(!closed.charge_open);
    }

    #[test]
    fn charge_type_falls_back_to_the_charger_power() {
        // Telemetry AC charge: power only, no fast-charger flag.
        let ac = [
            charging(0, 59.0, 7.0).sample(),
            charging(5, 60.0, 7.2).sample(),
        ];
        assert_eq!(summarize_charge(&ac).unwrap().is_dc, Some(false));
        // Above any on-board charger: not called AC without a flag.
        let fast = [
            charging(0, 20.0, 120.0).sample(),
            charging(5, 30.0, 90.0).sample(),
        ];
        assert_eq!(summarize_charge(&fast).unwrap().is_dc, None);
        // No power at all: unknown.
        let mut idle = charging(0, 59.0, 0.0);
        idle.charger_power_kw = None;
        assert_eq!(summarize_charge(&[idle.sample()]).unwrap().is_dc, None);
        // An explicit flag wins.
        let mut flagged = charging(0, 20.0, 15.0);
        flagged.fast_charger = Some(true);
        assert_eq!(
            summarize_charge(&[flagged.sample()]).unwrap().is_dc,
            Some(true)
        );
    }

    #[test]
    fn charge_energy_comes_from_power_when_no_counter_is_reported() {
        // Telemetry: ACChargingPower at 7 kW for two hours, one reading every
        // five minutes, no charge_energy_added.
        let samples: Vec<Sample> = (0..=24)
            .map(|step| {
                charging(step * 5, 59.0 + f64::from(u8::try_from(step).unwrap()), 7.0).sample()
            })
            .collect();
        let charge = summarize_charge(&samples).unwrap();
        let kwh = charge.energy_added_kwh.expect("energy from power");
        assert!((kwh - 14.0).abs() < 1e-6, "{kwh}");
        // A gap longer than the cap is not integrated.
        let gapped = [
            charging(0, 59.0, 7.0).sample(),
            charging(120, 80.0, 7.0).sample(),
        ];
        assert_eq!(summarize_charge(&gapped).unwrap().energy_added_kwh, None);
        // A reported counter still wins.
        let mut with_counter = samples.clone();
        if let Some(last) = with_counter.last_mut() {
            last.charge_energy_added_kwh = Some(13.2);
        }
        let kwh = summarize_charge(&with_counter).unwrap().energy_added_kwh;
        assert_eq!(kwh, Some(13.2));
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
        let cues = vec![newest.clone(), parked.clone(), moving];
        assert_eq!(
            super::drive_end_cutoff(&cues, start),
            parked.at,
            "P with unknown speed ends at the park transition, not a later repeat"
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

        let newer = super::DriveClock::moving_at(start + Duration::seconds(15));
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
    fn late_samples_attach_to_the_closed_drive_and_do_not_open() {
        let start = Utc.with_ymd_and_hms(2024, 11, 10, 12, 9, 0).unwrap();
        let end = start + Duration::minutes(10);
        let closed = ClosedDrive {
            id: 7,
            start_at: start,
            end_at: end,
            end_latitude: None,
            end_longitude: None,
        };
        let now = Utc::now();
        let inside = gate_live_sample(start + Duration::minutes(2), now, Some(closed));
        assert!(!inside.allow_open);
        assert_eq!(inside.attach_closed, Some(7));
        let before = gate_live_sample(start - Duration::seconds(10), now, Some(closed));
        assert!(!before.allow_open);
        assert_eq!(before.attach_closed, Some(7));
        let ancient = gate_live_sample(start - Duration::hours(2), now, Some(closed));
        assert!(!ancient.allow_open);
        assert_eq!(ancient.attach_closed, None);
        let stale = gate_live_sample(now - Duration::seconds(45), now, Some(closed));
        assert!(!stale.allow_open);
        assert_eq!(stale.attach_closed, None);
        let fresh = gate_live_sample(now, now, Some(closed));
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

    #[test]
    fn a_leading_sample_without_odometer_does_not_discard_the_drive() {
        // Telemetry opened the drive before the car re-sent its odometer.
        let samples = vec![
            odometer_sample(0, None, 48.0),
            odometer_sample(1, Some(100.0), 48.5),
            odometer_sample(2, Some(101.9), 49.0),
        ];
        assert!(keep_closed_drive(&samples));
    }

    #[test]
    fn merge_clock_keeps_a_park_timer_newer_than_the_memory_driving_evidence() {
        let stored = DriveClock::moving_at(at(0));
        let memory = DriveClock {
            last_driving_at: at(2),
            last_message_at: at(3),
            park_since: Some(at(3)),
            stop_kind: Some(StopKind::Park),
        };
        let merged = merge_clock(stored, Some(memory));
        assert_eq!(merged.last_driving_at, at(2));
        assert_eq!(merged.park_since, Some(at(3)));
        assert_eq!(merged.stop_kind, Some(StopKind::Park));
    }

    #[test]
    fn merge_clock_drops_a_park_timer_older_than_the_driving_evidence() {
        let stored = DriveClock::moving_at(at(0));
        let memory = DriveClock {
            last_driving_at: at(4),
            last_message_at: at(4),
            park_since: Some(at(3)),
            stop_kind: Some(StopKind::Park),
        };
        let merged = merge_clock(stored, Some(memory));
        assert_eq!(merged.park_since, None);
        assert_eq!(merged.stop_kind, None);
    }

    #[test]
    fn movement_window_prefers_the_odometer_then_speed() {
        assert!(window_shows_movement(Some(0.1), 0));
        assert!(!window_shows_movement(Some(0.05), 40));
        assert!(window_shows_movement(None, 2));
        assert!(!window_shows_movement(None, 1));
        assert!(!window_shows_movement(Some(f64::NAN), 0));
    }
}
