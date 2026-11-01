//! All-or-nothing publication of command outputs.
//!
//! A command stages every output next to its target, writes and syncs it, and
//! publishes the set with one [`Publication::commit`]. Until then nothing is
//! visible at a target path: an error, a panic or a dropped publication leaves
//! the previous outputs and no temporary files behind.
//!
//! ```no_run
//! use cytiq_publish::{Existing, Publication, PublishError};
//! use std::io::Write;
//!
//! # fn main() -> Result<(), PublishError> {
//! let mut publication = Publication::new(Existing::Refuse);
//! publication.stage_file("out/data.csv", |w| -> Result<(), PublishError> {
//!     writeln!(w, "a,b").map_err(|e| PublishError::io("out/data.csv", e))
//! })?;
//! publication.stage_file("out/provenance.json", |w| -> Result<(), PublishError> {
//!     w.write_all(b"{}").map_err(|e| PublishError::io("out/provenance.json", e))
//! })?;
//! publication.commit()
//! # }
//! ```

mod error;
mod publication;

pub use crate::error::PublishError;
pub use crate::publication::{Existing, Publication, Staging, write_file};
