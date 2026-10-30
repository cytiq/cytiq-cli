use std::collections::HashSet;

use cytiq_fcs::{
    ColumnData, FcsDataset, SignalState, Spillover, compensate, linear_values,
    log_amplified_parameters,
};
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
    feature_columns_with_normalization(dataset, profile, &crate::NormalizationContract::None)
}

/// Extract features only after verifying the training normalization contract.
///
/// # Errors
/// Rejects incompatible normalization, panel, recipe or nonfinite values.
#[allow(clippy::too_many_lines, clippy::cast_possible_truncation)]
pub fn feature_columns_with_normalization(
    dataset: &FcsDataset,
    profile: &Profile,
    normalization: &crate::NormalizationContract,
) -> Result<Vec<Vec<f32>>, ClassifyError> {
    normalization.check(dataset)?;
    let invalid = |s: &str| ClassifyError::Invalid(s.to_owned());
    if profile.recipe == "cytiq-table-stored-v1" {
        return Err(invalid(
            "cytiq-table-stored-v1 is a training-only recipe without FCS feature extraction",
        ));
    }
    if ![
        "cytiq-fcm-arcsinh-v1",
        "cytiq-flowcyt-stored-v1",
        "cytiq-cytof-arcsinh-v1",
    ]
    .contains(&profile.recipe.as_str())
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
    // CyTOF software writes vendor and version into $CYT; its profiles name a family.
    let matches = instrument.is_some_and(|cyt| {
        if profile.recipe == "cytiq-cytof-arcsinh-v1" {
            cyt.to_lowercase()
                .contains(&profile.instrument.to_lowercase())
        } else {
            cyt == profile.instrument
        }
    });
    if !matches {
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
    if profile.recipe == "cytiq-cytof-arcsinh-v1" {
        return cytof_columns(dataset, profile, &indices);
    }
    if profile.recipe == "cytiq-flowcyt-stored-v1" {
        if dataset.text.iter().any(|(key, _)| {
            [
                "CYTIQ_COMPENSATION",
                "CYTIQ_TRANSFORM",
                "CYTIQ_SOURCE_SPILL",
                "CYTIQ_UNMIXING",
                "CYTIQ_NORMALIZATION",
            ]
            .iter()
            .any(|name| key.trim_start_matches('$').eq_ignore_ascii_case(name))
        }) {
            return Err(invalid(
                "FlowCyt recipe requires the original author export",
            ));
        }
        return stored_columns(dataset, &indices);
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

const SIGNAL_KEYWORDS: [&str; 5] = [
    "CYTIQ_COMPENSATION",
    "CYTIQ_TRANSFORM",
    "CYTIQ_SOURCE_SPILL",
    "CYTIQ_UNMIXING",
    "CYTIQ_NORMALIZATION",
];

/// Mass cytometry: linear stored ion counts, no compensation, `asinh(x / cofactor)`
/// on the profile's transformed (`fluorescence`) channels, as in cytiq-ml.
#[allow(clippy::cast_possible_truncation)]
fn cytof_columns(
    dataset: &FcsDataset,
    profile: &Profile,
    indices: &[usize],
) -> Result<Vec<Vec<f32>>, ClassifyError> {
    let invalid = |s: &str| ClassifyError::Invalid(s.to_owned());
    let keyword = |key: &str| {
        dataset.text.iter().find_map(|(candidate, value)| {
            candidate
                .trim_start_matches('$')
                .eq_ignore_ascii_case(key)
                .then_some(value)
        })
    };
    if SIGNAL_KEYWORDS.iter().any(|key| keyword(key).is_some()) {
        return Err(invalid("CyTOF recipe requires stored acquisition values"));
    }
    if ["SPILLOVER", "SPILL"]
        .iter()
        .any(|key| keyword(key).is_some_and(|value| !value.trim().is_empty()))
    {
        return Err(invalid("CyTOF recipe does not apply a spillover matrix"));
    }
    let logged = log_amplified_parameters(dataset);
    for index in indices {
        let parameter = &dataset.parameters[*index];
        let gain = keyword(&format!("P{}G", parameter.index))
            .and_then(|value| value.trim().parse::<f64>().ok())
            .unwrap_or(1.0);
        if logged.contains(&parameter.name.as_str())
            || (gain.abs() > f64::EPSILON && (gain - 1.0).abs() > f64::EPSILON)
        {
            return Err(invalid("CyTOF recipe expects linear unit-gain channels"));
        }
    }
    profile
        .features
        .iter()
        .zip(indices)
        .map(|(feature, index)| {
            dataset.data.columns[*index]
                .to_f64_vec()
                .into_iter()
                .map(|v| {
                    let v = if feature.fluorescence {
                        (v / profile.cofactor).asinh()
                    } else {
                        v
                    } as f32;
                    if v.is_finite() {
                        Ok(v)
                    } else {
                        Err(invalid("nonfinite classifier feature"))
                    }
                })
                .collect()
        })
        .collect()
}

#[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
fn stored_columns(dataset: &FcsDataset, indices: &[usize]) -> Result<Vec<Vec<f32>>, ClassifyError> {
    indices
        .iter()
        .map(|index| {
            let values: Vec<f32> = match &dataset.data.columns[*index] {
                ColumnData::UInt(values) => values.iter().map(|v| *v as f32).collect(),
                ColumnData::F32(values) => values.clone(),
                ColumnData::F64(values) => values.iter().map(|v| *v as f32).collect(),
            };
            if values.iter().any(|v| !v.is_finite()) {
                return Err(ClassifyError::Invalid("nonfinite FlowCyt features".into()));
            }
            Ok(values)
        })
        .collect()
}
