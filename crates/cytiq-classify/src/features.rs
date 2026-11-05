use std::collections::HashSet;

use cytiq_fcs::{
    ColumnData, FcsDataset, SignalState, Spillover, compensate, linear_values,
    log_amplified_parameters,
};
use serde::Deserialize;

use crate::ClassifyError;

/// Analysis software that writes itself into `$CYT` when it re-exports a file.
const REEXPORT_SOFTWARE: &str = "Kaluza";

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
pub fn feature_columns_with_normalization(
    dataset: &FcsDataset,
    profile: &Profile,
    normalization: &crate::NormalizationContract,
) -> Result<Vec<Vec<f32>>, ClassifyError> {
    normalization.check(dataset)?;
    validate_feature_recipe(profile)?;
    validate_instrument(dataset, profile)?;
    let indices = feature_indices(dataset, profile)?;
    match profile.recipe.as_str() {
        "cytiq-cytof-arcsinh-v1" => cytof_columns(dataset, profile, &indices),
        "cytiq-flowcyt-stored-v1" => flowcyt_columns(dataset, &indices),
        _ => fcm_columns(dataset, profile, &indices),
    }
}

fn validate_feature_recipe(profile: &Profile) -> Result<(), ClassifyError> {
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
    Ok(())
}

fn validate_instrument(dataset: &FcsDataset, profile: &Profile) -> Result<(), ClassifyError> {
    let invalid = |s: &str| ClassifyError::Invalid(s.to_owned());
    let keyword = |name: &str| {
        dataset.text.iter().find_map(|(k, v)| {
            k.trim_start_matches('$')
                .eq_ignore_ascii_case(name)
                .then_some(v)
        })
    };
    // CyTOF software writes vendor and version into $CYT; its profiles name a family.
    // Kaluza re-exports name Kaluza in $CYT and keep the acquiring cytometer in
    // $OriginalInstrument, in its own case (FlowCyt: `Navios` for `NAVIOS`).
    let matches = keyword("CYT").is_some_and(|cyt| {
        if profile.recipe == "cytiq-cytof-arcsinh-v1" {
            cyt.to_lowercase()
                .contains(&profile.instrument.to_lowercase())
        } else if cyt == REEXPORT_SOFTWARE {
            keyword("OriginalInstrument")
                .is_some_and(|original| original.eq_ignore_ascii_case(&profile.instrument))
        } else {
            *cyt == profile.instrument
        }
    });
    if !matches {
        return Err(invalid("instrument does not match model profile"));
    }
    Ok(())
}

fn feature_indices(dataset: &FcsDataset, profile: &Profile) -> Result<Vec<usize>, ClassifyError> {
    let invalid = |s: &str| ClassifyError::Invalid(s.to_owned());
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
    Ok(indices)
}

fn flowcyt_columns(
    dataset: &FcsDataset,
    indices: &[usize],
) -> Result<Vec<Vec<f32>>, ClassifyError> {
    let invalid = |s: &str| ClassifyError::Invalid(s.to_owned());
    if dataset.text.iter().any(|(key, _)| {
        SIGNAL_KEYWORDS
            .iter()
            .any(|name| key.trim_start_matches('$').eq_ignore_ascii_case(name))
    }) {
        return Err(invalid(
            "FlowCyt recipe requires the original author export",
        ));
    }
    stored_columns(dataset, indices)
}

fn validate_fcm_state(
    dataset: &FcsDataset,
    profile: &Profile,
    state: &SignalState,
) -> Result<(), ClassifyError> {
    let invalid = |s: &str| ClassifyError::Invalid(s.to_owned());
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
    Ok(())
}

#[allow(clippy::cast_possible_truncation)]
fn compensated_columns(
    dataset: &FcsDataset,
    profile: &Profile,
    indices: &[usize],
    state: &SignalState,
) -> Result<Vec<Vec<f64>>, ClassifyError> {
    let invalid = |s: &str| ClassifyError::Invalid(s.to_owned());
    let fluorescence = profile
        .features
        .iter()
        .filter(|f| f.fluorescence)
        .map(|f| f.channel.as_str())
        .collect::<HashSet<_>>();
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
    Ok(columns)
}

#[allow(clippy::cast_possible_truncation)]
fn fcm_columns(
    dataset: &FcsDataset,
    profile: &Profile,
    indices: &[usize],
) -> Result<Vec<Vec<f32>>, ClassifyError> {
    let invalid = |s: &str| ClassifyError::Invalid(s.to_owned());
    let state = SignalState::of(&dataset.text)?;
    validate_fcm_state(dataset, profile, &state)?;
    let columns = compensated_columns(dataset, profile, indices, &state)?;
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

#[cfg(test)]
mod tests {
    #![allow(clippy::cast_possible_truncation)]
    use super::*;
    use cytiq_fcs::{
        CompensationState, FcsReader, FcsWriteOptions, FcsWriteParameter, TransformState,
        UnmixingState,
    };
    use serde_json::json;
    use std::{io::Cursor, path::PathBuf};

    fn fixture() -> (FcsDataset, Profile) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/classify");
        let bundle = crate::Bundle::load(&root.join("catboost"), 0.0).unwrap();
        (
            FcsReader::open(root.join("raw.fcs"))
                .unwrap()
                .read()
                .unwrap(),
            bundle.profile,
        )
    }

    fn keywords(data: &FcsDataset, additions: &[(&str, String)]) -> FcsDataset {
        let mut pairs = cytiq_fcs::carried_keywords(&data.text);
        pairs.retain(|(key, _)| !additions.iter().any(|(name, _)| key == name));
        pairs.extend(
            additions
                .iter()
                .map(|(key, value)| ((*key).into(), value.clone())),
        );
        let parameters = data
            .parameters
            .iter()
            .map(FcsWriteParameter::from_source)
            .collect::<Vec<_>>();
        let columns = data
            .data
            .columns
            .iter()
            .map(|c| {
                c.to_f64_vec()
                    .into_iter()
                    .map(|v| v as f32)
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let mut bytes = Vec::new();
        cytiq_fcs::write_fcs3_1_f32_to_writer_with_options(
            &mut bytes,
            &parameters,
            &columns,
            &FcsWriteOptions::new().with_text_keywords(pairs),
        )
        .unwrap();
        FcsReader::new(Cursor::new(bytes)).read().unwrap()
    }

    #[test]
    fn feature_contract_rejects_each_invalid_recipe_and_panel_field() {
        let (data, profile) = fixture();
        for recipe in [
            "cytiq-fcm-arcsinh-v1",
            "cytiq-flowcyt-stored-v1",
            "cytiq-cytof-arcsinh-v1",
        ] {
            let mut p = profile.clone();
            p.recipe = recipe.into();
            assert!(validate_feature_recipe(&p).is_ok());
        }
        for recipe in ["unknown", "cytiq-table-stored-v1"] {
            let mut p = profile.clone();
            p.recipe = recipe.into();
            assert!(validate_feature_recipe(&p).is_err());
        }
        for cofactor in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let mut p = profile.clone();
            p.cofactor = cofactor;
            assert!(validate_feature_recipe(&p).is_err());
        }
        let mut p = profile.clone();
        p.features.clear();
        assert!(validate_feature_recipe(&p).is_err());
        let indices = feature_indices(&data, &profile).unwrap();
        assert_eq!(indices.len(), profile.features.len());
        for (i, feature) in indices.iter().zip(&profile.features) {
            assert_eq!(data.parameters[*i].name, feature.channel);
        }
        let mut p = profile.clone();
        p.features[0].name.clear();
        assert!(feature_indices(&data, &p).is_err());
        p = profile.clone();
        p.features[1].name = p.features[0].name.clone();
        assert!(feature_indices(&data, &p).is_err());
        p = profile.clone();
        p.features[1].channel = p.features[0].channel.clone();
        assert!(feature_indices(&data, &p).is_err());
        p = profile.clone();
        p.features[0].channel = "missing".into();
        assert!(feature_indices(&data, &p).is_err());
        p = profile.clone();
        p.features[0].labels = vec!["wrong".into()];
        assert!(feature_indices(&data, &p).is_err());
        let mut ambiguous = data.clone();
        ambiguous
            .parameters
            .push(ambiguous.parameters[indices[0]].clone());
        assert!(feature_indices(&ambiguous, &profile).is_err());
        let mut missing_label = data.clone();
        missing_label.parameters[indices[0]].label = None;
        p = profile.clone();
        p.features[0].labels = vec!["required".into()];
        assert!(feature_indices(&missing_label, &p).is_err());
        p.features[0].labels.clear();
        assert!(feature_indices(&missing_label, &p).is_ok());
    }

    #[test]
    fn fcm_state_requires_compatible_complete_compensation_and_transforms() {
        let (data, profile) = fixture();
        let fluorescence = profile
            .features
            .iter()
            .filter(|f| f.fluorescence)
            .map(|f| f.channel.clone())
            .collect::<Vec<_>>();
        let compensation = CompensationState {
            source: "fcs".into(),
            channels: fluorescence.clone(),
        };
        let mut state = SignalState::default();
        assert!(validate_fcm_state(&data, &profile, &state).is_ok());
        state.unmixing = Some(UnmixingState {
            components: vec![],
            detectors: vec![],
        });
        assert!(validate_fcm_state(&data, &profile, &state).is_err());
        state = SignalState {
            source_spillover: true,
            ..SignalState::default()
        };
        assert!(validate_fcm_state(&data, &profile, &state).is_err());
        state.compensation = Some(CompensationState {
            source: "fcs".into(),
            channels: vec![],
        });
        assert!(validate_fcm_state(&data, &profile, &state).is_err());
        state.compensation = Some(compensation.clone());
        assert!(validate_fcm_state(&data, &profile, &state).is_ok());
        state.source_spillover = false;
        let transformed = |method: &str, channels: Vec<String>, cofactor| {
            keywords(&data, &[("CYTIQ_TRANSFORM",json!({"method":method,"files":[{"channels":channels}],"parameters":{"cofactor":cofactor}}).to_string())])
        };
        let processed = transformed("arcsinh", fluorescence.clone(), profile.cofactor);
        state.transform = Some(TransformState {
            method: "arcsinh".into(),
            channels: fluorescence.clone(),
        });
        assert!(validate_fcm_state(&processed, &profile, &state).is_ok());
        state.compensation = None;
        assert!(validate_fcm_state(&processed, &profile, &state).is_err());
        state.compensation = Some(compensation);
        state.transform.as_mut().unwrap().method = "logicle".into();
        assert!(validate_fcm_state(&processed, &profile, &state).is_err());
        state.transform.as_mut().unwrap().method = "arcsinh".into();
        state.transform.as_mut().unwrap().channels.clear();
        assert!(validate_fcm_state(&processed, &profile, &state).is_err());
        state.transform.as_mut().unwrap().channels =
            profile.features.iter().map(|f| f.channel.clone()).collect();
        assert!(validate_fcm_state(&processed, &profile, &state).is_err());
        state.transform.as_mut().unwrap().channels = fluorescence.clone();
        assert!(
            validate_fcm_state(
                &transformed("arcsinh", fluorescence, profile.cofactor + 1.0),
                &profile,
                &state
            )
            .is_err()
        );
    }

    #[test]
    fn original_flowcyt_values_reject_each_processing_keyword() {
        let (data, profile) = fixture();
        let indices = feature_indices(&data, &profile).unwrap();
        let values = flowcyt_columns(&data, &indices).unwrap();
        for (index, column) in indices.iter().zip(values) {
            assert_eq!(
                column,
                data.data.columns[*index]
                    .to_f64_vec()
                    .into_iter()
                    .map(|v| v as f32)
                    .collect::<Vec<_>>()
            );
        }
        for key in SIGNAL_KEYWORDS {
            assert!(
                flowcyt_columns(&keywords(&data, &[(key, "present".into())]), &indices).is_err(),
                "{key}"
            );
        }
    }
}
