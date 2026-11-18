// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

//! Model, paint and the official render for a vehicle card.
//!
//! There is no drawn silhouette. The card shows the official render when one
//! exists, and a text placeholder otherwise.

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
    pub name: &'static str,
}

const PAINTS: &[(&[&str], Paint)] = &[
    (&["pearlwhite", "white", "blanc"], Paint { name: "blanc" }),
    (
        &["solidblack", "obsidian", "black", "noir"],
        Paint { name: "noir" },
    ),
    (
        &["ultrared", "redmulticoat", "red", "rouge"],
        Paint { name: "rouge" },
    ),
    (&["deepblue", "blue", "bleu"], Paint { name: "bleu" }),
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
        Paint { name: "gris" },
    ),
    (
        &["quicksilver", "silver", "argent"],
        Paint { name: "argent" },
    ),
    (&["stainless", "cyber"], Paint { name: "inox" }),
];

const NEUTRAL: Paint = Paint { name: "" };

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

/// French copy for a missing official render. The shared component localizes it.
pub(crate) const IMAGE_UNAVAILABLE: &str = "Image du véhicule indisponible";

/// Same-origin render URL, or nothing when this car has no official picture.
pub(crate) fn image_url(look: &Look, vehicle_id: i64) -> Option<String> {
    look.render
        .as_ref()
        .map(|spec| format!("/v/{vehicle_id}/car.png?v={}", spec.version()))
}

#[cfg(test)]
mod tests {
    use super::{Body, Look, image_url};

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
    fn image_url_is_same_origin_and_absent_without_a_render() {
        let plain = Look::from(Some("modely"), Some("PearlWhite"));
        assert!(image_url(&plain, 4).is_none());
        let known = plain.with_render(Some("PearlWhite"), Some("Gemini19"));
        let url = image_url(&known, 4).expect("render");
        assert!(url.starts_with("/v/4/car.png?v="), "{url}");
        assert!(!url.contains("tesla.com"), "{url}");
        assert!(!url.contains("<svg"), "{url}");
    }
}
