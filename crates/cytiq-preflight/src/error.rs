use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum PreflightError {
    #[error(transparent)]
    Manifest(#[from] cytiq_manifest::ManifestError),
    #[error("output `{path}` already exists; pass --overwrite to replace it")]
    OutputExists { path: PathBuf },
    #[error("failed to write preflight report `{path}`: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("preflight failed: {failed} check(s) failed; see `{report}`")]
    Failed { failed: usize, report: PathBuf },
}
