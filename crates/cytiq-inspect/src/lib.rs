//! FCS inspection: metadata export and per-channel technical statistics.
//!
//! [`meta`] backs `cytiq meta`; [`stats`] backs `cytiq stats`.
//! Each module keeps its own report schema and file-identity type.
//!
//! ```no_run
//! use cytiq_inspect::{meta::read_metadata, stats::{DEFAULT_BINS, read_stats}};
//!
//! let metadata = read_metadata("raw.fcs")?;
//! let statistics = read_stats("raw.fcs", DEFAULT_BINS)?;
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod meta;
pub mod stats;
