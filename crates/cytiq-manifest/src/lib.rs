//! Run manifest: one YAML or JSON file describing an acquisition run — the
//! cytometry type, the panel, the sample files and the controls.
//!
//! ```yaml
//! cytometry: conventional        # or spectral
//! panel:
//!   name: T-cell-8c
//!   markers:
//!     - { marker: CD3, channel: FL1-A }        # channel optional: matched by $PnS
//!     - { marker: Live/Dead, required: false }
//! samples:
//!   - { id: S01, file: fcs/s01.fcs }
//! controls:
//!   - { file: fcs/unstained.fcs, dye: Autofluorescence }
//!   - { file: fcs/cd3_fitc.fcs, dye: FITC, primary_channel: FL1-A }
//! ```
//!
//! Relative file paths are resolved against the manifest's directory.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("failed to read manifest `{path}`: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("manifest `{path}` must be `.yaml`, `.yml` or `.json`")]
    UnsupportedFormat { path: PathBuf },
    #[error("failed to parse manifest `{path}`: {message}")]
    Parse { path: PathBuf, message: String },
    #[error("manifest `{path}` is invalid: {reason}")]
    Invalid { path: PathBuf, reason: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Cytometry {
    /// Compensation with a spillover matrix.
    Conventional,
    /// Spectral unmixing with reference controls.
    Spectral,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunManifest {
    pub cytometry: Cytometry,
    pub panel: Panel,
    pub samples: Vec<SampleEntry>,
    #[serde(default)]
    pub controls: Vec<ControlEntry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    #[serde(default)]
    pub name: Option<String>,
    pub markers: Vec<Marker>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Marker {
    /// Marker name, matched against `$PnS` when `channel` is not given.
    pub marker: String,
    /// Detector channel (`$PnN`) carrying the marker.
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default = "default_required")]
    pub required: bool,
}

const fn default_required() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleEntry {
    pub id: String,
    pub file: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlEntry {
    pub file: PathBuf,
    pub dye: String,
    #[serde(default)]
    pub primary_channel: Option<String>,
}

impl ControlEntry {
    /// Unstained / autofluorescence control (`dye: Autofluorescence`,
    /// `AF`, or `unstained`).
    #[must_use]
    pub fn is_unstained(&self) -> bool {
        ["autofluorescence", "af", "unstained"]
            .iter()
            .any(|name| self.dye.trim().eq_ignore_ascii_case(name))
    }
}

impl RunManifest {
    /// Read, parse and validate a manifest, resolving relative file paths
    /// against its directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or parsed, or when the
    /// manifest is invalid (see [`Self::validate`]).
    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let text = fs::read_to_string(path).map_err(|source| ManifestError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let mut manifest = Self::parse(path, &text)?;
        let base = path.parent().unwrap_or_else(|| Path::new(""));
        for sample in &mut manifest.samples {
            sample.file = resolve(base, &sample.file);
        }
        for control in &mut manifest.controls {
            control.file = resolve(base, &control.file);
        }
        manifest.validate(path)?;
        Ok(manifest)
    }

    fn parse(path: &Path, text: &str) -> Result<Self, ManifestError> {
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        let parse_error = |message: String| ManifestError::Parse {
            path: path.to_path_buf(),
            message,
        };
        match extension.as_deref() {
            Some("yaml" | "yml") => {
                serde_saphyr::from_str(text).map_err(|error| parse_error(error.to_string()))
            }
            Some("json") => {
                serde_json::from_str(text).map_err(|error| parse_error(error.to_string()))
            }
            _ => Err(ManifestError::UnsupportedFormat {
                path: path.to_path_buf(),
            }),
        }
    }

    /// Check the manifest's own consistency (files are checked by
    /// `cytiq preflight`): at least one sample and marker, unique non-empty
    /// sample ids and marker names, non-empty paths, and an unstained control
    /// for spectral runs.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::Invalid`] naming the first problem.
    pub fn validate(&self, path: &Path) -> Result<(), ManifestError> {
        let invalid = |reason: String| {
            Err(ManifestError::Invalid {
                path: path.to_path_buf(),
                reason,
            })
        };
        if self.samples.is_empty() {
            return invalid("`samples` must list at least one sample".to_owned());
        }
        if self.panel.markers.is_empty() {
            return invalid("`panel.markers` must list at least one marker".to_owned());
        }
        let mut ids = HashSet::new();
        for sample in &self.samples {
            if sample.id.trim().is_empty() {
                return invalid("sample ids must not be empty".to_owned());
            }
            if !ids.insert(sample.id.trim()) {
                return invalid(format!("sample id `{}` appears more than once", sample.id));
            }
        }
        let files = self
            .samples
            .iter()
            .map(|sample| &sample.file)
            .chain(self.controls.iter().map(|control| &control.file));
        for file in files {
            if file.as_os_str().is_empty() {
                return invalid("file paths must not be empty".to_owned());
            }
        }
        let mut markers = HashSet::new();
        for marker in &self.panel.markers {
            if marker.marker.trim().is_empty() {
                return invalid("marker names must not be empty".to_owned());
            }
            if !markers.insert(marker.marker.trim().to_ascii_lowercase()) {
                return invalid(format!("marker `{}` appears more than once", marker.marker));
            }
        }
        if self.cytometry == Cytometry::Spectral
            && !self.controls.iter().any(ControlEntry::is_unstained)
        {
            return invalid(
                "spectral runs need an unstained control (`dye: Autofluorescence`)".to_owned(),
            );
        }
        Ok(())
    }
}

fn resolve(base: &Path, file: &Path) -> PathBuf {
    if file.is_absolute() {
        file.to_path_buf()
    } else {
        base.join(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const YAML: &str = "\
cytometry: conventional
panel:
  name: T cells
  markers:
    - { marker: CD3, channel: FL1-A }
    - { marker: Live/Dead, required: false }
samples:
  - { id: S01, file: fcs/s01.fcs }
controls:
  - { file: /abs/unstained.fcs, dye: Autofluorescence }
";

    fn write(name: &str, text: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!("cytiq-manifest-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join(name);
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn yaml_and_json_load_the_same_manifest() {
        let yaml_path = write("run.yaml", YAML);
        let from_yaml = RunManifest::load(&yaml_path).unwrap();
        let json = serde_json::to_string(&from_yaml).unwrap();
        let from_json = RunManifest::load(&write("run.json", &json)).unwrap();
        assert_eq!(from_yaml, from_json);
        assert_eq!(
            from_yaml.samples[0].file,
            yaml_path.parent().unwrap().join("fcs/s01.fcs")
        );
        assert_eq!(
            from_yaml.controls[0].file,
            PathBuf::from("/abs/unstained.fcs")
        );
        assert!(from_yaml.panel.markers[0].required);
        assert!(!from_yaml.panel.markers[1].required);
    }

    #[test]
    fn rejects_unknown_fields_and_duplicate_ids() {
        let unknown = RunManifest::load(&write(
            "unknown.yaml",
            "cytometry: conventional\nsampels: []\n",
        ));
        assert!(matches!(unknown, Err(ManifestError::Parse { .. })));

        let duplicate = YAML.replace(
            "  - { id: S01, file: fcs/s01.fcs }\n",
            "  - { id: S01, file: a.fcs }\n  - { id: S01, file: b.fcs }\n",
        );
        let error = RunManifest::load(&write("duplicate.yaml", &duplicate)).unwrap_err();
        assert!(error.to_string().contains("more than once"), "{error}");
    }

    #[test]
    fn spectral_runs_need_an_unstained_control() {
        let spectral = YAML
            .replace("cytometry: conventional", "cytometry: spectral")
            .replace("dye: Autofluorescence", "dye: FITC");
        let error = RunManifest::load(&write("spectral.yaml", &spectral)).unwrap_err();
        assert!(error.to_string().contains("unstained"), "{error}");
    }
}
