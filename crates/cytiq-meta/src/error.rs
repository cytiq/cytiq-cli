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
    #[error("failed to write `{path}`: {source}")]
    OutputWrite {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to serialize FCS metadata: {0}")]
    Serialize(#[from] serde_json::Error),
}

impl From<cytiq_publish::PublishError> for MetaError {
    fn from(error: cytiq_publish::PublishError) -> Self {
        use cytiq_publish::PublishError;
        match error {
            PublishError::Exists { path } => Self::OutputExists { path },
            PublishError::Io { path, source } => Self::OutputWrite { path, source },
            PublishError::StagedTwice { .. } | PublishError::RollbackFailed { .. } => {
                Self::OutputWrite {
                    path: error.path().to_path_buf(),
                    source: std::io::Error::other(error.to_string()),
                }
            }
        }
    }
}
