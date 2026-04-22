//! `cytiq-autospill` implements a narrow Rust port of the core `AutoSpill`
//! workflow for square compensation matrices.
//!
//! Primary reference:
//! <https://www.nature.com/articles/s41467-021-23126-8>
//!
//! Upstream R reference implementation:
//! <https://github.com/carlosproca/autospill>
//!
//! The goal of this crate is not to introduce Rust-specific tuning, but to keep
//! the same high-level stages and defaults visible in code:
//! FSC/SSC auto-gating, robust spillover estimation, iterative refinement, and
//! matrix export.

mod error;
mod export;
mod gating;
mod manifest;
mod regression;
mod runner;

pub use crate::error::AutospillError;
pub use crate::export::export_matrix;
pub use crate::manifest::{ControlsManifestRow, read_controls_manifest};
pub use crate::runner::{
    AutospillConfig, AutospillResult, AutospillRunner, ConvergenceStep, ConvergenceSummary,
    GateStats, MatrixKind,
};
