//! Long Arrow tables with exact `UInt64` counts and typed metadata.

use crate::{PopulationObservation, Result, require};
use arrow_array::{Array, ArrayRef, Float64Array, RecordBatch, StringArray, UInt64Array};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder};
use serde_json::{Map, Value};
use std::{collections::HashMap, fs::File, path::Path, sync::Arc};

const TEXT: &[&str] = &[
    "observation_id",
    "specimen_id",
    "panel_id",
    "acquisition_id",
    "source_id",
    "target_id",
    "target_kind",
    "measure",
    "unit",
    "status",
    "reason",
];
const JSON: &[&str] = &["target", "denominator", "method", "lineage"];
const COUNTS: &[&str] = &["value_u64", "n_events", "supporting_count"];
pub const MAX_OBSERVATIONS: usize = 1_000_000;

/// # Errors
/// Rejects invalid observations and failed Arrow/filesystem writes.
pub fn write(path: &Path, observations: &[PopulationObservation]) -> Result<()> {
    require(
        observations.len() <= MAX_OBSERVATIONS,
        "too many observations",
    )?;
    let mut fields = vec![Field::new("schema_version", DataType::UInt64, false)];
    fields.extend(
        TEXT.iter()
            .chain(JSON)
            .map(|name| Field::new(*name, DataType::Utf8, true)),
    );
    fields.extend(
        COUNTS
            .iter()
            .map(|name| Field::new(*name, DataType::UInt64, true)),
    );
    fields.push(Field::new("value_f64", DataType::Float64, true));
    let schema = Arc::new(Schema::new_with_metadata(
        fields,
        HashMap::from([("cytiq.population.schema_version".into(), "1".into())]),
    ));
    let mut writer = ArrowWriter::try_new(File::create(path)?, schema.clone(), None)?;
    for rows in observations.chunks(crate::DEFAULT_BATCH_SIZE) {
        for row in rows {
            row.validate()?;
        }
        let values = rows
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut columns: Vec<ArrayRef> = vec![Arc::new(UInt64Array::from(vec![1; rows.len()]))];
        for name in TEXT {
            let data = values
                .iter()
                .map(|v| match *name {
                    "target_id" => v["target"]["id"].as_str(),
                    "target_kind" => v["target"]["kind"].as_str(),
                    _ => v[*name].as_str(),
                })
                .collect::<Vec<_>>();
            columns.push(Arc::new(StringArray::from(data)));
        }
        for name in JSON {
            let data = values
                .iter()
                .map(|v| serde_json::to_string(&v[*name]))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            columns.push(Arc::new(StringArray::from(data)));
        }
        for name in COUNTS {
            columns.push(Arc::new(UInt64Array::from(
                values.iter().map(|v| v[*name].as_u64()).collect::<Vec<_>>(),
            )));
        }
        columns.push(Arc::new(Float64Array::from(
            rows.iter().map(|r| r.value_f64).collect::<Vec<_>>(),
        )));
        writer.write(&RecordBatch::try_new(schema.clone(), columns)?)?;
    }
    writer.close()?;
    Ok(())
}

/// # Errors
/// Rejects missing/changed schemas, inconsistent identities and excessive rows.
pub fn read(path: &Path) -> Result<Vec<PopulationObservation>> {
    read_from(File::open(path)?)
}

/// [`read`] from any Parquet source, such as `bytes::Bytes` in memory.
/// # Errors
/// Rejects missing/changed schemas, inconsistent identities and excessive rows.
pub fn read_from<R: parquet::file::reader::ChunkReader + 'static>(
    reader: R,
) -> Result<Vec<PopulationObservation>> {
    let builder = ParquetRecordBatchReaderBuilder::try_new(reader)?;
    require(
        builder.schema().fields().len() == TEXT.len() + JSON.len() + COUNTS.len() + 2,
        "unexpected observation columns",
    )?;
    require(
        builder
            .schema()
            .metadata()
            .get("cytiq.population.schema_version")
            .map(String::as_str)
            == Some("1"),
        "unsupported population table schema",
    )?;
    require(
        builder.metadata().file_metadata().num_rows() >= 0
            && u64::try_from(builder.metadata().file_metadata().num_rows()).unwrap_or(u64::MAX)
                <= MAX_OBSERVATIONS as u64,
        "too many observation rows",
    )?;
    let mut result = Vec::new();
    for batch in builder.with_batch_size(crate::DEFAULT_BATCH_SIZE).build()? {
        let batch = batch?;
        for index in 0..batch.num_rows() {
            let mut row = Map::new();
            row.insert(
                "schema_version".into(),
                Value::from(uint(&batch, "schema_version", index)?.ok_or_else(|| {
                    crate::PopulationError::Invalid("null schema version".into())
                })?),
            );
            for name in TEXT
                .iter()
                .filter(|name| !["target_id", "target_kind"].contains(name))
            {
                row.insert(
                    (*name).into(),
                    string(&batch, name, index)?
                        .map_or(Value::Null, |s| Value::String(s.to_owned())),
                );
            }
            for name in JSON {
                let encoded = string(&batch, name, index)?.ok_or_else(|| {
                    crate::PopulationError::Invalid("null structured observation field".into())
                })?;
                row.insert((*name).into(), serde_json::from_str(encoded)?);
            }
            for name in COUNTS {
                row.insert(
                    (*name).into(),
                    uint(&batch, name, index)?.map_or(Value::Null, Value::from),
                );
            }
            let column = batch
                .column_by_name("value_f64")
                .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
                .ok_or_else(|| crate::PopulationError::Invalid("invalid value_f64 type".into()))?;
            let number = if column.is_null(index) {
                Value::Null
            } else {
                require(
                    column.value(index).is_finite(),
                    "nonfinite floating observation",
                )?;
                Value::from(column.value(index))
            };
            row.insert("value_f64".into(), number);
            let observation: PopulationObservation = serde_json::from_value(Value::Object(row))?;
            observation.validate()?;
            require(
                string(&batch, "target_id", index)? == Some(observation.target.id.as_str())
                    && string(&batch, "target_kind", index)?
                        == serde_json::to_value(observation.target.kind)?.as_str(),
                "target columns disagree",
            )?;
            result.push(observation);
        }
    }
    Ok(result)
}

fn string<'a>(batch: &'a RecordBatch, name: &str, row: usize) -> Result<Option<&'a str>> {
    let column = batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .ok_or_else(|| crate::PopulationError::Invalid(format!("invalid string column: {name}")))?;
    Ok((!column.is_null(row)).then(|| column.value(row)))
}

fn uint(batch: &RecordBatch, name: &str, row: usize) -> Result<Option<u64>> {
    let column = batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<UInt64Array>())
        .ok_or_else(|| crate::PopulationError::Invalid(format!("invalid UInt64 column: {name}")))?;
    Ok((!column.is_null(row)).then(|| column.value(row)))
}
