//! `cytiq preflight`: check a run manifest and its FCS files before any
//! computation — files readable and raw, each sample's panel markers present
//! and unambiguous, one detector configuration per panel, and a usable
//! calibration: the samples' spillover, an external matrix or single-stain
//! controls for conventional panels, controls for spectral ones.

mod checks;
mod error;
mod runner;

pub use crate::checks::{Check, CheckStatus, SPILLOVER_CONDITION_WARN};
pub use crate::error::PreflightError;
pub use crate::runner::{PanelSummary, PreflightConfig, PreflightReport, PreflightRunner};
