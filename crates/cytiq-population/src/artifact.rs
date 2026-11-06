//! Byte references and complete directory publication.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use cytiq_provenance::outcome::{CommandOutcome, OutcomeReason};
use cytiq_provenance::{FileRecord, RunRecorder, hash_file};
use cytiq_publish::{Existing, Publication};
use serde::{Deserialize, Serialize};

use crate::{PopulationError, Result, require, serialization};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub path: PathBuf,
    pub sha256: String,
}

impl ArtifactRef {
    /// Snapshot an existing file by byte identity.
    /// # Errors
    /// Returns an error for a missing or unreadable file.
    pub fn of(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(PopulationError::Missing(path.to_owned()));
        }
        Ok(Self {
            path: std::fs::canonicalize(path)?,
            sha256: hash_file(path)?.1,
        })
    }

    /// # Errors
    /// Rejects missing files and changed bytes.
    pub fn verify(&self) -> Result<()> {
        require(
            Self::of(&self.path)?.sha256 == self.sha256,
            format!("artifact checksum mismatch: {}", self.path.display()),
        )
    }
}

/// Write JSON with LF; the containing Publication owns durability.
/// # Errors
/// Returns serialization or filesystem errors.
pub fn write_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    std::fs::write(path, bytes)?;
    Ok(())
}

/// A verified reuse decision or a new package run holding the destination lock.
pub enum PackageBegin {
    Reused(CommandOutcome),
    Compute(Box<PackageRun>),
}
pub struct PackageRun {
    output: PathBuf,
    overwrite: bool,
    recorder: RunRecorder,
    _lock: cytiq_publish::OutputLock,
}

/// Lock, recover and assess a package before computing its contents.
/// # Errors
/// Rejects locked or conflicting destinations and invalid policy combinations.
pub fn begin_package(
    output: &Path,
    overwrite: bool,
    command: &str,
    policy: cytiq_provenance::reuse::ReusePolicy,
    recorder: RunRecorder,
) -> Result<PackageBegin> {
    use cytiq_provenance::{
        outcome::Execution,
        reuse::{self, Decision, Destination, Prior, ReusePolicy},
    };
    require(
        !(overwrite && policy == ReusePolicy::Reject),
        "--reuse reject cannot be combined with --overwrite",
    )?;
    let lock = cytiq_publish::OutputLock::acquire(output)?;
    cytiq_publish::recovery::recover(&lock)?;
    if policy != ReusePolicy::Recompute {
        let mut prior = reuse::assess(
            &Destination::Dir(output.to_owned()),
            &recorder.snapshot_record(),
        );
        let mut saved = None;
        if let Prior::Match(record) = &prior {
            match verify_package(output) {
                Ok(outcome)
                    if outcome.command == command && Some(outcome.reason) == record.reason =>
                {
                    saved = Some(outcome);
                }
                _ => prior = Prior::Damaged,
            }
        }
        match reuse::decide(policy, &prior) {
            Decision::Reuse => {
                let Some(mut outcome) = saved else {
                    return Err(PopulationError::Invalid("missing verified outcome".into()));
                };
                let Prior::Match(record) = prior else {
                    return Err(PopulationError::Invalid(
                        "missing verified provenance".into(),
                    ));
                };
                recorder.verify_inputs()?;
                outcome.execution = Some(Execution {
                    fingerprint: record.compute_fingerprint(),
                    reused: true,
                });
                return Ok(PackageBegin::Reused(outcome));
            }
            Decision::Conflict => {
                return Err(cytiq_publish::PublishError::Exists {
                    path: output.to_owned(),
                }
                .into());
            }
            Decision::Compute => {}
        }
    }
    Ok(PackageBegin::Compute(Box::new(PackageRun {
        output: output.to_owned(),
        overwrite,
        recorder,
        _lock: lock,
    })))
}

impl PackageRun {
    /// Stage and publish the package while retaining its kernel lock.
    /// # Errors
    /// Returns validation, serialization and publication errors.
    pub fn write(
        self,
        command: &str,
        reason: OutcomeReason,
        write: impl FnOnce(&Path) -> Result<Vec<String>>,
    ) -> Result<CommandOutcome> {
        write_package_locked(
            &self.output,
            self.overwrite,
            self.recorder,
            command,
            reason,
            write,
        )
    }
}

/// Stage data, provenance and outcome in one directory swap.
/// # Errors
/// A failure before commit preserves the preceding package.
pub fn write_package(
    output: &Path,
    overwrite: bool,
    recorder: RunRecorder,
    command: &str,
    reason: OutcomeReason,
    write: impl FnOnce(&Path) -> Result<Vec<String>>,
) -> Result<CommandOutcome> {
    match begin_package(
        output,
        overwrite,
        command,
        cytiq_provenance::reuse::ReusePolicy::Recompute,
        recorder,
    )? {
        PackageBegin::Reused(outcome) => Ok(outcome),
        PackageBegin::Compute(run) => run.write(command, reason, write),
    }
}

fn write_package_locked(
    output: &Path,
    overwrite: bool,
    recorder: RunRecorder,
    command: &str,
    reason: OutcomeReason,
    write: impl FnOnce(&Path) -> Result<Vec<String>>,
) -> Result<CommandOutcome> {
    let mut publication = Publication::new(Existing::overwrite(overwrite));
    let staged = publication.stage_dir(output)?;
    let names = write(&staged)?;
    let mut unique = BTreeSet::new();
    for name in &names {
        require(
            safe_name(name)
                && !["provenance.json", "outcome.json"].contains(&name.as_str())
                && unique.insert(name),
            "invalid or repeated package filename",
        )?;
    }
    let pairs = names
        .iter()
        .map(|name| (output.join(name), staged.join(name)))
        .collect::<Vec<_>>();
    let record = recorder.prepare_staged(&pairs, reason)?;
    let destination = if output.exists() {
        std::fs::canonicalize(output)?
    } else {
        std::fs::canonicalize(
            output
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .join(
            output
                .file_name()
                .ok_or_else(|| PopulationError::Invalid("output needs a directory name".into()))?,
        )
    };
    for input in &record.inputs {
        require(
            !std::fs::canonicalize(&input.path)?.starts_with(&destination),
            "output directory contains an input; refusing to replace it",
        )?;
    }
    write_json(&staged.join("provenance.json"), &record)?;
    let mut outcome = CommandOutcome::new(command, true, reason, String::new());
    for name in names.iter().map(String::as_str).chain(["provenance.json"]) {
        let mut artifact = FileRecord::of(&staged.join(name))?;
        name.clone_into(&mut artifact.path);
        outcome.artifacts.push(artifact);
    }
    outcome.execution = Some(cytiq_provenance::outcome::Execution {
        fingerprint: record
            .execution_fingerprint
            .clone()
            .expect("v3 fingerprint"),
        reused: false,
    });
    write_json(&staged.join("outcome.json"), &outcome)?;
    publication.commit()?;
    Ok(outcome)
}

/// Verify the completion marker and every artifact before reading a package.
/// # Errors
/// Rejects absent, corrupt, inconsistent and incomplete packages.
pub fn verify_package(directory: &Path) -> Result<CommandOutcome> {
    let path = directory.join("outcome.json");
    if !path.exists() {
        return Err(PopulationError::Missing(path));
    }
    let outcome: CommandOutcome = serde_json::from_value(serialization::read(&path)?)?;
    require(
        outcome.schema_version == 1
            && outcome.completed
            && outcome.exit_code == outcome.reason.exit_code()
            && matches!(
                outcome.reason,
                OutcomeReason::Success | OutcomeReason::ChecksFailed | OutcomeReason::Incomplete
            ),
        "invalid completion record",
    )?;
    let mut names = BTreeSet::new();
    for artifact in &outcome.artifacts {
        require(
            safe_name(&artifact.path)
                && artifact.path != "outcome.json"
                && names.insert(artifact.path.as_str()),
            "invalid outcome artifact path",
        )?;
        let actual = FileRecord::of(&directory.join(&artifact.path))?;
        require(
            actual.sha256 == artifact.sha256 && actual.bytes == artifact.bytes,
            format!("incomplete package: {}", artifact.path),
        )?;
    }
    require(
        names.contains("provenance.json") && names.len() > 1,
        "package has no recorded data or provenance",
    )?;
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or_else(|| PopulationError::Invalid("non-UTF8 package file".into()))?;
        require(
            name == "outcome.json" || names.contains(name),
            "unrecorded file in package",
        )?;
    }
    Ok(outcome)
}

fn safe_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\'])
}
