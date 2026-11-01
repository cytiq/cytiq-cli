use std::env;
use std::path::{Path, PathBuf};

use cytiq_provenance::{BuildInfo, RunRecorder};
use log::debug;

/// What `build.rs` recorded about this binary.
pub(crate) fn build_info() -> BuildInfo {
    BuildInfo {
        git_commit: env!("CYTIQ_GIT_COMMIT").to_owned(),
        rustc: env!("CYTIQ_RUSTC_VERSION").to_owned(),
        target: env!("CYTIQ_TARGET").to_owned(),
        profile: env!("CYTIQ_PROFILE").to_owned(),
    }
}

/// Write the run's `provenance.json`: tool, arguments, the hashes of the
/// inputs as snapshotted before the run, and of the files it wrote.
pub(crate) fn record_provenance(
    recorder: RunRecorder,
    outputs: &[PathBuf],
    record_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    recorder.finish(outputs, record_path)?;
    debug!("wrote provenance record {}", record_path.display());
    Ok(())
}
