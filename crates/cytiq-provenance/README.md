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
  "schema_version": 1,
  "tool": "cytiq compensate",
  "version": "0.1.0",
  "arguments": ["compensate", "--input", "sample.fcs", "--output", "sample.comp.fcs"],
  "started_at_unix_ms": 1792790000000,
  "finished_at_unix_ms": 1792790000420,
  "inputs":  [{ "path": "sample.fcs", "bytes": 320905, "sha256": "…" }],
  "outputs": [{ "path": "sample.comp.fcs", "bytes": 321580, "sha256": "…" },
              { "path": "sample.comp.masks.parquet", "bytes": 21544, "sha256": "…" }]
}
```

`arguments` is the exact command line, so a run can be repeated from its
record. Debug directories are not listed.

## Public API

| Item | Purpose |
| --- | --- |
| `RunRecorder::start` / `finish` | Start a record before a command runs; hash its files and write the record afterwards |
| `RunRecord`, `FileRecord`, `SCHEMA_VERSION` | The document |
| `record_path_for_file`, `record_path_for_dir` | Where a command's record goes |
| `files_under` | Every file of an output directory, sorted, without records and `.tmp` files |
| `hash_file` | Streamed SHA-256 (also used by `cytiq-meta`) |
| `ProvenanceError` | Error type |

## Rules

- Records are written through a `.tmp` sibling and renamed, so a failed run
  never leaves half a record.
- Paths that are not files when the command finishes (optional outputs that
  were not requested) are skipped.
- `--dry-run` runs write no record.

## Dependencies

`serde` / `serde_json`, `sha2`, `log`, `thiserror`.

## Tests

```bash
cargo test -p cytiq-provenance
cargo test -p cytiq-cli   # every *_cli.rs checks its command's record via tests/common
```
