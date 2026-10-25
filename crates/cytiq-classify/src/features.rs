use std::collections::HashSet;

use cytiq_fcs::{FcsDataset, SignalState, Spillover, compensate, linear_values};
use serde::Deserialize;

use crate::ClassifyError;

#[derive(Clone, Debug, Deserialize)]
pub struct Feature {
    pub name: String,
    pub channel: String,
    pub labels: Vec<String>,
    pub fluorescence: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Profile {
    pub panel: String,
    pub instrument: String,
    pub recipe: String,
    pub cofactor: f64,
    pub features: Vec<Feature>,
}

/// Apply the same recipe as cytiq-ml's `feature_matrix`.
///
/// # Errors
/// Rejects incompatible panels, signal state or nonfinite values.
#[allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
pub fn feature_columns(
    dataset: &FcsDataset,
    profile: &Profile,
) -> Result<Vec<Vec<f32>>, ClassifyError> {
    let invalid = |s: &str| ClassifyError::Invalid(s.to_owned());
    if profile.recipe != "cytiq-fcm-arcsinh-v1"
        || !profile.cofactor.is_finite()
        || profile.cofactor <= 0.0
        || profile.features.is_empty()
    {
        return Err(invalid("unsupported feature recipe"));
    }
    let instrument = dataset.text.iter().find_map(|(k, v)| {
        k.trim_start_matches('$')
            .eq_ignore_ascii_case("CYT")
            .then_some(v)
    });
    if instrument != Some(profile.instrument.as_str()) {
        return Err(invalid("instrument does not match model profile"));
    }
    let mut indices = Vec::new();
    let mut unique = HashSet::new();
    let mut unique_channels = HashSet::new();
    for feature in &profile.features {
        if feature.name.is_empty()
            || !unique.insert(&feature.name)
            || !unique_channels.insert(&feature.channel)
        {
            return Err(invalid("duplicate feature name"));
        }
        let matches = dataset
            .parameters
            .iter()
            .enumerate()
            .filter(|(_, p)| p.name == feature.channel)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            return Err(invalid("missing or ambiguous feature channel"));
        }
        let (index, parameter) = matches[0];
        if !feature.labels.is_empty()
            && !parameter
                .label
                .as_ref()
                .is_some_and(|label| feature.labels.contains(label))
        {
            return Err(invalid("marker label does not match panel"));
        }
        indices.push(index);
    }
    let state = SignalState::of(&dataset.text)?;
    let fluorescence = profile
        .features
        .iter()
        .filter(|f| f.fluorescence)
        .map(|f| f.channel.as_str())
        .collect::<HashSet<_>>();
    if state.unmixing.is_some() {
        return Err(invalid(
            "spectral unmixing is incompatible with this FCM profile",
        ));
    }
    if let Some(compensation) = &state.compensation {
        if fluorescence.iter().any(|c| !compensation.covers(c)) {
            return Err(invalid("incomplete compensation state"));
        }
    } else if state.source_spillover {
        return Err(invalid(
            "source spillover without verified compensation state",
        ));
    }
    if let Some(transform) = &state.transform {
        let document: serde_json::Value =
            serde_json::from_str(dataset.text.get("CYTIQ_TRANSFORM").unwrap_or_default())?;
        let channels = transform
            .channels
            .iter()
            .map(String::as_str)
            .collect::<HashSet<_>>();
        if transform.method != "arcsinh"
            || !fluorescence.is_subset(&channels)
            || profile
                .features
                .iter()
                .any(|f| !f.fluorescence && channels.contains(f.channel.as_str()))
            || document["parameters"]["cofactor"].as_f64() != Some(profile.cofactor)
            || state.compensation.is_none()
        {
            return Err(invalid(
                "incompatible transformation or missing compensation state",
            ));
        }
    }
    let mut columns = indices
        .iter()
        .map(|i| linear_values(dataset, *i))
        .collect::<Vec<_>>();
    if state.compensation.is_none() {
        let spill = Spillover::from_dataset(dataset)?
            .ok_or_else(|| invalid("raw input requires a spillover matrix"))?;
        if fluorescence
            .iter()
            .any(|c| !spill.channels().iter().any(|s| s == c))
        {
            return Err(invalid("incomplete spillover matrix"));
        }
        let spill_columns = spill
            .channels()
            .iter()
            .map(|name| {
                let index = dataset
                    .parameters
                    .iter()
                    .position(|p| &p.name == name)
                    .ok_or_else(|| invalid("spillover channel not found"))?;
                Ok(linear_values(dataset, index))
            })
            .collect::<Result<Vec<_>, ClassifyError>>()?;
        let compensated = compensate(&spill_columns, &spill.compensation_matrix()?);
        for (feature, values) in profile.features.iter().zip(&mut columns) {
            if let Some(index) = spill.channels().iter().position(|n| n == &feature.channel) {
                // Match the f32 FCS written by cytiq compensate.
                *values = compensated[index]
                    .iter()
                    .map(|v| f64::from(*v as f32))
                    .collect();
            }
        }
    }
    let mut output = Vec::new();
    for (feature, values) in profile.features.iter().zip(columns) {
        let values = values
            .into_iter()
            .map(|v| {
                let v = if feature.fluorescence && state.transform.is_none() {
                    (v / profile.cofactor).asinh()
                } else {
                    v
                } as f32;
                if !v.is_finite() {
                    return Err(invalid("nonfinite classifier feature"));
                }
                Ok(v)
            })
            .collect::<Result<Vec<_>, _>>()?;
        output.push(values);
    }
    Ok(output)
}
