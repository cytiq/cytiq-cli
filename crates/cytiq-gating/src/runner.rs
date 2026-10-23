use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use cytiq_fcs::FcsReader;
use cytiq_mask::{EVENT_ID_COLUMN, EventMask, KEEP_COLUMN, SOURCE_EVENT_ID_COLUMN};
use log::{debug, info};
use rayon::ThreadPoolBuilder;

use crate::error::GatingError;
use crate::evaluate::{GateCount, GatingResults};
use crate::parse::parse_gatingml;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutputFormat {
    #[default]
    Csv,
    Json,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatingConfig {
    pub input: PathBuf,
    pub gating_ml: PathBuf,
    pub output: PathBuf,
    pub format: OutputFormat,
    pub threads: Option<usize>,
    /// Event masks (`masks.parquet`) restricting which events are gated;
    /// several are combined with AND.
    pub masks: Vec<PathBuf>,
    /// Optional Parquet file for per-event gate membership.
    pub membership: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GatingRunResult {
    pub event_count: usize,
    pub gates: Vec<GateCount>,
    pub output: PathBuf,
}

#[derive(Debug, Default)]
pub struct GatingRunner;

impl GatingRunner {
    /// Gate one FCS file with a `GatingML` 2.0 strategy and write per-gate counts.
    ///
    /// # Errors
    ///
    /// Returns an error when the inputs cannot be read or parsed, gating
    /// fails, or the report cannot be written.
    pub fn run(config: &GatingConfig) -> Result<GatingRunResult, GatingError> {
        if config.threads == Some(0) {
            return Err(GatingError::InvalidThreads);
        }
        match config.threads {
            Some(threads) => {
                debug!("using local rayon thread pool with {threads} threads");
                ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .map_err(|source| GatingError::ThreadPoolBuild { threads, source })?
                    .install(|| run_inner(config))
            }
            None => run_inner(config),
        }
    }
}

fn run_inner(config: &GatingConfig) -> Result<GatingRunResult, GatingError> {
    let xml =
        fs::read_to_string(&config.gating_ml).map_err(|source| GatingError::GatingMlRead {
            path: config.gating_ml.clone(),
            source,
        })?;
    let strategy = parse_gatingml(&xml)?;
    info!(
        "parsed {} gate(s) from {}",
        strategy.gates().len(),
        config.gating_ml.display()
    );

    let dataset = FcsReader::open(&config.input)
        .and_then(FcsReader::read)
        .map_err(|source| GatingError::FcsRead {
            path: config.input.clone(),
            source,
        })?;
    let mask = cytiq_mask::read_combined(&config.masks)?
        .map(|mask| mask.keep_for(&dataset))
        .transpose()?;
    if let Some(mask) = &mask {
        info!(
            "gating {} of {} event(s) inside the event mask",
            mask.iter().filter(|kept| **kept).count(),
            mask.len()
        );
    }
    let results = strategy.gate_sample_masked(&dataset, mask.as_deref())?;
    let report = results.report();

    if let Some(parent) = config
        .output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(|source| GatingError::OutputWrite {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    match config.format {
        OutputFormat::Csv => write_csv(&config.output, &report)?,
        OutputFormat::Json => write_json(&config.output, &report)?,
    }
    if let Some(path) = &config.membership {
        let source_event_ids =
            cytiq_fcs::source_event_ids(&dataset).map_err(|source| GatingError::FcsRead {
                path: config.input.clone(),
                source,
            })?;
        write_membership(path, &results, dataset.event_count(), source_event_ids)?;
    }

    Ok(GatingRunResult {
        event_count: results.event_count(),
        gates: report,
        output: config.output.clone(),
    })
}

/// One row per input event in the cytiq mask layout: a nullable Boolean
/// column per gate in evaluation order (null outside the event mask), and
/// `keep` marking events inside the event mask.
fn write_membership(
    path: &Path,
    results: &GatingResults,
    rows: usize,
    source_event_ids: Option<Vec<u64>>,
) -> Result<(), GatingError> {
    let in_mask = results
        .event_mask()
        .map_or_else(|| vec![true; rows], <[bool]>::to_vec);
    let mut mask = EventMask::new(in_mask.clone(), source_event_ids);
    for id in results.gate_ids() {
        if [EVENT_ID_COLUMN, SOURCE_EVENT_ID_COLUMN, KEEP_COLUMN].contains(&id) {
            return Err(GatingError::ReservedGateId { id: id.to_owned() });
        }
        let membership = results
            .membership(id)
            .expect("gate ids come from the results");
        let column = membership
            .iter()
            .zip(&in_mask)
            .map(|(member, kept)| kept.then_some(*member))
            .collect();
        mask = mask.with_filter(id, column);
    }
    debug!("writing gate membership to {}", path.display());
    mask.write_parquet(path)?;
    Ok(())
}

fn write_csv(path: &Path, report: &[GateCount]) -> Result<(), GatingError> {
    let csv_error = |source| GatingError::CsvWrite {
        path: path.to_path_buf(),
        source,
    };
    let mut writer = csv::Writer::from_path(path).map_err(csv_error)?;
    for row in report {
        writer.serialize(row).map_err(csv_error)?;
    }
    writer.flush().map_err(|source| GatingError::OutputWrite {
        path: path.to_path_buf(),
        source,
    })
}

fn write_json(path: &Path, report: &[GateCount]) -> Result<(), GatingError> {
    let io_error = |source| GatingError::OutputWrite {
        path: path.to_path_buf(),
        source,
    };
    let mut writer = BufWriter::new(File::create(path).map_err(io_error)?);
    serde_json::to_writer_pretty(&mut writer, report).map_err(|source| GatingError::JsonWrite {
        path: path.to_path_buf(),
        source,
    })?;
    writer.write_all(b"\n").map_err(io_error)?;
    writer.flush().map_err(io_error)
}
