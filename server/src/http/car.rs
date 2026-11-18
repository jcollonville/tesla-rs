// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Side-profile car drawings for the overview and fleet cards.
//!
//! The silhouettes are drawn here from a handful of profile points per body
//! style. They are not Tesla artwork. Colours come from `vehicles.exterior_color`
//! and are set with presentation attributes because the CSP forbids inline styles.
//! When an official render is known for the car (see `render`), the drawing stays
//! in the page as the fallback and `car.js` swaps the render in once it loads.

use std::fmt::Write as _;

use maud::{Markup, PreEscaped, html};

use super::render::Spec;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Body {
    Model3,
    ModelY,
    ModelS,
    ModelX,
    Cybertruck,
}

#[derive(Clone, Debug)]
pub(crate) struct Look {
    pub body: Body,
    /// Known model name, `None` when the archive does not say.
    pub model: Option<&'static str>,
    pub paint: &'static Paint,
    /// Official render to try first, `None` when the car has none.
    pub render: Option<Spec>,
}

#[derive(Debug)]
pub(crate) struct Paint {
    pub hex: &'static str,
    pub name: &'static str,
}

const PAINTS: &[(&[&str], Paint)] = &[
    (
        &["pearlwhite", "white", "blanc"],
        Paint {
            hex: "#e8e9eb",
            name: "blanc",
        },
    ),
    (
        &["solidblack", "obsidian", "black", "noir"],
        Paint {
            hex: "#1d1f23",
            name: "noir",
        },
    ),
    (
        &["ultrared", "redmulticoat", "red", "rouge"],
        Paint {
            hex: "#a3141f",
            name: "rouge",
        },
    ),
    (
        &["deepblue", "blue", "bleu"],
        Paint {
            hex: "#223f73",
            name: "bleu",
        },
    ),
    (
        &[
            "midnightsilver",
            "midnight",
            "stealth",
            "grey",
            "gray",
            "gris",
            "steel",
        ],
        Paint {
            hex: "#50545b",
            name: "gris",
        },
    ),
    (
        &["quicksilver", "silver", "argent"],
        Paint {
            hex: "#a7abb1",
            name: "argent",
        },
    ),
    (
        &["stainless", "cyber"],
        Paint {
            hex: "#b9bcc0",
            name: "inox",
        },
    ),
];

const NEUTRAL: Paint = Paint {
    hex: "#8a8f97",
    name: "",
};

impl Default for Look {
    fn default() -> Self {
        Self::from(None, None)
    }
}

impl Look {
    pub(crate) fn from(model: Option<&str>, color: Option<&str>) -> Self {
        let model_key = model
            .unwrap_or("")
            .to_ascii_lowercase()
            .replace(['_', ' ', '-'], "");
        let model_key = model_key.trim_start_matches("tesla");
        let (body, name) = match model_key.trim_start_matches("model") {
            "3" => (Body::Model3, Some("Model 3")),
            "y" => (Body::ModelY, Some("Model Y")),
            "s" => (Body::ModelS, Some("Model S")),
            "x" => (Body::ModelX, Some("Model X")),
            "cybertruck" | "ct" => (Body::Cybertruck, Some("Cybertruck")),
            _ => (Body::Model3, None),
        };
        let color_key = color
            .unwrap_or("")
            .to_ascii_lowercase()
            .replace(['_', ' ', '-'], "");
        let paint = if body == Body::Cybertruck && color_key.is_empty() {
            &PAINTS[6].1
        } else {
            PAINTS
                .iter()
                .find(|(keys, _)| keys.iter().any(|key| color_key.contains(key)))
                .map_or(&NEUTRAL, |(_, paint)| paint)
        };
        Self {
            body,
            model: name,
            paint,
            render: None,
        }
    }

    /// Adds the official render for a stored wheel name, when one exists.
    pub(crate) fn with_render(mut self, color: Option<&str>, wheel: Option<&str>) -> Self {
        self.render = Spec::from_vehicle(self.body, color, wheel);
        self
    }

    pub(crate) fn label(&self) -> String {
        let model = self.model.unwrap_or("Voiture");
        if self.paint.name.is_empty() {
            model.to_owned()
        } else {
            format!("{model} {}", self.paint.name)
        }
    }
}

struct Profile {
    top: &'static [(f64, f64)],
    glass: &'static [(f64, f64)],
    wheels: [f64; 2],
    wheel_y: f64,
    radius: f64,
    pillar: f64,
    angular: bool,
}

const MODEL_3: Profile = Profile {
    top: &[
        (27.0, 121.0),
        (15.0, 111.0),
        (16.0, 99.0),
        (40.0, 89.0),
        (96.0, 80.0),
        (142.0, 73.0),
        (178.0, 54.0),
        (216.0, 46.0),
        (254.0, 47.0),
        (292.0, 56.0),
        (336.0, 69.0),
        (367.0, 76.0),
        (383.0, 84.0),
        (386.0, 104.0),
        (377.0, 121.0),
    ],
    glass: &[
        (152.0, 73.5),
        (180.0, 57.5),
        (216.0, 50.0),
        (254.0, 51.0),
        (288.0, 59.0),
        (327.0, 71.5),
    ],
    wheels: [87.0, 316.0],
    wheel_y: 113.0,
    radius: 27.0,
    pillar: 238.0,
    angular: false,
};

const MODEL_Y: Profile = Profile {
    top: &[
        (25.0, 117.0),
        (13.0, 104.0),
        (14.0, 88.0),
        (40.0, 76.0),
        (96.0, 67.0),
        (138.0, 59.0),
        (176.0, 38.0),
        (214.0, 28.0),
        (262.0, 29.0),
        (306.0, 37.0),
        (346.0, 52.0),
        (372.0, 61.0),
        (386.0, 73.0),
        (389.0, 99.0),
        (380.0, 116.0),
    ],
    glass: &[
        (148.0, 60.0),
        (180.0, 42.0),
        (214.0, 32.5),
        (262.0, 33.5),
        (302.0, 41.5),
        (336.0, 55.0),
    ],
    wheels: [86.0, 316.0],
    wheel_y: 109.0,
    radius: 29.0,
    pillar: 240.0,
    angular: false,
};

const MODEL_S: Profile = Profile {
    top: &[
        (26.0, 121.0),
        (12.0, 110.0),
        (14.0, 98.0),
        (44.0, 88.0),
        (110.0, 80.0),
        (154.0, 72.0),
        (188.0, 54.0),
        (224.0, 46.0),
        (264.0, 47.0),
        (302.0, 56.0),
        (345.0, 68.0),
        (372.0, 75.0),
        (386.0, 83.0),
        (389.0, 103.0),
        (380.0, 120.0),
    ],
    glass: &[
        (164.0, 72.5),
        (190.0, 57.5),
        (224.0, 50.0),
        (264.0, 51.0),
        (298.0, 59.0),
        (337.0, 70.5),
    ],
    wheels: [84.0, 326.0],
    wheel_y: 113.0,
    radius: 27.0,
    pillar: 250.0,
    angular: false,
};

const MODEL_X: Profile = Profile {
    top: &[
        (25.0, 118.0),
        (12.0, 104.0),
        (14.0, 89.0),
        (44.0, 78.0),
        (104.0, 69.0),
        (142.0, 61.0),
        (178.0, 40.0),
        (216.0, 31.0),
        (264.0, 31.0),
        (308.0, 38.0),
        (348.0, 50.0),
        (373.0, 58.0),
        (387.0, 70.0),
        (389.0, 99.0),
        (381.0, 117.0),
    ],
    glass: &[
        (152.0, 62.0),
        (182.0, 43.5),
        (216.0, 35.5),
        (264.0, 35.5),
        (304.0, 42.5),
        (342.0, 55.5),
    ],
    wheels: [85.0, 322.0],
    wheel_y: 110.5,
    radius: 29.5,
    pillar: 244.0,
    angular: false,
};

const CYBERTRUCK: Profile = Profile {
    top: &[
        (22.0, 120.0),
        (12.0, 108.0),
        (14.0, 92.0),
        (200.0, 36.0),
        (388.0, 70.0),
        (390.0, 106.0),
        (382.0, 120.0),
    ],
    glass: &[(118.0, 67.0), (200.0, 42.0), (282.0, 56.0), (282.0, 66.0)],
    wheels: [84.0, 320.0],
    wheel_y: 110.0,
    radius: 30.0,
    pillar: 214.0,
    angular: true,
};

fn profile(body: Body) -> &'static Profile {
    match body {
        Body::Model3 => &MODEL_3,
        Body::ModelY => &MODEL_Y,
        Body::ModelS => &MODEL_S,
        Body::ModelX => &MODEL_X,
        Body::Cybertruck => &CYBERTRUCK,
    }
}

/// Catmull-Rom spline through the points as cubic Bézier segments.
fn spline(path: &mut String, points: &[(f64, f64)], angular: bool) {
    if angular {
        for point in points.iter().skip(1) {
            let _ = write!(path, " L{:.1} {:.1}", point.0, point.1);
        }
        return;
    }
    for index in 0..points.len().saturating_sub(1) {
        let p0 = points[index.saturating_sub(1)];
        let p1 = points[index];
        let p2 = points[index + 1];
        let p3 = points[(index + 2).min(points.len() - 1)];
        let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        let _ = write!(
            path,
            " C{:.1} {:.1} {:.1} {:.1} {:.1} {:.1}",
            c1.0, c1.1, c2.0, c2.1, p2.0, p2.1
        );
    }
}

fn body_path(shape: &Profile) -> String {
    let first = shape.top[0];
    let last = shape.top[shape.top.len() - 1];
    let mut path = format!("M{:.1} {:.1}", first.0, first.1);
    spline(&mut path, shape.top, shape.angular);
    let sill = last.1.max(first.1);
    let arch = shape.radius + 4.0;
    let drop = (sill - shape.wheel_y).clamp(0.0, arch - 1.0);
    let half = (arch * arch - drop * drop).sqrt();
    let [front, rear] = shape.wheels;
    let _ = write!(
        path,
        " L{:.1} {sill:.1} A{arch:.1} {arch:.1} 0 1 0 {:.1} {sill:.1} L{:.1} {sill:.1} A{arch:.1} {arch:.1} 0 1 0 {:.1} {sill:.1} Z",
        rear + half,
        rear - half,
        front + half,
        front - half,
    );
    path
}

fn glass_path(shape: &Profile) -> String {
    let first = shape.glass[0];
    let mut path = format!("M{:.1} {:.1}", first.0, first.1);
    spline(&mut path, shape.glass, shape.angular);
    path.push_str(" Z");
    path
}

fn shade(hex: &str, amount: f64) -> String {
    let value = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0x008a_8f97);
    let channel = |shift: u32| {
        let base = f64::from((value >> shift) & 0xff);
        let target = if amount >= 0.0 { 255.0 } else { 0.0 };
        let mixed = base + (target - base) * amount.abs();
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "clamped to a colour channel"
        )]
        let channel = mixed.round().clamp(0.0, 255.0) as u8;
        channel
    };
    format!("#{:02x}{:02x}{:02x}", channel(16), channel(8), channel(0))
}

/// The car drawing. `key` keeps gradient ids unique when a page shows several cars.
/// The car picture: the drawing, plus the official render on top when `look`
/// has one. The render is a same-origin image; `car.js` shows it only after it
/// has loaded, so a missing render leaves the drawing in place.
pub(crate) fn visual(look: &Look, key: &str, vehicle_id: i64) -> Markup {
    let Some(spec) = &look.render else {
        return drawing(look, key);
    };
    html! {
        div class="car-stage has-render" {
            img class="car-photo" src=(format!("/v/{vehicle_id}/car.png?v={}", spec.version()))
                alt=(look.label()) width="720" height="405" decoding="async";
            (drawing(look, key))
        }
    }
}

pub(crate) fn drawing(look: &Look, key: &str) -> Markup {
    let shape = profile(look.body);
    let body = body_path(shape);
    let glass = glass_path(shape);
    let hex = look.paint.hex;
    let paint = format!("car-{key}-paint");
    let window = format!("car-{key}-glass");
    let floor = format!("car-{key}-floor");
    let rim = format!("car-{key}-rim");
    let belt = shape.glass[0].1;
    let sill = shape.top[0].1.max(shape.top[shape.top.len() - 1].1);
    let nose = shape.top[2];
    let tail = shape.top[shape.top.len() - 3];
    let pillar_top = shape
        .glass
        .iter()
        .map(|point| point.1)
        .fold(f64::INFINITY, f64::min);
    html! {
        svg class="car" viewBox="0 0 400 160" role="img" aria-label=(look.label()) focusable="false" {
            defs {
                linearGradient id=(paint) x1="0" y1="0" x2="0" y2="1" {
                    stop offset="0" stop-color=(shade(hex, 0.28)) {}
                    stop offset="0.38" stop-color=(hex) {}
                    stop offset="0.72" stop-color=(shade(hex, -0.12)) {}
                    stop offset="1" stop-color=(shade(hex, -0.42)) {}
                }
                linearGradient id=(window) x1="0" y1="0" x2="1" y2="1" {
                    stop offset="0" stop-color="#4a525e" {}
                    stop offset="0.45" stop-color="#181b21" {}
                    stop offset="1" stop-color="#0a0b0d" {}
                }
                radialGradient id=(floor) cx="0.5" cy="0.5" r="0.5" {
                    stop offset="0" stop-color="#000" stop-opacity="0.38" {}
                    stop offset="1" stop-color="#000" stop-opacity="0" {}
                }
                radialGradient id=(rim) cx="0.4" cy="0.35" r="0.75" {
                    stop offset="0" stop-color="#7d838c" {}
                    stop offset="0.6" stop-color="#3b3f46" {}
                    stop offset="1" stop-color="#22252a" {}
                }
            }
            ellipse class="car-floor" cx="200" cy="141" rx="196" ry="9" fill=(format!("url(#{floor})")) {}
            @for x in shape.wheels {
                path class="car-well" d=(well(x, shape)) fill="#0c0d0f" {}
            }
            path class="car-body" d=(body) fill=(format!("url(#{paint})")) {}
            path class="car-glass" d=(glass) fill=(format!("url(#{window})")) {}
            line class="car-pillar" x1=(format!("{:.1}", shape.pillar - 3.0)) y1=(format!("{:.1}", pillar_top)) x2=(format!("{:.1}", shape.pillar)) y2=(format!("{:.1}", belt + 1.0)) stroke="#0b0c0e" stroke-width="3.2" {}
            path class="car-line" d=(format!("M{:.1} {:.1} Q200 {:.1} {:.1} {:.1}", nose.0 + 14.0, nose.1 - 1.0, belt + 4.0, tail.0 - 6.0, tail.1 + 3.0)) fill="none" stroke="#ffffff" stroke-opacity="0.22" stroke-width="1.2" {}
            path class="car-line" d=(format!("M{:.1} {:.1} L{:.1} {:.1}", shape.pillar, belt + 3.0, shape.pillar + 2.0, sill - 6.0)) fill="none" stroke="#000" stroke-opacity="0.2" stroke-width="0.8" {}
            rect class="car-handle" x=(format!("{:.1}", shape.pillar - 34.0)) y=(format!("{:.1}", belt + 8.0)) width="15" height="2.6" rx="1.3" fill="#000" fill-opacity="0.28" {}
            rect class="car-handle" x=(format!("{:.1}", shape.pillar + 30.0)) y=(format!("{:.1}", belt + 9.0)) width="15" height="2.6" rx="1.3" fill="#000" fill-opacity="0.28" {}
            path class="car-lamp" d=(format!("M{:.1} {:.1} q16 -6 32 -7 q-14 6 -30 10 z", nose.0 + 4.0, nose.1 - 1.0)) fill="#f4f7fb" fill-opacity="0.85" {}
            path class="car-lamp rear" d=(format!("M{:.1} {:.1} q10 1 17 5 l-1 3.5 q-8 -3 -16.5 -5 z", tail.0 - 21.0, tail.1 + 1.0)) fill="#c3212f" {}
            @for x in shape.wheels {
                g class="car-wheel" {
                    circle cx=(x) cy=(shape.wheel_y) r=(shape.radius) fill="#141518" {}
                    circle cx=(x) cy=(shape.wheel_y) r=(shape.radius * 0.7) fill=(format!("url(#{rim})")) {}
                    circle cx=(x) cy=(shape.wheel_y) r=(shape.radius * 0.6) fill="none" stroke="#a9aeb5" stroke-opacity="0.55" stroke-width="1" {}
                    (PreEscaped(spokes(x, shape.wheel_y, shape.radius)))
                    circle cx=(x) cy=(shape.wheel_y) r=(shape.radius * 0.14) fill="#55595f" {}
                }
            }
        }
    }
}

/// Dark upper half of the wheel arch, so the gap above the tyre reads as a wheel well.
fn well(cx: f64, shape: &Profile) -> String {
    let arch = shape.radius + 4.0;
    let cy = shape.wheel_y;
    format!(
        "M{:.1} {cy:.1} A{arch:.1} {arch:.1} 0 0 1 {:.1} {cy:.1} Z",
        cx - arch,
        cx + arch
    )
}

fn spokes(cx: f64, cy: f64, radius: f64) -> String {
    let mut out = String::new();
    for index in 0..5 {
        let angle = f64::from(index) * std::f64::consts::TAU / 5.0 - 0.3;
        let (sin, cos) = angle.sin_cos();
        let _ = write!(
            out,
            "<line x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\" stroke=\"#8d939b\" stroke-opacity=\"0.6\" stroke-width=\"2\" stroke-linecap=\"round\"/>",
            cx + cos * radius * 0.2,
            cy + sin * radius * 0.2,
            cx + cos * radius * 0.55,
            cy + sin * radius * 0.55,
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Body, Look, drawing};

    #[test]
    fn models_and_colours_map_from_fleet_and_teslamate_names() {
        let look = Look::from(Some("modely"), Some("PearlWhite"));
        assert_eq!(look.body, Body::ModelY);
        assert_eq!(look.label(), "Model Y blanc");
        assert_eq!(
            Look::from(Some("3"), Some("MidnightSilver")).paint.name,
            "gris"
        );
        assert_eq!(Look::from(Some("Model S"), None).body, Body::ModelS);
        assert_eq!(Look::from(Some("cybertruck"), None).paint.name, "inox");
        let unknown = Look::from(None, Some("Chartreuse"));
        assert_eq!(unknown.model, None);
        assert_eq!(unknown.label(), "Voiture");
    }

    #[test]
    fn drawing_uses_attributes_not_inline_styles() {
        for model in ["model3", "modely", "models", "modelx", "cybertruck"] {
            let svg = drawing(&Look::from(Some(model), Some("DeepBlue")), "t").into_string();
            assert!(svg.starts_with("<svg class=\"car\""), "{model}");
            assert!(!svg.contains("style="), "{model}");
            assert!(svg.contains("url(#car-t-paint)"), "{model}");
            assert!(!svg.contains("NaN"), "{model}");
        }
    }
}
