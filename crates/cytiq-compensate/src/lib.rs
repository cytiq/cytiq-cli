//! Apply a spillover or compensation matrix to FCS sample events and record
//! the compensation in the output, so later stages never compensate twice.

mod error;
mod matrix;
mod runner;

pub use crate::error::CompensateError;
pub use crate::matrix::{CompensationMatrix, MatrixKind, read_matrix_file};
pub use crate::runner::{CompensateConfig, CompensateResult, CompensateRunner, masks_path};
