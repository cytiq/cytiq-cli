//! Per-event masks shared by cytiq stages.
//!
//! A mask is a Parquet file with one row per event of the file it was computed
//! on:
//!
//! | column            | type            | meaning                                   |
//! | ----------------- | --------------- | ----------------------------------------- |
//! | `event_id`        | `UInt64`        | 0-based row in that file                  |
//! | `source_event_id` | `UInt64`        | 0-based row in the raw file               |
//! | filter columns    | nullable `Bool` | pass/fail; null means "not assessed"      |
//! | `keep`            | `Bool`          | whether later stages should use the event |
//!
//! `source_event_id` lets a mask computed on one file apply to any file derived
//! from the same raw acquisition.

mod error;
mod mask;
mod runner;

pub use crate::error::MaskError;
pub use crate::mask::{
    EVENT_ID_COLUMN, EventMask, KEEP_COLUMN, SOURCE_EVENT_ID_COLUMN, read_combined,
};
pub use crate::runner::{MaskCombineConfig, MaskCombineRunner};
