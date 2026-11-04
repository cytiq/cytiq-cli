//! Pure statistics over one channel's values.

#![allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]

use serde::Serialize;

/// Distribution of the finite values of a channel; `None` when it has none.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Summary {
    pub finite: usize,
    pub non_finite: usize,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
    pub sd: Option<f64>,
    pub p1: Option<f64>,
    pub p50: Option<f64>,
    pub p99: Option<f64>,
}

pub fn summarize(values: &[f64]) -> Summary {
    let mut finite: Vec<f64> = values
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .collect();
    let non_finite = values.len() - finite.len();
    if finite.is_empty() {
        return Summary {
            non_finite,
            ..Summary::default()
        };
    }
    let count = finite.len() as f64;
    let mean = finite.iter().sum::<f64>() / count;
    let variance = finite
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / count;
    let min = finite.iter().copied().fold(f64::INFINITY, f64::min);
    let max = finite.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Summary {
        finite: finite.len(),
        non_finite,
        min: Some(min),
        max: Some(max),
        mean: Some(mean),
        sd: Some(variance.sqrt()),
        p1: Some(percentile(&mut finite, 1.0)),
        p50: Some(percentile(&mut finite, 50.0)),
        p99: Some(percentile(&mut finite, 99.0)),
    }
}

/// Nearest-rank percentile (`p` in 0..=100) of non-empty finite values.
pub fn percentile(values: &mut [f64], p: f64) -> f64 {
    let rank = ((p / 100.0) * (values.len() - 1) as f64).round() as usize;
    let (_, value, _) = values.select_nth_unstable_by(rank, f64::total_cmp);
    *value
}

/// Counts of `values` in `bins` equal-width bins of `transform(value)` over
/// `[lo, hi]`; values outside the range land in the first or last bin.
pub fn bin(
    values: &[f64],
    lo: f64,
    hi: f64,
    bins: usize,
    transform: impl Fn(f64) -> f64,
) -> Vec<u64> {
    let mut counts = vec![0_u64; bins];
    let width = (hi - lo) / bins as f64;
    for value in values.iter().copied().filter(|value| value.is_finite()) {
        let position = if width > 0.0 {
            ((transform(value) - lo) / width).floor()
        } else {
            0.0
        };
        let index = position.clamp(0.0, (bins - 1) as f64) as usize;
        counts[index] += 1;
    }
    counts
}

/// How often consecutive values decrease (zero for a monotonic clock).
pub fn decreases(values: &[f64]) -> usize {
    values.windows(2).filter(|pair| pair[1] < pair[0]).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_finite_values_and_counts_the_rest() {
        let summary = summarize(&[1.0, 2.0, 3.0, 4.0, f64::NAN, f64::INFINITY]);

        assert_eq!(summary.finite, 4);
        assert_eq!(summary.non_finite, 2);
        assert_eq!((summary.min, summary.max), (Some(1.0), Some(4.0)));
        assert_eq!(summary.mean, Some(2.5));
        assert!((summary.sd.expect("sd") - 1.118_033_988_749_895).abs() < 1e-12);
        assert_eq!(summary.p50, Some(3.0));
    }

    #[test]
    fn a_channel_without_finite_values_has_no_distribution() {
        let summary = summarize(&[f64::NAN]);

        assert_eq!(summary.non_finite, 1);
        assert_eq!(summary.mean, None);
    }

    #[test]
    fn percentiles_use_the_nearest_rank() {
        let mut values: Vec<f64> = (0..=100).map(f64::from).rev().collect();

        assert!((percentile(&mut values, 1.0) - 1.0).abs() < f64::EPSILON);
        assert!((percentile(&mut values, 99.0) - 99.0).abs() < f64::EPSILON);
    }

    #[test]
    fn bins_clamp_out_of_range_values() {
        let counts = bin(
            &[-5.0, 0.0, 4.9, 5.0, 9.99, 10.0, 50.0, f64::NAN],
            0.0,
            10.0,
            2,
            |value| value,
        );

        assert_eq!(counts, vec![3, 4]);
    }

    #[test]
    fn a_degenerate_range_puts_everything_in_the_first_bin() {
        assert_eq!(
            bin(&[3.0, 3.0], 3.0, 3.0, 4, |value| value),
            vec![2, 0, 0, 0]
        );
    }

    #[test]
    fn decreases_count_clock_resets() {
        assert_eq!(decreases(&[0.0, 1.0, 2.0, 0.5, 1.0]), 1);
        assert_eq!(decreases(&[0.0, 0.0, 1.0]), 0);
    }
}
