//! `cytiq preflight`: check a run manifest and its FCS files before any
//! computation — files readable and raw, each sample's panel markers present
//! and unambiguous, one detector configuration per panel, and a usable
//! calibration: the samples' spillover, an external matrix or single-stain
//! controls for conventional panels, controls for spectral ones.
//!
//! The finding status is the same type as the root QC status:
//!
//! ```
//! let status: cytiq_qc::CheckStatus = cytiq_qc::preflight::CheckStatus::Pass;
//! assert_eq!(serde_json::to_string(&status).unwrap(), "\"pass\"");
//! ```

mod checks;
mod error;
mod runner;

pub use self::checks::{Check, SPILLOVER_CONDITION_WARN};
pub use self::error::PreflightError;
pub use self::runner::{PanelSummary, PreflightConfig, PreflightReport, PreflightRunner};
pub use crate::CheckStatus;
