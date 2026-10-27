#![allow(clippy::too_many_lines)]

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use cytiq_compensate::{MatrixKind, read_matrix_file};
use cytiq_fcs::{FcsDataset, FcsReader};
use log::{debug, info, warn};
use rayon::{ThreadPoolBuilder, prelude::*};
use serde::Serialize;

use crate::batch::{BatchRow, BatchSettings, UNKNOWN_MATRIX, summarize};
use crate::compensaid::{
    CompensaidParameters, CompensaidResult, FLAG_THRESHOLD, SsiStatus, compensaid,
};
use crate::error::SpillQcError;
use crate::export::{
    BATCH_FILE, CUTOFFS_FILE, REPORT_FILE, SSI_MATRIX_FILE, SSI_SEGMENTS_FILE, SUMMARY_FILE,
    write_batch, write_cutoffs, write_json, write_ssi_matrix, write_ssi_segments,
};
use crate::preprocess::{
    ExternalMatrix, LogicleRecord, MatrixRecord, Preparation, PreprocessOptions, prepare,
};
use crate::report::{DOT_PLOT_MAX_FILES, FileReport, file_report, write_report};

const REVIEW_NOTE: &str = "Flags need expert review: CompensAID reports high sensitivity but 0.16-0.27 precision against expert consensus. Check the pair on a dot plot before changing a spillover value.";

/// Settings of a `cytiq spillqc` run.
#[derive(Clone, Debug, PartialEq)]
pub struct SpillQcConfig {
    /// FCS files, directories of FCS files or file-name globs.
    pub inputs: Vec<PathBuf>,
    /// Output directory.
    pub output: PathBuf,
    /// An external matrix to assess instead of each file's own `$SPILL`.
    pub matrix: Option<(PathBuf, MatrixKind)>,
    pub preprocess: PreprocessOptions,
    pub parameters: CompensaidParameters,
    pub batch: BatchSettings,
    pub threads: Option<usize>,
    pub overwrite: bool,
}

impl SpillQcConfig {
    #[must_use]
    pub fn new(inputs: Vec<PathBuf>, output: PathBuf) -> Self {
        Self {
            inputs,
            output,
            matrix: None,
            preprocess: PreprocessOptions::default(),
            parameters: CompensaidParameters::default(),
            batch: BatchSettings::default(),
            threads: None,
            overwrite: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FileStatus {
    Assessed,
    Skipped,
}

/// A channel's density cut-off.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ChannelSummary {
    pub channel: String,
    pub marker: String,
    pub cutoff: f64,
    pub candidates: Vec<f64>,
}

/// A flagged pair of a file.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FlagSummary {
    pub primary_channel: String,
    pub primary_marker: String,
    pub secondary_channel: String,
    pub secondary_marker: String,
    pub ssi: Option<f64>,
    pub status: SsiStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FileSummary {
    pub input: String,
    /// Name of the file's output directory.
    pub name: String,
    pub status: FileStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub events_total: usize,
    pub margin_removed: usize,
    pub peacoqc_removed: usize,
    pub events_assessed: usize,
    pub matrix: Option<MatrixRecord>,
    pub logicle: Vec<LogicleRecord>,
    pub center: Option<f64>,
    pub separation: Option<f64>,
    pub channels: Vec<ChannelSummary>,
    pub pairs_assessed: usize,
    pub flags: Vec<FlagSummary>,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpillQcParameters {
    pub segments: usize,
    pub min_events: usize,
    pub flag_threshold: f64,
    pub preprocessed: bool,
    pub remove_margins: bool,
    pub peacoqc: bool,
    pub channels: Option<Vec<String>>,
    pub matrix: Option<String>,
    pub batch_min_files: usize,
    pub batch_min_fraction: f64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SpillQcCounts {
    pub files: usize,
    pub assessed: usize,
    pub skipped: usize,
    pub flagged_pairs: usize,
    pub recurrent_pairs: usize,
}

/// `summary.json`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpillQcSummary {
    pub tool: String,
    pub version: String,
    pub algorithm: String,
    pub note: String,
    pub parameters: SpillQcParameters,
    pub counts: SpillQcCounts,
    pub files: Vec<FileSummary>,
    pub batch: Vec<BatchRow>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SpillQcResult {
    pub summary: SpillQcSummary,
    /// Every file written, for provenance.
    pub outputs: Vec<PathBuf>,
}

pub struct SpillQcRunner;

impl SpillQcRunner {
    /// The FCS files a run with `config` reads.
    ///
    /// # Errors
    ///
    /// Returns an error when an input cannot be resolved.
    pub fn input_files(config: &SpillQcConfig) -> Result<Vec<PathBuf>, SpillQcError> {
        resolve_inputs(&config.inputs)
    }

    /// Assess every input and write the outputs.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration is invalid, an input cannot be
    /// read, an input was already compensated or transformed by cytiq, or an
    /// output cannot be written. Files that cannot be assessed are reported as
    /// skipped instead.
    pub fn run(config: &SpillQcConfig) -> Result<SpillQcResult, SpillQcError> {
        validate(config)?;
        let inputs = resolve_inputs(&config.inputs)?;
        let names = output_names(&inputs)?;
        prepare_output(&config.output, &names, config.overwrite)?;
        let external = config
            .matrix
            .as_ref()
            .map(|(path, kind)| {
                read_matrix_file(path, *kind).map(|matrix| ExternalMatrix { matrix })
            })
            .transpose()?;
        with_thread_pool(config.threads, || {
            run_inner(config, &inputs, &names, external.as_ref())
        })
    }
}

struct Assessed {
    summary: FileSummary,
    result: Option<CompensaidResult>,
    report: Option<FileReport>,
}

fn run_inner(
    config: &SpillQcConfig,
    inputs: &[PathBuf],
    names: &[String],
    external: Option<&ExternalMatrix>,
) -> Result<SpillQcResult, SpillQcError> {
    let with_dot_plots = inputs.len() <= DOT_PLOT_MAX_FILES;
    let assessed = inputs
        .par_iter()
        .zip(names)
        .map(|(input, name)| assess_file(config, input, name, external, with_dot_plots))
        .collect::<Vec<_>>();
    let mut files = Vec::with_capacity(assessed.len());
    for outcome in assessed {
        files.push(outcome?);
    }

    let mut outputs = Vec::new();
    for file in &files {
        if let Some(result) = &file.result {
            let directory = config.output.join(&file.summary.name);
            fs::create_dir_all(&directory).map_err(|source| SpillQcError::Write {
                path: directory.clone(),
                source,
            })?;
            for (file_name, write) in [
                (SSI_MATRIX_FILE, write_ssi_matrix as WriteFn),
                (SSI_SEGMENTS_FILE, write_ssi_segments),
                (CUTOFFS_FILE, write_cutoffs),
            ] {
                let path = directory.join(file_name);
                write(&path, result)?;
                outputs.push(path);
            }
        }
    }

    let grouped = files
        .iter()
        .filter_map(|file| {
            let result = file.result.as_ref()?;
            let id = file
                .summary
                .matrix
                .as_ref()
                .map_or(UNKNOWN_MATRIX, |matrix| matrix.id.as_str());
            Some((id, result))
        })
        .collect::<Vec<_>>();
    let batch = summarize(&grouped, &config.batch);

    let counts = SpillQcCounts {
        files: files.len(),
        assessed: files.iter().filter(|file| file.result.is_some()).count(),
        skipped: files.iter().filter(|file| file.result.is_none()).count(),
        flagged_pairs: files.iter().map(|file| file.summary.flags.len()).sum(),
        recurrent_pairs: batch.iter().filter(|row| row.status.is_recurrent()).count(),
    };
    let summary = SpillQcSummary {
        tool: "cytiq spillqc".to_owned(),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        algorithm: "CompensAID 0.99.7 Secondary Stain Index".to_owned(),
        note: REVIEW_NOTE.to_owned(),
        parameters: SpillQcParameters {
            segments: config.parameters.segments,
            min_events: config.parameters.min_events,
            flag_threshold: FLAG_THRESHOLD,
            preprocessed: config.preprocess.preprocessed,
            remove_margins: config.preprocess.remove_margins && !config.preprocess.preprocessed,
            peacoqc: config.preprocess.peacoqc && !config.preprocess.preprocessed,
            channels: config.preprocess.channels.clone(),
            matrix: config
                .matrix
                .as_ref()
                .map(|(path, _)| path.display().to_string()),
            batch_min_files: config.batch.min_files,
            batch_min_fraction: config.batch.min_fraction,
        },
        counts,
        files: files.iter().map(|file| file.summary.clone()).collect(),
        batch,
    };

    let batch_path = config.output.join(BATCH_FILE);
    write_batch(&batch_path, &summary.batch)?;
    outputs.push(batch_path);

    let report_path = config.output.join(REPORT_FILE);
    let reports = files
        .iter()
        .filter_map(|file| file.report.as_ref())
        .collect::<Vec<_>>();
    let skipped = files
        .iter()
        .filter(|file| file.result.is_none())
        .map(|file| {
            (
                file.summary.name.clone(),
                file.summary.reason.clone().unwrap_or_default(),
            )
        })
        .collect::<Vec<_>>();
    let summary_line = format!(
        "{} of {} files assessed; {} flagged pairs; {} recurrent across files sharing a matrix",
        summary.counts.assessed,
        summary.counts.files,
        summary.counts.flagged_pairs,
        summary.counts.recurrent_pairs
    );
    write_report(
        &report_path,
        &summary_line,
        &reports,
        &summary.batch,
        &skipped,
    )?;
    outputs.push(report_path);

    let summary_path = config.output.join(SUMMARY_FILE);
    write_json(&summary_path, &summary)?;
    outputs.push(summary_path);
    info!("spillqc: {summary_line}");
    Ok(SpillQcResult { summary, outputs })
}

type WriteFn = fn(&Path, &CompensaidResult) -> Result<(), SpillQcError>;

fn assess_file(
    config: &SpillQcConfig,
    input: &Path,
    name: &str,
    external: Option<&ExternalMatrix>,
    with_dot_plots: bool,
) -> Result<Assessed, SpillQcError> {
    debug!("spillqc: reading {}", input.display());
    let dataset = load_dataset(input)?;
    let empty = |reason: String, events_total: usize| FileSummary {
        input: input.display().to_string(),
        name: name.to_owned(),
        status: FileStatus::Skipped,
        reason: Some(reason),
        events_total,
        margin_removed: 0,
        peacoqc_removed: 0,
        events_assessed: 0,
        matrix: None,
        logicle: Vec::new(),
        center: None,
        separation: None,
        channels: Vec::new(),
        pairs_assessed: 0,
        flags: Vec::new(),
        warnings: Vec::new(),
    };
    let skipped = |summary: FileSummary| Assessed {
        summary,
        result: None,
        report: None,
    };
    if dataset.event_count() == 0 {
        warn!("spillqc: skipping {}: no events", input.display());
        return Ok(skipped(empty("the file has no events".to_owned(), 0)));
    }
    let prepared = match prepare(input, &dataset, external, &config.preprocess)? {
        Preparation::Ready(prepared) => prepared,
        Preparation::Skipped(reason) => {
            warn!("spillqc: skipping {}: {reason}", input.display());
            return Ok(skipped(empty(reason, dataset.event_count())));
        }
    };
    let events_assessed = prepared.columns.first().map_or(0, Vec::len);
    if events_assessed < 2 * config.parameters.min_events {
        let reason = format!("only {events_assessed} events remain after preprocessing");
        warn!("spillqc: skipping {}: {reason}", input.display());
        let mut summary = empty(reason, prepared.events_total);
        summary.margin_removed = prepared.margin_removed;
        summary.peacoqc_removed = prepared.peacoqc_removed;
        summary.matrix = prepared.matrix;
        return Ok(skipped(summary));
    }
    for warning in &prepared.warnings {
        warn!("spillqc: {}: {warning}", input.display());
    }

    let result = compensaid(&prepared.markers, &prepared.columns, &config.parameters);
    let flags = result
        .pairs
        .iter()
        .filter(|pair| pair.status.is_flag())
        .map(|pair| FlagSummary {
            primary_channel: result.channels[pair.primary].channel.clone(),
            primary_marker: result.channels[pair.primary].marker.clone(),
            secondary_channel: result.channels[pair.secondary].channel.clone(),
            secondary_marker: result.channels[pair.secondary].marker.clone(),
            ssi: pair.ssi,
            status: pair.status,
        })
        .collect::<Vec<_>>();
    info!(
        "spillqc: {}: {} events, {} pairs assessed, {} flagged",
        input.display(),
        events_assessed,
        result
            .pairs
            .iter()
            .filter(|pair| pair.ssi.is_some())
            .count(),
        flags.len()
    );
    let report = file_report(name, &result, &prepared.columns, with_dot_plots);
    let summary = FileSummary {
        input: input.display().to_string(),
        name: name.to_owned(),
        status: FileStatus::Assessed,
        reason: None,
        events_total: prepared.events_total,
        margin_removed: prepared.margin_removed,
        peacoqc_removed: prepared.peacoqc_removed,
        events_assessed,
        matrix: prepared.matrix,
        logicle: prepared.logicle,
        center: Some(result.center),
        separation: Some(result.separation),
        channels: result
            .channels
            .iter()
            .zip(&result.cutoffs)
            .map(|(channel, cutoff)| ChannelSummary {
                channel: channel.channel.clone(),
                marker: channel.marker.clone(),
                cutoff: cutoff.cutoff,
                candidates: cutoff.candidates.clone(),
            })
            .collect(),
        pairs_assessed: result
            .pairs
            .iter()
            .filter(|pair| pair.ssi.is_some())
            .count(),
        flags,
        warnings: prepared.warnings,
    };
    Ok(Assessed {
        summary,
        result: Some(result),
        report: Some(report),
    })
}

fn validate(config: &SpillQcConfig) -> Result<(), SpillQcError> {
    let invalid = |name, reason: &str| {
        Err(SpillQcError::InvalidParameter {
            name,
            reason: reason.to_owned(),
        })
    };
    if config.parameters.segments == 0 {
        return invalid("segments", "must be at least 1");
    }
    if config.parameters.min_events == 0 {
        return invalid("min-events", "must be at least 1");
    }
    if config.batch.min_files == 0 {
        return invalid("batch-min-files", "must be at least 1");
    }
    let fraction = config.batch.min_fraction;
    if !(fraction > 0.0 && fraction <= 1.0) {
        return invalid("batch-min-fraction", "must be in (0, 1]");
    }
    Ok(())
}

fn load_dataset(path: &Path) -> Result<FcsDataset, SpillQcError> {
    FcsReader::open(path)
        .and_then(FcsReader::read)
        .map_err(|source| SpillQcError::FcsRead {
            path: path.to_path_buf(),
            source,
        })
}

/// One output directory per input, named after the file stem.
fn output_names(inputs: &[PathBuf]) -> Result<Vec<String>, SpillQcError> {
    let mut seen: HashMap<String, &PathBuf> = HashMap::new();
    let mut names = Vec::with_capacity(inputs.len());
    for input in inputs {
        let name = input.file_stem().map_or_else(
            || "sample".to_owned(),
            |stem| stem.to_string_lossy().into_owned(),
        );
        if let Some(first) = seen.insert(name.clone(), input) {
            return Err(SpillQcError::DuplicateStem {
                stem: name,
                first: first.clone(),
                second: input.clone(),
            });
        }
        names.push(name);
    }
    Ok(names)
}

/// Create the output directory. An existing non-empty directory needs
/// `overwrite`, which removes only what this run writes.
fn prepare_output(output: &Path, names: &[String], overwrite: bool) -> Result<(), SpillQcError> {
    let write_error = |source| SpillQcError::Write {
        path: output.to_path_buf(),
        source,
    };
    if output.exists() {
        if !output.is_dir() {
            return Err(SpillQcError::OutputNotDirectory {
                path: output.to_path_buf(),
            });
        }
        let occupied = fs::read_dir(output).map_err(write_error)?.next().is_some();
        if occupied && !overwrite {
            return Err(SpillQcError::OutputExists {
                path: output.to_path_buf(),
            });
        }
        for name in names {
            let directory = output.join(name);
            if directory.is_dir() {
                fs::remove_dir_all(&directory).map_err(|source| SpillQcError::Write {
                    path: directory.clone(),
                    source,
                })?;
            }
        }
        for file in [SUMMARY_FILE, BATCH_FILE, REPORT_FILE] {
            let path = output.join(file);
            if path.is_file() {
                fs::remove_file(&path).map_err(|source| SpillQcError::Write { path, source })?;
            }
        }
    }
    fs::create_dir_all(output).map_err(write_error)
}

fn resolve_inputs(inputs: &[PathBuf]) -> Result<Vec<PathBuf>, SpillQcError> {
    let mut paths = Vec::new();
    for input in inputs {
        paths.extend(resolve_input(input)?);
    }
    if paths.is_empty() {
        return Err(SpillQcError::EmptyInputs);
    }
    let mut seen = HashSet::new();
    for path in &paths {
        let canonical =
            fs::canonicalize(path).map_err(|source| SpillQcError::InputCanonicalize {
                path: path.clone(),
                source,
            })?;
        if !seen.insert(canonical) {
            return Err(SpillQcError::DuplicateInput { path: path.clone() });
        }
    }
    Ok(paths)
}

fn resolve_input(input: &Path) -> Result<Vec<PathBuf>, SpillQcError> {
    let raw = input.to_string_lossy();
    if raw.contains('*') || raw.contains('?') {
        return resolve_glob(input, &raw);
    }
    if !input.exists() {
        return Err(SpillQcError::MissingInput {
            path: input.to_path_buf(),
        });
    }
    if input.is_dir() {
        let paths = fcs_files_in(input)?;
        if paths.is_empty() {
            return Err(SpillQcError::EmptyDirectory {
                path: input.to_path_buf(),
            });
        }
        return Ok(paths);
    }
    if is_fcs_path(input) {
        return Ok(vec![input.to_path_buf()]);
    }
    Err(SpillQcError::NonFcsInput {
        path: input.to_path_buf(),
    })
}

fn resolve_glob(input: &Path, raw: &str) -> Result<Vec<PathBuf>, SpillQcError> {
    let unsupported = || SpillQcError::UnsupportedGlob {
        pattern: raw.to_owned(),
    };
    let pattern = input
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(unsupported)?;
    let parent = input
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_text = parent.to_string_lossy();
    if parent_text.contains('*') || parent_text.contains('?') {
        return Err(unsupported());
    }
    let matches = fcs_files_in(parent)?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| wildcard_match(pattern.as_bytes(), name.as_bytes()))
        })
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Err(SpillQcError::EmptyGlob {
            pattern: raw.to_owned(),
        });
    }
    Ok(matches)
}

fn fcs_files_in(directory: &Path) -> Result<Vec<PathBuf>, SpillQcError> {
    let mut paths = fs::read_dir(directory)
        .map_err(|source| SpillQcError::DirectoryRead {
            path: directory.to_path_buf(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_fcs_path(path))
        .collect::<Vec<_>>();
    paths.sort();
    Ok(paths)
}

fn is_fcs_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("fcs"))
}

/// `*` matches any run of characters, `?` one character.
fn wildcard_match(pattern: &[u8], value: &[u8]) -> bool {
    match (pattern.first(), value.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            wildcard_match(&pattern[1..], value)
                || (!value.is_empty() && wildcard_match(pattern, &value[1..]))
        }
        (Some(b'?'), Some(_)) => wildcard_match(&pattern[1..], &value[1..]),
        (Some(expected), Some(actual)) if expected == actual => {
            wildcard_match(&pattern[1..], &value[1..])
        }
        _ => false,
    }
}

fn with_thread_pool<T: Send>(
    threads: Option<usize>,
    operation: impl FnOnce() -> Result<T, SpillQcError> + Send,
) -> Result<T, SpillQcError> {
    let Some(threads) = threads else {
        return operation();
    };
    debug!("using local rayon thread pool with {threads} threads");
    ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .map_err(|source| SpillQcError::ThreadPoolBuild { threads, source })?
        .install(operation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcards_match_file_names() {
        assert!(wildcard_match(b"*.fcs", b"a.fcs"));
        assert!(wildcard_match(b"tube_?.fcs", b"tube_3.fcs"));
        assert!(!wildcard_match(b"tube_?.fcs", b"tube_10.fcs"));
    }

    #[test]
    fn inputs_sharing_a_stem_are_rejected() {
        let inputs = vec![PathBuf::from("a/x.fcs"), PathBuf::from("b/x.fcs")];
        assert!(matches!(
            output_names(&inputs),
            Err(SpillQcError::DuplicateStem { .. })
        ));
    }

    #[test]
    fn batch_fraction_must_be_a_share() {
        let mut config = SpillQcConfig::new(vec![PathBuf::from("x.fcs")], PathBuf::from("out"));
        config.batch.min_fraction = 0.0;
        assert!(validate(&config).is_err());
        config.batch.min_fraction = 1.0;
        assert!(validate(&config).is_ok());
    }
}
