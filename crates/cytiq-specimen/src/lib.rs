//! Specimen protocols and summary-only reconciliation.

pub mod compare;
pub mod layout;
pub mod plan;
pub mod reconcile;
pub mod runner;

pub use compare::{ComparisonPolicy, SpecimenComparison, compare};
pub use layout::{SpecimenLayout, Strategy};
pub use plan::SpecimenPlan;
pub use reconcile::{SpecimenResult, reconcile};

#[derive(Debug, thiserror::Error)]
pub enum SpecimenError {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Population(#[from] cytiq_population::PopulationError),
    #[error(transparent)]
    Manifest(#[from] cytiq_manifest::ManifestError),
    #[error(transparent)]
    Fcs(#[from] cytiq_fcs::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Provenance(#[from] cytiq_provenance::ProvenanceError),
    #[error(transparent)]
    Publish(#[from] cytiq_publish::PublishError),
}

pub type Result<T> = std::result::Result<T, SpecimenError>;

pub(crate) fn require(condition: bool, reason: impl Into<String>) -> Result<()> {
    if condition {
        Ok(())
    } else {
        Err(SpecimenError::Invalid(reason.into()))
    }
}
