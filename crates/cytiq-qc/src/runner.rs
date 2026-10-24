use std::fs;
use std::path::{Path, PathBuf};

use cytiq_fcs::FcsReader;
use cytiq_manifest::RunManifest;
use cytiq_mask::{EventMask, MaskError};
use log::{debug, info};
use serde::Serialize;

use crate::error::QcError;
use crate::policy::{CheckStatus, QcCheck, QcMetrics, evaluate};

/// Configuration of `cytiq qc`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QcConfig {
    /// Run manifest holding the sample and its panel's `qc` policy.
    pub manifest: PathBuf,
    /// Sample id in the manifest.
    pub sample: String,
    /// The sample's final event mask, usually from `cytiq mask combine`.
    pub mask: PathBuf,
    /// JSON report path.
    pub output: PathBuf,
    pub overwrite: bool,
}

/// The `qc.json` report.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct QcReport {
    pub tool: String,
    pub version: String,
    pub manifest: String,
    pub sample: String,
    pub panel: String,
    pub mask: String,
    /// SHA-256 of the sample's raw acquisition.
    pub acquisition: String,
    /// The worst check status.
    pub status: CheckStatus,
    pub passed: usize,
    pub warnings: usize,
    pub failures: usize,
    /// `None` when the mask belongs to another acquisition.
    pub metrics: Option<QcMetrics>,
    pub checks: Vec<QcCheck>,
}

pub struct QcRunner;

impl QcRunner {
    /// The files a run with `config` reads: the manifest, the sample's raw
    /// FCS and the mask.
    ///
    /// # Errors
    ///
    /// Returns an error when the manifest is invalid or lacks the sample.
    pub fn input_files(config: &QcConfig) -> Result<Vec<PathBuf>, QcError> {
        let manifest = RunManifest::load(&config.manifest)?;
        let sample = find_sample(&manifest, config)?;
        Ok(vec![
            config.manifest.clone(),
            sample.file.clone(),
            config.mask.clone(),
        ])
    }

    /// Check the mask against the sample's panel policy and write the report,
    /// whatever the checks found; [`QcReport::ensure_passed`] turns failed
    /// checks into an error.
    ///
    /// # Errors
    ///
    /// Returns an error when the output exists without `overwrite`, the
    /// manifest is invalid or lacks the sample, the sample FCS or the mask
    /// cannot be read, or the report cannot be written.
    ///
    /// # Panics
    ///
    /// Never: [`RunManifest::load`] validates that every sample has a panel.
    pub fn run(config: &QcConfig) -> Result<QcReport, QcError> {
        if config.output.exists() && !config.overwrite {
            return Err(QcError::OutputExists {
                path: config.output.clone(),
            });
        }
        let manifest = RunManifest::load(&config.manifest)?;
        let sample = find_sample(&manifest, config)?;
        let panel = manifest
            .panel_of(sample)
            .expect("a validated manifest links every sample to a panel");
        let read_error = |source| QcError::FcsRead {
            path: sample.file.clone(),
            source,
        };
        let dataset = FcsReader::open(&sample.file)
            .and_then(FcsReader::read)
            .map_err(read_error)?;
        let acquisition =
            cytiq_fcs::acquisition_id(&sample.file, &dataset.text).map_err(read_error)?;
        let mask = EventMask::read_parquet(&config.mask)?;
        info!(
            "qc: sample `{}` (panel `{}`), {} raw event(s), mask {}",
            sample.id,
            panel.label(),
            dataset.event_count(),
            config.mask.display()
        );

        let scope = format!("sample {}", sample.id);
        let mut checks = Vec::new();
        let metrics = mask_checks(&scope, &mask, &dataset, &acquisition, &mut checks)?;
        if let Some(metrics) = &metrics {
            match &panel.qc {
                Some(policy) => checks.extend(evaluate(&scope, policy, metrics)),
                None => checks.push(QcCheck::new(
                    &scope,
                    "policy",
                    CheckStatus::Pass,
                    format!("panel `{}` has no `qc` policy", panel.label()),
                )),
            }
        }
        let count = |status| checks.iter().filter(|check| check.status == status).count();
        let report = QcReport {
            tool: "cytiq qc".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            manifest: config.manifest.display().to_string(),
            sample: sample.id.clone(),
            panel: panel.label().to_owned(),
            mask: config.mask.display().to_string(),
            acquisition,
            status: checks
                .iter()
                .map(|check| check.status)
                .max()
                .unwrap_or(CheckStatus::Pass),
            passed: count(CheckStatus::Pass),
            warnings: count(CheckStatus::Warn),
            failures: count(CheckStatus::Fail),
            metrics,
            checks,
        };
        write_report(&config.output, &report)?;
        Ok(report)
    }
}

impl QcReport {
    /// `Ok` when no check failed.
    ///
    /// # Errors
    ///
    /// Returns [`QcError::Failed`] naming the report when any check failed.
    pub fn ensure_passed(&self, report_path: &Path) -> Result<(), QcError> {
        if self.failures == 0 {
            Ok(())
        } else {
            Err(QcError::Failed {
                failed: self.failures,
                report: report_path.to_path_buf(),
            })
        }
    }
}

fn find_sample<'a>(
    manifest: &'a RunManifest,
    config: &QcConfig,
) -> Result<&'a cytiq_manifest::SampleEntry, QcError> {
    manifest
        .samples
        .iter()
        .find(|sample| sample.id.trim() == config.sample.trim())
        .ok_or_else(|| QcError::UnknownSample {
            sample: config.sample.clone(),
            manifest: config.manifest.clone(),
        })
}

/// The mask belongs to the sample's acquisition and covers every raw event;
/// metrics only when both hold.
fn mask_checks(
    scope: &str,
    mask: &EventMask,
    dataset: &cytiq_fcs::FcsDataset,
    acquisition: &str,
    checks: &mut Vec<QcCheck>,
) -> Result<Option<QcMetrics>, QcError> {
    let keep = match mask.keep_for(dataset, acquisition) {
        Ok(keep) => keep,
        Err(MaskError::AcquisitionMismatch { found, .. }) => {
            checks.push(QcCheck::new(
                scope,
                "mask_acquisition",
                CheckStatus::Fail,
                format!(
                    "the mask was computed on acquisition {found}, not this sample's {acquisition}"
                ),
            ));
            return Ok(None);
        }
        Err(MaskError::MissingEvent { source_event_id }) => {
            checks.push(QcCheck::new(
                scope,
                "mask_coverage",
                CheckStatus::Fail,
                format!(
                    "the mask has no row for raw event {source_event_id}: pass a mask over every raw event (put the `cytiq compensate` or `cytiq clean` mask first in `cytiq mask combine`)"
                ),
            ));
            return Ok(None);
        }
        Err(error) => return Err(error.into()),
    };
    checks.push(QcCheck::new(
        scope,
        "mask_coverage",
        CheckStatus::Pass,
        format!("the mask covers all {} raw event(s)", keep.len()),
    ));
    let kept = keep.iter().filter(|kept| **kept).count();
    Ok(Some(QcMetrics::of(mask, keep.len(), kept)))
}

fn write_report(path: &Path, report: &QcReport) -> Result<(), QcError> {
    debug!("writing QC report to {}", path.display());
    let write_error = |source| QcError::Write {
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
