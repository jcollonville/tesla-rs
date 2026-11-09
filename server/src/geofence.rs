// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

/// Great-circle distance in metres.
pub(crate) fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const EARTH_RADIUS_M: f64 = 6_371_000.0;
    let phi1 = lat1.to_radians();
    let phi2 = lat2.to_radians();
    let d_phi = (lat2 - lat1).to_radians();
    let d_lambda = (lon2 - lon1).to_radians();
    let a = (d_phi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (d_lambda / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    EARTH_RADIUS_M * c
}

/// A circular geofence. `radius_m` is metres.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Fence {
    /// Local id.
    pub id: i64,
    /// Centre latitude.
    pub latitude: f64,
    /// Centre longitude.
    pub longitude: f64,
    /// Radius in metres.
    pub radius_m: f64,
}

/// Smallest geofence that contains the point. Ties break toward the lower id.
pub(crate) fn matching_geofence(latitude: f64, longitude: f64, fences: &[Fence]) -> Option<i64> {
    if !latitude.is_finite() || !longitude.is_finite() {
        return None;
    }
    let mut best: Option<(f64, i64)> = None;
    for fence in fences {
        if !fence.radius_m.is_finite() || fence.radius_m <= 0.0 {
            continue;
        }
        let distance = haversine_m(latitude, longitude, fence.latitude, fence.longitude);
        if distance <= fence.radius_m {
            let replace = match best {
                None => true,
                Some((radius, id)) => {
                    let order = fence.radius_m.total_cmp(&radius);
                    order.is_lt() || (order.is_eq() && fence.id < id)
                }
            };
            if replace {
                best = Some((fence.radius_m, fence.id));
            }
        }
    }
    best.map(|(_, id)| id)
}

#[cfg(test)]
mod tests {
    use super::{Fence, matching_geofence};

    #[test]
    fn smallest_containing_fence_wins() {
        let fences = [
            Fence {
                id: 2,
                latitude: 48.8566,
                longitude: 2.3522,
                radius_m: 500.0,
            },
            Fence {
                id: 1,
                latitude: 48.8566,
                longitude: 2.3522,
                radius_m: 80.0,
            },
            Fence {
                id: 3,
                latitude: 0.0,
                longitude: 0.0,
                radius_m: 10.0,
            },
        ];
        assert_eq!(matching_geofence(48.8566, 2.3522, &fences), Some(1));
        assert_eq!(matching_geofence(40.0, 2.0, &fences), None);
    }
}
