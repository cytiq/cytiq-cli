#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    clippy::module_name_repetitions,
    clippy::similar_names,
    clippy::too_many_lines
)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use cytiq_fcs::{ColumnData, FcsDataset, FcsReader, Parameter};
use log::{debug, info};
use nalgebra::DMatrix;
use rayon::{ThreadPoolBuilder, prelude::*};
use serde::Serialize;

use crate::error::CytoScanError;
use crate::export::write_outputs;

pub const DEFAULT_RANDOM_SEED: u64 = 42;
pub const DEFAULT_EVENTS_PER_FILE: usize = 1_000;
pub const DEFAULT_AGGREGATE_EVENTS: usize = 10_000;
pub const DEFAULT_QUANTILE_STEP: f64 = 0.1;
pub const DEFAULT_OUTLIER_THRESHOLD: f64 = 0.5;
pub const DEFAULT_NOVELTY_QUANTILE: f64 = 0.05;
pub const DEFAULT_ISOLATION_TREES: usize = 1_000;
pub const DEFAULT_FINGERPRINT_RECURSIONS: usize = 4;
pub const DEFAULT_ARCSINH_COFACTOR: f64 = 150.0;

const EQUALITY_EPSILON: f64 = 1.0e-12;
const VARIANCE_FLOOR: f64 = 1.0e-6;
const MAX_GMM_COMPONENTS: usize = 4;
const GMM_ITERATIONS: usize = 100;
const GMM_TOLERANCE: f64 = 1.0e-7;
const EULER_MASCHERONI: f64 = 0.577_215_664_901_532_9;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FeatureMethod {
    Summary,
    #[default]
    Quantiles,
    Emd,
    Binning,
    Fingerprint,
}

impl FeatureMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Quantiles => "quantiles",
            Self::Emd => "emd",
            Self::Binning => "binning",
            Self::Fingerprint => "fingerprint",
        }
    }

    const fn uses_aggregate(self) -> bool {
        matches!(self, Self::Emd | Self::Binning | Self::Fingerprint)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagMethod {
    #[default]
    Outlier,
    Novelty,
    Both,
}

impl FlagMethod {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Outlier => "outlier",
            Self::Novelty => "novelty",
            Self::Both => "both",
        }
    }

    const fn includes_outlier(self) -> bool {
        matches!(self, Self::Outlier | Self::Both)
    }

    const fn includes_novelty(self) -> bool {
        matches!(self, Self::Novelty | Self::Both)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CytoScanConfig {
    pub inputs: Vec<PathBuf>,
    pub reference_inputs: Vec<PathBuf>,
    pub output_dir: PathBuf,
    pub channels: Vec<String>,
    pub feature_method: FeatureMethod,
    pub flag_method: FlagMethod,
    pub debug_dir: Option<PathBuf>,
    pub threads: Option<usize>,
}

impl CytoScanConfig {
    #[must_use]
    pub fn new(inputs: Vec<PathBuf>, output_dir: PathBuf, channels: Vec<String>) -> Self {
        Self {
            inputs,
            reference_inputs: Vec::new(),
            output_dir,
            channels,
            feature_method: FeatureMethod::Quantiles,
            flag_method: FlagMethod::Outlier,
            debug_dir: None,
            threads: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CytoScanResult {
    pub parameters: CytoScanParameters,
    pub test_features: CytoScanFeatureMatrix,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_features: Option<CytoScanFeatureMatrix>,
    pub summary_rows: Vec<CytoScanSummaryRow>,
    pub diagnostics: CytoScanDiagnostics,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CytoScanParameters {
    pub tool: String,
    pub version: String,
    pub feature_method: FeatureMethod,
    pub flag_method: FlagMethod,
    pub requested_channels: Vec<String>,
    pub resolved_test_inputs: Vec<PathBuf>,
    pub resolved_reference_inputs: Vec<PathBuf>,
    pub output_dir: PathBuf,
    pub debug_dir: Option<PathBuf>,
    pub threads: Option<usize>,
    pub constants: CytoScanConstants,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct CytoScanConstants {
    pub random_seed: u64,
    pub events_per_file: usize,
    pub aggregate_events: usize,
    pub quantile_step: f64,
    pub outlier_threshold: f64,
    pub novelty_quantile: f64,
    pub isolation_trees: usize,
    pub fingerprint_recursions: usize,
    pub arcsinh_cofactor: f64,
}

impl Default for CytoScanConstants {
    fn default() -> Self {
        Self {
            random_seed: DEFAULT_RANDOM_SEED,
            events_per_file: DEFAULT_EVENTS_PER_FILE,
            aggregate_events: DEFAULT_AGGREGATE_EVENTS,
            quantile_step: DEFAULT_QUANTILE_STEP,
            outlier_threshold: DEFAULT_OUTLIER_THRESHOLD,
            novelty_quantile: DEFAULT_NOVELTY_QUANTILE,
            isolation_trees: DEFAULT_ISOLATION_TREES,
            fingerprint_recursions: DEFAULT_FINGERPRINT_RECURSIONS,
            arcsinh_cofactor: DEFAULT_ARCSINH_COFACTOR,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CytoScanFeatureMatrix {
    pub feature_names: Vec<String>,
    pub rows: Vec<CytoScanFeatureRow>,
}

impl CytoScanFeatureMatrix {
    #[must_use]
    pub fn values(&self) -> Vec<Vec<f64>> {
        self.rows.iter().map(|row| row.values.clone()).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CytoScanFeatureRow {
    pub path: PathBuf,
    pub filename: String,
    pub values: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CytoScanSummaryRow {
    pub filename: String,
    pub path: PathBuf,
    pub flag_method: FlagMethod,
    pub feature_method: FeatureMethod,
    pub is_outlier: Option<bool>,
    pub outlier_score: Option<f64>,
    pub is_novelty: Option<bool>,
    pub novelty_log_likelihood: Option<f64>,
    pub channels: String,
    pub warnings: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CytoScanDiagnostics {
    pub aggregate_source: String,
    pub aggregate_files: Vec<PathBuf>,
    pub aggregate_events: usize,
    pub feature_count: usize,
    pub outlier_tree_count: Option<usize>,
    pub novelty_components: Option<usize>,
    pub novelty_threshold: Option<f64>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
struct LoadedFile {
    path: PathBuf,
    filename: String,
    channel_names: Vec<String>,
    matrix: Vec<Vec<f64>>,
    event_count: usize,
    warnings: Vec<String>,
}

#[derive(Clone, Debug)]
struct ResolvedChannel {
    requested_name: String,
    parameter_name: String,
    display_name: String,
    parameter_index: usize,
    max_range: Option<f64>,
}

#[derive(Clone, Debug)]
struct SpilloverMatrix {
    channels: Vec<String>,
    compensation: DMatrix<f64>,
}

#[derive(Clone, Debug)]
enum FeatureModel {
    None,
    Emd { aggregate_columns: Vec<Vec<f64>> },
    Binning { boundaries: Vec<Vec<f64>> },
    Fingerprint(FingerprintModel),
}

#[derive(Clone, Debug, PartialEq)]
struct FingerprintModel {
    splits: Vec<FingerprintSplit>,
    bins: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct FingerprintSplit {
    channel_index: usize,
    threshold: f64,
}

#[derive(Clone, Debug)]
struct NoveltyModel {
    gmm: DiagonalGmm,
    scale: FeatureScale,
    threshold: f64,
}

#[derive(Clone, Debug)]
struct FeatureScale {
    means: Vec<f64>,
    standard_deviations: Vec<f64>,
}

#[derive(Clone, Debug)]
struct DiagonalGmm {
    weights: Vec<f64>,
    means: Vec<Vec<f64>>,
    variances: Vec<Vec<f64>>,
    bic: f64,
}

#[derive(Clone, Debug)]
enum IsolationNode {
    Leaf {
        size: usize,
    },
    Split {
        feature_index: usize,
        threshold: f64,
        left: Box<IsolationNode>,
        right: Box<IsolationNode>,
    },
}

#[derive(Clone, Debug)]
struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    const fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.state
    }

    fn next_index(&mut self, upper_bound: usize) -> usize {
        (self.next_u64() % upper_bound as u64) as usize
    }

    fn next_f64(&mut self) -> f64 {
        let bits = self.next_u64() >> 11;
        bits as f64 / ((1_u64 << 53) as f64)
    }
}

pub struct CytoScanRunner;

impl CytoScanRunner {
    /// Run the CytoScan-style anomaly detection pipeline and write outputs.
    ///
    /// # Errors
    ///
    /// Returns an error when inputs cannot be resolved/read, selected channels
    /// cannot be processed, model fitting fails, or output files cannot be
    /// written.
    pub fn run(config: &CytoScanConfig) -> Result<CytoScanResult, CytoScanError> {
        with_thread_pool(config.threads, || Self::run_inner(config))
    }

    fn run_inner(config: &CytoScanConfig) -> Result<CytoScanResult, CytoScanError> {
        validate_config(config)?;
        let test_paths = resolve_inputs(&config.inputs)?;
        let reference_paths = if config.flag_method.includes_novelty() {
            let paths = resolve_inputs(&config.reference_inputs)?;
            if paths.is_empty() {
                return Err(CytoScanError::EmptyReference);
            }
            paths
        } else {
            Vec::new()
        };

        info!(
            "cytoscan pipeline: loading {} test file(s), {} reference file(s)",
            test_paths.len(),
            reference_paths.len()
        );
        let test_files = load_files(&test_paths, &config.channels)?;
        let reference_files = load_files(&reference_paths, &config.channels)?;
        let aggregate_files = if config.flag_method.includes_novelty() {
            &reference_files
        } else {
            &test_files
        };
        let aggregate_source = if config.flag_method.includes_novelty() {
            "reference"
        } else {
            "test"
        };
        let feature_model = build_feature_model(config.feature_method, aggregate_files)?;
        let test_features =
            build_feature_matrix(&test_files, config.feature_method, &feature_model)?;
        let reference_features = if config.flag_method.includes_novelty() {
            Some(build_feature_matrix(
                &reference_files,
                config.feature_method,
                &feature_model,
            )?)
        } else {
            None
        };
        let outlier_scores = if config.flag_method.includes_outlier() {
            Some(score_outliers(&test_features.values()))
        } else {
            None
        };
        let novelty = if config.flag_method.includes_novelty() {
            let reference_features = reference_features
                .as_ref()
                .expect("reference features should exist for novelty");
            ensure_feature_shape(reference_features, &test_features)?;
            let model = fit_novelty_model(&reference_features.values())?;
            let test_log_likelihoods = log_likelihoods(&model, &test_features.values());
            Some((model, test_log_likelihoods))
        } else {
            None
        };

        let warnings = collect_warnings(&test_files, &reference_files);
        let summary_rows = build_summary_rows(
            config,
            &test_files,
            &test_features,
            outlier_scores.as_deref(),
            novelty.as_ref().map(|(_, values)| values.as_slice()),
            novelty.as_ref().map(|(model, _)| model.threshold),
        );
        let diagnostics = CytoScanDiagnostics {
            aggregate_source: aggregate_source.to_owned(),
            aggregate_files: aggregate_files
                .iter()
                .map(|file| file.path.clone())
                .collect(),
            aggregate_events: aggregate_event_count(aggregate_files),
            feature_count: test_features.feature_names.len(),
            outlier_tree_count: config
                .flag_method
                .includes_outlier()
                .then_some(DEFAULT_ISOLATION_TREES),
            novelty_components: novelty.as_ref().map(|(model, _)| model.gmm.weights.len()),
            novelty_threshold: novelty.as_ref().map(|(model, _)| model.threshold),
            warnings,
        };
        let result = CytoScanResult {
            parameters: build_parameters(config, &test_paths, &reference_paths),
            test_features,
            reference_features,
            summary_rows,
            diagnostics,
        };
        write_outputs(config, &result)?;
        info!(
            "cytoscan pipeline: wrote outputs for {} test file(s) to {}",
            result.summary_rows.len(),
            config.output_dir.display()
        );

        Ok(result)
    }
}

/// Parse a comma-separated list of channel names.
///
/// # Errors
///
/// Returns an error when the list is empty after trimming.
pub fn parse_channel_list(raw: &str) -> Result<Vec<String>, String> {
    let channels = raw
        .split(',')
        .map(str::trim)
        .filter(|channel| !channel.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if channels.is_empty() {
        Err("channel list must contain at least one channel".to_owned())
    } else {
        Ok(channels)
    }
}

fn validate_config(config: &CytoScanConfig) -> Result<(), CytoScanError> {
    if config.inputs.is_empty() {
        return Err(CytoScanError::EmptyInputs);
    }
    if config.channels.is_empty() {
        return Err(CytoScanError::EmptyChannels);
    }
    if config.flag_method.includes_novelty() && config.reference_inputs.is_empty() {
        return Err(CytoScanError::MissingReference {
            flag_method: config.flag_method.as_str(),
        });
    }
    if let Some(0) = config.threads {
        return Err(CytoScanError::InvalidThreads);
    }

    Ok(())
}

fn with_thread_pool<T: Send>(
    threads: Option<usize>,
    operation: impl FnOnce() -> Result<T, CytoScanError> + Send,
) -> Result<T, CytoScanError> {
    let Some(threads) = threads else {
        return operation();
    };

    debug!("using local rayon thread pool with {threads} threads");
    ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|source| CytoScanError::ThreadPoolBuild { threads, source })?
        .install(operation)
}

fn build_parameters(
    config: &CytoScanConfig,
    test_paths: &[PathBuf],
    reference_paths: &[PathBuf],
) -> CytoScanParameters {
    CytoScanParameters {
        tool: "cytiq-cytoscan".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        feature_method: config.feature_method,
        flag_method: config.flag_method,
        requested_channels: config.channels.clone(),
        resolved_test_inputs: test_paths.to_vec(),
        resolved_reference_inputs: reference_paths.to_vec(),
        output_dir: config.output_dir.clone(),
        debug_dir: config.debug_dir.clone(),
        threads: config.threads,
        constants: CytoScanConstants::default(),
    }
}

fn resolve_inputs(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, CytoScanError> {
    let mut paths = Vec::new();
    for input in inputs {
        paths.extend(resolve_input(input)?);
    }
    if paths.is_empty() {
        return Err(CytoScanError::EmptyInputs);
    }

    paths.sort();
    let mut seen = HashSet::new();
    for path in &paths {
        let canonical =
            fs::canonicalize(path).map_err(|source| CytoScanError::InputCanonicalize {
                path: path.clone(),
                source,
            })?;
        if !seen.insert(canonical) {
            return Err(CytoScanError::DuplicateInput { path: path.clone() });
        }
    }

    Ok(paths)
}

fn resolve_input(input: &Path) -> Result<Vec<PathBuf>, CytoScanError> {
    let raw = input.to_string_lossy();
    if raw.contains('*') || raw.contains('?') {
        return resolve_glob(input, &raw);
    }
    if !input.exists() {
        return Err(CytoScanError::MissingInput {
            path: input.to_path_buf(),
        });
    }
    if input.is_dir() {
        return resolve_directory(input);
    }
    if input.is_file() && is_fcs_path(input) {
        return Ok(vec![input.to_path_buf()]);
    }
    Err(CytoScanError::NonFcsInput {
        path: input.to_path_buf(),
    })
}

fn resolve_glob(input: &Path, raw: &str) -> Result<Vec<PathBuf>, CytoScanError> {
    let parent = input.parent().filter(|path| !path.as_os_str().is_empty());
    let file_pattern = input
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| CytoScanError::UnsupportedGlob {
            pattern: raw.to_owned(),
        })?;
    let parent = parent.unwrap_or_else(|| Path::new("."));
    if parent.to_string_lossy().contains('*') || parent.to_string_lossy().contains('?') {
        return Err(CytoScanError::UnsupportedGlob {
            pattern: raw.to_owned(),
        });
    }

    let mut matches = fs::read_dir(parent)
        .map_err(|source| CytoScanError::DirectoryRead {
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
        return Err(CytoScanError::EmptyGlob {
            pattern: raw.to_owned(),
        });
    }

    Ok(matches)
}

fn resolve_directory(input: &Path) -> Result<Vec<PathBuf>, CytoScanError> {
    let mut paths = fs::read_dir(input)
        .map_err(|source| CytoScanError::DirectoryRead {
            path: input.to_path_buf(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_fcs_path(path))
        .collect::<Vec<_>>();
    paths.sort();
    if paths.is_empty() {
        return Err(CytoScanError::EmptyDirectory {
            path: input.to_path_buf(),
        });
    }

    Ok(paths)
}

fn is_fcs_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fcs"))
}

fn wildcard_match(pattern: &str, value: &str) -> bool {
    wildcard_match_inner(pattern.as_bytes(), value.as_bytes())
}

fn wildcard_match_inner(pattern: &[u8], value: &[u8]) -> bool {
    if pattern.is_empty() {
        return value.is_empty();
    }
    match pattern[0] {
        b'*' => {
            wildcard_match_inner(&pattern[1..], value)
                || (!value.is_empty() && wildcard_match_inner(pattern, &value[1..]))
        }
        b'?' => !value.is_empty() && wildcard_match_inner(&pattern[1..], &value[1..]),
        byte => {
            !value.is_empty()
                && byte == value[0]
                && wildcard_match_inner(&pattern[1..], &value[1..])
        }
    }
}

fn load_files(paths: &[PathBuf], channels: &[String]) -> Result<Vec<LoadedFile>, CytoScanError> {
    parallel_try_map(paths, "CytoScan FCS preprocessing", |_, path| {
        load_processed_file(path, channels)
    })
}

fn load_processed_file(
    path: &Path,
    requested_channels: &[String],
) -> Result<LoadedFile, CytoScanError> {
    debug!("loading FCS file {}", path.display());
    let dataset = FcsReader::open(path)
        .and_then(FcsReader::read)
        .map_err(|source| CytoScanError::FcsRead {
            path: path.to_path_buf(),
            source,
        })?;
    let event_count = event_count(&dataset);
    if event_count == 0 {
        return Err(CytoScanError::EmptyFcs {
            path: path.to_path_buf(),
        });
    }
    let channels = resolve_channels(path, &dataset, requested_channels)?;
    let spillover = parse_spillover(path, &dataset)?;
    let (matrix, warnings) = preprocess_matrix(path, &dataset, &channels, spillover.as_ref())?;
    Ok(LoadedFile {
        path: path.to_path_buf(),
        filename: file_name(path),
        channel_names: channels
            .iter()
            .map(|channel| channel.display_name.clone())
            .collect(),
        matrix,
        event_count,
        warnings,
    })
}

fn resolve_channels(
    path: &Path,
    dataset: &FcsDataset,
    requested_channels: &[String],
) -> Result<Vec<ResolvedChannel>, CytoScanError> {
    if requested_channels.is_empty() {
        return Err(CytoScanError::EmptyChannels);
    }

    let mut channels = Vec::with_capacity(requested_channels.len());
    for channel in requested_channels {
        let Some(parameter_index) = find_parameter_index(&dataset.parameters, channel) else {
            return Err(CytoScanError::MissingChannel {
                path: path.to_path_buf(),
                channel: channel.clone(),
                available_channels: dataset
                    .parameters
                    .iter()
                    .map(display_parameter_identifier)
                    .collect(),
            });
        };
        let parameter = &dataset.parameters[parameter_index];
        channels.push(ResolvedChannel {
            requested_name: channel.clone(),
            parameter_name: parameter.name.clone(),
            display_name: display_parameter_identifier(parameter),
            parameter_index,
            max_range: parameter_max_range(parameter),
        });
    }
    ensure_unique_channels(&channels)?;
    Ok(channels)
}

fn ensure_unique_channels(channels: &[ResolvedChannel]) -> Result<(), CytoScanError> {
    let mut seen = HashSet::new();
    for channel in channels {
        if !seen.insert(channel.parameter_index) {
            return Err(CytoScanError::DuplicateChannel {
                channel: channel.requested_name.clone(),
            });
        }
    }

    Ok(())
}

fn preprocess_matrix(
    path: &Path,
    dataset: &FcsDataset,
    channels: &[ResolvedChannel],
    spillover: Option<&SpilloverMatrix>,
) -> Result<(Vec<Vec<f64>>, Vec<String>), CytoScanError> {
    let event_count = event_count(dataset);
    let mut warnings = Vec::new();
    let mut output_columns = selected_raw_columns(path, dataset, channels)?;
    truncate_columns(channels, &mut output_columns);

    if let Some(spillover) = spillover {
        let compensated = compensate_selected_columns(path, dataset, channels, spillover)?;
        for (channel_index, compensated_column) in compensated.into_iter().enumerate() {
            if let Some(column) = compensated_column {
                output_columns[channel_index] = column
                    .into_iter()
                    .map(|value| (value / DEFAULT_ARCSINH_COFACTOR).asinh())
                    .collect();
            } else {
                warnings.push(format!(
                    "selected channel `{}` is not present in spillover metadata; leaving raw values unchanged",
                    channels[channel_index].display_name
                ));
            }
        }
    } else {
        warnings.push("FCS spillover metadata was not found; selected channels were not compensated or arcsinh transformed".to_owned());
    }

    let matrix = (0..event_count)
        .map(|event_index| {
            output_columns
                .iter()
                .map(|column| column[event_index])
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    Ok((matrix, warnings))
}

fn selected_raw_columns(
    path: &Path,
    dataset: &FcsDataset,
    channels: &[ResolvedChannel],
) -> Result<Vec<Vec<f64>>, CytoScanError> {
    channels
        .iter()
        .map(|channel| {
            let values = column_to_f64(
                dataset
                    .data
                    .column(channel.parameter_index)
                    .expect("parameter index should have a data column"),
            );
            if values.iter().any(|value| !value.is_finite()) {
                return Err(CytoScanError::NonFiniteChannel {
                    path: path.to_path_buf(),
                    channel: channel.display_name.clone(),
                });
            }
            Ok(values)
        })
        .collect()
}

fn truncate_columns(channels: &[ResolvedChannel], columns: &mut [Vec<f64>]) {
    for (channel, column) in channels.iter().zip(columns) {
        if let Some(max_range) = channel.max_range.filter(|value| value.is_finite()) {
            for value in column {
                if *value > max_range {
                    *value = max_range;
                }
            }
        }
    }
}

fn compensate_selected_columns(
    path: &Path,
    dataset: &FcsDataset,
    selected_channels: &[ResolvedChannel],
    spillover: &SpilloverMatrix,
) -> Result<Vec<Option<Vec<f64>>>, CytoScanError> {
    let spill_indexes = spillover
        .channels
        .iter()
        .map(|channel| {
            find_parameter_index(&dataset.parameters, channel).ok_or_else(|| {
                CytoScanError::InvalidSpillover {
                    path: path.to_path_buf(),
                    reason: format!(
                        "spillover channel `{channel}` was not found in FCS parameters"
                    ),
                }
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut spill_columns = spill_indexes
        .iter()
        .map(|index| {
            column_to_f64(
                dataset
                    .data
                    .column(*index)
                    .expect("spillover parameter index should have a data column"),
            )
        })
        .collect::<Vec<_>>();
    let spill_channels = spill_indexes
        .iter()
        .map(|index| ResolvedChannel {
            requested_name: dataset.parameters[*index].name.clone(),
            parameter_name: dataset.parameters[*index].name.clone(),
            display_name: display_parameter_identifier(&dataset.parameters[*index]),
            parameter_index: *index,
            max_range: parameter_max_range(&dataset.parameters[*index]),
        })
        .collect::<Vec<_>>();
    truncate_columns(&spill_channels, &mut spill_columns);

    let event_count = event_count(dataset);
    let mut compensated_columns = vec![vec![0.0; event_count]; spillover.channels.len()];
    for event_index in 0..event_count {
        for (output_index, output_column) in compensated_columns.iter_mut().enumerate() {
            output_column[event_index] = (0..spillover.channels.len())
                .map(|input_index| {
                    spill_columns[input_index][event_index]
                        * spillover.compensation[(input_index, output_index)]
                })
                .sum();
        }
    }
    let spill_lookup = spillover
        .channels
        .iter()
        .enumerate()
        .map(|(index, channel)| (channel.as_str(), index))
        .collect::<HashMap<_, _>>();

    Ok(selected_channels
        .iter()
        .map(|channel| {
            spill_lookup
                .get(channel.parameter_name.as_str())
                .map(|index| compensated_columns[*index].clone())
        })
        .collect())
}

fn parse_spillover(
    path: &Path,
    dataset: &FcsDataset,
) -> Result<Option<SpilloverMatrix>, CytoScanError> {
    let Some(raw) = dataset.text.iter().find_map(|(key, value)| {
        (key.eq_ignore_ascii_case("$SPILL")
            || key.eq_ignore_ascii_case("SPILL")
            || key.eq_ignore_ascii_case("$SPILLOVER")
            || key.eq_ignore_ascii_case("SPILLOVER"))
        .then_some(value)
    }) else {
        return Ok(None);
    };

    let tokens = raw
        .split(',')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let Some(first) = tokens.first() else {
        return Err(CytoScanError::InvalidSpillover {
            path: path.to_path_buf(),
            reason: "spillover value is empty".to_owned(),
        });
    };
    let size = first
        .parse::<usize>()
        .map_err(|error| CytoScanError::InvalidSpillover {
            path: path.to_path_buf(),
            reason: format!("invalid spillover channel count `{first}`: {error}"),
        })?;
    if size == 0 {
        return Err(CytoScanError::InvalidSpillover {
            path: path.to_path_buf(),
            reason: "spillover channel count must be greater than zero".to_owned(),
        });
    }
    let expected_len = 1 + size + size * size;
    if tokens.len() != expected_len {
        return Err(CytoScanError::InvalidSpillover {
            path: path.to_path_buf(),
            reason: format!(
                "expected {expected_len} comma-separated values for {size} channels, found {}",
                tokens.len()
            ),
        });
    }
    let channels = tokens[1..=size]
        .iter()
        .map(|channel| (*channel).to_owned())
        .collect::<Vec<_>>();
    let values = tokens[(1 + size)..]
        .iter()
        .map(|token| {
            token
                .parse::<f64>()
                .map_err(|error| CytoScanError::InvalidSpillover {
                    path: path.to_path_buf(),
                    reason: format!("invalid spillover value `{token}`: {error}"),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    if values.iter().any(|value| !value.is_finite()) {
        return Err(CytoScanError::InvalidSpillover {
            path: path.to_path_buf(),
            reason: "spillover matrix contains non-finite values".to_owned(),
        });
    }
    let matrix = DMatrix::from_row_slice(size, size, &values);
    let compensation = matrix
        .try_inverse()
        .ok_or_else(|| CytoScanError::SingularSpillover {
            path: path.to_path_buf(),
        })?;

    Ok(Some(SpilloverMatrix {
        channels,
        compensation,
    }))
}

fn event_count(dataset: &FcsDataset) -> usize {
    dataset.data.columns.first().map_or(0, ColumnData::len)
}

fn column_to_f64(column: &ColumnData) -> Vec<f64> {
    match column {
        ColumnData::UInt(values) => values.iter().map(|value| *value as f64).collect(),
        ColumnData::F32(values) => values.iter().map(|value| f64::from(*value)).collect(),
        ColumnData::F64(values) => values.clone(),
    }
}

fn find_parameter_index(parameters: &[Parameter], requested: &str) -> Option<usize> {
    let requested = requested.trim();
    parameters
        .iter()
        .position(|parameter| parameter.name == requested)
        .or_else(|| {
            parameters.iter().position(|parameter| {
                parameter
                    .label
                    .as_deref()
                    .is_some_and(|label| label == requested)
            })
        })
        .or_else(|| {
            parameters
                .iter()
                .position(|parameter| display_parameter_identifier(parameter) == requested)
        })
}

fn display_parameter_identifier(parameter: &Parameter) -> String {
    parameter.label.as_ref().map_or_else(
        || parameter.name.clone(),
        |label| format!("{} [{}]", parameter.name, label),
    )
}

fn parameter_max_range(parameter: &Parameter) -> Option<f64> {
    keyword_value(parameter, &format!("$P{}R", parameter.index))
        .or_else(|| keyword_value(parameter, "maxRange"))
        .or_else(|| keyword_value(parameter, "max"))
        .and_then(|value| value.trim().parse::<f64>().ok())
}

fn keyword_value<'a>(parameter: &'a Parameter, key: &str) -> Option<&'a str> {
    parameter
        .keywords
        .iter()
        .find_map(|(candidate, value)| (candidate == key).then_some(value.as_str()))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned()
}

fn collect_warnings(test_files: &[LoadedFile], reference_files: &[LoadedFile]) -> Vec<String> {
    test_files
        .iter()
        .chain(reference_files)
        .flat_map(|file| {
            file.warnings
                .iter()
                .map(|warning| format!("{}: {warning}", file.path.display()))
        })
        .collect()
}

fn aggregate_event_count(files: &[LoadedFile]) -> usize {
    files.iter().map(|file| file.event_count).sum()
}

fn parallel_try_map<T, U, E, F>(items: &[T], stage: &str, worker: F) -> Result<Vec<U>, E>
where
    T: Sync,
    U: Send,
    E: Send,
    F: Fn(usize, &T) -> Result<U, E> + Sync + Send,
{
    if items.len() < 2 {
        return items
            .iter()
            .enumerate()
            .map(|(index, item)| worker(index, item))
            .collect();
    }

    debug!("parallelizing {stage} over {} item(s)", items.len());
    let results = items
        .par_iter()
        .enumerate()
        .map(|(index, item)| worker(index, item))
        .collect::<Vec<_>>();
    results.into_iter().collect()
}

fn build_feature_model(
    method: FeatureMethod,
    aggregate_files: &[LoadedFile],
) -> Result<FeatureModel, CytoScanError> {
    if !method.uses_aggregate() {
        return Ok(FeatureModel::None);
    }
    let aggregate = aggregate_matrix(aggregate_files)?;
    if aggregate.is_empty() {
        return Err(CytoScanError::EmptyAggregate {
            method: method.as_str(),
        });
    }
    match method {
        FeatureMethod::Emd => Ok(FeatureModel::Emd {
            aggregate_columns: transpose(&aggregate),
        }),
        FeatureMethod::Binning => Ok(FeatureModel::Binning {
            boundaries: aggregate_boundaries(&aggregate),
        }),
        FeatureMethod::Fingerprint => Ok(FeatureModel::Fingerprint(FingerprintModel::fit(
            &aggregate,
            DEFAULT_FINGERPRINT_RECURSIONS,
        ))),
        FeatureMethod::Summary | FeatureMethod::Quantiles => Ok(FeatureModel::None),
    }
}

fn build_feature_matrix(
    files: &[LoadedFile],
    method: FeatureMethod,
    model: &FeatureModel,
) -> Result<CytoScanFeatureMatrix, CytoScanError> {
    if files.is_empty() {
        return Err(CytoScanError::EmptyFeatureMatrix);
    }
    let feature_names = feature_names(method, &files[0].channel_names, model);
    let rows = files
        .iter()
        .map(|file| {
            let values = file_features(file, method, model)?;
            Ok(CytoScanFeatureRow {
                path: file.path.clone(),
                filename: file.filename.clone(),
                values,
            })
        })
        .collect::<Result<Vec<_>, CytoScanError>>()?;
    Ok(CytoScanFeatureMatrix {
        feature_names,
        rows,
    })
}

fn file_features(
    file: &LoadedFile,
    method: FeatureMethod,
    model: &FeatureModel,
) -> Result<Vec<f64>, CytoScanError> {
    let sample = sampled_matrix(file, DEFAULT_EVENTS_PER_FILE);
    let features = match method {
        FeatureMethod::Summary => summary_features(&sample),
        FeatureMethod::Quantiles => quantile_features(&sample),
        FeatureMethod::Emd => {
            let FeatureModel::Emd { aggregate_columns } = model else {
                return Err(CytoScanError::FeatureShapeMismatch);
            };
            emd_features(&sample, aggregate_columns)
        }
        FeatureMethod::Binning => {
            let FeatureModel::Binning { boundaries } = model else {
                return Err(CytoScanError::FeatureShapeMismatch);
            };
            binning_features(&sample, boundaries)
        }
        FeatureMethod::Fingerprint => {
            let FeatureModel::Fingerprint(model) = model else {
                return Err(CytoScanError::FeatureShapeMismatch);
            };
            fingerprint_features(&sample, model)
        }
    };

    Ok(features)
}

fn feature_names(method: FeatureMethod, channels: &[String], model: &FeatureModel) -> Vec<String> {
    match method {
        FeatureMethod::Summary => channels
            .iter()
            .flat_map(|channel| {
                ["mean", "sd", "median", "IQR"]
                    .into_iter()
                    .map(move |suffix| format!("{channel}_{suffix}"))
            })
            .collect(),
        FeatureMethod::Quantiles => channels
            .iter()
            .flat_map(|channel| {
                (1..=9).map(move |index| format!("{channel}_{:.1}", f64::from(index) * 0.1))
            })
            .collect(),
        FeatureMethod::Emd => channels
            .iter()
            .map(|channel| format!("{channel}_EMD"))
            .collect(),
        FeatureMethod::Binning => channels
            .iter()
            .flat_map(|channel| (1..=10).map(move |index| format!("{channel}_bin{index}")))
            .collect(),
        FeatureMethod::Fingerprint => {
            let bins = match model {
                FeatureModel::Fingerprint(model) => model.bins,
                FeatureModel::None | FeatureModel::Emd { .. } | FeatureModel::Binning { .. } => {
                    1_usize << DEFAULT_FINGERPRINT_RECURSIONS
                }
            };
            (1..=bins)
                .map(|index| format!("fingerprint_bin{index}"))
                .collect()
        }
    }
}

fn sampled_matrix(file: &LoadedFile, limit: usize) -> Vec<Vec<f64>> {
    let indices = sample_indices(
        file.matrix.len(),
        limit,
        DEFAULT_RANDOM_SEED ^ stable_hash_path(&file.path),
    );
    indices
        .into_iter()
        .map(|index| file.matrix[index].clone())
        .collect()
}

fn aggregate_matrix(files: &[LoadedFile]) -> Result<Vec<Vec<f64>>, CytoScanError> {
    if files.is_empty() {
        return Err(CytoScanError::EmptyAggregate {
            method: "aggregate",
        });
    }
    let per_file = DEFAULT_AGGREGATE_EVENTS.div_ceil(files.len());
    let mut aggregate = Vec::new();
    for file in files {
        aggregate.extend(sampled_matrix(file, per_file));
    }
    if aggregate.len() > DEFAULT_AGGREGATE_EVENTS {
        let indices = sample_indices(
            aggregate.len(),
            DEFAULT_AGGREGATE_EVENTS,
            DEFAULT_RANDOM_SEED ^ 0x9e37_79b9_7f4a_7c15,
        );
        aggregate = indices
            .into_iter()
            .map(|index| aggregate[index].clone())
            .collect();
    }
    Ok(aggregate)
}

fn sample_indices(length: usize, limit: usize, seed: u64) -> Vec<usize> {
    if length <= limit {
        return (0..length).collect();
    }
    let mut indices = (0..length).collect::<Vec<_>>();
    let mut rng = DeterministicRng::new(seed);
    for index in 0..limit {
        let swap_index = index + rng.next_index(length - index);
        indices.swap(index, swap_index);
    }
    indices.truncate(limit);
    indices.sort_unstable();
    indices
}

fn stable_hash_path(path: &Path) -> u64 {
    path.to_string_lossy()
        .bytes()
        .fold(14_695_981_039_346_656_037_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211)
        })
}

fn summary_features(sample: &[Vec<f64>]) -> Vec<f64> {
    let columns = transpose(sample);
    columns
        .into_iter()
        .flat_map(|mut column| {
            let mean = mean(&column);
            let sd = sample_standard_deviation(&column, mean);
            let median = quantile_sorted(&mut column, 0.5);
            let q25 = quantile_sorted(&mut column, 0.25);
            let q75 = quantile_sorted(&mut column, 0.75);
            [mean, sd, median, q75 - q25]
        })
        .collect()
}

fn quantile_features(sample: &[Vec<f64>]) -> Vec<f64> {
    transpose(sample)
        .into_iter()
        .flat_map(|mut column| {
            (1..=9).map(move |index| quantile_sorted(&mut column, f64::from(index) * 0.1))
        })
        .collect()
}

fn emd_features(sample: &[Vec<f64>], aggregate_columns: &[Vec<f64>]) -> Vec<f64> {
    transpose(sample)
        .iter()
        .zip(aggregate_columns)
        .map(|(column, aggregate)| wasserstein_1d(column, aggregate))
        .collect()
}

fn binning_features(sample: &[Vec<f64>], boundaries: &[Vec<f64>]) -> Vec<f64> {
    let columns = transpose(sample);
    columns
        .iter()
        .zip(boundaries)
        .flat_map(|(column, boundary)| {
            let mut counts = [0_usize; 10];
            for value in column {
                counts[bin_index(*value, boundary)] += 1;
            }
            let denominator = column.len().max(1) as f64;
            counts.map(|count| count as f64 / denominator)
        })
        .collect()
}

fn fingerprint_features(sample: &[Vec<f64>], model: &FingerprintModel) -> Vec<f64> {
    let mut counts = vec![0_usize; model.bins];
    for row in sample {
        counts[model.assign(row)] += 1;
    }
    let denominator = sample.len().max(1) as f64;
    counts
        .into_iter()
        .map(|count| count as f64 / denominator)
        .collect()
}

fn aggregate_boundaries(aggregate: &[Vec<f64>]) -> Vec<Vec<f64>> {
    transpose(aggregate)
        .into_iter()
        .map(|mut column| {
            (0..=10)
                .map(|index| quantile_sorted(&mut column, f64::from(index) * 0.1))
                .collect()
        })
        .collect()
}

fn bin_index(value: f64, boundaries: &[f64]) -> usize {
    if boundaries.len() < 2 || value <= boundaries[0] {
        return 0;
    }
    for (index, boundary) in boundaries.iter().enumerate().skip(1) {
        if value <= *boundary {
            return (index - 1).min(9);
        }
    }
    9
}

fn transpose(matrix: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let Some(first) = matrix.first() else {
        return Vec::new();
    };
    (0..first.len())
        .map(|column_index| matrix.iter().map(|row| row[column_index]).collect())
        .collect()
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn sample_standard_deviation(values: &[f64], mean: f64) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let variance = values
        .iter()
        .map(|value| {
            let delta = value - mean;
            delta * delta
        })
        .sum::<f64>()
        / (values.len() - 1) as f64;
    variance.sqrt()
}

fn quantile_sorted(values: &mut [f64], probability: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(f64::total_cmp);
    let probability = probability.clamp(0.0, 1.0);
    if values.len() == 1 {
        return values[0];
    }
    let position = probability * (values.len() - 1) as f64;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        values[lower]
    } else {
        let fraction = position - lower as f64;
        values[lower] * (1.0 - fraction) + values[upper] * fraction
    }
}

fn wasserstein_1d(left: &[f64], right: &[f64]) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort_by(f64::total_cmp);
    right.sort_by(f64::total_cmp);

    let left_weight = 1.0 / left.len() as f64;
    let right_weight = 1.0 / right.len() as f64;
    let mut left_index = 0;
    let mut right_index = 0;
    let mut left_cdf: f64 = 0.0;
    let mut right_cdf: f64 = 0.0;
    let mut previous = left[0].min(right[0]);
    let mut distance = 0.0;

    while left_index < left.len() || right_index < right.len() {
        let next_left = left.get(left_index).copied().unwrap_or(f64::INFINITY);
        let next_right = right.get(right_index).copied().unwrap_or(f64::INFINITY);
        let next = next_left.min(next_right);
        distance += (left_cdf - right_cdf).abs() * (next - previous);
        while left_index < left.len() && left[left_index] <= next {
            left_cdf += left_weight;
            left_index += 1;
        }
        while right_index < right.len() && right[right_index] <= next {
            right_cdf += right_weight;
            right_index += 1;
        }
        previous = next;
    }

    distance
}

impl FingerprintModel {
    fn fit(rows: &[Vec<f64>], recursions: usize) -> Self {
        let bins = 1_usize << recursions;
        let mut splits = vec![
            FingerprintSplit {
                channel_index: 0,
                threshold: 0.0,
            };
            bins.saturating_sub(1)
        ];
        let indexes = (0..rows.len()).collect::<Vec<_>>();
        Self::fit_node(rows, &indexes, 0, recursions, &mut splits);
        Self { splits, bins }
    }

    fn fit_node(
        rows: &[Vec<f64>],
        indexes: &[usize],
        node_index: usize,
        remaining_depth: usize,
        splits: &mut [FingerprintSplit],
    ) {
        if remaining_depth == 0 || node_index >= splits.len() || rows.is_empty() {
            return;
        }
        let channel_index = highest_variance_channel(rows, indexes);
        let threshold = median_for_channel(rows, indexes, channel_index);
        splits[node_index] = FingerprintSplit {
            channel_index,
            threshold,
        };
        let mut left = Vec::new();
        let mut right = Vec::new();
        for index in indexes {
            if rows[*index][channel_index] <= threshold {
                left.push(*index);
            } else {
                right.push(*index);
            }
        }
        if left.is_empty() || right.is_empty() {
            let midpoint = indexes.len() / 2;
            left = indexes[..midpoint].to_vec();
            right = indexes[midpoint..].to_vec();
        }
        Self::fit_node(rows, &left, node_index * 2 + 1, remaining_depth - 1, splits);
        Self::fit_node(
            rows,
            &right,
            node_index * 2 + 2,
            remaining_depth - 1,
            splits,
        );
    }

    fn assign(&self, row: &[f64]) -> usize {
        let mut node_index = 0;
        let mut bin_index = 0;
        for _ in 0..DEFAULT_FINGERPRINT_RECURSIONS {
            let split = self.splits[node_index];
            let right = row
                .get(split.channel_index)
                .is_some_and(|value| *value > split.threshold);
            bin_index = bin_index * 2 + usize::from(right);
            node_index = node_index * 2 + 1 + usize::from(right);
            if node_index >= self.splits.len() {
                break;
            }
        }
        bin_index.min(self.bins.saturating_sub(1))
    }
}

fn highest_variance_channel(rows: &[Vec<f64>], indexes: &[usize]) -> usize {
    let channel_count = rows.first().map_or(0, Vec::len);
    (0..channel_count)
        .max_by(|left, right| {
            variance_for_channel(rows, indexes, *left)
                .total_cmp(&variance_for_channel(rows, indexes, *right))
        })
        .unwrap_or(0)
}

fn variance_for_channel(rows: &[Vec<f64>], indexes: &[usize], channel_index: usize) -> f64 {
    if indexes.is_empty() {
        return 0.0;
    }
    let mean = indexes
        .iter()
        .map(|index| rows[*index][channel_index])
        .sum::<f64>()
        / indexes.len() as f64;
    indexes
        .iter()
        .map(|index| {
            let delta = rows[*index][channel_index] - mean;
            delta * delta
        })
        .sum::<f64>()
        / indexes.len() as f64
}

fn median_for_channel(rows: &[Vec<f64>], indexes: &[usize], channel_index: usize) -> f64 {
    let mut values = indexes
        .iter()
        .map(|index| rows[*index][channel_index])
        .collect::<Vec<_>>();
    quantile_sorted(&mut values, 0.5)
}

fn ensure_feature_shape(
    reference: &CytoScanFeatureMatrix,
    test: &CytoScanFeatureMatrix,
) -> Result<(), CytoScanError> {
    if reference.feature_names == test.feature_names {
        Ok(())
    } else {
        Err(CytoScanError::FeatureShapeMismatch)
    }
}

fn build_summary_rows(
    config: &CytoScanConfig,
    test_files: &[LoadedFile],
    test_features: &CytoScanFeatureMatrix,
    outlier_scores: Option<&[f64]>,
    novelty_log_likelihoods: Option<&[f64]>,
    novelty_threshold: Option<f64>,
) -> Vec<CytoScanSummaryRow> {
    test_features
        .rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let outlier_score = outlier_scores.and_then(|scores| scores.get(index).copied());
            let novelty_log_likelihood =
                novelty_log_likelihoods.and_then(|scores| scores.get(index).copied());
            let is_outlier = outlier_score.map(|score| score >= DEFAULT_OUTLIER_THRESHOLD);
            let is_novelty = novelty_log_likelihood
                .zip(novelty_threshold)
                .map(|(score, threshold)| score <= threshold);
            CytoScanSummaryRow {
                filename: row.filename.clone(),
                path: row.path.clone(),
                flag_method: config.flag_method,
                feature_method: config.feature_method,
                is_outlier,
                outlier_score,
                is_novelty,
                novelty_log_likelihood,
                channels: config.channels.join(","),
                warnings: test_files
                    .get(index)
                    .map(|file| file.warnings.join("; "))
                    .unwrap_or_default(),
            }
        })
        .collect()
}

fn score_outliers(features: &[Vec<f64>]) -> Vec<f64> {
    if features.len() < 2 || features.first().is_none_or(Vec::is_empty) {
        return vec![0.0; features.len()];
    }
    let max_depth = (features.len() as f64).log2().ceil() as usize + 1;
    let normalization = average_path_length(features.len()).max(EQUALITY_EPSILON);
    let mut path_sums = vec![0.0; features.len()];
    for tree_index in 0..DEFAULT_ISOLATION_TREES {
        let mut rng = DeterministicRng::new(DEFAULT_RANDOM_SEED ^ tree_index as u64);
        let indexes = (0..features.len()).collect::<Vec<_>>();
        let tree = build_isolation_tree(features, &indexes, 0, max_depth, &mut rng);
        for (row_index, row) in features.iter().enumerate() {
            path_sums[row_index] += tree.path_length(row, 0.0);
        }
    }
    path_sums
        .into_iter()
        .map(|sum| {
            let average = sum / DEFAULT_ISOLATION_TREES as f64;
            2_f64.powf(-average / normalization)
        })
        .collect()
}

fn build_isolation_tree(
    features: &[Vec<f64>],
    indexes: &[usize],
    depth: usize,
    max_depth: usize,
    rng: &mut DeterministicRng,
) -> IsolationNode {
    if indexes.len() <= 1 || depth >= max_depth {
        return IsolationNode::Leaf {
            size: indexes.len(),
        };
    }
    let feature_count = features.first().map_or(0, Vec::len);
    if feature_count == 0 {
        return IsolationNode::Leaf {
            size: indexes.len(),
        };
    }
    for _ in 0..feature_count {
        let feature_index = rng.next_index(feature_count);
        let (minimum, maximum) = feature_min_max(features, indexes, feature_index);
        if maximum - minimum <= EQUALITY_EPSILON {
            continue;
        }
        let threshold = minimum + rng.next_f64() * (maximum - minimum);
        let mut left = Vec::new();
        let mut right = Vec::new();
        for index in indexes {
            if features[*index][feature_index] <= threshold {
                left.push(*index);
            } else {
                right.push(*index);
            }
        }
        if left.is_empty() || right.is_empty() {
            continue;
        }
        return IsolationNode::Split {
            feature_index,
            threshold,
            left: Box::new(build_isolation_tree(
                features,
                &left,
                depth + 1,
                max_depth,
                rng,
            )),
            right: Box::new(build_isolation_tree(
                features,
                &right,
                depth + 1,
                max_depth,
                rng,
            )),
        };
    }
    IsolationNode::Leaf {
        size: indexes.len(),
    }
}

impl IsolationNode {
    fn path_length(&self, row: &[f64], depth: f64) -> f64 {
        match self {
            Self::Leaf { size } => depth + average_path_length(*size),
            Self::Split {
                feature_index,
                threshold,
                left,
                right,
            } => {
                if row[*feature_index] <= *threshold {
                    left.path_length(row, depth + 1.0)
                } else {
                    right.path_length(row, depth + 1.0)
                }
            }
        }
    }
}

fn feature_min_max(features: &[Vec<f64>], indexes: &[usize], feature_index: usize) -> (f64, f64) {
    indexes.iter().fold(
        (f64::INFINITY, f64::NEG_INFINITY),
        |(minimum, maximum), index| {
            let value = features[*index][feature_index];
            (minimum.min(value), maximum.max(value))
        },
    )
}

fn average_path_length(size: usize) -> f64 {
    match size {
        0 | 1 => 0.0,
        2 => 1.0,
        value => {
            2.0 * ((value - 1) as f64).ln() + 2.0 * EULER_MASCHERONI
                - 2.0 * (value - 1) as f64 / value as f64
        }
    }
}

fn fit_novelty_model(features: &[Vec<f64>]) -> Result<NoveltyModel, CytoScanError> {
    if features.is_empty() || features.first().is_none_or(Vec::is_empty) {
        return Err(CytoScanError::EmptyFeatureMatrix);
    }
    let scale = FeatureScale::fit(features);
    let scaled = scale.transform_matrix(features);
    let max_components = MAX_GMM_COMPONENTS.min(scaled.len()).max(1);
    let mut best = None;
    for components in 1..=max_components {
        let gmm = fit_diagonal_gmm(&scaled, components);
        if best
            .as_ref()
            .is_none_or(|current: &DiagonalGmm| gmm.bic < current.bic)
        {
            best = Some(gmm);
        }
    }
    let gmm = best.ok_or(CytoScanError::EmptyFeatureMatrix)?;
    let reference_log_likelihoods = scaled
        .iter()
        .map(|row| gmm.log_likelihood_row(row))
        .collect::<Vec<_>>();
    let mut threshold_values = reference_log_likelihoods.clone();
    let threshold = quantile_sorted(&mut threshold_values, DEFAULT_NOVELTY_QUANTILE);

    Ok(NoveltyModel {
        gmm,
        scale,
        threshold,
    })
}

fn log_likelihoods(model: &NoveltyModel, features: &[Vec<f64>]) -> Vec<f64> {
    let scaled = model.scale.transform_matrix(features);
    scaled
        .iter()
        .map(|row| model.gmm.log_likelihood_row(row))
        .collect()
}

impl FeatureScale {
    fn fit(features: &[Vec<f64>]) -> Self {
        let columns = transpose(features);
        let means = columns
            .iter()
            .map(|column| mean(column))
            .collect::<Vec<_>>();
        let standard_deviations = columns
            .iter()
            .zip(&means)
            .map(|(column, mean)| {
                let sd = sample_standard_deviation(column, *mean);
                if sd <= EQUALITY_EPSILON { 1.0 } else { sd }
            })
            .collect::<Vec<_>>();
        Self {
            means,
            standard_deviations,
        }
    }

    fn transform_matrix(&self, features: &[Vec<f64>]) -> Vec<Vec<f64>> {
        features
            .iter()
            .map(|row| {
                row.iter()
                    .enumerate()
                    .map(|(index, value)| {
                        (value - self.means[index]) / self.standard_deviations[index]
                    })
                    .collect()
            })
            .collect()
    }
}

fn fit_diagonal_gmm(features: &[Vec<f64>], components: usize) -> DiagonalGmm {
    let dimensions = features.first().map_or(0, Vec::len);
    let mut weights = vec![1.0 / components as f64; components];
    let mut means = initial_gmm_means(features, components);
    let global_variance = global_variances(features);
    let mut variances = vec![global_variance; components];
    let mut previous_log_likelihood = f64::NEG_INFINITY;

    for _ in 0..GMM_ITERATIONS {
        let responsibilities = gmm_responsibilities(features, &weights, &means, &variances);
        let log_likelihood = responsibilities
            .iter()
            .map(|(_, row_log_likelihood)| *row_log_likelihood)
            .sum::<f64>();
        if (log_likelihood - previous_log_likelihood).abs() < GMM_TOLERANCE {
            break;
        }
        previous_log_likelihood = log_likelihood;

        for component in 0..components {
            let responsibility_sum = responsibilities
                .iter()
                .map(|(row, _)| row[component])
                .sum::<f64>()
                .max(EQUALITY_EPSILON);
            weights[component] = responsibility_sum / features.len() as f64;
            for dimension in 0..dimensions {
                means[component][dimension] = responsibilities
                    .iter()
                    .zip(features)
                    .map(|((row, _), feature_row)| row[component] * feature_row[dimension])
                    .sum::<f64>()
                    / responsibility_sum;
            }
            for dimension in 0..dimensions {
                variances[component][dimension] = responsibilities
                    .iter()
                    .zip(features)
                    .map(|((row, _), feature_row)| {
                        let delta = feature_row[dimension] - means[component][dimension];
                        row[component] * delta * delta
                    })
                    .sum::<f64>()
                    / responsibility_sum;
                variances[component][dimension] =
                    variances[component][dimension].max(VARIANCE_FLOOR);
            }
        }
    }

    let log_likelihood = features
        .iter()
        .map(|row| log_mixture_density(row, &weights, &means, &variances))
        .sum::<f64>();
    let parameter_count = components * dimensions * 2 + components.saturating_sub(1);
    let bic = -2.0 * log_likelihood + parameter_count as f64 * (features.len() as f64).ln();

    DiagonalGmm {
        weights,
        means,
        variances,
        bic,
    }
}

fn initial_gmm_means(features: &[Vec<f64>], components: usize) -> Vec<Vec<f64>> {
    let mut ordered = features.to_vec();
    ordered.sort_by(|left, right| row_norm(left).total_cmp(&row_norm(right)));
    (0..components)
        .map(|component| {
            let index = if components == 1 {
                ordered.len() / 2
            } else {
                component * (ordered.len() - 1) / (components - 1)
            };
            ordered[index].clone()
        })
        .collect()
}

fn row_norm(row: &[f64]) -> f64 {
    row.iter().map(|value| value * value).sum::<f64>().sqrt()
}

fn global_variances(features: &[Vec<f64>]) -> Vec<f64> {
    transpose(features)
        .iter()
        .map(|column| {
            let column_mean = mean(column);
            let variance = column
                .iter()
                .map(|value| {
                    let delta = value - column_mean;
                    delta * delta
                })
                .sum::<f64>()
                / column.len().max(1) as f64;
            variance.max(VARIANCE_FLOOR)
        })
        .collect()
}

fn gmm_responsibilities(
    features: &[Vec<f64>],
    weights: &[f64],
    means: &[Vec<f64>],
    variances: &[Vec<f64>],
) -> Vec<(Vec<f64>, f64)> {
    features
        .iter()
        .map(|row| {
            let logs = weights
                .iter()
                .enumerate()
                .map(|(component, weight)| {
                    weight.max(EQUALITY_EPSILON).ln()
                        + gaussian_log_density(row, &means[component], &variances[component])
                })
                .collect::<Vec<_>>();
            let log_sum = log_sum_exp(&logs);
            let responsibilities = logs
                .into_iter()
                .map(|value| (value - log_sum).exp())
                .collect::<Vec<_>>();
            (responsibilities, log_sum)
        })
        .collect()
}

impl DiagonalGmm {
    fn log_likelihood_row(&self, row: &[f64]) -> f64 {
        log_mixture_density(row, &self.weights, &self.means, &self.variances)
    }
}

fn log_mixture_density(
    row: &[f64],
    weights: &[f64],
    means: &[Vec<f64>],
    variances: &[Vec<f64>],
) -> f64 {
    let logs = weights
        .iter()
        .enumerate()
        .map(|(component, weight)| {
            weight.max(EQUALITY_EPSILON).ln()
                + gaussian_log_density(row, &means[component], &variances[component])
        })
        .collect::<Vec<_>>();
    log_sum_exp(&logs)
}

fn gaussian_log_density(row: &[f64], mean: &[f64], variance: &[f64]) -> f64 {
    let dimensions = row.len() as f64;
    let log_determinant = variance.iter().map(|value| value.ln()).sum::<f64>();
    let mahalanobis = row
        .iter()
        .zip(mean)
        .zip(variance)
        .map(|((value, mean), variance)| {
            let delta = value - mean;
            delta * delta / variance
        })
        .sum::<f64>();
    -0.5 * (dimensions * std::f64::consts::TAU.ln() + log_determinant + mahalanobis)
}

fn log_sum_exp(values: &[f64]) -> f64 {
    let maximum = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if !maximum.is_finite() {
        return maximum;
    }
    maximum
        + values
            .iter()
            .map(|value| (value - maximum).exp())
            .sum::<f64>()
            .ln()
}
