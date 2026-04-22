use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AutospillError {
    #[error("failed to read controls manifest `{path}`: {source}")]
    ManifestRead {
        path: PathBuf,
        #[source]
        source: csv::Error,
    },
    #[error("controls manifest is empty")]
    EmptyManifest,
    #[error("manifest row {row} has an empty `{field}`")]
    EmptyManifestField { row: usize, field: &'static str },
    #[error("manifest row {row} must use a relative `filename`, got `{filename}`")]
    AbsoluteManifestFilename { row: usize, filename: String },
    #[error("duplicate `dye` value `{value}` in controls manifest")]
    DuplicateDye { value: String },
    #[error("duplicate `primary_channel` value `{value}` in controls manifest")]
    DuplicatePrimaryChannel { value: String },
    #[error("control file `{path}` does not exist")]
    MissingControlFile { path: PathBuf },
    #[error("failed to read FCS control `{path}`: {source}")]
    FcsRead {
        path: PathBuf,
        #[source]
        source: cytiq_fcs::Error,
    },
    #[error("control `{path}` has no events")]
    EmptyControl { path: PathBuf },
    #[error("channel `{channel}` was not found in `{path}`")]
    MissingChannel { path: PathBuf, channel: String },
    #[error("control `{path}` produced a degenerate gate: {reason}")]
    DegenerateGate { path: PathBuf, reason: &'static str },
    #[error(
        "robust regression failed for control `{path}` between `{primary_channel}` and `{secondary_channel}`"
    )]
    RegressionFailed {
        path: PathBuf,
        primary_channel: String,
        secondary_channel: String,
    },
    #[error("spillover matrix became singular during refinement")]
    SingularMatrix,
    #[error("autospill did not converge after {iterations} iterations (delta.max={delta_max:e})")]
    NonConvergent { iterations: usize, delta_max: f64 },
    #[error("matrix export format for `{path}` is not supported")]
    UnsupportedOutputFormat { path: PathBuf },
    #[error("failed to write matrix to `{path}`: {source}")]
    MatrixWrite {
        path: PathBuf,
        #[source]
        source: csv::Error,
    },
    #[error("failed to create output file `{path}`: {source}")]
    CreateOutputFile {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write parquet output `{path}`: {source}")]
    ParquetWrite {
        path: PathBuf,
        #[source]
        source: parquet::errors::ParquetError,
    },
    #[error("failed to build parquet batch for `{path}`: {message}")]
    ParquetBatch { path: PathBuf, message: String },
}
