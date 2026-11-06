use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};

use crate::PublishError;

/// Kernel lock beside the output, held through recovery, assessment and publication.
#[derive(Debug)]
pub struct OutputLock {
    file: File,
    path: PathBuf,
    target: PathBuf,
}

impl OutputLock {
    /// # Errors
    /// Fails immediately if another process owns the target or locking is unsupported.
    pub fn acquire(target: &Path) -> Result<Self, PublishError> {
        // `--output .` is a valid directory destination. Resolve only nameless
        // components; an ordinary symlink output must keep its own lock name.
        let resolved = if target.file_name().is_none() && !target.as_os_str().is_empty() {
            fs::canonicalize(target).map_err(|e| PublishError::io(target, e))?
        } else {
            target.to_path_buf()
        };
        let target = resolved.as_path();
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::create_dir_all(parent).map_err(|e| PublishError::io(target, e))?;
        let parent = fs::canonicalize(parent).map_err(|e| PublishError::io(target, e))?;
        let name = target.file_name().ok_or_else(|| {
            PublishError::io(
                target,
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "output needs a name"),
            )
        })?;
        let target = parent.join(name);
        let mut lock_name = std::ffi::OsString::from(".");
        lock_name.push(name);
        lock_name.push(".cytiq-lock");
        let path = parent.join(lock_name);
        loop {
            let file = OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .map_err(|e| PublishError::io(&path, e))?;
            match file.try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => return Err(PublishError::Locked { path: target }),
                Err(TryLockError::Error(e)) => return Err(PublishError::io(&path, e)),
            }
            // A waiter may have opened the old inode just before its owner
            // unlinked it. Never work under a lock that no longer names us.
            if same_file(&file, &path) {
                return Ok(Self { file, path, target });
            }
        }
    }

    #[must_use]
    pub fn target(&self) -> &Path {
        &self.target
    }
}

#[cfg(unix)]
fn same_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    file.metadata()
        .ok()
        .zip(fs::metadata(path).ok())
        .is_some_and(|(a, b)| a.dev() == b.dev() && a.ino() == b.ino())
}

#[cfg(not(unix))]
fn same_file(_file: &File, path: &Path) -> bool {
    path.exists()
}

impl Drop for OutputLock {
    fn drop(&mut self) {
        if same_file(&self.file, &self.path) {
            let _ = fs::remove_file(&self.path);
        }
        // File closes after unlink, releasing the kernel lock even after a crash.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_current_directory_can_be_locked_without_changing_process_cwd() {
        let lock = OutputLock::acquire(Path::new(".")).unwrap();
        assert_eq!(lock.target(), fs::canonicalize(".").unwrap());
        assert!(matches!(
            OutputLock::acquire(Path::new(".")),
            Err(PublishError::Locked { .. })
        ));
    }

    #[test]
    fn lock_refuses_a_second_owner_and_disappears_on_release() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("result");
        let lock = OutputLock::acquire(&target).unwrap();
        assert!(matches!(
            OutputLock::acquire(&target),
            Err(PublishError::Locked { .. })
        ));
        assert!(temp.path().join(".result.cytiq-lock").exists());
        drop(lock);
        assert!(!temp.path().join(".result.cytiq-lock").exists());
        OutputLock::acquire(&target).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn replaced_inode_is_not_accepted_or_unlinked_by_the_old_owner() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("result");
        let old = OutputLock::acquire(&target).unwrap();
        fs::remove_file(&old.path).unwrap();
        let current = OutputLock::acquire(&target).unwrap();
        assert!(!same_file(&old.file, &old.path));
        drop(old);
        assert!(current.path.exists());
        assert!(matches!(
            OutputLock::acquire(&target),
            Err(PublishError::Locked { .. })
        ));
    }
}
