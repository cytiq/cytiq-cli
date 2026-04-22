use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::AutospillError;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct ControlsManifestRow {
    pub filename: String,
    pub dye: String,
    pub primary_channel: String,
    #[serde(default, deserialize_with = "empty_string_is_none")]
    pub antigen: Option<String>,
}

/// Read and validate the controls manifest CSV.
///
/// # Errors
///
/// Returns an error when the CSV cannot be parsed or when manifest validation
/// fails.
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

fn validate_controls_manifest(rows: &[ControlsManifestRow]) -> Result<(), AutospillError> {
    if rows.is_empty() {
        return Err(AutospillError::EmptyManifest);
    }

    let mut seen_dyes = HashSet::new();
    let mut seen_channels = HashSet::new();
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
        if row.primary_channel.trim().is_empty() {
            return Err(AutospillError::EmptyManifestField {
                row: row_number,
                field: "primary_channel",
            });
        }
        if !seen_dyes.insert(row.dye.clone()) {
            return Err(AutospillError::DuplicateDye {
                value: row.dye.clone(),
            });
        }
        if !seen_channels.insert(row.primary_channel.clone()) {
            return Err(AutospillError::DuplicatePrimaryChannel {
                value: row.primary_channel.clone(),
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
    use super::ControlsManifestRow;
    use crate::error::AutospillError;

    fn validate(rows: &[ControlsManifestRow]) -> Result<(), AutospillError> {
        super::validate_controls_manifest(rows)
    }

    fn row(filename: &str, dye: &str, channel: &str) -> ControlsManifestRow {
        ControlsManifestRow {
            filename: filename.to_owned(),
            dye: dye.to_owned(),
            primary_channel: channel.to_owned(),
            antigen: None,
        }
    }

    #[test]
    fn rejects_duplicate_dyes() {
        let rows = vec![row("a.fcs", "FITC", "FL1-A"), row("b.fcs", "FITC", "FL2-A")];
        let error = validate(&rows).expect_err("duplicate dye should fail");
        assert!(matches!(error, AutospillError::DuplicateDye { .. }));
    }

    #[test]
    fn rejects_duplicate_primary_channels() {
        let rows = vec![row("a.fcs", "FITC", "FL1-A"), row("b.fcs", "PE", "FL1-A")];
        let error = validate(&rows).expect_err("duplicate primary channel should fail");
        assert!(matches!(
            error,
            AutospillError::DuplicatePrimaryChannel { .. }
        ));
    }

    #[test]
    fn rejects_empty_primary_channel() {
        let rows = vec![row("a.fcs", "FITC", "")];
        let error = validate(&rows).expect_err("empty primary channel should fail");
        assert!(matches!(
            error,
            AutospillError::EmptyManifestField {
                field: "primary_channel",
                ..
            }
        ));
    }
}
