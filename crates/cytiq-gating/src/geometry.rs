//! Point-membership predicates for `GatingML` gate shapes.
//!
//! Semantics follow `FlowKit` / `flowutils` so results match the Python
//! reference implementation exactly.

use rayon::prelude::*;

/// `min <= x < max` on every dimension; a missing bound is unbounded.
#[must_use]
pub fn in_rectangle(columns: &[&[f64]], bounds: &[(Option<f64>, Option<f64>)]) -> Vec<bool> {
    let event_count = columns.first().map_or(0, |column| column.len());
    (0..event_count)
        .into_par_iter()
        .map(|event| {
            columns.iter().zip(bounds).all(|(column, (min, max))| {
                let value = column[event];
                min.is_none_or(|min| value >= min) && max.is_none_or(|max| value < max)
            })
        })
        .collect()
}

/// Winding-number polygon membership, as in `flowutils` `points_in_polygon`.
///
/// A point is inside when its winding number is odd, so self-intersecting
/// polygons get "holes" where boundaries overlap. Following Sunday's
/// algorithm, points on left/bottom edges are inside and points on
/// right/top edges are outside.
#[must_use]
pub fn in_polygon(x: &[f64], y: &[f64], vertices: &[[f64; 2]]) -> Vec<bool> {
    x.par_iter()
        .zip(y.par_iter())
        .map(|(&px, &py)| winding_number(px, py, vertices) % 2 != 0)
        .collect()
}

fn winding_number(px: f64, py: f64, vertices: &[[f64; 2]]) -> i32 {
    let mut winding = 0;
    for (index, start) in vertices.iter().enumerate() {
        let end = vertices[(index + 1) % vertices.len()];
        let is_left = (end[0] - start[0]) * (py - start[1]) - (px - start[0]) * (end[1] - start[1]);
        if start[1] <= py {
            if end[1] > py && is_left > 0.0 {
                winding += 1;
            }
        } else if end[1] <= py && is_left < 0.0 {
            winding -= 1;
        }
    }
    winding
}

/// Mahalanobis ellipsoid membership (boundary inclusive), as in `flowutils`
/// `points_in_ellipsoid`. `inverse_covariance` is row-major `n × n`.
#[must_use]
pub fn in_ellipsoid(
    columns: &[&[f64]],
    mean: &[f64],
    inverse_covariance: &[f64],
    distance_square: f64,
) -> Vec<bool> {
    let size = mean.len();
    let event_count = columns.first().map_or(0, |column| column.len());
    (0..event_count)
        .into_par_iter()
        .map_init(
            || vec![0.0; size],
            |delta, event| {
                for (dimension, value) in delta.iter_mut().enumerate() {
                    *value = columns[dimension][event] - mean[dimension];
                }
                let distance: f64 = (0..size)
                    .map(|column| {
                        let rotated: f64 = (0..size)
                            .map(|row| delta[row] * inverse_covariance[row * size + column])
                            .sum();
                        rotated * delta[column]
                    })
                    .sum();
                distance <= distance_square
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SQUARE: [[f64; 2]; 4] = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];

    fn polygon(points: &[[f64; 2]], vertices: &[[f64; 2]]) -> Vec<bool> {
        let x = points.iter().map(|point| point[0]).collect::<Vec<_>>();
        let y = points.iter().map(|point| point[1]).collect::<Vec<_>>();
        in_polygon(&x, &y, vertices)
    }

    #[test]
    fn rectangle_includes_min_and_excludes_max() {
        let x = [0.0, 1.0, 1.5, 2.0, 3.0];
        let result = in_rectangle(&[&x], &[(Some(1.0), Some(2.0))]);
        assert_eq!(result, [false, true, true, false, false]);
    }

    #[test]
    fn rectangle_missing_bound_is_unbounded() {
        let x = [-1e300, 0.0, 5.0];
        let y = [0.0, 0.0, 100.0];
        let result = in_rectangle(&[&x, &y], &[(None, Some(1.0)), (Some(0.0), None)]);
        assert_eq!(result, [true, true, false]);
    }

    #[test]
    fn rectangle_rejects_nan() {
        let x = [f64::NAN];
        assert_eq!(in_rectangle(&[&x], &[(None, Some(1.0))]), [false]);
    }

    #[test]
    fn polygon_edges_match_flowutils() {
        // Expected values were produced by flowutils.gating_c.points_in_polygon.
        let points = [
            [5.0, 5.0],
            [0.0, 5.0],
            [10.0, 5.0],
            [5.0, 0.0],
            [5.0, 10.0],
            [0.0, 0.0],
            [10.0, 10.0],
            [10.0, 0.0],
            [0.0, 10.0],
            [-1.0, 5.0],
            [11.0, 5.0],
        ];
        let expected = [
            true, true, false, true, false, true, false, false, false, false, false,
        ];
        assert_eq!(polygon(&points, &SQUARE), expected);
        let mut reversed = SQUARE;
        reversed.reverse();
        assert_eq!(polygon(&points, &reversed), expected);
    }

    #[test]
    fn polygon_handles_concave_shapes() {
        let arrow = [
            [0.0, 0.0],
            [10.0, 0.0],
            [5.0, 5.0],
            [10.0, 10.0],
            [0.0, 10.0],
        ];
        assert_eq!(
            polygon(&[[2.0, 5.0], [8.0, 5.0], [8.0, 1.0]], &arrow),
            [true, false, true]
        );
    }

    #[test]
    fn self_intersecting_polygon_excludes_even_winding() {
        let star = (0..5)
            .map(|index| {
                let angle = std::f64::consts::FRAC_PI_2
                    + f64::from(index) * 4.0 * std::f64::consts::PI / 5.0;
                [angle.cos(), angle.sin()]
            })
            .collect::<Vec<_>>();
        assert_eq!(
            polygon(&[[0.0, 0.0], [0.0, 0.7], [2.0, 2.0]], &star),
            [false, true, false]
        );
    }

    #[test]
    fn polygon_rejects_nan() {
        assert_eq!(polygon(&[[f64::NAN, 5.0]], &SQUARE), [false]);
    }

    #[test]
    fn ellipsoid_includes_boundary() {
        // Axis-aligned ellipse x²/4 + y² <= 1.
        let x = [0.0, 2.0, 2.1, 0.0];
        let y = [0.0, 0.0, 0.0, 1.0];
        let inverse = [0.25, 0.0, 0.0, 1.0];
        assert_eq!(
            in_ellipsoid(&[&x, &y], &[0.0, 0.0], &inverse, 1.0),
            [true, true, false, true]
        );
    }
}
