# Contributing to cytiq-publish

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs and re-exports only.
- `src/publication.rs` — `Existing`, `Publication`, `write_file`, the rename,
  backup, undo and sync helpers; unit tests for undo steps that fail.
- `src/error.rs` — `PublishError`, `RollbackFailure` (`thiserror`).
- `tests/publication.rs` — failure injection: failing writers, targets that
  appear or cannot be replaced, renames that fail mid-commit, modes, directory
  swaps.

## Invariants

- Nothing is visible at a target before `commit`. Staged outputs live under
  temporary names in the target's own directory; never stage elsewhere (a
  rename across file systems is a copy).
- `Refuse` never runs a writer for a target that exists and never renames over
  an existing file.
- Sync before publishing and sync the directory after: the directories that
  hold the targets and those that hold the directories created while staging.
  Do not drop a sync to save time; outputs are small next to the work that
  produces them.
- A failed commit publishes nothing. Every step of `commit` stays undoable
  until the directories are synced: back up each file to replace (hard link,
  else synced copy) before the first rename, never remove a target to make
  room for a file, keep a replaced directory moved aside, and journal every
  published output. Undo newest first and return the original error; delete
  backups only after the sync, and keep any backup an undo could not put back.
- Every temporary and backup name ends in `.tmp` (`.tmp`, `.old.tmp`), so
  `cytiq-provenance::files_under` skips what a crash leaves behind.
- No `unsafe` and no dependencies beyond `tempfile` and `thiserror`. Platform
  specifics (modes, directory sync) stay behind `cfg(unix)`.
- Error injection uses real file system conditions in tests, never hooks in
  this crate.

## Testing

```bash
cargo test -p cytiq-publish
```
