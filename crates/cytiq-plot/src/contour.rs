//! Equal-probability contours of a binned 2-D distribution.
//!
//! The histogram is smoothed with a separable Gaussian whose width follows
//! Scott's rule (`h = spread · n^(-1/6)` per axis, with a robust spread), so
//! sparse populations get smooth contours and isolated events fall outside
//! the outer contour as outliers. Thresholds are then chosen so the region
//! above the `k`-th threshold holds `1 − k · step` of the *events*
//! (`FlowJo`'s equal-probability contouring): cells are ranked by the
//! smoothed surface but weighed by the events they hold. Weighing them by the
//! smoothed mass instead lets the smoothing set the fractions: a wide kernel
//! over separate clusters spreads mass into empty space, and the `50%`
//! contour of the CD4/CD8/DN T cells of the pipeline fixture held 64% of
//! them. Marching
//! squares on a zero-padded grid gives closed lines; segment ends are keyed by
//! the integer id of the grid edge they lie on, so stitching is exact and the
//! output deterministic.

use std::collections::HashMap;

use crate::bin::Histogram;
use crate::spec::Polyline;

/// Smallest smoothing, in cells.
const MIN_SIGMA_CELLS: f64 = 1.0;
/// Largest smoothing, as a fraction of the grid.
const MAX_SIGMA_FRACTION: f64 = 1.0 / 16.0;

/// Smoothed cell values, row-major from the bottom row.
pub(crate) struct Surface {
    pub bins: usize,
    pub values: Vec<f64>,
}

/// Gaussian width per axis, in cells, for `points` in unit coordinates:
/// Scott's rule `spread · n^(-1/6)` with the MAD (else IQR) as spread,
/// clamped to `[1, bins / 16]` cells.
pub(crate) fn bandwidth(points: &[[f64; 2]], bins: usize) -> [f64; 2] {
    #[allow(clippy::cast_precision_loss, reason = "bins is at most 1024")]
    let cells = bins as f64;
    let max = (cells * MAX_SIGMA_FRACTION).max(MIN_SIGMA_CELLS);
    if points.len() < 2 {
        return [MIN_SIGMA_CELLS; 2];
    }
    #[allow(clippy::cast_precision_loss, reason = "event counts fit f64")]
    let factor = (points.len() as f64).powf(-1.0 / 6.0);
    let axis = |index: usize| {
        let values = points.iter().map(|point| point[index]).collect::<Vec<_>>();
        let center = cytiq_density::median(&values);
        let (spread, _) = cytiq_density::robust_spread(&values, center);
        (spread * factor * cells).clamp(MIN_SIGMA_CELLS, max)
    };
    [axis(0), axis(1)]
}

/// Smooth a histogram with a Gaussian of `sigma` cells per axis (radius
/// 3σ), zero outside the grid.
pub(crate) fn smooth(histogram: &Histogram, [sigma_x, sigma_y]: [f64; 2]) -> Surface {
    let bins = histogram.bins;
    let (kernel_x, kernel_y) = (gaussian_kernel(sigma_x), gaussian_kernel(sigma_y));
    let input = histogram
        .counts
        .iter()
        .map(|&count| f64::from(count))
        .collect::<Vec<_>>();
    let mut rows = vec![0.0; bins * bins];
    let radius = kernel_x.len() / 2;
    for row in 0..bins {
        for column in 0..bins {
            let mut sum = 0.0;
            for (offset, weight) in kernel_x.iter().enumerate() {
                let source = (column + offset).checked_sub(radius);
                if let Some(source) = source.filter(|&source| source < bins) {
                    sum += weight * input[row * bins + source];
                }
            }
            rows[row * bins + column] = sum;
        }
    }
    let mut values = vec![0.0; bins * bins];
    let radius = kernel_y.len() / 2;
    for row in 0..bins {
        for column in 0..bins {
            let mut sum = 0.0;
            for (offset, weight) in kernel_y.iter().enumerate() {
                let source = (row + offset).checked_sub(radius);
                if let Some(source) = source.filter(|&source| source < bins) {
                    sum += weight * rows[source * bins + column];
                }
            }
            values[row * bins + column] = sum;
        }
    }
    Surface { bins, values }
}

fn gaussian_kernel(sigma: f64) -> Vec<f64> {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "sigma is between 1 and 64 cells"
    )]
    let radius = (3.0 * sigma).ceil() as usize;
    let weights = (0..=2 * radius)
        .map(|index| {
            #[allow(clippy::cast_precision_loss, reason = "small index")]
            let distance = index as f64 - radius as f64;
            (-0.5 * (distance / sigma).powi(2)).exp()
        })
        .collect::<Vec<_>>();
    let total = weights.iter().sum::<f64>();
    weights.into_iter().map(|weight| weight / total).collect()
}

/// Thresholds, outermost (lowest) first, whose super-level sets of `surface`
/// hold `1 − step, 1 − 2·step, …` of the events of `histogram` (the surface
/// smooths it). Empty when the histogram holds no events.
pub(crate) fn probability_thresholds(
    surface: &Surface,
    histogram: &Histogram,
    step: f64,
) -> Vec<f64> {
    // Occupied cells, densest first by the smoothed surface, with their events.
    let mut cells = histogram
        .counts
        .iter()
        .zip(&surface.values)
        .filter(|(count, _)| **count > 0)
        .map(|(count, value)| (*value, f64::from(*count)))
        .collect::<Vec<_>>();
    let total = cells.iter().map(|(_, events)| events).sum::<f64>();
    if total <= 0.0 {
        return Vec::new();
    }
    cells.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
    let mut thresholds = Vec::new();
    let mut level = 1;
    loop {
        #[allow(clippy::cast_precision_loss, reason = "few levels")]
        let enclosed = 1.0 - step * f64::from(level);
        if enclosed <= 1e-9 {
            break;
        }
        let mut cumulative = 0.0;
        let mut threshold = cells[cells.len() - 1].0;
        for (value, events) in &cells {
            cumulative += events;
            if cumulative >= enclosed * total {
                threshold = *value;
                break;
            }
        }
        if thresholds.last().is_none_or(|last| threshold > *last) {
            thresholds.push(threshold);
        }
        level += 1;
    }
    thresholds
}

/// Closed contour lines of `surface` at `level`, in unit coordinates (grid
/// points at cell centres).
pub(crate) fn contour_lines(surface: &Surface, level: f64) -> Vec<Polyline> {
    let grid = Padded { surface };
    let segments = grid.segments(level);
    stitch(&segments)
        .into_iter()
        .map(|(edges, closed)| Polyline {
            points: edges
                .into_iter()
                .map(|edge| grid.crossing(edge, level))
                .collect(),
            closed,
        })
        .collect()
}

/// The surface with a ring of zeros around it, so every contour closes.
/// Grid edges have integer ids: `2 · (row · width + column)` for the edge to
/// the right of a point, `+ 1` for the edge above it.
struct Padded<'a> {
    surface: &'a Surface,
}

impl Padded<'_> {
    fn width(&self) -> usize {
        self.surface.bins + 2
    }

    fn value(&self, column: usize, row: usize) -> f64 {
        let bins = self.surface.bins;
        if column == 0 || row == 0 || column > bins || row > bins {
            0.0
        } else {
            self.surface.values[(row - 1) * bins + (column - 1)]
        }
    }

    fn horizontal(&self, column: usize, row: usize) -> usize {
        2 * (row * self.width() + column)
    }

    fn vertical(&self, column: usize, row: usize) -> usize {
        2 * (row * self.width() + column) + 1
    }

    /// Marching squares: one or two segments (pairs of edge ids) per cell
    /// the level crosses, cells in row-major order.
    fn segments(&self, level: f64) -> Vec<[usize; 2]> {
        let mut segments = Vec::new();
        for row in 0..self.width() - 1 {
            for column in 0..self.width() - 1 {
                let corners = [
                    self.value(column, row),
                    self.value(column + 1, row),
                    self.value(column + 1, row + 1),
                    self.value(column, row + 1),
                ];
                let case = corners
                    .iter()
                    .enumerate()
                    .fold(0_u8, |case, (bit, corner)| {
                        case | (u8::from(*corner >= level) << bit)
                    });
                let bottom = self.horizontal(column, row);
                let top = self.horizontal(column, row + 1);
                let left = self.vertical(column, row);
                let right = self.vertical(column + 1, row);
                let center_above = corners.iter().sum::<f64>() / 4.0 >= level;
                match case {
                    1 | 14 => segments.push([left, bottom]),
                    2 | 13 => segments.push([bottom, right]),
                    3 | 12 => segments.push([left, right]),
                    4 | 11 => segments.push([right, top]),
                    6 | 9 => segments.push([bottom, top]),
                    7 | 8 => segments.push([left, top]),
                    // Saddles, resolved by the mean of the four corners:
                    // bottom-left and top-right above the level ...
                    5 if center_above => segments.extend([[left, top], [bottom, right]]),
                    5 => segments.extend([[left, bottom], [right, top]]),
                    // ... or bottom-right and top-left above it.
                    10 if center_above => segments.extend([[left, bottom], [right, top]]),
                    10 => segments.extend([[bottom, right], [left, top]]),
                    _ => {}
                }
            }
        }
        segments
    }

    /// Where the contour crosses a grid edge, in unit coordinates.
    fn crossing(&self, edge: usize, level: f64) -> [f64; 2] {
        let point = edge / 2;
        let (column, row) = (point % self.width(), point / self.width());
        let (end_column, end_row) = if edge.is_multiple_of(2) {
            (column + 1, row)
        } else {
            (column, row + 1)
        };
        let (start, end) = (self.value(column, row), self.value(end_column, end_row));
        let fraction = if (end - start).abs() > 0.0 {
            ((level - start) / (end - start)).clamp(0.0, 1.0)
        } else {
            0.5
        };
        #[allow(clippy::cast_precision_loss, reason = "grid indices are small")]
        let (grid_x, grid_y, cells) = (
            column as f64 + fraction * (end_column as f64 - column as f64),
            row as f64 + fraction * (end_row as f64 - row as f64),
            self.surface.bins as f64,
        );
        // Padded point (1, 1) is the centre of cell (0, 0).
        [(grid_x - 0.5) / cells, (grid_y - 0.5) / cells]
    }
}

/// Join segments that share an edge into chains of edge ids, each with
/// whether it closes on itself. Chains start at the first unused segment in
/// creation order, so the result is deterministic.
fn stitch(segments: &[[usize; 2]]) -> Vec<(Vec<usize>, bool)> {
    let mut by_edge: HashMap<usize, Vec<usize>> = HashMap::new();
    for (index, [start, end]) in segments.iter().enumerate() {
        by_edge.entry(*start).or_default().push(index);
        by_edge.entry(*end).or_default().push(index);
    }
    let mut used = vec![false; segments.len()];
    let mut chains = Vec::new();
    for first in 0..segments.len() {
        if used[first] {
            continue;
        }
        used[first] = true;
        let [start, mut current] = segments[first];
        let mut edges = vec![start, current];
        let mut closed = false;
        while let Some(next) = by_edge
            .get(&current)
            .and_then(|candidates| candidates.iter().copied().find(|&segment| !used[segment]))
        {
            used[next] = true;
            let [a, b] = segments[next];
            current = if a == current { b } else { a };
            if current == start {
                closed = true;
                break;
            }
            edges.push(current);
        }
        chains.push((edges, closed));
    }
    chains
}

#[cfg(test)]
#[allow(clippy::cast_precision_loss, reason = "test statistics")]
mod tests {
    use super::*;

    /// Deterministic standard normal pairs (Box–Muller on a 64-bit LCG).
    fn gaussian_points(count: usize, center: [f64; 2], spread: f64) -> Vec<[f64; 2]> {
        let mut state = 0x2545_F491_4F6C_DD1D_u64;
        let mut uniform = move || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            #[allow(clippy::cast_precision_loss, reason = "53-bit mantissa")]
            let value = (state >> 11) as f64 / (1_u64 << 53) as f64;
            value.max(1e-12)
        };
        (0..count)
            .map(|_| {
                let (u, v) = (uniform(), uniform());
                let radius = (-2.0 * u.ln()).sqrt();
                let angle = std::f64::consts::TAU * v;
                [
                    center[0] + spread * radius * angle.cos(),
                    center[1] + spread * radius * angle.sin(),
                ]
            })
            .collect()
    }

    #[test]
    fn contour_of_a_radial_bump_is_closed() {
        let points = gaussian_points(20_000, [0.5, 0.5], 0.1);
        let histogram = Histogram::new(64, points.iter().copied());
        let surface = smooth(&histogram, bandwidth(&points, 64));
        let thresholds = probability_thresholds(&surface, &histogram, 0.1);
        assert_eq!(thresholds.len(), 9);
        assert!(thresholds.windows(2).all(|pair| pair[0] < pair[1]));
        let lines = contour_lines(&surface, thresholds[0]);
        let longest = lines.iter().max_by_key(|line| line.points.len()).unwrap();
        assert!(longest.closed);
        // The 90% contour of an isotropic Gaussian (σ = 0.1) has radius
        // σ·√(−2 ln 0.1) ≈ 0.215.
        let mean_radius = longest
            .points
            .iter()
            .map(|[x, y]| ((x - 0.5).powi(2) + (y - 0.5).powi(2)).sqrt())
            .sum::<f64>()
            / longest.points.len() as f64;
        assert!((mean_radius - 0.215).abs() < 0.02, "{mean_radius}");
    }

    #[test]
    fn outer_contour_encloses_requested_mass() {
        let points = gaussian_points(50_000, [0.4, 0.6], 0.08);
        let histogram = Histogram::new(128, points.iter().copied());
        let surface = smooth(&histogram, bandwidth(&points, 128));
        let outer = probability_thresholds(&surface, &histogram, 0.1)[0];
        let inside = points
            .iter()
            .filter(|point| surface.values[histogram.cell_of(**point)] >= outer)
            .count();
        let fraction = inside as f64 / points.len() as f64;
        assert!((fraction - 0.9).abs() < 0.02, "{fraction}");
    }

    #[test]
    fn saddle_cells_yield_two_non_crossing_segments() {
        // Two diagonal peaks in a 2×2 grid: a saddle in the middle cell.
        let surface = Surface {
            bins: 2,
            values: vec![1.0, 0.0, 0.0, 1.0],
        };
        // Centre mean 0.5 < 0.6: the two peaks stay separate.
        let separate = contour_lines(&surface, 0.6);
        assert_eq!(separate.len(), 2);
        assert!(separate.iter().all(|line| line.closed));
        // Centre mean 0.5 ≥ 0.4: the peaks join into one region.
        let joined = contour_lines(&surface, 0.4);
        assert_eq!(joined.len(), 1);
        assert!(joined[0].closed);
    }

    #[test]
    fn sparse_events_get_wider_smoothing_and_fall_outside_the_outer_contour() {
        let dense = gaussian_points(100_000, [0.5, 0.5], 0.1);
        let sparse = gaussian_points(1_000, [0.5, 0.5], 0.1);
        let [dense_x, _] = bandwidth(&dense, 256);
        let [sparse_x, _] = bandwidth(&sparse, 256);
        assert!(sparse_x > dense_x * 2.0, "{sparse_x} vs {dense_x}");
        // A far-away single event must not get its own contour.
        let mut points = sparse.clone();
        points.push([0.95, 0.05]);
        let histogram = Histogram::new(256, points.iter().copied());
        let surface = smooth(&histogram, bandwidth(&points, 256));
        let outer = probability_thresholds(&surface, &histogram, 0.1)[0];
        assert!(surface.values[histogram.cell_of([0.95, 0.05])] < outer);
    }

    #[test]
    fn empty_surface_has_no_thresholds() {
        let histogram = Histogram::new(16, std::iter::empty());
        let surface = smooth(&histogram, [1.0, 1.0]);
        assert_eq!(
            probability_thresholds(&surface, &histogram, 0.1),
            Vec::<f64>::new()
        );
    }

    /// Every level of three separate clusters of different sizes holds its
    /// share of the events, although the robust spread of all of them sets a
    /// wide kernel that spreads their mass into the space between them.
    #[test]
    fn every_level_of_separate_clusters_holds_its_share_of_the_events() {
        let mut points = gaussian_points(6_000, [0.25, 0.3], 0.03);
        points.extend(gaussian_points(3_000, [0.7, 0.7], 0.05));
        points.extend(gaussian_points(1_000, [0.3, 0.75], 0.02));
        let histogram = Histogram::new(256, points.iter().copied());
        let surface = smooth(&histogram, bandwidth(&points, 256));
        let thresholds = probability_thresholds(&surface, &histogram, 0.1);
        assert_eq!(thresholds.len(), 9);
        for (level, threshold) in thresholds.iter().enumerate() {
            let inside = points
                .iter()
                .filter(|point| surface.values[histogram.cell_of(**point)] >= *threshold)
                .count();
            let fraction = inside as f64 / points.len() as f64;
            let expected = 1.0 - 0.1 * (level + 1) as f64;
            assert!(
                (fraction - expected).abs() < 0.01,
                "level {level}: {fraction}"
            );
        }
    }
}
