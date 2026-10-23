//! Viability filter: drop events that are bright for a viability dye.
//!
//! Scatter alone cannot tell dead cells from live ones reliably, so without
//! a viability channel this filter reports `not_assessed` rather than
//! marking every event as live.

use std::collections::HashMap;
use std::f64::consts::LN_10;
use std::path::{Path, PathBuf};

use cytiq_density::{
    DEFAULT_MAX_VALLEY_RATIO, DEFAULT_MIN_PEAK_FRACTION, density, find_valleys, quantile_type7,
};
use cytiq_fcs::{FcsDataset, FcsReader};
use cytiq_gating::{CompensationRef, Dimension, Gate, GateKind, GatingStrategy, Transform};
use serde_json::{Value, json};

use crate::channels::ChannelRoles;
use crate::error::CleanError;
use crate::filter::{FilterFit, FilterReport, FilterStatus, LIVE_GATE, MIN_FIT_EVENTS, finite_at};

pub const DEFAULT_VIABILITY_COFACTOR: f64 = 150.0;
pub const DEFAULT_VIABILITY_CONTROL_QUANTILE: f64 = 0.995;

/// Id of the `fasinh` transformation written for the viability dimension.
pub const VIABILITY_TRANSFORM: &str = "viability_asinh";

/// `fasinh` decades; together with `T = cofactor · sinh(M ln 10)` this makes
/// the Gating-ML transform equal to `asinh(x / cofactor) / (M ln 10)`.
const ASINH_DECADES: f64 = 4.5;

#[derive(Clone, Debug, PartialEq)]
pub struct ViabilityConfig {
    pub cofactor: f64,
    /// Unstained/FMO control whose upper quantile sets the threshold.
    pub control: Option<PathBuf>,
    pub control_quantile: f64,
    /// Explicit threshold in compensated intensity units.
    pub threshold: Option<f64>,
}

impl Default for ViabilityConfig {
    fn default() -> Self {
        Self {
            cofactor: DEFAULT_VIABILITY_COFACTOR,
            control: None,
            control_quantile: DEFAULT_VIABILITY_CONTROL_QUANTILE,
            threshold: None,
        }
    }
}

/// The transformation and dimension used to evaluate the viability gate.
pub(crate) struct ViabilityAxis {
    pub transform: Transform,
    pub dimension: Dimension,
}

impl ViabilityAxis {
    pub(crate) fn new(channel: &str, cofactor: f64) -> Self {
        Self {
            transform: Transform::Asinh {
                t: cofactor * (ASINH_DECADES * LN_10).sinh(),
                m: ASINH_DECADES,
                a: 0.0,
            },
            dimension: Dimension {
                name: channel.to_owned(),
                compensation: CompensationRef::Fcs,
                transform: Some(VIABILITY_TRANSFORM.to_owned()),
                min: None,
                max: None,
            },
        }
    }

    pub(crate) fn transforms(&self) -> HashMap<String, Transform> {
        HashMap::from([(VIABILITY_TRANSFORM.to_owned(), self.transform.clone())])
    }

    /// Compensated, transformed values of every event in `dataset`.
    pub(crate) fn values(&self, dataset: &FcsDataset) -> Result<Vec<f64>, CleanError> {
        let strategy = GatingStrategy::new(Vec::new(), self.transforms(), HashMap::new())?;
        Ok(strategy.dimension_values(dataset, &self.dimension)?)
    }

    /// Back to compensated intensity units (inverse of `fasinh` with `A = 0`).
    fn to_raw(&self, scaled: f64) -> f64 {
        let Transform::Asinh { t, m, .. } = self.transform else {
            unreachable!("viability axis is always fasinh");
        };
        t / (m * LN_10).sinh() * (scaled * m * LN_10).sinh()
    }
}

pub(crate) fn fit(
    dataset: &FcsDataset,
    roles: &ChannelRoles,
    config: &ViabilityConfig,
    fit_indices: &[usize],
) -> Result<(FilterFit, Option<ViabilityAxis>), CleanError> {
    let Some(channel) = &roles.viability else {
        let fit = if roles.viability_source == "disabled" {
            FilterFit::skipped(
                LIVE_GATE,
                FilterStatus::Disabled,
                "disabled with --no-viability".to_owned(),
            )
        } else {
            FilterFit::skipped(
                LIVE_GATE,
                FilterStatus::NotAssessed,
                "no viability channel detected; scatter alone cannot identify dead cells"
                    .to_owned(),
            )
        };
        return Ok((fit, None));
    };

    let axis = ViabilityAxis::new(channel, config.cofactor);
    let values = finite_at(&axis.values(dataset)?, fit_indices);
    if values.len() < MIN_FIT_EVENTS {
        let fit = FilterFit::skipped(
            LIVE_GATE,
            FilterStatus::NotAssessed,
            format!(
                "only {} usable event(s); need at least {MIN_FIT_EVENTS}",
                values.len()
            ),
        );
        return Ok((fit, Some(axis)));
    }

    let (method, threshold, details) = if let Some(raw) = config.threshold {
        ("explicit_threshold", axis.transform.apply(raw), json!({}))
    } else if let Some(control) = &config.control {
        let (threshold, details) = control_threshold(&axis, control, config.control_quantile)?;
        ("control_quantile", threshold, details)
    } else {
        let valley = find_valleys(
            &density(&values),
            DEFAULT_MIN_PEAK_FRACTION,
            DEFAULT_MAX_VALLEY_RATIO,
        )
        .into_iter()
        .next();
        let Some(valley) = valley else {
            let fit = FilterFit::skipped(
                LIVE_GATE,
                FilterStatus::NotAssessed,
                format!(
                    "`{channel}` shows no separate dye-bright population (ambiguous); \
                     pass --viability-control or --viability-threshold"
                ),
            );
            return Ok((fit, Some(axis)));
        };
        (
            "density_valley",
            valley.x,
            json!({
                "live_peak": valley.left_peak_x,
                "dead_peak": valley.right_peak_x,
                "valley_ratio": valley.ratio,
            }),
        )
    };

    let mut dimension = axis.dimension.clone();
    dimension.max = Some(threshold);
    let mut parameters = json!({
        "threshold": threshold,
        "threshold_raw": axis.to_raw(threshold),
        "cofactor": config.cofactor,
        "compensation": "FCS",
        "fit_events": values.len(),
    });
    merge(&mut parameters, details);
    let fit = FilterFit {
        gate: Some(Gate {
            id: LIVE_GATE.to_owned(),
            parent_id: None,
            kind: GateKind::Rectangle {
                dimensions: vec![dimension],
            },
        }),
        report: FilterReport::applied(LIVE_GATE, method, vec![channel.clone()], parameters),
    };
    Ok((fit, Some(axis)))
}

fn control_threshold(
    axis: &ViabilityAxis,
    path: &Path,
    quantile: f64,
) -> Result<(f64, Value), CleanError> {
    let control = FcsReader::open(path)
        .and_then(FcsReader::read)
        .map_err(|source| CleanError::FcsRead {
            path: path.to_path_buf(),
            source,
        })?;
    if control.find_parameter(&axis.dimension.name).is_none() {
        return Err(CleanError::MissingChannel {
            name: axis.dimension.name.clone(),
            path: path.to_path_buf(),
        });
    }
    let mut values = axis
        .values(&control)?
        .into_iter()
        .filter(|value| value.is_finite())
        .collect::<Vec<_>>();
    if values.len() < MIN_FIT_EVENTS {
        return Err(CleanError::InvalidConfig(format!(
            "viability control `{}` has only {} usable event(s)",
            path.display(),
            values.len()
        )));
    }
    values.sort_by(f64::total_cmp);
    Ok((
        quantile_type7(&values, quantile),
        json!({
            "control": path.display().to_string(),
            "control_quantile": quantile,
            "control_events": values.len(),
        }),
    ))
}

fn merge(target: &mut Value, extra: Value) {
    if let (Value::Object(target), Value::Object(extra)) = (target, extra) {
        target.extend(extra);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_matches_plain_asinh_with_cofactor() {
        let axis = ViabilityAxis::new("L/D", 150.0);
        for raw in [-500.0, 0.0, 150.0, 10_000.0] {
            let expected = (raw / 150.0_f64).asinh() / (ASINH_DECADES * LN_10);
            assert!((axis.transform.apply(raw) - expected).abs() < 1e-12);
            assert!((axis.to_raw(axis.transform.apply(raw)) - raw).abs() < 1e-6);
        }
    }
}
