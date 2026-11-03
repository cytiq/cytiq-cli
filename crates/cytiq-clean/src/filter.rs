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

impl FilterStatus {
    /// The status as written in `metrics.json`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::NotAssessed => "not_assessed",
            Self::Disabled => "disabled",
        }
    }
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
