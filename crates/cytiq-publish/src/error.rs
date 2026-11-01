use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PublishError {
    #[error("{} already exists; pass --overwrite to replace it", path.display())]
    Exists { path: PathBuf },
    #[error("{} is staged more than once", path.display())]
    StagedTwice { path: PathBuf },
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// A commit failed with `source` and could not put back everything it
    /// had published.
    #[error("{source}; rolling back the commit failed: {}", joined(failures))]
    RollbackFailed {
        #[source]
        source: Box<PublishError>,
        failures: Vec<RollbackFailure>,
    },
}

impl PublishError {
    pub fn io(path: impl AsRef<Path>, source: io::Error) -> Self {
        Self::Io {
            path: path.as_ref().to_path_buf(),
            source,
        }
    }

    /// The path the error is about; for `RollbackFailed`, that of the error
    /// that failed the commit.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::Exists { path } | Self::StagedTwice { path } | Self::Io { path, .. } => path,
            Self::RollbackFailed { source, .. } => source.path(),
        }
    }
}

/// A step of a failed commit that could not be undone.
#[derive(Debug, thiserror::Error)]
#[error("{}{}: {source}", target.display(), kept_at(previous.as_deref()))]
pub struct RollbackFailure {
    /// The output, or the directory whose sync failed.
    pub target: PathBuf,
    /// Where the output it replaced is kept, when it replaced one.
    pub previous: Option<PathBuf>,
    #[source]
    pub source: io::Error,
}

fn kept_at(previous: Option<&Path>) -> String {
    previous
        .map(|previous| format!(" (previous output kept at {})", previous.display()))
        .unwrap_or_default()
}

fn joined(failures: &[RollbackFailure]) -> String {
    failures
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn denied() -> io::Error {
        io::Error::new(io::ErrorKind::PermissionDenied, "denied")
    }

    #[test]
    fn rollback_failed_names_the_error_and_every_step_it_could_not_undo() {
        let error = PublishError::RollbackFailed {
            source: Box::new(PublishError::io("out/b.csv", denied())),
            failures: vec![
                RollbackFailure {
                    target: PathBuf::from("out/a.csv"),
                    previous: Some(PathBuf::from("out/.a.csv.x1y2z3.old.tmp")),
                    source: denied(),
                },
                RollbackFailure {
                    target: PathBuf::from("out/model"),
                    previous: None,
                    source: denied(),
                },
            ],
        };
        assert_eq!(
            error.to_string(),
            "out/b.csv: denied; rolling back the commit failed: \
             out/a.csv (previous output kept at out/.a.csv.x1y2z3.old.tmp): denied; \
             out/model: denied"
        );
        assert_eq!(error.path(), Path::new("out/b.csv"));
    }
}
