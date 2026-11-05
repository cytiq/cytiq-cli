# Contributing to cytiq-provenance

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — `RunRecorder`, `RunRecord`, `FileRecord`, record paths,
  `files_under`, `hash_file`; unit tests.

## Invariants

- The record is a public contract versioned by `SCHEMA_VERSION`. Changing a
  field name, type or meaning bumps it and updates this crate's [README](README.md).
- `arguments` is the command line verbatim. Do not normalise or reorder it.
- Records are written through `cytiq-publish`: a synced temporary file in the
  same directory, renamed into place. `files_under` skips temporary files and
  directories (`*.tmp`, which covers the `*.old.tmp` backups a crash can
  leave) and provenance records.
- Inputs are hashed by `snapshot_inputs` before the command runs, never at
  `finish`.
- The CLI decides which files are inputs and outputs of each command
  (`crates/cytiq-engine/src/commands/*.rs` call `snapshot_inputs`, and `record_provenance` in `src/provenance.rs`).
  When a command gains an input or output file, add it there and to its
  `*_cli.rs` provenance check.

## Testing

```bash
cargo test -p cytiq-provenance
cargo test -p cytiq-engine
```
