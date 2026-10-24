//! `cytiq qc`: check one sample's final event mask against its panel's QC
//! policy from the run manifest — raw and kept event counts, the overall
//! fraction removed, and per filter whether it ran and how much it removed.
//!
//! Masks are the common contract of the preprocessing stages, so the checks
//! read the combined mask (`cytiq mask combine`) instead of each stage's own
//! report.

mod error;
mod policy;
mod runner;

pub use crate::error::QcError;
pub use crate::policy::{CheckStatus, FilterMetrics, QcCheck, QcMetrics, evaluate};
pub use crate::runner::{QcConfig, QcReport, QcRunner};
