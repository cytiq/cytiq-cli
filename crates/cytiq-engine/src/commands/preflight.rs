use std::path::PathBuf;

use clap::Args;
use cytiq_preflight::{CheckStatus, PreflightConfig, PreflightRunner};
use cytiq_provenance::{RunRecorder, record_path_for_file};
use log::{debug, error, info, warn};
use serde::Serialize;

use crate::provenance::record_provenance;

#[derive(Debug, Serialize, Args)]
pub(crate) struct PreflightArgs {
    /// Run manifest (`.yaml`, `.yml` or `.json`).
    #[arg(long)]
    manifest: PathBuf,
    /// JSON report path.
    #[arg(long)]
    output: PathBuf,
    /// Replace an existing report.
    #[arg(long)]
    overwrite: bool,
}

pub(crate) fn run(
    args: PreflightArgs,
    mut recorder: RunRecorder,
) -> Result<(), Box<dyn std::error::Error>> {
    let PreflightArgs {
        manifest,
        output,
        overwrite,
    } = args;
    info!(
        "starting preflight: manifest={}, output={}",
        manifest.display(),
        output.display()
    );
    let config = PreflightConfig {
        manifest,
        output,
        overwrite,
    };
    // An invalid manifest fails the run below with the same error.
    let inputs = PreflightRunner::input_files(&config)
        .unwrap_or_else(|_| vec![("manifest", config.manifest.clone())]);
    recorder.snapshot_roles(&inputs)?;
    let report = PreflightRunner::run(&config)?;
    for check in &report.checks {
        match check.status {
            CheckStatus::Fail => {
                error!("{}: {}: {}", check.scope, check.check, check.message);
            }
            CheckStatus::Warn => {
                warn!("{}: {}: {}", check.scope, check.check, check.message);
            }
            CheckStatus::Pass => {
                debug!("{}: {}: {}", check.scope, check.check, check.message);
            }
        }
    }
    info!(
        "preflight: {} passed, {} warning(s), {} failure(s); wrote {}",
        report.passed,
        report.warnings,
        report.failures,
        config.output.display()
    );
    record_provenance(
        recorder,
        std::slice::from_ref(&config.output),
        &record_path_for_file(&config.output),
    )?;
    report.ensure_passed(&config.output)?;
    Ok(())
}
