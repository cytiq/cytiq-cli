#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::similar_names,
    clippy::too_many_lines
)]

mod error;
mod export;
pub mod scales;

use scales::{FlowJoBiexp, Hyperlog, Logicle, ScaleParameterError};

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use cytiq_fcs::{
    ColumnData, FcsDataset, FcsReader, FcsWriteOptions, FcsWriteParameter, MARGINS_KEYWORD,
    ORIGINAL_ID_PARAMETER, Parameter, SOURCE_SPILLOVER_KEYWORD, TRANSFORM_KEYWORD,
};
use cytiq_publish::{Existing, Publication};
pub use error::TransformError;
use export::write_debug_artifacts;
use log::{debug, info};
use rayon::{ThreadPoolBuilder, prelude::*};
use serde::Serialize;

/// Upstream implementations this crate is validated against
/// (`parity/versions.txt`), recorded in provenance.
pub const UPSTREAM: &[(&str, &str)] = &[("flowCore", "2.22.1"), ("flowWorkspace", "4.22.1")];

const DEFAULT_SUFFIX: &str = ".transformed";
const DEFAULT_LOGICLE_T: f64 = 262_144.0;
const DEFAULT_LOGICLE_M: f64 = 4.5;
const DEFAULT_LOGICLE_W: f64 = 0.5;
const DEFAULT_LOGICLE_A: f64 = 0.0;
const DEFAULT_BIEXPONENTIAL_LENGTH: f64 = 256.0;
const DEFAULT_BIEXPONENTIAL_MAX_RANGE: f64 = 262_144.0;
const DEFAULT_BIEXPONENTIAL_POS: f64 = 4.418_539_922;
const DEFAULT_BIEXPONENTIAL_NEG: f64 = 0.0;
const DEFAULT_BIEXPONENTIAL_WIDTH: f64 = -100.0;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransformMethod {
    #[default]
    Arcsinh,
    Biexponential,
    Hyperlog,
    Logicle,
    None,
}

impl TransformMethod {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Arcsinh => "arcsinh",
            Self::Biexponential => "biexponential",
            Self::Hyperlog => "hyperlog",
            Self::Logicle => "logicle",
            Self::None => "none",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct LogicleParameters {
    pub t: f64,
    pub m: f64,
    pub w: f64,
    pub a: f64,
}

impl Default for LogicleParameters {
    fn default() -> Self {
        Self {
            t: DEFAULT_LOGICLE_T,
            m: DEFAULT_LOGICLE_M,
            w: DEFAULT_LOGICLE_W,
            a: DEFAULT_LOGICLE_A,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct BiexponentialParameters {
    pub length: f64,
    pub max_range: f64,
    pub pos: f64,
    pub neg: f64,
    pub width: f64,
}

impl Default for BiexponentialParameters {
    fn default() -> Self {
        Self {
            length: DEFAULT_BIEXPONENTIAL_LENGTH,
            max_range: DEFAULT_BIEXPONENTIAL_MAX_RANGE,
            pos: DEFAULT_BIEXPONENTIAL_POS,
            neg: DEFAULT_BIEXPONENTIAL_NEG,
            width: DEFAULT_BIEXPONENTIAL_WIDTH,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct TransformParameters {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cofactor: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logicle: Option<LogicleParameters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hyperlog: Option<LogicleParameters>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub biexponential: Option<BiexponentialParameters>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TransformFormat {
    #[default]
    Fcs,
    Csv,
    Parquet,
}

impl TransformFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fcs => "fcs",
            Self::Csv => "csv",
            Self::Parquet => "parquet",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct TransformConfig {
    pub inputs: Vec<PathBuf>,
    pub output: PathBuf,
    pub method: TransformMethod,
    pub cofactor: f64,
    pub logicle: LogicleParameters,
    pub biexponential: BiexponentialParameters,
    pub channels: Vec<String>,
    pub exclude_channels: Vec<String>,
    pub format: TransformFormat,
    pub suffix: String,
    pub overwrite: bool,
    pub dry_run: bool,
    pub metadata_json: Option<PathBuf>,
    pub debug_dir: Option<PathBuf>,
    pub threads: Option<usize>,
    /// Transform files that an earlier `cytiq transform` already transformed.
    pub allow_retransform: bool,
    /// Transform channels whose spillover matrix was not applied yet.
    pub allow_uncompensated: bool,
}

impl TransformConfig {
    pub fn new(inputs: Vec<PathBuf>, output: PathBuf) -> Self {
        Self {
            inputs,
            output,
            method: TransformMethod::Arcsinh,
            cofactor: 150.0,
            logicle: LogicleParameters::default(),
            biexponential: BiexponentialParameters::default(),
            channels: Vec::new(),
            exclude_channels: Vec::new(),
            format: TransformFormat::Fcs,
            suffix: DEFAULT_SUFFIX.to_owned(),
            overwrite: false,
            dry_run: false,
            metadata_json: None,
            debug_dir: None,
            threads: None,
            allow_retransform: false,
            allow_uncompensated: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TransformRunResult {
    pub dry_run: bool,
    pub metadata_path: Option<PathBuf>,
    pub files: Vec<TransformFileResult>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TransformFileResult {
    pub input_path: PathBuf,
    pub output_path: PathBuf,
    pub method: TransformMethod,
    pub parameters: TransformParameters,
    pub channels: Vec<String>,
    pub excluded_channels: Vec<String>,
    pub events: usize,
    pub metrics: Vec<TransformChannelMetrics>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TransformChannelMetrics {
    pub channel: String,
    pub events: usize,
    pub raw: TransformValueMetrics,
    pub transformed: TransformValueMetrics,
    pub raw_negative_count: usize,
    pub raw_zero_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct TransformValueMetrics {
    pub min: f64,
    pub max: f64,
    pub mean: f64,
    pub standard_deviation: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
struct TransformMetadataDocument {
    tool: String,
    version: String,
    method: TransformMethod,
    parameters: TransformParameters,
    files: Vec<TransformFileResult>,
}

#[derive(Clone, Debug, PartialEq)]
struct OutputPlan {
    input: PathBuf,
    output: PathBuf,
    same_file: bool,
}

#[derive(Clone, Debug)]
struct ResolvedSelection {
    channels: Vec<ResolvedChannel>,
    excluded: Vec<String>,
}

#[derive(Clone, Debug)]
struct ResolvedChannel {
    index: usize,
    name: String,
}

#[derive(Clone, Debug, PartialEq)]
struct TransformedData {
    columns: Vec<Vec<f64>>,
    metrics: Vec<TransformChannelMetrics>,
}

pub struct TransformRunner;

impl TransformRunner {
    pub fn run(config: &TransformConfig) -> Result<TransformRunResult, TransformError> {
        with_thread_pool(config.threads, || Self::run_inner(config))
    }

    /// The FCS files a run with `config` reads, with directories and globs
    /// resolved as [`Self::run`] does.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration is invalid or an input cannot
    /// be resolved.
    pub fn input_files(config: &TransformConfig) -> Result<Vec<PathBuf>, TransformError> {
        validate_config(config)?;
        resolve_inputs(&config.inputs)
    }

    fn run_inner(config: &TransformConfig) -> Result<TransformRunResult, TransformError> {
        validate_config(config)?;
        let inputs = resolve_inputs(&config.inputs)?;
        let plans = plan_outputs(&inputs, config)?;
        if !config.dry_run {
            prepare_metadata_output(config)?;
        }
        let metadata_path = config.metadata_json.clone();
        let file_results = plans
            .par_iter()
            .map(|plan| transform_one(plan, config))
            .collect::<Vec<_>>();
        let mut files = Vec::with_capacity(file_results.len());
        for result in file_results {
            files.push(result?);
        }

        let result = TransformRunResult {
            dry_run: config.dry_run,
            metadata_path,
            files,
        };
        if !config.dry_run {
            write_metadata_json(config, &result)?;
            if let Some(debug_dir) = config.debug_dir.as_deref() {
                write_debug_artifacts(&result, debug_dir)?;
            }
        }

        Ok(result)
    }
}

fn with_thread_pool<T: Send>(
    threads: Option<usize>,
    operation: impl FnOnce() -> Result<T, TransformError> + Send,
) -> Result<T, TransformError> {
    let Some(threads) = threads else {
        return operation();
    };

    debug!("using local rayon thread pool with {threads} threads");
    ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|source| TransformError::ThreadPoolBuild { threads, source })?
        .install(operation)
}

/// Parse a comma-separated list of channel names.
pub fn parse_channel_list(raw: &str) -> Result<Vec<String>, TransformError> {
    let channels = raw
        .split(',')
        .map(str::trim)
        .filter(|channel| !channel.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if channels.is_empty() {
        Err(TransformError::EmptyChannelList)
    } else {
        Ok(channels)
    }
}

fn validate_config(config: &TransformConfig) -> Result<(), TransformError> {
    if config.inputs.is_empty() {
        return Err(TransformError::EmptyInputs);
    }
    if config.format != TransformFormat::Fcs {
        return Err(TransformError::UnsupportedFormat {
            format: config.format.as_str(),
        });
    }
    validate_positive("cofactor", config.cofactor)?;
    validate_positive("logicle-t", config.logicle.t)?;
    validate_positive("logicle-m", config.logicle.m)?;
    validate_non_negative("logicle-w", config.logicle.w)?;
    // A may be negative down to -W; the scale constructors check its range.
    if !config.logicle.a.is_finite() {
        return Err(TransformError::InvalidFiniteParameter {
            parameter: "logicle-a",
        });
    }
    validate_positive("biexponential-length", config.biexponential.length)?;
    validate_positive("biexponential-max-range", config.biexponential.max_range)?;
    validate_positive("biexponential-pos", config.biexponential.pos)?;
    validate_non_negative("biexponential-neg", config.biexponential.neg)?;
    if !config.biexponential.width.is_finite() {
        return Err(TransformError::InvalidFiniteParameter {
            parameter: "biexponential-width",
        });
    }
    if config.threads == Some(0) {
        return Err(TransformError::InvalidPositiveParameter {
            parameter: "threads",
        });
    }
    TransformOperator::from_config(config)?;

    Ok(())
}

fn validate_positive(parameter: &'static str, value: f64) -> Result<(), TransformError> {
    if value.is_finite() && value > 0.0 {
        Ok(())
    } else {
        Err(TransformError::InvalidPositiveParameter { parameter })
    }
}

fn validate_non_negative(parameter: &'static str, value: f64) -> Result<(), TransformError> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(TransformError::InvalidNonNegativeParameter { parameter })
    }
}

fn resolve_inputs(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, TransformError> {
    let mut paths = Vec::new();
    for input in inputs {
        paths.extend(resolve_input(input)?);
    }
    if paths.is_empty() {
        return Err(TransformError::EmptyInputs);
    }

    paths.sort();
    let mut seen = HashSet::new();
    for path in &paths {
        let canonical =
            fs::canonicalize(path).map_err(|source| TransformError::InputCanonicalize {
                path: path.clone(),
                source,
            })?;
        if !seen.insert(canonical) {
            return Err(TransformError::DuplicateInput { path: path.clone() });
        }
    }

    Ok(paths)
}

fn resolve_input(input: &Path) -> Result<Vec<PathBuf>, TransformError> {
    let raw = input.to_string_lossy();
    if raw.contains('*') || raw.contains('?') {
        return resolve_glob(input, &raw);
    }
    if !input.exists() {
        return Err(TransformError::MissingInput {
            path: input.to_path_buf(),
        });
    }
    if input.is_dir() {
        return resolve_directory(input);
    }
    if input.is_file() && is_fcs_path(input) {
        return Ok(vec![input.to_path_buf()]);
    }
    Err(TransformError::NonFcsInput {
        path: input.to_path_buf(),
    })
}

fn resolve_glob(input: &Path, raw: &str) -> Result<Vec<PathBuf>, TransformError> {
    let parent = input.parent().filter(|path| !path.as_os_str().is_empty());
    let file_pattern = input
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| TransformError::UnsupportedGlob {
            pattern: raw.to_owned(),
        })?;
    let parent = parent.unwrap_or_else(|| Path::new("."));
    if parent.to_string_lossy().contains('*') || parent.to_string_lossy().contains('?') {
        return Err(TransformError::UnsupportedGlob {
            pattern: raw.to_owned(),
        });
    }

    let mut matches = fs::read_dir(parent)
        .map_err(|source| TransformError::DirectoryRead {
            path: parent.to_path_buf(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_fcs_path(path))
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| wildcard_match(file_pattern, name))
        })
        .collect::<Vec<_>>();
    matches.sort();
    if matches.is_empty() {
        return Err(TransformError::EmptyGlob {
            pattern: raw.to_owned(),
        });
    }

    Ok(matches)
}

fn resolve_directory(input: &Path) -> Result<Vec<PathBuf>, TransformError> {
    let mut paths = fs::read_dir(input)
        .map_err(|source| TransformError::DirectoryRead {
            path: input.to_path_buf(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_fcs_path(path))
        .collect::<Vec<_>>();
    paths.sort();
    if paths.is_empty() {
        return Err(TransformError::EmptyDirectory {
            path: input.to_path_buf(),
        });
    }

    Ok(paths)
}

fn plan_outputs(
    inputs: &[PathBuf],
    config: &TransformConfig,
) -> Result<Vec<OutputPlan>, TransformError> {
    let output = &config.output;
    let multiple_inputs = inputs.len() > 1;
    if multiple_inputs && output.is_file() {
        return Err(TransformError::MultipleInputsOutputFile {
            path: output.clone(),
        });
    }

    let mut plans = Vec::with_capacity(inputs.len());
    let mut seen_outputs = HashSet::new();
    for input in inputs {
        let output_path = if multiple_inputs || output.is_dir() {
            output.join(output_file_name(input, &config.suffix))
        } else {
            output.clone()
        };
        validate_output_path(input, &output_path, config)?;
        let output_key = comparable_path(&output_path);
        if !seen_outputs.insert(output_key) {
            return Err(TransformError::DuplicateOutput { path: output_path });
        }
        let same_file = paths_same_file(input, &output_path);
        plans.push(OutputPlan {
            input: input.clone(),
            output: output_path,
            same_file,
        });
    }

    Ok(plans)
}

fn validate_output_path(
    input: &Path,
    output: &Path,
    config: &TransformConfig,
) -> Result<(), TransformError> {
    if !is_fcs_path(output) {
        return Err(TransformError::NonFcsOutput {
            path: output.to_path_buf(),
        });
    }
    if paths_same_file(input, output) && !config.overwrite {
        return Err(TransformError::SameInputOutput {
            path: output.to_path_buf(),
        });
    }
    if output.exists() && !config.overwrite {
        return Err(TransformError::OutputExists {
            path: output.to_path_buf(),
        });
    }

    Ok(())
}

fn transform_one(
    plan: &OutputPlan,
    config: &TransformConfig,
) -> Result<TransformFileResult, TransformError> {
    let dataset = load_dataset(&plan.input)?;
    ensure_not_transformed(&plan.input, &dataset, config)?;
    let events = event_count(&dataset);
    let selection = resolve_channel_selection(&plan.input, &dataset, config)?;
    ensure_compensated(&plan.input, &dataset, &selection.channels, config)?;
    let mut result = TransformFileResult {
        input_path: plan.input.clone(),
        output_path: plan.output.clone(),
        method: config.method,
        parameters: method_parameters(config),
        channels: selection
            .channels
            .iter()
            .map(|channel| channel.name.clone())
            .collect(),
        excluded_channels: selection.excluded,
        events,
        metrics: Vec::new(),
    };

    info!(
        "transform plan: {} -> {} with {} channels",
        plan.input.display(),
        plan.output.display(),
        result.channels.len()
    );
    if config.dry_run {
        return Ok(result);
    }

    let transformed = transformed_columns(&dataset, &selection.channels, config, &plan.input)?;
    result.metrics = transformed.metrics;
    let operator = TransformOperator::from_config(config)?;
    let parameters = write_parameters(&dataset, &selection.channels);
    let metadata = fcs_metadata_json(config, &result)?;
    let acquisition = cytiq_fcs::acquisition_id(&plan.input, &dataset.text).map_err(|source| {
        TransformError::FcsRead {
            path: plan.input.clone(),
            source,
        }
    })?;
    let mut keywords = carried_keywords(&dataset);
    cytiq_fcs::set_acquisition(&mut keywords, &acquisition);
    let options = FcsWriteOptions::new()
        .with_text_keywords(keywords)
        .with_text_keyword(
            MARGINS_KEYWORD,
            transformed_margin_bounds(&plan.input, &dataset, &selection.channels, &operator)?,
        )
        .with_text_keyword(TRANSFORM_KEYWORD, metadata);
    write_fcs_output(
        plan,
        &parameters,
        &transformed.columns,
        &options,
        Existing::overwrite(config.overwrite),
    )?;
    Ok(result)
}

fn ensure_not_transformed(
    path: &Path,
    dataset: &FcsDataset,
    config: &TransformConfig,
) -> Result<(), TransformError> {
    if cytiq_fcs::normalization_state(&dataset.text)
        .map_err(|source| TransformError::FcsRead {
            path: path.to_path_buf(),
            source,
        })?
        .is_some()
    {
        return Err(TransformError::AlreadyTransformed {
            path: path.to_path_buf(),
            method: "cytonorm (cannot retransform normalized values)".to_owned(),
        });
    }
    let state =
        cytiq_fcs::transform_state(&dataset.text).map_err(|source| TransformError::FcsRead {
            path: path.to_path_buf(),
            source,
        })?;
    match state {
        Some(state) if !config.allow_retransform => Err(TransformError::AlreadyTransformed {
            path: path.to_path_buf(),
            method: state.method,
        }),
        _ => Ok(()),
    }
}

/// Compensation must come first: `cytiq compensate` refuses transformed
/// channels, so a channel transformed while the file's spillover matrix is
/// still pending can never be compensated correctly.
fn ensure_compensated(
    path: &Path,
    dataset: &FcsDataset,
    channels: &[ResolvedChannel],
    config: &TransformConfig,
) -> Result<(), TransformError> {
    if config.allow_uncompensated {
        return Ok(());
    }
    let spillover =
        cytiq_fcs::Spillover::from_dataset(dataset).map_err(|source| TransformError::FcsRead {
            path: path.to_path_buf(),
            source,
        })?;
    let Some(spillover) = spillover else {
        return Ok(());
    };
    let pending = spillover
        .channels()
        .iter()
        .filter_map(|channel| dataset.find_parameter(channel))
        .collect::<HashSet<_>>();
    match channels
        .iter()
        .find(|channel| pending.contains(&channel.index))
    {
        Some(channel) => Err(TransformError::UncompensatedInput {
            path: path.to_path_buf(),
            channel: channel.name.clone(),
        }),
        None => Ok(()),
    }
}

/// Source keywords that still describe the transformed values. The spillover
/// matrix does not: compensating transformed values is wrong, so it is kept
/// only for provenance under `CYTIQ_SOURCE_SPILL`.
fn carried_keywords(dataset: &FcsDataset) -> Vec<(String, String)> {
    let mut keywords = Vec::new();
    for (key, value) in cytiq_fcs::carried_keywords(&dataset.text) {
        if cytiq_fcs::is_spillover_keyword(&key) {
            keywords.push((SOURCE_SPILLOVER_KEYWORD.to_owned(), value));
        } else if key != MARGINS_KEYWORD && key != TRANSFORM_KEYWORD {
            keywords.push((key, value));
        }
    }
    keywords
}

/// Margin bounds in the space of the written values. Every transform method is
/// monotonically increasing, so a raw bound maps to the same cut on transformed
/// values; bounds are rounded to f32 like the values themselves.
fn transformed_margin_bounds(
    path: &Path,
    dataset: &FcsDataset,
    channels: &[ResolvedChannel],
    operator: &TransformOperator,
) -> Result<String, TransformError> {
    let bounds =
        cytiq_fcs::dataset_margin_bounds(&dataset.text, &dataset.parameters).map_err(|source| {
            TransformError::FcsRead {
                path: path.to_path_buf(),
                source,
            }
        })?;
    let bounds = bounds
        .into_iter()
        .filter_map(|(name, bounds)| {
            let selected = channels.iter().any(|channel| channel.name == name);
            let mapped = bounds.map(|value| {
                let value = if selected {
                    operator.apply(value)
                } else {
                    value
                };
                f64::from(value as f32)
            });
            let finite = mapped.lower.is_finite() && mapped.upper.is_none_or(f64::is_finite);
            finite.then_some((name, mapped))
        })
        .collect();
    Ok(cytiq_fcs::encode_margin_bounds(&bounds))
}

fn load_dataset(path: &Path) -> Result<FcsDataset, TransformError> {
    debug!("loading FCS file {}", path.display());
    let dataset = FcsReader::open(path)
        .and_then(FcsReader::read)
        .and_then(|dataset| cytiq_fcs::ensure_linear_scale(&dataset).map(|()| dataset))
        .map_err(|source| TransformError::FcsRead {
            path: path.to_path_buf(),
            source,
        })?;
    if event_count(&dataset) == 0 {
        return Err(TransformError::EmptyFcs {
            path: path.to_path_buf(),
        });
    }

    Ok(dataset)
}

fn resolve_channel_selection(
    path: &Path,
    dataset: &FcsDataset,
    config: &TransformConfig,
) -> Result<ResolvedSelection, TransformError> {
    let mut channels = if config.channels.is_empty() {
        default_transform_channels(dataset)
    } else {
        resolve_requested_channels(path, dataset, &config.channels)?
    };
    if let Some(channel) = channels
        .iter()
        .find(|channel| channel.name == ORIGINAL_ID_PARAMETER)
    {
        return Err(TransformError::ReservedChannel {
            channel: channel.name.clone(),
        });
    }
    let excluded = resolve_requested_channels(path, dataset, &config.exclude_channels)?;
    let excluded_indexes = excluded
        .iter()
        .map(|channel| channel.index)
        .collect::<HashSet<_>>();
    channels.retain(|channel| !excluded_indexes.contains(&channel.index));
    ensure_unique_channels(&channels)?;
    if channels.is_empty() {
        return Err(TransformError::NoTransformableChannels {
            path: path.to_path_buf(),
        });
    }

    Ok(ResolvedSelection {
        channels,
        excluded: excluded.into_iter().map(|channel| channel.name).collect(),
    })
}

fn default_transform_channels(dataset: &FcsDataset) -> Vec<ResolvedChannel> {
    dataset
        .parameters
        .iter()
        .enumerate()
        .filter(|(_, parameter)| {
            !is_acquisition_channel(parameter) && parameter.name != ORIGINAL_ID_PARAMETER
        })
        .map(|(index, parameter)| ResolvedChannel {
            index,
            name: parameter.name.clone(),
        })
        .collect()
}

fn resolve_requested_channels(
    path: &Path,
    dataset: &FcsDataset,
    requested_channels: &[String],
) -> Result<Vec<ResolvedChannel>, TransformError> {
    let mut channels = Vec::with_capacity(requested_channels.len());
    for channel in requested_channels {
        let Some(index) = find_parameter_index(&dataset.parameters, channel) else {
            return Err(TransformError::MissingChannel {
                path: path.to_path_buf(),
                channel: channel.clone(),
                available_channels: dataset
                    .parameters
                    .iter()
                    .map(display_parameter_identifier)
                    .collect(),
            });
        };
        channels.push(ResolvedChannel {
            index,
            name: dataset.parameters[index].name.clone(),
        });
    }
    ensure_unique_channels(&channels)?;
    Ok(channels)
}

fn ensure_unique_channels(channels: &[ResolvedChannel]) -> Result<(), TransformError> {
    let mut seen = HashSet::new();
    for channel in channels {
        if !seen.insert(channel.index) {
            return Err(TransformError::DuplicateChannel {
                channel: channel.name.clone(),
            });
        }
    }

    Ok(())
}

fn transformed_columns(
    dataset: &FcsDataset,
    channels: &[ResolvedChannel],
    config: &TransformConfig,
    input: &Path,
) -> Result<TransformedData, TransformError> {
    let selected = channels
        .iter()
        .map(|channel| (channel.index, channel.name.clone()))
        .collect::<std::collections::HashMap<_, _>>();
    let operator = TransformOperator::from_config(config)?;
    let column_results = dataset
        .data
        .columns
        .par_iter()
        .enumerate()
        .map(|(index, column)| {
            let selected_channel = selected.get(&index).map(String::as_str);
            transform_column(
                column,
                selected_channel,
                &operator,
                input,
                &dataset.parameters[index].name,
            )
        })
        .collect::<Vec<_>>();
    let mut columns = Vec::with_capacity(column_results.len());
    let mut metrics = Vec::new();
    for result in column_results {
        let transformed = result?;
        columns.push(transformed.values);
        if let Some(channel_metrics) = transformed.metrics {
            metrics.push(channel_metrics);
        }
    }

    Ok(TransformedData { columns, metrics })
}

#[derive(Clone, Debug, PartialEq)]
struct TransformedColumn {
    values: Vec<f64>,
    metrics: Option<TransformChannelMetrics>,
}

fn transform_column(
    column: &ColumnData,
    selected_channel: Option<&str>,
    operator: &TransformOperator,
    input: &Path,
    channel: &str,
) -> Result<TransformedColumn, TransformError> {
    let values = match column {
        ColumnData::UInt(values) => values
            .par_iter()
            .map(|value| {
                transform_raw_value(
                    *value as f64,
                    selected_channel.is_some(),
                    operator,
                    input,
                    channel,
                )
            })
            .collect::<Vec<_>>(),
        ColumnData::F32(values) => values
            .par_iter()
            .map(|value| {
                transform_raw_value(
                    f64::from(*value),
                    selected_channel.is_some(),
                    operator,
                    input,
                    channel,
                )
            })
            .collect::<Vec<_>>(),
        ColumnData::F64(values) => values
            .par_iter()
            .map(|value| {
                transform_raw_value(*value, selected_channel.is_some(), operator, input, channel)
            })
            .collect::<Vec<_>>(),
    };

    let mut output = Vec::with_capacity(values.len());
    let mut raw_values = Vec::new();
    let mut transformed_values = Vec::new();
    for value in values {
        let value = value?;
        if selected_channel.is_some() {
            raw_values.push(f64::from(value.raw));
            transformed_values.push(value.transformed);
        }
        output.push(value.transformed);
    }

    let metrics = selected_channel
        .map(|channel| build_channel_metrics(channel, &raw_values, &transformed_values));

    Ok(TransformedColumn {
        values: output,
        metrics,
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct TransformedValue {
    raw: f32,
    /// Transformed values are rounded to f32; values of unselected channels
    /// (such as `Original_ID`) are kept exact.
    transformed: f64,
}

fn transform_raw_value(
    value: f64,
    selected: bool,
    operator: &TransformOperator,
    input: &Path,
    channel: &str,
) -> Result<TransformedValue, TransformError> {
    if selected && !value.is_finite() {
        return Err(TransformError::NonFiniteChannel {
            path: input.to_path_buf(),
            channel: channel.to_owned(),
        });
    }
    let output = if selected {
        operator.apply(value)
    } else {
        value
    };
    if !output.is_finite() {
        return Err(TransformError::NonFiniteOutputValues {
            path: input.to_path_buf(),
        });
    }

    Ok(TransformedValue {
        raw: value as f32,
        transformed: if selected {
            f64::from(output as f32)
        } else {
            output
        },
    })
}

fn build_channel_metrics(
    channel: &str,
    raw_values: &[f64],
    transformed_values: &[f64],
) -> TransformChannelMetrics {
    TransformChannelMetrics {
        channel: channel.to_owned(),
        events: raw_values.len(),
        raw: summarize_values(raw_values),
        transformed: summarize_values(transformed_values),
        raw_negative_count: raw_values.iter().filter(|value| **value < 0.0).count(),
        raw_zero_count: raw_values
            .iter()
            .filter(|value| value.abs() <= f64::EPSILON)
            .count(),
    }
}

fn summarize_values(values: &[f64]) -> TransformValueMetrics {
    if values.is_empty() {
        return TransformValueMetrics {
            min: 0.0,
            max: 0.0,
            mean: 0.0,
            standard_deviation: 0.0,
        };
    }

    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let standard_deviation = if values.len() < 2 {
        0.0
    } else {
        let variance = values
            .iter()
            .map(|value| {
                let delta = value - mean;
                delta * delta
            })
            .sum::<f64>()
            / (values.len() - 1) as f64;
        variance.sqrt()
    };

    TransformValueMetrics {
        min,
        max,
        mean,
        standard_deviation,
    }
}

/// Per-value transform. Logicle follows `flowCore::logicleTransform` (output
/// on `[0, M]`), hyperlog follows `flowCore::hyperlogtGml2` (output on
/// `[0, 1]`), and biexponential follows `flowWorkspace::flowjo_biexp` (output
/// on `[0, length]`).
#[derive(Clone, Debug)]
enum TransformOperator {
    Arcsinh { cofactor: f64 },
    Biexponential(FlowJoBiexp),
    Hyperlog(Hyperlog),
    Logicle { scale: Logicle, decades: f64 },
    None,
}

impl TransformOperator {
    fn from_config(config: &TransformConfig) -> Result<Self, TransformError> {
        let invalid = |method: TransformMethod| {
            move |error: ScaleParameterError| TransformError::InvalidScaleParameters {
                method: method.as_str(),
                message: error.to_string(),
            }
        };
        let LogicleParameters { t, m, w, a } = config.logicle;
        Ok(match config.method {
            TransformMethod::Arcsinh => Self::Arcsinh {
                cofactor: config.cofactor,
            },
            TransformMethod::Biexponential => {
                let parameters = config.biexponential;
                Self::Biexponential(
                    FlowJoBiexp::new(
                        // validated as finite and positive in `validate_config`
                        parameters.length.round().max(1.0) as usize,
                        parameters.max_range,
                        parameters.pos,
                        parameters.neg,
                        parameters.width,
                    )
                    .map_err(invalid(TransformMethod::Biexponential))?,
                )
            }
            TransformMethod::Hyperlog => Self::Hyperlog(
                Hyperlog::new(t, w, m, a).map_err(invalid(TransformMethod::Hyperlog))?,
            ),
            TransformMethod::Logicle => Self::Logicle {
                scale: Logicle::new(t, w, m, a).map_err(invalid(TransformMethod::Logicle))?,
                decades: m,
            },
            TransformMethod::None => Self::None,
        })
    }

    fn apply(&self, value: f64) -> f64 {
        match self {
            Self::Arcsinh { cofactor } => (value / cofactor).asinh(),
            Self::Biexponential(transform) => transform.scale(value),
            Self::Hyperlog(transform) => transform.scale(value),
            Self::Logicle { scale, decades } => decades * scale.scale(value),
            Self::None => value,
        }
    }
}

/// Stage the output beside its target and rename it into place, so neither a
/// failed write nor a same-file transform ever leaves a partial FCS behind.
fn write_fcs_output(
    plan: &OutputPlan,
    parameters: &[FcsWriteParameter],
    columns: &[Vec<f64>],
    options: &FcsWriteOptions,
    existing: Existing,
) -> Result<(), TransformError> {
    let mut publication = Publication::new(existing);
    publication.stage_path(&plan.output, |path| {
        cytiq_fcs::write_fcs3_1_exact_with_options(path, parameters, columns, options).map_err(
            |source| TransformError::FcsWrite {
                path: plan.output.clone(),
                source,
            },
        )
    })?;
    Ok(publication.commit()?)
}

fn prepare_metadata_output(config: &TransformConfig) -> Result<(), TransformError> {
    let Some(path) = config.metadata_json.as_deref() else {
        return Ok(());
    };
    if path.exists() && !config.overwrite {
        return Err(TransformError::MetadataExists {
            path: path.to_path_buf(),
        });
    }
    if let Some(parent) = path.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent).map_err(|source| TransformError::MetadataDirectoryCreate {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    Ok(())
}

fn write_metadata_json(
    config: &TransformConfig,
    result: &TransformRunResult,
) -> Result<(), TransformError> {
    let Some(path) = config.metadata_json.as_deref() else {
        return Ok(());
    };

    let document = TransformMetadataDocument {
        tool: "cytiq transform".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        method: config.method,
        parameters: method_parameters(config),
        files: result.files.clone(),
    };
    let bytes = serde_json::to_vec_pretty(&document).map_err(|source| {
        TransformError::MetadataSerialize {
            path: path.to_path_buf(),
            source,
        }
    })?;
    cytiq_publish::write_file(path, Existing::overwrite(config.overwrite), |writer| {
        writer
            .write_all(&bytes)
            .map_err(|source| TransformError::MetadataWrite {
                path: path.to_path_buf(),
                source,
            })
    })
}

fn fcs_metadata_json(
    config: &TransformConfig,
    result: &TransformFileResult,
) -> Result<String, TransformError> {
    let document = TransformMetadataDocument {
        tool: "cytiq transform".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        method: config.method,
        parameters: method_parameters(config),
        files: vec![result.clone()],
    };
    serde_json::to_string(&document).map_err(|source| TransformError::MetadataSerialize {
        path: result.output_path.clone(),
        source,
    })
}

/// Source parameters with their keywords. Transformed channels get a `$PnR`
/// computed from the transformed values; the others keep the original range.
fn write_parameters(dataset: &FcsDataset, channels: &[ResolvedChannel]) -> Vec<FcsWriteParameter> {
    dataset
        .parameters
        .iter()
        .enumerate()
        .map(|(index, parameter)| {
            let mut parameter = FcsWriteParameter::from_source(parameter);
            if channels.iter().any(|channel| channel.index == index) {
                parameter.range = None;
            }
            parameter
        })
        .collect()
}

fn is_acquisition_channel(parameter: &Parameter) -> bool {
    let name = normalize_channel_name(&parameter.name);
    matches!(
        name.as_str(),
        "TIME"
            | "EVENT_LENGTH"
            | "EVENT-LENGTH"
            | "EVENTLENGTH"
            | "CELL_LENGTH"
            | "CELL-LENGTH"
            | "CELLLENGTH"
            | "FSC-A"
            | "FSC-H"
            | "FSC-W"
            | "SSC-A"
            | "SSC-H"
            | "SSC-W"
    )
}

fn normalize_channel_name(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

fn find_parameter_index(parameters: &[Parameter], requested: &str) -> Option<usize> {
    let requested = requested.trim();
    parameters.iter().position(|parameter| {
        parameter.name == requested
            || parameter.label.as_deref() == Some(requested)
            || display_parameter_identifier(parameter) == requested
    })
}

fn display_parameter_identifier(parameter: &Parameter) -> String {
    match parameter.label.as_deref() {
        Some(label) if label != parameter.name => format!("{} [{}]", parameter.name, label),
        _ => parameter.name.clone(),
    }
}

fn output_file_name(input: &Path, suffix: &str) -> String {
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    format!("{stem}{suffix}.fcs")
}

fn is_fcs_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fcs"))
}

fn paths_same_file(first: &Path, second: &Path) -> bool {
    let Ok(first) = fs::canonicalize(first) else {
        return false;
    };
    let Ok(second) = fs::canonicalize(second) else {
        return false;
    };
    first == second
}

fn comparable_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_or_else(|_| path.to_path_buf(), |directory| directory.join(path))
    }
}

fn method_parameters(config: &TransformConfig) -> TransformParameters {
    match config.method {
        TransformMethod::Arcsinh => TransformParameters {
            cofactor: Some(config.cofactor),
            ..TransformParameters::default()
        },
        TransformMethod::Biexponential => TransformParameters {
            biexponential: Some(config.biexponential),
            ..TransformParameters::default()
        },
        TransformMethod::Hyperlog => TransformParameters {
            hyperlog: Some(config.logicle),
            ..TransformParameters::default()
        },
        TransformMethod::Logicle => TransformParameters {
            logicle: Some(config.logicle),
            ..TransformParameters::default()
        },
        TransformMethod::None => TransformParameters::default(),
    }
}

fn event_count(dataset: &FcsDataset) -> usize {
    dataset.data.columns.first().map_or(0, ColumnData::len)
}

fn wildcard_match(pattern: &str, text: &str) -> bool {
    wildcard_match_inner(pattern.as_bytes(), text.as_bytes())
}

fn wildcard_match_inner(pattern: &[u8], text: &[u8]) -> bool {
    let mut pattern_index = 0;
    let mut text_index = 0;
    let mut star_index = None;
    let mut star_text_index = 0;

    while text_index < text.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == text[text_index])
        {
            pattern_index += 1;
            text_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            pattern_index += 1;
            star_text_index = text_index;
        } else if let Some(star) = star_index {
            pattern_index = star + 1;
            star_text_index += 1;
            text_index = star_text_index;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::{TransformMethod, wildcard_match};

    #[test]
    fn arcsinh_formula_matches_direct_calculation() {
        let cofactor = 150.0_f64;
        for value in [-100.0, 0.0, 10.0, 10_000.0] {
            let actual = (value / cofactor).asinh();
            let expected = super::TransformOperator::from_config(&super::TransformConfig {
                inputs: Vec::new(),
                output: "out.fcs".into(),
                method: TransformMethod::Arcsinh,
                cofactor,
                logicle: super::LogicleParameters::default(),
                biexponential: super::BiexponentialParameters::default(),
                channels: Vec::new(),
                exclude_channels: Vec::new(),
                format: super::TransformFormat::Fcs,
                suffix: ".transformed".to_owned(),
                overwrite: false,
                dry_run: false,
                metadata_json: None,
                debug_dir: None,
                threads: None,
                allow_retransform: false,
                allow_uncompensated: false,
            })
            .expect("valid parameters")
            .apply(value);
            assert!((actual - expected).abs() <= f64::EPSILON);
        }
    }

    #[test]
    fn log_like_methods_return_finite_values_for_negative_zero_and_large_inputs() {
        let config = super::TransformConfig::new(Vec::new(), "out.fcs".into());
        for method in [
            TransformMethod::Biexponential,
            TransformMethod::Hyperlog,
            TransformMethod::Logicle,
        ] {
            let mut config = config.clone();
            config.method = method;
            let operator =
                super::TransformOperator::from_config(&config).expect("valid parameters");
            for value in [-1_000.0, -1.0, 0.0, 1.0, 1_000_000.0] {
                assert!(operator.apply(value).is_finite());
            }
        }
    }

    #[test]
    fn wildcard_matching_supports_simple_file_globs() {
        assert!(wildcard_match("*.fcs", "sample.fcs"));
        assert!(wildcard_match("sample-?.fcs", "sample-1.fcs"));
        assert!(!wildcard_match("sample-?.fcs", "sample-10.fcs"));
    }
}
