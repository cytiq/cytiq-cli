//! Recover only the exact temporary names owned by a locked destination.
use crate::{OutputLock, PublishError};
use std::fs;
use std::path::{Path, PathBuf};

/// List the target's six-character tempfile names, excluding other outputs.
/// # Errors
/// Returns directory read errors.
pub fn leftovers(target: &Path) -> Result<Vec<PathBuf>, PublishError> {
    let parent = target
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let Some(name) = target.file_name().and_then(|s| s.to_str()) else {
        return Ok(Vec::new());
    };
    let prefix = format!(".{name}.");
    let mut paths = Vec::new();
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(paths),
        Err(e) => return Err(PublishError::io(parent, e)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| PublishError::io(parent, e))?;
        let name = entry.file_name();
        let Some(rest) = name.to_str().and_then(|s| s.strip_prefix(&prefix)) else {
            continue;
        };
        let random = rest
            .strip_suffix(".old.tmp")
            .or_else(|| rest.strip_suffix(".tmp"));
        if random.is_some_and(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_alphanumeric())) {
            paths.push(entry.path());
        }
    }
    paths.sort();
    Ok(paths)
}

/// Restore an output moved aside before a crash, then discard its abandoned staging.
/// Ambiguous backups are preserved for inspection.
/// # Errors
/// Returns I/O errors or a conflict when recovery cannot choose one predecessor.
pub fn recover(lock: &OutputLock) -> Result<(), PublishError> {
    let target = lock.target();
    let paths = leftovers(target)?;
    if !target
        .try_exists()
        .map_err(|e| PublishError::io(target, e))?
    {
        let mut backups = Vec::new();
        for path in paths
            .iter()
            .filter(|p| p.to_string_lossy().ends_with(".old.tmp"))
        {
            let metadata = fs::symlink_metadata(path).map_err(|e| PublishError::io(path, e))?;
            if metadata.is_dir() {
                let previous = path.join("previous");
                match fs::symlink_metadata(&previous) {
                    Ok(_) => backups.push(previous),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        // An empty aside can be left before the original rename.
                        // Preserve anything we cannot identify as our predecessor.
                        if fs::read_dir(path)
                            .map_err(|e| PublishError::io(path, e))?
                            .next()
                            .is_some()
                        {
                            return Err(PublishError::Exists {
                                path: target.to_owned(),
                            });
                        }
                    }
                    Err(e) => return Err(PublishError::io(&previous, e)),
                }
            } else {
                backups.push(path.clone());
            }
        }
        if backups.len() > 1 {
            return Err(PublishError::Exists {
                path: target.to_owned(),
            });
        }
        if let Some(previous) = backups.first() {
            fs::rename(previous, target).map_err(|e| PublishError::io(target, e))?;
        }
    }
    for path in paths {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(PublishError::io(&path, e)),
        };
        let result = if metadata.is_dir() {
            fs::remove_dir_all(&path)
        } else {
            fs::remove_file(&path)
        };
        result.map_err(|e| PublishError::io(&path, e))?;
    }
    #[cfg(unix)]
    fs::File::open(target.parent().unwrap_or(Path::new(".")))
        .and_then(|f| f.sync_all())
        .map_err(|e| PublishError::io(target, e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_restores_only_the_locked_outputs_predecessor() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("result");
        let backup = temp.path().join(".result.abc123.old.tmp");
        fs::create_dir_all(backup.join("previous")).unwrap();
        fs::write(backup.join("previous/data"), b"previous").unwrap();
        fs::write(temp.path().join(".result.def456.tmp"), b"partial").unwrap();
        let unrelated = temp.path().join(".result.other.abc123.tmp");
        fs::write(&unrelated, b"keep").unwrap();
        let lock = OutputLock::acquire(&target).unwrap();
        assert_eq!(leftovers(&target).unwrap().len(), 2);
        recover(&lock).unwrap();
        assert_eq!(fs::read(target.join("data")).unwrap(), b"previous");
        assert_eq!(leftovers(&target).unwrap(), Vec::<PathBuf>::new());
        assert!(unrelated.exists());
    }

    #[test]
    fn a_previous_file_is_restored_when_directory_replacement_was_interrupted() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("result");
        let backup = temp.path().join(".result.abc123.old.tmp");
        fs::create_dir(&backup).unwrap();
        fs::write(backup.join("previous"), b"original file").unwrap();
        let lock = OutputLock::acquire(&target).unwrap();
        recover(&lock).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"original file");
        assert!(!backup.exists());
    }

    #[test]
    fn ambiguous_backups_are_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("result");
        for name in ["abc123", "def456"] {
            fs::create_dir_all(temp.path().join(format!(".result.{name}.old.tmp/previous")))
                .unwrap();
        }
        let lock = OutputLock::acquire(&target).unwrap();
        assert!(recover(&lock).is_err());
        assert_eq!(leftovers(&target).unwrap().len(), 2);
    }
}
