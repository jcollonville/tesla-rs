// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Maps and chart payloads.
//!
//! The overview mini-map is drawn from the archive's own positions with a local
//! equirectangular projection, and that page does not print coordinates. A drive
//! or a charge that the operator opened also carries a rounded track in
//! `data-map` so the browser can place it on the basemap. The SVG underneath is
//! the same local drawing, used when the basemap cannot load.
//!
//! Charts ship as a server-rendered fallback SVG plus a JSON payload in a
//! `data-chart` attribute that `charts.js` turns into an interactive chart.

use std::fmt::Write as _;

use maud::{Markup, PreEscaped, html};
use serde::Serialize;

use super::widget::element;

use crate::rdp::rdp_limit;

/// A stream fix this close to another point covers it on the map and charts.
const STREAM_COVER_SECS: i64 = 15;

/// Which points of a drive to draw, given each point's time and whether it
/// came from the Owner stream. Input is ordered by time.
///
/// The logger no longer records the stream, but older drives still hold
/// its `source = 'stream'` rows. The stream sent about one fix per second;
/// telemetry and polling are sparser and sampled on another clock, so
/// interleaving them zigzags between two tracks. Where the stream covers a span only its fixes are kept; elsewhere
/// every point is kept, so a telemetry-only stretch still draws.
pub(crate) fn one_source_mask(points: &[(chrono::DateTime<chrono::Utc>, bool)]) -> Vec<bool> {
    let stream: Vec<i64> = points
        .iter()
        .filter(|(_, is_stream)| *is_stream)
        .map(|(at, _)| at.timestamp_millis())
        .collect();
    let cover = STREAM_COVER_SECS.saturating_mul(1_000);
    points
        .iter()
        .map(|(at, is_stream)| {
            if *is_stream {
                return true;
            }
            let ms = at.timestamp_millis();
            let next = stream.partition_point(|stamp| *stamp < ms);
            let near = |index: usize| {
                stream
                    .get(index)
                    .is_some_and(|stamp| stamp.abs_diff(ms) <= cover.unsigned_abs())
            };
            !(near(next) || next.checked_sub(1).is_some_and(near))
        })
        .collect()
}

/// Metres per degree of latitude.
const M_PER_DEG: f64 = 111_320.0;

/// Flat projection around a reference latitude, in metres.
#[derive(Clone, Copy)]
pub(crate) struct Projection {
    lat0: f64,
    lon0: f64,
    cos: f64,
}

impl Projection {
    pub(crate) fn around(lat: f64, lon: f64) -> Self {
        Self {
            lat0: lat,
            lon0: lon,
            cos: lat.to_radians().cos().max(0.01),
        }
    }

    /// East and north offsets in metres.
    pub(crate) fn metres(&self, lat: f64, lon: f64) -> (f64, f64) {
        (
            (lon - self.lon0) * M_PER_DEG * self.cos,
            (lat - self.lat0) * M_PER_DEG,
        )
    }
}

/// Screen transform from projected metres into a `width` × `height` box.
#[derive(Clone, Copy)]
pub(crate) struct Frame {
    projection: Projection,
    scale: f64,
    cx: f64,
    cy: f64,
    mx: f64,
    my: f64,
}

impl Frame {
    /// Fits every point into the box with `pad` units of margin.
    pub(crate) fn fit(points: &[(f64, f64)], width: f64, height: f64, pad: f64) -> Option<Self> {
        let first = points.first()?;
        let projection = Projection::around(first.0, first.1);
        let mut min = (f64::INFINITY, f64::INFINITY);
        let mut max = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for (lat, lon) in points {
            let (x, y) = projection.metres(*lat, *lon);
            min = (min.0.min(x), min.1.min(y));
            max = (max.0.max(x), max.1.max(y));
        }
        let span_x = (max.0 - min.0).max(80.0);
        let span_y = (max.1 - min.1).max(80.0);
        let scale = ((width - 2.0 * pad) / span_x).min((height - 2.0 * pad) / span_y);
        Some(Self {
            projection,
            scale,
            cx: width / 2.0,
            cy: height / 2.0,
            mx: f64::midpoint(min.0, max.0),
            my: f64::midpoint(min.1, max.1),
        })
    }

    /// Centres the box on one point, `half_height_m` metres above and below it.
    pub(crate) fn centred(lat: f64, lon: f64, width: f64, height: f64, half_height_m: f64) -> Self {
        Self {
            projection: Projection::around(lat, lon),
            scale: height / 2.0 / half_height_m,
            cx: width / 2.0,
            cy: height / 2.0,
            mx: 0.0,
            my: 0.0,
        }
    }

    pub(crate) fn point(&self, lat: f64, lon: f64) -> (f64, f64) {
        let (x, y) = self.projection.metres(lat, lon);
        (
            self.cx + (x - self.mx) * self.scale,
            self.cy - (y - self.my) * self.scale,
        )
    }

    /// Screen units per metre.
    pub(crate) fn scale(&self) -> f64 {
        self.scale
    }
}

fn path_of(points: impl Iterator<Item = (f64, f64)>) -> String {
    let mut path = String::new();
    for (index, (x, y)) in points.enumerate() {
        let command = if index == 0 { 'M' } else { 'L' };
        let _ = write!(path, "{command}{x:.1} {y:.1}");
    }
    path
}

/// Round scale-bar length in metres that fits about a fifth of the width.
fn scale_bar(frame: &Frame, width: f64) -> (f64, String) {
    let target = width / 5.0 / frame.scale();
    let steps = [
        50.0, 100.0, 200.0, 250.0, 500.0, 1_000.0, 2_000.0, 5_000.0, 10_000.0, 20_000.0, 50_000.0,
        100_000.0,
    ];
    let metres = steps
        .iter()
        .copied()
        .rev()
        .find(|step| *step <= target)
        .unwrap_or(50.0);
    let label = if metres >= 1_000.0 {
        format!("{}\u{202f}km", metres / 1_000.0)
    } else {
        format!("{metres}\u{202f}m")
    };
    (metres * frame.scale(), label)
}

const ROUTE_W: f64 = 800.0;
const ROUTE_H: f64 = 440.0;

/// Drive route: the simplified track with start and end markers.
/// Returns the markup and the frame, so chart samples can be placed on it.
pub(crate) fn route_map(
    located: &[(f64, f64)],
    id: &str,
    sync: &str,
    samples: &[Option<(f64, f64)>],
) -> Option<Markup> {
    if located.len() < 2 {
        return None;
    }
    let frame = Frame::fit(located, ROUTE_W, ROUTE_H, 48.0)?;
    let mut previous = [-99.0, -99.0];
    let sample_xy: Vec<[f64; 2]> = samples
        .iter()
        .map(|sample| {
            if let Some((lat, lon)) = sample {
                let (x, y) = frame.point(*lat, *lon);
                previous = [(x * 10.0).round() / 10.0, (y * 10.0).round() / 10.0];
            }
            previous
        })
        .collect();
    let flat: Vec<[f64; 2]> = located.iter().map(|(lat, lon)| [*lat, *lon]).collect();
    let kept = rdp_limit(&flat, 400);
    let track = path_of(
        kept.iter()
            .filter_map(|index| located.get(*index))
            .map(|(lat, lon)| frame.point(*lat, *lon)),
    );
    let start = frame.point(located[0].0, located[0].1);
    let last = located[located.len() - 1];
    let end = frame.point(last.0, last.1);
    let (bar, label) = scale_bar(&frame, ROUTE_W);
    let xy = serde_json::to_string(&sample_xy).unwrap_or_else(|_| "[]".to_owned());
    let markup = html! {
        svg id=(id) class="map route-map" viewBox=(format!("0 0 {ROUTE_W} {ROUTE_H}")) role="img" aria-label="Tracé du trajet" data-sync=(sync) data-xy=(xy) {
            path class="map-casing" d=(track) {}
            path class="map-track" d=(track) pathLength="1" {}
            circle class="map-start" cx=(format!("{:.1}", start.0)) cy=(format!("{:.1}", start.1)) r="7" {}
            circle class="map-end-halo" cx=(format!("{:.1}", end.0)) cy=(format!("{:.1}", end.1)) r="15" {}
            circle class="map-end" cx=(format!("{:.1}", end.0)) cy=(format!("{:.1}", end.1)) r="7.5" {}
            g class="map-scale" transform=(format!("translate(24 {:.0})", ROUTE_H - 24.0)) {
                path d=(format!("M0 -5 V0 H{bar:.1} V-5")) {}
                text x=(format!("{:.1}", bar + 8.0)) y="0" { (label) }
            }
            circle class="map-cursor" cx="-20" cy="-20" r="8" {}
        }
    };
    Some(markup)
}

/// `data-map` payload. Coordinates are longitude then latitude, five decimals.
/// `None` when no sample can be placed.
pub(crate) fn map_json(samples: &[Option<(f64, f64)>]) -> Option<String> {
    let mut body = String::from("{\"cursor\":[");
    let mut any = false;
    for (index, sample) in samples.iter().enumerate() {
        if index > 0 {
            body.push(',');
        }
        match sample.and_then(|(lat, lon)| finite_pair(lat, lon)) {
            Some((lat, lon)) => {
                any = true;
                let _ = write!(body, "[{lon:.5},{lat:.5}]");
            }
            None => body.push_str("null"),
        }
    }
    if !any {
        return None;
    }
    body.push_str("]}");
    Some(body)
}

/// One charge location, longitude then latitude.
pub(crate) fn point_json(lat: f64, lon: f64) -> Option<String> {
    let (lat, lon) = finite_pair(lat, lon)?;
    Some(format!("{{\"point\":[{lon:.5},{lat:.5}]}}"))
}

fn finite_pair(lat: f64, lon: f64) -> Option<(f64, f64)> {
    if !lat.is_finite() || !lon.is_finite() {
        return None;
    }
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
        return None;
    }
    Some((lat, lon))
}

const MINI_W: f64 = 360.0;
const MINI_H: f64 = 220.0;

/// What the overview mini-map knows about the car.
pub(crate) struct MiniMap<'a> {
    pub lat: f64,
    pub lon: f64,
    pub heading: Option<f64>,
    pub moving: bool,
    pub tracks: &'a [Vec<(f64, f64)>],
    pub label: &'a str,
}

/// Small abstract map kept for the coordinate-leak test. The console draws this
/// through `tl-mini-map` instead.
#[cfg(test)]
pub(crate) fn mini_map(map: &MiniMap<'_>) -> Markup {
    let frame = Frame::centred(map.lat, map.lon, MINI_W, MINI_H, 1_100.0);
    let mut roads = String::new();
    for track in map.tracks {
        if track.len() < 2 {
            continue;
        }
        let _ = write!(
            roads,
            "<path class=\"map-road\" d=\"{}\"/>",
            path_of(track.iter().map(|(lat, lon)| frame.point(*lat, *lon)))
        );
    }
    let (cx, cy) = (MINI_W / 2.0, MINI_H / 2.0);
    let (bar, label) = scale_bar(&frame, MINI_W);
    let class = if map.moving {
        "mini-map live"
    } else {
        "mini-map"
    };
    html! {
        svg class=(class) viewBox=(format!("0 0 {MINI_W} {MINI_H}")) role="img" aria-label=(format!("Position : {}", map.label)) focusable="false" {
            defs {
                pattern id="mini-dots" width="18" height="18" patternUnits="userSpaceOnUse" {
                    circle class="map-dot" cx="1.5" cy="1.5" r="1" {}
                }
                radialGradient id="mini-fade" cx="0.5" cy="0.5" r="0.62" {
                    stop offset="0.55" stop-color="#000" stop-opacity="0" {}
                    stop offset="1" stop-color="#000" stop-opacity="1" {}
                }
            }
            rect class="map-bg" width=(MINI_W) height=(MINI_H) {}
            rect width=(MINI_W) height=(MINI_H) fill="url(#mini-dots)" {}
            (PreEscaped(roads))
            rect class="map-vignette" width=(MINI_W) height=(MINI_H) fill="url(#mini-fade)" {}
            circle class="map-pulse" cx=(cx) cy=(cy) r="26" {}
            circle class="map-halo" cx=(cx) cy=(cy) r="15" {}
            @if let Some(heading) = map.heading {
                path class="map-heading" d=(format!("M{cx} {:.1} l7 13 l-7 -4 l-7 4 z", cy - 22.0)) transform=(format!("rotate({heading:.0} {cx} {cy})")) {}
            }
            circle class="map-car" cx=(cx) cy=(cy) r="7" {}
            g class="map-scale" transform=(format!("translate(16 {:.0})", MINI_H - 16.0)) {
                path d=(format!("M0 -4 V0 H{bar:.1} V-4")) {}
                text x=(format!("{:.1}", bar + 6.0)) y="0" { (label) }
            }
        }
    }
}

/// Projected mini-map. Screen units only: latitude and longitude never leave `Frame`.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct MiniMapModel {
    pub label: String,
    pub moving: bool,
    pub empty: bool,
    pub width: f64,
    pub height: f64,
    pub roads: Vec<String>,
    pub heading: Option<f64>,
    pub scale_width: f64,
    pub scale_label: String,
}

impl Default for MiniMapModel {
    fn default() -> Self {
        mini_map_empty_model()
    }
}

pub(crate) fn mini_map_model(map: &MiniMap<'_>) -> MiniMapModel {
    let frame = Frame::centred(map.lat, map.lon, MINI_W, MINI_H, 1_100.0);
    let mut roads = Vec::new();
    for track in map.tracks {
        if track.len() < 2 {
            continue;
        }
        roads.push(path_of(
            track.iter().map(|(lat, lon)| frame.point(*lat, *lon)),
        ));
    }
    let (bar, label) = scale_bar(&frame, MINI_W);
    MiniMapModel {
        label: format!("Position : {}", map.label),
        moving: map.moving,
        empty: false,
        width: MINI_W,
        height: MINI_H,
        roads,
        heading: map.heading,
        scale_width: bar,
        scale_label: label,
    }
}

pub(crate) fn mini_map_empty_model() -> MiniMapModel {
    MiniMapModel {
        label: "Position inconnue".to_owned(),
        moving: false,
        empty: true,
        width: MINI_W,
        height: MINI_H,
        roads: Vec::new(),
        heading: None,
        scale_width: 0.0,
        scale_label: String::new(),
    }
}

/// X axis description for `charts.js`.
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(crate) enum Axis {
    /// `t` holds seconds after `origin_ms`. Labels use `offset_min` from UTC.
    Time { origin_ms: i64, offset_min: i32 },
    /// `t` holds plain numbers in `unit`.
    Number {
        unit: String,
        label: String,
        decimals: u8,
    },
}

#[derive(Serialize, Clone)]
pub(crate) struct Series {
    pub name: String,
    pub unit: String,
    pub decimals: u8,
    pub tone: &'static str,
    pub values: Vec<Option<f64>>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub area: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub signed: bool,
}

/// One line chart: shared x values and one or more series.
#[derive(Serialize)]
pub(crate) struct LineChart<'a> {
    pub kind: &'static str,
    pub x: &'a Axis,
    pub t: &'a [f64],
    pub series: Vec<Series>,
}

/// One bar chart with a link per bucket.
#[derive(Serialize)]
pub(crate) struct BarChart<'a> {
    pub kind: &'static str,
    pub labels: &'a [String],
    pub titles: &'a [String],
    pub hrefs: &'a [String],
    pub series: Vec<Series>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub stacked: bool,
}

/// Indices that keep at most `max` samples, always including the last one.
pub(crate) fn stride(len: usize, max: usize) -> Vec<usize> {
    if len <= max || max < 2 {
        return (0..len).collect();
    }
    let mut out: Vec<usize> = (0..max)
        .map(|index| index * (len - 1) / (max - 1))
        .collect();
    out.dedup();
    out
}

/// A `figure` with the static fallback and the interactive payload.
pub(crate) fn line_figure(
    title: &str,
    summary: &str,
    sync: Option<&str>,
    chart: &LineChart<'_>,
) -> Markup {
    let payload = serde_json::to_string(chart).unwrap_or_else(|_| "{}".to_owned());
    let first = chart.series.first();
    let fallback: Vec<(f64, f64)> = first
        .map(|series| {
            chart
                .t
                .iter()
                .zip(&series.values)
                .filter_map(|(x, y)| Some((*x, (*y)?)))
                .collect()
        })
        .unwrap_or_default();
    html! {
        figure class="chart" data-chart=(payload) data-sync=[sync] tabindex="0" aria-label=(format!("{title}. {summary}")) {
            figcaption {
                span class="chart-title" { (title) }
                span class="chart-summary" { (summary) }
            }
            div class="chart-plot" {
                (PreEscaped(chart_svg(&fallback)))
            }
        }
    }
}

/// A bar `figure`: static bars and labels, replaced by `charts.js` when it runs.
pub(crate) fn bar_figure(title: &str, summary: &str, chart: &BarChart<'_>) -> Markup {
    let payload = serde_json::to_string(chart).unwrap_or_else(|_| "{}".to_owned());
    let totals: Vec<f64> = (0..chart.labels.len())
        .map(|index| {
            chart
                .series
                .iter()
                .filter_map(|series| series.values.get(index).copied().flatten())
                .sum::<f64>()
        })
        .collect();
    let max = totals.iter().copied().fold(0.0_f64, f64::max).max(1e-9);
    let width = u32::try_from(totals.len().max(1)).unwrap_or(1) * 8;
    let figure = html! {
        figure class="chart" data-chart=(payload) tabindex="0" aria-label=(format!("{title}. {summary}")) {
            figcaption {
                span class="chart-title" { (title) }
                span class="chart-summary" { (summary) }
            }
            div class="chart-plot" {
                svg class="chart-fallback" viewBox=(format!("0 0 {width} 40")) width="640" height="180" preserveAspectRatio="none" role="img" aria-hidden="true" {
                    @for (index, total) in totals.iter().enumerate() {
                        @let height = (total.max(0.0) / max) * 36.0;
                        @let x = u32::try_from(index).unwrap_or(0) * 8;
                        @let rect = html! { rect x=(x) y=(format!("{:.2}", 38.0 - height)) width="6" height=(format!("{height:.2}")) fill="currentColor" {} };
                        @if let Some(href) = chart.hrefs.get(index) {
                            a href=(href) { (rect) }
                        } @else {
                            (rect)
                        }
                    }
                }
            }
            ol class="chart-labels" {
                @for (index, label) in chart.labels.iter().enumerate() {
                    li {
                        @if let Some(href) = chart.hrefs.get(index) {
                            a href=(href) { (label) }
                        } @else {
                            (label)
                        }
                    }
                }
            }
        }
    };
    element(
        "tl-chart",
        &[("variant", "card"), ("title", title), ("summary", summary)],
        None,
        figure,
    )
}

/// Static line used when scripts are off, and as the first paint.
pub(crate) fn chart_svg(series: &[(f64, f64)]) -> String {
    if series.len() < 2 {
        return "<p class=\"muted\">Pas assez de points.</p>".to_owned();
    }
    let min_x = series.first().map_or(0.0, |point| point.0);
    let max_x = series.last().map_or(1.0, |point| point.0);
    let span_x = (max_x - min_x).max(1e-9);
    let min_y = series
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    let max_y = series
        .iter()
        .map(|point| point.1)
        .fold(f64::NEG_INFINITY, f64::max);
    let span = (max_y - min_y).max(1e-9);
    let coords = series
        .iter()
        .map(|point| {
            let x = (point.0 - min_x) / span_x * 100.0;
            let y = 40.0 - ((point.1 - min_y) / span) * 36.0;
            format!("{x:.2},{y:.2}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "<svg class=\"chart-fallback\" viewBox=\"0 0 100 44\" width=\"640\" height=\"180\" preserveAspectRatio=\"none\" role=\"img\" aria-hidden=\"true\"><polyline fill=\"none\" stroke=\"currentColor\" stroke-width=\"1.2\" vector-effect=\"non-scaling-stroke\" points=\"{coords}\"/></svg>"
    )
}

/// Horizontal bar from `start` to `end` percent, for battery ranges.
pub(crate) fn soc_bar(start: Option<f64>, end: Option<f64>) -> Markup {
    let (Some(start), Some(end)) = (start, end) else {
        return html! {};
    };
    let low = start.min(end).clamp(0.0, 100.0);
    let high = start.max(end).clamp(0.0, 100.0);
    let class = if end >= start {
        "soc-bar gain"
    } else {
        "soc-bar use"
    };
    html! {
        svg class=(class) viewBox="0 0 100 6" preserveAspectRatio="none" aria-hidden="true" focusable="false" {
            rect class="soc-track" x="0" y="0" width="100" height="6" rx="3" {}
            rect class="soc-fill" x=(format!("{low:.1}")) y="0" width=(format!("{:.1}", (high - low).max(1.5))) height="6" rx="3" {}
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_source_mask_prefers_the_stream_where_it_covers() {
        let start = chrono::Utc::now();
        let at = |ms: i64| start + chrono::Duration::milliseconds(ms);
        let mut points = Vec::new();
        for secs in 0..60_i64 {
            points.push((at(secs * 1_000), true));
            if secs % 10 == 0 {
                points.push((at(secs * 1_000 + 500), false));
            }
        }
        for secs in [120_i64, 130, 140] {
            points.push((at(secs * 1_000), false));
        }
        let mask = super::one_source_mask(&points);
        let kept: Vec<_> = points
            .iter()
            .zip(&mask)
            .filter(|(_, keep)| **keep)
            .collect();
        assert_eq!(kept.len(), 63, "stream span plus the telemetry-only tail");
        assert!(kept[..60].iter().all(|((_, stream), _)| *stream));
        assert!(kept[60..].iter().all(|((_, stream), _)| !*stream));
    }

    #[test]
    fn one_source_mask_keeps_everything_without_a_stream() {
        let start = chrono::Utc::now();
        let points: Vec<_> = (0..5_i64)
            .map(|secs| (start + chrono::Duration::seconds(secs * 10), false))
            .collect();
        assert!(super::one_source_mask(&points).iter().all(|keep| *keep));
        assert_eq!(super::one_source_mask(&[]), Vec::<bool>::new());
    }

    use super::{
        Frame, MiniMap, chart_svg, map_json, mini_map, mini_map_model, point_json, route_map,
        stride,
    };

    #[test]
    fn chart_svg_has_no_style_attribute() {
        let svg = chart_svg(&[(0.0, 1.0), (1.0, 3.0)]);
        assert!(svg.contains("<polyline"));
        assert!(svg.contains("height=\"180\""));
        assert!(!svg.contains("style="));
        assert!(!svg.contains("<text"));
        assert!(!svg.contains("<script"));
    }

    #[test]
    fn frames_keep_north_up_and_fit_the_box() {
        let frame = Frame::fit(&[(0.0, 0.0), (0.01, 0.02)], 800.0, 400.0, 20.0).unwrap();
        let south_west = frame.point(0.0, 0.0);
        let north_east = frame.point(0.01, 0.02);
        assert!(north_east.0 > south_west.0);
        assert!(north_east.1 < south_west.1);
        for (x, y) in [south_west, north_east] {
            assert!((19.0..=781.0).contains(&x), "{x}");
            assert!((19.0..=381.0).contains(&y), "{y}");
        }
    }

    #[test]
    fn maps_never_print_coordinates() {
        let points = [(0.012_345, 0.067_891), (0.013_579, 0.068_642)];
        let route = route_map(&points, "m", "s", &[Some(points[1])]).unwrap();
        let tracks = vec![points.to_vec()];
        let mini = mini_map(&MiniMap {
            lat: points[0].0,
            lon: points[0].1,
            heading: Some(90.0),
            moving: false,
            tracks: &tracks,
            label: "Maison",
        });
        let route_svg = route.into_string();
        let mini_svg = mini.into_string();
        for svg in [&route_svg, &mini_svg] {
            assert!(!svg.contains("0.0123"), "{svg}");
            assert!(!svg.contains("0.0678"), "{svg}");
            assert!(!svg.contains("style="), "{svg}");
        }
        assert!(!route_svg.contains("url(#"), "{route_svg}");
        let payload = map_json(&[Some(points[0]), None, Some(points[1])]).unwrap();
        assert!(
            payload.contains("[0.06789,0.01235]") || payload.contains("[0.06789,0.01234]"),
            "{payload}"
        );
        assert!(payload.contains("null"), "{payload}");
        assert!(point_json(0.0, 0.0).is_some());
        assert!(point_json(120.0, 0.0).is_none());
        let model = serde_json::to_string(&mini_map_model(&MiniMap {
            lat: points[0].0,
            lon: points[0].1,
            heading: Some(90.0),
            moving: false,
            tracks: &tracks,
            label: "Maison",
        }))
        .unwrap();
        assert!(!model.contains("0.0123"), "{model}");
        assert!(!model.contains("0.0678"), "{model}");
        assert!(!model.contains("latitude"), "{model}");
    }

    #[test]
    fn stride_keeps_both_ends() {
        let kept = stride(1000, 10);
        assert_eq!(kept.first(), Some(&0));
        assert_eq!(kept.last(), Some(&999));
        assert!(kept.len() <= 10);
        assert_eq!(stride(5, 10), vec![0, 1, 2, 3, 4]);
    }
}
