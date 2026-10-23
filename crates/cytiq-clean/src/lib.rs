//! `cytiq-clean` removes debris, doublets and dead cells from an FCS file.
//!
//! Three independent filters each produce a gate, fitted from the data:
//!
//! - **debris**: FSC-A threshold at the density valley above the low-FSC
//!   debris peak (or a population auto-gate, or fixed thresholds);
//! - **singlets**: `FSC-A / FSC-H` ratio within `median ± k·MAD` (or an
//!   FSC-W upper bound);
//! - **viability**: threshold on a viability dye, from the density valley
//!   between dim (live) and bright (dead) events or from a control file.
//!
//! A filter that cannot be evaluated reliably (for example viability without
//! a viability dye in the panel) reports `not_assessed` instead of passing
//! every event. Events are never copied: the output is a per-event mask keyed
//! by the original row index, plus the gates as `GatingML` 2.0 so they can
//! be edited by hand and re-applied with `--gates`.
//!
//! The debris population auto-gate is the `AutoSpill` FSC/SSC gate:
//! <https://www.nature.com/articles/s41467-021-23126-8>

mod channels;
mod debris;
mod error;
mod export;
mod filter;
mod report;
mod runner;
mod singlets;
mod viability;

pub use crate::channels::{ChannelOverrides, ChannelRoles, is_viability_parameter};
pub use crate::debris::{DEBRIS_MAX_PEAK_POSITION_RATIO, DebrisConfig, DebrisMode};
pub use crate::error::CleanError;
pub use crate::export::{GATINGML_FILE, MASKS_FILE, METRICS_FILE, REPORT_FILE};
pub use crate::filter::{
    CLEAN_GATE, FilterReport, FilterStatus, LIVE_GATE, MIN_FIT_EVENTS, NON_DEBRIS_GATE,
    SINGLETS_GATE,
};
pub use crate::runner::{
    CleanConfig, CleanMasks, CleanMetrics, CleanResult, CleanRunner, DEFAULT_FSC_A, DEFAULT_FSC_H,
    DEFAULT_FSC_W, DEFAULT_MAX_FIT_EVENTS, DEFAULT_SINGLET_MAD, DEFAULT_SSC_A, FilterReports,
};
pub use crate::viability::{
    DEFAULT_VIABILITY_COFACTOR, DEFAULT_VIABILITY_CONTROL_QUANTILE, VIABILITY_TRANSFORM,
    ViabilityConfig,
};
