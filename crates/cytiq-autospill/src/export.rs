use std::fs::File;
use std::path::Path;
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::file::properties::WriterProperties;

use crate::error::AutospillError;
use crate::runner::{AutospillResult, MatrixKind};

/// Write the requested matrix format to `path`.
///
/// # Errors
///
/// Returns an error when the output extension is unsupported or when the file
/// cannot be written.
pub fn export_matrix(
    result: &AutospillResult,
    kind: MatrixKind,
    path: impl AsRef<Path>,
) -> Result<(), AutospillError> {
    let path = path.as_ref();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| AutospillError::UnsupportedOutputFormat {
            path: path.to_path_buf(),
        })?;

    match extension.as_str() {
        "csv" => write_delimited_matrix(result, kind, path, b','),
        "tsv" => write_delimited_matrix(result, kind, path, b'\t'),
        "parquet" => write_parquet_matrix(result, kind, path),
        _ => Err(AutospillError::UnsupportedOutputFormat {
            path: path.to_path_buf(),
        }),
    }
}

fn write_delimited_matrix(
    result: &AutospillResult,
    kind: MatrixKind,
    path: &Path,
    delimiter: u8,
) -> Result<(), AutospillError> {
    let mut writer = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .from_path(path)
        .map_err(|source| AutospillError::MatrixWrite {
            path: path.to_path_buf(),
            source,
        })?;

    let mut header = Vec::with_capacity(result.channels.len() + 1);
    header.push("channel".to_owned());
    header.extend(result.channels.iter().cloned());
    writer
        .write_record(header)
        .map_err(|source| AutospillError::MatrixWrite {
            path: path.to_path_buf(),
            source,
        })?;

    for (row_name, row) in result.channels.iter().zip(result.matrix(kind)) {
        let mut record = Vec::with_capacity(row.len() + 1);
        record.push(row_name.clone());
        record.extend(row.iter().map(|value| format!("{value:.12}")));
        writer
            .write_record(record)
            .map_err(|source| AutospillError::MatrixWrite {
                path: path.to_path_buf(),
                source,
            })?;
    }
    writer
        .flush()
        .map_err(|error| AutospillError::CreateOutputFile {
            path: path.to_path_buf(),
            source: error,
        })?;
    Ok(())
}

fn write_parquet_matrix(
    result: &AutospillResult,
    kind: MatrixKind,
    path: &Path,
) -> Result<(), AutospillError> {
    let file = File::create(path).map_err(|source| AutospillError::CreateOutputFile {
        path: path.to_path_buf(),
        source,
    })?;
    let schema = Arc::new(Schema::new(
        std::iter::once(Field::new("channel", DataType::Utf8, false))
            .chain(
                result
                    .channels
                    .iter()
                    .map(|channel| Field::new(channel, DataType::Float64, false)),
            )
            .collect::<Vec<_>>(),
    ));

    let matrix = result.matrix(kind);
    let mut columns = Vec::<ArrayRef>::with_capacity(result.channels.len() + 1);
    columns.push(Arc::new(StringArray::from(result.channels.clone())));
    for column_index in 0..result.channels.len() {
        let values = matrix
            .iter()
            .map(|row| row[column_index])
            .collect::<Vec<_>>();
        columns.push(Arc::new(Float64Array::from(values)));
    }
    let batch = RecordBatch::try_new(Arc::clone(&schema), columns).map_err(|error| {
        AutospillError::ParquetBatch {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;

    let properties = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(properties)).map_err(|source| {
        AutospillError::ParquetWrite {
            path: path.to_path_buf(),
            source,
        }
    })?;
    writer
        .write(&batch)
        .map_err(|source| AutospillError::ParquetWrite {
            path: path.to_path_buf(),
            source,
        })?;
    writer
        .close()
        .map_err(|source| AutospillError::ParquetWrite {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::runner::{AutospillResult, ConvergenceSummary, MatrixKind};

    #[test]
    fn writes_square_csv_matrix() {
        let result = sample_result();
        let path = temp_path("matrix.csv");
        super::export_matrix(&result, MatrixKind::Compensation, &path)
            .expect("csv export should succeed");

        let output = fs::read_to_string(&path).expect("csv output should exist");
        assert!(output.starts_with("channel,FL1-A,FL2-A"));
        assert!(output.contains("FL1-A,1.020408163265"));
    }

    fn sample_result() -> AutospillResult {
        AutospillResult {
            channels: vec!["FL1-A".to_owned(), "FL2-A".to_owned()],
            spillover_matrix: vec![vec![1.0, 0.2], vec![0.1, 1.0]],
            compensation_matrix: vec![
                vec![1.020_408_163_265_306_1, -0.204_081_632_653_061_2],
                vec![-0.102_040_816_326_530_6, 1.020_408_163_265_306_1],
            ],
            gate_stats: Vec::new(),
            convergence: ConvergenceSummary {
                converged: true,
                iterations: 2,
                final_delta_max: 0.0,
                steps: Vec::new(),
            },
        }
    }

    fn temp_path(file_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be monotonic")
            .as_nanos();
        std::env::temp_dir().join(format!("cytiq-autospill-{unique}-{file_name}"))
    }
}
