//! Singlet filter: drop doublets whose FSC area is too large for their
//! height (or whose FSC width is too large).

use cytiq_density::{mad, median};
use cytiq_fcs::FcsDataset;
use cytiq_gating::{Gate, GateKind};
use serde_json::json;

use crate::channels::ChannelRoles;
use crate::error::CleanError;
use crate::filter::{
    FilterFit, FilterReport, FilterStatus, MIN_FIT_EVENTS, SINGLETS_GATE, raw_dimension, raw_values,
};

/// Headroom above the largest observed FSC-H for the singlet wedge, so the
/// polygon's top edge never cuts through real events.
const WEDGE_HEADROOM: f64 = 1.05;

pub(crate) fn fit(
    dataset: &FcsDataset,
    roles: &ChannelRoles,
    mad_factor: f64,
    fit_indices: &[usize],
) -> Result<FilterFit, CleanError> {
    if let Some(fsc_h) = &roles.fsc_h {
        fit_area_height(dataset, &roles.fsc_a, fsc_h, mad_factor, fit_indices)
    } else if let Some(fsc_w) = &roles.fsc_w {
        fit_width(dataset, fsc_w, mad_factor, fit_indices)
    } else {
        Ok(FilterFit::skipped(
            SINGLETS_GATE,
            FilterStatus::NotAssessed,
            "neither FSC-H nor FSC-W is present".to_owned(),
        ))
    }
}

/// Keep events whose `FSC-A / FSC-H` ratio lies within `median ± k·MAD`.
/// In FSC-A × FSC-H space this band is a wedge from the origin, which is
/// exported as a triangle polygon.
fn fit_area_height(
    dataset: &FcsDataset,
    fsc_a: &str,
    fsc_h: &str,
    mad_factor: f64,
    fit_indices: &[usize],
) -> Result<FilterFit, CleanError> {
    let area = raw_values(dataset, fsc_a)?;
    let height = raw_values(dataset, fsc_h)?;
    let ratios = fit_indices
        .iter()
        .filter(|&&index| height[index] > 0.0 && area[index].is_finite())
        .map(|&index| area[index] / height[index])
        .collect::<Vec<_>>();
    if ratios.len() < MIN_FIT_EVENTS {
        return Ok(too_few_events(ratios.len()));
    }
    let center = median(&ratios);
    let spread = mad(&ratios, center);
    let low = (center - mad_factor * spread).max(0.0);
    let high = center + mad_factor * spread;
    let top = height
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(1.0_f64, f64::max)
        * WEDGE_HEADROOM;

    Ok(FilterFit {
        gate: Some(Gate {
            id: SINGLETS_GATE.to_owned(),
            parent_id: None,
            kind: GateKind::Polygon {
                dimensions: [raw_dimension(fsc_a), raw_dimension(fsc_h)],
                vertices: vec![[0.0, 0.0], [high * top, top], [low * top, top]],
            },
        }),
        report: FilterReport::applied(
            SINGLETS_GATE,
            "fsc_area_height_ratio",
            vec![fsc_a.to_owned(), fsc_h.to_owned()],
            json!({
                "ratio_median": center,
                "ratio_mad": spread,
                "ratio_min": low,
                "ratio_max": high,
                "mad_factor": mad_factor,
                "fit_events": ratios.len(),
            }),
        ),
    })
}

/// Keep events whose FSC-W is at most `median + k·MAD`.
fn fit_width(
    dataset: &FcsDataset,
    fsc_w: &str,
    mad_factor: f64,
    fit_indices: &[usize],
) -> Result<FilterFit, CleanError> {
    let width = raw_values(dataset, fsc_w)?;
    let values = crate::filter::finite_at(&width, fit_indices);
    if values.len() < MIN_FIT_EVENTS {
        return Ok(too_few_events(values.len()));
    }
    let center = median(&values);
    let spread = mad(&values, center);
    let max = center + mad_factor * spread;
    let mut dimension = raw_dimension(fsc_w);
    dimension.max = Some(max);

    Ok(FilterFit {
        gate: Some(Gate {
            id: SINGLETS_GATE.to_owned(),
            parent_id: None,
            kind: GateKind::Rectangle {
                dimensions: vec![dimension],
            },
        }),
        report: FilterReport::applied(
            SINGLETS_GATE,
            "fsc_width",
            vec![fsc_w.to_owned()],
            json!({
                "width_median": center,
                "width_mad": spread,
                "width_max": max,
                "mad_factor": mad_factor,
                "fit_events": values.len(),
            }),
        ),
    })
}

fn too_few_events(count: usize) -> FilterFit {
    FilterFit::skipped(
        SINGLETS_GATE,
        FilterStatus::NotAssessed,
        format!("only {count} usable event(s); need at least {MIN_FIT_EVENTS}"),
    )
}
