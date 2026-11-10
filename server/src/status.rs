// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Published vehicle status.
//!
//! Owner stream samples only carry drive columns (speed, power, odometer, position,
//! heading, elevation, shift, state of charge, and range). Polling `vehicle_data`
//! is the source for everything else. The live frame is the merge of the two:
//! a newer stream sample overrides those drive fields, and a stream sample never
//! clears the last non-null poll field.

use crate::state::{Observation, Presence};

macro_rules! copy_some {
    ($dest:ident, $src:ident, $($field:ident),+ $(,)?) => {
        $(
            if $src.$field.is_some() {
                $dest.$field.clone_from(&$src.$field);
            }
        )+
    };
}

macro_rules! fill_none {
    ($dest:ident, $src:ident, $($field:ident),+ $(,)?) => {
        $(
            if $dest.$field.is_none() {
                $dest.$field.clone_from(&$src.$field);
            }
        )+
    };
}

/// Fold a `vehicle_data` observation onto the last published status.
///
/// Non-null fields from a poll at or after `current.at` replace the previous
/// value. An older poll only fills gaps. Nulls in the poll do not erase a
/// value that was already published.
pub(crate) fn merge_poll(current: Option<&Observation>, incoming: &Observation) -> Observation {
    let Some(current) = current else {
        return incoming.clone();
    };
    let mut out = current.clone();
    if incoming.at >= current.at {
        copy_present(&mut out, incoming);
        out.at = incoming.at;
        out.presence = incoming.presence;
    } else {
        fill_gaps(&mut out, incoming);
    }
    out
}

/// Fold one Owner stream sample onto the last published status.
///
/// Drive fields that are present override the published value when the sample
/// is at least as new as the published status. Climate, charge, and every other
/// poll-only field stay as they were. A sample older than the published status
/// is ignored.
pub(crate) fn merge_stream(current: Option<&Observation>, incoming: &Observation) -> Observation {
    let Some(current) = current else {
        return incoming.clone();
    };
    if incoming.at < current.at {
        return current.clone();
    }
    let mut out = current.clone();
    copy_stream(&mut out, incoming);
    out.at = incoming.at;
    out.presence = Presence::Online;
    out
}

fn copy_present(dest: &mut Observation, src: &Observation) {
    copy_some!(
        dest,
        src,
        shift_state,
        charging_state,
        speed_kmh,
        power_kw,
        odometer_km,
        battery_level,
        usable_battery_level,
        rated_range_km,
        ideal_range_km,
        est_range_km,
        latitude,
        longitude,
        elevation_m,
        heading_deg,
        inside_temp_c,
        outside_temp_c,
        charge_energy_added_kwh,
        charger_power_kw,
        charger_voltage,
        charger_current,
        fast_charger,
        fast_charger_type,
        charge_limit_soc,
        update_status,
        software_version,
        locked,
        sentry_mode,
        climate_on,
        charge_current_request,
        charge_port_door_open,
        charge_port_latch,
        conn_charge_cable,
        plugged_in,
        time_to_full_charge_h,
        scheduled_charging_start_time,
        tpms_pressure_fl,
        tpms_pressure_fr,
        tpms_pressure_rl,
        tpms_pressure_rr,
        df,
        dr,
        pf,
        pr,
        fd_window,
        fp_window,
        rd_window,
        rp_window,
        ft,
        rt,
        is_user_present,
        battery_heater_on,
        is_preconditioning,
        model,
        trim_badging,
        exterior_color,
        wheel_type,
        distance_unit,
        efficiency_kwh_per_km,
    );
}

fn fill_gaps(dest: &mut Observation, src: &Observation) {
    fill_none!(
        dest,
        src,
        shift_state,
        charging_state,
        speed_kmh,
        power_kw,
        odometer_km,
        battery_level,
        usable_battery_level,
        rated_range_km,
        ideal_range_km,
        est_range_km,
        latitude,
        longitude,
        elevation_m,
        heading_deg,
        inside_temp_c,
        outside_temp_c,
        charge_energy_added_kwh,
        charger_power_kw,
        charger_voltage,
        charger_current,
        fast_charger,
        fast_charger_type,
        charge_limit_soc,
        update_status,
        software_version,
        locked,
        sentry_mode,
        climate_on,
        charge_current_request,
        charge_port_door_open,
        charge_port_latch,
        conn_charge_cable,
        plugged_in,
        time_to_full_charge_h,
        scheduled_charging_start_time,
        tpms_pressure_fl,
        tpms_pressure_fr,
        tpms_pressure_rl,
        tpms_pressure_rr,
        df,
        dr,
        pf,
        pr,
        fd_window,
        fp_window,
        rd_window,
        rp_window,
        ft,
        rt,
        is_user_present,
        battery_heater_on,
        is_preconditioning,
        model,
        trim_badging,
        exterior_color,
        wheel_type,
        distance_unit,
        efficiency_kwh_per_km,
    );
}

/// Drive columns the Owner stream is allowed to publish over a poll.
fn copy_stream(dest: &mut Observation, src: &Observation) {
    copy_some!(
        dest,
        src,
        shift_state,
        speed_kmh,
        power_kw,
        odometer_km,
        battery_level,
        latitude,
        longitude,
        elevation_m,
        heading_deg,
        rated_range_km,
        est_range_km,
    );
}

/// Clock helper for merge tests.
#[cfg(test)]
pub(crate) fn at_secs(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(1_700_000_000 + seconds, 0).expect("timestamp")
}

#[cfg(test)]
mod tests {
    use super::{at_secs, merge_poll, merge_stream};
    use crate::state::{Observation, Presence};

    fn poll_at(seconds: i64) -> Observation {
        let mut obs = Observation::at(at_secs(seconds), Presence::Online);
        obs.inside_temp_c = Some(21.5);
        obs.outside_temp_c = Some(9.0);
        obs.charge_energy_added_kwh = Some(3.5);
        obs.charge_limit_soc = Some(80.0);
        obs.charging_state = Some("Disconnected".to_owned());
        obs.plugged_in = Some(false);
        obs.latitude = Some(48.1);
        obs.longitude = Some(2.2);
        obs.speed_kmh = Some(0.0);
        obs.power_kw = Some(0.0);
        obs.odometer_km = Some(100.0);
        obs
    }

    fn stream_at(seconds: i64) -> Observation {
        let mut obs = Observation::at(at_secs(seconds), Presence::Online);
        obs.latitude = Some(48.2);
        obs.longitude = Some(2.3);
        obs.speed_kmh = Some(42.0);
        obs.power_kw = Some(11.0);
        obs.odometer_km = Some(100.4);
        obs.elevation_m = Some(30.0);
        obs.heading_deg = Some(90.0);
        obs.shift_state = Some("D".to_owned());
        obs.battery_level = Some(77.0);
        obs
    }

    #[test]
    fn stream_after_poll_keeps_climate_and_charge_and_moves_position() {
        let poll = poll_at(0);
        let merged = merge_stream(Some(&poll), &stream_at(5));
        assert_eq!(merged.inside_temp_c, Some(21.5));
        assert_eq!(merged.outside_temp_c, Some(9.0));
        assert_eq!(merged.charge_energy_added_kwh, Some(3.5));
        assert_eq!(merged.charge_limit_soc, Some(80.0));
        assert_eq!(merged.charging_state.as_deref(), Some("Disconnected"));
        assert_eq!(merged.plugged_in, Some(false));
        assert_eq!(merged.latitude, Some(48.2));
        assert_eq!(merged.longitude, Some(2.3));
        assert_eq!(merged.speed_kmh, Some(42.0));
        assert_eq!(merged.power_kw, Some(11.0));
        assert_eq!(merged.odometer_km, Some(100.4));
        assert_eq!(merged.elevation_m, Some(30.0));
        assert_eq!(merged.heading_deg, Some(90.0));
        assert_eq!(merged.shift_state.as_deref(), Some("D"));
        assert_eq!(merged.battery_level, Some(77.0));
    }

    #[test]
    fn newer_poll_updates_climate_and_charge() {
        let poll = poll_at(0);
        let after_stream = merge_stream(Some(&poll), &stream_at(5));
        let mut newer = poll_at(10);
        newer.inside_temp_c = Some(22.0);
        newer.outside_temp_c = Some(8.0);
        newer.charge_energy_added_kwh = Some(4.0);
        newer.charge_limit_soc = Some(90.0);
        newer.charging_state = Some("Charging".to_owned());
        newer.plugged_in = Some(true);
        let merged = merge_poll(Some(&after_stream), &newer);
        assert_eq!(merged.inside_temp_c, Some(22.0));
        assert_eq!(merged.outside_temp_c, Some(8.0));
        assert_eq!(merged.charge_energy_added_kwh, Some(4.0));
        assert_eq!(merged.charge_limit_soc, Some(90.0));
        assert_eq!(merged.charging_state.as_deref(), Some("Charging"));
        assert_eq!(merged.plugged_in, Some(true));
        assert_eq!(merged.latitude, Some(48.1));
        assert_eq!(merged.speed_kmh, Some(0.0));
    }

    #[test]
    fn stale_stream_does_not_replace_a_newer_poll_position() {
        let poll = poll_at(10);
        let merged = merge_stream(Some(&poll), &stream_at(5));
        assert_eq!(merged.latitude, Some(48.1));
        assert_eq!(merged.speed_kmh, Some(0.0));
        assert_eq!(merged.inside_temp_c, Some(21.5));
    }

    #[test]
    fn stream_without_a_speed_keeps_the_published_speed() {
        let poll = poll_at(0);
        let mut sample = stream_at(5);
        sample.speed_kmh = None;
        sample.power_kw = None;
        let merged = merge_stream(Some(&poll), &sample);
        assert_eq!(merged.speed_kmh, Some(0.0));
        assert_eq!(merged.power_kw, Some(0.0));
        assert_eq!(merged.latitude, Some(48.2));
        assert_eq!(merged.inside_temp_c, Some(21.5));
    }

    #[test]
    fn poll_null_does_not_erase_a_known_temperature() {
        let poll = poll_at(0);
        let mut sparse = Observation::at(at_secs(10), Presence::Online);
        sparse.speed_kmh = Some(5.0);
        let merged = merge_poll(Some(&poll), &sparse);
        assert_eq!(merged.inside_temp_c, Some(21.5));
        assert_eq!(merged.outside_temp_c, Some(9.0));
        assert_eq!(merged.charge_limit_soc, Some(80.0));
        assert_eq!(merged.speed_kmh, Some(5.0));
    }
}
