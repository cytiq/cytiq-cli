//! Signal state recorded by cytiq stages, so later stages can refuse to
//! transform or compensate values twice.

use serde::{Deserialize, Serialize};

use crate::error::FcsError;
use crate::types::FcsText;

/// Written by `cytiq transform`; its JSON value describes the transform.
pub const TRANSFORM_KEYWORD: &str = "CYTIQ_TRANSFORM";
/// Written by `cytiq autospectral`; its JSON value is an [`UnmixingState`].
pub const UNMIXING_KEYWORD: &str = "CYTIQ_UNMIXING";
/// Written by `cytiq compensate`; its JSON value is a [`CompensationState`].
pub const COMPENSATION_KEYWORD: &str = "CYTIQ_COMPENSATION";
/// Population-specific batch normalization, applied after transformation.
pub const NORMALIZATION_KEYWORD: &str = "CYTIQ_NORMALIZATION";

/// Portable identity of the normalizer that produced the stored values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NormalizationState {
    pub schema_version: u32,
    pub model_sha256: String,
    pub batch: String,
    pub panel_sha256: String,
    pub channels: Vec<String>,
}

/// Read and validate the normalization keyword.
///
/// # Errors
/// Rejects malformed states, unknown versions and invalid checksums.
pub fn normalization_state(text: &FcsText) -> Result<Option<NormalizationState>, FcsError> {
    let Some(value) = text.get(NORMALIZATION_KEYWORD) else {
        return Ok(None);
    };
    let invalid = || FcsError::InvalidKeywordValue {
        key: NORMALIZATION_KEYWORD.to_owned(),
        value: value.to_owned(),
    };
    let state: NormalizationState = serde_json::from_str(value).map_err(|_| invalid())?;
    let unique: std::collections::HashSet<_> = state.channels.iter().collect();
    if state.schema_version != 1
        || !crate::is_sha256(&state.model_sha256)
        || !crate::is_sha256(&state.panel_sha256)
        || state.batch.trim().is_empty()
        || state.channels.is_empty()
        || unique.len() != state.channels.len()
        || state.channels.iter().any(|c| c.trim().is_empty())
    {
        return Err(invalid());
    }
    Ok(Some(state))
}

/// Comparable transformation settings without paths, metrics or timestamps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransformContract {
    pub method: String,
    pub parameters: serde_json::Value,
}

impl TransformContract {
    /// Require complete Cytiq transform metadata covering every selected channel.
    ///
    /// # Errors
    /// Rejects absent, incomplete or unsupported transformation metadata.
    pub fn of(text: &FcsText, channels: &[String]) -> Result<Self, FcsError> {
        let value = text.get(TRANSFORM_KEYWORD).unwrap_or_default();
        let invalid = || FcsError::InvalidKeywordValue {
            key: TRANSFORM_KEYWORD.to_owned(),
            value: value.to_owned(),
        };
        let state = transform_state(text)?.ok_or_else(invalid)?;
        if channels.iter().any(|c| !state.covers(c)) {
            return Err(invalid());
        }
        let doc: serde_json::Value = serde_json::from_str(value).map_err(|_| invalid())?;
        let parameters = doc.get("parameters").cloned().ok_or_else(invalid)?;
        let (settings, names): (&serde_json::Value, &[&str]) = match state.method.as_str() {
            "arcsinh" => (&parameters, &["cofactor"]),
            "logicle" => (&parameters["logicle"], &["t", "m", "w", "a"]),
            "hyperlog" => (&parameters["hyperlog"], &["t", "m", "w", "a"]),
            "biexponential" => (
                &parameters["biexponential"],
                &["length", "max_range", "pos", "neg", "width"],
            ),
            _ => return Err(invalid()),
        };
        if names
            .iter()
            .any(|n| !settings[*n].as_f64().is_some_and(f64::is_finite))
            || (state.method == "arcsinh" && settings["cofactor"].as_f64().unwrap_or(0.0) <= 0.0)
        {
            return Err(invalid());
        }
        let fields: serde_json::Map<String, serde_json::Value> = names
            .iter()
            .map(|name| {
                Ok((
                    (*name).to_owned(),
                    serde_json::Value::from(settings[*name].as_f64().ok_or_else(invalid)?),
                ))
            })
            .collect::<Result<_, FcsError>>()?;
        let parameters = if state.method == "arcsinh" {
            serde_json::Value::Object(fields)
        } else {
            serde_json::json!({state.method.clone(): fields})
        };
        Ok(Self {
            method: state.method,
            parameters,
        })
    }
}
/// The source spillover matrix of a transformed or compensated file. It no
/// longer applies to the stored values, so it is kept under a non-standard
/// keyword.
pub const SOURCE_SPILLOVER_KEYWORD: &str = "CYTIQ_SOURCE_SPILL";

/// Transform recorded in `CYTIQ_TRANSFORM`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransformState {
    pub method: String,
    /// Channels (`$PnN`) whose values were transformed.
    pub channels: Vec<String>,
}

impl TransformState {
    #[must_use]
    pub fn covers(&self, channel: &str) -> bool {
        self.channels.iter().any(|name| name == channel)
    }
}

#[derive(Deserialize)]
struct TransformDocument {
    method: String,
    #[serde(default)]
    files: Vec<TransformDocumentFile>,
}

#[derive(Deserialize)]
struct TransformDocumentFile {
    #[serde(default)]
    channels: Vec<String>,
}

/// The transform a dataset went through, if any.
///
/// # Errors
///
/// Returns an error when `CYTIQ_TRANSFORM` is present but cannot be parsed.
pub fn transform_state(text: &FcsText) -> Result<Option<TransformState>, FcsError> {
    let Some(value) = text.get(TRANSFORM_KEYWORD) else {
        return Ok(None);
    };
    let document: TransformDocument =
        serde_json::from_str(value).map_err(|_| FcsError::InvalidKeywordValue {
            key: TRANSFORM_KEYWORD.to_owned(),
            value: value.to_owned(),
        })?;
    Ok(Some(TransformState {
        method: document.method,
        channels: document
            .files
            .into_iter()
            .flat_map(|file| file.channels)
            .collect(),
    }))
}

/// Compensation recorded in `CYTIQ_COMPENSATION`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompensationState {
    /// `fcs` for the file's own spillover keyword, otherwise the matrix file.
    pub source: String,
    /// Channels (`$PnN`) whose values were compensated.
    pub channels: Vec<String>,
}

impl CompensationState {
    #[must_use]
    pub fn covers(&self, channel: &str) -> bool {
        self.channels.iter().any(|name| name == channel)
    }

    /// The `CYTIQ_COMPENSATION` keyword value.
    ///
    /// # Panics
    ///
    /// Never: the state holds only strings.
    #[must_use]
    pub fn to_keyword_value(&self) -> String {
        serde_json::to_string(self).expect("compensation state serializes to JSON")
    }
}

/// The compensation a dataset went through, if any.
///
/// # Errors
///
/// Returns an error when `CYTIQ_COMPENSATION` is present but cannot be parsed.
pub fn compensation_state(text: &FcsText) -> Result<Option<CompensationState>, FcsError> {
    text.get(COMPENSATION_KEYWORD)
        .map(|value| {
            serde_json::from_str(value).map_err(|_| FcsError::InvalidKeywordValue {
                key: COMPENSATION_KEYWORD.to_owned(),
                value: value.to_owned(),
            })
        })
        .transpose()
}

/// Spectral unmixing recorded in `CYTIQ_UNMIXING`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmixingState {
    /// Unmixed abundance parameters (`$PnN`) written in place of detectors.
    pub components: Vec<String>,
    /// Detector channels the components were unmixed from.
    pub detectors: Vec<String>,
}

impl UnmixingState {
    #[must_use]
    pub fn covers(&self, channel: &str) -> bool {
        self.components.iter().any(|name| name == channel)
    }

    /// The `CYTIQ_UNMIXING` keyword value.
    ///
    /// # Panics
    ///
    /// Never: the state holds only strings.
    #[must_use]
    pub fn to_keyword_value(&self) -> String {
        serde_json::to_string(self).expect("unmixing state serializes to JSON")
    }
}

/// The spectral unmixing a dataset went through, if any.
///
/// # Errors
///
/// Returns an error when `CYTIQ_UNMIXING` is present but cannot be parsed.
pub fn unmixing_state(text: &FcsText) -> Result<Option<UnmixingState>, FcsError> {
    text.get(UNMIXING_KEYWORD)
        .map(|value| {
            serde_json::from_str(value).map_err(|_| FcsError::InvalidKeywordValue {
                key: UNMIXING_KEYWORD.to_owned(),
                value: value.to_owned(),
            })
        })
        .transpose()
}

/// Every processing step cytiq recorded in a dataset, read once so stages
/// check their input the same way.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SignalState {
    pub normalization: Option<NormalizationState>,
    pub transform: Option<TransformState>,
    pub compensation: Option<CompensationState>,
    pub unmixing: Option<UnmixingState>,
    /// `CYTIQ_SOURCE_SPILL` is present: a cytiq stage moved the file's
    /// spillover matrix aside.
    pub source_spillover: bool,
}

impl SignalState {
    /// Read the `CYTIQ_*` signal keywords of a dataset.
    ///
    /// # Errors
    ///
    /// Returns an error when a signal keyword is present but cannot be parsed.
    pub fn of(text: &FcsText) -> Result<Self, FcsError> {
        Ok(Self {
            normalization: normalization_state(text)?,
            transform: transform_state(text)?,
            compensation: compensation_state(text)?,
            unmixing: unmixing_state(text)?,
            source_spillover: text.get(SOURCE_SPILLOVER_KEYWORD).is_some(),
        })
    }

    /// No cytiq stage changed the stored values.
    #[must_use]
    pub const fn is_raw(&self) -> bool {
        self.transform.is_none()
            && self.compensation.is_none()
            && self.unmixing.is_none()
            && self.normalization.is_none()
    }

    /// Whether `channel` holds transformed values. Files from older cytiq
    /// versions do not list their channels, so their transform covers all.
    #[must_use]
    pub fn transformed(&self, channel: &str) -> bool {
        self.transform
            .as_ref()
            .is_some_and(|state| state.channels.is_empty() || state.covers(channel))
    }

    #[must_use]
    pub fn compensated(&self, channel: &str) -> bool {
        self.compensation
            .as_ref()
            .is_some_and(|state| state.covers(channel))
    }

    #[must_use]
    pub fn unmixed(&self, channel: &str) -> bool {
        self.unmixing
            .as_ref()
            .is_some_and(|state| state.covers(channel))
    }
}

/// The steps in pipeline order, for error messages, for example
/// "compensated by `cytiq compensate`, transformed (arcsinh) by `cytiq
/// transform`", or "raw".
impl std::fmt::Display for SignalState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut steps = Vec::new();
        if self.normalization.is_some() {
            steps.push("normalized by `cytiq cytonorm`".to_owned());
        }
        if self.compensation.is_some() {
            steps.push("compensated by `cytiq compensate`".to_owned());
        }
        if self.unmixing.is_some() {
            steps.push("unmixed by `cytiq autospectral`".to_owned());
        }
        if let Some(transform) = &self.transform {
            steps.push(format!(
                "transformed ({}) by `cytiq transform`",
                transform.method
            ));
        }
        if steps.is_empty() {
            formatter.write_str("raw")
        } else {
            formatter.write_str(&steps.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FcsReader, FcsWriteOptions, FcsWriteParameter};

    fn text(keywords: &[(&str, &str)]) -> FcsText {
        let options = keywords
            .iter()
            .fold(FcsWriteOptions::new(), |options, (key, value)| {
                options.with_text_keyword(*key, *value)
            });
        let mut bytes = Vec::new();
        crate::write_fcs3_1_f32_to_writer_with_options(
            &mut bytes,
            &[
                FcsWriteParameter::new("FL1-A"),
                FcsWriteParameter::new("FL2-A"),
            ],
            &[vec![1.0], vec![2.0]],
            &options,
        )
        .unwrap();
        FcsReader::new(std::io::Cursor::new(bytes))
            .read()
            .unwrap()
            .text
    }

    #[test]
    fn a_file_without_signal_keywords_is_raw() {
        let state = SignalState::of(&text(&[("$SPILLOVER", "1,FL1-A,1")])).unwrap();
        assert!(state.is_raw());
        assert!(!state.source_spillover);
        assert_eq!(state.to_string(), "raw");
    }

    #[test]
    fn normalization_requires_valid_identity_and_is_never_raw() {
        let value = serde_json::json!({"schema_version":1,"model_sha256":"ab".repeat(32),"panel_sha256":"cd".repeat(32),"batch":"day1","channels":["FL1-A"]});
        let state = SignalState::of(&text(&[(NORMALIZATION_KEYWORD, &value.to_string())])).unwrap();
        assert!(!state.is_raw());
        assert!(state.to_string().contains("normalized"));
        for mutation in [serde_json::json!(2), serde_json::json!("bad")] {
            let mut invalid = value.clone();
            if mutation.is_number() {
                invalid["schema_version"] = mutation;
            } else {
                invalid["model_sha256"] = mutation;
            }
            assert!(
                normalization_state(&text(&[(NORMALIZATION_KEYWORD, &invalid.to_string())]))
                    .is_err()
            );
        }
    }

    #[test]
    fn complete_transform_contract_rejects_missing_parameters_or_channel_coverage() {
        let valid = r#"{"method":"arcsinh","parameters":{"cofactor":150.0},"files":[{"channels":["FL1-A"]}]}"#;
        let selected = vec!["FL1-A".to_owned()];
        assert!(TransformContract::of(&text(&[(TRANSFORM_KEYWORD, valid)]), &selected).is_ok());
        assert!(
            TransformContract::of(
                &text(&[(
                    TRANSFORM_KEYWORD,
                    r#"{"method":"arcsinh","files":[{"channels":["FL1-A"]}]}"#
                )]),
                &selected
            )
            .is_err()
        );
        assert!(
            TransformContract::of(&text(&[(TRANSFORM_KEYWORD, valid)]), &["FL2-A".to_owned()])
                .is_err()
        );
    }

    #[test]
    fn reads_every_step_in_pipeline_order() {
        let state = SignalState::of(&text(&[
            (
                TRANSFORM_KEYWORD,
                r#"{"method":"arcsinh","files":[{"channels":["FL1-A"]}]}"#,
            ),
            (
                COMPENSATION_KEYWORD,
                r#"{"source":"fcs","channels":["FL1-A","FL2-A"]}"#,
            ),
            (SOURCE_SPILLOVER_KEYWORD, "2,FL1-A,FL2-A,1,0,0,1"),
        ]))
        .unwrap();
        assert!(!state.is_raw());
        assert!(state.transformed("FL1-A"));
        assert!(!state.transformed("FL2-A"));
        assert!(state.compensated("FL2-A"));
        assert!(!state.unmixed("FL1-A"));
        assert!(state.source_spillover);
        assert_eq!(
            state.to_string(),
            "compensated by `cytiq compensate`, transformed (arcsinh) by `cytiq transform`"
        );
    }

    #[test]
    fn a_legacy_transform_covers_every_channel() {
        let state =
            SignalState::of(&text(&[(TRANSFORM_KEYWORD, r#"{"method":"logicle"}"#)])).unwrap();
        assert!(state.transformed("FL2-A"));
    }

    #[test]
    fn rejects_a_malformed_signal_keyword() {
        assert!(SignalState::of(&text(&[(UNMIXING_KEYWORD, "not json")])).is_err());
    }
}
