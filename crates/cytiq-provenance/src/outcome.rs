//! Completion records for the population/specimen command families.

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
}
