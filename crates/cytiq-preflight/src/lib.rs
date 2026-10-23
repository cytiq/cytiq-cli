//! `cytiq preflight`: check a run manifest and its FCS files before any
//! computation — files readable and raw, panel markers present and
//! unambiguous, one detector configuration across samples and controls, and
//! a usable spillover matrix for conventional runs.

mod checks;
mod error;
mod runner;

pub use crate::checks::{Check, CheckStatus, SPILLOVER_CONDITION_WARN};
pub use crate::error::PreflightError;
pub use crate::runner::{PreflightConfig, PreflightReport, PreflightRunner};
