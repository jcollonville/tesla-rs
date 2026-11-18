// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! French display formatting for the console: decimal commas, narrow no-break
//! spaces before units, local day names. CSV and the API keep machine formats.

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use chrono_tz::Tz;

use crate::app::AppState;

/// Narrow no-break space, the French separator before units and inside numbers.
pub(crate) const NNBSP: char = '\u{202f}';

const DAYS: [&str; 7] = [
    "lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche",
];
const MONTHS: [&str; 12] = [
    "janvier",
    "février",
    "mars",
    "avril",
    "mai",
    "juin",
    "juillet",
    "août",
    "septembre",
    "octobre",
    "novembre",
    "décembre",
];
const SHORT_MONTHS: [&str; 12] = [
    "janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.",
    "déc.",
];

/// Configured display zone for one vehicle, with the global setting as fallback.
pub(crate) async fn zone(app: &AppState, vehicle_id: Option<i64>) -> Tz {
    let name = crate::settings::text_or(
        &app.inner.db,
        vehicle_id,
        "TESLA_TIMEZONE",
        &app.inner.config.timezone,
    )
    .await;
    name.parse().unwrap_or(chrono_tz::UTC)
}

/// `1234.5` with one decimal becomes `1 234,5`.
pub(crate) fn number(value: f64, decimals: usize) -> String {
    if !value.is_finite() {
        return "—".to_owned();
    }
    let text = format!("{:.*}", decimals, value.abs());
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let mut grouped = String::new();
    for (index, digit) in whole.chars().enumerate() {
        if index > 0 && (whole.len() - index) % 3 == 0 {
            grouped.push(NNBSP);
        }
        grouped.push(digit);
    }
    let negative = value < 0.0 && text.chars().any(|ch| ch.is_ascii_digit() && ch != '0');
    let sign = if negative { "−" } else { "" };
    if fraction.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped},{fraction}")
    }
}

/// Value and unit, or an em dash when the value is unknown.
pub(crate) fn unit(value: Option<f64>, decimals: usize, unit: &str) -> String {
    value.map_or_else(
        || "—".to_owned(),
        |value| format!("{}{NNBSP}{unit}", number(value, decimals)),
    )
}

/// Distances under 10 km keep one decimal; longer ones are whole kilometres.
pub(crate) fn km(value: Option<f64>) -> String {
    match value {
        Some(km) if km.abs() < 100.0 => unit(Some(km), 1, "km"),
        other => unit(other, 0, "km"),
    }
}

pub(crate) fn kwh(value: Option<f64>) -> String {
    unit(value, 1, "kWh")
}

pub(crate) fn percent(value: Option<f64>) -> String {
    unit(value, 0, "%")
}

pub(crate) fn money(value: Option<f64>) -> String {
    unit(value, 2, "€")
}

pub(crate) fn count(value: i64) -> String {
    number(f64::from(i32::try_from(value).unwrap_or(i32::MAX)), 0)
}

/// Seconds from `start` to `at`, for chart time axes.
pub(crate) fn seconds_since(at: DateTime<Utc>, start: DateTime<Utc>) -> f64 {
    let millis = (at - start).num_milliseconds();
    f64::from(i32::try_from(millis).unwrap_or(i32::MAX)) / 1000.0
}

pub(crate) fn seconds_f64(seconds: i64) -> f64 {
    f64::from(i32::try_from(seconds).unwrap_or(i32::MAX))
}

pub(crate) fn hours_to_seconds(hours: f64) -> i64 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "clamped to a week before the cast"
    )]
    let seconds = (hours * 3600.0).round().clamp(0.0, 604_800.0) as i64;
    seconds
}

/// Watt-hours per kilometre from energy and distance.
pub(crate) fn efficiency(energy_kwh: Option<f64>, distance_km: Option<f64>) -> Option<f64> {
    match (energy_kwh, distance_km) {
        (Some(energy), Some(distance)) if distance > 0.05 && energy > 0.0 => {
            Some(energy * 1000.0 / distance)
        }
        _ => None,
    }
}

/// `35 min`, `1 h 05`, `45 s`.
pub(crate) fn duration(seconds: Option<i64>) -> String {
    let Some(seconds) = seconds else {
        return "—".to_owned();
    };
    let seconds = seconds.max(0);
    if seconds < 60 {
        return format!("{seconds}{NNBSP}s");
    }
    let minutes = (seconds + 30) / 60;
    if minutes < 60 {
        return format!("{minutes}{NNBSP}min");
    }
    let hours = minutes / 60;
    let rest = minutes % 60;
    if hours >= 24 {
        return format!("{}{NNBSP}j {}{NNBSP}h", hours / 24, hours % 24);
    }
    format!("{hours}{NNBSP}h{NNBSP}{rest:02}")
}

pub(crate) fn duration_i32(seconds: Option<i32>) -> String {
    duration(seconds.map(i64::from))
}

/// `08:12` in the display zone.
pub(crate) fn clock(at: DateTime<Utc>, tz: Tz) -> String {
    at.with_timezone(&tz).format("%H:%M").to_string()
}

pub(crate) fn local_date(at: DateTime<Utc>, tz: Tz) -> NaiveDate {
    at.with_timezone(&tz).date_naive()
}

/// `Aujourd'hui`, `Hier`, `lundi 16 novembre`, with the year when it differs.
pub(crate) fn day_label(date: NaiveDate, today: NaiveDate) -> String {
    if date == today {
        return "Aujourd'hui".to_owned();
    }
    if Some(date) == today.pred_opt() {
        return "Hier".to_owned();
    }
    let text = long_date(date, date.year() != today.year());
    capitalize(&text)
}

/// `lundi 16 novembre 2026`.
pub(crate) fn long_date(date: NaiveDate, with_year: bool) -> String {
    let day = DAYS[date.weekday().num_days_from_monday() as usize];
    let month = MONTHS[date.month0() as usize];
    if with_year {
        format!("{day} {} {month} {}", date.day(), date.year())
    } else {
        format!("{day} {} {month}", date.day())
    }
}

/// `16 nov.` for chart labels.
pub(crate) fn short_date(date: NaiveDate) -> String {
    format!("{} {}", date.day(), SHORT_MONTHS[date.month0() as usize])
}

/// `nov.` for monthly chart labels.
pub(crate) fn short_month(date: NaiveDate) -> &'static str {
    SHORT_MONTHS[date.month0() as usize]
}

/// `Novembre 2026`.
pub(crate) fn month_title(date: NaiveDate) -> String {
    capitalize(&format!(
        "{} {}",
        MONTHS[date.month0() as usize],
        date.year()
    ))
}

/// `Lundi 16 novembre 2026 · 08:12`.
pub(crate) fn moment(at: DateTime<Utc>, tz: Tz) -> String {
    let local = at.with_timezone(&tz);
    format!(
        "{} · {}",
        capitalize(&long_date(local.date_naive(), true)),
        local.format("%H:%M")
    )
}

/// `à l'instant`, `il y a 4 min`, `il y a 2 h`, `il y a 3 j`.
pub(crate) fn ago(at: Option<DateTime<Utc>>, now: DateTime<Utc>) -> String {
    let Some(at) = at else {
        return "jamais".to_owned();
    };
    let secs = (now - at).num_seconds();
    if secs < 45 {
        return "à l'instant".to_owned();
    }
    let minutes = (secs + 30) / 60;
    if minutes < 60 {
        return format!("il y a {minutes}{NNBSP}min");
    }
    let hours = minutes / 60;
    if hours < 48 {
        return format!("il y a {hours}{NNBSP}h");
    }
    format!("il y a {}{NNBSP}j", hours / 24)
}

pub(crate) fn capitalize(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// A place as a short name and an optional detail line.
#[derive(Clone, Default)]
pub(crate) struct Place {
    pub name: String,
    pub detail: Option<String>,
}

impl Place {
    /// Geofence names win; otherwise the first address segment is the name.
    pub(crate) fn from(address: Option<&str>, geofence: Option<&str>) -> Option<Self> {
        let address = address.map(str::trim).filter(|text| !text.is_empty());
        let geofence = geofence.map(str::trim).filter(|text| !text.is_empty());
        match (geofence, address) {
            (Some(fence), Some(address)) if fence != address => Some(Self {
                name: fence.to_owned(),
                detail: Some(address.to_owned()),
            }),
            (Some(fence), _) => Some(Self {
                name: fence.to_owned(),
                detail: None,
            }),
            (None, Some(address)) => {
                let (head, tail) = address.split_once(',').unwrap_or((address, ""));
                let tail = tail.trim();
                Some(Self {
                    name: head.trim().to_owned(),
                    detail: (!tail.is_empty()).then(|| tail.to_owned()),
                })
            }
            (None, None) => None,
        }
    }

    pub(crate) fn name_or_unknown(place: Option<&Self>) -> String {
        place.map_or_else(|| "Lieu inconnu".to_owned(), |place| place.name.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::{NNBSP, Place, ago, day_label, duration, km, number};
    use chrono::{NaiveDate, TimeZone, Utc};

    #[test]
    fn numbers_use_french_separators() {
        assert_eq!(number(1234.56, 1), format!("1{NNBSP}234,6"));
        assert_eq!(number(-3.0, 0), "−3");
        assert_eq!(number(-0.01, 0), "0");
        assert_eq!(km(Some(12.34)), format!("12,3{NNBSP}km"));
        assert_eq!(km(Some(312.4)), format!("312{NNBSP}km"));
        assert_eq!(km(None), "—");
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(duration(Some(42)), format!("42{NNBSP}s"));
        assert_eq!(duration(Some(35 * 60)), format!("35{NNBSP}min"));
        assert_eq!(duration(Some(3900)), format!("1{NNBSP}h{NNBSP}05"));
        assert_eq!(duration(None), "—");
    }

    #[test]
    fn day_labels_are_relative_then_long() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        assert_eq!(day_label(today, today), "Aujourd'hui");
        assert_eq!(day_label(today.pred_opt().unwrap(), today), "Hier");
        assert_eq!(
            day_label(NaiveDate::from_ymd_opt(2026, 11, 16).unwrap(), today),
            "Lundi 16 novembre"
        );
        assert_eq!(
            day_label(NaiveDate::from_ymd_opt(2025, 2, 1).unwrap(), today),
            "Samedi 1 février 2025"
        );
    }

    #[test]
    fn relative_times() {
        let now = Utc.with_ymd_and_hms(2026, 11, 18, 12, 0, 0).unwrap();
        assert_eq!(ago(Some(now), now), "à l'instant");
        assert_eq!(
            ago(Some(now - chrono::Duration::minutes(4)), now),
            format!("il y a 4{NNBSP}min")
        );
        assert_eq!(ago(None, now), "jamais");
    }

    #[test]
    fn places_prefer_the_geofence_name() {
        let place = Place::from(Some("8 allée des Tilleuls, Valbrume"), Some("Maison")).unwrap();
        assert_eq!(place.name, "Maison");
        assert_eq!(
            place.detail.as_deref(),
            Some("8 allée des Tilleuls, Valbrume")
        );
        let place = Place::from(Some("8 allée des Tilleuls, Valbrume"), None).unwrap();
        assert_eq!(place.name, "8 allée des Tilleuls");
        assert_eq!(place.detail.as_deref(), Some("Valbrume"));
        assert!(Place::from(Some(" "), None).is_none());
    }
}
