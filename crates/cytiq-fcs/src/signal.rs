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
            transform: transform_state(text)?,
            compensation: compensation_state(text)?,
            unmixing: unmixing_state(text)?,
            source_spillover: text.get(SOURCE_SPILLOVER_KEYWORD).is_some(),
        })
    }

    /// No cytiq stage changed the stored values.
    #[must_use]
    pub const fn is_raw(&self) -> bool {
        self.transform.is_none() && self.compensation.is_none() && self.unmixing.is_none()
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
