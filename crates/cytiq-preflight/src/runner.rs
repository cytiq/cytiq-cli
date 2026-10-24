use std::fs;
use std::path::{Path, PathBuf};

use cytiq_manifest::{CompensationSource, ControlEntry, Panel, RunManifest, SampleEntry};
use log::{debug, info};
use serde::Serialize;

use crate::checks::{
    Check, CheckStatus, LoadedFile, check_control_coverage, check_control_dyes, check_detectors,
    check_file, check_markers, check_matrix, check_primary_channel, check_spillover, panel_scope,
};
use crate::error::PreflightError;

/// Configuration of `cytiq preflight`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreflightConfig {
    /// Run manifest (`.yaml`, `.yml` or `.json`).
    pub manifest: PathBuf,
    /// JSON report path.
    pub output: PathBuf,
    pub overwrite: bool,
}

/// The `preflight.json` report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PreflightReport {
    pub tool: String,
    pub version: String,
    pub manifest: String,
    /// FCS files the manifest lists, resolved.
    pub files: Vec<String>,
    pub panels: Vec<PanelSummary>,
    /// The worst check status.
    pub status: CheckStatus,
    pub passed: usize,
    pub warnings: usize,
    pub failures: usize,
    pub checks: Vec<Check>,
}

/// What the report found for one panel.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PanelSummary {
    pub name: String,
    /// Sample ids.
    pub samples: Vec<String>,
    /// Control dyes, including controls shared by every panel.
    pub controls: Vec<String>,
    /// Compensation source of a conventional panel; `None` for spectral.
    pub compensation: Option<CompensationSource>,
}

pub struct PreflightRunner;

impl PreflightRunner {
    /// The files a run with `config` reads, with their provenance role: the
    /// manifest, the sample and control FCS files it lists, then the panels'
    /// compensation matrices.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest is invalid.
    pub fn input_files(
        config: &PreflightConfig,
    ) -> Result<Vec<(&'static str, PathBuf)>, PreflightError> {
        let manifest = RunManifest::load(&config.manifest)?;
        let mut files = vec![("manifest", config.manifest.clone())];
        files.extend(
            manifest
                .samples
                .iter()
                .map(|sample| ("input", sample.file.clone())),
        );
        files.extend(
            manifest
                .controls
                .iter()
                .map(|control| ("control", control.file.clone())),
        );
        files.extend(
            manifest
                .panels
                .iter()
                .filter_map(|panel| match &panel.compensation {
                    Some(CompensationSource::Matrix { matrix, .. }) => {
                        Some(("matrix", matrix.clone()))
                    }
                    _ => None,
                }),
        );
        Ok(files)
    }

    /// Check the manifest and its files and write the report, whatever the
    /// checks found; [`PreflightReport::ensure_passed`] turns failed checks
    /// into an error.
    ///
    /// # Errors
    ///
    /// Returns an error when the output exists without `overwrite`, the
    /// manifest is invalid, or the report cannot be written.
    pub fn run(config: &PreflightConfig) -> Result<PreflightReport, PreflightError> {
        if config.output.exists() && !config.overwrite {
            return Err(PreflightError::OutputExists {
                path: config.output.clone(),
            });
        }
        let manifest = RunManifest::load(&config.manifest)?;
        let files = manifest_files(&manifest);
        info!(
            "preflight: {} sample(s), {} control(s), {} panel(s)",
            manifest.samples.len(),
            manifest.controls.len(),
            manifest.panels.len()
        );
        let checks = run_checks(&manifest);
        let count = |status| checks.iter().filter(|check| check.status == status).count();
        let report = PreflightReport {
            tool: "cytiq preflight".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            manifest: config.manifest.display().to_string(),
            files: files
                .iter()
                .map(|file| file.display().to_string())
                .collect(),
            panels: manifest
                .panels
                .iter()
                .map(|panel| PanelSummary {
                    name: panel.label().to_owned(),
                    samples: samples_of(&manifest, panel)
                        .map(|sample| sample.id.clone())
                        .collect(),
                    controls: manifest
                        .controls_of(panel)
                        .map(|control| control.dye.clone())
                        .collect(),
                    compensation: manifest.compensation_of(panel).cloned(),
                })
                .collect(),
            status: checks
                .iter()
                .map(|check| check.status)
                .max()
                .unwrap_or(CheckStatus::Pass),
            passed: count(CheckStatus::Pass),
            warnings: count(CheckStatus::Warn),
            failures: count(CheckStatus::Fail),
            checks,
        };
        write_report(&config.output, &report)?;
        Ok(report)
    }
}

impl PreflightReport {
    /// `Ok` when no check failed.
    ///
    /// # Errors
    ///
    /// Returns [`PreflightError::Failed`] naming the report when any check
    /// failed.
    pub fn ensure_passed(&self, report_path: &Path) -> Result<(), PreflightError> {
        if self.failures == 0 {
            Ok(())
        } else {
            Err(PreflightError::Failed {
                failed: self.failures,
                report: report_path.to_path_buf(),
            })
        }
    }
}

fn run_checks(manifest: &RunManifest) -> Vec<Check> {
    let mut checks = Vec::new();
    // Each file is read once; panels refer to them by position.
    let mut samples = Vec::with_capacity(manifest.samples.len());
    for sample in &manifest.samples {
        let (file_checks, loaded) = check_file(&format!("sample {}", sample.id), &sample.file);
        checks.extend(file_checks);
        samples.push(loaded);
    }
    let mut controls = Vec::with_capacity(manifest.controls.len());
    for control in &manifest.controls {
        let scope = format!("control {}", control.dye);
        let (file_checks, loaded) = check_file(&scope, &control.file);
        checks.extend(file_checks);
        if let (Some(dataset), Some(channel)) = (&loaded.dataset, &control.primary_channel) {
            checks.push(check_primary_channel(&scope, dataset, channel));
        }
        controls.push(loaded);
    }
    for panel in &manifest.panels {
        checks.extend(check_panel(manifest, panel, &samples, &controls));
    }
    checks.extend(duplicate_files(manifest));
    checks
}

fn samples_of<'a>(
    manifest: &'a RunManifest,
    panel: &'a Panel,
) -> impl Iterator<Item = &'a SampleEntry> {
    manifest.samples.iter().filter(move |sample| {
        manifest
            .panel_of(sample)
            .is_some_and(|own| std::ptr::eq(own, panel))
    })
}

/// Markers, calibration and detector configuration of one panel's files.
fn check_panel(
    manifest: &RunManifest,
    panel: &Panel,
    samples: &[LoadedFile],
    controls: &[LoadedFile],
) -> Vec<Check> {
    let mut checks = Vec::new();
    let panel_samples = manifest
        .samples
        .iter()
        .zip(samples)
        .filter(|(sample, _)| {
            manifest
                .panel_of(sample)
                .is_some_and(|own| std::ptr::eq(own, panel))
        })
        .map(|(_, loaded)| loaded)
        .collect::<Vec<_>>();
    let panel_controls = manifest
        .controls
        .iter()
        .zip(controls)
        .filter(|(control, _)| {
            control.panel.is_none() || control.panel.as_deref() == panel.name.as_deref()
        })
        .collect::<Vec<(&ControlEntry, &LoadedFile)>>();
    for loaded in &panel_samples {
        if let Some(dataset) = &loaded.dataset {
            checks.extend(check_markers(&loaded.scope, dataset, panel));
        }
    }
    let reference = panel_samples
        .iter()
        .find_map(|loaded| loaded.dataset.as_ref());
    let scope = panel_scope(panel);
    if !panel_controls.is_empty() {
        checks.extend(check_control_dyes(&scope, &panel_controls));
    }
    match manifest.compensation_of(panel) {
        Some(CompensationSource::Fcs) => {
            for loaded in &panel_samples {
                if let Some(dataset) = &loaded.dataset {
                    checks.push(check_spillover(&loaded.scope, dataset));
                }
            }
        }
        Some(CompensationSource::Matrix { matrix, kind }) => {
            checks.extend(check_matrix(&scope, matrix, *kind, &panel_samples));
        }
        Some(CompensationSource::Controls) => {
            checks.push(check_control_coverage(
                &scope,
                panel,
                &panel_controls,
                reference,
                CheckStatus::Fail,
            ));
        }
        None => {
            checks.push(check_control_coverage(
                &scope,
                panel,
                &panel_controls,
                reference,
                CheckStatus::Warn,
            ));
        }
    }
    let files = panel_samples
        .iter()
        .copied()
        .chain(panel_controls.iter().map(|(_, loaded)| *loaded))
        .collect::<Vec<_>>();
    checks.extend(check_detectors(&files));
    checks
}

fn duplicate_files(manifest: &RunManifest) -> Vec<Check> {
    let mut paths = manifest
        .samples
        .iter()
        .map(|sample| sample.file.as_path())
        .chain(
            manifest
                .controls
                .iter()
                .map(|control| control.file.as_path()),
        )
        .collect::<Vec<&Path>>();
    paths.sort();
    paths
        .windows(2)
        .filter(|pair| pair[0] == pair[1])
        .map(|pair| Check {
            scope: "manifest".to_owned(),
            check: "unique_files",
            status: CheckStatus::Warn,
            message: format!("`{}` is listed more than once", pair[0].display()),
        })
        .collect()
}

fn write_report(path: &Path, report: &PreflightReport) -> Result<(), PreflightError> {
    debug!("writing preflight report to {}", path.display());
    let write_error = |source| PreflightError::Write {
        path: path.to_path_buf(),
        source,
    };
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent).map_err(write_error)?;
    }
    let mut bytes = serde_json::to_vec_pretty(report)
        .map_err(|error| write_error(std::io::Error::other(error)))?;
    bytes.push(b'\n');
    fs::write(path, bytes).map_err(write_error)
}

/// Sample then control FCS files of a manifest, resolved.
fn manifest_files(manifest: &RunManifest) -> Vec<PathBuf> {
    manifest
        .samples
        .iter()
        .map(|sample| sample.file.clone())
        .chain(manifest.controls.iter().map(|control| control.file.clone()))
        .collect()
}
