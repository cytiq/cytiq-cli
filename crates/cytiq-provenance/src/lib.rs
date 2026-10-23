//! Run provenance: which command produced which files from which inputs.
//!
//! Each cytiq command that writes files also writes a `provenance.json`
//! record next to its output: the tool and version, the exact command-line
//! arguments, start and finish times, and the size and SHA-256 of every input
//! and output file.

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use log::debug;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ProvenanceError {
    #[error("failed to hash `{path}` for provenance: {source}")]
    Hash {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to write provenance record `{path}`: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

/// Size and SHA-256 of one file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

impl FileRecord {
    /// Hash `path`.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read.
    pub fn of(path: &Path) -> Result<Self, ProvenanceError> {
        let (bytes, sha256) = hash_file(path).map_err(|source| ProvenanceError::Hash {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            path: path.display().to_string(),
            bytes,
            sha256,
        })
    }
}

/// The `provenance.json` document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRecord {
    pub schema_version: u32,
    /// `cytiq <command>`.
    pub tool: String,
    pub version: String,
    /// Command-line arguments after the program name, exactly as given.
    pub arguments: Vec<String>,
    /// Milliseconds since the Unix epoch.
    pub started_at_unix_ms: u128,
    pub finished_at_unix_ms: u128,
    pub inputs: Vec<FileRecord>,
    pub outputs: Vec<FileRecord>,
}

/// Collects a run's record: start it before the command runs, finish it
/// with the files the command read and wrote.
#[derive(Clone, Debug)]
pub struct RunRecorder {
    tool: String,
    version: String,
    arguments: Vec<String>,
    started_at_unix_ms: u128,
}

impl RunRecorder {
    #[must_use]
    pub fn start(
        tool: impl Into<String>,
        version: impl Into<String>,
        arguments: Vec<String>,
    ) -> Self {
        Self {
            tool: tool.into(),
            version: version.into(),
            arguments,
            started_at_unix_ms: unix_ms(),
        }
    }

    /// Hash `inputs` and `outputs` and write the record to `record_path`
    /// (through a `.tmp` sibling, so a failed run never leaves half a record).
    /// Paths that are not files (missing optional outputs, directories) are
    /// skipped.
    ///
    /// # Errors
    ///
    /// Returns an error when a file cannot be hashed or the record cannot be
    /// written.
    pub fn finish(
        self,
        inputs: &[PathBuf],
        outputs: &[PathBuf],
        record_path: &Path,
    ) -> Result<RunRecord, ProvenanceError> {
        let records = |paths: &[PathBuf]| {
            paths
                .iter()
                .filter(|path| path.is_file())
                .map(|path| FileRecord::of(path))
                .collect::<Result<Vec<_>, _>>()
        };
        let record = RunRecord {
            schema_version: SCHEMA_VERSION,
            tool: self.tool,
            version: self.version,
            arguments: self.arguments,
            started_at_unix_ms: self.started_at_unix_ms,
            finished_at_unix_ms: unix_ms(),
            inputs: records(inputs)?,
            outputs: records(outputs)?,
        };
        write_record(record_path, &record)?;
        Ok(record)
    }
}

/// `sample.comp.fcs` -> `sample.comp.fcs.provenance.json`.
#[must_use]
pub fn record_path_for_file(output: &Path) -> PathBuf {
    let mut name = output.as_os_str().to_owned();
    name.push(".provenance.json");
    PathBuf::from(name)
}

/// `clean/` -> `clean/provenance.json`.
#[must_use]
pub fn record_path_for_dir(output_dir: &Path) -> PathBuf {
    output_dir.join("provenance.json")
}

/// Every file under `dir`, recursively and sorted, except provenance records
/// and temporary files. A missing directory yields nothing.
#[must_use]
pub fn files_under(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if !is_record_or_temporary(&path) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn is_record_or_temporary(path: &Path) -> bool {
    let temporary = path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("tmp"));
    let record = path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "provenance.json" || name.ends_with(".provenance.json"));
    temporary || record
}

/// Size and lowercase hex SHA-256 of a file, streamed.
///
/// # Errors
///
/// Returns an error when the file cannot be read.
pub fn hash_file(path: &Path) -> io::Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let bytes = io::copy(&mut file, &mut hasher)?;
    let sha256 = hasher
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    Ok((bytes, sha256))
}

fn write_record(path: &Path, record: &RunRecord) -> Result<(), ProvenanceError> {
    debug!("writing provenance record to {}", path.display());
    let write_error = |source| ProvenanceError::Write {
        path: path.to_path_buf(),
        source,
    };
    let mut bytes = serde_json::to_vec_pretty(record).map_err(|error| write_error(error.into()))?;
    bytes.push(b'\n');
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    fs::write(&temporary, bytes).map_err(write_error)?;
    fs::rename(&temporary, path).map_err(write_error)
}

fn unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_inputs_and_outputs_with_hashes() {
        let directory =
            std::env::temp_dir().join(format!("cytiq-provenance-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let input = directory.join("in.txt");
        let output = directory.join("out.txt");
        fs::write(&input, b"abc").unwrap();
        fs::write(&output, b"").unwrap();
        let record_path = record_path_for_file(&output);

        let record = RunRecorder::start("cytiq test", "0.0.0", vec!["--input".to_owned()])
            .finish(
                &[input, directory.join("missing.txt")],
                &[output],
                &record_path,
            )
            .unwrap();

        assert_eq!(record.inputs.len(), 1, "missing paths are skipped");
        assert_eq!(
            record.inputs[0].sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(record.outputs[0].bytes, 0);
        assert!(record.finished_at_unix_ms >= record.started_at_unix_ms);
        let written: RunRecord =
            serde_json::from_str(&fs::read_to_string(&record_path).unwrap()).unwrap();
        assert_eq!(written, record);
        assert!(record_path.ends_with("out.txt.provenance.json"));
    }
}
