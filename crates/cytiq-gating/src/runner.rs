use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use cytiq_fcs::FcsReader;
use log::{debug, info};
use rayon::ThreadPoolBuilder;

use crate::error::GatingError;
use crate::evaluate::GateCount;
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
    let results = strategy.gate_sample(&dataset)?;
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

    Ok(GatingRunResult {
        event_count: results.event_count(),
        gates: report,
        output: config.output.clone(),
    })
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
