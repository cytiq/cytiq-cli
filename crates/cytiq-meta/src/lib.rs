//! `cytiq-meta` extracts the metadata of an FCS file as a JSON document.
//!
//! The document records the file identity (size and SHA-256), the FCS
//! version, segment offsets, parameters and every TEXT keyword in file order.
//! Derived FCS files written by other `cytiq` commands keep only the keywords
//! they need, so this is the place to preserve the full source metadata.

mod error;
mod runner;

pub use crate::error::MetaError;
pub use crate::runner::{
    FcsMetadata, FileIdentity, MetaConfig, MetaRunResult, MetaRunner, ParameterMetadata,
    ParameterWidthMetadata, SegmentOffsets, read_metadata,
};
