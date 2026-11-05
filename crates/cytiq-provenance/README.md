# cytiq-provenance

Run records for cytiq commands: which command, with which arguments, read
which files and wrote which files.

Every command that writes files (all except `meta`, whose output already
records the input's SHA-256) writes a `provenance.json` next to its output:

- `<output>.provenance.json` when the output is a file (`sample.comp.fcs` →
  `sample.comp.fcs.provenance.json`);
- `<dir>/provenance.json` when the output is a directory (`clean`,
  `autospectral`, `cytoscan`, `transform` with a directory output).

```json
{
  "schema_version": 2,
  "tool": "cytiq compensate",
  "version": "0.1.0",
  "build": { "git_commit": "…", "rustc": "rustc 1.99.1 …", "target": "aarch64-apple-darwin", "profile": "release" },
  "platform": { "os": "macos", "arch": "aarch64", "cpus": 12 },
  "arguments": ["compensate", "--input", "sample.fcs", "--output", "sample.comp.fcs"],
  "config": { "input": "sample.fcs", "output": "sample.comp.fcs", "matrix": null, "matrix_kind": null, "overwrite": false },
  "constants": null,
  "algorithms": { "FlowKit": "1.3.0", "flowCore": "2.22.1" },
  "started_at_unix_ms": 1792790000000,
  "finished_at_unix_ms": 1792790000420,
  "inputs":  [{ "path": "sample.fcs", "role": "input", "bytes": 320905, "sha256": "…" }],
  "outputs": [{ "path": "sample.comp.fcs", "bytes": 321580, "sha256": "…" },
              { "path": "sample.comp.masks.parquet", "bytes": 21544, "sha256": "…" }]
}
```

`arguments` is the exact command line, so a run can be repeated from its
record; `config` is every option with its default applied, so the record
stays meaningful when defaults change. `constants` and `algorithms` capture
what is fixed in the code: algorithm constants and the upstream versions the
port is validated against. Inputs are hashed before the command runs, so a
command that replaces its own input still records what it read; each input
carries its `role`. Debug directories are not listed.

`build.git_commit` is `CYTIQ_BUILD_COMMIT` when set at build time (as in
Docker), else the commit cargo recorded when packaging the crate (a crates.io
install; `-dirty` marks uncommitted changes), else `git rev-parse HEAD`, which
does not detect uncommitted changes. Following each record's inputs back to earlier outputs
reconstructs the chain from the raw FCS files.

Schema version 2 added `build`, `platform`, `config`, `constants`,
`algorithms` and input roles, and moved input hashing before the run.

## Public API

| Item | Purpose |
| --- | --- |
| `RunRecorder::start` / `snapshot_inputs` / `snapshot_roles` / `finish` | Start a record and hash the inputs, with their role, before a command runs; hash its outputs and write the record afterwards |
| `RunRecorder::with_build`, `set_config`, `set_constants`, `set_algorithms` | Build info, effective configuration, fixed algorithm constants, upstream versions |
| `RunRecord`, `FileRecord`, `BuildInfo`, `Platform`, `SCHEMA_VERSION` (2) | The document |
| `record_path_for_file`, `record_path_for_dir` | Where a command's record goes |
| `files_under` | Every file of an output directory, sorted, without records and `.tmp` files or directories (staged outputs and `.old.tmp` backups of `cytiq-publish`) |
| `hash_file` | Streamed SHA-256 (also used by `cytiq-meta`) |
| `ProvenanceError` | Error type |

## Rules

- Records are written through `cytiq-publish` (a synced temporary file,
  renamed into place), so a failed run never leaves half a record.
- A run's outputs are complete when its record exists and every output it
  lists has the recorded size and SHA-256. A failed run publishes none of its
  outputs, but a crash in the middle of publishing can leave some outputs new
  and others previous ones next to the previous run's record; only the hashes
  tell.
- Paths that are not files (optional inputs and outputs that were not given)
  are skipped.
- An input that changed during the run and is not also an output fails
  `finish` (`InputChanged`) rather than write a record that does not describe
  what was read. An input that is also an output (`transform --overwrite` in
  place) is recorded with both hashes.
- `--dry-run` runs write no record.

## Dependencies

`serde` / `serde_json`, `sha2`, `log`, `thiserror`.

## Tests

```bash
cargo test -p cytiq-provenance
cargo test -p cytiq-engine   # every *_cli.rs checks its command's record via tests/common
```
