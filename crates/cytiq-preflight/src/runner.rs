use std::fs;
use std::path::{Path, PathBuf};

use cytiq_manifest::{Cytometry, RunManifest};
use log::{debug, info};
use serde::Serialize;

use crate::checks::{
    Check, CheckStatus, check_detectors, check_file, check_markers, check_primary_channel,
    check_spillover,
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
    /// The worst check status.
    pub status: CheckStatus,
    pub passed: usize,
    pub warnings: usize,
    pub failures: usize,
    pub checks: Vec<Check>,
}

pub struct PreflightRunner;

impl PreflightRunner {
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
        info!(
            "preflight: {} sample(s), {} control(s), {} marker(s)",
            manifest.samples.len(),
            manifest.controls.len(),
            manifest.panel.markers.len()
        );
        let checks = run_checks(&manifest);
        let count = |status| checks.iter().filter(|check| check.status == status).count();
        let report = PreflightReport {
            tool: "cytiq preflight".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            manifest: config.manifest.display().to_string(),
            files: manifest
                .samples
                .iter()
                .map(|sample| sample.file.display().to_string())
                .chain(
                    manifest
                        .controls
                        .iter()
                        .map(|control| control.file.display().to_string()),
                )
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
    let mut files = Vec::new();
    for sample in &manifest.samples {
        let scope = format!("sample {}", sample.id);
        let (file_checks, loaded) = check_file(&scope, &sample.file);
        checks.extend(file_checks);
        if let Some(dataset) = &loaded.dataset {
            checks.extend(check_markers(&scope, dataset, &manifest.panel));
            if manifest.cytometry == Cytometry::Conventional {
                checks.push(check_spillover(&scope, dataset));
            }
        }
        files.push(loaded);
    }
    for control in &manifest.controls {
        let scope = format!("control {}", control.dye);
        let (file_checks, loaded) = check_file(&scope, &control.file);
        checks.extend(file_checks);
        if let (Some(dataset), Some(channel)) = (&loaded.dataset, &control.primary_channel) {
            checks.push(check_primary_channel(&scope, dataset, channel));
        }
        files.push(loaded);
    }
    checks.extend(check_detectors(&files));
    checks.extend(duplicate_files(manifest));
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
