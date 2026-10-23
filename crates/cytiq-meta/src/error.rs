use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MetaError {
    #[error("failed to read FCS input `{path}`: {source}")]
    FcsRead {
        path: PathBuf,
        #[source]
        source: cytiq_fcs::Error,
    },
    #[error("failed to hash `{path}`: {source}")]
    Hash {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("output `{path}` already exists; pass --overwrite to replace it")]
    OutputExists { path: PathBuf },
    #[error("failed to create output directory `{path}`: {source}")]
    OutputDirectoryCreate {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to write `{path}`: {source}")]
    OutputWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize FCS metadata: {0}")]
    Serialize(#[from] serde_json::Error),
}
