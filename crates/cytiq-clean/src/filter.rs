//! Types shared by the three cleaning filters.

use cytiq_gating::{CompensationRef, Dimension, Gate, GatingStrategy};
use serde::Serialize;
use serde_json::Value;

/// Gate ids written to `gates.gatingml.xml`; `--gates` looks them up again.
pub const NON_DEBRIS_GATE: &str = "non_debris";
pub const SINGLETS_GATE: &str = "singlets";
pub const LIVE_GATE: &str = "live";
pub const CLEAN_GATE: &str = "clean";

/// Fewer usable events than this and a filter reports `not_assessed`.
pub const MIN_FIT_EVENTS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterStatus {
    /// The gate was fitted (or supplied) and contributes to `keep`.
    Applied,
    /// The filter could not be evaluated reliably; its mask column is null
    /// and it does not contribute to `keep`.
    NotAssessed,
    /// The filter was switched off by the user.
    Disabled,
}

/// Per-filter section of `metrics.json`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FilterReport {
    pub status: FilterStatus,
    pub reason: Option<String>,
    pub gate_id: &'static str,
    pub method: Option<&'static str>,
    pub channels: Vec<String>,
    /// Method-specific fitted values (thresholds, medians, valleys, ...).
    pub parameters: Value,
    pub passed: Option<usize>,
    pub removed: Option<usize>,
    pub removed_fraction: Option<f64>,
}

impl FilterReport {
    pub(crate) fn applied(
        gate_id: &'static str,
        method: &'static str,
        channels: Vec<String>,
        parameters: Value,
    ) -> Self {
        Self {
            status: FilterStatus::Applied,
            reason: None,
            gate_id,
            method: Some(method),
            channels,
            parameters,
            passed: None,
            removed: None,
            removed_fraction: None,
        }
    }

    pub(crate) fn skipped(gate_id: &'static str, status: FilterStatus, reason: String) -> Self {
        Self {
            status,
            reason: Some(reason),
            gate_id,
            method: None,
            channels: Vec::new(),
            parameters: Value::Null,
            passed: None,
            removed: None,
            removed_fraction: None,
        }
    }
}

/// Outcome of fitting one filter: a gate when it was applied.
#[derive(Clone, Debug)]
pub struct FilterFit {
    pub gate: Option<Gate>,
    pub report: FilterReport,
}

impl FilterFit {
    pub(crate) fn skipped(gate_id: &'static str, status: FilterStatus, reason: String) -> Self {
        Self {
            gate: None,
            report: FilterReport::skipped(gate_id, status, reason),
        }
    }
}

/// An uncompensated, untransformed dimension (scatter channels).
pub(crate) fn raw_dimension(name: &str) -> Dimension {
    Dimension {
        name: name.to_owned(),
        compensation: CompensationRef::Uncompensated,
        transform: None,
        min: None,
        max: None,
    }
}

/// Raw values of a scatter channel, as gate evaluation sees them.
pub(crate) fn raw_values(
    dataset: &cytiq_fcs::FcsDataset,
    name: &str,
) -> Result<Vec<f64>, cytiq_gating::GatingError> {
    GatingStrategy::default().dimension_values(dataset, &raw_dimension(name))
}

/// Values at `indices` that are finite.
pub(crate) fn finite_at(values: &[f64], indices: &[usize]) -> Vec<f64> {
    indices
        .iter()
        .map(|&index| values[index])
        .filter(|value| value.is_finite())
        .collect()
}

/// Evenly spaced subset of the events selected by `mask`, at most `limit`.
/// Deterministic, so repeated runs fit identical gates.
#[must_use]
pub fn sample_indices(mask: &[bool], limit: usize) -> Vec<usize> {
    let selected = mask
        .iter()
        .enumerate()
        .filter_map(|(index, keep)| keep.then_some(index))
        .collect::<Vec<_>>();
    if selected.len() <= limit || limit == 0 {
        return selected;
    }
    (0..limit)
        .map(|position| selected[position * selected.len() / limit])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_evenly_from_selected_events() {
        let mask = (0..1000).map(|index| index % 2 == 0).collect::<Vec<_>>();
        let sample = sample_indices(&mask, 100);
        assert_eq!(sample.len(), 100);
        assert!(sample.iter().all(|index| index % 2 == 0));
        assert_eq!(sample[0], 0);
        assert!(*sample.last().unwrap() > 900);
    }

    #[test]
    fn keeps_everything_under_the_limit() {
        assert_eq!(sample_indices(&[true, false, true], 10), [0, 2]);
    }
}
