//! The 0–4 exit contract of every `cytiq` command and its JSON completion record.
//!
//! `0` success, `1` runtime error (the only code worth retrying), `2` invalid
//! input, configuration or corrupt data, `3` completed but checks failed, `4`
//! output conflict, missing input or incomplete result.

use std::io::{self, Write as _};
use std::path::Path;

use cytiq_publish::{Existing, PublishError};
use serde::{Deserialize, Serialize};

use crate::FileRecord;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeReason {
    Success,
    RuntimeError,
    InvalidConfiguration,
    ChecksFailed,
    OutputConflict,
    Incomplete,
}

impl OutcomeReason {
    #[must_use]
    pub const fn exit_code(self) -> u8 {
        match self {
            Self::Success => 0,
            Self::RuntimeError => 1,
            Self::InvalidConfiguration => 2,
            Self::ChecksFailed => 3,
            Self::OutputConflict | Self::Incomplete => 4,
        }
    }

    /// Classify an I/O failure by kind: a missing path is incomplete input, an
    /// existing one a conflict, malformed or truncated data invalid input;
    /// anything else (permissions, interruptions, a full disk) is a runtime
    /// error.
    #[must_use]
    pub const fn of_io_kind(kind: io::ErrorKind) -> Self {
        use io::ErrorKind as K;
        match kind {
            K::NotFound | K::NotADirectory => Self::Incomplete,
            K::AlreadyExists | K::DirectoryNotEmpty => Self::OutputConflict,
            K::InvalidInput
            | K::InvalidData
            | K::UnexpectedEof
            | K::IsADirectory
            | K::InvalidFilename => Self::InvalidConfiguration,
            _ => Self::RuntimeError,
        }
    }

    #[must_use]
    pub fn of_io(error: &io::Error) -> Self {
        Self::of_io_kind(error.kind())
    }

    /// An existing output is a conflict; I/O follows [`Self::of_io`]; a
    /// staging bug or a failed rollback is a runtime error.
    #[must_use]
    pub fn of_publish(error: &PublishError) -> Self {
        match error {
            PublishError::Exists { .. } => Self::OutputConflict,
            PublishError::Io { source, .. } => Self::of_io(source),
            PublishError::StagedTwice { .. } | PublishError::RollbackFailed { .. } => {
                Self::RuntimeError
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommandOutcome {
    pub schema_version: u32,
    pub command: String,
    pub completed: bool,
    pub reason: OutcomeReason,
    pub exit_code: u8,
    pub message: String,
    /// Paths are relative to the package; never includes outcome.json itself.
    pub artifacts: Vec<FileRecord>,
}

impl CommandOutcome {
    #[must_use]
    pub fn new(command: &str, completed: bool, reason: OutcomeReason, message: String) -> Self {
        Self {
            schema_version: 1,
            command: command.to_owned(),
            completed,
            reason,
            exit_code: reason.exit_code(),
            message,
            artifacts: Vec::new(),
        }
    }

    /// Publish the outcome atomically as pretty JSON, replacing an earlier one
    /// (a retried step writes to the same path).
    /// # Errors
    /// Fails when the file cannot be written, e.g. its parent is a file.
    pub fn write(&self, path: &Path) -> Result<(), PublishError> {
        let mut bytes = serde_json::to_vec_pretty(self)
            .map_err(|error| PublishError::io(path, error.into()))?;
        bytes.push(b'\n');
        cytiq_publish::write_file(path, Existing::Replace, |writer| {
            writer
                .write_all(&bytes)
                .map_err(|source| PublishError::io(path, source))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn io_kinds_map_to_the_exit_contract() {
        use io::ErrorKind as K;
        for (kind, code) in [
            (K::NotFound, 4),
            (K::NotADirectory, 4),
            (K::AlreadyExists, 4),
            (K::DirectoryNotEmpty, 4),
            (K::InvalidInput, 2),
            (K::InvalidData, 2),
            (K::UnexpectedEof, 2),
            (K::IsADirectory, 2),
            (K::InvalidFilename, 2),
            (K::PermissionDenied, 1),
            (K::Interrupted, 1),
            (K::StorageFull, 1),
            (K::Other, 1),
        ] {
            assert_eq!(
                OutcomeReason::of_io_kind(kind).exit_code(),
                code,
                "{kind:?}"
            );
        }
        assert_eq!(
            OutcomeReason::of_io_kind(K::NotFound),
            OutcomeReason::Incomplete
        );
        assert_eq!(
            OutcomeReason::of_io_kind(K::AlreadyExists),
            OutcomeReason::OutputConflict
        );
    }

    #[test]
    fn publish_errors_distinguish_conflicts_from_io_and_internal_failures() {
        let exists = PublishError::Exists {
            path: PathBuf::from("out.csv"),
        };
        assert_eq!(
            OutcomeReason::of_publish(&exists),
            OutcomeReason::OutputConflict
        );
        let missing = PublishError::io("out/a.csv", io::Error::from(io::ErrorKind::NotFound));
        assert_eq!(
            OutcomeReason::of_publish(&missing),
            OutcomeReason::Incomplete
        );
        let denied = PublishError::io("a", io::Error::from(io::ErrorKind::PermissionDenied));
        assert_eq!(
            OutcomeReason::of_publish(&denied),
            OutcomeReason::RuntimeError
        );
        let twice = PublishError::StagedTwice {
            path: PathBuf::from("a"),
        };
        assert_eq!(
            OutcomeReason::of_publish(&twice),
            OutcomeReason::RuntimeError
        );
        let rollback = PublishError::RollbackFailed {
            source: Box::new(exists),
            failures: Vec::new(),
        };
        assert_eq!(
            OutcomeReason::of_publish(&rollback),
            OutcomeReason::RuntimeError
        );
    }

    #[test]
    fn write_replaces_an_earlier_outcome_and_round_trips() {
        let directory =
            std::env::temp_dir().join(format!("cytiq-outcome-{}-{}", std::process::id(), line!()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("outcome.json");
        std::fs::write(&path, "stale").unwrap();
        let outcome = CommandOutcome::new(
            "cytiq qc",
            true,
            OutcomeReason::ChecksFailed,
            "1 check failed".into(),
        );
        outcome.write(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.ends_with('\n'));
        let read: CommandOutcome = serde_json::from_str(&text).unwrap();
        assert_eq!(read, outcome);
        assert_eq!(read.exit_code, 3);
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn write_below_a_file_fails() {
        let parent =
            std::env::temp_dir().join(format!("cytiq-outcome-file-{}", std::process::id()));
        std::fs::write(&parent, "not a directory").unwrap();
        let outcome =
            CommandOutcome::new("cytiq meta", true, OutcomeReason::Success, String::new());
        assert!(outcome.write(&parent.join("outcome.json")).is_err());
        std::fs::remove_file(&parent).unwrap();
    }
}
