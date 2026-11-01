# Contributing to cytiq-publish

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs and re-exports only.
- `src/publication.rs` — `Existing`, `Publication`, `write_file`, the rename,
  backup and sync helpers.
- `src/error.rs` — `PublishError` (`thiserror`).
- `tests/publication.rs` — failure injection: failing writers, targets that
  appear or cannot be replaced, modes, directory swaps.

## Invariants

- Nothing is visible at a target before `commit`. Staged outputs live under
  temporary names in the target's own directory; never stage elsewhere (a
  rename across file systems is a copy).
- `Refuse` never runs a writer for a target that exists and never renames over
  an existing file.
- Sync before publishing and sync the directory after. Do not drop a sync to
  save time; outputs are small next to the work that produces them.
- No `unsafe` and no dependencies beyond `tempfile` and `thiserror`. Platform
  specifics (modes, directory sync) stay behind `cfg(unix)`.
- Error injection uses real file system conditions in tests, never hooks in
  this crate.

## Testing

```bash
cargo test -p cytiq-publish
```
