#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss
)]

use std::path::{Path, PathBuf};

use cytiq_fcs::{ColumnData, FcsReader, Parameter};
use nalgebra::DMatrix;
use ndarray::Array2;

use crate::error::AutospillError;
use crate::gating::{GateComputation, gate_events};
use crate::manifest::ControlsManifestRow;
use crate::regression::{robust_linear_regression, skewness, trim_joint_pairs};

// Refinement defaults are grouped here because they define the public algorithm
// behavior of the crate. These values come from the `AutoSpill` paper and the
// upstream R defaults (`get_autospill_param_minimal.r`), not from Rust-specific
// tuning.
//
// Paper:
// https://www.nature.com/articles/s41467-021-23126-8

// Robust regression settings used for the initial spillover estimate and for
// each compensation-error measurement during refinement.
const RLM_ITER_MAX: usize = 100;
const RLM_TRIM_FACTOR: f64 = 0.01;

// Iterative refinement settings. The reference algorithm starts on linear data,
// then moves to transformed / bi-exponential data once the coarse linear error
// is small enough. Lambda is reduced from a coarse update to a fine update when
// the recent delta history stops improving fast enough.
const RS_ITER_MAX: usize = 100;
const RS_LAMBDA_COARSE: f64 = 1.0;
const RS_LAMBDA_FINE: f64 = 0.1;
const RS_DELTA_HISTORY: usize = 10;
const RS_DELTA_THRESHOLD_LINEAR: f64 = 1e-2;
const RS_DELTA_THRESHOLD_BIEXP: f64 = 1e-4;
const RS_DELTA_THRESHOLD_CHANGE: f64 = 1e-6;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MatrixKind {
    #[default]
    Compensation,
    Spillover,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AutospillConfig {
    pub fsc: String,
    pub ssc: String,
    pub matrix_kind: MatrixKind,
}

impl Default for AutospillConfig {
    fn default() -> Self {
        Self {
            fsc: "FSC-A".to_owned(),
            ssc: "SSC-A".to_owned(),
            matrix_kind: MatrixKind::Compensation,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GateStats {
    pub dye: String,
    pub primary_channel: String,
    pub total_events: usize,
    pub gated_events: usize,
    pub strict_points: usize,
    pub bound_maxima: usize,
    pub region_maxima: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConvergenceStep {
    pub iteration: usize,
    pub scale: String,
    pub lambda: f64,
    pub delta: f64,
    pub delta_max: f64,
    pub delta_change: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConvergenceSummary {
    pub converged: bool,
    pub iterations: usize,
    pub final_delta_max: f64,
    pub steps: Vec<ConvergenceStep>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AutospillResult {
    pub channels: Vec<String>,
    pub spillover_matrix: Vec<Vec<f64>>,
    pub compensation_matrix: Vec<Vec<f64>>,
    pub gate_stats: Vec<GateStats>,
    pub convergence: ConvergenceSummary,
}

impl AutospillResult {
    #[must_use]
    pub fn matrix(&self, kind: MatrixKind) -> &[Vec<f64>] {
        match kind {
            MatrixKind::Compensation => &self.compensation_matrix,
            MatrixKind::Spillover => &self.spillover_matrix,
        }
    }
}

pub struct AutospillRunner;

impl AutospillRunner {
    /// Run the `AutoSpill` pipeline for the provided manifest rows and controls.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest references invalid channels or files,
    /// when gating or regression fails, or when spillover refinement does not
    /// converge.
    pub fn run(
        manifest_rows: &[ControlsManifestRow],
        input_dir: impl AsRef<Path>,
        config: &AutospillConfig,
    ) -> Result<AutospillResult, AutospillError> {
        let channels = manifest_rows
            .iter()
            .map(|row| row.primary_channel.clone())
            .collect::<Vec<_>>();
        let controls = load_controls(manifest_rows, input_dir.as_ref(), config, &channels)?;

        let gates = controls
            .iter()
            .map(|control| {
                gate_events(&control.scatter).map_err(|reason| AutospillError::DegenerateGate {
                    path: control.path.clone(),
                    reason,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let gate_stats = controls
            .iter()
            .zip(&gates)
            .map(|(control, gate)| GateStats {
                dye: control.row.dye.clone(),
                primary_channel: control.row.primary_channel.clone(),
                total_events: control.events.nrows(),
                gated_events: gate.gated_indices.len(),
                strict_points: gate.strict_points,
                bound_maxima: gate.bound_maxima,
                region_maxima: gate.region_maxima,
            })
            .collect::<Vec<_>>();

        let initial_spillover = estimate_spillover(&controls, &gates)?;
        let (spillover_matrix, compensation_matrix, convergence) =
            refine_spillover(&controls, &gates, &initial_spillover)?;

        Ok(AutospillResult {
            channels,
            spillover_matrix: matrix_to_rows(&spillover_matrix),
            compensation_matrix: matrix_to_rows(&compensation_matrix),
            gate_stats,
            convergence,
        })
    }
}

#[derive(Clone, Debug)]
struct ControlData {
    row: ControlsManifestRow,
    path: PathBuf,
    events: Array2<f64>,
    scatter: Vec<[f64; 2]>,
    primary_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RefinementScale {
    Linear,
    BiExponential,
}

fn load_controls(
    manifest_rows: &[ControlsManifestRow],
    input_dir: &Path,
    config: &AutospillConfig,
    channels: &[String],
) -> Result<Vec<ControlData>, AutospillError> {
    manifest_rows
        .iter()
        .enumerate()
        .map(|(primary_index, row)| {
            let path = input_dir.join(&row.filename);
            if !path.is_file() {
                return Err(AutospillError::MissingControlFile { path });
            }
            let dataset = FcsReader::open(&path)
                .and_then(FcsReader::read)
                .map_err(|source| AutospillError::FcsRead {
                    path: path.clone(),
                    source,
                })?;
            let event_count = dataset.data.columns.first().map_or(0, ColumnData::len);
            if event_count == 0 {
                return Err(AutospillError::EmptyControl { path });
            }

            let channel_indices = channels
                .iter()
                .map(|channel| {
                    find_parameter_index(&dataset.parameters, channel).ok_or_else(|| {
                        AutospillError::MissingChannel {
                            path: path.clone(),
                            channel: channel.clone(),
                        }
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let fsc_index =
                find_parameter_index(&dataset.parameters, &config.fsc).ok_or_else(|| {
                    AutospillError::MissingChannel {
                        path: path.clone(),
                        channel: config.fsc.clone(),
                    }
                })?;
            let ssc_index =
                find_parameter_index(&dataset.parameters, &config.ssc).ok_or_else(|| {
                    AutospillError::MissingChannel {
                        path: path.clone(),
                        channel: config.ssc.clone(),
                    }
                })?;

            let marker_columns = channel_indices
                .iter()
                .map(|&index| {
                    column_to_f64(
                        dataset
                            .data
                            .column(index)
                            .expect("validated parameter index"),
                    )
                })
                .collect::<Vec<_>>();
            let fsc_values = column_to_f64(
                dataset
                    .data
                    .column(fsc_index)
                    .expect("validated parameter index"),
            );
            let ssc_values = column_to_f64(
                dataset
                    .data
                    .column(ssc_index)
                    .expect("validated parameter index"),
            );

            let mut events = Array2::zeros((event_count, channels.len()));
            for (column_index, column) in marker_columns.iter().enumerate() {
                for (row_index, value) in column.iter().enumerate() {
                    events[(row_index, column_index)] = *value;
                }
            }
            let scatter = fsc_values
                .iter()
                .zip(&ssc_values)
                .map(|(fsc, ssc)| [*fsc, *ssc])
                .collect::<Vec<_>>();

            Ok(ControlData {
                row: row.clone(),
                path,
                events,
                scatter,
                primary_index,
            })
        })
        .collect()
}

fn estimate_spillover(
    controls: &[ControlData],
    gates: &[GateComputation],
) -> Result<DMatrix<f64>, AutospillError> {
    let size = controls.len();
    let mut rows = vec![0.0; size * size];

    for (control, gate) in controls.iter().zip(gates) {
        let primary_values =
            gated_column(&control.events, &gate.gated_indices, control.primary_index);
        for secondary_index in 0..size {
            let cell = &mut rows[control.primary_index * size + secondary_index];
            if secondary_index == control.primary_index {
                *cell = 1.0;
                continue;
            }

            let secondary_values =
                gated_column(&control.events, &gate.gated_indices, secondary_index);
            // The initial matrix is built from one robust fit per
            // `secondary ~ primary` pair after trimming joint extremes.
            let (trimmed_primary, trimmed_secondary) =
                trim_joint_pairs(&primary_values, &secondary_values, RLM_TRIM_FACTOR);
            let regression =
                robust_linear_regression(&trimmed_primary, &trimmed_secondary, RLM_ITER_MAX)
                    .ok_or_else(|| AutospillError::RegressionFailed {
                        path: control.path.clone(),
                        primary_channel: control.row.primary_channel.clone(),
                        secondary_channel: controls[secondary_index].row.primary_channel.clone(),
                    })?;
            *cell = regression.slope;
        }
    }

    Ok(DMatrix::from_row_slice(size, size, &rows))
}

fn refine_spillover(
    controls: &[ControlData],
    gates: &[GateComputation],
    initial_spillover: &DMatrix<f64>,
) -> Result<(DMatrix<f64>, DMatrix<f64>, ConvergenceSummary), AutospillError> {
    let size = controls.len();
    let identity = DMatrix::identity(size, size);
    let mut spillover_current = identity.clone();
    let mut spillover_update = initial_spillover - identity.clone();
    let mut scale = RefinementScale::Linear;
    let mut lambda = RS_LAMBDA_COARSE;
    let mut delta = -1.0;
    let mut delta_threshold = RS_DELTA_THRESHOLD_LINEAR;
    let mut delta_history = [-1.0; RS_DELTA_HISTORY];
    let mut iteration = 0_usize;
    let mut final_iteration = false;
    let mut steps = Vec::new();

    loop {
        // Each iteration applies the current update, renormalizes every row so
        // the diagonal stays at 1, and then inverts the spillover matrix to
        // obtain the current compensation matrix.
        spillover_current += &spillover_update;
        normalize_rows_to_unit_diagonal(&mut spillover_current)?;
        let compensation = spillover_current
            .clone()
            .try_inverse()
            .ok_or(AutospillError::SingularMatrix)?;

        // `slopes` measures the residual compensation error after applying the
        // current matrix. In the ideal case this is the identity matrix.
        let (slopes, _) = compensation_error(controls, gates, &compensation)?;
        let slope_error = slopes - identity.clone();

        let previous_delta = delta;
        delta =
            sample_standard_deviation(slope_error.iter().copied().collect::<Vec<_>>().as_slice());
        let delta_max = slope_error
            .iter()
            .map(|value| value.abs())
            .fold(0.0, f64::max);

        let history_slot = iteration % RS_DELTA_HISTORY;
        delta_history[history_slot] = if previous_delta >= 0.0 {
            delta - previous_delta
        } else {
            -1.0
        };
        let delta_change = delta_history.iter().sum::<f64>() / (delta_history.len() as f64);

        steps.push(ConvergenceStep {
            iteration,
            scale: match scale {
                RefinementScale::Linear => "linear".to_owned(),
                RefinementScale::BiExponential => "bi-exp".to_owned(),
            },
            lambda,
            delta,
            delta_max,
            delta_change,
        });

        // Match the reference schedule:
        // 1. converge coarsely in linear space,
        // 2. switch to transformed space,
        // 3. reduce lambda once progress stalls,
        // 4. stop when transformed-space error is small or no longer improving.
        if scale == RefinementScale::Linear && delta_max < delta_threshold {
            scale = RefinementScale::BiExponential;
            delta_threshold = RS_DELTA_THRESHOLD_BIEXP;
            lambda = RS_LAMBDA_COARSE;
            delta = -1.0;
            delta_history.fill(-1.0);
        }
        if delta_change > -RS_DELTA_THRESHOLD_CHANGE
            && (lambda - RS_LAMBDA_COARSE).abs() <= f64::EPSILON
        {
            lambda = RS_LAMBDA_FINE;
            delta = -1.0;
            delta_history.fill(-1.0);
        }

        let converged = matches!(scale, RefinementScale::BiExponential)
            && (delta_max < delta_threshold || delta_change > -RS_DELTA_THRESHOLD_CHANGE);
        let exit = (converged && final_iteration)
            || (delta_change > -RS_DELTA_THRESHOLD_CHANGE
                && matches!(scale, RefinementScale::Linear))
            || (!converged && iteration == RS_ITER_MAX)
            || iteration > RS_ITER_MAX;
        final_iteration = converged;
        iteration += 1;

        // The published update is `lambda * (slope_error %*% spillover_curr)`.
        // Intuitively, the remaining compensation slope error tells us how the
        // current spillover estimate still distorts other channels.
        spillover_update = lambda * (slope_error * &spillover_current);

        if exit {
            if !converged {
                return Err(AutospillError::NonConvergent {
                    iterations: iteration,
                    delta_max,
                });
            }

            return Ok((
                spillover_current,
                compensation,
                ConvergenceSummary {
                    converged,
                    iterations: iteration,
                    final_delta_max: delta_max,
                    steps,
                },
            ));
        }
    }
}

fn compensation_error(
    controls: &[ControlData],
    gates: &[GateComputation],
    compensation: &DMatrix<f64>,
) -> Result<(DMatrix<f64>, DMatrix<f64>), AutospillError> {
    let size = controls.len();
    let mut slope_values = vec![0.0; size * size];
    let mut skew_values = vec![0.0; size * size];

    for (control, gate) in controls.iter().zip(gates) {
        let compensated = control.events.dot(&dmat_to_array2(compensation));
        let primary_values = gated_column(&compensated, &gate.gated_indices, control.primary_index);
        for (secondary_index, secondary_control) in controls.iter().enumerate().take(size) {
            let offset = control.primary_index * size + secondary_index;
            if secondary_index == control.primary_index {
                slope_values[offset] = 1.0;
                continue;
            }

            let secondary_values = gated_column(&compensated, &gate.gated_indices, secondary_index);
            // Re-measure the compensated slope with the same robust fit used for
            // the initial matrix. Refinement is driven by these observed slopes.
            let (trimmed_primary, trimmed_secondary) =
                trim_joint_pairs(&primary_values, &secondary_values, RLM_TRIM_FACTOR);
            let regression =
                robust_linear_regression(&trimmed_primary, &trimmed_secondary, RLM_ITER_MAX)
                    .ok_or_else(|| AutospillError::RegressionFailed {
                        path: control.path.clone(),
                        primary_channel: control.row.primary_channel.clone(),
                        secondary_channel: secondary_control.row.primary_channel.clone(),
                    })?;
            slope_values[offset] = regression.slope;
            skew_values[offset] = skewness(&trimmed_secondary);
        }
    }

    Ok((
        DMatrix::from_row_slice(size, size, &slope_values),
        DMatrix::from_row_slice(size, size, &skew_values),
    ))
}

fn normalize_rows_to_unit_diagonal(matrix: &mut DMatrix<f64>) -> Result<(), AutospillError> {
    for row in 0..matrix.nrows() {
        let diagonal = matrix[(row, row)];
        if diagonal.abs() <= f64::EPSILON {
            return Err(AutospillError::SingularMatrix);
        }
        for column in 0..matrix.ncols() {
            matrix[(row, column)] /= diagonal;
        }
    }
    Ok(())
}

fn gated_column(matrix: &Array2<f64>, indices: &[usize], column: usize) -> Vec<f64> {
    indices
        .iter()
        .map(|&index| matrix[(index, column)])
        .collect()
}

fn column_to_f64(column: &ColumnData) -> Vec<f64> {
    match column {
        ColumnData::UInt(values) => values.iter().map(|value| *value as f64).collect(),
        ColumnData::F32(values) => values.iter().map(|value| f64::from(*value)).collect(),
        ColumnData::F64(values) => values.clone(),
    }
}

fn find_parameter_index(parameters: &[Parameter], channel: &str) -> Option<usize> {
    parameters
        .iter()
        .position(|parameter| parameter.name == channel)
        .or_else(|| {
            parameters.iter().position(|parameter| {
                parameter
                    .label
                    .as_deref()
                    .is_some_and(|label| label == channel)
            })
        })
}

fn dmat_to_array2(matrix: &DMatrix<f64>) -> Array2<f64> {
    let mut array = Array2::zeros((matrix.nrows(), matrix.ncols()));
    for row in 0..matrix.nrows() {
        for column in 0..matrix.ncols() {
            array[(row, column)] = matrix[(row, column)];
        }
    }
    array
}

fn matrix_to_rows(matrix: &DMatrix<f64>) -> Vec<Vec<f64>> {
    (0..matrix.nrows())
        .map(|row| {
            (0..matrix.ncols())
                .map(|column| matrix[(row, column)])
                .collect()
        })
        .collect()
}

fn sample_standard_deviation(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / (values.len() as f64);
    let variance = values
        .iter()
        .map(|value| {
            let centered = value - mean;
            centered * centered
        })
        .sum::<f64>()
        / ((values.len() - 1) as f64);
    variance.sqrt()
}
