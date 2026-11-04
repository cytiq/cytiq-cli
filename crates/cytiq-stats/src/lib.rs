//! `cytiq-stats` summarizes every channel of an FCS file for technical QC.
//!
//! One pass over the decoded events gives, per channel, the value range and
//! distribution (min, max, mean, standard deviation, 1st/50th/99th
//! percentiles), how many events sit at the detector limit (`$PnR - 1`, the
//! saturated or margin events) or at or below zero, and a fixed-bin histogram.
//! A `Time` channel also gets its monotonicity, duration and event-rate
//! histogram. The report is raw material: thresholds and decisions belong to
//! the caller, so a stricter policy never needs the events again.

mod error;
mod runner;
mod summary;

pub use crate::error::StatsError;
pub use crate::runner::{
    ChannelKind, ChannelStats, DEFAULT_BINS, FileIdentity, Histogram, HistogramScale, StatsConfig,
    StatsReport, StatsRunner, TIME_BINS, TimeStats, read_stats,
};
