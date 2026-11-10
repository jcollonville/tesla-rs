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
    /// Minutes after midnight. This is the Fleet API unit.
    pub scheduled_charging_start_time: Option<i32>,
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
/// Latch `Engaged` or `Blocking` wins, then a real `conn_charge_cable`, then
/// `charging_state`. `Disconnected` means unplugged. An explicit `Disengaged`
/// latch with no other signal is unplugged. Missing inputs stay unknown.
pub(crate) fn derive_plugged_in(
    charge_port_latch: Option<&str>,
    charging_state: Option<&str>,
    conn_charge_cable: Option<&str>,
) -> Option<bool> {
    if latch_is(charge_port_latch, &["engaged", "blocking"]) {
        return Some(true);
    }
    if cable_is_connected(conn_charge_cable) {
        return Some(true);
    }
    if let Some(state) = nonempty(charging_state) {
        return Some(!is_disconnected(state));
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

fn is_disconnected(state: &str) -> bool {
    matches!(
        state.to_ascii_lowercase().as_str(),
        "disconnected" | "none" | "null"
    )
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|text| !text.is_empty())
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
/// or when speed is above 1 km/h and no park shift is reported. It ends on park,
/// on a charge session, or when the car leaves `online`. A charge starts on
/// `Charging` or `Starting` and ends on any other charging state, including sleep.
/// The reducer never asks the car to wake.
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

    if tracker.drive_open {
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

/// Drive boundaries implied by one Owner API streaming sample.
///
/// An unknown shift keeps the current drive open. Streaming frames often omit
/// `shift_state` when the value did not change, and that must not close the drive.
/// A charge stays open until a drive actually starts. Fleet polling remains the
/// source that notices sleep.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StreamDrive {
    pub start_drive: bool,
    pub end_drive: bool,
    pub end_charge: bool,
    pub activity: Activity,
    pub driving: bool,
}

pub(crate) fn stream_drive_decision(
    tracker: &Tracker,
    shift: Option<&str>,
    speed_kmh: Option<f64>,
) -> StreamDrive {
    let known = match normalized_shift(shift) {
        Some("D" | "R" | "N") => Some(true),
        Some("P") => Some(false),
        _ => match speed_kmh {
            Some(speed) if speed.is_finite() && speed > 1.0 => Some(true),
            _ => None,
        },
    };
    let driving = known.unwrap_or(tracker.drive_open);
    let start_drive = driving && !tracker.drive_open;
    let end_drive = !driving && tracker.drive_open;
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
        Activity, Observation, Presence, Tracker, Transition, reduce, sanitize_charger_type,
        should_store_stream_position, stream_collides_with_poll, stream_drive_decision,
        summarize_charge, summarize_drive,
    };

    fn at(minute: i64) -> chrono::DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 6, 1, 10, 0, 0).unwrap() + Duration::minutes(minute)
    }

    fn online(minute: i64) -> Observation {
        Observation::at(at(minute), Presence::Online)
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
        assert_eq!(events, vec![Transition::DriveEnded]);
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
            super::derive_plugged_in(Some("Engaged"), Some("Disconnected"), Some("<invalid>")),
            Some(true)
        );
        assert_eq!(
            super::derive_plugged_in(Some("Disengaged"), Some("Disconnected"), Some("SAE")),
            Some(true)
        );
        assert_eq!(
            super::derive_plugged_in(None, Some("Charging"), None),
            Some(true)
        );
        assert_eq!(
            super::derive_plugged_in(Some("Disengaged"), Some("Disconnected"), Some("<invalid>")),
            Some(false)
        );
        assert_eq!(
            super::derive_plugged_in(Some("Disengaged"), None, None),
            Some(false)
        );
        assert_eq!(super::derive_plugged_in(None, None, None), None);
        assert_eq!(super::soc_level(None, Some(70.0)), Some(70.0));
        assert_eq!(super::soc_level(Some(80.0), Some(79.0)), Some(80.0));

        let mut charging = online(23);
        charging.charging_state = Some("Charging".to_owned());
        charging.charge_energy_added_kwh = Some(1.0);
        charging.charger_power_kw = Some(11.0);
        charging.battery_level = Some(75.0);
        charging.fast_charger = Some(false);
        let (next, events) = reduce(&tracker, &charging);
        assert_eq!(events, vec![Transition::ChargeStarted]);
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
    fn sleep_closes_an_open_drive_and_speed_without_shift_counts() {
        let mut rolling = online(0);
        rolling.speed_kmh = Some(15.0);
        let (tracker, events) = reduce(&Tracker::default(), &rolling);
        assert_eq!(events, vec![Transition::DriveStarted]);

        let sleep = Observation::at(at(5), Presence::Asleep);
        let (tracker, events) = reduce(&tracker, &sleep);
        assert_eq!(events, vec![Transition::DriveEnded]);
        assert_eq!(tracker.activity, Activity::Asleep);
        assert!(!tracker.drive_open);
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
}
