#![allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]

use std::collections::HashMap;

use cytiq_transform::scales::FlowJoBiexp;

use crate::manifest::TransformSpec;

const DEFAULT_LENGTH: f64 = 256.0;
const DEFAULT_MAX_RANGE: f64 = 262_144.0;
const DEFAULT_POS: f64 = 4.418_539_922;
const DEFAULT_NEG: f64 = 0.0;
const DEFAULT_WIDTH: f64 = -100.0;

/// Per-dye transform used by the bi-exponential refinement stage, as
/// autospill's `get.transformation`: `flowWorkspace::flowjo_biexp(channelRange
/// = length, maxValue = max.range, pos, neg, widthBasis = width)`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BiExponentialTransform {
    scale: FlowJoBiexp,
}

impl BiExponentialTransform {
    #[must_use]
    pub fn from_spec(spec: &TransformSpec) -> Self {
        let scale = FlowJoBiexp::new(
            spec.length.round().max(1.0) as usize,
            spec.max_range,
            spec.pos,
            spec.neg,
            spec.width,
        )
        .unwrap_or_else(|_| default_scale());
        Self { scale }
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
        self.scale.scale(value)
    }

    #[must_use]
    pub fn inverse(&self, value: f64) -> f64 {
        self.scale.inverse(value)
    }
}

fn default_scale() -> FlowJoBiexp {
    FlowJoBiexp::new(
        DEFAULT_LENGTH as usize,
        DEFAULT_MAX_RANGE,
        DEFAULT_POS,
        DEFAULT_NEG,
        DEFAULT_WIDTH,
    )
    .expect("autospill default bi-exponential parameters are valid")
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
            spec_map.get(channel).map_or_else(
                || BiExponentialTransform::default_for_channel(channel),
                |spec| BiExponentialTransform::from_spec(spec),
            )
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
    fn default_transform_matches_autospill_flowjo_biexp() {
        // flowWorkspace::flowjo_biexp(256, 262144, 4.418539922, 0, -100)
        let transform = BiExponentialTransform::default_for_channel("FITC");
        assert!((transform.apply(0.0) - 57.0).abs() < 1e-9);
        assert!((transform.apply(-1_000.0) - 10.271_648_150_319_1).abs() < 1e-9);
        assert!((transform.apply(1_000.0) - 103.728_351_851_371_09).abs() < 1e-9);
    }

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
