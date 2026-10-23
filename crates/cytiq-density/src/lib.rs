//! `cytiq-density` holds the density-estimation and scatter auto-gating
//! primitives shared by the `cytiq` workflow crates.
//!
//! - [`density`]: port of R's `stats::density.default` (Gaussian kernel,
//!   `bw.nrd0`, 512 points), as used by `PeacoQC`
//!   (<https://github.com/saeyslab/PeacoQC>).
//! - [`gate_events`]: the `AutoSpill` FSC/SSC auto-gate
//!   (<https://www.nature.com/articles/s41467-021-23126-8>).
//! - [`find_valleys`]: density valleys separating the populations of a 1-D
//!   kernel density estimate.

mod kde;
mod scatter_gate;
mod valley;

pub use crate::kde::{Density, bandwidth_nrd0, density, quantile_type7};
pub use crate::scatter_gate::{
    GateComputation, convex_hull, gate_events, mad, median, point_in_polygon,
};
pub use crate::valley::{
    DEFAULT_MAX_VALLEY_RATIO, DEFAULT_MIN_PEAK_FRACTION, Valley, find_valleys,
};
