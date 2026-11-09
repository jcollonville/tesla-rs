// Copyright (C) 2026 Jean Collonvillé
// SPDX-License-Identifier: AGPL-3.0-only

/// Indices kept by the Ramer-Douglas-Peucker algorithm.
///
/// `epsilon` is the perpendicular distance in the same units as the points
/// (degrees, for a latitude/longitude polyline).
pub(crate) fn rdp_indices(points: &[[f64; 2]], epsilon: f64) -> Vec<usize> {
    let len = points.len();
    if len <= 2 {
        return (0..len).collect();
    }
    let mut keep = vec![false; len];
    keep[0] = true;
    keep[len - 1] = true;
    mark(points, 0, len - 1, epsilon, &mut keep);
    keep.into_iter()
        .enumerate()
        .filter_map(|(index, kept)| kept.then_some(index))
        .collect()
}

/// Simplifies a polyline until it has at most `max_points` vertices.
pub(crate) fn rdp_limit(points: &[[f64; 2]], max_points: usize) -> Vec<usize> {
    if max_points == 0 || points.len() <= max_points {
        return (0..points.len()).collect();
    }
    let mut epsilon = 1e-6;
    let mut chosen = rdp_indices(points, epsilon);
    for _ in 0..24 {
        if chosen.len() <= max_points {
            return chosen;
        }
        epsilon *= 1.8;
        chosen = rdp_indices(points, epsilon);
    }
    if chosen.len() <= max_points {
        chosen
    } else {
        stride_indices(points.len(), max_points)
    }
}

/// Evenly spaced indices, always including the first and the last.
pub(crate) fn stride_indices(len: usize, max_points: usize) -> Vec<usize> {
    if max_points == 0 || len <= max_points {
        return (0..len).collect();
    }
    if max_points == 1 || len == 0 {
        return vec![0];
    }
    let last = len - 1;
    let slots = max_points - 1;
    let mut out = Vec::with_capacity(max_points);
    for step in 0..max_points {
        let index = (step * last) / slots;
        if out.last().copied() != Some(index) {
            out.push(index);
        }
    }
    if out.last().copied() != Some(last) {
        out.push(last);
    }
    out
}

fn mark(points: &[[f64; 2]], start: usize, end: usize, epsilon: f64, keep: &mut [bool]) {
    if end <= start + 1 {
        return;
    }
    let mut max_distance = 0.0;
    let mut farthest = start;
    for index in (start + 1)..end {
        let distance = perpendicular(points[start], points[end], points[index]);
        if distance > max_distance {
            max_distance = distance;
            farthest = index;
        }
    }
    if max_distance > epsilon {
        keep[farthest] = true;
        mark(points, start, farthest, epsilon, keep);
        mark(points, farthest, end, epsilon, keep);
    }
}

fn perpendicular(start: [f64; 2], end: [f64; 2], point: [f64; 2]) -> f64 {
    let dx = end[0] - start[0];
    let dy = end[1] - start[1];
    let len_sq = dx * dx + dy * dy;
    if len_sq <= f64::EPSILON {
        return distance(start, point);
    }
    let raw = ((point[0] - start[0]) * dx + (point[1] - start[1]) * dy) / len_sq;
    let t = raw.clamp(0.0, 1.0);
    distance([start[0] + t * dx, start[1] + t * dy], point)
}

fn distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    let dx = left[0] - right[0];
    let dy = left[1] - right[1];
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::{rdp_indices, rdp_limit, stride_indices};

    #[test]
    fn straight_line_collapses_and_a_corner_is_kept() {
        let line: Vec<[f64; 2]> = (0..20).map(|i| [f64::from(i), 0.0]).collect();
        let kept = rdp_indices(&line, 1e-6);
        assert_eq!(kept, vec![0, 19]);

        let mut corner = line;
        corner[10] = [10.0, 5.0];
        let kept = rdp_indices(&corner, 0.1);
        assert!(kept.contains(&10));
        assert_eq!(*kept.first().unwrap(), 0);
        assert_eq!(*kept.last().unwrap(), 19);
    }

    #[test]
    fn limit_and_stride_keep_endpoints() {
        let points: Vec<[f64; 2]> = (0..50).map(|i| [f64::from(i), f64::from(i % 7)]).collect();
        let kept = rdp_limit(&points, 8);
        assert!(kept.len() <= 8);
        assert_eq!(kept[0], 0);
        assert_eq!(*kept.last().unwrap(), 49);

        let stride = stride_indices(10, 4);
        assert_eq!(stride.first().copied(), Some(0));
        assert_eq!(stride.last().copied(), Some(9));
        assert!(stride.len() <= 4);
    }
}
