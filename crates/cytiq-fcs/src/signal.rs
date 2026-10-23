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
