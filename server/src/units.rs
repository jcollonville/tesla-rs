// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

/// Statute miles to kilometres. Fleet API odometer and range values are miles.
pub(crate) const MILES_TO_KM: f64 = 1.609_344;

/// Converts a Fleet API mile value to kilometres.
pub(crate) fn miles_to_km(miles: f64) -> Option<f64> {
    finite_positive_or_any(miles).map(|value| value * MILES_TO_KM)
}

/// Converts a speed sample to kilometres per hour.
///
/// `gui_distance_units` from the car is `km/hr` or `mi/hr`. When it is missing,
/// the historical Fleet API behaviour is miles per hour.
pub(crate) fn speed_to_kmh(speed: f64, gui_distance_units: Option<&str>) -> Option<f64> {
    let speed = finite(speed)?;
    if units_are_metric(gui_distance_units) {
        Some(speed)
    } else {
        Some(speed * MILES_TO_KM)
    }
}

/// Fleet API `scheduled_charging_start_time` is minutes after midnight, at most 1440.
pub(crate) fn minutes_after_midnight(value: f64) -> Option<i32> {
    if !value.is_finite() || !(0.0..=1_440.0).contains(&value) {
        return None;
    }
    format!("{value:.0}").parse().ok()
}

pub(crate) fn units_are_metric(gui_distance_units: Option<&str>) -> bool {
    gui_distance_units
        .unwrap_or("")
        .to_ascii_lowercase()
        .contains("km")
}

fn finite(value: f64) -> Option<f64> {
    value.is_finite().then_some(value)
}

fn finite_positive_or_any(value: f64) -> Option<f64> {
    finite(value).filter(|value| *value >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::{MILES_TO_KM, miles_to_km, speed_to_kmh};

    fn near(left: f64, right: f64) -> bool {
        (left - right).abs() < 1e-9
    }

    #[test]
    fn converts_miles_and_honours_gui_speed_units() {
        let km = miles_to_km(1.0).unwrap();
        assert!(near(km, MILES_TO_KM));
        assert!(miles_to_km(f64::NAN).is_none());
        assert!(near(speed_to_kmh(10.0, Some("km/hr")).unwrap(), 10.0));
        assert!(near(
            speed_to_kmh(10.0, Some("mi/hr")).unwrap(),
            10.0 * MILES_TO_KM
        ));
        assert!(near(speed_to_kmh(10.0, None).unwrap(), 10.0 * MILES_TO_KM));
    }
}
