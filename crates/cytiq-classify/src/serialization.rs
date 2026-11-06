//! Shared knowledge serialization; bundle error compatibility stays local.
use crate::ClassifyError;
use serde_json::Value;
use std::path::{Path, PathBuf};

pub(crate) use cytiq_population::serialization::digest;

pub(crate) fn read(path: &Path) -> Result<Value, ClassifyError> {
    cytiq_population::serialization::read(path).map_err(|e| ClassifyError::Invalid(e.to_string()))
}

pub(crate) fn resolve(directory: &Path, stem: &str) -> Result<PathBuf, ClassifyError> {
    cytiq_population::serialization::resolve(directory, stem)
        .map_err(|e| ClassifyError::Invalid(e.to_string()))
}
