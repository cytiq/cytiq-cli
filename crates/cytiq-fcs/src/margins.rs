//! Detector margin bounds (flowCore `minRange`/`maxRange`) that survive
//! derived files.
//!
//! Margins are defined on raw detector values. A file written by a cytiq stage
//! records the bounds of each channel in the `CYTIQ_MARGINS` keyword, already
//! mapped into the space of the written values, so later stages never infer
//! them from a regenerated `$PnR`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::FcsError;
use crate::signal::SignalState;
use crate::types::{FcsDataset, FcsText, Parameter};

pub const MARGINS_KEYWORD: &str = "CYTIQ_MARGINS";

/// flowCore clamps the low margin of channels without an explicit range here.
const DEFAULT_LOWER_BOUND: f64 = -111.0;

/// Data-independent margin bounds of one channel. Events at or below
/// `max(lower, min(values))` and at or above `min(upper, max(values))` are
/// margin events.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MarginBounds {
    pub lower: f64,
    pub upper: Option<f64>,
}

impl MarginBounds {
    /// Bounds from raw acquisition keywords, as `PeacoQC::RemoveMargins` sees
    /// them through flowCore: `minRange = min($PnE zero value, max(-111,
    /// min(e)))`, `maxRange = $PnR - 1`, low threshold `max(min(minRange, 0),
    /// min(e))`. That threshold reduces to `max(lower, min(e))` with `lower`
    /// below, so it can be recorded before the data is known.
    #[must_use]
    pub fn from_raw_parameter(parameter: &Parameter) -> Self {
        let explicit_min =
            keyword_f64(parameter, "minRange").or_else(|| keyword_f64(parameter, "min"));
        let lower = match explicit_min {
            Some(minimum) => minimum.min(0.0),
            None => zero_value(parameter)
                .map_or(DEFAULT_LOWER_BOUND, |zero| zero.min(DEFAULT_LOWER_BOUND)),
        };
        let upper = keyword_f64(parameter, &format!("$P{}R", parameter.index))
            .map(|range| range - 1.0)
            .or_else(|| keyword_f64(parameter, "maxRange"))
            .or_else(|| keyword_f64(parameter, "max"));
        Self { lower, upper }
    }

    /// Low and high thresholds for a channel whose values span
    /// `minimum..=maximum`.
    #[must_use]
    pub fn thresholds(&self, minimum: f64, maximum: f64) -> (f64, f64) {
        (
            self.lower.max(minimum),
            self.upper.map_or(maximum, |upper| upper.min(maximum)),
        )
    }

    /// The detector limit alone: events at or above `upper` are on a margin.
    /// Unlike [`Self::thresholds`] (the `PeacoQC::RemoveMargins` rule) neither
    /// the observed extremes nor `lower` count. A discrete channel keeps its
    /// lowest and highest levels, and float data keeps the noise of negative
    /// populations: FCS declares no low detector limit, and flowCore's fixed
    /// -111 floor flags 31% of the events of a real BD sample
    /// (`parity/REPORT.md` §2.10). Integer data never goes below that floor.
    #[must_use]
    pub fn detector_limit(&self) -> f64 {
        self.upper.unwrap_or(f64::INFINITY)
    }

    /// Map the bounds through a monotonically increasing transform.
    #[must_use]
    pub fn map(&self, transform: impl Fn(f64) -> f64) -> Self {
        Self {
            lower: transform(self.lower),
            upper: self.upper.map(transform),
        }
    }
}

/// Margin bounds recorded in `CYTIQ_MARGINS`, keyed by `$PnN`.
///
/// # Errors
///
/// Returns an error when the keyword is present but is not valid JSON.
pub fn recorded_margin_bounds(
    text: &FcsText,
) -> Result<Option<BTreeMap<String, MarginBounds>>, FcsError> {
    text.get(MARGINS_KEYWORD)
        .map(|value| {
            serde_json::from_str(value).map_err(|_| FcsError::InvalidKeywordValue {
                key: MARGINS_KEYWORD.to_owned(),
                value: value.to_owned(),
            })
        })
        .transpose()
}

/// Encode margin bounds as the `CYTIQ_MARGINS` keyword value.
///
/// # Panics
///
/// Never: the map has string keys and finite or null values. Non-finite
/// bounds serialize as `null`.
#[must_use]
pub fn encode_margin_bounds(bounds: &BTreeMap<String, MarginBounds>) -> String {
    serde_json::to_string(bounds).expect("margin bounds serialize to JSON")
}

/// Margin bounds of every parameter whose bounds are known in the space of
/// its stored values: recorded by an earlier cytiq stage, or taken from raw
/// acquisition keywords for channels no cytiq transform, compensation or
/// unmixing has produced. Such a channel without recorded bounds is left out.
///
/// # Errors
///
/// Returns an error when `CYTIQ_MARGINS`, `CYTIQ_TRANSFORM` or
/// `CYTIQ_COMPENSATION` is present but invalid.
pub fn dataset_margin_bounds(
    text: &FcsText,
    parameters: &[Parameter],
) -> Result<BTreeMap<String, MarginBounds>, FcsError> {
    let recorded = recorded_margin_bounds(text)?.unwrap_or_default();
    let state = SignalState::of(text)?;
    // Compensation and unmixing mix channels, so raw bounds no longer
    // describe them either.
    let transformed =
        |name: &str| state.transformed(name) || state.compensated(name) || state.unmixed(name);

    Ok(parameters
        .iter()
        .filter_map(|parameter| {
            let bounds = recorded.get(&parameter.name).copied().or_else(|| {
                (!transformed(&parameter.name)).then(|| MarginBounds::from_raw_parameter(parameter))
            })?;
            Some((parameter.name.clone(), bounds))
        })
        .collect())
}

/// Per-event pass flags for the detector margins of `indices`: an event
/// fails when it sits at or above the detector limit (`$PnR - 1`) of any
/// listed channel. Only that limit counts ([`MarginBounds::detector_limit`]);
/// `cytiq peacoqc --remove-margins` keeps the upstream rule that also removes
/// the observed extremes and values at or below flowCore's low bound.
/// Compensation and unmixing mix channels, so this must run on raw values
/// before them.
///
/// # Errors
///
/// Returns an error when `CYTIQ_*` state keywords are invalid.
///
/// # Panics
///
/// Panics when an index is out of range.
pub fn raw_margin_pass(dataset: &FcsDataset, indices: &[usize]) -> Result<Vec<bool>, FcsError> {
    let bounds = dataset_margin_bounds(&dataset.text, &dataset.parameters)?;
    let mut pass = vec![true; dataset.event_count()];
    for index in indices {
        let parameter = &dataset.parameters[*index];
        let bounds = bounds
            .get(&parameter.name)
            .copied()
            .unwrap_or_else(|| MarginBounds::from_raw_parameter(parameter));
        let values = dataset
            .data
            .column(*index)
            .expect("every parameter has a data column")
            .to_f64_vec();
        let limit = bounds.detector_limit();
        for (event, value) in pass.iter_mut().zip(&values) {
            *event &= *value < limit;
        }
    }
    Ok(pass)
}

fn keyword_f64(parameter: &Parameter, key: &str) -> Option<f64> {
    parameter
        .keywords
        .iter()
        .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
        .and_then(|value| value.trim().parse::<f64>().ok())
}

/// Second field of `$PnE`, which flowCore uses as the channel's zero value.
fn zero_value(parameter: &Parameter) -> Option<f64> {
    parameter
        .keywords
        .iter()
        .find_map(|(key, value)| (*key == format!("$P{}E", parameter.index)).then_some(value))
        .and_then(|value| value.split(',').nth(1))
        .and_then(|value| value.trim().parse::<f64>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ParameterDataType, ParameterWidth};

    fn parameter(keywords: &[(&str, &str)]) -> Parameter {
        Parameter {
            index: 1,
            name: "FL1-A".to_owned(),
            label: None,
            width: ParameterWidth::Bits(32),
            data_type: ParameterDataType::Float,
            keywords: keywords
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect(),
        }
    }

    // Reference: the flowCore/PeacoQC expression the bounds replace.
    fn flowcore_thresholds(parameter: &Parameter, values: &[f64]) -> (f64, f64) {
        let minimum = values.iter().copied().fold(f64::INFINITY, f64::min);
        let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let real_min = DEFAULT_LOWER_BOUND.max(minimum);
        let min_range = keyword_f64(parameter, "minRange")
            .unwrap_or_else(|| zero_value(parameter).map_or(real_min, |zero| zero.min(real_min)));
        let max_range = keyword_f64(parameter, "$P1R").map_or(maximum, |range| range - 1.0);
        (min_range.min(0.0).max(minimum), max_range.min(maximum))
    }

    #[test]
    fn raw_thresholds_match_flowcore() {
        type Case<'a> = (&'a [(&'a str, &'a str)], &'a [f64]);
        let cases: [Case; 4] = [
            (&[("$P1R", "1024"), ("$P1E", "0,0")], &[-5.0, 10.0, 1023.0]),
            (&[("$P1R", "1024"), ("$P1E", "0,0")], &[-500.0, 10.0, 900.0]),
            (&[("$P1R", "1024")], &[-500.0, 10.0, 900.0]),
            (
                &[("$P1R", "262144"), ("$P1E", "4,1")],
                &[1.0, 5.0, 262_143.0],
            ),
        ];
        for (keywords, values) in cases {
            let parameter = parameter(keywords);
            let minimum = values.iter().copied().fold(f64::INFINITY, f64::min);
            let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            assert_eq!(
                MarginBounds::from_raw_parameter(&parameter).thresholds(minimum, maximum),
                flowcore_thresholds(&parameter, values),
                "keywords {keywords:?}, values {values:?}"
            );
        }
    }

    #[test]
    fn raw_margins_keep_negative_noise_and_flag_saturation() {
        let parameters = [crate::FcsWriteParameter::new("FL1-A").with_range(1024.0)];
        let values = vec![-500.0_f32, -111.0, 0.0, 1022.0, 1023.0, 1500.0];
        let mut bytes = Vec::new();
        crate::write_fcs3_1_f32_to_writer(&mut bytes, &parameters, &[values]).unwrap();
        let dataset = crate::FcsReader::new(std::io::Cursor::new(bytes))
            .read()
            .unwrap();

        assert_eq!(
            raw_margin_pass(&dataset, &[0]).unwrap(),
            [true, true, true, true, false, false]
        );
    }

    #[test]
    fn round_trips_recorded_bounds() {
        let bounds = BTreeMap::from([(
            "FL1-A".to_owned(),
            MarginBounds {
                lower: -0.25,
                upper: Some(4.5),
            },
        )]);
        let decoded: BTreeMap<String, MarginBounds> =
            serde_json::from_str(&encode_margin_bounds(&bounds)).unwrap();
        assert_eq!(decoded, bounds);
    }
}
