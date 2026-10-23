use std::fs;
use std::path::Path;

use cytiq_fcs::FcsDataset;
use cytiq_gating::{GatingResults, GatingStrategy};
use cytiq_mask::EventMask;

use crate::error::CleanError;
use crate::filter::{LIVE_GATE, NON_DEBRIS_GATE, SINGLETS_GATE};
use crate::report::write_report_html;
use crate::runner::{CleanMasks, CleanMetrics};

pub const MASKS_FILE: &str = "masks.parquet";
pub const METRICS_FILE: &str = "metrics.json";
pub const GATINGML_FILE: &str = "gates.gatingml.xml";
pub const REPORT_FILE: &str = "report.html";

pub(crate) fn write_outputs(
    directory: &Path,
    metrics: &CleanMetrics,
    masks: &CleanMasks,
    gatingml: &str,
    strategy: &GatingStrategy,
    dataset: &FcsDataset,
    results: &GatingResults,
) -> Result<(), CleanError> {
    fs::create_dir_all(directory).map_err(|source| CleanError::OutputWrite {
        path: directory.to_path_buf(),
        source,
    })?;
    write_masks_parquet(&directory.join(MASKS_FILE), masks)?;
    write_metrics_json(&directory.join(METRICS_FILE), metrics)?;
    write_text(&directory.join(GATINGML_FILE), gatingml)?;
    write_report_html(
        &directory.join(REPORT_FILE),
        metrics,
        strategy,
        dataset,
        results,
    )
}

/// One row per input event. `source_event_id` is the raw-file row (equal to
/// `event_id` unless the input has `Original_ID`). Filter columns are null when
/// the filter was not assessed, so "not checked" is never confused with
/// "passed".
fn write_masks_parquet(path: &Path, masks: &CleanMasks) -> Result<(), CleanError> {
    let event_count = masks.keep.len();
    let optional = |mask: &Option<Vec<bool>>| match mask {
        Some(mask) => mask.iter().copied().map(Some).collect(),
        None => vec![None; event_count],
    };
    EventMask::new(
        masks.acquisition.clone(),
        masks.keep.clone(),
        masks.source_event_ids.clone(),
    )
    .with_filter(NON_DEBRIS_GATE, optional(&masks.non_debris))
    .with_filter(SINGLETS_GATE, optional(&masks.singlets))
    .with_filter(LIVE_GATE, optional(&masks.live))
    .write_parquet(path)
    .map_err(|error| CleanError::ParquetWrite {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn write_metrics_json(path: &Path, metrics: &CleanMetrics) -> Result<(), CleanError> {
    let mut bytes = serde_json::to_vec_pretty(metrics).map_err(|source| CleanError::JsonWrite {
        path: path.to_path_buf(),
        source,
    })?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(|source| CleanError::OutputWrite {
        path: path.to_path_buf(),
        source,
    })
}

pub(crate) fn write_text(path: &Path, text: &str) -> Result<(), CleanError> {
    fs::write(path, text).map_err(|source| CleanError::OutputWrite {
        path: path.to_path_buf(),
        source,
    })
}
