use std::path::{Path, PathBuf};

use cytiq_fcs::{
    COMPENSATION_KEYWORD, CompensationState, FcsDataset, FcsReader, FcsWriteOptions,
    FcsWriteParameter, MARGINS_KEYWORD, SOURCE_SPILLOVER_KEYWORD,
};
use cytiq_mask::EventMask;
use log::{debug, info};

use crate::error::CompensateError;
use crate::matrix::{CompensationMatrix, MatrixKind, read_matrix_file};

/// Configuration of `cytiq compensate`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompensateConfig {
    pub input: PathBuf,
    pub output: PathBuf,
    /// Matrix file and what it holds; `None` uses the input's `$SPILLOVER`.
    pub matrix: Option<(PathBuf, MatrixKind)>,
    pub overwrite: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompensateResult {
    pub output: PathBuf,
    pub masks: PathBuf,
    /// Compensated channels (`$PnN`), in matrix order.
    pub channels: Vec<String>,
    pub event_count: usize,
    /// Events on a raw detector margin of a compensated channel.
    pub margin_event_count: usize,
}

/// `sample.comp.fcs` -> `sample.comp.masks.parquet`.
#[must_use]
pub fn masks_path(output: &Path) -> PathBuf {
    output.with_extension("masks.parquet")
}

pub struct CompensateRunner;

impl CompensateRunner {
    /// Compensate one FCS file and write the compensated FCS plus a raw-margin
    /// event mask next to it.
    ///
    /// # Errors
    ///
    /// Returns an error when the input is missing, already compensated or
    /// transformed on a matrix channel, the matrix is invalid or does not
    /// match the input channels, or an output cannot be written.
    pub fn run(config: &CompensateConfig) -> Result<CompensateResult, CompensateError> {
        validate_paths(config)?;
        let input = &config.input;
        let dataset = load_dataset(input)?;
        let matrix = match &config.matrix {
            Some((path, kind)) => read_matrix_file(path, *kind)?,
            None => CompensationMatrix::from_dataset(input, &dataset)?,
        };
        let matrix_indices = resolve_matrix_channels(input, &dataset, &matrix)?;
        let channels = matrix_indices
            .iter()
            .map(|index| dataset.parameters[*index].name.clone())
            .collect::<Vec<_>>();
        ensure_raw_input(input, &dataset, &channels)?;
        info!(
            "compensate: {} event(s), {} channel(s) from {}",
            dataset.event_count(),
            channels.len(),
            config.matrix.as_ref().map_or_else(
                || "$SPILLOVER".to_owned(),
                |(path, _)| path.display().to_string()
            )
        );

        // Compensation mixes channels: this is the last point where raw
        // detector margins can be identified.
        let margin_pass =
            cytiq_fcs::raw_margin_pass(&dataset, &matrix_indices).map_err(|source| {
                CompensateError::FcsRead {
                    path: input.clone(),
                    source,
                }
            })?;
        let (parameters, columns) = compensated_columns(&dataset, &matrix_indices, &matrix);
        let options = output_keywords(input, &dataset, config, &channels, &parameters)?;
        write_fcs(&config.output, &parameters, &columns, &options)?;

        let masks = masks_path(&config.output);
        let source_event_ids =
            cytiq_fcs::source_event_ids(&dataset).map_err(|source| CompensateError::FcsRead {
                path: input.clone(),
                source,
            })?;
        let mask = EventMask::new(margin_pass.clone(), source_event_ids)
            .with_filter("margin_pass", margin_pass.into_iter().map(Some).collect());
        mask.write_parquet(&masks)?;

        Ok(CompensateResult {
            output: config.output.clone(),
            masks,
            channels,
            event_count: mask.len(),
            margin_event_count: mask.len() - mask.kept(),
        })
    }
}

fn validate_paths(config: &CompensateConfig) -> Result<(), CompensateError> {
    if !config.input.is_file() {
        return Err(CompensateError::MissingInput {
            path: config.input.clone(),
        });
    }
    let is_fcs = config
        .output
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fcs"));
    if !is_fcs {
        return Err(CompensateError::UnsupportedOutputFormat {
            path: config.output.clone(),
        });
    }
    for path in [config.output.clone(), masks_path(&config.output)] {
        if path.exists() && !config.overwrite {
            return Err(CompensateError::OutputExists { path });
        }
    }
    Ok(())
}

fn load_dataset(path: &Path) -> Result<FcsDataset, CompensateError> {
    debug!("loading FCS file {}", path.display());
    let dataset = FcsReader::open(path)
        .and_then(FcsReader::read)
        .map_err(|source| CompensateError::FcsRead {
            path: path.to_path_buf(),
            source,
        })?;
    if dataset.event_count() == 0 {
        return Err(CompensateError::EmptyInput {
            path: path.to_path_buf(),
        });
    }
    Ok(dataset)
}

fn resolve_matrix_channels(
    path: &Path,
    dataset: &FcsDataset,
    matrix: &CompensationMatrix,
) -> Result<Vec<usize>, CompensateError> {
    matrix
        .channels
        .iter()
        .map(|channel| {
            dataset
                .find_parameter(channel)
                .ok_or_else(|| CompensateError::MissingChannel {
                    path: path.to_path_buf(),
                    channel: channel.clone(),
                })
        })
        .collect()
}

fn ensure_raw_input(
    path: &Path,
    dataset: &FcsDataset,
    channels: &[String],
) -> Result<(), CompensateError> {
    let read_error = |source| CompensateError::FcsRead {
        path: path.to_path_buf(),
        source,
    };
    if let Some(state) = cytiq_fcs::compensation_state(&dataset.text).map_err(read_error)? {
        return Err(CompensateError::AlreadyCompensated {
            path: path.to_path_buf(),
            channels: state.channels,
        });
    }
    if let Some(state) = cytiq_fcs::transform_state(&dataset.text).map_err(read_error)?
        && let Some(channel) = channels
            .iter()
            .find(|channel| state.channels.is_empty() || state.covers(channel))
    {
        return Err(CompensateError::TransformedInput {
            path: path.to_path_buf(),
            channel: channel.clone(),
        });
    }
    Ok(())
}

fn stored_values(dataset: &FcsDataset, index: usize) -> Vec<f64> {
    dataset
        .data
        .column(index)
        .expect("every parameter has a data column")
        .to_f64_vec()
}

/// Output parameters and values. Matrix channels hold compensated values;
/// other channels whose `$PnE`/`$PnG` scaling changes their values are written
/// linearised, because the output always declares `$PnE=0,0`. Changed channels
/// lose `$PnG` and take a `$PnR` computed from their new values.
fn compensated_columns(
    dataset: &FcsDataset,
    matrix_indices: &[usize],
    matrix: &CompensationMatrix,
) -> (Vec<FcsWriteParameter>, Vec<Vec<f64>>) {
    let linear = matrix_indices
        .iter()
        .map(|index| cytiq_fcs::linear_values(dataset, *index))
        .collect::<Vec<_>>();
    let compensated = cytiq_fcs::compensate(&linear, &matrix.values);

    let mut parameters = Vec::with_capacity(dataset.parameters.len());
    let mut columns = Vec::with_capacity(dataset.parameters.len());
    for (index, parameter) in dataset.parameters.iter().enumerate() {
        let stored = stored_values(dataset, index);
        let values = if let Some(position) = matrix_indices.iter().position(|i| *i == index) {
            compensated[position].clone()
        } else if parameter.name.eq_ignore_ascii_case("time") {
            // `$TIMESTEP` is carried over and still applies to stored time.
            stored.clone()
        } else {
            cytiq_fcs::linear_values(dataset, index)
        };
        let mut output = FcsWriteParameter::from_source(parameter);
        if values
            .iter()
            .zip(&stored)
            .any(|(value, raw)| value.to_bits() != raw.to_bits())
        {
            output.range = None;
            output.keywords.retain(|(suffix, _)| suffix != "G");
        }
        parameters.push(output);
        columns.push(values);
    }
    (parameters, columns)
}

fn output_keywords(
    path: &Path,
    dataset: &FcsDataset,
    config: &CompensateConfig,
    channels: &[String],
    parameters: &[FcsWriteParameter],
) -> Result<FcsWriteOptions, CompensateError> {
    let mut keywords = Vec::new();
    for (key, value) in cytiq_fcs::carried_keywords(&dataset.text) {
        if cytiq_fcs::is_spillover_keyword(&key) {
            keywords.push((SOURCE_SPILLOVER_KEYWORD.to_owned(), value));
        } else if key != MARGINS_KEYWORD {
            keywords.push((key, value));
        }
    }

    // Bounds survive only for channels whose values were written unchanged.
    let mut bounds =
        cytiq_fcs::dataset_margin_bounds(&dataset.text, &dataset.parameters).map_err(|source| {
            CompensateError::FcsRead {
                path: path.to_path_buf(),
                source,
            }
        })?;
    for parameter in parameters {
        if parameter.range.is_none() || channels.contains(&parameter.name) {
            bounds.remove(&parameter.name);
        }
    }
    let state = CompensationState {
        source: config
            .matrix
            .as_ref()
            .map_or_else(|| "fcs".to_owned(), |(path, _)| path.display().to_string()),
        channels: channels.to_vec(),
    };
    Ok(FcsWriteOptions::new()
        .with_text_keywords(keywords)
        .with_text_keyword(MARGINS_KEYWORD, cytiq_fcs::encode_margin_bounds(&bounds))
        .with_text_keyword(COMPENSATION_KEYWORD, state.to_keyword_value()))
}

#[allow(clippy::cast_possible_truncation)]
fn write_fcs(
    path: &Path,
    parameters: &[FcsWriteParameter],
    columns: &[Vec<f64>],
    options: &FcsWriteOptions,
) -> Result<(), CompensateError> {
    // Compensated values rarely fit f32 exactly, but f32 keeps far more
    // precision than the detectors deliver; doubles are needed only for
    // integer data or event identifiers above 2^24.
    let needs_f64 = parameters
        .iter()
        .zip(columns)
        .filter(|(parameter, _)| parameter.range.is_some())
        .any(|(_, column)| {
            column
                .iter()
                .any(|value| f64::from(*value as f32).to_bits() != value.to_bits())
        });
    debug!("writing compensated FCS to {}", path.display());
    let result = if needs_f64 {
        cytiq_fcs::write_fcs3_1_f64_with_options(path, parameters, columns, options)
    } else {
        let columns = columns
            .iter()
            .map(|column| column.iter().map(|value| *value as f32).collect())
            .collect::<Vec<Vec<f32>>>();
        cytiq_fcs::write_fcs3_1_f32_with_options(path, parameters, &columns, options)
    };
    result.map_err(|source| CompensateError::FcsWrite {
        path: path.to_path_buf(),
        source,
    })
}
