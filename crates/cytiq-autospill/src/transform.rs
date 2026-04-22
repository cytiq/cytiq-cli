use std::collections::HashMap;

use crate::manifest::TransformSpec;

const DEFAULT_LENGTH: f64 = 256.0;
const DEFAULT_MAX_RANGE: f64 = 262_144.0;
const DEFAULT_POS: f64 = 4.418_539_922;
const DEFAULT_NEG: f64 = 0.0;
const DEFAULT_WIDTH: f64 = -100.0;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BiExponentialTransform {
    length: f64,
    max_range: f64,
    linear_scale: f64,
    positive_scale: f64,
    negative_scale: f64,
}

impl BiExponentialTransform {
    #[must_use]
    pub fn from_spec(spec: &TransformSpec) -> Self {
        let max_range = spec.max_range.max(1.0);
        let length = spec.length.max(1.0);
        let width_factor = 1.0 + spec.width.abs() / 100.0;
        let positive_decades = spec.pos.max(1.0);
        let negative_decades = spec.neg.max(0.0) + 1.0;

        let linear_scale = (max_range / 10_f64.powf(positive_decades)).max(1.0) * width_factor;
        let negative_linear_scale = (max_range / 10_f64.powf(negative_decades)).max(linear_scale);
        let positive_scale = length / (1.0 + max_range / linear_scale).ln();
        let negative_scale = length / (1.0 + max_range / negative_linear_scale).ln();

        Self {
            length,
            max_range,
            linear_scale,
            positive_scale,
            negative_scale,
        }
    }

    #[must_use]
    pub fn default_for_channel(channel: &str) -> Self {
        Self::from_spec(&TransformSpec {
            channel: channel.to_owned(),
            length: DEFAULT_LENGTH,
            max_range: DEFAULT_MAX_RANGE,
            pos: DEFAULT_POS,
            neg: DEFAULT_NEG,
            width: DEFAULT_WIDTH,
        })
    }

    #[must_use]
    pub fn apply(&self, value: f64) -> f64 {
        if !value.is_finite() {
            return value;
        }

        if value >= 0.0 {
            self.positive_scale * (1.0 + value / self.linear_scale).ln()
        } else {
            -self.negative_scale * (1.0 + (-value) / self.linear_scale).ln()
        }
    }

    #[must_use]
    pub fn inverse(&self, value: f64) -> f64 {
        if !value.is_finite() {
            return value;
        }

        if value >= 0.0 {
            self.linear_scale * ((value / self.positive_scale).exp() - 1.0)
        } else {
            -self.linear_scale * (((-value) / self.negative_scale).exp() - 1.0)
        }
    }
}

pub(crate) fn resolve_transforms(
    channels: &[String],
    specs: &[TransformSpec],
) -> Vec<BiExponentialTransform> {
    let spec_map = specs
        .iter()
        .map(|spec| (spec.channel.clone(), spec))
        .collect::<HashMap<_, _>>();

    channels
        .iter()
        .map(|channel| {
            spec_map
                .get(channel)
                .map_or_else(|| BiExponentialTransform::default_for_channel(channel), |spec| {
                    BiExponentialTransform::from_spec(spec)
                })
        })
        .collect()
}

#[must_use]
pub(crate) fn transformed_slope_to_linear(
    secondary_transform: &BiExponentialTransform,
    primary_transform: &BiExponentialTransform,
    intercept: f64,
    slope: f64,
    primary_low_transformed: f64,
    primary_high_transformed: f64,
) -> f64 {
    let secondary_low_transformed = intercept + slope * primary_low_transformed;
    let secondary_high_transformed = intercept + slope * primary_high_transformed;

    if (primary_high_transformed - primary_low_transformed).abs() <= f64::EPSILON
        || (secondary_high_transformed - secondary_low_transformed).abs() <= f64::EPSILON
    {
        return 0.0;
    }

    let primary_low = primary_transform.inverse(primary_low_transformed);
    let primary_high = primary_transform.inverse(primary_high_transformed);
    let secondary_low = secondary_transform.inverse(secondary_low_transformed);
    let secondary_high = secondary_transform.inverse(secondary_high_transformed);

    if (primary_high - primary_low).abs() <= f64::EPSILON
        || (secondary_high_transformed - secondary_low_transformed).abs() <= f64::EPSILON
    {
        return 0.0;
    }

    slope * (secondary_high - secondary_low) * (primary_high_transformed - primary_low_transformed)
        / ((secondary_high_transformed - secondary_low_transformed) * (primary_high - primary_low))
}

#[cfg(test)]
mod tests {
    use super::{BiExponentialTransform, transformed_slope_to_linear};

    #[test]
    fn transform_round_trips_values() {
        let transform = BiExponentialTransform::default_for_channel("FITC");
        for value in [-1_000.0, -5.0, 0.0, 5.0, 1_000.0, 50_000.0] {
            let transformed = transform.apply(value);
            let round_trip = transform.inverse(transformed);
            let tolerance = value.abs().max(1.0) * 1e-6 + 1e-6;
            assert!((round_trip - value).abs() <= tolerance);
        }
    }

    #[test]
    fn transformed_slope_converts_back_to_linear() {
        let transform = BiExponentialTransform::default_for_channel("FITC");
        let slope = transformed_slope_to_linear(&transform, &transform, 0.0, 0.5, -5.0, 5.0);
        assert!(slope.is_finite());
    }
}
