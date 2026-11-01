# Contributing to cytiq-meta

Workspace-wide rules (commands, style, commits, PRs) live in the root
[CONTRIBUTING.md](../../CONTRIBUTING.md). This file covers what is specific to
this crate.

## Layout

- `src/lib.rs` — crate docs and re-exports only.
- `src/runner.rs` — `MetaRunner`, `read_metadata`, the serializable document
  types, hashing and atomic output.
- `src/error.rs` — `MetaError` (`thiserror`).

## Invariants

- The JSON document is a public contract. Any change to field names, types or
  meaning must bump `SCHEMA_VERSION` in `src/runner.rs` and update this crate's
  [document shape](README.md#what-is-recorded). Adding a field is still a schema change.
- `text` and `supplemental_text` are copied verbatim from `cytiq-fcs`, in file
  order. Do not trim, normalize, deduplicate or sort keywords here; that would
  defeat the purpose of the crate.
- The SHA-256 and byte count describe the raw file on disk, hashed separately
  from parsing.
- Segment offsets come from the HEADER first and fall back to the
  `$BEGIN*`/`$END*` TEXT keywords only when the HEADER field is empty.
- Output is all-or-nothing: check `overwrite` before reading the input, then
  write through `cytiq_publish::write_file`, which stages a synced temporary
  file beside the output and renames it. Never write to the final path.
- Write only the document to stdout. Diagnostics go through `log` and end up on
  stderr.

## Where changes belong

- Changes to FCS parsing or to what `FcsDataset` exposes belong in
  `cytiq-fcs`, not here.
- CLI flags and argument parsing belong in `crates/cytiq-cli/src/main.rs`.
  Keep this crate free of `clap`.

## Testing

```bash
cargo test -p cytiq-meta
cargo test -p cytiq-cli --test meta_cli
```

The crate has no unit tests yet; the CLI tests build small FCS fixtures in a
temp directory and check the stdout/file output, `--overwrite` handling and the
SHA-256. Add unit tests next to the code with `#[cfg(test)]` for new logic in
`runner.rs`, for example the offset and `$TOT` fallbacks.
