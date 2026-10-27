//! Port of `flowDensity::deGate(x, all.cuts = TRUE, upper = TRUE)` (flowDensity
//! 1.44, `helper_functions.R`), the per-channel cut-off search `CompensAID`
//! relies on.
//!
//! The density is R's `density(x)` smoothed by `smooth.spline(spar = 0.4)` and
//! clamped at zero (`.densityForPlot`). Peaks are strict local maxima above
//! `tinypeak_removal * max(y)` (`.getPeaks`, `twin.factor = 1`). With two or
//! more peaks every valley is returned: the lowest grid point strictly between
//! adjacent peaks (`.getIntersect`). With one peak the cut-off is the first
//! flat, low point of the right tail (`.trackSlope(upper = TRUE)`), or the end
//! of the grid when there is none.
//!
//! `.getIntersect` ranks the grid points by R's `splinefun(dens)` at the same
//! points, an interpolating spline that returns the density up to round-off.
//! The ranking is therefore the density's, except across a gap where the
//! clamped density is exactly zero: there R's pick among the zeros follows
//! that round-off (about 1e-21, set by every earlier step and by the compiler),
//! which no port reproduces. This port takes the first zero.

#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::many_single_char_names
)]

use crate::kde::density;
use crate::smooth_spline::smooth_spline_fitted_xy;

/// `deGate`'s default `tinypeak.removal`.
pub const DEFAULT_TINYPEAK_REMOVAL: f64 = 1.0 / 25.0;
/// `.densityForPlot`'s `spar`.
const SPAR: f64 = 0.4;
/// `count.lim`: fewer events return `-Inf`.
const COUNT_LIMIT: usize = 20;
/// `.trackSlope` arguments `alpha`, `magnitude`, `w = slope.w` and `seq.w`.
const SLOPE_ALPHA: f64 = 0.1;
const SLOPE_MAGNITUDE: f64 = 0.3;
const SLOPE_WINDOW: usize = 4;
const SLOPE_STEP: usize = 4;

/// Result of [`degate_upper_cuts`].
#[derive(Clone, Debug, PartialEq)]
pub enum DeGate {
    /// Two or more peaks: every valley between adjacent peaks, ascending.
    Cuts(Vec<f64>),
    /// One peak: the cut-off on its right tail.
    Cut(f64),
    /// Fewer than 20 events; R returns `-Inf`.
    TooFewEvents,
    /// The right tail is too short to track its slope; R returns `NA`.
    Unavailable,
}

impl DeGate {
    /// The cut-offs as R returns them (`-Inf` and `NA` become `-inf` and NaN).
    #[must_use]
    pub fn values(&self) -> Vec<f64> {
        match self {
            Self::Cuts(cuts) => cuts.clone(),
            Self::Cut(cut) => vec![*cut],
            Self::TooFewEvents => vec![f64::NEG_INFINITY],
            Self::Unavailable => vec![f64::NAN],
        }
    }
}

/// `flowDensity::deGate(x, all.cuts = TRUE, upper = TRUE,
/// tinypeak.removal = tinypeak_removal)` on finite `values`.
#[must_use]
pub fn degate_upper_cuts(values: &[f64], tinypeak_removal: f64) -> DeGate {
    if values.len() < COUNT_LIMIT {
        return DeGate::TooFewEvents;
    }
    let estimate = density(values);
    let mut y = smooth_spline_fitted_xy(&estimate.x, &estimate.y, SPAR);
    for value in &mut y {
        if *value < 0.0 {
            *value = 0.0;
        }
    }
    let x = estimate.x;

    let peaks = peak_indices(&y, tinypeak_removal);
    if let [peak] = peaks[..] {
        return match track_upper_slope(&x, &y, peak) {
            Some(cut) if cut.is_infinite() => DeGate::Cut(x[x.len() - 1]),
            Some(cut) => DeGate::Cut(cut),
            None => DeGate::Unavailable,
        };
    }
    DeGate::Cuts(
        peaks
            .windows(2)
            .map(|pair| valley(&x, &y, pair[0], pair[1]))
            .collect(),
    )
}

/// `.getPeaks(dens, peak.removal)` with `w = 1` and `twin.factor = 1`; falls
/// back to the (first) maximum when no strict local maximum qualifies.
fn peak_indices(y: &[f64], peak_removal: f64) -> Vec<usize> {
    let maximum = y.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let floor = peak_removal * maximum;
    let peaks = (1..y.len().saturating_sub(1))
        .filter(|&i| y[i] > y[i + 1] && y[i] > y[i - 1] && y[i] > floor)
        .collect::<Vec<_>>();
    if peaks.is_empty() {
        vec![first_extreme(y, |candidate, best| candidate > best)]
    } else {
        peaks
    }
}

/// `.getIntersect`: the grid point with the lowest density strictly between
/// two peaks, the first one on ties (`which.min`).
fn valley(x: &[f64], y: &[f64], left: usize, right: usize) -> f64 {
    let (low, high) = (x[left].min(x[right]), x[left].max(x[right]));
    let mut best: Option<usize> = None;
    for i in 0..x.len() {
        if x[i] < high && x[i] > low && best.is_none_or(|b| y[i] < y[b]) {
            best = Some(i);
        }
    }
    best.map_or(f64::NAN, |i| x[i])
}

/// `.trackSlope(dens, peak.ind, upper = TRUE, alpha = 0.1, magnitude = 0.3,
/// w = 4, seq.w = 4)`. `None` is R's `NA` (no room right of the peak),
/// `Some(inf)` its `Inf` (no flat, low point).
fn track_upper_slope(x: &[f64], y: &[f64], peak: usize) -> Option<f64> {
    // 1-based as in R: peak.ind = peak + 1, length(d.y) = n
    let n = y.len() as isize;
    let peak_r = peak as isize + 1;
    let mut w = SLOPE_WINDOW as isize;
    let (mut lo, mut up) = (peak_r + w, n - w);
    if lo > up {
        w = 2;
        lo = peak_r + w;
        up = n - w;
    }
    // seq(from = lo, to = up, by = w) errors on a wrong-sign `by`
    if lo > up {
        return None;
    }
    let mut slopes = Vec::new();
    let mut heights = Vec::new();
    let mut i = lo;
    while i <= up {
        let (ahead, behind) = ((i + w - 1) as usize, (i - w - 1) as usize);
        slopes.push(((y[ahead] - y[behind]) / (x[ahead] - x[behind])).abs());
        heights.push(y[(i - 1) as usize]);
        i += SLOPE_STEP as isize;
    }
    let steepest = slopes.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let peak_height = y[peak];
    let first = (0..slopes.len()).find(|&k| {
        slopes[k] < steepest * SLOPE_ALPHA && heights[k] < SLOPE_MAGNITUDE * peak_height
    });
    // d.x[peak.ind + w * ind] with the 1-based ind = k + 1
    Some(first.map_or(f64::INFINITY, |k| {
        x[(peak_r + w * (k as isize + 1) - 1) as usize]
    }))
}

fn first_extreme(values: &[f64], better: impl Fn(f64, f64) -> bool) -> usize {
    let mut best = 0;
    for (index, value) in values.iter().enumerate() {
        if better(*value, values[best]) {
            best = index;
        }
    }
    best
}

#[cfg(test)]
#[allow(clippy::unreadable_literal, clippy::excessive_precision)]
mod tests {
    use super::*;

    fn assert_cuts(actual: &DeGate, expected: &[f64], label: &str) {
        let values = actual.values();
        assert_eq!(values.len(), expected.len(), "{label}: {actual:?}");
        for (a, e) in values.iter().zip(expected) {
            if e.is_nan() {
                assert!(a.is_nan(), "{label}: {a} vs NA");
            } else if e.is_infinite() {
                assert_eq!(a, e, "{label}");
            } else {
                assert!((a - e).abs() < 1e-9, "{label}: {a} vs {e}");
            }
        }
    }

    #[test]
    fn matches_flowdensity_degate() {
        for (label, values, tinypeak, expected) in DEGATE_CASES {
            let result = degate_upper_cuts(values, *tinypeak);
            assert_cuts(&result, expected, label);
        }
    }

    #[test]
    fn fewer_than_twenty_events_return_negative_infinity() {
        let values = (0..19).map(f64::from).collect::<Vec<_>>();
        assert_eq!(
            degate_upper_cuts(&values, DEFAULT_TINYPEAK_REMOVAL),
            DeGate::TooFewEvents
        );
    }

    #[test]
    fn valley_takes_the_lowest_point_strictly_between_peaks() {
        let x = [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        let y = [0.0, 3.0, 1.0, 1.0, 2.0, 0.0];
        assert!((valley(&x, &y, 1, 4) - 2.0).abs() < f64::EPSILON);
    }

    include!("../../../data/fixtures/density/degate_reference.rs");
}
