//! Debris filter: drop low-FSC events that are not intact cells.

use cytiq_density::{
    DEFAULT_MAX_VALLEY_RATIO, DEFAULT_MIN_PEAK_FRACTION, density, find_valleys, gate_events,
};
use cytiq_fcs::FcsDataset;
use cytiq_gating::{Gate, GateKind};
use serde_json::json;

use crate::channels::ChannelRoles;
use crate::error::CleanError;
use crate::filter::{
    FilterFit, FilterReport, FilterStatus, MIN_FIT_EVENTS, NON_DEBRIS_GATE, finite_at,
    raw_dimension, raw_values,
};

/// A debris peak must sit at most at this fraction of the FSC position of
/// the population to its right. This keeps the valley between two cell
/// populations (e.g. lymphocytes and monocytes) from being taken as the
/// debris boundary when a sample has no debris.
pub const DEBRIS_MAX_PEAK_POSITION_RATIO: f64 = 0.25;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DebrisMode {
    /// FSC-A threshold at the density valley above the low-FSC debris peak.
    #[default]
    Adaptive,
    /// `AutoSpill` FSC/SSC auto-gate around the single dominant population.
    /// Suited to beads and cell lines; drops minor cell populations.
    Population,
    /// User-supplied FSC/SSC minimums.
    Fixed,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DebrisConfig {
    pub mode: DebrisMode,
    pub fsc_min: Option<f64>,
    pub ssc_min: Option<f64>,
}

pub(crate) fn fit(
    dataset: &FcsDataset,
    roles: &ChannelRoles,
    config: &DebrisConfig,
    fit_indices: &[usize],
) -> Result<FilterFit, CleanError> {
    match config.mode {
        DebrisMode::Adaptive => fit_adaptive(dataset, roles, fit_indices),
        DebrisMode::Population => fit_population(dataset, roles, fit_indices),
        DebrisMode::Fixed => Ok(fit_fixed(roles, config)),
    }
}

fn fit_adaptive(
    dataset: &FcsDataset,
    roles: &ChannelRoles,
    fit_indices: &[usize],
) -> Result<FilterFit, CleanError> {
    let fsc = finite_at(&raw_values(dataset, &roles.fsc_a)?, fit_indices);
    if fsc.len() < MIN_FIT_EVENTS {
        return Ok(too_few_events(fsc.len()));
    }
    let valley = find_valleys(
        &density(&fsc),
        DEFAULT_MIN_PEAK_FRACTION,
        DEFAULT_MAX_VALLEY_RATIO,
    )
    .into_iter()
    .find(|valley| valley.left_peak_x <= DEBRIS_MAX_PEAK_POSITION_RATIO * valley.right_peak_x);
    let Some(valley) = valley else {
        return Ok(FilterFit::skipped(
            NON_DEBRIS_GATE,
            FilterStatus::NotAssessed,
            format!(
                "no low-FSC debris population separated from cells on `{}`",
                roles.fsc_a
            ),
        ));
    };

    let mut dimension = raw_dimension(&roles.fsc_a);
    dimension.min = Some(valley.x);
    Ok(FilterFit {
        gate: Some(gate(GateKind::Rectangle {
            dimensions: vec![dimension],
        })),
        report: FilterReport::applied(
            NON_DEBRIS_GATE,
            "fsc_density_valley",
            vec![roles.fsc_a.clone()],
            json!({
                "fsc_min": valley.x,
                "debris_peak": valley.left_peak_x,
                "cell_peak": valley.right_peak_x,
                "valley_ratio": valley.ratio,
                "fit_events": fsc.len(),
            }),
        ),
    })
}

fn fit_population(
    dataset: &FcsDataset,
    roles: &ChannelRoles,
    fit_indices: &[usize],
) -> Result<FilterFit, CleanError> {
    let Some(ssc_a) = &roles.ssc_a else {
        return Err(CleanError::InvalidConfig(
            "--debris-mode population needs an SSC channel".to_owned(),
        ));
    };
    let fsc = raw_values(dataset, &roles.fsc_a)?;
    let ssc = raw_values(dataset, ssc_a)?;
    let points = fit_indices
        .iter()
        .map(|&index| [fsc[index], ssc[index]])
        .filter(|point| point.iter().all(|value| value.is_finite()))
        .collect::<Vec<_>>();
    if points.len() < MIN_FIT_EVENTS {
        return Ok(too_few_events(points.len()));
    }
    let computation = match gate_events(&points, None) {
        Ok(computation) => computation,
        Err(reason) => {
            return Ok(FilterFit::skipped(
                NON_DEBRIS_GATE,
                FilterStatus::NotAssessed,
                format!("FSC/SSC population auto-gate failed: {reason}"),
            ));
        }
    };

    Ok(FilterFit {
        gate: Some(gate(GateKind::Polygon {
            dimensions: [raw_dimension(&roles.fsc_a), raw_dimension(ssc_a)],
            vertices: computation.hull.clone(),
        })),
        report: FilterReport::applied(
            NON_DEBRIS_GATE,
            "fsc_ssc_population",
            vec![roles.fsc_a.clone(), ssc_a.clone()],
            json!({
                "hull_vertices": computation.hull.len(),
                "bound_maxima": computation.bound_maxima,
                "region_maxima": computation.region_maxima,
                "strict_points": computation.strict_points,
                "fit_events": points.len(),
            }),
        ),
    })
}

fn fit_fixed(roles: &ChannelRoles, config: &DebrisConfig) -> FilterFit {
    let mut dimensions = Vec::new();
    let mut channels = Vec::new();
    if let Some(fsc_min) = config.fsc_min {
        let mut dimension = raw_dimension(&roles.fsc_a);
        dimension.min = Some(fsc_min);
        dimensions.push(dimension);
        channels.push(roles.fsc_a.clone());
    }
    if let (Some(ssc_min), Some(ssc_a)) = (config.ssc_min, &roles.ssc_a) {
        let mut dimension = raw_dimension(ssc_a);
        dimension.min = Some(ssc_min);
        dimensions.push(dimension);
        channels.push(ssc_a.clone());
    }
    FilterFit {
        gate: Some(gate(GateKind::Rectangle { dimensions })),
        report: FilterReport::applied(
            NON_DEBRIS_GATE,
            "fixed_threshold",
            channels,
            json!({ "fsc_min": config.fsc_min, "ssc_min": config.ssc_min }),
        ),
    }
}

fn gate(kind: GateKind) -> Gate {
    Gate {
        id: NON_DEBRIS_GATE.to_owned(),
        parent_id: None,
        kind,
    }
}

fn too_few_events(count: usize) -> FilterFit {
    FilterFit::skipped(
        NON_DEBRIS_GATE,
        FilterStatus::NotAssessed,
        format!("only {count} usable event(s); need at least {MIN_FIT_EVENTS}"),
    )
}
