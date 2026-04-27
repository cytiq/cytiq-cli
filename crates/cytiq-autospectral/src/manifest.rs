use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::AutoSpectralError;

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ControlManifestRow {
    pub filename: String,
    #[serde(alias = "fluorophore")]
    pub dye: String,
    #[serde(default, alias = "channel", deserialize_with = "empty_string_is_none")]
    pub primary_channel: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct SampleManifestRow {
    pub filename: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct DetectorSpec {
    pub channel: String,
    #[serde(default, deserialize_with = "empty_string_is_none")]
    pub label: Option<String>,
    #[serde(default)]
    pub wavelength: Option<f64>,
}

/// Read and validate the controls manifest CSV.
///
/// # Errors
///
/// Returns an error if the file cannot be read, parsed, or fails validation.
pub fn read_controls_manifest(
    path: impl AsRef<Path>,
) -> Result<Vec<ControlManifestRow>, AutoSpectralError> {
    let path = path.as_ref();
    let mut reader =
        csv::Reader::from_path(path).map_err(|source| AutoSpectralError::ControlsManifestRead {
            path: path.to_path_buf(),
            source,
        })?;
    let mut rows = Vec::new();
    for record in reader.deserialize() {
        let row = record.map_err(|source| AutoSpectralError::ControlsManifestRead {
            path: path.to_path_buf(),
            source,
        })?;
        rows.push(row);
    }
    validate_controls_manifest(&rows)?;
    Ok(rows)
}

/// Read and validate the samples manifest CSV.
///
/// # Errors
///
/// Returns an error if the file cannot be read, parsed, or fails validation.
pub fn read_samples_manifest(
    path: impl AsRef<Path>,
) -> Result<Vec<SampleManifestRow>, AutoSpectralError> {
    let path = path.as_ref();
    let mut reader =
        csv::Reader::from_path(path).map_err(|source| AutoSpectralError::SamplesManifestRead {
            path: path.to_path_buf(),
            source,
        })?;
    let mut rows = Vec::new();
    for record in reader.deserialize() {
        let row = record.map_err(|source| AutoSpectralError::SamplesManifestRead {
            path: path.to_path_buf(),
            source,
        })?;
        rows.push(row);
    }
    validate_samples_manifest(&rows)?;
    Ok(rows)
}

/// Read and validate the detectors manifest CSV.
///
/// # Errors
///
/// Returns an error if the file cannot be read, parsed, or fails validation.
pub fn read_detectors_manifest(
    path: impl AsRef<Path>,
) -> Result<Vec<DetectorSpec>, AutoSpectralError> {
    let path = path.as_ref();
    let mut reader = csv::Reader::from_path(path).map_err(|source| {
        AutoSpectralError::DetectorsManifestRead {
            path: path.to_path_buf(),
            source,
        }
    })?;
    let mut rows = Vec::new();
    for record in reader.deserialize() {
        let row = record.map_err(|source| AutoSpectralError::DetectorsManifestRead {
            path: path.to_path_buf(),
            source,
        })?;
        rows.push(row);
    }
    validate_detectors_manifest(&rows)?;
    Ok(rows)
}

fn validate_controls_manifest(rows: &[ControlManifestRow]) -> Result<(), AutoSpectralError> {
    if rows.is_empty() {
        return Err(AutoSpectralError::EmptyControlsManifest);
    }

    let mut seen_dyes = HashSet::new();
    let mut autofluorescence_count = 0_usize;
    for (index, row) in rows.iter().enumerate() {
        let row_number = index + 1;
        validate_filename("controls", row_number, &row.filename)?;
        if row.dye.trim().is_empty() {
            return Err(AutoSpectralError::EmptyManifestField {
                manifest: "controls",
                row: row_number,
                field: "dye",
            });
        }
        if row
            .primary_channel
            .as_deref()
            .is_some_and(|primary_channel| primary_channel.trim().is_empty())
        {
            return Err(AutoSpectralError::EmptyManifestField {
                manifest: "controls",
                row: row_number,
                field: "primary_channel",
            });
        }
        if !seen_dyes.insert(row.dye.clone()) {
            return Err(AutoSpectralError::DuplicateDye {
                value: row.dye.clone(),
            });
        }
        if is_autofluorescence_dye(&row.dye) {
            autofluorescence_count += 1;
        }
    }

    match autofluorescence_count {
        0 => Err(AutoSpectralError::MissingAutofluorescenceControl),
        1 if rows.len() > 1 => Ok(()),
        1 => Err(AutoSpectralError::NoSingleStainControls),
        _ => Err(AutoSpectralError::DuplicateAutofluorescenceControl),
    }
}

fn validate_samples_manifest(rows: &[SampleManifestRow]) -> Result<(), AutoSpectralError> {
    if rows.is_empty() {
        return Err(AutoSpectralError::EmptySamplesManifest);
    }

    for (index, row) in rows.iter().enumerate() {
        validate_filename("samples", index + 1, &row.filename)?;
    }

    Ok(())
}

fn validate_detectors_manifest(rows: &[DetectorSpec]) -> Result<(), AutoSpectralError> {
    if rows.is_empty() {
        return Err(AutoSpectralError::EmptyDetectorsManifest);
    }

    let mut seen_channels = HashSet::new();
    for (index, row) in rows.iter().enumerate() {
        let row_number = index + 1;
        if row.channel.trim().is_empty() {
            return Err(AutoSpectralError::EmptyManifestField {
                manifest: "detectors",
                row: row_number,
                field: "channel",
            });
        }
        if !seen_channels.insert(row.channel.clone()) {
            return Err(AutoSpectralError::DuplicateDetectorChannel {
                value: row.channel.clone(),
            });
        }
    }

    Ok(())
}

fn validate_filename(
    manifest: &'static str,
    row: usize,
    filename: &str,
) -> Result<(), AutoSpectralError> {
    if filename.trim().is_empty() {
        return Err(AutoSpectralError::EmptyManifestField {
            manifest,
            row,
            field: "filename",
        });
    }
    if PathBuf::from(filename).is_absolute() {
        return Err(AutoSpectralError::AbsoluteManifestFilename {
            manifest,
            row,
            filename: filename.to_owned(),
        });
    }

    Ok(())
}

pub(crate) fn is_autofluorescence_dye(dye: &str) -> bool {
    matches!(
        normalize_identifier(dye).as_str(),
        "autofluorescence" | "af"
    )
}

pub(crate) fn normalize_identifier(value: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    for suffix in ["-a", "-h", "-w"] {
        if let Some(stripped) = normalized.strip_suffix(suffix) {
            return stripped.to_owned();
        }
    }

    normalized
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

    use super::{ControlManifestRow, SampleManifestRow};
    use crate::AutoSpectralError;

    fn control(filename: &str, dye: &str) -> ControlManifestRow {
        ControlManifestRow {
            filename: filename.to_owned(),
            dye: dye.to_owned(),
            primary_channel: None,
        }
    }

    #[test]
    fn controls_require_autofluorescence() {
        let error = super::validate_controls_manifest(&[control("fitc.fcs", "FITC")])
            .expect_err("missing AF should fail");
        assert!(matches!(
            error,
            AutoSpectralError::MissingAutofluorescenceControl
        ));
    }

    #[test]
    fn controls_require_single_stains() {
        let error =
            super::validate_controls_manifest(&[control("unstained.fcs", "Autofluorescence")])
                .expect_err("AF-only manifest should fail");
        assert!(matches!(error, AutoSpectralError::NoSingleStainControls));
    }

    #[test]
    fn controls_reject_duplicate_dyes() {
        let rows = [
            control("unstained.fcs", "Autofluorescence"),
            control("a.fcs", "FITC"),
            control("b.fcs", "FITC"),
        ];
        let error = super::validate_controls_manifest(&rows).expect_err("duplicate should fail");
        assert!(matches!(error, AutoSpectralError::DuplicateDye { .. }));
    }

    #[test]
    fn controls_accept_explicit_primary_channel() {
        let rows = [
            control("unstained.fcs", "Autofluorescence"),
            ControlManifestRow {
                filename: "apc.fcs".to_owned(),
                dye: "APC".to_owned(),
                primary_channel: Some("R1-A".to_owned()),
            },
        ];
        super::validate_controls_manifest(&rows).expect("explicit channel should validate");
    }

    #[test]
    fn controls_deserialize_autospectral_column_names() {
        let csv = "filename,fluorophore,channel\nunstained.fcs,AF,\napc.fcs,APC,R1-A\n";
        let mut reader = csv::Reader::from_reader(Cursor::new(csv));
        let rows = reader
            .deserialize()
            .collect::<Result<Vec<ControlManifestRow>, _>>()
            .expect("controls should deserialize");

        assert_eq!(rows[0].dye, "AF");
        assert_eq!(rows[0].primary_channel, None);
        assert_eq!(rows[1].dye, "APC");
        assert_eq!(rows[1].primary_channel.as_deref(), Some("R1-A"));
        super::validate_controls_manifest(&rows).expect("AF alias should validate");
    }

    #[test]
    fn samples_reject_absolute_paths() {
        let rows = [SampleManifestRow {
            filename: "/tmp/sample.fcs".to_owned(),
        }];
        let error = super::validate_samples_manifest(&rows).expect_err("absolute should fail");
        assert!(matches!(
            error,
            AutoSpectralError::AbsoluteManifestFilename { .. }
        ));
    }
}
