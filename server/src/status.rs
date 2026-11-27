// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Published vehicle status.
//!
//! Polls and telemetry fold onto the last published frame: a newer reading
//! overrides the fields it carries, and a null never clears a known value.

use crate::state::Observation;

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
        scheduled_charging_mode,
        scheduled_departure_time,
        off_peak_charging_enabled,
        off_peak_hours_end_time,
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
        scheduled_charging_mode,
        scheduled_departure_time,
        off_peak_charging_enabled,
        off_peak_hours_end_time,
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

/// Clock helper for merge tests.
#[cfg(test)]
pub(crate) fn at_secs(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(1_700_000_000 + seconds, 0).expect("timestamp")
}

#[cfg(test)]
mod tests {
    use super::{at_secs, merge_poll};
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

    #[test]
    fn newer_poll_updates_climate_and_charge() {
        let poll = poll_at(0);
        let mut newer = poll_at(10);
        newer.inside_temp_c = Some(22.0);
        newer.outside_temp_c = Some(8.0);
        newer.charge_energy_added_kwh = Some(4.0);
        newer.charge_limit_soc = Some(90.0);
        newer.charging_state = Some("Charging".to_owned());
        newer.plugged_in = Some(true);
        let merged = merge_poll(Some(&poll), &newer);
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
