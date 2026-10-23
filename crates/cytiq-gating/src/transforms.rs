//! `GatingML` 2.0 parametrized transformations (section 6 of the specification).

use std::f64::consts::LN_10;

use cytiq_transform::scales::{Hyperlog, Logicle};
use rayon::prelude::*;

/// A `GatingML` 2.0 transformation applied to a single dimension.
#[derive(Clone, Debug, PartialEq)]
pub enum Transform {
    /// `flin(x, T, A) = (x + A) / (T + A)`.
    Linear { t: f64, a: f64 },
    /// `flog(x, T, M) = (1 / M) · log10(x / T) + 1`.
    Log { t: f64, m: f64 },
    /// `fasinh(x, T, M, A) = (asinh(x · sinh(M ln10) / T) + A ln10) / ((M + A) ln10)`.
    Asinh { t: f64, m: f64, a: f64 },
    /// `logicle(x, T, W, M, A)`.
    Logicle(Logicle),
    /// `hyperlog(x, T, W, M, A)`.
    Hyperlog(Hyperlog),
}

impl Transform {
    #[must_use]
    pub fn apply(&self, value: f64) -> f64 {
        match self {
            Self::Linear { t, a } => (value + a) / (t + a),
            Self::Log { t, m } => (1.0 / m) * (value / t).log10() + 1.0,
            Self::Asinh { t, m, a } => {
                let pre_scale = (m * LN_10).sinh() / t;
                ((value * pre_scale).asinh() + a * LN_10) / ((m + a) * LN_10)
            }
            Self::Logicle(logicle) => logicle.scale(value),
            Self::Hyperlog(hyperlog) => hyperlog.scale(value),
        }
    }

    /// Apply the transform to every value of a column.
    #[must_use]
    pub fn apply_column(&self, values: &[f64]) -> Vec<f64> {
        values.par_iter().map(|value| self.apply(*value)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-12, "{actual} != {expected}");
    }

    #[test]
    fn linear_maps_top_of_scale_to_one() {
        let transform = Transform::Linear { t: 1000.0, a: 0.0 };
        assert_close(transform.apply(1000.0), 1.0);
        assert_close(transform.apply(250.0), 0.25);
    }

    #[test]
    fn log_maps_top_of_scale_to_one() {
        let transform = Transform::Log {
            t: 10_000.0,
            m: 4.0,
        };
        assert_close(transform.apply(10_000.0), 1.0);
        assert_close(transform.apply(100.0), 0.5);
        assert!(transform.apply(-1.0).is_nan());
    }

    #[test]
    fn asinh_maps_top_of_scale_to_one() {
        let transform = Transform::Asinh {
            t: 262_144.0,
            m: 4.5,
            a: 0.0,
        };
        assert_close(transform.apply(262_144.0), 1.0);
        assert_close(transform.apply(0.0), 0.0);
    }

    #[test]
    fn asinh_offsets_zero_by_extra_decades() {
        let transform = Transform::Asinh {
            t: 1000.0,
            m: 4.0,
            a: 1.0,
        };
        assert_close(transform.apply(0.0), 0.2);
    }

    #[test]
    fn logicle_maps_top_of_scale_to_one() {
        let transform = Transform::Logicle(Logicle::new(262_144.0, 0.5, 4.5, 0.0).unwrap());
        assert_close(transform.apply(262_144.0), 1.0);
    }
}
