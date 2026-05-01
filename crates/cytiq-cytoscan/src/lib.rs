//! `cytiq-cytoscan` implements CytoScan-style anomaly detection for
//! cytometry FCS file cohorts.
//!
//! Primary reference:
//! <https://doi.org/10.1101/2025.09.24.678276>
//!
//! Upstream R reference implementation:
//! <https://github.com/AUMC-HEMA/CytoScan>

mod error;
mod export;
mod runner;

pub use crate::error::CytoScanError;
pub use crate::runner::{
    CytoScanConfig, CytoScanConstants, CytoScanDiagnostics, CytoScanFeatureMatrix,
    CytoScanFeatureRow, CytoScanParameters, CytoScanResult, CytoScanRunner, CytoScanSummaryRow,
    DEFAULT_AGGREGATE_EVENTS, DEFAULT_ARCSINH_COFACTOR, DEFAULT_EVENTS_PER_FILE,
    DEFAULT_FINGERPRINT_RECURSIONS, DEFAULT_ISOLATION_TREES, DEFAULT_NOVELTY_QUANTILE,
    DEFAULT_OUTLIER_THRESHOLD, DEFAULT_QUANTILE_STEP, DEFAULT_RANDOM_SEED, FeatureMethod,
    FlagMethod, parse_channel_list,
};
