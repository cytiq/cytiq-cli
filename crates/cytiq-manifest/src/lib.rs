//! Run manifest: one YAML or JSON file describing an acquisition run — the
//! cytometry type, the panels, the sample files and the controls.
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
//! A run with several panels lists them under `panels` (named) and links each
//! sample to one; a control without `panel` belongs to every panel:
//!
//! ```yaml
//! cytometry: conventional
//! panels:
//!   - name: T
//!     markers: [{ marker: CD3 }]
//!     compensation: { source: matrix, matrix: autospill/t.csv, kind: compensation }
//!   - name: B
//!     markers: [{ marker: CD19 }]
//! samples:
//!   - { id: S01-T, file: s01_t.fcs, panel: T, specimen: S01 }
//!   - { id: S01-B, file: s01_b.fcs, panel: B, specimen: S01 }
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
#[serde(try_from = "ManifestDocument")]
pub struct RunManifest {
    pub cytometry: Cytometry,
    /// One or more panels; a manifest with a single `panel` has one entry.
    pub panels: Vec<Panel>,
    pub samples: Vec<SampleEntry>,
    #[serde(default)]
    pub controls: Vec<ControlEntry>,
}

/// The manifest as written: `panel` (one) or `panels` (several).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDocument {
    cytometry: Cytometry,
    #[serde(default)]
    panel: Option<Panel>,
    #[serde(default)]
    panels: Option<Vec<Panel>>,
    samples: Vec<SampleEntry>,
    #[serde(default)]
    controls: Vec<ControlEntry>,
}

impl TryFrom<ManifestDocument> for RunManifest {
    type Error = String;

    fn try_from(document: ManifestDocument) -> Result<Self, Self::Error> {
        let panels = match (document.panel, document.panels) {
            (Some(panel), None) => vec![panel],
            (None, Some(panels)) => panels,
            (Some(_), Some(_)) => {
                return Err("give either `panel` or `panels`, not both".to_owned());
            }
            (None, None) => return Err("missing field `panels` (or `panel`)".to_owned()),
        };
        Ok(Self {
            cytometry: document.cytometry,
            panels,
            samples: document.samples,
            controls: document.controls,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    /// Required, and unique, when the run has several panels.
    #[serde(default)]
    pub name: Option<String>,
    pub markers: Vec<Marker>,
    /// Where the compensation matrix of a conventional panel comes from;
    /// `None` means the samples' own `$SPILLOVER` (`source: fcs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compensation: Option<CompensationSource>,
}

impl Panel {
    /// `name`, or `panel` for an unnamed single panel.
    #[must_use]
    pub fn label(&self) -> &str {
        self.name.as_deref().unwrap_or("panel")
    }
}

/// Source of a conventional panel's compensation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompensationSource {
    /// Each sample's own `$SPILLOVER`/`$SPILL` keyword.
    Fcs,
    /// A matrix file, for example from `cytiq autospill`, applied with
    /// `cytiq compensate --matrix`.
    Matrix { matrix: PathBuf, kind: MatrixKind },
    /// A matrix `cytiq autospill` will compute from the panel's single-stain
    /// controls.
    Controls,
}

/// What a matrix file holds, as `cytiq compensate --matrix-kind` takes it.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixKind {
    Spillover,
    Compensation,
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
    /// Panel name; required when the run has several panels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<String>,
    /// The biological specimen the file was acquired from, shared by its
    /// files in different panels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub specimen: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlEntry {
    pub file: PathBuf,
    pub dye: String,
    #[serde(default)]
    pub primary_channel: Option<String>,
    /// Panel name; a control without one belongs to every panel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<String>,
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
        for panel in &mut manifest.panels {
            if let Some(CompensationSource::Matrix { matrix, .. }) = &mut panel.compensation {
                *matrix = resolve(base, matrix);
            }
        }
        manifest.validate(path)?;
        Ok(manifest)
    }

    /// The panel `sample` was acquired with: the one it names, or the only
    /// panel. `None` only for a manifest that fails [`Self::validate`].
    #[must_use]
    pub fn panel_of(&self, sample: &SampleEntry) -> Option<&Panel> {
        match &sample.panel {
            Some(name) => self
                .panels
                .iter()
                .find(|panel| panel.name.as_deref() == Some(name.as_str())),
            None => match self.panels.as_slice() {
                [panel] => Some(panel),
                _ => None,
            },
        }
    }

    /// Controls of `panel`: those naming it, and those naming no panel.
    pub fn controls_of<'a>(&'a self, panel: &'a Panel) -> impl Iterator<Item = &'a ControlEntry> {
        self.controls.iter().filter(move |control| {
            control.panel.is_none() || control.panel.as_deref() == panel.name.as_deref()
        })
    }

    /// The compensation source of a conventional panel (`fcs` by default).
    #[must_use]
    pub fn compensation_of<'a>(&self, panel: &'a Panel) -> Option<&'a CompensationSource> {
        const FCS: &CompensationSource = &CompensationSource::Fcs;
        match self.cytometry {
            Cytometry::Conventional => Some(panel.compensation.as_ref().unwrap_or(FCS)),
            Cytometry::Spectral => None,
        }
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
    /// `cytiq preflight`): samples, panels and markers present; unique
    /// non-empty sample ids, panel names and marker names; every sample and
    /// control linked to an existing panel; non-empty paths; compensation
    /// only for conventional panels, with controls when it comes from them;
    /// an unstained control for every spectral panel.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::Invalid`] naming the first problem.
    pub fn validate(&self, path: &Path) -> Result<(), ManifestError> {
        self.check().map_err(|reason| ManifestError::Invalid {
            path: path.to_path_buf(),
            reason,
        })
    }

    fn check(&self) -> Result<(), String> {
        if self.samples.is_empty() {
            return Err("`samples` must list at least one sample".to_owned());
        }
        self.check_panels()?;
        let mut ids = HashSet::new();
        let mut specimens = HashSet::new();
        for sample in &self.samples {
            if sample.id.trim().is_empty() {
                return Err("sample ids must not be empty".to_owned());
            }
            if !ids.insert(sample.id.trim()) {
                return Err(format!("sample id `{}` appears more than once", sample.id));
            }
            let Some(panel) = self.panel_of(sample) else {
                return Err(match &sample.panel {
                    Some(name) => format!("sample `{}`: panel `{name}` is not defined", sample.id),
                    None => format!(
                        "sample `{}` must name its `panel`: the run has several",
                        sample.id
                    ),
                });
            };
            if let Some(specimen) = &sample.specimen
                && !specimens.insert((specimen.trim(), panel.label()))
            {
                return Err(format!(
                    "specimen `{specimen}` has more than one file in panel `{}`",
                    panel.label()
                ));
            }
        }
        for control in &self.controls {
            if let Some(name) = &control.panel
                && !self
                    .panels
                    .iter()
                    .any(|panel| panel.name.as_deref() == Some(name))
            {
                return Err(format!(
                    "control `{}`: panel `{name}` is not defined",
                    control.dye
                ));
            }
        }
        let files = self
            .samples
            .iter()
            .map(|sample| &sample.file)
            .chain(self.controls.iter().map(|control| &control.file));
        for file in files {
            if file.as_os_str().is_empty() {
                return Err("file paths must not be empty".to_owned());
            }
        }
        for panel in &self.panels {
            self.check_calibration(panel)?;
        }
        Ok(())
    }

    fn check_panels(&self) -> Result<(), String> {
        if self.panels.is_empty() {
            return Err("`panels` must list at least one panel".to_owned());
        }
        let mut names = HashSet::new();
        for panel in &self.panels {
            match panel.name.as_deref().map(str::trim) {
                Some("") => return Err("panel names must not be empty".to_owned()),
                Some(name) if !names.insert(name) => {
                    return Err(format!("panel `{name}` appears more than once"));
                }
                None if self.panels.len() > 1 => {
                    return Err("every panel needs a `name` when the run has several".to_owned());
                }
                _ => {}
            }
            if panel.markers.is_empty() {
                return Err(format!(
                    "panel `{}`: `markers` must list at least one marker",
                    panel.label()
                ));
            }
            let mut markers = HashSet::new();
            for marker in &panel.markers {
                if marker.marker.trim().is_empty() {
                    return Err("marker names must not be empty".to_owned());
                }
                if !markers.insert(marker.marker.trim().to_ascii_lowercase()) {
                    return Err(format!(
                        "panel `{}`: marker `{}` appears more than once",
                        panel.label(),
                        marker.marker
                    ));
                }
            }
        }
        Ok(())
    }

    fn check_calibration(&self, panel: &Panel) -> Result<(), String> {
        let label = panel.label();
        match (self.cytometry, &panel.compensation) {
            (Cytometry::Spectral, Some(_)) => Err(format!(
                "panel `{label}`: `compensation` applies to conventional runs; spectral runs are unmixed from their controls"
            )),
            (Cytometry::Spectral, None) => {
                if self.controls_of(panel).any(ControlEntry::is_unstained) {
                    Ok(())
                } else {
                    Err(format!(
                        "panel `{label}`: spectral runs need an unstained control (`dye: Autofluorescence`)"
                    ))
                }
            }
            (Cytometry::Conventional, Some(CompensationSource::Matrix { matrix, .. }))
                if matrix.as_os_str().is_empty() =>
            {
                Err(format!(
                    "panel `{label}`: `compensation.matrix` must not be empty"
                ))
            }
            (Cytometry::Conventional, Some(CompensationSource::Controls))
                if !self
                    .controls_of(panel)
                    .any(|control| !control.is_unstained()) =>
            {
                Err(format!(
                    "panel `{label}`: `compensation: {{ source: controls }}` needs single-stain controls"
                ))
            }
            _ => Ok(()),
        }
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
        assert!(from_yaml.panels[0].markers[0].required);
        assert!(!from_yaml.panels[0].markers[1].required);
        assert_eq!(
            from_yaml.compensation_of(&from_yaml.panels[0]),
            Some(&CompensationSource::Fcs)
        );
    }

    #[test]
    fn rejects_unknown_fields_and_duplicate_ids() {
        let unknown = RunManifest::load(&write(
            "unknown.yaml",
            "cytometry: conventional\nsampels: []\n",
        ));
        assert!(matches!(unknown, Err(ManifestError::Parse { .. })));
        let typo = MULTI.replace("kind: compensation", "kind: compensation, matrx: x.csv");
        assert!(matches!(
            RunManifest::load(&write("typo.yaml", &typo)),
            Err(ManifestError::Parse { .. })
        ));

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

    const MULTI: &str = "\
cytometry: conventional
panels:
  - name: T
    markers: [{ marker: CD3 }]
    compensation: { source: matrix, matrix: autospill/t.csv, kind: compensation }
  - name: B
    markers: [{ marker: CD19 }]
    compensation: { source: controls }
samples:
  - { id: S01-T, file: s01_t.fcs, panel: T, specimen: S01 }
  - { id: S01-B, file: s01_b.fcs, panel: B, specimen: S01 }
controls:
  - { file: unstained.fcs, dye: Autofluorescence }
  - { file: cd19.fcs, dye: PE, panel: B }
";

    #[test]
    fn single_panel_manifest_still_loads() {
        let fixture =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/fixtures/pipeline/run.yaml");
        let manifest = RunManifest::load(&fixture).unwrap();
        assert_eq!(manifest.panels.len(), 1);
        let sample = &manifest.samples[0];
        assert_eq!(manifest.panel_of(sample), Some(&manifest.panels[0]));
    }

    #[test]
    fn links_samples_and_controls_to_their_panels() {
        let path = write("multi.yaml", MULTI);
        let manifest = RunManifest::load(&path).unwrap();
        let t = &manifest.panels[0];
        let b = &manifest.panels[1];
        assert_eq!(manifest.panel_of(&manifest.samples[1]), Some(b));
        assert_eq!(
            manifest.controls_of(t).count(),
            1,
            "only the shared control"
        );
        assert_eq!(manifest.controls_of(b).count(), 2);
        // Matrix paths resolve against the manifest, like FCS files.
        assert_eq!(
            manifest.compensation_of(t),
            Some(&CompensationSource::Matrix {
                matrix: path.parent().unwrap().join("autospill/t.csv"),
                kind: MatrixKind::Compensation,
            })
        );
        let json = serde_json::to_string(&manifest).unwrap();
        assert_eq!(
            RunManifest::load(&write("multi.json", &json)).unwrap(),
            manifest
        );
    }

    #[test]
    fn samples_need_a_panel_when_there_are_several() {
        let unlinked = MULTI.replace(", panel: T, specimen: S01", "");
        let error = RunManifest::load(&write("unlinked.yaml", &unlinked)).unwrap_err();
        assert!(
            error.to_string().contains("must name its `panel`"),
            "{error}"
        );

        let unknown = MULTI.replace("panel: T, specimen", "panel: NK, specimen");
        let error = RunManifest::load(&write("unknown-panel.yaml", &unknown)).unwrap_err();
        assert!(
            error.to_string().contains("panel `NK` is not defined"),
            "{error}"
        );
    }

    #[test]
    fn rejects_panel_and_panels_together() {
        let both = YAML.replace("samples:", "panels: []\nsamples:");
        let error = RunManifest::load(&write("both.yaml", &both)).unwrap_err();
        assert!(error.to_string().contains("not both"), "{error}");
    }

    #[test]
    fn rejects_compensation_in_spectral_runs() {
        let spectral = YAML
            .replace("cytometry: conventional", "cytometry: spectral")
            .replace(
                "  name: T cells\n",
                "  name: T cells\n  compensation: { source: fcs }\n",
            );
        let error = RunManifest::load(&write("spectral-comp.yaml", &spectral)).unwrap_err();
        assert!(error.to_string().contains("conventional runs"), "{error}");
    }

    #[test]
    fn controls_source_needs_single_stain_controls() {
        let without = MULTI.replace("  - { file: cd19.fcs, dye: PE, panel: B }\n", "");
        let error = RunManifest::load(&write("no-stains.yaml", &without)).unwrap_err();
        assert!(
            error.to_string().contains("needs single-stain controls"),
            "{error}"
        );
    }

    #[test]
    fn a_specimen_has_one_file_per_panel() {
        let twice = MULTI.replace("panel: B, specimen: S01", "panel: T, specimen: S01");
        let error = RunManifest::load(&write("specimen.yaml", &twice)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("more than one file in panel `T`"),
            "{error}"
        );
    }
}
