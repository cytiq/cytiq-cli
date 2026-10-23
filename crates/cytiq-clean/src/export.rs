use std::fs::{self, File};
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, BooleanArray, RecordBatch, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use cytiq_fcs::FcsDataset;
use cytiq_gating::{GatingResults, GatingStrategy};
use log::debug;
use parquet::arrow::ArrowWriter;

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
    debug!("writing event masks to {}", path.display());
    let parquet_error = |message: String| CleanError::ParquetWrite {
        path: path.to_path_buf(),
        message,
    };
    let schema = Arc::new(Schema::new(vec![
        Field::new("event_id", DataType::UInt64, false),
        Field::new("source_event_id", DataType::UInt64, false),
        Field::new(NON_DEBRIS_GATE, DataType::Boolean, true),
        Field::new(SINGLETS_GATE, DataType::Boolean, true),
        Field::new(LIVE_GATE, DataType::Boolean, true),
        Field::new("keep", DataType::Boolean, false),
    ]));
    let event_count = masks.keep.len();
    let optional = |mask: &Option<Vec<bool>>| -> ArrayRef {
        Arc::new(match mask {
            Some(mask) => BooleanArray::from(mask.clone()),
            None => BooleanArray::new_null(event_count),
        })
    };
    let event_ids = (0..event_count as u64).collect::<Vec<_>>();
    let source_event_ids = masks
        .source_event_ids
        .clone()
        .unwrap_or_else(|| event_ids.clone());
    let columns: Vec<ArrayRef> = vec![
        Arc::new(UInt64Array::from(event_ids)),
        Arc::new(UInt64Array::from(source_event_ids)),
        optional(&masks.non_debris),
        optional(&masks.singlets),
        optional(&masks.live),
        Arc::new(BooleanArray::from(masks.keep.clone())),
    ];
    let batch = RecordBatch::try_new(Arc::clone(&schema), columns)
        .map_err(|error| parquet_error(error.to_string()))?;

    let file = File::create(path).map_err(|source| CleanError::OutputWrite {
        path: path.to_path_buf(),
        source,
    })?;
    let mut writer = ArrowWriter::try_new(file, schema, None)
        .map_err(|error| parquet_error(error.to_string()))?;
    writer
        .write(&batch)
        .map_err(|error| parquet_error(error.to_string()))?;
    writer
        .close()
        .map_err(|error| parquet_error(error.to_string()))?;
    Ok(())
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
