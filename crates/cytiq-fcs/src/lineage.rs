//! Event lineage between a raw FCS file and files derived from it.

use crate::error::FcsError;
use crate::types::{ColumnData, FcsDataset};

/// 1-based row of each event in the raw file, written by `PeacoQC` (upstream
/// and cytiq) when it removes events.
pub const ORIGINAL_ID_PARAMETER: &str = "Original_ID";

/// 0-based index of each event in the raw file, read from `Original_ID`.
/// `None` when the dataset has no `Original_ID` column, i.e. its rows are the
/// raw rows.
///
/// # Errors
///
/// Returns an error when an `Original_ID` value is not a positive integer that
/// the column type represents exactly.
pub fn source_event_ids(dataset: &FcsDataset) -> Result<Option<Vec<u64>>, FcsError> {
    let Some(index) = dataset
        .parameters
        .iter()
        .position(|parameter| parameter.name == ORIGINAL_ID_PARAMETER)
    else {
        return Ok(None);
    };
    let invalid = |value: String| FcsError::InvalidKeywordValue {
        key: ORIGINAL_ID_PARAMETER.to_owned(),
        value,
    };
    let from_float = |value: f64, exact_limit: f64| {
        if value >= 1.0 && value <= exact_limit && value.fract() == 0.0 {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok(value as u64 - 1)
        } else {
            Err(invalid(value.to_string()))
        }
    };
    let ids = match dataset.data.column(index) {
        Some(ColumnData::UInt(values)) => values
            .iter()
            .map(|value| {
                value
                    .checked_sub(1)
                    .ok_or_else(|| invalid(value.to_string()))
            })
            .collect::<Result<_, _>>()?,
        // Integers above 2^24 (f32) or 2^53 (f64) may already be rounded.
        Some(ColumnData::F32(values)) => values
            .iter()
            .map(|value| from_float(f64::from(*value), 16_777_216.0))
            .collect::<Result<_, _>>()?,
        Some(ColumnData::F64(values)) => values
            .iter()
            .map(|value| from_float(*value, 9_007_199_254_740_992.0))
            .collect::<Result<_, _>>()?,
        None => return Ok(None),
    };
    Ok(Some(ids))
}
