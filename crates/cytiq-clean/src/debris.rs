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
    /// No debris filter: it is `disabled` and removes nothing. For files
    /// another tool already gated, where no debris is left and the lowest-FSC
    /// population is cells.
    Off,
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
        DebrisMode::Off => Ok(FilterFit::skipped(
            NON_DEBRIS_GATE,
            FilterStatus::Disabled,
            "disabled with --debris-mode off".to_owned(),
        )),
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

#[cfg(test)]
mod tests {
    use super::*;
    use cytiq_fcs::{ColumnData, FcsReader};
    use std::path::PathBuf;

    fn dataset() -> FcsDataset {
        FcsReader::open(
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/clean/sample.fcs"),
        )
        .unwrap()
        .read()
        .unwrap()
    }
    fn roles() -> ChannelRoles {
        ChannelRoles {
            fsc_a: "FSC-A".into(),
            ssc_a: Some("SSC-A".into()),
            ..ChannelRoles::default()
        }
    }

    #[test]
    fn population_requires_scatter_channels() {
        let data = dataset();
        let missing_roles = ChannelRoles {
            ssc_a: None,
            ..roles()
        };
        assert!(
            matches!(fit_population(&data, &missing_roles, &[]), Err(CleanError::InvalidConfig(reason)) if reason == "--debris-mode population needs an SSC channel")
        );
        let roles = ChannelRoles {
            ssc_a: Some("missing".into()),
            ..roles()
        };
        assert!(fit_population(&data, &roles, &[]).is_err());
    }

    #[test]
    fn population_reports_too_few_usable_events_and_degenerate_scatter() {
        let mut data = dataset();
        let fsc = data.find_parameter("FSC-A").unwrap();
        data.data.columns[fsc] = ColumnData::F64(vec![f64::NAN; data.event_count()]);
        let fitted = fit_population(&data, &roles(), &(0..100).collect::<Vec<_>>()).unwrap();
        assert!(fitted.gate.is_none());
        assert_eq!(fitted.report.status, FilterStatus::NotAssessed);
        assert_eq!(
            fitted.report.reason.as_deref(),
            Some("only 0 usable event(s); need at least 100")
        );
        data.data.columns[fsc] = ColumnData::F64(vec![10.0; data.event_count()]);
        let fitted = fit_population(&data, &roles(), &(0..99).collect::<Vec<_>>()).unwrap();
        assert_eq!(
            fitted.report.reason.as_deref(),
            Some("only 99 usable event(s); need at least 100")
        );
        let fitted = fit_population(&data, &roles(), &(0..100).collect::<Vec<_>>()).unwrap();
        assert!(fitted.gate.is_none());
        assert_eq!(fitted.report.status, FilterStatus::NotAssessed);
        assert_eq!(
            fitted.report.reason.as_deref(),
            Some("FSC/SSC population auto-gate failed: scatter values are constant")
        );
    }

    #[test]
    fn population_produces_a_raw_polygon_and_reproducible_report() {
        let data = dataset();
        let fitted = fit_population(
            &data,
            &roles(),
            &(0..data.event_count()).collect::<Vec<_>>(),
        )
        .unwrap();
        let gate = fitted.gate.unwrap();
        assert_eq!(gate.id, NON_DEBRIS_GATE);
        assert_eq!(gate.parent_id, None);
        let GateKind::Polygon {
            dimensions,
            vertices,
        } = gate.kind
        else {
            panic!("expected polygon");
        };
        assert_eq!(dimensions, [raw_dimension("FSC-A"), raw_dimension("SSC-A")]);
        assert!(vertices.len() >= 3);
        assert_eq!(fitted.report.status, FilterStatus::Applied);
        assert_eq!(fitted.report.method, Some("fsc_ssc_population"));
        assert_eq!(fitted.report.channels, ["FSC-A", "SSC-A"]);
        assert_eq!(fitted.report.parameters["fit_events"], data.event_count());
        assert_eq!(fitted.report.parameters["hull_vertices"], vertices.len());
        assert!(fitted.report.parameters["strict_points"].as_u64().unwrap() >= 3);
    }
}
