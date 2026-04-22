use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use plotly::common::{Mode, Title};
use plotly::{HeatMap, Layout, Plot, Scatter};
use serde::Serialize;

use crate::error::AutospillError;
use crate::gating::GateComputation;
use crate::regression::{
    ordinary_linear_regression, ordinary_linear_regression_without_intercept,
    robust_standard_deviation,
};
use crate::runner::{
    AutospillResult, AutospreadResult, ControlData, LoadedControls, quantile_bin_count,
    quantile_bins, sqrt_like_transform,
};

const SCATTER_PLOTS_PER_FILE: usize = 12;

#[derive(Clone, Debug, Serialize)]
struct ArtifactSummary {
    kind: String,
    file: String,
}

#[derive(Clone, Debug, Serialize)]
struct DebugSummary<T> {
    mode: &'static str,
    artifacts: Vec<ArtifactSummary>,
    gate_stats: Vec<crate::runner::GateStats>,
    extra: T,
}

#[derive(Clone, Debug, Serialize)]
struct AutospillExtra {
    convergence: crate::runner::ConvergenceSummary,
    primary_channels: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct AutospreadExtra {
    spread_stats: Vec<crate::runner::SpreadStats>,
    primary_channels: Vec<String>,
}

pub(crate) fn write_autospill_debug_artifacts(
    debug_dir: &Path,
    loaded: &LoadedControls,
    gates: &[GateComputation],
    result: &AutospillResult,
) -> Result<(), AutospillError> {
    fs::create_dir_all(debug_dir).map_err(|source| AutospillError::DebugOutputCreate {
        path: debug_dir.to_path_buf(),
        source,
    })?;

    let mut artifacts = Vec::new();
    for (control, gate) in loaded.controls.iter().zip(gates) {
        let file_name = format!("gate_{}.html", slug(&control.row.dye));
        let path = debug_dir.join(&file_name);
        write_html_bundle(
            &path,
            &format!("Gate {}", control.row.dye),
            vec![(
                format!("{} ({})", control.row.dye, control.row.primary_channel),
                gate_plot(control, gate),
            )],
        )?;
        artifacts.push(ArtifactSummary {
            kind: "gate".to_owned(),
            file: file_name,
        });
    }

    let convergence_file = "convergence.html".to_owned();
    write_html_bundle(
        &debug_dir.join(&convergence_file),
        "AutoSpill convergence",
        vec![("Convergence".to_owned(), convergence_plot(result))],
    )?;
    artifacts.push(ArtifactSummary {
        kind: "convergence".to_owned(),
        file: convergence_file,
    });

    for (kind, title, row_labels, col_labels, matrix) in [
        (
            "spillover_matrix",
            "Spillover matrix",
            &result.dyes,
            &result.detectors,
            &result.spillover_matrix,
        ),
        (
            "compensation_matrix",
            "Compensation matrix",
            &result.detectors,
            &result.dyes,
            &result.compensation_matrix,
        ),
    ] {
        let file_name = format!("{kind}.html");
        write_html_bundle(
            &debug_dir.join(&file_name),
            title,
            vec![(title.to_owned(), heatmap_plot(title, row_labels, col_labels, matrix))],
        )?;
        artifacts.push(ArtifactSummary {
            kind: kind.to_owned(),
            file: file_name,
        });
    }

    for (control_index, control) in loaded.controls.iter().enumerate() {
        let sections = autospill_scatter_sections(control, &gates[control_index], result);
        for (batch_index, batch) in sections.chunks(SCATTER_PLOTS_PER_FILE).enumerate() {
            let file_name = format!("scatter_{}_{batch_index:02}.html", slug(&control.row.dye));
            write_html_bundle(
                &debug_dir.join(&file_name),
                &format!("Scatter {}", control.row.dye),
                batch.to_vec(),
            )?;
            artifacts.push(ArtifactSummary {
                kind: "scatter".to_owned(),
                file: file_name,
            });
        }
    }

    let summary = DebugSummary {
        mode: "autospill",
        artifacts,
        gate_stats: result.gate_stats.clone(),
        extra: AutospillExtra {
            convergence: result.convergence.clone(),
            primary_channels: result.primary_channels.clone(),
        },
    };
    write_json(&debug_dir.join("summary.json"), &summary)
}

pub(crate) fn write_autospread_debug_artifacts(
    debug_dir: &Path,
    loaded: &LoadedControls,
    gates: &[GateComputation],
    result: &AutospreadResult,
) -> Result<(), AutospillError> {
    fs::create_dir_all(debug_dir).map_err(|source| AutospillError::DebugOutputCreate {
        path: debug_dir.to_path_buf(),
        source,
    })?;

    let mut artifacts = Vec::new();
    for (control, gate) in loaded.controls.iter().zip(gates) {
        let file_name = format!("gate_{}.html", slug(&control.row.dye));
        write_html_bundle(
            &debug_dir.join(&file_name),
            &format!("Gate {}", control.row.dye),
            vec![(
                format!("{} ({})", control.row.dye, control.row.primary_channel),
                gate_plot(control, gate),
            )],
        )?;
        artifacts.push(ArtifactSummary {
            kind: "gate".to_owned(),
            file: file_name,
        });
    }

    let spread_matrix_file = "spread_matrix.html".to_owned();
    write_html_bundle(
        &debug_dir.join(&spread_matrix_file),
        "Spread matrix",
        vec![(
            "Spread matrix".to_owned(),
            heatmap_plot("Spread matrix", &result.dyes, &result.detectors, &result.spread_matrix),
        )],
    )?;
    artifacts.push(ArtifactSummary {
        kind: "spread_matrix".to_owned(),
        file: spread_matrix_file,
    });

    for (control_index, control) in loaded.controls.iter().enumerate() {
        let sections = autospread_sections(control, &gates[control_index], result);
        for (batch_index, batch) in sections.chunks(SCATTER_PLOTS_PER_FILE).enumerate() {
            let file_name = format!("spread_{}_{batch_index:02}.html", slug(&control.row.dye));
            write_html_bundle(
                &debug_dir.join(&file_name),
                &format!("Spread {}", control.row.dye),
                batch.to_vec(),
            )?;
            artifacts.push(ArtifactSummary {
                kind: "spread".to_owned(),
                file: file_name,
            });
        }
    }

    let summary = DebugSummary {
        mode: "autospread",
        artifacts,
        gate_stats: result.gate_stats.clone(),
        extra: AutospreadExtra {
            spread_stats: result.spread_stats.clone(),
            primary_channels: result.primary_channels.clone(),
        },
    };
    write_json(&debug_dir.join("summary.json"), &summary)
}

fn gate_plot(control: &ControlData, gate: &GateComputation) -> Plot {
    let all_x = control.scatter.iter().map(|point| point[0]).collect::<Vec<_>>();
    let all_y = control.scatter.iter().map(|point| point[1]).collect::<Vec<_>>();
    let gated_x = gate
        .gated_indices
        .iter()
        .map(|&index| control.scatter[index][0])
        .collect::<Vec<_>>();
    let gated_y = gate
        .gated_indices
        .iter()
        .map(|&index| control.scatter[index][1])
        .collect::<Vec<_>>();

    let mut plot = Plot::new();
    plot.add_trace(Scatter::new(all_x, all_y).mode(Mode::Markers).name("all"));
    plot.add_trace(Scatter::new(gated_x, gated_y).mode(Mode::Markers).name("gated"));
    plot.set_layout(Layout::new().title(Title::with_text("FSC/SSC gate")));
    plot
}

fn convergence_plot(result: &AutospillResult) -> Plot {
    let iterations = result
        .convergence
        .steps
        .iter()
        .map(|step| step.iteration)
        .collect::<Vec<_>>();
    let delta = result
        .convergence
        .steps
        .iter()
        .map(|step| step.delta)
        .collect::<Vec<_>>();
    let delta_max = result
        .convergence
        .steps
        .iter()
        .map(|step| step.delta_max)
        .collect::<Vec<_>>();
    let delta_change = result
        .convergence
        .steps
        .iter()
        .map(|step| step.delta_change)
        .collect::<Vec<_>>();

    let mut plot = Plot::new();
    plot.add_trace(Scatter::new(iterations.clone(), delta).mode(Mode::LinesMarkers).name("delta"));
    plot.add_trace(
        Scatter::new(iterations.clone(), delta_max)
            .mode(Mode::LinesMarkers)
            .name("delta_max"),
    );
    plot.add_trace(
        Scatter::new(iterations, delta_change)
            .mode(Mode::LinesMarkers)
            .name("delta_change"),
    );
    plot.set_layout(Layout::new().title(Title::with_text("Convergence")));
    plot
}

fn heatmap_plot(
    title: &str,
    row_labels: &[String],
    col_labels: &[String],
    matrix: &[Vec<f64>],
) -> Plot {
    let mut plot = Plot::new();
    plot.add_trace(HeatMap::new(col_labels.to_vec(), row_labels.to_vec(), matrix.to_vec()));
    plot.set_layout(Layout::new().title(Title::with_text(title)));
    plot
}

fn autospill_scatter_sections(
    control: &ControlData,
    gate: &GateComputation,
    result: &AutospillResult,
) -> Vec<(String, Plot)> {
    let primary = gate
        .gated_indices
        .iter()
        .map(|&index| control.detector_events[(index, control.primary_detector_index)])
        .collect::<Vec<_>>();
    let x_min = primary.iter().copied().fold(f64::INFINITY, f64::min);
    let x_max = primary.iter().copied().fold(f64::NEG_INFINITY, f64::max);

    (0..result.detectors.len())
        .filter(|detector_index| *detector_index != control.primary_detector_index)
        .map(|detector_index| {
            let secondary = gate
                .gated_indices
                .iter()
                .map(|&index| control.detector_events[(index, detector_index)])
                .collect::<Vec<_>>();
            let coefficient = result.spillover_matrix[control.dye_index][detector_index];
            let fit_x = vec![x_min, x_max];
            let fit_y = fit_x.iter().map(|value| coefficient * value).collect::<Vec<_>>();

            let mut plot = Plot::new();
            plot.add_trace(
                Scatter::new(primary.clone(), secondary)
                    .mode(Mode::Markers)
                    .name("gated"),
            );
            plot.add_trace(Scatter::new(fit_x, fit_y).mode(Mode::Lines).name("spillover"));
            plot.set_layout(Layout::new().title(Title::with_text(format!(
                "{} -> {}",
                control.row.dye, result.detectors[detector_index]
            ))));
            (format!("{} -> {}", control.row.dye, result.detectors[detector_index]), plot)
        })
        .collect()
}

fn autospread_sections(
    control: &ControlData,
    gate: &GateComputation,
    result: &AutospreadResult,
) -> Vec<(String, Plot)> {
    let primary_values = gate
        .gated_indices
        .iter()
        .map(|&index| control.detector_events[(index, control.primary_detector_index)])
        .collect::<Vec<_>>();
    let bins = quantile_bins(&primary_values, quantile_bin_count(primary_values.len()));

    (0..result.detectors.len())
        .filter(|detector_index| *detector_index != control.primary_detector_index)
        .filter_map(|detector_index| {
            let secondary_values = gate
                .gated_indices
                .iter()
                .map(|&index| control.detector_events[(index, detector_index)])
                .collect::<Vec<_>>();

            let mut fluorescence = Vec::new();
            let mut sigma = Vec::new();
            for bin in &bins {
                let primary_bin = bin.iter().map(|&index| primary_values[index]).collect::<Vec<_>>();
                let secondary_bin =
                    bin.iter().map(|&index| secondary_values[index]).collect::<Vec<_>>();
                if primary_bin.len() < 3 || secondary_bin.len() < 3 {
                    continue;
                }
                fluorescence.push(crate::regression::median(&primary_bin));
                sigma.push(robust_standard_deviation(&secondary_bin));
            }

            if fluorescence.len() < 2 {
                return None;
            }

            let x = fluorescence
                .iter()
                .map(|value| sqrt_like_transform(*value))
                .collect::<Vec<_>>();
            let first_regression = ordinary_linear_regression(&x, &sigma)?;
            let sigma0 = first_regression.intercept.max(0.0);
            let adjusted_sigma = sigma
                .iter()
                .map(|value| sqrt_like_transform((value * value - sigma0 * sigma0).max(0.0)))
                .collect::<Vec<_>>();
            let spread = ordinary_linear_regression_without_intercept(&x, &adjusted_sigma)
                .map(|summary| summary.slope)
                .unwrap_or(0.0);

            let x_min = x.iter().copied().fold(f64::INFINITY, f64::min);
            let x_max = x.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let fit_x = vec![x_min, x_max];
            let fit_y = fit_x
                .iter()
                .map(|value| first_regression.intercept + first_regression.slope * value)
                .collect::<Vec<_>>();

            let mut plot = Plot::new();
            plot.add_trace(Scatter::new(x, sigma).mode(Mode::Markers).name("bins"));
            plot.add_trace(Scatter::new(fit_x, fit_y).mode(Mode::Lines).name("sigma0 fit"));
            plot.set_layout(Layout::new().title(Title::with_text(format!(
                "{} -> {} (ssm={spread:.4})",
                control.row.dye, result.detectors[detector_index]
            ))));
            Some((
                format!("{} -> {}", control.row.dye, result.detectors[detector_index]),
                plot,
            ))
        })
        .collect()
}

fn write_html_bundle(
    path: &Path,
    title: &str,
    sections: Vec<(String, Plot)>,
) -> Result<(), AutospillError> {
    let mut html = String::new();
    let _ = write!(
        html,
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{title}</title>{}</head><body><h1>{title}</h1>",
        Plot::online_cdn_js()
    );
    for (index, (heading, plot)) in sections.iter().enumerate() {
        let _ = write!(
            html,
            "<section><h2>{heading}</h2>{}</section>",
            plot.to_inline_html(Some(&format!("plot-{index}")))
        );
    }
    html.push_str("</body></html>");
    fs::write(path, html).map_err(|source| AutospillError::DebugWrite {
        path: path.to_path_buf(),
        source,
    })
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), AutospillError> {
    let bytes = serde_json::to_vec_pretty(value).map_err(|source| AutospillError::DebugWrite {
        path: path.to_path_buf(),
        source: std::io::Error::other(source),
    })?;
    fs::write(path, bytes).map_err(|source| AutospillError::DebugWrite {
        path: path.to_path_buf(),
        source,
    })
}

fn slug(value: &str) -> String {
    value.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}
