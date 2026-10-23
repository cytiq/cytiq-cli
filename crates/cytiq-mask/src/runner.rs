use std::path::PathBuf;

use log::debug;

use crate::error::MaskError;
use crate::mask::{EventMask, read_combined};

/// Configuration of `cytiq mask combine`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaskCombineConfig {
    pub masks: Vec<PathBuf>,
    pub output: PathBuf,
    pub overwrite: bool,
}

pub struct MaskCombineRunner;

impl MaskCombineRunner {
    /// Combine the configured masks and write the result; returns the
    /// combined mask.
    ///
    /// # Errors
    ///
    /// Returns an error when the output exists and `overwrite` is false, or
    /// when a mask cannot be read or written.
    pub fn run(config: &MaskCombineConfig) -> Result<EventMask, MaskError> {
        if config.output.exists() && !config.overwrite {
            return Err(MaskError::OutputExists {
                path: config.output.clone(),
            });
        }
        let combined = read_combined(&config.masks)?.ok_or(MaskError::Empty)?;
        combined.write_parquet(&config.output)?;
        debug!(
            "combined {} mask(s): kept {} of {} event(s)",
            config.masks.len(),
            combined.kept(),
            combined.len()
        );
        Ok(combined)
    }
}
