#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RegressionResult {
    pub intercept: f64,
    pub slope: f64,
}

// AutoSpill estimates each spillover term from `secondary ~ primary` using a
// robust linear fit with an intercept. The reference implementation uses robust
// regression to reduce the influence of outliers and heteroskedastic tails that
// are common in flow data; we keep the same intent here with an iteratively
// reweighted fit.
pub(crate) fn robust_linear_regression(
    x: &[f64],
    y: &[f64],
    max_iterations: usize,
) -> Option<RegressionResult> {
    if x.len() != y.len() || x.len() < 3 {
        return None;
    }

    let mut weights = vec![1.0; x.len()];
    let (mut intercept, mut slope) = weighted_fit(x, y, &weights)?;
    for _ in 0..max_iterations {
        let residuals = x
            .iter()
            .zip(y)
            .map(|(x_value, y_value)| y_value - (intercept + slope * x_value))
            .collect::<Vec<_>>();
        let scale = scaled_mad(&residuals);
        if scale <= f64::EPSILON {
            break;
        }

        for (weight, residual) in weights.iter_mut().zip(&residuals) {
            let threshold = 1.345 * scale;
            let absolute = residual.abs();
            *weight = if absolute <= threshold {
                1.0
            } else {
                threshold / absolute
            };
        }

        let (next_intercept, next_slope) = weighted_fit(x, y, &weights)?;
        if (next_intercept - intercept).abs() < 1e-9 && (next_slope - slope).abs() < 1e-9 {
            intercept = next_intercept;
            slope = next_slope;
            break;
        }
        intercept = next_intercept;
        slope = next_slope;
    }

    Some(RegressionResult { intercept, slope })
}

pub(crate) fn trim_joint_pairs(
    primary: &[f64],
    secondary: &[f64],
    trim_factor: f64,
) -> (Vec<f64>, Vec<f64>) {
    if primary.len() != secondary.len() || primary.len() < 3 {
        return (Vec::new(), Vec::new());
    }

    let trim_n = ((primary.len() as f64) * trim_factor).round() as usize;
    let trim_n = trim_n.clamp(1, primary.len());
    let primary_low = nth_sorted(primary, trim_n - 1, false);
    let primary_high = nth_sorted(primary, trim_n - 1, true);
    let secondary_low = nth_sorted(secondary, trim_n - 1, false);
    let secondary_high = nth_sorted(secondary, trim_n - 1, true);

    // The trimming is joint: an event is kept only if it stays away from the
    // extreme tails in both the primary and secondary channels. That mirrors the
    // R implementation and avoids fitting the regression against paired extremes
    // where one detector is saturated or dominated by artefacts.
    let mut trimmed_primary = Vec::new();
    let mut trimmed_secondary = Vec::new();
    for (&primary_value, &secondary_value) in primary.iter().zip(secondary) {
        if primary_value > primary_low
            && primary_value < primary_high
            && secondary_value > secondary_low
            && secondary_value < secondary_high
        {
            trimmed_primary.push(primary_value);
            trimmed_secondary.push(secondary_value);
        }
    }

    if trimmed_primary.len() < 3 {
        (primary.to_vec(), secondary.to_vec())
    } else {
        (trimmed_primary, trimmed_secondary)
    }
}

pub(crate) fn skewness(values: &[f64]) -> f64 {
    if values.len() < 3 {
        return 0.0;
    }

    let mean = values.iter().sum::<f64>() / (values.len() as f64);
    let variance = values
        .iter()
        .map(|value| {
            let centered = value - mean;
            centered * centered
        })
        .sum::<f64>()
        / (values.len() as f64);
    let standard_deviation = variance.sqrt();
    if standard_deviation <= f64::EPSILON {
        return 0.0;
    }

    let third_moment = values
        .iter()
        .map(|value| {
            let centered = value - mean;
            centered * centered * centered
        })
        .sum::<f64>()
        / (values.len() as f64);
    third_moment / standard_deviation.powi(3)
}

fn weighted_fit(x: &[f64], y: &[f64], weights: &[f64]) -> Option<(f64, f64)> {
    let weight_sum = weights.iter().sum::<f64>();
    if weight_sum <= f64::EPSILON {
        return None;
    }

    let mean_x = x
        .iter()
        .zip(weights)
        .map(|(value, weight)| value * weight)
        .sum::<f64>()
        / weight_sum;
    let mean_y = y
        .iter()
        .zip(weights)
        .map(|(value, weight)| value * weight)
        .sum::<f64>()
        / weight_sum;

    let covariance = x
        .iter()
        .zip(y)
        .zip(weights)
        .map(|((x_value, y_value), weight)| weight * (x_value - mean_x) * (y_value - mean_y))
        .sum::<f64>();
    let variance = x
        .iter()
        .zip(weights)
        .map(|(value, weight)| weight * (value - mean_x) * (value - mean_x))
        .sum::<f64>();
    if variance <= f64::EPSILON {
        return None;
    }

    let slope = covariance / variance;
    let intercept = mean_y - slope * mean_x;
    Some((intercept, slope))
}

fn scaled_mad(values: &[f64]) -> f64 {
    let center = median(values);
    let deviations = values
        .iter()
        .map(|value| (value - center).abs())
        .collect::<Vec<_>>();
    1.4826 * median(&deviations)
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    let middle = sorted.len() / 2;
    if sorted.len().is_multiple_of(2) {
        f64::midpoint(sorted[middle - 1], sorted[middle])
    } else {
        sorted[middle]
    }
}

fn nth_sorted(values: &[f64], index: usize, descending: bool) -> f64 {
    let mut sorted = values.to_vec();
    if descending {
        sorted.sort_by(|left, right| right.total_cmp(left));
    } else {
        sorted.sort_by(f64::total_cmp);
    }
    sorted[index]
}

#[cfg(test)]
mod tests {
    use super::{robust_linear_regression, trim_joint_pairs};

    #[test]
    fn robust_regression_handles_outliers() {
        let x = (1..=40).map(f64::from).collect::<Vec<_>>();
        let mut y = x.iter().map(|value| 2.0 * value + 5.0).collect::<Vec<_>>();
        y[5] = 500.0;
        y[22] = -300.0;

        let result = robust_linear_regression(&x, &y, 100)
            .expect("regression should converge with outliers");

        assert!((result.intercept - 5.0).abs() < 1.0);
        assert!((result.slope - 2.0).abs() < 0.05);
    }

    #[test]
    fn trim_joint_pairs_removes_extremes_from_both_axes() {
        let primary = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10_000.0];
        let secondary = vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0, -2_000.0];
        let (trimmed_primary, trimmed_secondary) = trim_joint_pairs(&primary, &secondary, 0.2);

        assert_eq!(trimmed_primary, vec![3.0, 4.0, 5.0, 6.0, 7.0]);
        assert_eq!(trimmed_secondary, vec![6.0, 8.0, 10.0, 12.0, 14.0]);
    }
}
