//! Signal state recorded by cytiq stages, so later stages can refuse to
//! transform or compensate values twice.

use serde::Deserialize;

use crate::error::FcsError;
use crate::types::FcsText;

/// Written by `cytiq transform`; its JSON value describes the transform.
pub const TRANSFORM_KEYWORD: &str = "CYTIQ_TRANSFORM";
/// The source spillover matrix of a transformed file. It no longer applies to
/// the stored values, so it is kept under a non-standard keyword.
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
