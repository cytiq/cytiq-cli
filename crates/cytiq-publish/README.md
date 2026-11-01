# cytiq-publish

All-or-nothing publication of command outputs. A command stages every output
next to its target, writes and syncs it, and publishes the set with one
`commit`. Until then nothing appears at a target path: an error, a panic or a
dropped publication leaves the previous outputs in place and no temporary
files behind.

## API

| Item | Purpose |
| --- | --- |
| `Existing::{Refuse, Replace}` | what to do with a target that exists; `Existing::overwrite(flag)` maps `--overwrite` |
| `Publication::new(existing)` | an empty set of staged outputs |
| `check(target)` | fail early, before any work, when a target is refused |
| `stage_file(target, \|writer\| ...)` | stage a file written through a buffered writer |
| `stage_path(target, \|path\| ...)` | stage a file written by a writer that takes a path |
| `stage_dir(target)` | stage a directory; returns the empty directory to fill |
| `commit()` | publish every staged output in staging order, then sync their directories |
| `write_file(target, existing, \|writer\| ...)` | one file: stage and commit |

Errors are `PublishError::{Exists, StagedTwice, Io}`. The staging closures
return the caller's error type, which needs `From<PublishError>`.

## Guarantees

- A temporary output is `.<target name>.<random>.tmp` in the target's
  directory, so publishing is a rename on one file system.
- Files are flushed and `fsync`ed before they are published; files inside a
  staged directory are synced at commit; the directories holding the targets
  are synced after the renames.
- Files get the permissions `File::create` gives them and directories those of
  `fs::create_dir` (0o666 and 0o777 minus the umask), not `tempfile`'s
  owner-only 0o600 / 0o700.
- `Refuse` is checked when an output is staged, before its writer runs, and
  again at commit before anything is renamed. Files are then published with a
  no-clobber rename.
- `Replace` leaves the previous output in place until commit. A directory is
  replaced by moving the previous one aside, renaming the staged one in, and
  restoring the previous one if that rename fails.
- Parent directories created while staging are removed again if nothing is
  published.

`commit` renames one output at a time. If a rename fails, the outputs before it
stay published and the rest are discarded, so stage a completion record such
as `provenance.json` last: a run is complete if and only if its record exists.

## Usage

```rust
use std::io::Write;
use cytiq_publish::{Existing, Publication, PublishError};

fn run(output: &std::path::Path, overwrite: bool) -> Result<(), PublishError> {
    let mut publication = Publication::new(Existing::overwrite(overwrite));
    publication.check(output.join("result.csv"))?;
    // ... compute ...
    publication.stage_file(output.join("result.csv"), |w| {
        w.write_all(b"a,b\n").map_err(|e| PublishError::io("result.csv", e))
    })?;
    publication.commit()
}
```
