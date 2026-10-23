# Contributing to cytiq-provenance

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — `RunRecorder`, `RunRecord`, `FileRecord`, record paths,
  `files_under`, `hash_file`; unit tests.

## Invariants

- The record is a public contract versioned by `SCHEMA_VERSION`. Changing a
  field name, type or meaning bumps it and updates the `## Provenance` section
  of the root `README.md`.
- `arguments` is the command line verbatim. Do not normalise or reorder it.
- Records are written atomically (`.tmp` + rename).
- The CLI decides which files are inputs and outputs of each command
  (`crates/cytiq-cli/src/main.rs`, `record_provenance`). When a command gains
  an input or output file, add it there and to its `*_cli.rs` provenance check.

## Testing

```bash
cargo test -p cytiq-provenance
cargo test -p cytiq-cli
```
