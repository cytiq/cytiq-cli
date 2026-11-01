use std::ffi::OsString;
use std::fs::{self, File, Permissions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};

use tempfile::{Builder, NamedTempFile, TempDir};

use crate::PublishError;

/// What a publication does with a target that already exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Existing {
    /// Fail: when the output is staged, before it is written, and again at
    /// commit.
    Refuse,
    /// Replace it at commit; until then the previous output stays in place.
    Replace,
}

impl Existing {
    /// `Replace` when the user passed `--overwrite`, `Refuse` otherwise.
    #[must_use]
    pub fn overwrite(overwrite: bool) -> Self {
        if overwrite {
            Self::Replace
        } else {
            Self::Refuse
        }
    }
}

/// The writer [`Publication::stage_file`] hands out. The publication flushes
/// and syncs it; the closure only writes.
pub type Staging<'a> = BufWriter<&'a mut File>;

enum Staged {
    File(NamedTempFile),
    Directory(TempDir),
}

struct Entry {
    target: PathBuf,
    staged: Staged,
}

/// A set of outputs staged next to their targets and published together.
///
/// Temporary names are `.<target name>.<random>.tmp` in the target's
/// directory, so publishing is a rename on one file system. Files get the
/// permissions `File::create` would give them, directories those of
/// `fs::create_dir`. Dropping the publication without a successful
/// [`commit`](Self::commit) removes everything it staged and every parent
/// directory it created that is still empty.
pub struct Publication {
    existing: Existing,
    entries: Vec<Entry>,
    /// Parent directories created while staging, outermost first.
    created: Vec<PathBuf>,
    committed: bool,
}

impl Publication {
    #[must_use]
    pub fn new(existing: Existing) -> Self {
        Self {
            existing,
            entries: Vec::new(),
            created: Vec::new(),
            committed: false,
        }
    }

    /// Fails with [`PublishError::Exists`] when `target` exists and existing
    /// outputs are refused. Commands call it for every output before doing any
    /// work.
    ///
    /// # Errors
    /// [`PublishError::Exists`] as described.
    pub fn check(&self, target: impl AsRef<Path>) -> Result<(), PublishError> {
        let target = target.as_ref();
        if self.existing == Existing::Refuse && exists(target) {
            return Err(PublishError::Exists {
                path: target.to_path_buf(),
            });
        }
        Ok(())
    }

    /// Stage the file `target`: `write` fills a temporary file next to it.
    ///
    /// # Errors
    /// [`PublishError`] when the target is refused, staged twice or the
    /// temporary file cannot be created, flushed or synced; whatever `write`
    /// returns. With [`Existing::Refuse`] and an existing target, `write` is
    /// not called.
    pub fn stage_file<E, F>(&mut self, target: impl AsRef<Path>, write: F) -> Result<(), E>
    where
        E: From<PublishError>,
        F: FnOnce(&mut Staging<'_>) -> Result<(), E>,
    {
        let target = self.prepare(target.as_ref())?;
        let mut file = temporary_file(&target)?;
        let mut writer = BufWriter::new(file.as_file_mut());
        write(&mut writer)?;
        writer
            .flush()
            .map_err(|source| PublishError::io(&target, source))?;
        drop(writer);
        file.as_file()
            .sync_all()
            .map_err(|source| PublishError::io(&target, source))?;
        self.entries.push(Entry {
            target,
            staged: Staged::File(file),
        });
        Ok(())
    }

    /// Stage the file `target` through a writer that takes a path: `write`
    /// receives the path of an empty temporary file next to `target` and may
    /// truncate or replace it.
    ///
    /// # Errors
    /// As [`stage_file`](Self::stage_file).
    pub fn stage_path<E, F>(&mut self, target: impl AsRef<Path>, write: F) -> Result<(), E>
    where
        E: From<PublishError>,
        F: FnOnce(&Path) -> Result<(), E>,
    {
        let target = self.prepare(target.as_ref())?;
        let file = temporary_file(&target)?;
        write(file.path())?;
        sync_file(file.path()).map_err(|source| PublishError::io(&target, source))?;
        self.entries.push(Entry {
            target,
            staged: Staged::File(file),
        });
        Ok(())
    }

    /// Stage the directory `target` and return the empty directory to fill
    /// in its place. Its files are synced at commit.
    ///
    /// # Errors
    /// [`PublishError`] when the target is refused, staged twice or the
    /// directory cannot be created.
    pub fn stage_dir(&mut self, target: impl AsRef<Path>) -> Result<PathBuf, PublishError> {
        let target = self.prepare(target.as_ref())?;
        let prefix = hidden_prefix(&target);
        let mut builder = Builder::new();
        builder.prefix(&prefix).suffix(".tmp");
        request_mode(&mut builder, 0o777);
        let directory = builder
            .tempdir_in(parent(&target))
            .map_err(|source| PublishError::io(&target, source))?;
        let path = directory.path().to_path_buf();
        self.entries.push(Entry {
            target,
            staged: Staged::Directory(directory),
        });
        Ok(path)
    }

    /// Publish every staged output, in staging order, then sync the
    /// directories that hold them.
    ///
    /// With [`Existing::Refuse`] every target is checked again first, so a
    /// target that appeared since staging fails the commit before anything is
    /// published. If publishing one output fails, the outputs before it stay
    /// published and the rest are discarded; stage a completion record (such
    /// as provenance) last so that its presence means the set is complete.
    ///
    /// # Errors
    /// [`PublishError::Exists`] for a refused target, [`PublishError::Io`]
    /// when a rename or sync fails.
    pub fn commit(mut self) -> Result<(), PublishError> {
        for entry in &self.entries {
            self.check(&entry.target)?;
            if let Staged::Directory(directory) = &entry.staged {
                sync_tree(directory.path())
                    .map_err(|source| PublishError::io(&entry.target, source))?;
            }
        }
        let mut parents: Vec<PathBuf> = Vec::new();
        for entry in std::mem::take(&mut self.entries) {
            let directory = parent(&entry.target).to_path_buf();
            publish(entry, self.existing)?;
            if !parents.contains(&directory) {
                parents.push(directory);
            }
        }
        self.committed = true;
        for directory in parents {
            sync_directory(&directory).map_err(|source| PublishError::io(&directory, source))?;
        }
        Ok(())
    }

    fn prepare(&mut self, target: &Path) -> Result<PathBuf, PublishError> {
        if target.file_name().is_none() {
            return Err(PublishError::io(
                target,
                io::Error::new(io::ErrorKind::InvalidInput, "an output needs a file name"),
            ));
        }
        if self.entries.iter().any(|entry| entry.target == target) {
            return Err(PublishError::StagedTwice {
                path: target.to_path_buf(),
            });
        }
        self.check(target)?;
        let directory = parent(target);
        let missing = directory
            .ancestors()
            .take_while(|ancestor| !ancestor.as_os_str().is_empty() && !exists(ancestor))
            .collect::<Vec<_>>();
        fs::create_dir_all(directory).map_err(|source| PublishError::io(directory, source))?;
        self.created
            .extend(missing.into_iter().rev().map(Path::to_path_buf));
        Ok(target.to_path_buf())
    }
}

impl Drop for Publication {
    fn drop(&mut self) {
        self.entries.clear();
        if !self.committed {
            for directory in self.created.iter().rev() {
                // Fails, as intended, when the directory is no longer empty.
                let _ = fs::remove_dir(directory);
            }
        }
    }
}

/// Write one file all-or-nothing: [`Publication::stage_file`] then
/// [`Publication::commit`].
///
/// # Errors
/// As those two.
pub fn write_file<E, F>(target: impl AsRef<Path>, existing: Existing, write: F) -> Result<(), E>
where
    E: From<PublishError>,
    F: FnOnce(&mut Staging<'_>) -> Result<(), E>,
{
    let mut publication = Publication::new(existing);
    publication.stage_file(target, write)?;
    publication.commit().map_err(E::from)
}

fn publish(entry: Entry, existing: Existing) -> Result<(), PublishError> {
    let Entry { target, staged } = entry;
    match staged {
        Staged::File(file) => {
            let persisted = match existing {
                Existing::Refuse => file.persist_noclobber(&target),
                Existing::Replace => file.persist(&target),
            };
            match persisted {
                Ok(_) => Ok(()),
                Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                    Err(PublishError::Exists { path: target })
                }
                Err(error) => Err(PublishError::io(&target, error.error)),
            }
        }
        Staged::Directory(directory) => publish_directory(directory, &target, existing),
    }
}

/// Rename a staged directory onto `target`. An existing target is moved
/// aside first and restored if the rename fails.
fn publish_directory(
    directory: TempDir,
    target: &Path,
    existing: Existing,
) -> Result<(), PublishError> {
    let io_error = |source| PublishError::io(target, source);
    if !exists(target) {
        fs::rename(directory.path(), target).map_err(io_error)?;
        let _ = directory.keep();
        return Ok(());
    }
    if existing == Existing::Refuse {
        return Err(PublishError::Exists {
            path: target.to_path_buf(),
        });
    }
    let backup = Builder::new()
        .prefix(&hidden_prefix(target))
        .suffix(".old")
        .tempdir_in(parent(target))
        .map_err(io_error)?;
    let previous = backup.path().join("previous");
    fs::rename(target, &previous).map_err(io_error)?;
    if let Err(source) = fs::rename(directory.path(), target) {
        fs::rename(&previous, target).map_err(io_error)?;
        return Err(io_error(source));
    }
    let _ = directory.keep();
    // `backup` drops here and removes the previous output.
    Ok(())
}

fn temporary_file(target: &Path) -> Result<NamedTempFile, PublishError> {
    let prefix = hidden_prefix(target);
    let mut builder = Builder::new();
    builder.prefix(&prefix).suffix(".tmp");
    request_mode(&mut builder, 0o666);
    builder
        .tempfile_in(parent(target))
        .map_err(|source| PublishError::io(target, source))
}

fn hidden_prefix(target: &Path) -> OsString {
    let mut prefix = OsString::from(".");
    prefix.push(target.file_name().unwrap_or_default());
    prefix.push(".");
    prefix
}

fn parent(target: &Path) -> &Path {
    target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// A path is taken when anything is there, including a dangling symlink.
fn exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

/// Request the mode that `File::create` (0o666) or `fs::create_dir` (0o777)
/// use; the process umask applies to both.
#[cfg(unix)]
fn request_mode(builder: &mut Builder<'_, '_>, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    builder.permissions(Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn request_mode(_builder: &mut Builder<'_, '_>, _mode: u32) {}

fn sync_file(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Sync every file and directory under `root`, iteratively.
fn sync_tree(root: &Path) -> io::Result<()> {
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                sync_file(&entry.path())?;
            }
        }
        sync_directory(&directory)?;
    }
    Ok(())
}
