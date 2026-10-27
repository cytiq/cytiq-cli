//! `cytiq-spillqc` checks the spillover matrix applied to fully stained
//! samples with `CompensAID`'s Secondary Stain Index, and summarizes the flags
//! across files compensated with the same matrix.
//!
//! Primary reference: Olsman et al., "`CompensAID`: An Automated Detection Tool
//! for Reference Errors", Cytometry A 109(2):124-134 (2026),
//! <https://doi.org/10.1002/cyto.a.70016>.
//!
//! Upstream R reference implementation:
//! <https://github.com/Olsman/CompensAID>

mod batch;
mod compensaid;
mod error;
mod export;
mod preprocess;
mod report;
mod runner;

pub use crate::batch::{
    BatchRow, BatchSettings, BatchStatus, DEFAULT_BATCH_MIN_FILES, DEFAULT_BATCH_MIN_FRACTION,
    summarize,
};
pub use crate::compensaid::{
    ChannelCutoff, CompensaidParameters, CompensaidResult, DEFAULT_MIN_EVENTS, DEFAULT_SEGMENTS,
    FLAG_THRESHOLD, MarkerChannel, PairAssessment, SsiSegment, SsiStatus, compensaid,
};
pub use crate::error::SpillQcError;
pub use crate::export::{
    BATCH_FILE, CUTOFFS_FILE, REPORT_FILE, SSI_MATRIX_FILE, SSI_SEGMENTS_FILE, SUMMARY_FILE,
};
pub use crate::preprocess::{LogicleRecord, MatrixRecord, PreprocessOptions};
pub use crate::runner::{
    ChannelSummary, FileStatus, FileSummary, FlagSummary, SpillQcConfig, SpillQcCounts,
    SpillQcParameters, SpillQcResult, SpillQcRunner, SpillQcSummary,
};

/// Upstream implementations this crate is validated against
/// (`parity/versions.txt`), recorded in provenance.
pub const UPSTREAM: &[(&str, &str)] = &[
    ("CompensAID", "0.99.7@db74803"),
    ("flowDensity", "1.44.0"),
    ("flowCore", "2.22.1"),
    ("PeacoQC", "1.20.0"),
];
