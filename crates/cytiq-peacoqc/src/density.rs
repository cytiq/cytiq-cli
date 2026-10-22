//! Port of R's `stats::density.default` with the defaults `PeacoQC` relies on:
//! Gaussian kernel, `bw = "nrd0"`, `adjust = 1`, `cut = 3`, `n = 512`.
//!
//! The implementation mirrors R step by step: linear binning onto a `2n` grid
//! (`BinDist`), circular convolution with the kernel sampled on R's
//! `kords` grid, and linear interpolation (`approx`) back onto `n` points
//! between `min(x) - cut * bw` and `max(x) + cut * bw`.

#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

const GRID_POINTS: usize = 512;
const CUT: f64 = 3.0;

/// Kernel density estimate evaluated on an evenly spaced grid.
#[derive(Clone, Debug, PartialEq)]
pub struct Density {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
}

/// Evaluate `stats::density(values)` with R's defaults.
///
/// `values` must be finite and contain at least two observations.
#[must_use]
pub fn density(values: &[f64]) -> Density {
    let bandwidth = bandwidth_nrd0(values);
    let total = values.len() as f64;
    let minimum = values.iter().copied().fold(f64::INFINITY, f64::min);
    let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let from = minimum - CUT * bandwidth;
    let to = maximum + CUT * bandwidth;
    let lo = from - 4.0 * bandwidth;
    let up = to + 4.0 * bandwidth;

    let n = GRID_POINTS;
    let binned = bin_dist(values, 1.0 / total, lo, up, n);

    // kords <- seq.int(0, (2n - 1) / (n - 1) * (up - lo), length.out = 2n)
    // (R >= 4.4, old.coords = FALSE), with the tail mirrored to negative lags.
    let kernel_step = (up - lo) / (n - 1) as f64;
    let kernel = (0..2 * n)
        .map(|index| {
            let offset = if index <= n { index } else { 2 * n - index };
            gaussian(offset as f64 * kernel_step, bandwidth)
        })
        .collect::<Vec<_>>();

    // Re(fft(fft(y) * Conj(fft(kords)), inverse = TRUE)) / length(y) is the
    // circular cross-correlation sum_j y[j] * kords[(j - k) mod 2n].
    let estimates = (0..n)
        .map(|shift| {
            let value = binned
                .iter()
                .enumerate()
                .filter(|(_, weight)| **weight != 0.0)
                .map(|(index, weight)| weight * kernel[(index + 2 * n - shift) % (2 * n)])
                .sum::<f64>();
            value.max(0.0)
        })
        .collect::<Vec<_>>();

    let grid_step = (up - lo) / (n - 1) as f64;
    let output_step = (to - from) / (n - 1) as f64;
    let x = (0..n)
        .map(|index| from + index as f64 * output_step)
        .collect::<Vec<_>>();
    let y = x
        .iter()
        .map(|point| {
            let position = (point - lo) / grid_step;
            let lower = (position.floor() as usize).min(n - 2);
            let fraction = position - lower as f64;
            estimates[lower] + (estimates[lower + 1] - estimates[lower]) * fraction
        })
        .collect();

    Density { x, y }
}

/// R's `bw.nrd0`: `0.9 * min(sd, IQR / 1.34) * n^-0.2` with its fallbacks.
#[must_use]
pub fn bandwidth_nrd0(values: &[f64]) -> f64 {
    let count = values.len() as f64;
    let mean = values.iter().sum::<f64>() / count;
    let variance = values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>()
        / (count - 1.0);
    let standard_deviation = variance.sqrt();
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let interquartile = quantile_type7(&sorted, 0.75) - quantile_type7(&sorted, 0.25);
    let mut low = standard_deviation.min(interquartile / 1.34);
    if low <= 0.0 {
        low = standard_deviation;
        if low <= 0.0 {
            low = values[0].abs();
            if low <= 0.0 {
                low = 1.0;
            }
        }
    }
    0.9 * low * count.powf(-0.2)
}

/// R's default (type 7) quantile on already sorted data.
#[must_use]
pub fn quantile_type7(sorted: &[f64], probability: f64) -> f64 {
    let position = probability * (sorted.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    sorted[lower] + (sorted[upper] - sorted[lower]) * (position - lower as f64)
}

/// R's `BinDist` (src/library/stats/src/massdist.c): linear binning of
/// weighted observations onto `2n` cells, of which the first `n` are used.
fn bin_dist(values: &[f64], weight: f64, lo: f64, hi: f64, n: usize) -> Vec<f64> {
    let mut binned = vec![0.0; 2 * n];
    let max_index = n as i64 - 2;
    let delta = (hi - lo) / (n - 1) as f64;
    for value in values {
        let position = (value - lo) / delta;
        let index = position.floor() as i64;
        let fraction = position - index as f64;
        if (0..=max_index).contains(&index) {
            binned[index as usize] += weight * (1.0 - fraction);
            binned[index as usize + 1] += weight * fraction;
        } else if index == -1 {
            binned[0] += weight * fraction;
        } else if index == max_index + 1 {
            binned[index as usize] += weight * (1.0 - fraction);
        }
    }
    binned
}

fn gaussian(value: f64, standard_deviation: f64) -> f64 {
    let z = value / standard_deviation;
    (-0.5 * z * z).exp() / (standard_deviation * (2.0 * std::f64::consts::PI).sqrt())
}

#[cfg(test)]
#[allow(clippy::unreadable_literal, clippy::excessive_precision)]
mod tests {
    use super::*;

    // Reference values from R 4.5:
    //   x <- c(1, 2, 2.5, 3, 7, 8, 8.2, 9)
    //   bw.nrd0(x); d <- density(x); d$x[c(1, 256, 512)]; d$y[c(1, 100, 256, 400, 512)]
    const SAMPLE: [f64; 8] = [1.0, 2.0, 2.5, 3.0, 7.0, 8.0, 8.2, 9.0];

    #[test]
    fn bandwidth_matches_r_bw_nrd0() {
        assert!((bandwidth_nrd0(&SAMPLE) - R_BW).abs() < 1e-12);
    }

    #[test]
    fn density_matches_r_density() {
        let estimate = density(&SAMPLE);
        assert_eq!(estimate.x.len(), 512);
        for (index, expected) in R_X {
            assert!((estimate.x[index] - expected).abs() < 1e-10, "x[{index}]");
        }
        for (index, expected) in R_Y {
            assert!(
                (estimate.y[index] - expected).abs() < 1e-9,
                "y[{index}] = {} vs {expected}",
                estimate.y[index]
            );
        }
    }

    include!("../../../data/fixtures/peacoqc/density_reference.rs");
}
