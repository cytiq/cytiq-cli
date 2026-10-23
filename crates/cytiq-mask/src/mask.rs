use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, BooleanArray, RecordBatch, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use cytiq_fcs::FcsDataset;
use log::debug;
use parquet::arrow::ArrowWriter;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::error::MaskError;

pub const EVENT_ID_COLUMN: &str = "event_id";
pub const SOURCE_EVENT_ID_COLUMN: &str = "source_event_id";
pub const KEEP_COLUMN: &str = "keep";

/// One row per event of the file the mask was computed on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventMask {
    pub event_ids: Vec<u64>,
    pub source_event_ids: Vec<u64>,
    /// Filter columns in order; `None` means the filter did not assess the
    /// event, never that it passed.
    pub filters: Vec<(String, Vec<Option<bool>>)>,
    pub keep: Vec<bool>,
}

impl EventMask {
    /// A mask over `keep.len()` events whose rows are their raw rows unless
    /// `source_event_ids` says otherwise.
    #[must_use]
    pub fn new(keep: Vec<bool>, source_event_ids: Option<Vec<u64>>) -> Self {
        let event_ids = (0..keep.len() as u64).collect::<Vec<_>>();
        Self {
            source_event_ids: source_event_ids.unwrap_or_else(|| event_ids.clone()),
            event_ids,
            filters: Vec::new(),
            keep,
        }
    }

    #[must_use]
    pub fn with_filter(mut self, name: impl Into<String>, values: Vec<Option<bool>>) -> Self {
        self.filters.push((name.into(), values));
        self
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.keep.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.keep.is_empty()
    }

    #[must_use]
    pub fn kept(&self) -> usize {
        self.keep.iter().filter(|keep| **keep).count()
    }

    fn validate(&self) -> Result<(), MaskError> {
        let expected = self.keep.len();
        let lengths = [
            (EVENT_ID_COLUMN, self.event_ids.len()),
            (SOURCE_EVENT_ID_COLUMN, self.source_event_ids.len()),
        ]
        .into_iter()
        .chain(
            self.filters
                .iter()
                .map(|(name, values)| (name.as_str(), values.len())),
        );
        for (column, actual) in lengths {
            if actual != expected {
                return Err(MaskError::LengthMismatch {
                    column: column.to_owned(),
                    expected,
                    actual,
                });
            }
        }
        Ok(())
    }

    /// Write the mask as Parquet.
    ///
    /// # Errors
    ///
    /// Returns an error when columns have different lengths or the file cannot
    /// be written.
    pub fn write_parquet(&self, path: &Path) -> Result<(), MaskError> {
        self.validate()?;
        debug!("writing event mask to {}", path.display());
        let parquet_error = |message: String| MaskError::Parquet {
            path: path.to_path_buf(),
            message,
        };
        let mut fields = vec![
            Field::new(EVENT_ID_COLUMN, DataType::UInt64, false),
            Field::new(SOURCE_EVENT_ID_COLUMN, DataType::UInt64, false),
        ];
        let mut columns: Vec<ArrayRef> = vec![
            Arc::new(UInt64Array::from(self.event_ids.clone())),
            Arc::new(UInt64Array::from(self.source_event_ids.clone())),
        ];
        for (name, values) in &self.filters {
            fields.push(Field::new(name, DataType::Boolean, true));
            columns.push(Arc::new(BooleanArray::from(values.clone())));
        }
        fields.push(Field::new(KEEP_COLUMN, DataType::Boolean, false));
        columns.push(Arc::new(BooleanArray::from(self.keep.clone())));
        let schema = Arc::new(Schema::new(fields));
        let batch = RecordBatch::try_new(Arc::clone(&schema), columns)
            .map_err(|error| parquet_error(error.to_string()))?;

        let file = File::create(path).map_err(|source| MaskError::Io {
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

    /// Read a mask written by any cytiq stage.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read, lacks `source_event_id`
    /// or `keep`, has non-boolean filter columns, or repeats a source event.
    pub fn read_parquet(path: &Path) -> Result<Self, MaskError> {
        debug!("reading event mask from {}", path.display());
        let invalid = |reason: String| MaskError::Invalid {
            path: path.to_path_buf(),
            reason,
        };
        let parquet_error = |message: String| MaskError::Parquet {
            path: path.to_path_buf(),
            message,
        };
        let file = File::open(path).map_err(|source| MaskError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let reader = ParquetRecordBatchReaderBuilder::try_new(file)
            .and_then(ParquetRecordBatchReaderBuilder::build)
            .map_err(|error| parquet_error(error.to_string()))?;

        let mut event_ids = Vec::new();
        let mut source_event_ids = Vec::new();
        let mut keep = Vec::new();
        let mut filters: Vec<(String, Vec<Option<bool>>)> = Vec::new();
        let mut has_event_ids = false;
        for batch in reader {
            let batch = batch.map_err(|error| parquet_error(error.to_string()))?;
            let schema = batch.schema();
            for (field, column) in schema.fields().iter().zip(batch.columns()) {
                let name = field.name().as_str();
                match name {
                    EVENT_ID_COLUMN | SOURCE_EVENT_ID_COLUMN => {
                        let values = column
                            .as_any()
                            .downcast_ref::<UInt64Array>()
                            .filter(|values| values.null_count() == 0)
                            .ok_or_else(|| {
                                invalid(format!("`{name}` must be a non-null UInt64 column"))
                            })?;
                        if name == EVENT_ID_COLUMN {
                            has_event_ids = true;
                            event_ids.extend(values.values().iter());
                        } else {
                            source_event_ids.extend(values.values().iter());
                        }
                    }
                    _ => {
                        let values = column
                            .as_any()
                            .downcast_ref::<BooleanArray>()
                            .ok_or_else(|| invalid(format!("`{name}` must be a Boolean column")))?;
                        if name == KEEP_COLUMN {
                            if values.null_count() > 0 {
                                return Err(invalid("`keep` must not contain nulls".to_owned()));
                            }
                            keep.extend(values.values().iter());
                        } else {
                            let position = filters
                                .iter()
                                .position(|(existing, _)| existing == name)
                                .unwrap_or_else(|| {
                                    filters.push((name.to_owned(), Vec::new()));
                                    filters.len() - 1
                                });
                            filters[position].1.extend(values.iter());
                        }
                    }
                }
            }
        }

        if keep.is_empty() && source_event_ids.is_empty() {
            return Err(invalid(
                "the mask has no `keep` and `source_event_id` rows".to_owned(),
            ));
        }
        if !has_event_ids {
            event_ids = (0..keep.len() as u64).collect();
        }
        let mask = Self {
            event_ids,
            source_event_ids,
            filters,
            keep,
        };
        mask.validate()
            .map_err(|error| invalid(error.to_string()))?;
        let mut seen = HashSet::with_capacity(mask.len());
        if let Some(duplicate) = mask
            .source_event_ids
            .iter()
            .find(|source_event_id| !seen.insert(**source_event_id))
        {
            return Err(invalid(format!(
                "`source_event_id` {duplicate} appears more than once"
            )));
        }
        Ok(mask)
    }

    /// Combine masks of the same acquisition into one keyed by
    /// `source_event_id`. An event is kept only when every mask has it and
    /// keeps it. Rows follow the first mask; `event_id` restarts at 0. Filter
    /// columns are concatenated, and a repeated name is prefixed with the
    /// 1-based index of its mask (`2.keep_margin`).
    ///
    /// # Errors
    ///
    /// Returns an error when `masks` is empty.
    pub fn combine(masks: &[Self]) -> Result<Self, MaskError> {
        let Some(first) = masks.first() else {
            return Err(MaskError::Empty);
        };
        let lookups = masks.iter().map(Self::row_lookup).collect::<Vec<_>>();
        let rows = &first.source_event_ids;
        let keep = rows
            .iter()
            .map(|source_event_id| {
                masks.iter().zip(&lookups).all(|(mask, lookup)| {
                    lookup
                        .get(source_event_id)
                        .is_some_and(|row| mask.keep[*row])
                })
            })
            .collect::<Vec<_>>();

        let mut combined = Self::new(keep, Some(rows.clone()));
        let mut names = HashSet::new();
        for (index, (mask, lookup)) in masks.iter().zip(&lookups).enumerate() {
            for (name, values) in &mask.filters {
                let name = if names.insert(name.clone()) {
                    name.clone()
                } else {
                    format!("{}.{name}", index + 1)
                };
                let column = rows
                    .iter()
                    .map(|source_event_id| lookup.get(source_event_id).and_then(|row| values[*row]))
                    .collect();
                combined.filters.push((name, column));
            }
        }
        Ok(combined)
    }

    /// `keep` for each row of `dataset`, matched through `Original_ID` when the
    /// dataset has one and by row otherwise.
    ///
    /// # Errors
    ///
    /// Returns an error when `Original_ID` is invalid or the mask has no row
    /// for one of the dataset's events.
    pub fn keep_for(&self, dataset: &FcsDataset) -> Result<Vec<bool>, MaskError> {
        let lookup = self.row_lookup();
        let source_event_ids = cytiq_fcs::source_event_ids(dataset)
            .map_err(MaskError::Lineage)?
            .unwrap_or_else(|| (0..dataset.event_count() as u64).collect());
        source_event_ids
            .iter()
            .map(|source_event_id| {
                lookup
                    .get(source_event_id)
                    .map(|row| self.keep[*row])
                    .ok_or(MaskError::MissingEvent {
                        source_event_id: *source_event_id,
                    })
            })
            .collect()
    }

    fn row_lookup(&self) -> HashMap<u64, usize> {
        self.source_event_ids
            .iter()
            .enumerate()
            .map(|(row, source_event_id)| (*source_event_id, row))
            .collect()
    }
}

/// Read and combine one or more masks; `None` when `paths` is empty.
///
/// # Errors
///
/// Returns an error when any mask cannot be read.
pub fn read_combined(paths: &[PathBuf]) -> Result<Option<EventMask>, MaskError> {
    if paths.is_empty() {
        return Ok(None);
    }
    let masks = paths
        .iter()
        .map(|path| EventMask::read_parquet(path))
        .collect::<Result<Vec<_>, _>>()?;
    EventMask::combine(&masks).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "cytiq-mask-{}-{}",
            std::process::id(),
            name.replace('.', "-")
        ));
        std::fs::create_dir_all(&directory).unwrap();
        directory.join(name)
    }

    #[test]
    fn round_trips_parquet() {
        let path = temp_path("round-trip.parquet");
        let mask = EventMask::new(vec![true, false, true], Some(vec![4, 7, 9]))
            .with_filter("margin_pass", vec![Some(true), Some(false), None]);
        mask.write_parquet(&path).unwrap();
        assert_eq!(EventMask::read_parquet(&path).unwrap(), mask);
    }

    #[test]
    fn combine_ands_keep_by_source_event() {
        let first = EventMask::new(vec![true, true, false, true], None)
            .with_filter("margin_pass", vec![Some(true); 4]);
        // Derived file: raw events 1 and 3 only, the second one removed.
        let second = EventMask::new(vec![true, false], Some(vec![1, 3]))
            .with_filter("margin_pass", vec![Some(true), Some(false)]);

        let combined = EventMask::combine(&[first, second]).unwrap();

        assert_eq!(combined.source_event_ids, vec![0, 1, 2, 3]);
        assert_eq!(combined.keep, vec![false, true, false, false]);
        assert_eq!(combined.filters[0].0, "margin_pass");
        assert_eq!(combined.filters[1].0, "2.margin_pass");
        assert_eq!(
            combined.filters[1].1,
            vec![None, Some(true), None, Some(false)]
        );
    }

    #[test]
    fn rejects_duplicate_source_events() {
        let path = temp_path("duplicate.parquet");
        EventMask::new(vec![true, true], Some(vec![5, 5]))
            .write_parquet(&path)
            .unwrap();
        let error = EventMask::read_parquet(&path).unwrap_err();
        assert!(error.to_string().contains("more than once"), "{error}");
    }

    fn dataset(original_ids: Option<Vec<f32>>) -> FcsDataset {
        let mut parameters = vec![cytiq_fcs::FcsWriteParameter::new("FL1-A")];
        let mut columns = vec![vec![1.0_f32, 2.0, 3.0]];
        if let Some(ids) = original_ids {
            parameters.push(cytiq_fcs::FcsWriteParameter::new("Original_ID"));
            columns.push(ids);
        }
        let mut bytes = Vec::new();
        cytiq_fcs::write_fcs3_1_f32_to_writer(&mut bytes, &parameters, &columns).unwrap();
        cytiq_fcs::FcsReader::new(std::io::Cursor::new(bytes))
            .read()
            .unwrap()
    }

    #[test]
    fn keep_for_follows_original_id() {
        let raw_mask = EventMask::new(vec![true, false, true, false, true], None);
        // A derived file holding raw rows 0, 2 and 3 (Original_ID is 1-based).
        let derived = dataset(Some(vec![1.0, 3.0, 4.0]));
        assert_eq!(
            raw_mask.keep_for(&derived).unwrap(),
            vec![true, true, false]
        );
        assert_eq!(
            raw_mask.keep_for(&dataset(None)).unwrap(),
            vec![true, false, true]
        );
    }

    #[test]
    fn keep_for_rejects_mask_of_another_acquisition() {
        let mask = EventMask::new(vec![true, true], None);
        assert!(matches!(
            mask.keep_for(&dataset(None)),
            Err(MaskError::MissingEvent { source_event_id: 2 })
        ));
    }

    #[test]
    fn rejects_mismatched_column_lengths() {
        let mask = EventMask::new(vec![true, true], None).with_filter("live", vec![None]);
        assert!(matches!(
            mask.write_parquet(&temp_path("mismatch.parquet")),
            Err(MaskError::LengthMismatch { .. })
        ));
    }
}
