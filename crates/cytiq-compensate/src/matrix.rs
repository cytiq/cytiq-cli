use std::path::Path;

use cytiq_fcs::{FcsDataset, Spillover};

use crate::error::CompensateError;

/// What a matrix file holds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatrixKind {
    /// Rows are fluorochromes, columns detectors: the fraction of each
    /// fluorochrome seen in each detector (`$SPILLOVER`).
    Spillover,
    /// The inverse of the spillover matrix; rows are detectors, columns the
    /// compensated channels.
    Compensation,
}

/// A square compensation matrix over named channels, row-major, applied as
/// `compensated[j] = Σ_i raw[i] * matrix[i, j]` (see [`cytiq_fcs::compensate`]).
#[derive(Clone, Debug, PartialEq)]
pub struct CompensationMatrix {
    pub channels: Vec<String>,
    pub values: Vec<f64>,
}

impl CompensationMatrix {
    /// Invert a spillover matrix.
    ///
    /// # Errors
    ///
    /// Returns an error when the matrix is singular.
    pub fn from_spillover(spillover: &Spillover) -> Result<Self, cytiq_fcs::Error> {
        Ok(Self {
            channels: spillover.channels().to_vec(),
            values: spillover.compensation_matrix()?,
        })
    }

    /// The compensation matrix of the dataset's own spillover keyword.
    ///
    /// # Errors
    ///
    /// Returns an error when the keyword is missing, invalid or singular.
    pub fn from_dataset(path: &Path, dataset: &FcsDataset) -> Result<Self, CompensateError> {
        let spillover_error = |source| CompensateError::Spillover {
            path: path.to_path_buf(),
            source,
        };
        let spillover = Spillover::from_dataset(dataset)
            .map_err(spillover_error)?
            .ok_or_else(|| CompensateError::MissingSpillover {
                path: path.to_path_buf(),
            })?;
        Self::from_spillover(&spillover).map_err(spillover_error)
    }
}

/// Read a square matrix written by `cytiq autospill` (`.csv` or `.tsv`): a
/// header row with a corner label and the channel names, then one row per
/// channel starting with its name. Rows may be in any order.
///
/// # Errors
///
/// Returns an error when the file cannot be read, is not square over one set
/// of channel names, has non-finite values, or a spillover matrix is singular.
pub fn read_matrix_file(
    path: &Path,
    kind: MatrixKind,
) -> Result<CompensationMatrix, CompensateError> {
    let invalid = |reason: String| CompensateError::InvalidMatrix {
        path: path.to_path_buf(),
        reason,
    };
    let read_error = |source| CompensateError::MatrixRead {
        path: path.to_path_buf(),
        source,
    };
    let delimiter = match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) if extension.eq_ignore_ascii_case("tsv") => b'\t',
        Some(extension) if extension.eq_ignore_ascii_case("csv") => b',',
        _ => return Err(invalid("expected a `.csv` or `.tsv` file".to_owned())),
    };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .trim(csv::Trim::All)
        .from_path(path)
        .map_err(read_error)?;
    let channels = reader
        .headers()
        .map_err(read_error)?
        .iter()
        .skip(1)
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if channels.is_empty() {
        return Err(invalid("the header has no channel columns".to_owned()));
    }

    let size = channels.len();
    let mut rows: Vec<Option<Vec<f64>>> = vec![None; size];
    for record in reader.records() {
        let record = record.map_err(read_error)?;
        let label = record.get(0).unwrap_or_default();
        let position = channels
            .iter()
            .position(|channel| channel == label)
            .ok_or_else(|| {
                invalid(format!(
                    "row `{label}` is not one of the column channels; only square matrices over one channel set are supported"
                ))
            })?;
        if rows[position].is_some() {
            return Err(invalid(format!("row `{label}` appears more than once")));
        }
        let values = record
            .iter()
            .skip(1)
            .map(|value| {
                value
                    .parse::<f64>()
                    .ok()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| {
                        invalid(format!("row `{label}` has non-numeric value `{value}`"))
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if values.len() != size {
            return Err(invalid(format!(
                "row `{label}` has {} values, expected {size}",
                values.len()
            )));
        }
        rows[position] = Some(values);
    }
    let values = rows
        .into_iter()
        .zip(&channels)
        .map(|(row, channel)| row.ok_or_else(|| invalid(format!("row `{channel}` is missing"))))
        .collect::<Result<Vec<_>, _>>()?
        .concat();

    match kind {
        MatrixKind::Compensation => Ok(CompensationMatrix { channels, values }),
        MatrixKind::Spillover => {
            let spillover_error = |source| CompensateError::Spillover {
                path: path.to_path_buf(),
                source,
            };
            let spillover = Spillover::from_parts(channels, values).map_err(spillover_error)?;
            CompensationMatrix::from_spillover(&spillover).map_err(spillover_error)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/fixtures/autospill")
            .join(name)
    }

    fn write_temp(name: &str, contents: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("cytiq-compensate-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(name);
        std::fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn spillover_and_compensation_files_agree() {
        let from_spillover =
            read_matrix_file(&fixture("expected_spillover.csv"), MatrixKind::Spillover).unwrap();
        let from_compensation = read_matrix_file(
            &fixture("expected_compensation.csv"),
            MatrixKind::Compensation,
        )
        .unwrap();
        assert_eq!(from_spillover.channels, from_compensation.channels);
        for (left, right) in from_spillover.values.iter().zip(&from_compensation.values) {
            assert!((left - right).abs() < 1.0e-9, "{left} vs {right}");
        }
    }

    #[test]
    fn reorders_rows_to_column_order() {
        let path = write_temp("reordered.csv", "channel,A,B\nB,0.1,1\nA,1,0.2\n");
        let matrix = read_matrix_file(&path, MatrixKind::Compensation).unwrap();
        assert_eq!(matrix.values, vec![1.0, 0.2, 0.1, 1.0]);
    }

    #[test]
    fn rejects_rectangular_matrices() {
        let path = write_temp("rectangular.csv", "dye,A,B,C\nFITC,1,0.1,0\nPE,0.2,1,0\n");
        let error = read_matrix_file(&path, MatrixKind::Spillover).unwrap_err();
        assert!(error.to_string().contains("square"), "{error}");
    }
}
