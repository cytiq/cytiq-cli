//! Population-specific batch normalization, independently implemented from
//! `CytoNorm` (2020, doi:10.1002/cyto.a.23904) and `CytoNorm` 2.0 (2025,
//! doi:10.1002/cyto.a.24910). No R runtime is needed for training or inference.
mod bundle;
mod manifest;
mod runner;
mod spline;

pub use bundle::{CytoNormModel, Goal, Mapping, MarkerContract, ModelDocument};
pub use manifest::{
    Manifest, Marker, Mode, Panel, Role, Sample, SomSettings, SparsePolicy, Target,
};
pub use runner::{ApplyConfig, CytoNormRunner, RunResult, TrainConfig};
pub use spline::MonotoneSpline;

pub const UPSTREAM: &[(&str, &str)] = &[
    ("CytoNorm", "2.0.12"),
    ("FlowSOM", "2.18.0"),
    ("ConsensusClusterPlus", "1.74.0"),
];
pub const DEFAULT_TRAINING_EVENTS: usize = 1_000_000;
pub const DEFAULT_MIN_CLUSTER_EVENTS: usize = 100;

#[derive(Debug, thiserror::Error)]
pub enum CytoNormError {
    #[error("invalid CytoNorm input: {0}")]
    Invalid(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("FCS: {0}")]
    Fcs(#[from] cytiq_fcs::Error),
    #[error("FlowSOM: {0}")]
    FlowSom(#[from] cytiq_flowsom::FlowSomError),
    #[error("event mask: {0}")]
    Mask(#[from] cytiq_mask::MaskError),
    #[error("Parquet: {0}")]
    Parquet(#[from] parquet::errors::ParquetError),
    #[error("Arrow: {0}")]
    Arrow(#[from] arrow_schema::ArrowError),
    #[error("{0}")]
    Publish(#[from] cytiq_publish::PublishError),
}

pub(crate) type Result<T> = std::result::Result<T, CytoNormError>;
pub(crate) fn invalid(message: impl Into<String>) -> CytoNormError {
    CytoNormError::Invalid(message.into())
}
