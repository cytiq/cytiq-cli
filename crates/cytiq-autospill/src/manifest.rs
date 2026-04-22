use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::AutospillError;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ControlsManifestRow {
    pub filename: String,
    pub dye: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct DetectorSpec {
    pub channel: String,
    #[serde(default, deserialize_with = "empty_string_is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub wavelength: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct TransformSpec {
    pub channel: String,
    pub length: f64,
    #[serde(rename = "max_range")]
    pub max_range: f64,
    pub pos: f64,
    pub neg: f64,
    pub width: f64,
}

/// Read and validate the controls manifest CSV.
///
/// # Errors
///
/// Returns an error if the file cannot be read, parsed, or fails manifest
/// validation.
pub fn read_controls_manifest(
    path: impl AsRef<Path>,
) -> Result<Vec<ControlsManifestRow>, AutospillError> {
    let path = path.as_ref();
    let mut reader =
        csv::Reader::from_path(path).map_err(|source| AutospillError::ManifestRead {
            path: path.to_path_buf(),
            source,
        })?;
    let mut rows = Vec::new();
    for record in reader.deserialize() {
        let row = record.map_err(|source| AutospillError::ManifestRead {
            path: path.to_path_buf(),
            source,
        })?;
        rows.push(row);
    }
    validate_controls_manifest(&rows)?;
    Ok(rows)
}

/// Read and validate the detectors manifest CSV.
///
/// # Errors
///
/// Returns an error if the file cannot be read, parsed, or fails manifest
/// validation.
pub fn read_detectors_manifest(
    path: impl AsRef<Path>,
) -> Result<Vec<DetectorSpec>, AutospillError> {
    let path = path.as_ref();
    let mut reader =
        csv::Reader::from_path(path).map_err(|source| AutospillError::DetectorRead {
            path: path.to_path_buf(),
            source,
        })?;
    let mut rows = Vec::new();
    for record in reader.deserialize() {
        let row = record.map_err(|source| AutospillError::DetectorRead {
            path: path.to_path_buf(),
            source,
        })?;
        rows.push(row);
    }
    validate_detectors_manifest(&rows)?;
    Ok(rows)
}

/// Read and validate the transform manifest CSV.
///
/// # Errors
///
/// Returns an error if the file cannot be read, parsed, or fails manifest
/// validation.
pub fn read_transform_manifest(
    path: impl AsRef<Path>,
) -> Result<Vec<TransformSpec>, AutospillError> {
    let path = path.as_ref();
    let mut reader =
        csv::Reader::from_path(path).map_err(|source| AutospillError::TransformRead {
            path: path.to_path_buf(),
            source,
        })?;
    let mut rows = Vec::new();
    for record in reader.deserialize() {
        let row = record.map_err(|source| AutospillError::TransformRead {
            path: path.to_path_buf(),
            source,
        })?;
        rows.push(row);
    }
    validate_transform_manifest(&rows)?;
    Ok(rows)
}

fn validate_controls_manifest(rows: &[ControlsManifestRow]) -> Result<(), AutospillError> {
    if rows.is_empty() {
        return Err(AutospillError::EmptyManifest);
    }

    let mut seen_dyes = HashSet::new();
    for (index, row) in rows.iter().enumerate() {
        let row_number = index + 1;
        if row.filename.trim().is_empty() {
            return Err(AutospillError::EmptyManifestField {
                row: row_number,
                field: "filename",
            });
        }
        if PathBuf::from(&row.filename).is_absolute() {
            return Err(AutospillError::AbsoluteManifestFilename {
                row: row_number,
                filename: row.filename.clone(),
            });
        }
        if row.dye.trim().is_empty() {
            return Err(AutospillError::EmptyManifestField {
                row: row_number,
                field: "dye",
            });
        }
        if !seen_dyes.insert(row.dye.clone()) {
            return Err(AutospillError::DuplicateDye {
                value: row.dye.clone(),
            });
        }
    }

    Ok(())
}

fn validate_detectors_manifest(rows: &[DetectorSpec]) -> Result<(), AutospillError> {
    if rows.is_empty() {
        return Err(AutospillError::EmptyDetectorsManifest);
    }

    let mut seen_channels = HashSet::new();
    for (index, row) in rows.iter().enumerate() {
        let row_number = index + 1;
        if row.channel.trim().is_empty() {
            return Err(AutospillError::EmptyDetectorField {
                row: row_number,
                field: "channel",
            });
        }
        if !seen_channels.insert(row.channel.clone()) {
            return Err(AutospillError::DuplicateDetectorChannel {
                value: row.channel.clone(),
            });
        }
    }

    Ok(())
}

fn validate_transform_manifest(rows: &[TransformSpec]) -> Result<(), AutospillError> {
    let mut seen_channels = HashSet::new();
    for (index, row) in rows.iter().enumerate() {
        let row_number = index + 1;
        if row.channel.trim().is_empty() {
            return Err(AutospillError::EmptyTransformField {
                row: row_number,
                field: "channel",
            });
        }
        if !seen_channels.insert(row.channel.clone()) {
            return Err(AutospillError::DuplicateTransformChannel {
                value: row.channel.clone(),
            });
        }
    }

    Ok(())
}

fn empty_string_is_none<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value.and_then(|value| {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    }))
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{ControlsManifestRow, DetectorSpec, TransformSpec};
    use crate::error::AutospillError;

    fn validate(rows: &[ControlsManifestRow]) -> Result<(), AutospillError> {
        super::validate_controls_manifest(rows)
    }

    fn row(filename: &str, dye: &str) -> ControlsManifestRow {
        ControlsManifestRow {
            filename: filename.to_owned(),
            dye: dye.to_owned(),
        }
    }

    #[test]
    fn rejects_duplicate_dyes() {
        let rows = vec![row("a.fcs", "FITC"), row("b.fcs", "FITC")];
        let error = validate(&rows).expect_err("duplicate dye should fail");
        assert!(matches!(error, AutospillError::DuplicateDye { .. }));
    }

    #[test]
    fn rejects_empty_dye() {
        let rows = vec![row("a.fcs", "")];
        let error = validate(&rows).expect_err("empty dye should fail");
        assert!(matches!(
            error,
            AutospillError::EmptyManifestField { field: "dye", .. }
        ));
    }

    #[test]
    fn deserializes_legacy_extra_columns() {
        let mut reader = csv::Reader::from_reader(Cursor::new(
            "filename,dye,primary_channel,antigen,control_kind,wavelength\ncontrol_a.fcs,FITC,FL1-A,CD3,single_color,530\n",
        ));
        let row: ControlsManifestRow = reader
            .deserialize()
            .next()
            .expect("row should exist")
            .expect("row should deserialize");

        assert_eq!(
            row,
            ControlsManifestRow {
                filename: "control_a.fcs".to_owned(),
                dye: "FITC".to_owned(),
            }
        );
    }

    #[test]
    fn rejects_duplicate_detector_channels() {
        let rows = vec![
            DetectorSpec {
                channel: "FL1-A".to_owned(),
                label: None,
                wavelength: None,
            },
            DetectorSpec {
                channel: "FL1-A".to_owned(),
                label: None,
                wavelength: None,
            },
        ];
        let error = super::validate_detectors_manifest(&rows)
            .expect_err("duplicate detector channel should fail");
        assert!(matches!(
            error,
            AutospillError::DuplicateDetectorChannel { .. }
        ));
    }

    #[test]
    fn rejects_duplicate_transform_channels() {
        let rows = vec![
            TransformSpec {
                channel: "FITC".to_owned(),
                length: 256.0,
                max_range: 262_144.0,
                pos: 4.0,
                neg: 0.0,
                width: -100.0,
            },
            TransformSpec {
                channel: "FITC".to_owned(),
                length: 256.0,
                max_range: 262_144.0,
                pos: 4.0,
                neg: 0.0,
                width: -100.0,
            },
        ];
        let error = super::validate_transform_manifest(&rows)
            .expect_err("duplicate transform channel should fail");
        assert!(matches!(
            error,
            AutospillError::DuplicateTransformChannel { .. }
        ));
    }
}
