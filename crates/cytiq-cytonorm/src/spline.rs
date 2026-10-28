//! Monotone cubic Hermite interpolation from Fritsch & Carlson (1980).
//! Implemented from the method; R `stats::splinefun` is a black-box test oracle.
#![allow(
    clippy::many_single_char_names,
    clippy::cast_precision_loss,
    clippy::float_cmp
)]
use crate::{Result, invalid};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonotoneSpline {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub slopes: Vec<f64>,
}
impl MonotoneSpline {
    /// Fit nondecreasing points, averaging targets at identical source values.
    ///
    /// # Errors
    /// Requires two distinct finite sources and monotone finite targets.
    pub fn fit(x: &[f64], y: &[f64]) -> Result<Self> {
        if x.len() != y.len()
            || x.len() < 2
            || x.iter().chain(y).any(|v| !v.is_finite())
            || x.windows(2).any(|w| w[0] > w[1])
            || y.windows(2).any(|w| w[0] > w[1])
        {
            return Err(invalid("spline knots must be finite and nondecreasing"));
        }
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        let mut i = 0;
        while i < x.len() {
            let mut j = i + 1;
            while j < x.len() && x[j] == x[i] {
                j += 1;
            }
            xs.push(x[i]);
            ys.push(y[i..j].iter().sum::<f64>() / (j - i) as f64);
            i = j;
        }
        if xs.len() < 2 {
            return Err(invalid("spline requires two distinct source knots"));
        }
        let secants = xs
            .windows(2)
            .zip(ys.windows(2))
            .map(|(a, b)| (b[1] - b[0]) / (a[1] - a[0]))
            .collect::<Vec<_>>();
        let mut slopes = vec![0.0; xs.len()];
        slopes[0] = secants[0];
        slopes[xs.len() - 1] = secants[secants.len() - 1];
        for i in 1..xs.len() - 1 {
            slopes[i] = secants[i - 1].midpoint(secants[i]);
        }
        for (i, &d) in secants.iter().enumerate() {
            if d == 0.0 {
                slopes[i] = 0.0;
                slopes[i + 1] = 0.0;
            } else {
                let a = slopes[i] / d;
                let b = slopes[i + 1] / d;
                let norm = a.hypot(b);
                if norm > 3.0 {
                    let factor = 3.0 / norm;
                    slopes[i] = factor * a * d;
                    slopes[i + 1] = factor * b * d;
                }
            }
        }
        let spline = Self {
            x: xs,
            y: ys,
            slopes,
        };
        spline.validate()?;
        Ok(spline)
    }
    /// Validate stored coefficients, including the monotonicity constraint.
    ///
    /// # Errors
    /// Rejects malformed or non-monotone coefficient arrays.
    pub fn validate(&self) -> Result<()> {
        if self.x.len() < 2
            || self.x.len() != self.y.len()
            || self.x.len() != self.slopes.len()
            || self
                .x
                .iter()
                .chain(&self.y)
                .chain(&self.slopes)
                .any(|v| !v.is_finite())
            || self.x.windows(2).any(|w| w[0] >= w[1])
            || self.y.windows(2).any(|w| w[0] > w[1])
            || self.slopes.iter().any(|s| *s < 0.0)
        {
            return Err(invalid("invalid monotone spline coefficients"));
        }
        // Compare to the canonical fit: arbitrary slopes must not bypass monotonicity.
        for i in 0..self.x.len() - 1 {
            let d = (self.y[i + 1] - self.y[i]) / (self.x[i + 1] - self.x[i]);
            if (d == 0.0 && (self.slopes[i] != 0.0 || self.slopes[i + 1] != 0.0))
                || (d > 0.0 && (self.slopes[i] / d).hypot(self.slopes[i + 1] / d) > 3.0 + 1e-12)
            {
                return Err(invalid("spline slopes violate monotonicity"));
            }
        }
        Ok(())
    }
    /// Evaluate with linear endpoint extrapolation. Call validate after loading.
    #[must_use]
    pub fn evaluate(&self, value: f64) -> f64 {
        let n = self.x.len();
        if value <= self.x[0] {
            return self.y[0] + self.slopes[0] * (value - self.x[0]);
        }
        if value >= self.x[n - 1] {
            return self.y[n - 1] + self.slopes[n - 1] * (value - self.x[n - 1]);
        }
        let i = self.x.partition_point(|x| *x <= value) - 1;
        let width = self.x[i + 1] - self.x[i];
        let t = (value - self.x[i]) / width;
        let t2 = t * t;
        let t3 = t2 * t;
        (2.0 * t3 - 3.0 * t2 + 1.0) * self.y[i]
            + (t3 - 2.0 * t2 + t) * width * self.slopes[i]
            + (-2.0 * t3 + 3.0 * t2) * self.y[i + 1]
            + (t3 - t2) * width * self.slopes[i + 1]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_committed_r_spline_and_quantile_oracles() {
        #[derive(serde::Deserialize)]
        struct SplineCase {
            x: Vec<f64>,
            y: Vec<f64>,
            query: Vec<f64>,
            expected: Vec<f64>,
        }
        #[derive(serde::Deserialize)]
        struct QuantileCase {
            sorted: Vec<f64>,
            probabilities: Vec<f64>,
            expected: Vec<f64>,
        }
        #[derive(serde::Deserialize)]
        struct Fixture {
            splines: Vec<SplineCase>,
            quantiles: Vec<QuantileCase>,
        }
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../data/fixtures/cytonorm/numeric.json"))
                .unwrap();
        for case in fixture.splines {
            let s = MonotoneSpline::fit(&case.x, &case.y).unwrap();
            for (x, expected) in case.query.into_iter().zip(case.expected) {
                let actual = s.evaluate(x);
                assert!(
                    (actual - expected).abs() <= 1e-10 + 1e-10 * expected.abs(),
                    "x={x}: Rust={actual} R={expected}"
                );
            }
        }
        for case in fixture.quantiles {
            for (p, expected) in case.probabilities.into_iter().zip(case.expected) {
                let actual = cytiq_density::quantile_type7(&case.sorted, p);
                assert!((actual - expected).abs() <= 1e-10 + 1e-10 * expected.abs());
            }
        }
    }
    #[test]
    fn interpolates_r_reference_with_linear_tails() {
        // R 4.5.3: splinefun(0:3,c(0,2,3,4),method="monoH.FC")(c(-1,0,.5,1,3,4))
        let s = MonotoneSpline::fit(&[0.0, 1.0, 2.0, 3.0], &[0.0, 2.0, 3.0, 4.0]).unwrap();
        for (x, y) in [
            (-1.0, -2.0),
            (0.0, 0.0),
            (0.5, 1.0625),
            (1.0, 2.0),
            (3.0, 4.0),
            (4.0, 5.0),
        ] {
            assert!((s.evaluate(x) - y).abs() < 1e-12);
        }
    }
    #[test]
    fn collapses_ties_and_handles_flat_segments() {
        let s = MonotoneSpline::fit(&[1.0, 1.0, 2.0], &[0.0, 2.0, 3.0]).unwrap();
        assert_eq!(s.y, vec![1.0, 3.0]);
        assert_eq!(s.evaluate(1.5), 2.0);
        assert!(MonotoneSpline::fit(&[1.0, 1.0], &[0.0, 2.0]).is_err());
        let s = MonotoneSpline::fit(&[0.0, 1.0, 2.0, 3.0], &[0.0, 0.0, 1.0, 1.0]).unwrap();
        let out = (0..301)
            .map(|i| s.evaluate(f64::from(i) / 100.0))
            .collect::<Vec<_>>();
        assert!(out.windows(2).all(|v| v[0] <= v[1]));
    }
}
